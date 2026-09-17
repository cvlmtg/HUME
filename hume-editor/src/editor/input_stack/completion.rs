//! The LSP completion-menu layer — an overlay pushed above `Insert`, never a
//! mode of its own. `dispatch_at` (`mappings/mod.rs`) routes into
//! [`completion_input`] by layer type.

use termina::event::{KeyCode, KeyEvent, Modifiers};

use hume_engine::pipeline::{EngineView, RenderContext};
use hume_engine::types::EditorMode;
use hume_rope::offset::ExclusiveRange;

use super::super::event::EditorEvent;
use super::super::keymap::WalkResult;
use super::super::lsp::completion::{CompletionMenuUi, CompletionSession};
use super::super::{Editor, EditorState, Severity};
use super::placement::popup_placement;
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef};

/// An open LSP completion session, pushed above `Insert` — an overlay, not a
/// mode layer (`mode()` returns `None`; `InputStack::mode_layer()` skips it,
/// reading `Insert` from the layer beneath). `ui` is `None` until the first
/// Tab/Down/BackTab/Up moves the selection off its implicit default of 0.
pub(in crate::editor) struct CompletionLayer {
    pub(in crate::editor) session: CompletionSession,
    pub(in crate::editor) ui: Option<CompletionMenuUi>,
}

impl Layer for CompletionLayer {
    fn handler(&self) -> LayerHandler {
        completion_input
    }
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    /// A `Completion` menu can land directly above a `Popup` (hover, the
    /// `gn`/`gp` diagnostic overlay — both non-modal, so `is_stack_settled`
    /// doesn't treat one as the stack having moved) — clear it first,
    /// keeping `PopupLayer`'s "never buried" invariant true. Evicts only
    /// the *pushed-layer* popup home (`InputStack::clear_popup_layer`), not
    /// `clear_popups`: a completion session must coexist with a `Sticky`
    /// signature-help popup sitting in the same mode layer's slot — a bare
    /// `clear_popups()` here would silently kill sighelp on every
    /// completion open.
    fn setup(&mut self, state: &mut EditorState, _view: &EngineView) {
        state.input.clear_popup_layer();
    }
    fn tear_down(&mut self, state: &mut EditorState, _view: &EngineView) {
        state.views.completion_menu.set(None);
    }
}

impl Editor {
    /// Write the LSP completion menu into the shared `PopupState` Arc —
    /// same widget as [`Self::sync_menu_view`] (unwrapped rows,
    /// selected-row styling), but anchored at the completion session's
    /// token-start char rather than the live cursor (which drifts as the
    /// user types further into the token). Called every frame from
    /// `prepare_frame`'s step 10, same as [`Self::sync_popup_view`]/
    /// [`Self::sync_menu_view`] and for the same reason: it needs
    /// `EngineView::pane_rect`, which reads `last_pane_area` — only current
    /// after step 9 runs.
    pub(in crate::editor) fn sync_completion_menu_view(&mut self, ctx: &mut RenderContext) {
        if self.state.input.completion().is_none()
            && self.state.views.completion_menu.read().is_none()
        {
            return;
        }

        // `session.anchor()` is a char offset captured when the session
        // began; it isn't remapped through edits, so an out-of-band shrink
        // (LSP applyEdit, file reload) or a pane switch since can leave it
        // pointing past the focused buffer's current end, or at a buffer
        // that isn't even the one on screen. `DisplayLineMap::locate` (reached via
        // `popup_placement`) has no way to tell a stale offset from a live
        // one, so check both here.
        //
        // Sequential borrows rather than one closure over
        // `self.state.input`: the session's shared borrow has to end
        // before `popup_placement` and `menu_rows` each take `&mut self`.
        let border = self.state.settings.popup_border;
        let resolved = (|| -> Option<hume_ui::popup::PopupState> {
            let session = self.state.input.completion()?;
            if session.bid() != self.focused_buffer_id() {
                return None;
            }
            let anchor_char = session.anchor();
            let len = self.state.buffers.get(session.bid()).text().end();
            if anchor_char >= len {
                return None;
            }
            let placement = popup_placement(self, ctx, anchor_char)?;

            let selected_idx = self.state.input.completion_ui().map_or(0, |ui| ui.selected);
            let session = self.state.input.completion_mut()?;
            let rows = session.menu_rows();
            Some(hume_ui::popup::resolve_menu(
                rows,
                selected_idx,
                placement,
                border,
            ))
        })();

        self.state.views.completion_menu.set(resolved);
    }
}

/// Named sugar over the generic lookup — the ~350 existing call sites
/// (`ed.state.input.completion()`) stay as they are, and `stack.rs` stays
/// agnostic.
impl super::stack::InputStack {
    pub(in crate::editor) fn completion(&self) -> Option<&CompletionSession> {
        self.find::<CompletionLayer>().map(|l| &l.session)
    }

    pub(in crate::editor) fn completion_mut(&mut self) -> Option<&mut CompletionSession> {
        self.find_mut::<CompletionLayer>().map(|l| &mut l.session)
    }

    /// The completion session's UI selection, flattened — same shape as
    /// [`Self::minibuf_completion`]. Reads only; see
    /// [`Self::completion_ui_mut`] to assign or clear it.
    pub(in crate::editor) fn completion_ui(&self) -> Option<&CompletionMenuUi> {
        self.find::<CompletionLayer>().and_then(|l| l.ui.as_ref())
    }

    /// The `Completion` layer's UI slot itself (not its content) — same
    /// shape as [`Self::minibuf_completion_mut`], for
    /// [`move_completion_selection`]'s `get_or_insert`.
    pub(in crate::editor) fn completion_ui_mut(&mut self) -> Option<&mut Option<CompletionMenuUi>> {
        self.find_mut::<CompletionLayer>().map(|l| &mut l.ui)
    }
}

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
pub(in crate::editor) fn completion_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let key = match ev {
        InputEvent::Key(key) => key,
        // A paste bypasses per-char refiltering the same way an
        // Insert-mode paste bypasses trigger-char hooks (see
        // `Editor::apply_insert_mode_paste`'s doc) — dismiss the stale
        // session, then let `Insert` insert the text.
        InputEvent::Paste(text) => {
            ed.state.dismiss_completion(&ed.view);
            ed.fall_through(r, InputEvent::Paste(text));
            return;
        }
        // A mouse event has no token position to refilter against — it
        // falls straight through to `Insert`, which either moves the
        // cursor within the buffer (no post-step to run: unlike a key,
        // there's nothing here to re-check the session's anchor against)
        // or, via `focus_pane`, ends Insert outright and takes this
        // layer with it.
        InputEvent::Mouse(mouse) => {
            ed.fall_through(r, InputEvent::Mouse(mouse));
            return;
        }
    };
    let non_empty = ed
        .state
        .input
        .completion()
        .is_some_and(|session| !session.is_empty());
    if non_empty {
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                move_completion_selection(ed, true);
                return;
            }
            KeyCode::BackTab | KeyCode::Up => {
                move_completion_selection(ed, false);
                return;
            }
            KeyCode::Enter => {
                accept_completion_selection(ed);
                return;
            }
            KeyCode::Escape => {
                ed.state.dismiss_completion(&ed.view);
                return;
            }
            KeyCode::Backspace => {
                // The char Backspace is about to delete is the one right
                // before `head`. If `head` is already at (or before) the
                // anchor, that char lies *outside* the completed token —
                // crossing it, not just narrowing the filter. The
                // deletion itself still falls through below.
                let head = ed.current_selections().primary().head();
                let anchor = ed
                    .state
                    .input
                    .completion()
                    .expect("non_empty implies a session")
                    .anchor();
                if head <= anchor {
                    ed.state.dismiss_completion(&ed.view);
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
        ed.state.config.keymap.insert.walk(&[key]),
        WalkResult::Leaf(_)
    );
    ed.fall_through(r, InputEvent::Key(key));
    // The callee may have taken this layer with it — `apply_insert_edit`
    // dismissing on a stale `ChangeSet`, or Insert itself exiting and
    // tearing this layer down as part of the same truncate. `r`'s id
    // check catches both: skip rather than write through a stale ref.
    if !ed.state.input.is_live(r) {
        return;
    }
    if is_command {
        ed.state.dismiss_completion(&ed.view);
        return;
    }
    refilter_lsp_completion_after_edit(ed, key);
}

/// Moves the completion menu's selection by one row. The popup scrolls
/// to keep the selection visible, so the bound is the full ranked
/// candidate list, not just the visible window.
fn move_completion_selection(ed: &mut Editor, forward: bool) {
    let Some(session) = ed.state.input.completion() else {
        return;
    };
    // `completion_input`'s empty-session guard already returned before
    // dispatching here, so `n` is always positive.
    let n = session.len();
    let Some(ui_slot) = ed.state.input.completion_ui_mut() else {
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
fn accept_completion_selection(ed: &mut Editor) {
    let selected = ed.state.input.completion_ui().map_or(0, |ui| ui.selected);
    let Some(session) = ed.state.take_completion_session(&ed.view) else {
        return;
    };
    if let Err(msg) = session.accept(&mut ed.state, &mut ed.lsp, selected) {
        ed.report(Severity::Error, msg);
    }
}

/// Re-ranks the open completion session against the token text between
/// its anchor and the current cursor — called after a printable char or
/// Backspace has already landed in the buffer.
fn refilter_lsp_completion_after_edit(ed: &mut Editor, key: KeyEvent) {
    let is_char =
        matches!(key.code, KeyCode::Char(_)) && !key.modifiers.contains(Modifiers::CONTROL);
    if !is_char && key.code != KeyCode::Backspace {
        return;
    }
    // Phase 1 — shared reads only: peek the anchor/bid without taking
    // the session, so no put-back is ever needed.
    let Some(session) = ed.state.input.completion() else {
        return;
    };
    let anchor = session.anchor();
    let bid = session.bid();
    let head = ed.current_selections().primary().head();
    // Backspace crossing the anchor already dismissed the session
    // above, before the edit ran. But `head` can still land before
    // `anchor` here — e.g. a Steel hook mutating selections mid-
    // session, or any other out-of-band cursor move that doesn't route
    // through this layer's own dismissal. Dismiss rather than slice
    // with an inverted or out-of-range span.
    let len = ed.doc().text().end();
    if head < anchor || head > len {
        ed.state.dismiss_completion(&ed.view);
        return;
    }
    let text = ed
        .doc()
        .text()
        .slice(ExclusiveRange::new(anchor, head))
        .to_string();

    // Phase 2 — disjoint-field reads: `text_gen` comes from
    // `state.buffers`, read *before* borrowing `state.input`'s session
    // mutably — `state.buffers` and `state.input` are sibling fields, but
    // the session lives inside `state.input`, so a `&mut` on it and a
    // second borrow of `state` as a whole cannot coexist.
    let text_gen = ed.state.buffers.get(bid).text_gen;
    let Some(session) = ed.state.input.completion_mut() else {
        return; // can't happen (checked above), but never assume it
    };
    session.update_filter(text_gen, text.clone());
    let incomplete = session.incomplete();

    // Phase 3 — borrows from phase 2 have ended; back to whole-`ed`.
    // `on-completion-refilter` fires only while the server said
    // `isIncomplete` — a complete list needs no re-request, so a normal
    // session stays hook-silent on every keystroke.
    if incomplete {
        ed.state.queue_event(EditorEvent::OnCompletionRefilter {
            buffer: bid,
            filter_text: text,
        });
    }
    if let Some(slot) = ed.state.input.completion_ui_mut() {
        *slot = None;
    }
}
