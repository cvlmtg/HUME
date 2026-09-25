//! The single pane that owns the terminal cursor, the live buffer, and any
//! open Insert session — the *active tab's* focused pane. Every other open
//! tab's own focused pane is stashed on `EditorState::tabs` instead (see
//! `tab::store`'s module doc).
//!
//! [`Focus`]'s field is private to this module, so [`focus_pane`] is the
//! only way to change it: a raw assignment would skip the Insert/paste
//! session teardown `focus_pane` does first, leaving a session open and
//! pointed at a pane the user has already left (a custom keybinding or a
//! Steel hook driving a pane-focus command can trigger a focus change from
//! anywhere, so this can't be caught by auditing dispatch call sites alone).

use hume_engine::pipeline::{EngineView, PaneId};

use super::EditorState;

/// See this module's own doc.
#[derive(Debug, Default, Clone, Copy)]
pub(in crate::editor) struct Focus(PaneId);

impl Focus {
    /// Seed the very first focus, before any command has had a chance to
    /// move it — `Editor::open`/`Editor::for_testing`'s own bootstrap, the
    /// only two legitimate callers of a *constructor* rather than
    /// `focus_pane`'s teardown-then-assign (there is no prior session to
    /// tear down before the first pane exists).
    pub(in crate::editor) fn new(pid: PaneId) -> Self {
        Self(pid)
    }

    pub(in crate::editor) fn id(&self) -> PaneId {
        self.0
    }

    /// Test-only escape hatch for setup that must seed focus directly,
    /// bypassing `focus_pane`'s session teardown — a fresh test editor has
    /// no session open yet, so there is nothing for it to end.
    #[cfg(test)]
    pub(in crate::editor) fn set_for_test(&mut self, pid: PaneId) {
        self.0 = pid;
    }
}

/// End any open Insert session and any open paste session on the *currently
/// focused* pane — the shared teardown [`focus_pane`] and every pane-buffer
/// switch (`buffer::lifecycle::switch_pane_to_buffer`, `Editor::
/// reset_config_state`'s reload) run before they move focus or swap the
/// focused pane's buffer out from under it. Both teardowns read state keyed
/// on the pane `state.focus` names *right now* (`end_insert_session`'s
/// blank-line indent trim, `commit_paste_session`'s focused-pane paste
/// group), so every caller must run this — while focus still names the pane
/// being left or the buffer being swapped out — before the change, not
/// after: done later, they'd land on the new target instead of the state
/// actually being torn down.
///
/// Both calls are no-ops past their own guard (no `Insert` layer open;
/// `paste_group` check) whenever nothing is open, so every caller can route
/// through this unconditionally instead of repeating either check itself.
pub(in crate::editor) fn end_focus_sessions(state: &mut EditorState, view: &EngineView) {
    super::commands::end_insert_session(state, view);
    state.commit_paste_session(view);
}

/// Move focus to `pid`, first calling [`end_focus_sessions`] — the one
/// production chokepoint every focus change goes through
/// (`commands::pane::close_focused_pane`/`split_pane_onto`,
/// `commands::jump::focus_in_direction`/`cmd_pane_focus_next`,
/// `tab::install_live`, `mouse::mouse_left_down`).
///
/// Usually already done by the time this runs — `tab::take_live` calls
/// `end_focus_sessions` itself before touching the layout (see its own
/// doc), and this call is then a no-op. Kept here too rather than only at
/// that one call site, since `focus_in_direction`/`cmd_pane_focus_next`/
/// `mouse_left_down` switch focus *within* a tab, where no layout
/// displacement happens and no earlier teardown has run. A tabline click
/// (`mouse::tabline_click`) switches tabs by calling `tab::switch_to_tab`
/// directly, bypassing dispatch (`step_paste_commit`'s own chokepoint) —
/// this is the only place left that closes that gap for it too.
pub(in crate::editor) fn focus_pane(state: &mut EditorState, view: &EngineView, pid: PaneId) {
    end_focus_sessions(state, view);
    state.focus.0 = pid;
    // The other of the two chokepoints `focused_buffer_id()` can change
    // through — see `switch_pane_to_buffer`'s own doc for why this pair
    // promotes `BufferStore.mru` synchronously instead of leaving it to
    // `Editor::detect_buffer_enter`'s later, settle()-gated observation.
    state.buffers.touch_mru(view.panes[pid].buffer_id);
}
