//! Input handling for the fuzzy-picker layer, plus the query/close helpers
//! it needs. The last resident of what was originally a five-layer file:
//! `confirm`/`menu`/`drawer`/`popup` moved into their own
//! `input_stack/{confirm,menu,drawer,popup}.rs` files; `picker` follows once
//! `editor/picker.rs` itself relocates into `input_stack/picker/` too.
//! `dispatch_at` (in `mod.rs`) still calls [`Editor::picker_input`] directly.

use termina::event::{KeyCode, Modifiers};

use super::super::Editor;
use super::super::input_stack::{InputEvent, LayerRef};

impl Editor {
    /// Handles one key while the picker is open. Always fully consumes —
    /// unlike the menu (a stray key closes it and falls through) or the
    /// drawer (a stray key falls through untouched), the picker is
    /// full-modal: it owns the entire interaction, so an unrecognized key
    /// (Left/Right/Home/Tab/…) is simply consumed and ignored rather than
    /// leaking through to whatever mode sits underneath. `r` is unused —
    /// every retirement path goes through `picker::close_picker[_with]`,
    /// which finds the picker's own ref itself rather than needing this
    /// call's.
    ///
    /// `on_select` fires exactly once via `.take()`-equivalent truncate +
    /// `queue_steel_call` (never invoked inline) — same one-shot discipline
    /// as the menu. `visible_rows` comes from `panel_geometry` against the
    /// same `last_pane_area` the next frame's `sync_picker_view` will use,
    /// so a keystroke and the following paint always agree on how many rows
    /// are visible (before the first frame, geometry is `None` and paging is
    /// a documented no-op on the store). A mouse event — click, drag, or
    /// wheel — is swallowed the same way: full-modal means the picker owns
    /// the pointer too, not just the keyboard, so a click can't move the
    /// cursor in the buffer underneath it.
    pub(super) fn picker_input(&mut self, _r: LayerRef, ev: InputEvent) {
        let key = match ev {
            InputEvent::Key(key) => key,
            // Flattened to one line, same as a minibuffer paste, then
            // appended to the query in one bulk mutation — one rerank, at
            // most one `on_query_change` callback, instead of one per
            // pasted char.
            InputEvent::Paste(text) => {
                let cb = self
                    .picker_mut()
                    .insert_str(&super::bracketed_paste::flatten_single_line(&text));
                self.queue_query_change(cb);
                return;
            }
            InputEvent::Mouse(_) => return,
        };
        let visible_rows = hume_ui::picker_panel::panel_geometry(self.view.last_pane_area)
            .map_or(0, |geo| geo.list_rows);

        // Every movement key differs only in the delta passed to
        // `move_selection` — collapsed to one borrow instead of one per key.
        let step: Option<isize> = match key.code {
            KeyCode::Down => Some(1),
            KeyCode::Up => Some(-1),
            KeyCode::Char('n') if key.modifiers.contains(Modifiers::CONTROL) => Some(1),
            KeyCode::Char('p') if key.modifiers.contains(Modifiers::CONTROL) => Some(-1),
            KeyCode::PageDown => Some(visible_rows as isize),
            KeyCode::PageUp => Some(-(visible_rows as isize)),
            // `div_ceil`, not the `(visible_rows / 2).max(1)` the drawer and
            // `cmd_half_page_*` use, so this is `0` when `visible_rows` is `0` —
            // keeping paging a documented no-op before the first frame.
            KeyCode::Char('d') if key.modifiers.contains(Modifiers::CONTROL) => {
                Some(visible_rows.div_ceil(2) as isize)
            }
            KeyCode::Char('u') if key.modifiers.contains(Modifiers::CONTROL) => {
                Some(-(visible_rows.div_ceil(2) as isize))
            }
            _ => None,
        };
        if let Some(delta) = step {
            self.picker_mut().move_selection(delta, visible_rows);
            return;
        }

        match key.code {
            KeyCode::Backspace => {
                // `None` (and so no fire) both on an already-empty query and
                // on a non-live session — see `pop_grapheme`'s doc.
                let cb = self.picker_mut().pop_grapheme();
                self.queue_query_change(cb);
            }
            KeyCode::Enter => {
                // No match (or nothing pushed yet) behaves like Esc — Enter
                // is always a terminal action, never a silent no-op.
                self.close_picker_with_selection(None);
            }
            KeyCode::Escape => {
                super::super::picker::close_picker(
                    &mut self.state,
                    steel::rvals::SteelVal::BoolV(false),
                );
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(Modifiers::CONTROL | Modifiers::ALT) =>
            {
                let cb = self.picker_mut().insert_char(ch);
                self.queue_query_change(cb);
            }
            _ => {
                // `#:actions` — tried only here, after every built-in key
                // above has already had first refusal, so a declared action
                // can never override movement/Backspace/Enter/Escape/query
                // input (see `PickerSession::action_for`'s doc).
                if let Some(proc) = self.picker_mut().action_for(key).cloned() {
                    self.close_picker_with_selection(Some(proc));
                }
            }
        }
    }

    /// Queues `cb` — the `on_query_change` callback a `PickerSession` query
    /// mutator (`insert_char`/`pop_grapheme`) just returned, if any — with
    /// `(token query)` as it stands right now, via `queue_steel_call`,
    /// never invoked inline, same discipline as every other picker callback
    /// (`close_picker`). The token lets `live-picker!`'s Scheme wrapper
    /// (`bootstrap.scm`) spawn/stop/replace against the right session
    /// without closing over a not-yet-bound `define` — see
    /// `builtins/mod.rs`'s `picker!`/`live-picker!` rationale block. A bare
    /// `None` is a silent no-op: the mutator itself already decided there
    /// was nothing to fire (a `picker!` session, or a backspace on an
    /// already-empty query). Debouncing (a query changes on every
    /// keystroke, but a live source shouldn't re-spawn on every one) is
    /// composed once inside `live-picker!`'s own wrapper, not hand-wired by
    /// every plugin author — keystrokes are human-rate, unlike
    /// `debounce_viewport_change`'s fire site (`timer_bridge.rs`), which is
    /// every scroll step of every frame and so earns its own Rust-side
    /// coalescer.
    fn queue_query_change(&mut self, cb: Option<steel::rvals::SteelVal>) {
        let Some(cb) = cb else {
            return;
        };
        let session = self.picker_mut();
        let token = session.token();
        let query = session.query().to_string();
        self.state.queue_steel_call(
            cb,
            vec![
                steel::rvals::SteelVal::IntV(token as isize),
                steel::rvals::SteelVal::StringV(query.into()),
            ],
        );
    }

    /// The open picker session — only ever called from `picker_input`, which
    /// `dispatch_at` only reaches while a `Picker` layer is on top.
    fn picker_mut(&mut self) -> &mut super::super::picker::PickerSession {
        self.state
            .input
            .picker_mut()
            .expect("picker_input is only called while a Picker layer is on top of the stack")
    }

    /// Close the picker, firing `callback` (or `on_select` when `None`) with
    /// the selected payload — `#f` when nothing matches, so accepting is
    /// always a terminal action rather than a silent no-op.
    fn close_picker_with_selection(&mut self, callback: Option<steel::rvals::SteelVal>) {
        let payload = self
            .picker_mut()
            .selected_payload()
            .cloned()
            .unwrap_or(steel::rvals::SteelVal::BoolV(false));
        super::super::picker::close_picker_with(&mut self.state, callback, payload);
    }
}
