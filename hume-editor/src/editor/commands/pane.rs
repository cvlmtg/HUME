//! Pane creation and splitting: the single source of truth for seeding and
//! tearing down a pane's per-pane state maps.

use hume_engine::pipeline::{
    BufferId, DetachedPane, Direction, EngineView, FocusedLayout, PaneId, SinglePane,
    UnattachedPane,
};

use crate::editor::EditorState;
use crate::editor::commands::FocusedPane;
use crate::editor::error::CommandError;
use crate::editor::focus::{self, focus_pane};
use crate::editor::pane_state::PaneChrome;

/// A new pane viewing `bid`, with the full chrome, not yet in any layout.
/// The returned token makes the caller splice it into one.
fn open_pane(state: &mut EditorState, view: &mut EngineView, bid: BufferId) -> UnattachedPane {
    state.panes.open(
        view,
        bid,
        state.buffers.get(bid),
        state.settings.jump_list_capacity,
        PaneChrome::Full,
    )
}

/// Create a pane viewing `bid` and splice it into the layout beside the
/// focused pane, on `direction`'s axis, in one step. Returns the new pane's
/// id; focus stays where it was.
pub(in crate::editor) fn open_pane_in_layout(
    state: &mut EditorState,
    view: &mut EngineView,
    bid: BufferId,
    direction: Direction,
) -> PaneId {
    let unattached = open_pane(state, view, bid);
    view.split_focused(unattached, direction)
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
/// `open_pane`'s seeding. The pipe runs it started are cancelled and input
/// layers bound to it retired first. Takes a `DetachedPane` rather than a bare
/// `PaneId`: the token is proof the pane is already unreachable from every
/// layout tree (see `DetachedPane`'s own doc), so this can never be called
/// on a pane a tree still references.
pub(super) fn drop_pane_state(state: &mut EditorState, view: &mut EngineView, pane: DetachedPane) {
    let pid = pane.pane_id();
    state.cancel_pipe_runs_launched_by(pid);
    state.retire_bound_to(view, pid);
    view.remove_pane(pane);
    state.panes.remove(pid);
}

/// Close the focused pane: prune it from the active tab's layout, move focus
/// to the promoted sibling, and drop all its per-pane state. Refused, with
/// nothing changed, when it is the tab's only pane.
pub(super) fn close_focused_pane(
    state: &mut EditorState,
    view: &mut EngineView,
) -> Result<(), SinglePane> {
    let detached = focus::close_focused(state, view)?;
    drop_pane_state(state, view, detached);
    Ok(())
}

/// Status message reported when a split is rejected for being too small.
/// Shared constant: the typed `:split`/`:vsplit [path]` guard and
/// `split_pane_onto`'s guard both report this for the same failure.
pub(super) const SPLIT_TOO_SMALL_MSG: &str = "pane too small to split";

/// Minimum content rows a pane must keep on its split axis for a height
/// split (`:split`) to be allowed.
const MIN_PANE_HEIGHT: u16 = 3;
/// Minimum content columns a pane must keep on its split axis for a width
/// split (`:vsplit`) to be allowed. Wider than `MIN_PANE_HEIGHT` because text
/// needs more horizontal room than vertical to stay usable.
const MIN_PANE_WIDTH: u16 = 10;

/// Whether splitting the focused pane on `direction` would leave every pane
/// sharing that axis (not just the two new ones) at or above the minimum
/// size, including the 1-cell seam divider drawn between siblings (see
/// `hume_engine::pipeline::split_rect`).
///
/// Simulates the split via `ScreenLayout::focused_split_rect` rather than
/// halving the focused pane's current rect: since `equalize` resizes every
/// pane on the split's axis, not just the pair being split, a pane deep in a
/// row of several can be pushed under the minimum by a split that never
/// touches it directly.
pub(in crate::editor) fn fits_split(
    state: &EditorState,
    view: &EngineView,
    direction: Direction,
) -> bool {
    let rect = state
        .screen_layout(view)
        .focused_split_rect(view.focused_layout(), direction);
    match direction {
        Direction::Vertical => rect.height >= MIN_PANE_HEIGHT,
        Direction::Horizontal => rect.width >= MIN_PANE_WIDTH,
    }
}

/// Split the focused pane so the new pane views `bid`, and move focus to it.
/// Refuses with [`SPLIT_TOO_SMALL_MSG`] if the focused pane is too small to
/// fit two panes plus the seam divider (see `fits_split`). `commands::run`
/// returns that refusal as `false`, which is how the Steel
/// `call!` boolean (see `hume_scripting::host::CommandHost::run_command_sync`)
/// reports the refusal back to a caller like `stdlib/with-pane-command`.
///
/// Shared core for the typed `:split`/`:vsplit [path]` commands (which resolve
/// `bid` from an optional path argument first) and the bare keymap-bound
/// `pane-split`/`pane-vsplit` commands (which always split onto the focused
/// pane's own buffer).
pub(in crate::editor) fn split_pane_onto(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    bid: BufferId,
    direction: Direction,
) -> Result<(), CommandError> {
    if !fits_split(state, view, direction) {
        return Err(CommandError::transient(SPLIT_TOO_SMALL_MSG));
    }
    let old_focused = fp.pid();
    let old_buffer_id = view.panes[old_focused].buffer_id();
    let new_pid = open_pane_in_layout(state, view, bid, direction);

    // A bare split (same buffer as the source pane) inherits its cursor and
    // scroll position: `open_pane` seeds fresh state at the buffer's initial
    // selection, which would otherwise jump the new pane to the top of the
    // file regardless of where the source pane was scrolled to. `:split
    // <path>` (a different buffer) intentionally starts fresh.
    if bid == old_buffer_id {
        let selections = state
            .panes
            .entries
            .shown(&view.panes, old_focused)
            .selections()
            .clone();
        state
            .panes
            .entries
            .shown_mut(&view.panes, new_pid)
            .set_selections(selections, state.buffers.get(bid).text());
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
