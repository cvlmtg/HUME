//! Every change of the focused pane: the pane that owns the terminal
//! cursor, the live buffer, and any open Insert session. The engine stores
//! it with the active tab's layout (`EngineView::focused_pane`), so it is
//! always a pane of that layout. Every other open tab's own focused pane is
//! stashed on `EditorState::tabs` instead (see `tab::store`'s module doc).
//!
//! The engine methods that move focus take the [`FocusKey`], which [`Focus`]
//! keeps in a field private to this module, so the functions here are the
//! only way to move it. Each runs the Insert/paste session teardown a focus
//! change needs; skipping it would leave a session open and pointed at a
//! pane the user has already left (a custom keybinding or a Steel hook
//! driving a pane-focus command can trigger a focus change from anywhere,
//! so this can't be caught by auditing dispatch call sites alone).

use hume_engine::pipeline::{
    DetachedPane, EngineView, FocusKey, FocusedLayout, PaneId, SinglePane,
};

use super::{EditorState, doc_ops};

/// See this module's own doc.
#[derive(Debug)]
pub(in crate::editor) struct Focus(FocusKey);

impl Focus {
    pub(in crate::editor) fn new(key: FocusKey) -> Self {
        Self(key)
    }

    /// Test-only escape hatch for setup that must move focus directly,
    /// bypassing `focus_pane`'s session teardown. Panics on a pane outside
    /// the active layout.
    #[cfg(test)]
    pub(in crate::editor) fn set_for_test(&self, view: &mut EngineView, pid: PaneId) {
        view.focus(&self.0, pid)
            .expect("test focus target is a pane of the active layout");
    }

    /// Test-only escape hatch for a hand-built layout, bypassing
    /// `install_layout`'s session teardown.
    #[cfg(test)]
    pub(in crate::editor) fn replace_layout_for_test(
        &self,
        view: &mut EngineView,
        layout: FocusedLayout,
    ) -> FocusedLayout {
        view.replace_layout(&self.0, layout)
    }
}

/// End the open Insert or paste session, if any: the shared teardown
/// [`focus_pane`] and every pane-buffer switch (`buffer::lifecycle::
/// switch_pane_to_buffer`, `Editor::reset_config_state`'s reload) run, so
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
        &state.panes.state,
        &mut state.active_session,
    );
}

/// Move focus to `pid`, a pane of the active layout, first calling
/// [`end_focus_sessions`]. `focus_in_direction`/`cmd_pane_focus_next`/
/// `mouse_left_down` switch focus *within* a tab this way, where no layout
/// displacement happens and no earlier teardown has run.
pub(in crate::editor) fn focus_pane(state: &mut EditorState, view: &mut EngineView, pid: PaneId) {
    end_focus_sessions(state, view);
    view.focus(&state.focus.0, pid)
        .expect("a focus target is picked from the active layout");
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

/// Make `layout` the active tab's, returning the layout it replaces.
///
/// Usually the session teardown is already done by the time this runs:
/// `tab::take_live` calls `end_focus_sessions` itself before touching the
/// layout (see its own doc), and this call is then a no-op. A tabline click
/// (`mouse::tabline_click`) switches tabs by calling `tab::switch_to_tab`
/// directly, bypassing dispatch (`step_paste_commit`'s own chokepoint);
/// this closes that gap for it too.
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

/// One of the two chokepoints `focused_buffer_id()` can change through; see
/// `switch_pane_to_buffer`'s own doc for why this pair promotes
/// `BufferStore.mru` synchronously instead of leaving it to
/// `Editor::detect_buffer_enter`'s later, settle()-gated observation.
fn touch_focused_mru(state: &mut EditorState, view: &EngineView) {
    state
        .buffers
        .touch_mru(view.panes[view.focused_pane()].buffer_id);
}
