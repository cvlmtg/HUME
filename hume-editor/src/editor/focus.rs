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

/// Move focus to `pid`, ending any open Insert session and any open paste
/// session first — the one production chokepoint every focus change goes
/// through (`commands::pane::close_focused_pane`/`split_pane_onto`,
/// `commands::jump::focus_in_direction`/`cmd_pane_focus_next`,
/// `tab::install_live`, `mouse::mouse_left_down`). Both teardowns read state
/// keyed on the *outgoing* pane (`end_insert_session`'s blank-line indent
/// trim, `commit_paste_session`'s focused-pane paste group) and so must run
/// — while `state.focus` still names that outgoing pane — before the
/// assignment below, not after: done later, they'd land on `pid`'s buffer
/// instead of the one actually being left.
///
/// The Insert-session half is usually already done by the time this runs —
/// `tab::take_live` and `commands::tab::close_tab` both call
/// `commands::end_insert_session_if_active` themselves before touching the
/// layout (see `take_live`'s doc), and this call is then a no-op past the
/// mode check. Kept here too rather than only at those two call sites,
/// since `focus_in_direction`/`cmd_pane_focus_next`/`mouse_left_down`
/// switch focus *within* a tab, where no layout displacement happens and no
/// earlier teardown has run.
///
/// `commit_paste_session` is otherwise reached only from the dispatch
/// pipeline (`step_paste_commit`), which every *keyboard* command passes
/// through. A tabline click (`mouse::tabline_click`) switches tabs by
/// calling `tab::switch_to_tab` directly, bypassing dispatch — this is the
/// only place left that closes that gap for it. It is timing-agnostic with
/// respect to the layout (it resolves the outgoing pane/buffer by id, never
/// through the layout tree), so — unlike the Insert-session half — it has
/// no matching earlier call in `take_live`/`close_tab`.
///
/// Both calls are no-ops past their own guard (mode check; `paste_group`
/// check) whenever nothing is open, so every writer above can route through
/// this unconditionally instead of repeating either check itself.
pub(in crate::editor) fn focus_pane(state: &mut EditorState, view: &EngineView, pid: PaneId) {
    super::commands::end_insert_session_if_active(state, view);
    state.commit_paste_session(view);
    state.focus.0 = pid;
}
