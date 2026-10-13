//! Every change of the focused pane (the pane that owns the terminal
//! cursor, the live buffer, and any open Insert session), and of the buffer
//! a pane shows. The engine stores the focused pane with the active tab's
//! layout (`EngineView::focused_pane`), so it is always a pane of that
//! layout. Every other open tab's own focused pane is stashed on
//! `EditorState::tabs` instead (see `tab::store`'s module doc).
//!
//! The engine methods that move focus or change a pane's buffer take the
//! [`FocusKey`], which [`Focus`] keeps in a field private to this module, so
//! the functions here are the only way to make either change. Each runs the
//! Insert/paste session teardown the change needs; skipping it would leave a
//! session open and pointed at a pane the user has already left (a custom
//! keybinding or a Steel hook driving a pane-focus command can trigger a
//! focus change from anywhere, so this can't be caught by auditing dispatch
//! call sites alone).

use hume_engine::pipeline::{
    BufferId, DetachedPane, EngineView, FocusKey, FocusedLayout, PaneId, SinglePane, UnattachedPane,
};

use super::{EditorState, doc_ops};

/// See this module's own doc.
#[derive(Debug)]
pub(in crate::editor) struct Focus(FocusKey);

impl Focus {
    /// Take `view`'s key and make `first` its sole, focused pane.
    pub(in crate::editor) fn new(
        view: &mut EngineView,
        key: FocusKey,
        first: UnattachedPane,
    ) -> Self {
        // Discards the placeholder `EngineView::new` seeded: no real pane
        // behind it to leak.
        let _ = view.replace_layout(&key, FocusedLayout::new(first));
        Self(key)
    }

    /// Test-only escape hatch for setup that must move focus directly,
    /// bypassing `focus_pane`'s session teardown. Panics on a pane outside
    /// the active layout.
    #[cfg(test)]
    pub(in crate::editor) fn set_for_test(&self, view: &mut EngineView, pid: PaneId) {
        view.focus(&self.0, pid);
    }
}

/// End the open Insert or paste session, if any: the shared teardown
/// [`focus_pane`] and every pane-buffer switch ([`switch_pane_to_buffer`],
/// `Editor::reset_config_state`'s reload) run, so
/// that any command which moves focus or retargets the focused pane's buffer
/// leaves Insert mode.
///
/// Both teardowns act on the `(pane, buffer)` the session itself recorded
/// (`EditorState::active_session`), never on current focus, so their
/// correctness does not depend on running before or after the focus write.
/// Callers still run this first for their own reasons: a pane close drops
/// the pane's state right after, and a reload's `truncate_to_base` drops
/// layers without running their teardown.
///
/// `end_insert_session` is a no-op past its own guard (no `Insert` layer
/// open) whenever nothing is open, so every caller can route through this
/// unconditionally instead of checking first. Whatever it leaves behind,
/// a Paste session or a `Replay`-kind `EditSession` (what `Editor::
/// replay_dot`'s own pre-open produces before its replayed body decides
/// what it actually needs, see `edit_session::open_or_retarget`'s doc),
/// commits the same way through
/// [`doc_ops::commit_open_session`]: this is the chokepoint every
/// focus/buffer-switch path already runs before removing the `(pane,
/// buffer)` state a later commit would need to read.
pub(in crate::editor) fn end_focus_sessions(state: &mut EditorState, view: &EngineView) {
    super::commands::end_insert_session(state, view);
    doc_ops::commit_open_session(
        &mut state.buffers,
        &state.panes.entries,
        &mut state.active_session,
    );
}

/// Redirect pane `pid` to `target` without recording a jump.
///
/// When `pid` is focused and `target` differs from its current buffer:
/// - ends any open Insert/paste session first ([`end_focus_sessions`]),
///   before the `buffer_id` write it reads through. A stale session would
///   make the next keystroke panic in `doc_ops::apply_doc_edit_grouped`.
/// - promotes `target` in `BufferStore.mru`. With [`focus_pane`], this keeps
///   `mru`'s tail equal to the buffer on screen.
///
/// Saves the pane's scroll for the old buffer, restores `target`'s saved scroll
/// (zero on first visit), and seeds the pane's state for `target` if it has
/// never shown `target` before.
pub(in crate::editor) fn switch_pane_to_buffer(
    state: &mut EditorState,
    ev: &mut EngineView,
    pid: PaneId,
    target: BufferId,
) {
    let is_focused_switch = pid == ev.focused_pane() && ev.panes[pid].buffer_id() != target;
    if is_focused_switch {
        end_focus_sessions(state, ev);
    }
    let last_content_line = state.buffers.get(target).text().last_content_line();
    ev.show_buffer(&state.focus.0, pid, target, last_content_line);
    if is_focused_switch {
        state.buffers.touch_mru(target);
    }
    // Seeds the pane's state for `target` on its first visit to
    // `target`. Required regardless of reveal, since `frame.rs`'s scroll
    // step indexes it directly. A different buffer's cursor/viewport pairing
    // always needs re-settling, whether or not the recalled selection's head
    // happens to equal the outgoing one: `EditorState::layout_key`'s
    // `buffer_tag` names the buffer, so a switch to a different one always
    // differs from `PaneBufferState::last_layout_key`: the very first read
    // for a `(pane, buffer)` pair is `None`, which differs from anything.
    super::pane_state::ensure(
        &mut state.panes.entries,
        &state.buffers,
        &ev.panes,
        pid,
        target,
    );
}

/// Move focus to `pid`, a pane of the active layout, first calling
/// [`end_focus_sessions`]. `focus_toward`/`cmd_pane_focus_next`/
/// `mouse_left_down` switch focus *within* a tab this way, where no layout
/// displacement happens and no earlier teardown has run.
pub(in crate::editor) fn focus_pane(state: &mut EditorState, view: &mut EngineView, pid: PaneId) {
    end_focus_sessions(state, view);
    view.focus(&state.focus.0, pid);
    touch_focused_mru(state, view);
}

/// Close the focused pane, moving focus to the pane that takes its place.
/// Refused, with nothing changed, when it is the active tab's only pane.
/// The caller drops the returned pane's state.
pub(in crate::editor) fn close_focused(
    state: &mut EditorState,
    view: &mut EngineView,
) -> Result<DetachedPane, SinglePane> {
    let detached = view.close_focused(&state.focus.0)?;
    end_focus_sessions(state, view);
    touch_focused_mru(state, view);
    Ok(detached)
}

/// Make `layout` the active tab's, returning the layout it replaces, after
/// ending the outgoing focused pane's sessions ([`end_focus_sessions`]).
#[must_use]
pub(in crate::editor) fn install_layout(
    state: &mut EditorState,
    view: &mut EngineView,
    layout: FocusedLayout,
) -> FocusedLayout {
    end_focus_sessions(state, view);
    let outgoing = view.replace_layout(&state.focus.0, layout);
    touch_focused_mru(state, view);
    outgoing
}

/// Promote the focused pane's buffer in `BufferStore.mru` at the focus move
/// itself, as `switch_pane_to_buffer` does for a buffer switch, rather than
/// when `Editor::detect_buffer_enter` next observes the change after
/// `settle()`. `goto-alternate-buffer` reads `mru`'s tail as the buffer on
/// screen, so a command chained after the focus move in the same dispatch
/// would otherwise read the previous one.
fn touch_focused_mru(state: &mut EditorState, view: &EngineView) {
    state
        .buffers
        .touch_mru(view.panes[view.focused_pane()].buffer_id());
}
