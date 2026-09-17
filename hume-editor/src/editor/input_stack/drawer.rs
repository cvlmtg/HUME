//! The bottom-drawer layer — `(show-drawer-list! items on-select)`'s raw
//! state, browsed with Helix-style "stay open while editing" semantics.

use termina::event::{KeyCode, Modifiers};

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;

use super::super::Editor;
use super::super::EditorState;
use super::stack::{InputEvent, Layer, LayerRef};

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
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    fn tear_down(&mut self, _state: &mut EditorState, _view: &EngineView) {}
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
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            let drawer = ed
                .state
                .input
                .drawer_mut()
                .expect("dispatch_at already checked kind(r) == DrawerLayer");
            if drawer.selected + 1 < drawer.items.len() {
                drawer.selected += 1;
                clamp_drawer_scroll(ed);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let drawer = ed
                .state
                .input
                .drawer_mut()
                .expect("dispatch_at already checked kind(r) == DrawerLayer");
            if drawer.selected > 0 {
                drawer.selected -= 1;
                clamp_drawer_scroll(ed);
            }
        }
        KeyCode::Char('d') if key.modifiers.contains(Modifiers::CONTROL) => {
            let half = (drawer_visible_rows(ed) / 2).max(1);
            if let Some(drawer) = ed.state.input.drawer_mut() {
                drawer.selected =
                    (drawer.selected + half).min(drawer.items.len().saturating_sub(1));
            }
            clamp_drawer_scroll(ed);
        }
        KeyCode::Char('u') if key.modifiers.contains(Modifiers::CONTROL) => {
            let half = (drawer_visible_rows(ed) / 2).max(1);
            if let Some(drawer) = ed.state.input.drawer_mut() {
                drawer.selected = drawer.selected.saturating_sub(half);
            }
            clamp_drawer_scroll(ed);
        }
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
            let mut removed = ed.state.input.truncate(r);
            let Some(drawer) = removed.pop().and_then(|l| l.downcast::<DrawerLayer>()) else {
                unreachable!("dispatch_at already checked kind(r) == DrawerLayer");
            };
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
