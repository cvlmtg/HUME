//! Every change of the focused pane (the pane that owns the terminal
//! cursor, the live buffer, and any open Insert session), and of the buffer
//! a pane shows. The engine stores the focused pane with the active tab's
//! layout, so it is always a pane of that layout; every other tab's focused
//! pane is stashed on `EditorState::tabs`.
//!
//! The engine methods that move focus or change a pane's buffer take the
//! [`FocusKey`], which [`Focus`] keeps private to this module, so these
//! functions are the only way to make either change. Each runs the
//! Insert/paste session teardown the change needs, which would otherwise
//! leave a session pointed at a pane the user has left.

use hume_core::{BufferId, PaneId};
use hume_engine::pipeline::{
    DetachedPane, EngineView, FocusKey, FocusedLayout, SinglePane, UnattachedPane,
};

use super::EditorState;

/// The [`FocusKey`], private to this module.
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

/// End the open Insert, paste or replay session, if any, so a command that
/// moves focus or retargets the focused pane's buffer leaves Insert mode.
/// Safe to call with nothing open.
///
/// Both teardowns act on the `(pane, buffer)` the session recorded, not on
/// current focus, so they do not depend on running before the focus write.
/// Callers still run this first: a pane close drops the pane's state right
/// after, and a reload's `truncate_to_base` drops layers without running
/// their teardown.
pub(in crate::editor) fn end_focus_sessions(state: &mut EditorState, view: &EngineView) {
    super::commands::end_insert_session(state, view);
    super::edit_session::commit_paste_group(
        &mut state.buffers,
        &state.panes.entries,
        &mut state.paste_session,
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
/// Saves the pane's scroll for the buffer it leaves, settled first as the next
/// frame would settle it (`frame::settle_pane`), restores `target`'s saved scroll
/// (zero on first visit), and seeds the pane's state for `target` if it has
/// never shown `target` before. The first pane to show `target` owns its
/// restored selection history.
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
    super::frame::settle_pane(state, ev, pid);
    #[allow(clippy::disallowed_methods)] // settled above, so it saves the view a frame would show
    ev.show_buffer(&state.focus.0, pid, target, last_content_line);
    if is_focused_switch {
        state.buffers.touch_mru(target);
    }
    state.buffers.get_mut(target).claim_restored_history(pid);
    // `frame::reveal_due_cursor` indexes the pane's state for `target`
    // directly, so it must exist from the first visit.
    super::pane_state::ensure(
        &mut state.panes.entries,
        &state.buffers,
        &ev.panes,
        pid,
        target,
    );
}

/// Move focus to `pid`, a pane of the active layout, ending any open session
/// first.
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
/// itself, not when `Editor::detect_buffer_enter` next observes it:
/// `goto-alternate-buffer` reads `mru`'s tail as the buffer on screen, and a
/// command chained after the move in the same dispatch would otherwise read
/// the previous one.
fn touch_focused_mru(state: &mut EditorState, view: &EngineView) {
    state
        .buffers
        .touch_mru(view.panes[view.focused_pane()].buffer_id());
}
