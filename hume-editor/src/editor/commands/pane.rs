//! Pane creation and splitting: the single source of truth for seeding and
//! tearing down a pane's per-pane state maps.

use hume_engine::pipeline::{BufferId, Direction, EngineView, PaneId};
use slotmap::SecondaryMap;

use crate::editor::error::CommandError;
use crate::editor::pane_state::PaneTransient;
use crate::editor::{EditorState, Severity};

/// Create a new pane viewing `buffer_id`, seed all per-pane maps, return its id.
///
/// The single source of truth for pane creation: used by `split_pane_onto`
/// (and thus the typed `:split`/`:vsplit` commands and the bare `pane-split`/
/// `pane-vsplit` keymap commands), and called directly by tests that only
/// have `&mut EditorState` + `&mut EngineView` access.
pub(in crate::editor) fn open_pane(
    state: &mut EditorState,
    view: &mut EngineView,
    buffer_id: BufferId,
) -> PaneId {
    // Every pane gets the same providers (sign column + gutter + bracket/
    // search/diagnostic/extra highlight + inlay hints + virtual lines +
    // completion overlay + popup overlay + menu overlay + LSP
    // completion-menu overlay) as the initial pane — see `build_pane`. Each
    // pane's Arcs are freshly allocated here, never shared with any other
    // pane (see `PaneHighlights`/`SignMap`), so per-pane decoration data
    // can never bleed across panes.
    let (pane, render_handles) = crate::ui::build_pane(
        &mut view.registry,
        &state.minibuf_completion_view,
        &state.popup_view,
        &state.menu_view,
        &state.completion_menu_view,
        &state.picker_view,
        buffer_id,
    );
    let pid = view.panes.insert(pane);
    state.panes.state.insert(pid, SecondaryMap::new());
    crate::editor::pane_state::ensure(&mut state.panes.state, &state.buffers, pid, buffer_id);
    state.panes.transient.insert(pid, PaneTransient::default());
    state.panes.jumps.insert(
        pid,
        crate::editor::jump_list::JumpList::new(state.settings.jump_list_capacity),
    );
    state.panes.render.insert(pid, render_handles);
    pid
}

/// Remove every per-pane state map entry for `pid` (`panes`, per-buffer
/// state, transient state, jump list, render handles) — the inverse of
/// `open_pane`'s seeding. Shared by `close_focused_pane` and
/// `split_pane_onto`'s failure-rollback path.
fn drop_pane_state(state: &mut EditorState, view: &mut EngineView, pid: PaneId) {
    view.panes.remove(pid);
    state.panes.state.remove(pid);
    state.panes.transient.remove(pid);
    state.panes.jumps.remove(pid);
    state.panes.render.remove(pid);
}

/// Close the focused pane: prune it from the layout tree, move focus to the
/// promoted sibling, and drop all its per-pane state.
///
/// Precondition: more than one pane exists — callers check `view.panes.len() > 1`
/// before calling. `remove_leaf` returning `None` (sole leaf) is a bug here.
pub(super) fn close_focused_pane(state: &mut EditorState, view: &mut EngineView) {
    let old = state.focused_pane_id;
    let survivor = view
        .layout
        .remove_leaf(old)
        .expect("close_focused_pane requires more than one pane");
    state.focused_pane_id = survivor;
    drop_pane_state(state, view, old);
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
/// sharing that axis — not just the two new ones — at or above the minimum
/// size, including the 1-cell seam divider drawn between siblings (see
/// `hume_engine::pipeline::split_rect`).
///
/// Simulates the split via `LayoutTree::predicted_split_rect` rather than
/// halving the focused pane's current rect: since `equalize` resizes every
/// pane on the split's axis, not just the pair being split, a pane deep in a
/// row of several can be pushed under the minimum by a split that never
/// touches it directly.
///
/// Recomputes geometry from `view.last_pane_area` on every call rather than
/// trusting a cross-frame cache, so a split issued right after a close/split
/// earlier in the same replay batch always sees current geometry. Before the
/// first `prepare_frame` there is no real terminal area yet — allow the
/// split; `prepare_frame` sizes it correctly on the next frame regardless.
pub(in crate::editor) fn fits_split(
    state: &EditorState,
    view: &EngineView,
    direction: Direction,
) -> bool {
    if view.last_pane_area.area() == 0 {
        return true;
    }
    let Some(rect) = view.layout.predicted_split_rect(
        state.focused_pane_id,
        view.last_pane_area,
        view.reserve_seam,
        direction,
    ) else {
        return true;
    };
    match direction {
        Direction::Vertical => rect.height >= MIN_PANE_HEIGHT,
        Direction::Horizontal => rect.width >= MIN_PANE_WIDTH,
    }
}

/// Split the focused pane so the new pane views `bid`, and move focus to it.
/// No-ops with a status warning if the focused pane is too small to fit two
/// panes plus the seam divider (see `fits_split`).
///
/// Shared core for the typed `:split`/`:vsplit [path]` commands (which resolve
/// `bid` from an optional path argument first) and the bare keymap-bound
/// `pane-split`/`pane-vsplit` commands (which always split onto the focused
/// pane's own buffer).
pub(in crate::editor) fn split_pane_onto(
    state: &mut EditorState,
    view: &mut EngineView,
    bid: BufferId,
    direction: Direction,
) -> Result<(), CommandError> {
    if !fits_split(state, view, direction) {
        state.report(Severity::Warning, SPLIT_TOO_SMALL_MSG.to_string());
        return Ok(());
    }
    let old_focused = state.focused_pane_id;
    let old_buffer_id = view.panes[old_focused].buffer_id;
    let new_pid = open_pane(state, view, bid);

    let found = view.layout.split_leaf(old_focused, new_pid, direction);
    if !found {
        // `open_pane` already inserted `new_pid`'s state before the layout
        // mutation could fail — undo it rather than leaving an orphaned pane
        // with no layout leaf, which would later violate `close_focused_pane`'s
        // precondition on `remove_leaf`.
        drop_pane_state(state, view, new_pid);
        return Err(CommandError::new(format!(
            "internal error: split target {old_focused:?} missing from pane layout"
        )));
    }

    // A bare split (same buffer as the source pane) inherits its cursor and
    // scroll position — `open_pane` seeds fresh state at the buffer's initial
    // selection, which would otherwise jump the new pane to the top of the
    // file regardless of where the source pane was scrolled to. `:split
    // <path>` (a different buffer) intentionally starts fresh.
    if bid == old_buffer_id {
        let selections = state.panes.state[old_focused][bid].selections.clone();
        state.panes.state[new_pid][bid].selections = selections;
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
        // new pane can Ctrl+O back to positions the user visited before the
        // split. The two lists diverge from here — later jumps in either pane
        // don't affect the other. Cursor position within the history is
        // preserved too, so a split mid-navigation stays mid-navigation.
        state.panes.jumps[new_pid] = state.panes.jumps[old_focused].clone();
    }

    // `open_pane` already seeded every per-pane map for `new_pid`, so a direct
    // assignment is complete. Do NOT route through `switch_focused_pane`: its
    // Normal-mode debug_assert would fire when called from the typed
    // `:split`/`:vsplit` path, which dispatches while still in `Mode::Command`
    // (mode flips to Normal only after `execute_command` returns).
    state.focused_pane_id = new_pid;
    Ok(())
}
