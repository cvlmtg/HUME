//! The bottom-drawer layer — `(show-drawer-list! items on-select)`'s raw
//! state, browsed with Helix-style "stay open while editing" semantics.

use std::sync::Arc;

use termina::event::{KeyCode, Modifiers};

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;

use super::super::Editor;
use super::super::EditorState;
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef};

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
    // `tear_down` stays at the trait's empty default — an explicit
    // `close-drawer!` (routed through `EditorState::retire`, which reaches
    // this) must stay silent; `drawer_input`'s own `Esc` arm takes the
    // layer *by value* via `EditorState::take_layer` and fires its own
    // callback explicitly instead.
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
}

/// Named sugar over the generic lookup — the ~350 existing call sites
/// (`ed.state.input.drawer()`) stay as they are, and `stack.rs` stays agnostic.
impl super::stack::InputStack {
    pub(in crate::editor) fn drawer(&self) -> Option<&DrawerLayer> {
        self.find()
    }

    pub(in crate::editor) fn drawer_mut(&mut self) -> Option<&mut DrawerLayer> {
        self.find_mut()
    }
}

/// Handles one key while the bottom drawer is open. Movement, half-page
/// scroll, and `Enter` (which fires `on-select` repeatedly across a
/// browse session, unlike the menu, without closing the drawer) are
/// handled in place; `Esc` retires the layer and fires `#f`; any other
/// key falls through completely untouched (no close, no callback),
/// leaving the drawer open while focus moves to whatever the fallen-
/// through key does (Helix-style browse-while-editing).
///
/// No mode gate of its own — same reasoning as [`super::menu::menu_input`].
/// A mouse event falls through untouched, same as any other key the
/// drawer doesn't bind — browsing never blocks the cursor from moving.
pub(in crate::editor) fn drawer_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let key = match ev {
        InputEvent::Key(key) => key,
        // Swallowed like every other key the drawer doesn't bind to
        // movement/scroll/Enter/Esc — stays open, same as today's
        // `Paste` handling under a drawer (`mappings/bracketed_paste.rs`'s
        // old menu/drawer guard).
        InputEvent::Paste(_) => return,
        InputEvent::Mouse(mouse) => {
            ed.fall_through(r, InputEvent::Mouse(mouse));
            return;
        }
    };
    // Every movement key differs only in the delta passed to
    // `move_drawer_selection` — collapsed to one borrow instead of one per
    // key, mirroring `picker_input`'s own step table.
    let step: Option<isize> = match key.code {
        KeyCode::Char('j') | KeyCode::Down => Some(1),
        KeyCode::Char('k') | KeyCode::Up => Some(-1),
        KeyCode::Char('d') if key.modifiers.contains(Modifiers::CONTROL) => {
            Some((drawer_visible_rows(ed) / 2).max(1) as isize)
        }
        KeyCode::Char('u') if key.modifiers.contains(Modifiers::CONTROL) => {
            Some(-((drawer_visible_rows(ed) / 2).max(1) as isize))
        }
        _ => None,
    };
    if let Some(delta) = step {
        move_drawer_selection(ed, delta);
        return;
    }

    match key.code {
        KeyCode::Enter => {
            let drawer = ed
                .state
                .input
                .drawer()
                .expect("dispatch_at already checked kind(r) == DrawerLayer");
            let idx = steel::rvals::SteelVal::IntV(drawer.selected as isize);
            let callback = drawer.callback.clone();
            ed.state.queue_steel_call(callback, vec![idx]);
        }
        KeyCode::Escape => {
            let drawer = ed.state.take_layer::<DrawerLayer>(&ed.view, r);
            ed.state
                .queue_steel_call(drawer.callback, vec![steel::rvals::SteelVal::BoolV(false)]);
            ed.state.sync_drawer_view();
        }
        _ => {
            ed.fall_through(r, InputEvent::Key(key));
        }
    }
}

/// Number of drawer rows visible at once — computed via
/// `hume_ui::drawer::visible_rows`, the same arithmetic
/// `DrawerWidget::height` uses to size what it paints next frame. `max`
/// is `EngineView::bottom_band_max` of the last-rendered *terminal*
/// height (not the already-chrome-reduced pane height) — the same call
/// the engine itself makes, so this can never drift from what it will
/// next paint. Shared by [`clamp_drawer_scroll`] and the Ctrl-u/Ctrl-d
/// half-page handlers so "half a page" always agrees with what's on
/// screen.
fn drawer_visible_rows(ed: &Editor) -> usize {
    let max = EngineView::bottom_band_max(ed.view.last_terminal_area.height);
    let Some(drawer) = ed.state.input.drawer() else {
        return 0;
    };
    hume_ui::drawer::visible_rows(drawer.items.len(), max)
}

/// Moves the drawer's selection by `delta` (clamped to `[0, len - 1]`), then
/// syncs the scroll/view — shared by every movement key (`j`/`k`/Ctrl-d/
/// Ctrl-u) so each key site is just "which delta", not its own lookup.
fn move_drawer_selection(ed: &mut Editor, delta: isize) {
    let drawer = ed
        .state
        .input
        .drawer_mut()
        .expect("dispatch_at already checked kind(r) == DrawerLayer");
    let len = drawer.items.len();
    if len > 0 {
        let new = (drawer.selected as isize + delta).clamp(0, len as isize - 1);
        drawer.selected = new as usize;
    }
    clamp_drawer_scroll(ed);
}

/// Clamps `drawer.scroll` so `drawer.selected` stays within the visible
/// window (`clamp_scroll_to_window`, shared with `PickerSession::
/// move_selection`), then syncs the view.
fn clamp_drawer_scroll(ed: &mut Editor) {
    let visible_rows = drawer_visible_rows(ed);
    let Some(drawer) = ed.state.input.drawer_mut() else {
        return;
    };
    drawer.scroll =
        hume_ui::menu_box::clamp_scroll_to_window(drawer.selected, drawer.scroll, visible_rows);
    ed.state.sync_drawer_view();
}
