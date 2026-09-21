//! The completion overlay — pushed above `Insert` for an LSP session, or
//! above `Command` for a minibuffer one. One `CompletionLayer` type serves
//! both; `dispatch_at` (`mappings/mod.rs`) routes into whichever key
//! handler `handler()` names, chosen by the session's own target — see
//! `completion/session.rs`'s `CompletionTarget` doc for what genuinely
//! differs between the two (the accept mechanism, further-typing behavior)
//! and what doesn't (this layer's own `Layer` impl, menu navigation).

use termina::event::{KeyCode, KeyEvent, Modifiers};

use hume_engine::pipeline::{EngineView, RenderContext};
use hume_engine::types::EditorMode;
use hume_rope::offset::ExclusiveRange;

use super::super::completion::{CompletionMenuUi, CompletionSession};
use super::super::event::EditorEvent;
use super::super::keymap::WalkResult;
use super::super::{Editor, EditorState, Severity};
use super::placement::popup_placement;
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, PopupEviction, Removal};

/// An open completion session, pushed above whichever base layer opened it
/// — an overlay, not a mode layer (`mode()` returns `None`;
/// `InputStack::mode_layer()` skips it, reading the mode from the layer
/// beneath). `ui` is `None` until the first Tab/Down/BackTab/Up moves the
/// selection off its implicit default of 0.
pub(in crate::editor) struct CompletionLayer {
    pub(in crate::editor) session: CompletionSession,
    pub(in crate::editor) ui: Option<CompletionMenuUi>,
}

impl Layer for CompletionLayer {
    fn handler(&self) -> LayerHandler {
        // The session's own target is the one axis further-typing behavior
        // follows — see `CompletionTarget`'s own doc (`completion/
        // session.rs`) for why a `Buffer` session always refilters in place
        // and a `Minibuf` one always cycles/dismisses.
        if self.session.minibuf_span().is_some() {
            completion_input_minibuf
        } else {
            completion_input_buffer
        }
    }
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    /// A `Completion` menu can land directly above a `Popup` (hover, the
    /// `gn`/`gp` diagnostic overlay — both non-modal, so `is_settled_for`
    /// doesn't treat one as the stack having moved) — `push_layer` evicts
    /// it on the way in, keeping `PopupLayer`'s "never buried" invariant
    /// true. `LayerOnly`, not the default `Both`: a completion session must
    /// coexist with a `Sticky` signature-help popup sitting in the same
    /// mode layer's slot — evicting `Both` here would silently kill sighelp
    /// on every completion open.
    fn popup_eviction(&self) -> PopupEviction {
        PopupEviction::LayerOnly
    }
    fn tear_down(&mut self, state: &mut EditorState, _view: &EngineView, _why: Removal) {
        // Both targets share this one layer type, but each renders into its
        // own slot (`Buffer` → `completion_menu`, `Minibuf` →
        // `minibuf_completion`) — clearing the wrong one leaves a stale
        // `PopupState` in the other until the next frame's sync silently
        // clears it for us (see `sync_minibuf_completion_view`'s own
        // is-open check), which is a lucky accident, not a guarantee.
        if self.session.minibuf_span().is_some() {
            state.views.minibuf_completion.set(None);
        } else {
            state.views.completion_menu.set(None);
        }
    }
}

impl Editor {
    /// Write the Insert-mode/LSP completion menu into the shared
    /// `PopupState` Arc — same widget as [`Self::sync_menu_view`]
    /// (unwrapped rows, selected-row styling), but anchored at the
    /// completion session's token-start char rather than the live cursor
    /// (which drifts as the user types further into the token).
    /// `Buffer`-target only — a `Minibuf`-target session renders through
    /// [`Editor::sync_minibuf_completion_view`] instead, into its own slot.
    ///
    /// Called every frame from `prepare_frame`'s step 10, same as
    /// [`Self::sync_popup_view`]/[`Self::sync_menu_view`] and for the same
    /// reason: it needs `EngineView::pane_rect`, which reads
    /// `last_pane_area` — only current after step 9 runs.
    pub(in crate::editor) fn sync_completion_menu_view(&mut self, ctx: &mut RenderContext) {
        let buffer_session_open = self
            .state
            .input
            .completion()
            .is_some_and(|s| s.buffer().is_some());
        if !buffer_session_open && self.state.views.completion_menu.read().is_none() {
            return;
        }

        // `bt.anchor()` is mapped forward through every edit `observe_edit`
        // was told about — but an edit that bypasses it entirely (an LSP
        // applyEdit, a file reload) or a pane switch can still leave it
        // pointing past the focused buffer's current end, or at a buffer
        // that isn't even the one on screen, in the narrow window before
        // `Editor::dismiss_invalid_completion`'s next settle-time pass
        // catches the mismatch (`scripting_setup.rs`). `DisplayLineMap::locate`
        // (reached via `popup_placement`) has no way to tell a stale offset
        // from a live one, so this fail-safe checks both here.
        //
        // Sequential borrows rather than one closure over
        // `self.state.input`: the session's shared borrow has to end
        // before `popup_placement` and `menu_rows` each take `&mut self`.
        let border = self.state.settings.popup_border;
        let resolved = (|| -> Option<hume_ui::popup::PopupState> {
            let session = self.state.input.completion()?;
            let bt = session.buffer()?;
            if bt.bid() != self.focused_buffer_id() {
                return None;
            }
            let anchor_char = bt.anchor();
            let len = self.state.buffers.get(bt.bid()).text().end();
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

/// The open completion session, but only if its token is `token` — the
/// guard `completion-add-items!` checks before reaching a `&mut
/// CompletionSession` at all. A mismatch, or no session open at all, is
/// expected-normal — a late async source racing a session the user already
/// replaced or dismissed — so callers treat `None` as a silent no-op, never
/// an error. Mirrors `picker::session_for_token`'s own doc and shape
/// exactly (`input_stack/picker/mod.rs`).
pub(in crate::editor) fn session_for_token(
    state: &mut EditorState,
    token: u64,
) -> Option<&mut CompletionSession> {
    state.input.completion_mut().filter(|s| s.token() == token)
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

    /// The completion session's UI selection, flattened (`Option<&
    /// CompletionMenuUi>`, not `Option<&mut Option<CompletionMenuUi>>`).
    /// Reads only; see [`Self::completion_ui_mut`] to assign or clear it.
    pub(in crate::editor) fn completion_ui(&self) -> Option<&CompletionMenuUi> {
        self.find::<CompletionLayer>().and_then(|l| l.ui.as_ref())
    }

    /// The `Completion` layer's UI slot itself (not its content) at `r`,
    /// unflattened so a caller can `get_or_insert` into it — see
    /// [`move_completion_selection`]'s own use. Address-based, not
    /// `find_mut`-based: both callers already know `r` from their own
    /// dispatch.
    pub(in crate::editor) fn completion_ui_mut(
        &mut self,
        r: LayerRef,
    ) -> Option<&mut Option<CompletionMenuUi>> {
        self.at_mut::<CompletionLayer>(r).map(|l| &mut l.ui)
    }
}

/// Handles one key while an Insert-mode/LSP completion session is open.
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
fn completion_input_buffer(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
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
        .at::<CompletionLayer>(r)
        .is_some_and(|l| !l.session.is_empty());
    if non_empty {
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                move_completion_selection(ed, r, true);
                return;
            }
            KeyCode::BackTab | KeyCode::Up => {
                move_completion_selection(ed, r, false);
                return;
            }
            KeyCode::Enter => {
                accept_completion_selection(ed, r);
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
                    .at::<CompletionLayer>(r)
                    .expect("non_empty implies a session")
                    .session
                    .buffer()
                    .expect("completion_input_buffer is only reached by a Buffer-target session")
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
    refilter_lsp_completion_after_edit(ed, r, key);
}

/// Handles one key while a minibuffer completion session is open — always
/// cycle-and-apply: Tab/Shift-Tab move the selection *and* immediately
/// splice the newly-selected candidate into the minibuffer
/// (there is no separate accept step, unlike the Buffer-target's Enter);
/// every other key dismisses the popup first, then falls through
/// unchanged — the minibuffer's own always-eager-apply UX.
fn completion_input_minibuf(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let key = match ev {
        InputEvent::Key(key) => key,
        // Neither a paste nor a mouse click has a meaningful "cycle" or
        // "accept" reading here — dismiss and let the minibuffer's own
        // handler see it, same discipline as the Buffer-target's own
        // paste/mouse arms above.
        InputEvent::Paste(text) => {
            ed.state.dismiss_completion(&ed.view);
            ed.fall_through(r, InputEvent::Paste(text));
            return;
        }
        InputEvent::Mouse(mouse) => {
            ed.state.dismiss_completion(&ed.view);
            ed.fall_through(r, InputEvent::Mouse(mouse));
            return;
        }
    };
    match key.code {
        KeyCode::Tab => {
            move_completion_selection(ed, r, true);
            apply_selected_minibuf_candidate(ed, r);
            return;
        }
        KeyCode::BackTab => {
            move_completion_selection(ed, r, false);
            apply_selected_minibuf_candidate(ed, r);
            return;
        }
        KeyCode::Enter => {
            // If the selected candidate is a directory (trailing `/`),
            // Enter descends into it instead of confirming the command
            // line: the candidate is already in the input (Tab applied
            // it), so dismiss this session and restart completion for the
            // directory's children, rather than falling through to
            // `Command`'s own Confirm handling.
            let selected = ed
                .state
                .input
                .at::<CompletionLayer>(r)
                .and_then(|l| l.ui.as_ref())
                .map_or(0, |ui| ui.selected);
            let is_dir = ed
                .state
                .input
                .at::<CompletionLayer>(r)
                .and_then(|l| l.session.selected_item(selected))
                .is_some_and(|item| item.insert_text().ends_with('/'));
            if is_dir {
                ed.state.dismiss_completion(&ed.view);
                super::command::complete_minibuf(ed, false);
                return;
            }
        }
        _ => {}
    }
    ed.state.dismiss_completion(&ed.view);
    ed.fall_through(r, InputEvent::Key(key));
}

/// Splices the currently-selected candidate's `insert_text` into the
/// minibuffer over `span_start..cursor` — reading `cursor` fresh each call
/// is equivalent to (and simpler than) tracking "the previous candidate's
/// own span" separately, since nothing else can move the cursor between
/// Tab presses without having already dismissed the session first (any
/// other key does, via `completion_input_minibuf`'s fallthrough arm).
pub(in crate::editor) fn apply_selected_minibuf_candidate(ed: &mut Editor, r: LayerRef) {
    let Some(layer) = ed.state.input.at::<CompletionLayer>(r) else {
        return;
    };
    let Some(span) = layer.session.minibuf_span() else {
        return;
    };
    let selected = layer.ui.as_ref().map_or(0, |ui| ui.selected);
    let Some(item) = layer.session.selected_item(selected) else {
        return;
    };
    let insert_text = item.insert_text().to_owned();
    let Some(mb) = ed.state.input.minibuf_mut() else {
        return;
    };
    mb.splice(span, &insert_text);
    // The next apply (cycling to a different candidate) must replace what
    // this one just inserted, not the original pre-completion token.
    if let Some(layer) = ed.state.input.at_mut::<CompletionLayer>(r) {
        layer.session.note_minibuf_splice(insert_text.len());
    }
}

/// Moves the completion menu's selection by one row. The popup scrolls
/// to keep the selection visible, so the bound is the full ranked
/// candidate list, not just the visible window. Shared by both targets —
/// already target-agnostic. A no-op on an empty session — reachable for a
/// `Minibuf`-target session, whose key handler has no non-empty guard the
/// way `completion_input_buffer`'s does.
fn move_completion_selection(ed: &mut Editor, r: LayerRef, forward: bool) {
    let Some(layer) = ed.state.input.at::<CompletionLayer>(r) else {
        return;
    };
    let current = layer.ui.as_ref().map_or(0, |ui| ui.selected);
    let Some(next) = layer.session.step_selection(current, forward) else {
        return;
    };
    let Some(ui_slot) = ed.state.input.completion_ui_mut(r) else {
        return;
    };
    ui_slot
        .get_or_insert(CompletionMenuUi { selected: 0 })
        .selected = next;
}

/// Accepts the currently-selected completion item through the same
/// gen-checked edit path as `completion-accept!` — the session ends
/// either way (success or failure), matching `EditorHostImpl`'s own
/// `completion_accept`. `Buffer`-target only — a `Minibuf`-target session
/// has no separate accept step (see `completion_input_minibuf`'s doc).
fn accept_completion_selection(ed: &mut Editor, r: LayerRef) {
    let selected = ed
        .state
        .input
        .at::<CompletionLayer>(r)
        .and_then(|l| l.ui.as_ref())
        .map_or(0, |ui| ui.selected);
    let Some(session) = ed.state.take_completion_session(&ed.view) else {
        return;
    };
    if let Err(msg) = session.accept(&mut ed.state, &ed.view, &mut ed.lsp, selected) {
        ed.report(Severity::Error, msg);
    }
}

/// Re-ranks the open completion session against the token text between
/// its anchor and the current cursor — called after a printable char or
/// Backspace has already landed in the buffer. `Buffer`-target only.
fn refilter_lsp_completion_after_edit(ed: &mut Editor, r: LayerRef, key: KeyEvent) {
    let is_char =
        matches!(key.code, KeyCode::Char(_)) && !key.modifiers.contains(Modifiers::CONTROL);
    if !is_char && key.code != KeyCode::Backspace {
        return;
    }
    // Phase 1 — shared reads only: peek the anchor/bid without taking
    // the session, so no put-back is ever needed.
    let Some(layer) = ed.state.input.at::<CompletionLayer>(r) else {
        return;
    };
    let Some(bt) = layer.session.buffer() else {
        return; // can't happen — this handler is Buffer-target only
    };
    let anchor = bt.anchor();
    let bid = bt.bid();
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

    // Phase 2: re-borrow the session mutably to re-rank it.
    let Some(layer) = ed.state.input.at_mut::<CompletionLayer>(r) else {
        return; // can't happen (checked above), but never assume it
    };
    layer.session.update_filter(text.clone());
    let incomplete = layer.session.incomplete();

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
    ed.state.reset_completion_selection();
}
