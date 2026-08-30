//! The native command dispatch pipeline: `run_native_body` executes a
//! command's own effect; the `step_*` functions are the bookkeeping
//! (dot-repeat, jump list, selection recipe, sticky extend) that wraps every
//! dispatch, composed into one pipeline by `run_dispatch_pipeline`.

use std::borrow::Cow;

use hume_editing::selection::Selection;
use hume_engine::pipeline::{BufferId, EngineView};

use crate::editor::dispatch::CmdCtx;
use crate::editor::doc_ops;
use crate::editor::jump_list::JumpEntry;
use crate::editor::registry::{CmdMeta, MappableCommand, SelectionTracking};
use crate::editor::replay::{RepeatableAction, SelectionStep};
use crate::editor::{EditorState, Mode, Severity};
use hume_ops::MotionMode;

use super::{current_selections, doc, focused_buffer_id};

// ── Native command body execution ───────────────────────────────────────────

/// Run the body of a native command (Motion/Selection/Edit/EditorCmd) with
/// no dispatch bookkeeping.  Infallible — EditorCmd errors are reported but
/// never propagated.
///
/// Called by both the unified pipeline ([`run_dispatch_pipeline`]) and the
/// dot-repeat replay path ([`crate::editor::Editor::replay_dot`]).
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
pub(in crate::editor) fn run_native_body(
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
        MappableCommand::Motion {
            fun, around_fun, ..
        }
        | MappableCommand::Selection {
            fun, around_fun, ..
        } => {
            let fun = if state
                .buffers
                .get(buf)
                .overrides
                .word_selects_whitespace(&state.settings)
            {
                around_fun.unwrap_or(fun)
            } else {
                fun
            };
            doc_ops::apply_doc_motion(
                &state.buffers,
                &mut state.panes.state,
                focused,
                buf,
                |b, s| fun(b, s, count, motion_mode),
            );
        }
        MappableCommand::Edit { fun, .. } => {
            // `apply_doc_edit` itself routes into the grouped path when an
            // edit group is already open (insert session or dot-repeat
            // replay), so the edit composes into the open group rather than
            // creating a standalone undo revision.
            doc_ops::apply_doc_edit(
                &mut state.buffers,
                &state.config.decorations,
                &mut state.panes.state,
                focused,
                buf,
                fun,
            );
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
pub(in crate::editor) fn step_paste_commit(
    state: &mut EditorState,
    view: &EngineView,
    defers: bool,
) {
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
pub(in crate::editor) fn step_stamp_repeatable(
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
///   Establishes + extend                          → append step
///   Establishes + move + in-place                 → reset + push establish
///   Establishes + move + reaching (or collapsed)  → clear
///   Composes (any mode)                           → append step
///   Untracked                                     → clear
///
/// Reaching motions (`select-next-word` / `-prev-word` / WORD variants) are
/// not recorded in Move mode: replaying such a step advances past the cursor,
/// causing dot-repeat to act on the wrong word. Extend steps of reaching
/// motions (`Ctrl+w`) are still recorded — extending grows an existing
/// selection by a relative amount and is safe to replay.
///
/// `Composes` (`copy-selection-on-next-line`/`-prev-line`) always appends: it
/// transforms whatever extent is already staged rather than establishing one,
/// so replaying it alone from a fresh cursor would rebuild nothing.
// `&Cow` not `&str`: `.clone()` must preserve Borrowed (built-ins) or Owned
// (Steel) without an unconditional heap alloc.
#[allow(clippy::ptr_arg)]
pub(super) fn step_update_recipe(
    state: &mut EditorState,
    meta: &CmdMeta,
    name: &Cow<'static, str>,
    ctx: &CmdCtx,
    char_arg: Option<char>,
) {
    state.selection_recipe_writes += 1;
    match meta.selection_tracking {
        SelectionTracking::Composes => {
            state.selection_recipe.push(SelectionStep {
                command: name.clone(),
                count: ctx.count.unwrap_or(1),
                char_arg,
                extend: ctx.extend,
            });
        }
        SelectionTracking::Establishes if ctx.extend => {
            state.selection_recipe.push(SelectionStep {
                command: name.clone(),
                count: ctx.count.unwrap_or(1),
                char_arg,
                extend: true,
            });
        }
        SelectionTracking::Establishes if !meta.is_motion && !meta.reaching => {
            state.selection_recipe.clear();
            state.selection_recipe.push(SelectionStep {
                command: name.clone(),
                count: ctx.count.unwrap_or(1),
                char_arg,
                extend: false,
            });
        }
        SelectionTracking::Establishes | SelectionTracking::Untracked => {
            state.selection_recipe.clear();
        }
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
pub(in crate::editor) fn run_dispatch_pipeline(
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
        !(meta.repeatable && meta.selection_tracking != SelectionTracking::Untracked),
        "command '{name}' is both repeatable and selection-tracking — \
         step_stamp_repeatable and step_update_recipe would both fire",
    );
    // BEFORE
    step_paste_commit(state, view, meta.defers_paste_commit);
    let pre_jump = step_capture_pre_jump(state, view, &meta);
    let char_arg = state.pending_char;
    let pre_recipe = step_snapshot_recipe(state, meta.repeatable);

    // BODY — cmd moved in; meta + name captured above so no further clone needed.
    run_native_body(state, view, cmd, ctx.count, ctx.extend);

    // AFTER
    step_record_jump(state, view, pre_jump, meta.is_jump);
    step_stamp_repeatable(state, &name, ctx.count.unwrap_or(1), char_arg, pre_recipe);
    step_update_recipe(state, &meta, &name, &ctx, char_arg);
    step_clear_extend(state, meta.clears_extend);
}
