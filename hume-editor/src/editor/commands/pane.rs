//! Pane creation and splitting: the single source of truth for seeding and
//! tearing down a pane's per-pane state maps.

use hume_core::{BufferId, PaneId};
use hume_engine::pipeline::{
    DetachedPane, Direction, EngineView, FocusedLayout, SinglePane, SplitShare, SplitTooSmall,
    UnattachedPane,
};

use crate::editor::EditorState;
use crate::editor::commands::FocusedPane;
use crate::editor::error::CommandError;
use crate::editor::focus::{self, focus_pane};
use crate::editor::pane_state::PaneChrome;
use crate::editor::pipe_run::{PipeRefusal, PipeRunning};

/// A new pane viewing `bid`, with the full chrome, not yet in any layout.
/// The returned token makes the caller splice it into one.
fn open_pane(state: &mut EditorState, view: &mut EngineView, bid: BufferId) -> UnattachedPane {
    state.panes.open(
        view,
        bid,
        state.buffers.get_mut(bid),
        state.settings.jump_list_capacity,
        PaneChrome::Full,
    )
}

/// Split the focused pane on `direction`'s axis with a new pane viewing the
/// buffer `buffer` names. `share` places the new seam, `None` for a plain
/// split. Refused, before `buffer` runs and with no pane created, when the
/// split does not fit the screen (see `EngineView::split_focused`).
/// Returns the new pane's id; focus stays where it was.
pub(in crate::editor) fn open_pane_in_layout<E: From<SplitTooSmall>>(
    state: &mut EditorState,
    view: &mut EngineView,
    direction: Direction,
    share: Option<SplitShare>,
    buffer: impl FnOnce(&mut EditorState, &mut EngineView) -> Result<BufferId, E>,
) -> Result<PaneId, E> {
    let screen = state.screen_layout(view);
    view.split_focused(&screen, direction, share, |view| {
        let bid = buffer(state, view)?;
        Ok(open_pane(state, view, bid))
    })
}

/// Create a pane viewing `bid` as the sole content of a brand-new tab
/// inserted right after the current one, and switch to it. The other shape
/// of pane creation, alongside `open_pane_in_layout`'s
/// split-beside-an-existing-pane one: a new tab isn't beside anything, and
/// its graft touches `TabStore` (stash the outgoing tab, allocate the new
/// one), not just `view.layout`, so it can't share that function's body.
pub(in crate::editor) fn open_pane_as_new_tab(
    state: &mut EditorState,
    view: &mut EngineView,
    bid: BufferId,
) {
    let layout = FocusedLayout::new(open_pane(state, view, bid));
    crate::editor::tab::open_tab(state, view, layout);
}

/// Remove every per-pane state map entry for a detached pane (`panes`,
/// per-buffer state, jump list, render handles): the inverse of
/// `open_pane`'s seeding. Input layers bound to it are retired first. Takes
/// a `DetachedPane` rather than a bare `PaneId`: the token is proof the pane
/// is already unreachable from every layout tree (see `DetachedPane`'s own
/// doc), so this can never be called on a pane a tree still references.
///
/// # Panics
/// Panics when the pane started a pipe run still in flight: every close
/// refuses that before detaching a pane ([`CloseRefused::PipeRunning`]).
pub(super) fn drop_pane_state(state: &mut EditorState, view: &mut EngineView, pane: DetachedPane) {
    let pid = pane.pane_id();
    assert_eq!(
        state.pipe_runs.launched_from(&[pid]),
        None,
        "drop_pane_state: a closed pane's pipe run would stop"
    );
    state.retire_bound_to(view, pid);
    view.remove_pane(pane);
    state.panes.remove(pid);
}

/// Why a pane or tab close changed nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum CloseRefused {
    /// The tab's only pane, or the only tab.
    Last,
    /// Closing would stop this pipe run, started from a pane being closed.
    PipeRunning(PipeRunning),
}

impl PipeRefusal for CloseRefused {
    fn pipe_running(&self) -> Option<PipeRunning> {
        match *self {
            CloseRefused::Last => None,
            CloseRefused::PipeRunning(running) => Some(running),
        }
    }
}

/// `Err` when one of `panes` started a pipe run still in flight.
pub(super) fn check_no_pipe_from(
    state: &EditorState,
    panes: &[PaneId],
) -> Result<(), CloseRefused> {
    match state.pipe_runs.launched_from(panes) {
        Some(bid) => Err(CloseRefused::PipeRunning(PipeRunning(bid))),
        None => Ok(()),
    }
}

/// Close the focused pane: prune it from the active tab's layout, move focus
/// to the promoted sibling, and drop all its per-pane state. Refused, with
/// nothing changed, when it is the tab's only pane, or when it started a
/// pipe run still in flight.
pub(super) fn close_focused_pane(
    state: &mut EditorState,
    view: &mut EngineView,
) -> Result<(), CloseRefused> {
    view.refuse_close_focused()
        .map_err(|SinglePane| CloseRefused::Last)?;
    check_no_pipe_from(state, &[view.focused_pane()])?;
    let detached = focus::close_focused(state, view).unwrap_or_else(|SinglePane| {
        unreachable!("a tab with several panes closes its focused one")
    });
    drop_pane_state(state, view, detached);
    Ok(())
}

impl From<SplitTooSmall> for CommandError {
    fn from(SplitTooSmall: SplitTooSmall) -> Self {
        CommandError::transient("pane too small to split")
    }
}

/// Split the focused pane so the new pane views the buffer `buffer` names,
/// and move focus to it. Refused when the split does not fit, before
/// `buffer` runs (see `open_pane_in_layout`). `commands::run`
/// returns that refusal as `false`, which is how the Steel
/// `call!` boolean (see `hume_scripting::host::CommandHost::run_command_sync`)
/// reports the refusal back to a caller like `stdlib/with-pane-command`.
///
/// Shared core for the typed `:split`/`:vsplit [path]` commands (whose
/// `buffer` opens an optional path argument) and the bare keymap-bound
/// `pane-split`/`pane-vsplit` commands (which always split onto the focused
/// pane's own buffer).
pub(in crate::editor) fn split_pane_onto(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    direction: Direction,
    buffer: impl FnOnce(&mut EditorState, &mut EngineView) -> Result<BufferId, CommandError>,
) -> Result<(), CommandError> {
    let old_focused = fp.pid();
    let old_buffer_id = view.panes[old_focused].buffer_id();
    let new_pid = open_pane_in_layout(state, view, direction, None, buffer)?;
    let bid = view.panes[new_pid].buffer_id();

    // A bare split (same buffer as the source pane) inherits its selections,
    // their history and its scroll position: `open_pane` seeds fresh state at
    // the buffer's initial selection, which would otherwise jump the new pane
    // to the top of the file regardless of where the source pane was
    // scrolled to. `:split <path>` (a different buffer) starts fresh.
    if bid == old_buffer_id {
        state.panes.entries.copy_selections(
            &view.panes,
            old_focused,
            new_pid,
            state.buffers.get(bid).text(),
        );
        // A same-buffer split inherits the source pane's live view state
        // (viewport, scroll memory, wrap mode) so the new pane matches where
        // the source was instead of falling back to fresh/global seeds. A
        // `:split <path>` onto a different buffer keeps those fresh seeds.
        let [new_pane, old_pane] = view
            .panes
            .get_disjoint_mut([new_pid, old_focused])
            .expect("new_pid and old_focused are distinct, valid pane keys");
        new_pane.inherit_view_state(old_pane);

        // A same-buffer split inherits the source pane's jump history so the
        // new pane can Ctrl-o back to positions the user visited before the
        // split. The two lists diverge from here: later jumps in either pane
        // don't affect the other. Cursor position within the history is
        // preserved too, so a split mid-navigation stays mid-navigation.
        state.panes.entries[new_pid].jumps = state.panes.entries[old_focused].jumps.clone();
    }

    // `open_pane` already seeded every per-pane map for `new_pid`, so
    // `focus_pane` is complete here.
    focus_pane(state, view, new_pid);
    Ok(())
}
