//! The bottom-drawer layer — `(show-drawer-list! items on-select)`'s raw
//! state, browsed with Helix-style "stay open while editing" semantics.

use std::sync::Arc;

use termina::event::{KeyCode, Modifiers};

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;

use super::super::Editor;
use super::super::EditorState;
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, Removal};

/// `(show-drawer-list! items on-select)`'s raw state, including the
/// not-yet-exhausted Steel callback — cleared by `Esc` or `close-drawer!`,
/// not by `Enter` (the drawer stays open across selections).
pub(in crate::editor) struct DrawerLayer {
    /// Shared with `hume_ui::drawer::DrawerViewState::rows` (an
    /// `Arc::clone`, not a deep copy) — `sync_drawer_view` runs
    /// unconditionally every frame while the drawer is open, and a
    /// references batch can carry thousands of rows.
    pub(in crate::editor) items: std::sync::Arc<Vec<String>>,
    pub(in crate::editor) selected: usize,
    /// Index of the first visible row — clamped to keep `selected` in view
    /// whenever the selection moves ([`clamp_drawer_scroll`]).
    pub(in crate::editor) scroll: usize,
    pub(in crate::editor) callback: steel::rvals::SteelVal,
}

impl Layer for DrawerLayer {
    fn handler(&self) -> LayerHandler {
        drawer_input
    }
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    // No `setup`/`popup_eviction` override — the trait's own default
    // (`PopupEviction::Both`) is exactly right: a `Drawer` is non-modal
    // (`is_modal` below), so it can land directly above a `Popup` the same
    // way a `Menu` can, and `push_layer` evicts it on the way in regardless
    // of which of the two opens second.
    //
    /// Fires `#f` when `why` is [`Removal::Incidental`] — swept up as
    /// collateral above some other target. No concrete path reaches this
    /// today: a `Drawer` only ever lands directly on `Base` (every entry
    /// gate that pushes one requires the mode layer to be `Base`, and
    /// `push_layer`'s eviction clears the one non-modal layer, `Popup`,
    /// that could otherwise sit beneath a fresh one), and `Base` is never a
    /// truncate target. Still branches on `why`, matching `MenuLayer`'s own
    /// fix for the same bug class, rather than leaving "silent unless
    /// swept as collateral" true only by that accident of what happens to
    /// land where today. Stays silent on [`Removal::Explicit`]: an
    /// explicit `close-drawer!` (routed through `EditorState::excise_layer`,
    /// which reaches this as the named target without touching whatever
    /// else is stacked above it — an `Insert` session browsing the drawer,
    /// say) must stay silent; `drawer_input`'s own `Esc` arm takes the layer
    /// *by value* via `EditorState::take_firing_false` and fires its own
    /// callback explicitly instead.
    fn tear_down(&mut self, state: &mut EditorState, _view: &EngineView, why: Removal) {
        if let Removal::Incidental = why {
            state.queue_steel_call(
                self.callback.clone(),
                vec![steel::rvals::SteelVal::BoolV(false)],
            );
        }
    }
    /// Non-modal: the drawer is built to be worked over (a stray key falls
    /// through and it stays open), so an async opener's staleness check
    /// (`InputStack::is_settled_for`) must not read "a drawer is open" as
    /// "the stack moved" — a code-action menu, or a fresh `completion-begin!`,
    /// still needs to open while the user is browsing one.
    fn is_modal(&self) -> bool {
        false
    }
}

impl EditorState {
    /// Mirror the open drawer layer into `self.views`' drawer slot for
    /// `DrawerWidget` to read. Called directly at every drawer mutation site
    /// (open, selection move, scroll, close) for immediacy, *and*
    /// unconditionally every frame from `Editor::prepare_frame` (like the
    /// popup/menu/picker `sync_*_view`s) so the view can never drift from
    /// the model — in particular, so `reset_config_state`'s
    /// `input.truncate_to_base()` call (which bypasses `close-drawer!`'s
    /// callback queueing) can't leave a stale view painting a closed
    /// drawer.
    pub(in crate::editor) fn sync_drawer_view(&self) {
        let resolved = self
            .input
            .drawer()
            .map(|d| hume_ui::drawer::DrawerViewState {
                rows: Arc::clone(&d.items),
                selected: d.selected,
                scroll: d.scroll,
            });
        self.views.drawer.set(resolved);
    }

    /// Replaces the open drawer's rows in place, keeping the browse session
    /// — the single writer behind `update-drawer-list!`, so that builtin and
    /// the selection keys (`move_drawer_selection`/`clamp_drawer_scroll`)
    /// can't drift on clamping. `selected` is clamped into the new list and
    /// the scroll re-clamped into the visible window, the same
    /// `clamp_scroll_to_window` the keys get. Returns whether the update
    /// applied (`false` when no drawer is open — an expected-normal race,
    /// never an error — or when `items` is empty, so a 0-row drawer with an
    /// `Enter` that would fire `0` can never be built from either entry
    /// point; callers close instead).
    pub(in crate::editor) fn set_drawer_items(
        &mut self,
        terminal_height: u16,
        items: Vec<String>,
        callback: steel::rvals::SteelVal,
        selected: usize,
    ) -> bool {
        if items.is_empty() {
            return false;
        }
        let Some(drawer) = self.input.find_mut::<DrawerLayer>() else {
            return false;
        };
        drawer.items = Arc::new(items);
        drawer.callback = callback;
        let len = drawer.items.len();
        drawer.selected = selected.min(len - 1);
        let visible = drawer_visible_for(terminal_height, len);
        drawer.scroll =
            hume_ui::menu_box::clamp_scroll_to_window(drawer.selected, drawer.scroll, len, visible);
        self.sync_drawer_view();
        true
    }
}

impl super::stack::FiresFalseOnReplace for DrawerLayer {
    fn into_callback(self: Box<Self>) -> steel::rvals::SteelVal {
        self.callback
    }
}

/// Named sugar over the generic lookup — the ~350 existing call sites
/// (`ed.state.input.drawer()`) stay as they are, and `stack.rs` stays
/// agnostic. Read-only: every handler-side mutation now addresses its own
/// dispatched-to layer via `at_mut::<DrawerLayer>(r)` instead, so there is
/// no `drawer_mut()` sibling — this one's only remaining caller is
/// render-sync (`sync_drawer_view`, independent of dispatch), which never
/// needs `&mut`.
impl super::stack::InputStack {
    pub(in crate::editor) fn drawer(&self) -> Option<&DrawerLayer> {
        self.find()
    }
}

/// Handles one key while the bottom drawer is open. Ctrl-d/Ctrl-u move
/// the selection one line at a time, and `Enter` (which fires `on-select`
/// repeatedly across a browse session, unlike the menu, without closing
/// the drawer) is handled in place; `Esc` retires the layer and fires
/// `#f`; any other key — including `j`/`k` and the arrow keys — falls
/// through completely untouched (no close, no callback), leaving the
/// drawer open while focus moves to whatever the fallen-through key does
/// (Helix-style browse-while-editing, so vertical motion in the source
/// buffer keeps working).
///
/// No mode gate of its own — same reasoning as [`super::menu::menu_input`].
/// A mouse event falls through untouched, same as any other key the
/// drawer doesn't bind — browsing never blocks the cursor from moving.
pub(in crate::editor) fn drawer_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let key = match ev {
        InputEvent::Key(key) => key,
        // Swallowed like every other key the drawer doesn't bind to
        // single-line selection/Enter/Esc — stays open, same as today's
        // `Paste` handling under a drawer (`mappings/bracketed_paste.rs`'s
        // old menu/drawer guard).
        InputEvent::Paste(_) => return,
        InputEvent::Mouse(mouse) => {
            ed.fall_through(r, InputEvent::Mouse(mouse));
            return;
        }
    };
    // Both selection keys differ only in the delta passed to
    // `move_drawer_selection` — collapsed to one borrow instead of one per
    // key, mirroring `picker_input`'s own step table.
    let step: Option<isize> = match key.code {
        KeyCode::Char('d') if key.modifiers.contains(Modifiers::CONTROL) => Some(1),
        KeyCode::Char('u') if key.modifiers.contains(Modifiers::CONTROL) => Some(-1),
        _ => None,
    };
    if let Some(delta) = step {
        move_drawer_selection(ed, r, delta);
        return;
    }

    match key.code {
        KeyCode::Enter => {
            let drawer = ed
                .state
                .input
                .at::<DrawerLayer>(r)
                .expect("dispatch_at already checked kind(r) == DrawerLayer");
            let idx = steel::rvals::SteelVal::IntV(drawer.selected as isize);
            let callback = drawer.callback.clone();
            ed.state.queue_steel_call(callback, vec![idx]);
        }
        KeyCode::Escape => {
            ed.state.take_firing_false::<DrawerLayer>(&ed.view, r);
            ed.state.sync_drawer_view();
        }
        _ => {
            ed.fall_through(r, InputEvent::Key(key));
        }
    }
}

/// Rows visible at once for a drawer holding `len` items on a terminal
/// `terminal_height` tall — `hume_ui::drawer::visible_rows`, the same
/// arithmetic `DrawerWidget::height` uses to size what it paints next
/// frame. The height is the last-rendered *terminal* height (not the
/// already-chrome-reduced pane height) — the same call the engine itself
/// makes, so this can never drift from what it will next paint. Shared by
/// the selection keys (via `drawer_visible_rows`) and the in-place refresh
/// (`EditorState::set_drawer_items`), so the scroll window always agrees
/// with what's on screen on both paths.
fn drawer_visible_for(terminal_height: u16, len: usize) -> usize {
    hume_ui::drawer::visible_rows(len, EngineView::bottom_band_max(terminal_height))
}

/// Number of drawer rows visible at once — read by [`clamp_drawer_scroll`]
/// so the scroll window always agrees with what's on screen.
fn drawer_visible_rows(ed: &Editor, r: LayerRef) -> usize {
    let Some(drawer) = ed.state.input.at::<DrawerLayer>(r) else {
        return 0;
    };
    drawer_visible_for(ed.view.last_terminal_area.height, drawer.items.len())
}

/// Moves the drawer's selection by `delta` (clamped to `[0, len - 1]`), then
/// syncs the scroll/view — shared by both selection keys (Ctrl-d/Ctrl-u)
/// so each key site is just "which delta", not its own lookup.
fn move_drawer_selection(ed: &mut Editor, r: LayerRef, delta: isize) {
    let drawer = ed
        .state
        .input
        .at_mut::<DrawerLayer>(r)
        .expect("dispatch_at already checked kind(r) == DrawerLayer");
    let len = drawer.items.len();
    if len > 0 {
        let new = (drawer.selected as isize + delta).clamp(0, len as isize - 1);
        drawer.selected = new as usize;
    }
    clamp_drawer_scroll(ed, r);
}

/// Clamps `drawer.scroll` so `drawer.selected` stays within the visible
/// window (`clamp_scroll_to_window`, shared with `PickerSession::
/// move_selection`), then syncs the view.
fn clamp_drawer_scroll(ed: &mut Editor, r: LayerRef) {
    let visible_rows = drawer_visible_rows(ed, r);
    let Some(drawer) = ed.state.input.at_mut::<DrawerLayer>(r) else {
        return;
    };
    let len = drawer.items.len();
    drawer.scroll = hume_ui::menu_box::clamp_scroll_to_window(
        drawer.selected,
        drawer.scroll,
        len,
        visible_rows,
    );
    ed.state.sync_drawer_view();
}
