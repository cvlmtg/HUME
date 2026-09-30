//! The completion overlay: [`BufferCompletionLayer`] pushed above `Insert`,
//! [`MinibufCompletionLayer`] pushed above `Command`. Two concrete layer
//! types, not one: `completion/session.rs`'s module doc explains why a
//! `Buffer`/`Minibuf` session are different types rather than one enum,
//! and the same split carries into their layers, so `dispatch_at`
//! (`mappings/mod.rs`) routes by ordinary generic layer lookup
//! (`find`/`at`/`ref_of`) rather than a runtime target check. What the two
//! layers' `Layer` impls share (`mode`, `popup_eviction`, `removal_scope`)
//! and menu navigation stay written once, in the free functions below, and
//! called from both.

use termina::event::KeyCode;

use hume_engine::pipeline::{EngineView, RenderContext};
use hume_engine::types::EditorMode;

use super::super::completion::{BufferSession, CompletionItem, MinibufSession};
use super::super::keymap::WalkResult;
use super::super::{Editor, EditorState, Severity};
use super::placement::popup_placement;
use super::stack::{
    InputEvent, Layer, LayerHandler, LayerRef, PopupEviction, Removal, RemovalScope,
};

/// An open Insert-mode completion session, pushed above whichever base
/// layer opened it: an overlay, not a mode layer (`mode()` returns `None`;
/// `InputStack::mode_layer()` skips it, reading the mode from the layer
/// beneath).
pub(in crate::editor) struct BufferCompletionLayer {
    pub(in crate::editor) session: BufferSession,
}

/// [`BufferCompletionLayer`]'s `:` command-line counterpart.
pub(in crate::editor) struct MinibufCompletionLayer {
    pub(in crate::editor) session: MinibufSession,
}

impl Layer for BufferCompletionLayer {
    fn snapshot_mut(&mut self) -> Option<&mut super::PaneSnapshot> {
        None
    }

    fn handler(&self) -> LayerHandler {
        completion_input_buffer
    }
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    /// A `Completion` menu can land directly above a `Popup` (hover, the
    /// `gn`/`gp` diagnostic overlay; both non-modal, so `is_settled_for`
    /// doesn't treat one as the stack having moved). `push_layer` evicts
    /// it on the way in, keeping `PopupLayer`'s "never buried" invariant
    /// true. `LayerOnly`, not the default `Both`: a completion session must
    /// coexist with a `Sticky` signature-help popup sitting in the same
    /// mode layer's slot; evicting `Both` here would silently kill sighelp
    /// on every completion open.
    fn popup_eviction(&self) -> PopupEviction {
        PopupEviction::LayerOnly
    }
    /// A non-modal `Popup` (hover, `gn`/`gp`, a `Sticky` signature-help
    /// popup) can land directly above a completion layer by design. See
    /// this type's own doc and `popup_eviction`'s `LayerOnly` override just
    /// above, which exists for the same coexistence. The default `Stack`
    /// scope would otherwise take that popup down along with the session on
    /// every dismiss, contradicting both.
    fn removal_scope(&self) -> RemovalScope {
        RemovalScope::SelfOnly
    }
    fn tear_down(&mut self, state: &mut EditorState, _view: &EngineView, _why: Removal) {
        state.views.completion_menu.set(None);
    }
}

impl Layer for MinibufCompletionLayer {
    fn snapshot_mut(&mut self) -> Option<&mut super::PaneSnapshot> {
        None
    }

    fn handler(&self) -> LayerHandler {
        completion_input_minibuf
    }
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    fn popup_eviction(&self) -> PopupEviction {
        PopupEviction::LayerOnly
    }
    fn removal_scope(&self) -> RemovalScope {
        RemovalScope::SelfOnly
    }
    fn tear_down(&mut self, state: &mut EditorState, _view: &EngineView, _why: Removal) {
        state.views.minibuf_completion.set(None);
    }
}

impl Editor {
    /// Write the Insert-mode completion menu into the shared `PopupState`
    /// Arc: same widget as [`Self::sync_menu_view`] (unwrapped rows,
    /// selected-row styling), but anchored at the leftmost contributing
    /// token's start rather than the live cursor (which drifts as the user
    /// types further into the token). `Buffer`-target only; a
    /// `Minibuf`-target session renders through
    /// [`Editor::sync_minibuf_completion_view`] instead, into its own slot.
    ///
    /// Called every frame from `prepare_frame`'s overlay sync, same as
    /// [`Self::sync_popup_view`]/[`Self::sync_menu_view`] and for the same
    /// reason: it needs `EngineView::pane_rect`, which reads
    /// `last_pane_area`, only current after step 9 runs.
    pub(in crate::editor) fn sync_completion_menu_view(&mut self, ctx: &mut RenderContext) {
        let buffer_session_open = self.state.input.buffer_completion().is_some();
        if !buffer_session_open && self.state.views.completion_menu.read().is_none() {
            return;
        }

        // An edit that bypasses the session (an LSP applyEdit, a file
        // reload) or a pane switch can leave the anchor behind the text, or
        // the session on a buffer that isn't the one on screen, in the narrow
        // window before the next settle-time `reconcile_completion` pass
        // catches the mismatch (`scripting_setup.rs`). The
        // anchor reads as absent against a text it was not ranked on, and the
        // buffer check below covers the other case.
        //
        // Sequential borrows rather than one closure over
        // `self.state.input`: the session's shared borrow has to end
        // before `popup_placement` takes `&mut self`.
        let border = self.state.settings.popup_border;
        let resolved = (|| -> Option<hume_ui::popup::PopupState> {
            let session = self.state.input.buffer_completion()?;
            if session.bid() != self.focused_buffer_id() {
                return None;
            }
            let anchor = session.menu_anchor(self.state.buffers.get(session.bid()).text())?;
            let placement = popup_placement(self, ctx, anchor)?;

            let selected_idx = self.state.input.completion_selected();
            let session = self.state.input.buffer_completion()?;
            let window =
                hume_ui::popup::menu_window(session.len(), selected_idx, placement.pane_rect);
            let rows = session.rows_in(window.range.clone());
            Some(hume_ui::popup::resolve_menu(
                &rows, window, placement, border,
            ))
        })();

        self.state.views.completion_menu.set(resolved);
    }
}

/// Named sugar over the generic lookup; `stack.rs` stays agnostic.
impl super::stack::InputStack {
    pub(in crate::editor) fn buffer_completion(&self) -> Option<&BufferSession> {
        self.find::<BufferCompletionLayer>().map(|l| &l.session)
    }

    pub(in crate::editor) fn buffer_completion_mut(&mut self) -> Option<&mut BufferSession> {
        self.find_mut::<BufferCompletionLayer>()
            .map(|l| &mut l.session)
    }

    pub(in crate::editor) fn minibuf_completion(&self) -> Option<&MinibufSession> {
        self.find::<MinibufCompletionLayer>().map(|l| &l.session)
    }

    pub(in crate::editor) fn minibuf_completion_mut(&mut self) -> Option<&mut MinibufSession> {
        self.find_mut::<MinibufCompletionLayer>()
            .map(|l| &mut l.session)
    }

    /// The open completion menu's selected row, defaulting to `0` when
    /// neither layer is open. The one accessor every caller that doesn't
    /// already hold the layer (via [`Self::at`]/[`Self::at_mut`]) should
    /// use.
    pub(in crate::editor) fn completion_selected(&self) -> usize {
        self.buffer_completion()
            .map(BufferSession::selected)
            .or_else(|| self.minibuf_completion().map(MinibufSession::selected))
            .unwrap_or(0)
    }
}

/// Handles one key while an Insert-mode completion session is open.
/// Tab/Down/BackTab/Up/Enter/Esc are claimed only while the session has
/// at least one visible match. A session narrowed to empty (continued
/// typing, or a source still pending) shows no menu, so nothing here
/// should intercept a key: Esc must leave Insert in one press and Enter
/// must insert a newline, exactly as if no session were open. Every
/// other key resolves through the Insert keymap: a bound command (a
/// motion or an edit command) dismisses the session outright; anything
/// else falls through and the session is reconciled with the buffer's new
/// state.
fn completion_input_buffer(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let key = match ev {
        InputEvent::Key(key) => key,
        // A paste bypasses per-char refiltering the same way an
        // Insert-mode paste bypasses trigger-char hooks (see
        // `Editor::apply_insert_mode_paste`'s doc), so dismiss the stale
        // session, then let `Insert` insert the text.
        InputEvent::Paste(text) => {
            ed.state.dismiss_completion(&ed.view);
            ed.fall_through(r, InputEvent::Paste(text));
            return;
        }
        // A click ends Insert outright via `focus_pane` and takes this layer
        // with it; any other mouse event leaves the cursor and text alone.
        InputEvent::Mouse(mouse) => {
            ed.fall_through(r, InputEvent::Mouse(mouse));
            return;
        }
    };
    let non_empty = ed
        .state
        .input
        .at::<BufferCompletionLayer>(r)
        .is_some_and(|l| !l.session.is_empty());
    if non_empty {
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                move_buffer_completion_selection(ed, r, true);
                return;
            }
            KeyCode::BackTab | KeyCode::Up => {
                move_buffer_completion_selection(ed, r, false);
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
            _ => {}
        }
    }
    // Every key bound in the Insert keymap resolves to a cursor motion
    // or an edit command, neither of which can keep the session's
    // tokens correctly tracked. Walked before delegating, since the
    // command itself may reload the keymap.
    let is_command = matches!(
        ed.state.config.keymap.insert.walk(&[key]),
        WalkResult::Leaf(_)
    );
    ed.fall_through(r, InputEvent::Key(key));
    // The callee may have taken this layer with it (a `completion-trigger`
    // re-pushing the session as a fresh layer, or Insert itself exiting and
    // tearing this layer down as part of the same truncate). `r`'s id check
    // catches both: skip rather than write through a stale ref.
    if !ed.state.input.is_live(r) {
        return;
    }
    if is_command {
        ed.state.dismiss_completion(&ed.view);
    } else {
        ed.state.reconcile_completion(&ed.view);
    }
}

/// Handles one key while a minibuffer completion session is open. Always
/// cycle-and-apply: Tab/Shift-Tab move the selection *and* immediately
/// splice the newly-selected candidate into the minibuffer
/// (there is no separate accept step, unlike the Buffer-target's Enter);
/// every other key dismisses the popup first, then falls through
/// unchanged: the minibuffer's own always-eager-apply UX.
fn completion_input_minibuf(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let key = match ev {
        InputEvent::Key(key) => key,
        // Neither a paste nor a mouse click has a meaningful "cycle" or
        // "accept" reading here, so dismiss and let the minibuffer's own
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
            move_minibuf_completion_selection(ed, r, true);
            ed.state.apply_minibuf_candidate(r);
            return;
        }
        KeyCode::BackTab => {
            move_minibuf_completion_selection(ed, r, false);
            ed.state.apply_minibuf_candidate(r);
            return;
        }
        KeyCode::Enter => {
            // If the selected candidate names a directory, Enter descends
            // into it instead of confirming the command line: the
            // candidate is already in the input (Tab applied it), so
            // dismiss this session and restart completion for the
            // directory's children, rather than falling through to
            // `Command`'s own Confirm handling. Gated on the item's own
            // declared kind, not its source's identity or its text: any
            // `'minibuf` source's item can opt in this way, and an
            // unrelated candidate that merely ends in `/` (a URL, a
            // namespaced tag) is never mistaken for one.
            let is_dir = ed
                .state
                .input
                .at::<MinibufCompletionLayer>(r)
                .is_some_and(|l| {
                    l.session
                        .selected_item(l.session.selected())
                        .is_some_and(CompletionItem::is_folder)
                });
            if is_dir {
                ed.state.dismiss_completion(&ed.view);
                ed.start_minibuf_completion();
                return;
            }
        }
        _ => {}
    }
    ed.state.dismiss_completion(&ed.view);
    ed.fall_through(r, InputEvent::Key(key));
}

/// Moves the Insert-mode completion menu's selection by one row. The popup
/// scrolls to keep the selection visible, so the bound is the full ranked
/// candidate list, not just the visible window.
fn move_buffer_completion_selection(ed: &mut Editor, r: LayerRef, forward: bool) {
    if let Some(layer) = ed.state.input.at_mut::<BufferCompletionLayer>(r) {
        layer.session.step_selection(forward);
    }
}

/// [`move_buffer_completion_selection`]'s `Minibuf` counterpart, a no-op
/// on an empty session, reachable here since this key handler has no
/// non-empty guard the way `completion_input_buffer`'s does.
fn move_minibuf_completion_selection(ed: &mut Editor, r: LayerRef, forward: bool) {
    if let Some(layer) = ed.state.input.at_mut::<MinibufCompletionLayer>(r) {
        layer.session.step_selection(forward);
    }
}

/// Accepts the currently-selected completion item through the same
/// gen-checked edit path as `completion-accept!`. The session ends
/// either way (success or failure), matching `EditorHostImpl`'s own
/// `completion_accept`. `Buffer`-target only; a `Minibuf`-target session
/// has no separate accept step (see `completion_input_minibuf`'s doc).
///
/// Wrapped in the same [`Editor::with_dot_capture`] `handle_insert` uses
/// around a bound key's own dispatch: this Enter keypress is outside any
/// Insert-key binding, so there's no `Binding` entry to fall back to
/// (`fallback: None`); `accept` itself always calls `EditorState::
/// mark_dot_interactive` once it gets far enough to commit, so a completion
/// accept is either interactive (recorded as its own net edit) or nothing
/// (an early `Err`, reported below, records no entry at all).
fn accept_completion_selection(ed: &mut Editor, r: LayerRef) {
    let selected = ed
        .state
        .input
        .at::<BufferCompletionLayer>(r)
        .map_or(0, |l| l.session.selected());
    let Some(session) = ed.state.take_buffer_completion(&ed.view) else {
        return;
    };

    ed.with_dot_capture(None, |ed| {
        if let Err(msg) = session.accept(&mut ed.state, &ed.view, &mut ed.lsp, selected) {
            ed.report(Severity::Error, msg);
        }
    });
}
