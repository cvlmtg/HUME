//! Editor-level command functions.
//!
//! Each function in this module is a command operating on
//! `&mut EditorState` + `&mut EngineView` (the D7 handler shape) — composite
//! operations involving mode changes, registers, undo groups, or
//! parameterized motions (find/till/replace).
//!
//! They are registered in [`super::registry`] and called via function pointer
//! from `execute_keymap_command`, exactly like the pure `cmd_*` functions in
//! `ops/motion.rs`, `ops/edit.rs`, etc.
//!
//! The `count` parameter is the user's numeric prefix (default 1). Commands
//! that don't use a count accept it and ignore it (`_count`).

/// Display label used when no named theme is active (the compiled-in default).
pub(super) const DEFAULT_THEME_LABEL: &str = "default (built-in)";

use std::borrow::Cow;

use hume_editing::changeset::ChangeSet;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::Text;
use hume_engine::pipeline::{BufferId, Direction, EngineView, PaneId};
use slotmap::SecondaryMap;

use super::registry::MappableCommand;

use super::CmdCtx;
use super::buffer::Buffer;
use super::doc_ops;
use super::jump_list::JumpEntry;
use super::pane_state::PaneTransient;
use super::registry::CmdMeta;
use super::search::SearchPattern;
use super::{EditorState, InsertSession, Mode, RegisterPrefix, RepeatableAction, SelectionStep};
use super::{Severity, register_ops};
use crate::editor::error::CommandError;
use crate::ops::MotionMode;
use crate::ops::edit::clear_blank_line_indent;

// ── EditorState helpers ───────────────────────────────────────────────────────

impl EditorState {
    /// Consume the pending `"<reg>` prefix and return the explicit register name,
    /// or `None` if no prefix was typed (bare default case).
    pub(super) fn take_register_prefix(&mut self) -> Option<char> {
        match self.register_prefix.take() {
            Some(RegisterPrefix::Selected(c)) => Some(c),
            _ => None,
        }
    }

    /// Write `values` into `name`, routing `'c'` through the OS clipboard.
    pub(super) fn write_register(&mut self, name: char, values: Vec<String>) {
        if let Some(w) =
            register_ops::write_register(&mut self.registers, &mut self.clipboard, name, values)
        {
            self.report(Severity::Warning, w);
        }
    }

    /// Route a kill (`d`/`c`) yank: bare default and `"k` both go to the kill
    /// ring; any other explicit register prefix routes through `write_register`.
    pub(super) fn route_kill(&mut self, yanked: Vec<String>) {
        match self.take_register_prefix() {
            None | Some(crate::ops::register::KILL_RING_REGISTER) => self.kill_ring.push(yanked),
            Some(reg) => self.write_register(reg, yanked),
        }
    }

    /// Commit the open paste session on the focused (pane, buffer) pair, if any.
    ///
    /// Records exactly one history revision for the entire paste + all cycles.
    /// Called before any non-`[`/`]` dispatch so the session is committed
    /// before undo, motions, or the next `p`/`P`.
    ///
    /// Invariant: an open paste session can only exist on the focused (pane,
    /// buffer) pair — sessions are opened only there (`open_paste_session_and_apply`),
    /// every focus/buffer switch dispatches through this same commit step first,
    /// mouse handlers never open or switch during a session, and buffer close
    /// clears `paste_group` explicitly. The debug assert below fails fast if that
    /// invariant is ever violated instead of silently leaving a stray session open.
    pub(super) fn commit_paste_session(&mut self, view: &EngineView) {
        let focused = self.focused_pane_id;
        let buf = focused_buffer_id(self, view);

        debug_assert!(
            self.panes.state.iter().all(|(pid, inner)| {
                inner
                    .iter()
                    .all(|(bid, pbs)| (pid, bid) == (focused, buf) || pbs.paste_group.is_none())
            }),
            "an open paste session exists outside the focused (pane, buffer) pair",
        );

        if self.panes.state[focused][buf].paste_group.is_none() {
            return;
        }
        let post_sels = self.panes.state[focused][buf].selections.clone();
        let pbs = &mut self.panes.state[focused][buf];
        self.buffers
            .get_mut(buf)
            .commit_edit_group(&mut pbs.paste_group, post_sels);
    }
}

// ── Native command body execution ───────────────────────────────────────────

/// Run the body of a native command (Motion/Selection/Edit/EditorCmd) with
/// no dispatch bookkeeping.  Infallible — EditorCmd errors are reported but
/// never propagated.
///
/// Called by both the unified pipeline ([`run_dispatch_pipeline`]) and the
/// dot-repeat replay path ([`Editor::replay_dot`]).
///
/// The single writer of `state.explicit_count`: `count` is `None` for a bare
/// keyboard press (no count typed) or for a Steel `call!` that explicitly asked
/// for the same treatment (a script-side count of `0`, decoded by
/// `parse_count_extend`). Visual-move commands (`move-down`/`move-up`) read
/// `explicit_count == false` as "move by visual row" rather than buffer line.
/// Save/restore (not a plain set) so a Steel command's body dispatching its own
/// native command via `call!` — which nests inside this same function while the
/// outer call's stack frame is still live — gets its own value instead of
/// leaking the outer command's.
pub(super) fn run_native_body(
    state: &mut EditorState,
    view: &mut EngineView,
    cmd: MappableCommand,
    count: Option<usize>,
    extend: bool,
) {
    let prev_explicit_count = std::mem::replace(&mut state.explicit_count, count.is_some());
    let count = count.unwrap_or(1).max(1);
    let motion_mode = if extend {
        MotionMode::Extend
    } else {
        MotionMode::Move
    };
    let buf = focused_buffer_id(state, view);
    let focused = state.focused_pane_id;
    match cmd {
        MappableCommand::Motion { fun, .. } => {
            doc_ops::apply_doc_motion(
                &state.buffers,
                &mut state.panes.state,
                focused,
                buf,
                |b, s| fun(b, s, count, motion_mode),
            );
        }
        MappableCommand::Selection { fun, .. } => {
            doc_ops::apply_doc_motion(
                &state.buffers,
                &mut state.panes.state,
                focused,
                buf,
                |b, s| fun(b, s, count, motion_mode),
            );
        }
        MappableCommand::Edit { fun, .. } => {
            // Route through the grouped path when an edit group is already open
            // (insert session or dot-repeat replay) so the edit composes into the
            // open group rather than creating a standalone undo revision.
            if is_group_open_current(state, view) {
                doc_ops::apply_doc_edit_grouped(
                    &mut state.buffers,
                    &mut state.panes.state,
                    focused,
                    buf,
                    fun,
                );
            } else {
                doc_ops::apply_doc_edit(
                    &mut state.buffers,
                    &mut state.panes.state,
                    focused,
                    buf,
                    fun,
                );
            }
        }
        MappableCommand::EditorCmd { fun, .. } => {
            if let Err(e) = fun(state, view, count, motion_mode) {
                state.report(Severity::Error, e.message().to_owned());
            }
        }
        MappableCommand::SteelBacked { .. } | MappableCommand::Lazy { .. } => {
            unreachable!("run_native_body called on non-native command");
        }
    }
    state.explicit_count = prev_explicit_count;
}

// ── Dispatch step functions ─────────────────────────────────────────────────

// ── Shared steps (used by both native and Steel dispatch paths) ──────────────

/// Commit paste session unless the command defers it (ring-cycle pastes).
pub(super) fn step_paste_commit(state: &mut EditorState, view: &EngineView, defers: bool) {
    if !defers {
        state.commit_paste_session(view);
    }
}

// ── Native-only pre-body steps ─────────────────────────────────────────────────

/// Capture pre-jump cursor position for jump-list recording.
///
/// Selection commands are excluded: a large text-object selection is a
/// select-then-act staging step, not deliberate navigation, so it must not
/// pollute the jump list on a threshold-exceeding extent. Jump-flagged
/// selections (e.g. `%` select-all, `jump: true`) still record via the
/// `meta.is_jump` arm.
pub(super) fn step_capture_pre_jump(
    state: &EditorState,
    view: &EngineView,
    meta: &CmdMeta,
) -> Option<(Selection, usize, BufferId)> {
    (meta.is_jump || meta.is_visual_move || meta.is_motion).then(|| {
        let bid = focused_buffer_id(state, view);
        let primary = current_selections(state, view).primary();
        let line = doc(state, view).text().char_to_line(primary.head());
        (primary, line, bid)
    })
}

/// Snapshot selection recipe before body for dot-repeat recording.
///
/// The snapshot captures the selection extent the user built before the edit,
/// so `.` can re-establish it.  Inner dispatches (Steel `call!`) may overwrite
/// `selection_recipe` during the body; the snapshot is taken before they run.
pub(super) fn step_snapshot_recipe(
    state: &mut EditorState,
    repeatable: bool,
) -> Option<Vec<SelectionStep>> {
    if repeatable {
        Some(std::mem::take(&mut state.selection_recipe))
    } else {
        None
    }
}

// ── AFTER (native steps) ────────────────────────────────────────────────────

/// Stamp `last_command` after the body when `stamps` is `true`.
///
/// `stamps` comes from `CmdMeta.stamps_last_command`, which is `false` only for
/// `exit-insert` — it closes an insert session a kill (`c`) may have opened, so
/// stamping it would clobber the `"change"` marker and break `c <text> Esc p` → ring.
///
/// Called **after** body for native (smart-p reads old value during body),
/// **before** body for Steel (outer name pre-stamped; inner `call!` overrides).
pub(super) fn step_stamp_last_command(
    state: &mut EditorState,
    name: Cow<'static, str>,
    stamps: bool,
) {
    if stamps {
        state.last_command = Some(name);
    }
}

/// Record jump list entry if the command is a jump or the cursor moved
/// past the threshold.
pub(super) fn step_record_jump(
    state: &mut EditorState,
    view: &EngineView,
    pre_jump: Option<(Selection, usize, BufferId)>,
    is_jump: bool,
) {
    if let Some((pre_primary, pre_line, pre_bid)) = pre_jump {
        let post_line = doc(state, view)
            .text()
            .char_to_line(current_selections(state, view).primary().head());
        if is_jump || pre_line.abs_diff(post_line) > state.settings.jump_line_threshold {
            state.panes.jumps[state.focused_pane_id].push(JumpEntry::from_pre_motion(
                pre_primary,
                pre_line,
                pre_bid,
            ));
        }
    }
}

/// Record last_repeatable_action for dot-repeat from the pre-body
/// selection recipe snapshot.
// `&Cow` not `&str`: `.clone()` must preserve Borrowed (built-ins) or Owned
// (Steel) without an unconditional heap alloc.
#[allow(clippy::ptr_arg)]
pub(super) fn step_stamp_repeatable(
    state: &mut EditorState,
    name: &Cow<'static, str>,
    count: usize,
    char_arg: Option<char>,
    pre_recipe: Option<Vec<SelectionStep>>,
) {
    if let Some(recipe) = pre_recipe {
        state.last_repeatable_action = Some(RepeatableAction {
            command: name.clone(),
            count,
            char_arg,
            insert_keys: Vec::new(),
            selection_recipe: recipe,
        });
    }
}

/// Update the selection recipe buffer after a command dispatch.
///
/// Accumulation rule:
///   sel-builder + extend                           → append step
///   sel-builder + move + non-collapsed + in-place  → reset + push establish
///   sel-builder + move + reaching (or collapsed)   → clear
///   everything else                                 → clear
///
/// Reaching motions (`select-next-word` / `-prev-word` / WORD variants) are
/// not recorded in Move mode: replaying such a step advances past the cursor,
/// causing dot-repeat to act on the wrong word. Extend steps of reaching
/// motions (`Ctrl+w`) are still recorded — extending grows an existing
/// selection by a relative amount and is safe to replay.
// `&Cow` not `&str`: `.clone()` must preserve Borrowed (built-ins) or Owned
// (Steel) without an unconditional heap alloc.
#[allow(clippy::ptr_arg)]
pub(super) fn step_update_recipe(
    state: &mut EditorState,
    view: &EngineView,
    meta: &CmdMeta,
    name: &Cow<'static, str>,
    ctx: &CmdCtx,
    char_arg: Option<char>,
) {
    if meta.tracks_selection {
        let sels = current_selections(state, view);
        if ctx.extend {
            state.selection_recipe.push(SelectionStep {
                command: name.clone(),
                count: ctx.count.unwrap_or(1),
                char_arg,
                extend: true,
            });
        } else if !sels.primary().is_collapsed() && !meta.reaching {
            state.selection_recipe.clear();
            state.selection_recipe.push(SelectionStep {
                command: name.clone(),
                count: ctx.count.unwrap_or(1),
                char_arg,
                extend: false,
            });
        } else {
            state.selection_recipe.clear();
        }
    } else {
        state.selection_recipe.clear();
    }
}

/// Exit sticky Extend mode after a selection-consuming edit.
///
/// Mirrors the "done selecting" signal of `;` (collapse) and Vim's visual-mode
/// operator exit. No-op unless the editor is currently in Extend — a `change`
/// command (which already entered Insert) will not be affected because
/// `state.mode()` is `Insert` by the time the AFTER block runs.
pub(super) fn step_clear_extend(state: &mut EditorState, clears_extend: bool) {
    if clears_extend && state.mode() == Mode::Extend {
        state.set_mode(Mode::Normal);
    }
}

// ── Native dispatch pipeline (composed from step functions) ────────────────

/// Execute a native command through the full dispatch pipeline.
///
/// Composed from the step functions above.  Both the keypress path
/// (`Editor::dispatch` → native branch) and the Steel sync path
/// (`EditorHostImpl::run_command_sync`) delegate here.
pub(super) fn run_dispatch_pipeline(
    state: &mut EditorState,
    view: &mut EngineView,
    cmd: MappableCommand,
    ctx: CmdCtx,
) {
    let meta = cmd.meta();
    // Clone the name once, before the body consumes `cmd`. A `&'static str` name
    // (every built-in) clones with no allocation; the AFTER steps reuse this.
    let name = cmd.name().clone();
    // A command cannot be both repeatable (an edit that modifies the buffer)
    // and a selection-builder (a pure cursor movement). If this ever fires,
    // a new variant broke the invariant that step_stamp_repeatable and
    // step_update_recipe rely on for their unconditional sequencing.
    debug_assert!(
        !(meta.repeatable && meta.tracks_selection),
        "command '{name}' is both repeatable and selection-tracking — \
         step_stamp_repeatable and step_update_recipe would both fire",
    );
    // Ring-cycle defer only makes sense on a paste command.
    debug_assert!(
        !meta.defers_paste_commit || meta.is_paste,
        "command '{name}' defers paste commit but is not a paste command",
    );

    // BEFORE
    step_paste_commit(state, view, meta.defers_paste_commit);
    let pre_jump = step_capture_pre_jump(state, view, &meta);
    let char_arg = state.pending_char;
    let pre_recipe = step_snapshot_recipe(state, meta.repeatable);

    // BODY — cmd moved in; meta + name captured above so no further clone needed.
    run_native_body(state, view, cmd, ctx.count, ctx.extend);

    // AFTER
    step_stamp_last_command(state, name.clone(), meta.stamps_last_command);
    step_record_jump(state, view, pre_jump, meta.is_jump);
    step_stamp_repeatable(state, &name, ctx.count.unwrap_or(1), char_arg, pre_recipe);
    step_update_recipe(state, view, &meta, &name, &ctx, char_arg);
    step_clear_extend(state, meta.clears_extend);
}

// ── Free helpers for EditorCmd handlers ──────────────────────────────────────

/// Buffer id the focused pane is viewing.
pub(super) fn focused_buffer_id(state: &EditorState, view: &EngineView) -> BufferId {
    view.panes[state.focused_pane_id].buffer_id
}

/// Shared reference to the focused buffer.
pub(super) fn doc<'a>(state: &'a EditorState, view: &EngineView) -> &'a Buffer {
    state.buffers.get(focused_buffer_id(state, view))
}

/// Apply a motion to the focused (pane, buffer) pair.
///
/// Thin wrapper around [`doc_ops::apply_doc_motion`] that resolves the
/// focused pane/buffer so call sites don't repeat that lookup.
pub(super) fn apply_focused_motion(
    state: &mut EditorState,
    view: &EngineView,
    f: impl FnOnce(&Text, SelectionSet) -> SelectionSet,
) {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf, f);
}

/// Apply an edit to the focused (pane, buffer) pair.
///
/// Thin wrapper around [`doc_ops::apply_doc_edit`]; see [`apply_focused_motion`].
pub(super) fn apply_focused_edit(
    state: &mut EditorState,
    view: &EngineView,
    cmd: impl FnOnce(Text, SelectionSet) -> (Text, SelectionSet, ChangeSet),
) {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_edit(
        &mut state.buffers,
        &mut state.panes.state,
        focused,
        buf,
        cmd,
    );
}

/// Apply a grouped edit (inside an open insert/paste session) to the focused
/// (pane, buffer) pair.
///
/// Thin wrapper around [`doc_ops::apply_doc_edit_grouped`]; see
/// [`apply_focused_motion`].
pub(super) fn apply_focused_edit_grouped(
    state: &mut EditorState,
    view: &EngineView,
    cmd: impl FnOnce(Text, SelectionSet) -> (Text, SelectionSet, ChangeSet),
) {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_edit_grouped(
        &mut state.buffers,
        &mut state.panes.state,
        focused,
        buf,
        cmd,
    );
}

// ── Pane creation / splitting ─────────────────────────────────────────────────

/// Create a new pane viewing `buffer_id`, seed all per-pane maps, return its id.
///
/// The single source of truth for pane creation: used by `split_pane_onto`
/// (and thus the typed `:split`/`:vsplit` commands and the bare `pane-split`/
/// `pane-vsplit` keymap commands), and called directly by tests that only
/// have `&mut EditorState` + `&mut EngineView` access.
pub(super) fn open_pane(
    state: &mut EditorState,
    view: &mut EngineView,
    buffer_id: BufferId,
) -> PaneId {
    // Every pane gets the same providers (sign column + gutter + bracket/
    // search/diagnostic/extra highlight + inlay hints + virtual lines +
    // completion overlay + popup overlay + menu overlay + LSP
    // completion-menu overlay) as the initial pane — see `build_pane`. Each
    // pane's Arcs are freshly allocated here, never shared with any other
    // pane (see `PaneHighlights`/`PaneSigns`), so per-pane decoration data
    // can never bleed across panes.
    let (pane, render_handles) = crate::ui::build_pane(
        &mut view.registry,
        &state.completion_view,
        &state.popup_view,
        &state.menu_view,
        &state.lsp_completion_view,
        state.settings.wrap_mode,
        buffer_id,
    );
    let pid = view.panes.insert(pane);
    state.panes.state.insert(pid, SecondaryMap::new());
    super::pane_state::ensure(&mut state.panes.state, &state.buffers, pid, buffer_id);
    state.panes.transient.insert(pid, PaneTransient::default());
    state.panes.jumps.insert(
        pid,
        super::jump_list::JumpList::new(state.settings.jump_list_capacity),
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

/// Whether the focused pane's current on-screen rect has room for another
/// split on `direction`, including the 1-cell seam divider drawn between the
/// two resulting panes (see `hume_engine::pipeline::split_rect`).
///
/// Recomputes geometry from `view.last_pane_area` on every call (see
/// `EngineView::pane_rect`) rather than trusting a cross-frame cache, so a
/// split issued right after a close/split earlier in the same replay batch
/// always sees current geometry. Before the first `prepare_frame` there is
/// no real terminal area yet — allow the split; `prepare_frame` sizes it
/// correctly on the next frame regardless.
pub(super) fn fits_split(state: &EditorState, view: &EngineView, direction: Direction) -> bool {
    if view.last_pane_area.area() == 0 {
        return true;
    }
    let Some(rect) = view.pane_rect(state.focused_pane_id) else {
        return true;
    };
    match direction {
        Direction::Vertical => rect.height > 2 * MIN_PANE_HEIGHT,
        Direction::Horizontal => rect.width > 2 * MIN_PANE_WIDTH,
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
pub(super) fn split_pane_onto(
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

    let found = view.layout.split_leaf(old_focused, new_pid, direction, 0.5);
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

/// `true` when the focused buffer is read-only.
pub(super) fn focused_buffer_read_only(state: &EditorState, view: &EngineView) -> bool {
    doc(state, view).is_read_only()
}

/// Focused pane's selections for the current buffer.
pub(super) fn current_selections<'a>(
    state: &'a EditorState,
    view: &EngineView,
) -> &'a SelectionSet {
    let bid = focused_buffer_id(state, view);
    &state.panes.state[state.focused_pane_id][bid].selections
}

/// `true` if any current selection is a collapsed cursor sitting on a blank,
/// auto-indented line — the condition under which [`clear_blank_line_indent`]
/// would actually change the buffer. Checked before calling it so the common
/// case (exiting Insert mode away from a blank line) skips the edit entirely
/// instead of running an identity one (see
/// [`crate::ops::edit::blank_line_ws_range`]'s doc comment).
fn has_blank_line_cursor(state: &EditorState, view: &EngineView) -> bool {
    let buf = doc(state, view).text();
    current_selections(state, view).iter_sorted().any(|sel| {
        sel.is_collapsed() && crate::ops::edit::blank_line_ws_range(buf, sel.head()).is_some()
    })
}

/// The most-recently-focused buffer other than the current one.
pub(super) fn alternate_buffer(state: &EditorState, view: &EngineView) -> Option<BufferId> {
    state.buffers.mru_excluding(focused_buffer_id(state, view))
}

/// `true` when the focused (pane, buffer) has an open edit group.
fn is_group_open_current(state: &EditorState, view: &EngineView) -> bool {
    let bid = focused_buffer_id(state, view);
    state.panes.state[state.focused_pane_id][bid]
        .edit_group
        .is_some()
}

/// Open a new edit group on the focused (pane, buffer) pair.
pub(super) fn begin_edit_group_current(state: &mut EditorState, view: &EngineView) {
    let pid = state.focused_pane_id;
    let bid = focused_buffer_id(state, view);
    doc_ops::begin_edit_group(&state.buffers, &mut state.panes.state, pid, bid);
}

/// Commit and close the open edit group on the focused (pane, buffer) pair.
pub(super) fn commit_edit_group_current(state: &mut EditorState, view: &EngineView) {
    let pid = state.focused_pane_id;
    let bid = focused_buffer_id(state, view);
    doc_ops::commit_edit_group(&mut state.buffers, &mut state.panes.state, pid, bid);
}

/// Active search pattern on the focused buffer, if any.
pub(super) fn search_pattern<'a>(
    state: &'a EditorState,
    view: &EngineView,
) -> Option<&'a SearchPattern> {
    state
        .buffers
        .get(focused_buffer_id(state, view))
        .search_pattern
        .as_ref()
}

/// Viewport state of the focused pane.
pub(super) fn viewport<'a>(
    state: &EditorState,
    view: &'a EngineView,
) -> &'a hume_engine::pane::ViewportState {
    &view.panes[state.focused_pane_id].viewport
}

/// Resolved `(wrap_mode, tab_width, whitespace)` for the focused doc and pane.
pub(super) fn focused_format_context(
    state: &EditorState,
    view: &EngineView,
) -> (
    hume_engine::pane::WrapMode,
    u8,
    hume_engine::pane::WhitespaceConfig,
) {
    let buf = doc(state, view);
    let tab_width = buf.overrides.tab_width(&state.settings);
    let whitespace = buf.overrides.whitespace(&state.settings);
    let pane = &view.panes[state.focused_pane_id];
    let wrap_mode = pane
        .wrap_mode
        .resolve(pane.content_width(buf.text().len_lines()));
    (wrap_mode, tab_width, whitespace)
}

/// Snapshot the focused pane's current cursor as a `JumpEntry`.
pub(super) fn current_jump_entry(state: &EditorState, view: &EngineView) -> JumpEntry {
    let pid = state.focused_pane_id;
    let bid = focused_buffer_id(state, view);
    let sels = state.panes.state[pid][bid].selections.clone();
    JumpEntry::new(sels, state.buffers.get(bid).text(), bid)
}

/// Redirect the focused pane to `target` without recording a jump.
pub(super) fn switch_to_buffer_without_jump(
    state: &mut EditorState,
    view: &mut EngineView,
    target: BufferId,
) {
    let pid = state.focused_pane_id;
    super::buffer::lifecycle::switch_pane_to_buffer(
        view,
        &state.buffers,
        &mut state.panes.state,
        pid,
        target,
    );
}

/// Replace the focused pane's selections for the current buffer.
pub(super) fn set_current_selections(
    state: &mut EditorState,
    view: &EngineView,
    sels: SelectionSet,
) {
    let bid = focused_buffer_id(state, view);
    state.panes.state[state.focused_pane_id][bid].selections = sels;
}

/// Replace the primary selection in the focused pane (merging overlaps).
pub(super) fn set_primary_selection(
    state: &mut EditorState,
    view: &EngineView,
    new_sel: hume_editing::selection::Selection,
) {
    let pid = state.focused_pane_id;
    let bid = focused_buffer_id(state, view);
    let idx = state.panes.state[pid][bid].selections.primary_index();
    let sels = std::mem::take(&mut state.panes.state[pid][bid].selections);
    state.panes.state[pid][bid].selections = sels.replace(idx, new_sel);
}

/// Enter Insert mode as a repeatable insert action.
///
/// No-op (with a warning) if the focused buffer is read-only. Replay-signal:
/// if an edit group is already open, recording is suppressed but the mode
/// change still happens.
pub(super) fn begin_insert_session(state: &mut EditorState, view: &EngineView) {
    if focused_buffer_read_only(state, view) {
        state.report(Severity::Info, "Buffer is read-only".to_string());
        return;
    }
    // Guard is load-bearing for dot-repeat replay: `replay_dot` opens
    // an edit group before re-dispatching the command, so a group already being
    // open here means "we are replaying" → skip session creation and re-type from
    // `insert_keys` instead of recording fresh. Do NOT weaken this into a separate
    // flag without also fixing the replay signal.
    //
    // The implied assumption — that no Steel body can reach `begin_insert_session`
    // with a group already open outside of replay — holds because Steel has no
    // transaction / begin-edit-group builtin, and none should ever be added:
    // fine-grained undo grouping belongs to native commands, not scripts.
    if !is_group_open_current(state, view) {
        begin_edit_group_current(state, view);
        state.insert_session = Some(InsertSession {
            keystrokes: Vec::new(),
            step_back_on_exit: false,
        });
    }
    // Outside the guard above (unlike `insert_session`) so replay — which
    // skips session creation — still starts each replayed session with no
    // pending auto-indent to vacate, matching a fresh interactive session.
    state.autoindent_pending = false;
    state.set_mode(Mode::Insert);
}

/// Exit Insert mode and finalise the undo/repeat state.
pub(super) fn end_insert_session(state: &mut EditorState, view: &EngineView) {
    let step_back = state
        .insert_session
        .as_ref()
        .is_some_and(|s| s.step_back_on_exit);
    // Vim autoindent parity: trim a blank auto-indented line's whitespace
    // before committing, so leaving Insert mode on one behaves like Enter
    // does in `insert_newline_indent`. Joins the still-open session group —
    // not a separate undo step. Gated on two conditions: `autoindent_pending`
    // (the line's indent was auto-inserted by *this* session and nothing has
    // been typed on it since — vim only vacates indent it created, never
    // pre-existing or hand-typed whitespace) and `has_blank_line_cursor` (the
    // common case, cursor not on a blank line, skips the edit rather than
    // running an identity one on every Insert-mode exit).
    if state.autoindent_pending && has_blank_line_cursor(state, view) {
        apply_focused_edit_grouped(state, view, clear_blank_line_indent);
    }
    commit_edit_group_current(state, view);
    if let (Some(session), Some(action)) = (
        state.insert_session.take(),
        state.last_repeatable_action.as_mut(),
    ) {
        action.insert_keys = session.keystrokes;
    }
    if step_back {
        apply_focused_motion(state, view, |b, sels| {
            sels.map(|sel| {
                let head = sel.head();
                let line_start = b.line_to_char(b.char_to_line(head));
                let new_head = if head > line_start {
                    hume_editing::grapheme::prev_grapheme_boundary(b, head)
                } else {
                    head
                };
                hume_editing::selection::Selection::collapsed(new_head)
            })
        });
    }
    state.set_mode(Mode::Normal);
}

mod edit;
mod find;
mod jump;
mod mode;
mod scroll;
mod search;
mod typed_buffer;
mod typed_file;
mod typed_lsp;
mod typed_misc;

pub(super) use edit::*;
pub(super) use find::*;
pub(super) use jump::*;
pub(super) use mode::*;
pub(super) use scroll::*;
pub(super) use search::*;
pub(super) use typed_buffer::*;
pub(super) use typed_file::*;
pub(super) use typed_lsp::*;
pub(super) use typed_misc::*;

// Visual-line commands live in visual_move.rs; re-export for the registry glob.
pub(super) use super::visual_move::{
    cmd_visual_move_down, cmd_visual_move_up, cmd_visual_select_word_nearest_on_line,
};
