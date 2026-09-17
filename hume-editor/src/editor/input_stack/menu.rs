//! The selection-menu layer — `(show-menu! items on-select)`'s raw state.

use termina::event::KeyCode;

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;

use super::super::mouse::is_fresh_gesture;
use super::super::{Editor, EditorState};
use super::stack::{InputEvent, Layer, LayerRef};

/// `(show-menu! items on-select)`'s raw content — held on `EditorState`
/// until the next frame's `Editor::sync_menu_view` resolves it into a
/// positioned `hume_ui::popup::PopupState` with `selected` set. `callback`
/// fires exactly once (one per selection or dismissal), then the whole
/// model is dropped.
///
/// `rows` is pre-measured at construction, not re-measured per frame: labels
/// never change during a menu's lifetime (only `selected` does), so building
/// the `MenuRows` once here — instead of `sync_menu_view` calling
/// `MenuRows::measure` every frame the menu stays open — costs nothing
/// `Editor::sync_menu_view` isn't already paying at `show-menu!` time.
pub(in crate::editor) struct MenuLayer {
    pub(in crate::editor) rows: hume_ui::popup::MenuRows,
    pub(in crate::editor) selected: usize,
    pub(in crate::editor) callback: steel::rvals::SteelVal,
}

impl Layer for MenuLayer {
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    fn tear_down(&mut self, _state: &mut EditorState, _view: &EngineView) {}
}

/// Named sugar over the generic lookup — the ~350 existing call sites
/// (`ed.state.input.menu()`) stay as they are, and `stack.rs` stays agnostic.
impl super::stack::InputStack {
    pub(in crate::editor) fn menu(&self) -> Option<&MenuLayer> {
        self.find()
    }

    pub(in crate::editor) fn menu_mut(&mut self) -> Option<&mut MenuLayer> {
        self.find_mut()
    }
}

/// Handles one key while a selection menu is open. Movement is handled
/// in place; `Enter`/`Esc`/a stray key all retire the layer and fire the
/// callback exactly once (one-shot `.take()`-equivalent discipline via
/// `truncate`) — `queue_steel_call` never invokes it inline, matching
/// every other Rust→Steel callback in this codebase. A stray key both
/// closes the menu (with a `#f` callback) *and* falls through to normal
/// dispatch this same call.
///
/// No mode gate of its own: `show-menu!` only pushes with the mode
/// layer at `Base` (its own async-staleness check, `host_impl/ui.rs`),
/// and `push_mode_layer` always pushes a new mode layer *above* whatever
/// overlay sits on `Base` — so a `Menu` layer is dispatch's top only
/// while the mode layer beneath it is still `Base`.
///
/// A fresh mouse press or wheel notch gets the same treatment as a
/// stray key (close with `#f`, then fall through); a release, drag, or
/// move is the tail of a gesture already in flight (see
/// [`is_fresh_gesture`]'s doc) and falls through untouched, leaving the
/// menu open.
pub(in crate::editor) fn menu_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let key = match ev {
        InputEvent::Key(key) => key,
        // A paste is swallowed without closing the menu — same
        // "consumes stray input" treatment the menu gives any other key
        // it doesn't recognize as a choice, minus the `#f` callback
        // that arm fires: a paste was never a choice attempt.
        InputEvent::Paste(_) => return,
        InputEvent::Mouse(mouse) => {
            if !is_fresh_gesture(mouse.kind) {
                ed.fall_through(r, InputEvent::Mouse(mouse));
                return;
            }
            let menu = take_menu(ed, r);
            ed.state
                .queue_steel_call(menu.callback, vec![steel::rvals::SteelVal::BoolV(false)]);
            ed.fall_through(r, InputEvent::Mouse(mouse));
            return;
        }
    };
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            let menu = ed
                .state
                .input
                .menu_mut()
                .expect("dispatch_at already checked kind(r) == MenuLayer");
            if menu.selected + 1 < menu.rows.len() {
                menu.selected += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let menu = ed
                .state
                .input
                .menu_mut()
                .expect("dispatch_at already checked kind(r) == MenuLayer");
            menu.selected = menu.selected.saturating_sub(1);
        }
        KeyCode::Enter => {
            let menu = take_menu(ed, r);
            let idx = steel::rvals::SteelVal::IntV(menu.selected as isize);
            ed.state.queue_steel_call(menu.callback, vec![idx]);
        }
        KeyCode::Escape => {
            let menu = take_menu(ed, r);
            ed.state
                .queue_steel_call(menu.callback, vec![steel::rvals::SteelVal::BoolV(false)]);
        }
        _ => {
            let menu = take_menu(ed, r);
            ed.state
                .queue_steel_call(menu.callback, vec![steel::rvals::SteelVal::BoolV(false)]);
            ed.fall_through(r, InputEvent::Key(key));
        }
    }
}

/// Take the menu at `r` off the stack, handing back its model so the
/// caller can fire the one callback this layer owes
/// (`.take()`-equivalent one-shot discipline via `truncate`) — shared by
/// every `menu_input` retirement path (Enter, Escape, a stray key, a
/// fresh mouse gesture).
fn take_menu(ed: &mut Editor, r: LayerRef) -> MenuLayer {
    let mut removed = ed.state.input.truncate(r);
    let Some(menu) = removed.pop().and_then(|l| l.downcast::<MenuLayer>()) else {
        unreachable!("dispatch_at already checked kind(r) == MenuLayer");
    };
    *menu
}
