//! Key handling for an open LSP completion session — an overlay layer
//! pushed above `Insert`, never a mode of its own. `dispatch_at` (`mod.rs`)
//! routes into [`Editor::completion_input`] by layer kind.

use termina::event::{KeyCode, KeyEvent, Modifiers};

use super::super::event::EditorEvent;
use super::super::input_stack::{InputEvent, LayerRef};
use super::super::keymap::WalkResult;
use super::super::lsp::completion::CompletionMenuUi;
use super::super::{Editor, Severity};
use hume_rope::offset::ExclusiveRange;

impl Editor {
    /// Handles one key while a completion session is open.
    /// Tab/Down/BackTab/Up/Enter/Esc are claimed only while the session has
    /// at least one visible match — a session narrowed to empty (continued
    /// typing, or an `isIncomplete` list awaiting an async re-request) shows
    /// no menu, so nothing here should intercept a key: Esc must leave
    /// Insert in one press and Enter must insert a newline, exactly as if no
    /// session were open. Every other key resolves through the Insert
    /// keymap: a bound command (a motion, or an edit command that bypasses
    /// `apply_insert_edit`, the one chokepoint keeping the anchor in sync)
    /// dismisses the session outright; anything else falls through and is
    /// resynced against the buffer's new state once the edit lands.
    pub(super) fn completion_input(&mut self, r: LayerRef, ev: InputEvent) {
        let InputEvent::Key(key) = ev;
        let non_empty = self
            .state
            .input
            .completion()
            .is_some_and(|session| !session.is_empty());
        if non_empty {
            match key.code {
                KeyCode::Tab | KeyCode::Down => {
                    self.move_completion_selection(true);
                    return;
                }
                KeyCode::BackTab | KeyCode::Up => {
                    self.move_completion_selection(false);
                    return;
                }
                KeyCode::Enter => {
                    self.accept_completion_selection();
                    return;
                }
                KeyCode::Escape => {
                    self.state.dismiss_completion(&self.view);
                    return;
                }
                KeyCode::Backspace => {
                    // The char Backspace is about to delete is the one right
                    // before `head`. If `head` is already at (or before) the
                    // anchor, that char lies *outside* the completed token —
                    // crossing it, not just narrowing the filter. The
                    // deletion itself still falls through below.
                    let head = self.current_selections().primary().head();
                    let anchor = self
                        .state
                        .input
                        .completion()
                        .expect("non_empty implies a session")
                        .anchor();
                    if head <= anchor {
                        self.state.dismiss_completion(&self.view);
                    }
                }
                _ => {}
            }
        }
        // Every key bound in the Insert keymap resolves to a cursor motion
        // or an edit command, neither of which can keep the session's
        // anchor correctly tracked — walked before delegating, since the
        // command itself may reload the keymap.
        let is_command = matches!(
            self.state.config.keymap.insert.walk(&[key]),
            WalkResult::Leaf(_)
        );
        self.fall_through(r, InputEvent::Key(key));
        // The callee may have taken this layer with it — `apply_insert_edit`
        // dismissing on a stale `ChangeSet`, or Insert itself exiting and
        // tearing this layer down as part of the same truncate. `r`'s id
        // check catches both: skip rather than write through a stale ref.
        if !self.state.input.is_live(r) {
            return;
        }
        if is_command {
            self.state.dismiss_completion(&self.view);
            return;
        }
        self.refilter_lsp_completion_after_edit(key);
    }

    /// Moves the completion menu's selection by one row. The popup scrolls
    /// to keep the selection visible, so the bound is the full ranked
    /// candidate list, not just the visible window.
    fn move_completion_selection(&mut self, forward: bool) {
        let Some(session) = self.state.input.completion() else {
            return;
        };
        // `completion_input`'s empty-session guard already returned before
        // dispatching here, so `n` is always positive.
        let n = session.len();
        let Some(ui_slot) = self.state.input.completion_ui_mut() else {
            return;
        };
        let ui = ui_slot.get_or_insert(CompletionMenuUi { selected: 0 });
        if forward {
            ui.selected = (ui.selected + 1) % n;
        } else {
            ui.selected = ui.selected.checked_sub(1).unwrap_or(n - 1);
        }
    }

    /// Accepts the currently-selected completion item through the same
    /// gen-checked edit path as `completion-accept!` — the session ends
    /// either way (success or failure), matching `EditorHostImpl`'s own
    /// `completion_accept`.
    fn accept_completion_selection(&mut self) {
        let selected = self.state.input.completion_ui().map_or(0, |ui| ui.selected);
        let Some(session) = self.state.take_completion_session(&self.view) else {
            return;
        };
        if let Err(msg) = session.accept(&mut self.state, &mut self.lsp, selected) {
            self.report(Severity::Error, msg);
        }
    }

    /// Re-ranks the open completion session against the token text between
    /// its anchor and the current cursor — called after a printable char or
    /// Backspace has already landed in the buffer.
    fn refilter_lsp_completion_after_edit(&mut self, key: KeyEvent) {
        let is_char =
            matches!(key.code, KeyCode::Char(_)) && !key.modifiers.contains(Modifiers::CONTROL);
        if !is_char && key.code != KeyCode::Backspace {
            return;
        }
        // Phase 1 — shared reads only: peek the anchor/bid without taking
        // the session, so no put-back is ever needed.
        let Some(session) = self.state.input.completion() else {
            return;
        };
        let anchor = session.anchor();
        let bid = session.bid();
        let head = self.current_selections().primary().head();
        // Backspace crossing the anchor already dismissed the session
        // above, before the edit ran. But `head` can still land before
        // `anchor` here — e.g. a Steel hook mutating selections mid-
        // session, or any other out-of-band cursor move that doesn't route
        // through this layer's own dismissal. Dismiss rather than slice
        // with an inverted or out-of-range span.
        let len = self.doc().text().end();
        if head < anchor || head > len {
            self.state.dismiss_completion(&self.view);
            return;
        }
        let text = self
            .doc()
            .text()
            .slice(ExclusiveRange::new(anchor, head))
            .to_string();

        // Phase 2 — disjoint-field reads: `text_gen` comes from
        // `state.buffers`, read *before* borrowing `state.input`'s session
        // mutably — `state.buffers` and `state.input` are sibling fields, but
        // the session lives inside `state.input`, so a `&mut` on it and a
        // second borrow of `state` as a whole cannot coexist.
        let text_gen = self.state.buffers.get(bid).text_gen;
        let Some(session) = self.state.input.completion_mut() else {
            return; // can't happen (checked above), but never assume it
        };
        session.update_filter(text_gen, text.clone());
        let incomplete = session.incomplete();

        // Phase 3 — borrows from phase 2 have ended; back to whole-`self`.
        // `on-completion-refilter` fires only while the server said
        // `isIncomplete` — a complete list needs no re-request, so a normal
        // session stays hook-silent on every keystroke.
        if incomplete {
            self.state.queue_event(EditorEvent::OnCompletionRefilter {
                buffer: bid,
                filter_text: text,
            });
        }
        if let Some(slot) = self.state.input.completion_ui_mut() {
            *slot = None;
        }
    }
}
