//! Pane creation and splitting: the single source of truth for seeding and
//! tearing down a pane's per-pane state maps.

use hume_engine::pipeline::{
    BufferId, DetachedPane, Direction, EngineView, LayoutTree, PaneId, Pruned,
};
use slotmap::SecondaryMap;

use super::end_insert_session;
use crate::editor::error::CommandError;
use crate::editor::pane_state::PaneTransient;
use crate::editor::tab::{TabId, install_live, take_live};
use crate::editor::{EditorState, Mode, Severity};

/// Create a new pane viewing `buffer_id`, seed all per-pane maps, return its
/// id — but leave it outside every tab's layout (see `TabStore`'s own doc:
/// a pane is meant to be reachable from exactly one tab's `LayoutTree` at
/// all times, active or stashed; this alone doesn't make that true yet).
///
/// Private on purpose: this is the one raw constructor, and the two
/// functions below it — `open_pane_in_layout` (splice beside an existing
/// pane) and `open_pane_as_new_tab` (splice as a brand-new tab) — are the
/// *only* sanctioned ways to reach it, in production or in tests. Nothing
/// else, anywhere, may call this directly; there is no third legitimate way
/// to create a pane.
fn open_pane(state: &mut EditorState, view: &mut EngineView, buffer_id: BufferId) -> PaneId {
    // Every pane gets the same providers (sign column + gutter + bracket/
    // search/diagnostic/extra highlight + inlay hints + virtual lines +
    // completion overlay + popup overlay + menu overlay + LSP
    // completion-menu overlay) as the initial pane — see `build_pane`. Each
    // pane's Arcs are freshly allocated here, never shared with any other
    // pane (see `PaneHighlights`/`SignMap`), so per-pane decoration data
    // can never bleed across panes.
    let (pane, render_handles) =
        crate::editor::pane_state::build_pane(&mut view.registry, &state.views, buffer_id);
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

/// Create a pane viewing `bid` and splice it into the layout beside
/// `target`, on `direction`'s axis, in one step — `open_pane` alone only
/// does the first half (see its own doc), leaving the new pane outside
/// every tab's layout until a caller does the second half itself. Returns
/// the new pane's id, or an error naming `target` if it isn't present in
/// the layout (an invariant violation) — checked before `open_pane` runs,
/// so there is never a pane to roll back: no caller can create one with no
/// layout leaf to begin with.
pub(in crate::editor) fn open_pane_in_layout(
    state: &mut EditorState,
    view: &mut EngineView,
    target: PaneId,
    bid: BufferId,
    direction: Direction,
) -> Result<PaneId, CommandError> {
    if !view.layout.contains_leaf(target) {
        return Err(CommandError::new(format!(
            "internal error: split target {target:?} missing from pane layout"
        )));
    }
    let new_pid = open_pane(state, view, bid);
    let grafted = view.layout.split_leaf(target, new_pid, direction);
    debug_assert!(grafted, "contains_leaf just confirmed target is present");
    Ok(new_pid)
}

/// Create a pane viewing `bid` as the sole content of a brand-new tab
/// inserted right after the current one, and switch to it. The other shape
/// of pane creation, alongside `open_pane_in_layout`'s
/// split-beside-an-existing-pane one — a new tab isn't beside anything, and
/// its graft touches `TabStore` (stash the outgoing tab, allocate the new
/// one), not just `view.layout`, so it can't share that function's body.
/// `commands::tab::open_tab` is the one caller.
///
/// Before `take_live`: `open_pane` never reads `view.layout`, so the new
/// pane exists before the live layout is displaced, and `take_live`'s own
/// placeholder never needs to appear here at all.
pub(in crate::editor) fn open_pane_as_new_tab(
    state: &mut EditorState,
    view: &mut EngineView,
    bid: BufferId,
) -> (PaneId, TabId) {
    let new_pid = open_pane(state, view, bid);
    let (outgoing_layout, outgoing_focus) = take_live(state, view);
    let tab_id = state.tabs.open_after_current(
        outgoing_layout,
        outgoing_focus,
        LayoutTree::Leaf(new_pid),
        new_pid,
    );
    install_live(state, view, LayoutTree::Leaf(new_pid), new_pid);
    (new_pid, tab_id)
}

/// End the focused pane's open Insert session, if any — a no-op past the
/// mode check otherwise, so every caller can route through this
/// unconditionally instead of repeating the check itself.
///
/// Split out of `focus_pane` below so `tab::take_live` can run it *before*
/// `view.layout` is displaced, not just before `focused_pane_id` is
/// reassigned — see `take_live`'s own doc for why the layout has to stay
/// the outgoing tab's for the duration of this call.
pub(in crate::editor) fn end_insert_session_if_active(state: &mut EditorState, view: &EngineView) {
    if state.mode() == Mode::Insert {
        end_insert_session(state, view);
    }
}

/// Move focus to `pid`, ending any open Insert session and any open paste
/// session first — the one production chokepoint every `focused_pane_id`
/// write goes through (`close_focused_pane`/`split_pane_onto` below,
/// `jump::focus_in_direction`/`cmd_pane_focus_next`, `tab::install_live`,
/// `mouse::mouse_left_down`). Both teardowns read state keyed on the
/// *outgoing* pane (`end_insert_session`'s blank-line indent trim,
/// `commit_paste_session`'s focused-pane paste group) and so must run —
/// while `state.focused_pane_id` still names that outgoing pane — before the
/// assignment below, not after: done later, they'd land on `pid`'s buffer
/// instead of the one actually being left.
///
/// The Insert-session half is usually already done by the time this runs —
/// `tab::take_live` and `commands::tab::close_tab` both call
/// `end_insert_session_if_active` themselves before touching the layout (see
/// `take_live`'s doc), and this call is then a no-op past the mode check.
/// Kept here too rather than only at those two call sites, since
/// `jump::focus_in_direction`/`cmd_pane_focus_next`/`mouse::mouse_left_down`
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
    end_insert_session_if_active(state, view);
    state.commit_paste_session(view);
    state.focused_pane_id = pid;
}

/// Remove every per-pane state map entry for a detached pane (`panes`,
/// per-buffer state, transient state, jump list, render handles) — the
/// inverse of `open_pane`'s seeding. Takes a `DetachedPane` rather than a
/// bare `PaneId`: the token is proof the pane is already unreachable from
/// every layout tree (see `DetachedPane`'s own doc), so this can never be
/// called on a pane a tree still references. Shared by `close_focused_pane`
/// and `commands::tab::close_tab` (once per token `LayoutTree::into_detached`
/// yields for the closing tab's tree).
pub(super) fn drop_pane_state(state: &mut EditorState, view: &mut EngineView, pane: DetachedPane) {
    let pid = pane.pane_id();
    view.panes.remove(pid);
    state.panes.state.remove(pid);
    state.panes.transient.remove(pid);
    state.panes.jumps.remove(pid);
    state.panes.render.remove(pid);
}

/// Close the focused pane: prune it from the layout tree, move focus to the
/// promoted sibling, and drop all its per-pane state.
///
/// Precondition: the active tab's layout is split — callers check
/// `!view.layout.is_single_pane()` before calling. `view.panes.len() > 1` is
/// NOT the right count: panes are a global pool shared by every tab, so it
/// stays true whenever any other tab holds a pane, even when the active tab
/// has only this one. `remove_leaf` returning `None` here is a bug.
pub(super) fn close_focused_pane(state: &mut EditorState, view: &mut EngineView) {
    let old = state.focused_pane_id;
    let Pruned { detached, survivor } = view
        .layout
        .remove_leaf(old)
        .expect("close_focused_pane requires the active tab's layout to be split");
    focus_pane(state, view, survivor);
    drop_pane_state(state, view, detached);
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
        state.report(Severity::Info, SPLIT_TOO_SMALL_MSG.to_string());
        return Ok(());
    }
    let old_focused = state.focused_pane_id;
    let old_buffer_id = view.panes[old_focused].buffer_id;
    let new_pid = open_pane_in_layout(state, view, old_focused, bid, direction)?;

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

    // `open_pane` already seeded every per-pane map for `new_pid`, so
    // `focus_pane` is complete here. Not `switch_focused_pane` — that
    // test-only choke-point's Normal-mode debug_assert would fire when
    // called from the typed `:split`/`:vsplit` path, which dispatches while
    // still in `Mode::Command` (mode flips to Normal only after
    // `execute_command` returns). `focus_pane` has no such assertion: its
    // Insert-exit check is a no-op outside Insert mode, which is always true
    // on this path today, but routes the same as every other pane-focus
    // writer rather than special-casing this one for a mode that can't
    // reach it yet.
    focus_pane(state, view, new_pid);
    Ok(())
}
