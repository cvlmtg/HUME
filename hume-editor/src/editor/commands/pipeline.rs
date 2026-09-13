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
use crate::editor::registry::{CmdMeta, MappableCommand, SelectionBody, SelectionTracking};
use crate::editor::replay::{RepeatableAction, SelectionStep};
use crate::editor::settings::ObjectJumpAlign;
use crate::editor::{EditorState, Mode};
use hume_ops::{MotionMode, WordCtx};

use crate::editor::syntax::ensure_syntax_current;

use super::structural::object_spans;
use super::{current_selections, doc, effective_word_chars, focused_buffer_id};

// ── Native dispatch funnel ──────────────────────────────────────────────────

/// Wraps a native `MappableCommand` variant's body (`SelectionBody`, the
/// `Edit` fn pointer, or [`EditorCmdFn`](crate::editor::registry::EditorCmdFn))
/// so that destructuring `MappableCommand::Motion { fun, .. }` (or any of its
/// three siblings) anywhere outside this file yields an opaque value with no
/// way to call it. `.0` is readable only here, where it's defined — the one
/// place a native variant's body may actually run, wrapped by every
/// post-dispatch bookkeeping step [`run_dispatch_pipeline`] composes around it
/// (paste-session commit, jump-list update, dot-repeat recording). A second
/// naked match on `fun` elsewhere would silently drop that whole cluster.
///
/// No public accessor by design: [`Self::new`] is the only part of this type
/// that registration code outside `commands` (`registry/defaults/`) ever
/// touches. A private field is enforced by the compiler everywhere, tests
/// included — where a source-scanning lint checking for the destructuring
/// pattern by hand would miss one `rustfmt` wraps across lines, and would
/// skip `tests/` directories by construction.
#[derive(Clone, Copy)]
pub(in crate::editor) struct NativeBody<F>(F);

impl<F> NativeBody<F> {
    pub(in crate::editor) fn new(body: F) -> Self {
        Self(body)
    }
}

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
/// `explicit_count == false` as "move by visual line" rather than buffer line.
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
    let focused = state.focus.id();
    match cmd {
        MappableCommand::Motion { fun, .. } | MappableCommand::Selection { fun, .. } => match fun.0
        {
            SelectionBody::Plain(fun) => {
                doc_ops::apply_doc_motion(
                    &state.buffers,
                    &mut state.panes.state,
                    focused,
                    buf,
                    |b, s| fun(b, s, count, motion_mode),
                );
            }
            SelectionBody::Word(fun) => {
                let doc = state.buffers.get(buf);
                let ctx = WordCtx {
                    mode: motion_mode,
                    around: doc.overrides.word_selects_whitespace(&state.settings),
                    chars: effective_word_chars(doc, &state.settings),
                };
                doc_ops::apply_doc_motion(
                    &state.buffers,
                    &mut state.panes.state,
                    focused,
                    buf,
                    |b, s| fun(b, s, count, ctx),
                );
            }
            SelectionBody::Structural(body) => {
                // Bring the tree up to date before collecting spans from it:
                // `settle`'s async reparse tick only posts a request, and the
                // worker may still be parsing it when this runs — most
                // reliably during macro replay, which dispatches the next
                // key faster than tree-sitter finishes — so a stale tree
                // would yield wrong spans (or a panic on an out-of-range
                // byte offset).
                ensure_syntax_current(state, buf);
                // Collected before `apply_doc_motion`'s call below, which
                // needs `&state.buffers` and `&mut state.panes.state` at
                // once — `ObjectSpans` is owned precisely so its tree borrow
                // ends here, before that call.
                let spans = object_spans(state.buffers.get(buf), body);
                doc_ops::apply_doc_motion(
                    &state.buffers,
                    &mut state.panes.state,
                    focused,
                    buf,
                    |t, s| body.apply(t, s, count, motion_mode, &spans),
                );
            }
        },
        MappableCommand::Edit { fun, .. } => {
            // `apply_doc_edit` itself routes into the grouped path when an
            // edit group is already open (insert session or dot-repeat
            // replay), so the edit composes into the open group rather than
            // creating a standalone undo revision.
            doc_ops::apply_doc_edit(
                &mut state.buffers,
                &state.config.decorations,
                &mut state.panes.state,
                &mut state.panes.jumps,
                focused,
                buf,
                fun.0,
            );
        }
        MappableCommand::EditorCmd { fun, .. } => {
            if let Err(e) = fun.0(state, view, count, motion_mode) {
                // Reported at the error's own severity (e.g. search's "no
                // match" is transient — statusline only; an I/O failure is
                // logged) — see `CommandError::new` vs `::transient`. Every
                // Err still stamps `command_refused` regardless of severity:
                // rollback is about whether an edit happened, not about how
                // loudly the failure is recorded.
                state.report(e.severity(), e.message().to_owned());
                state.command_refused = true;
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
pub(in crate::editor::commands::pipeline) fn step_capture_pre_jump(
    state: &EditorState,
    view: &EngineView,
    meta: &CmdMeta,
) -> Option<(Selection, hume_rope::line::ContentLine, BufferId)> {
    meta.moves_cursor().then(|| jump_position(state, view))
}

/// Invalidate a still-open Insert-mode typed run before a cursor-motion
/// command runs — it would otherwise select across text the cursor jumped
/// away from once Insert exits. Covers every route into a native command: a
/// key press, a Steel `call!`, a hook, `run_command_sync`.
///
/// Gated on `state.mode() == Mode::Insert`, checked in BEFORE against the
/// *pre-body* mode. `exit-insert` itself needs no special-casing — it
/// registers with no `.jump()`/`.visual_move()` (`registry/defaults/
/// editor_cmds.rs`), so `moves_cursor()` is `false` and it never reaches
/// here — but placing this check in AFTER instead would read the *post-body*
/// mode, which is already `Insert` again for every entry command
/// (`i`/`a`/`o`/`c`/…) by the time their own body returns, and would wipe
/// the pins `begin_typed_run` just installed (same hazard `step_clear_extend`
/// documents for its own AFTER placement).
///
/// Two routes into a native command bypass this pipeline entirely, and both
/// are already safe without it: Insert mode's `Edit`-command short-circuit
/// (`mappings/insert.rs`) has a meta that hardcodes all three motion flags
/// `false`, so `moves_cursor()` would answer `false` here too; dot-repeat
/// replay (`replay.rs`) calls `run_native_body` directly, but reopens an
/// edit group first, which clears the pins itself
/// (`doc_ops::begin_edit_group`). See `CmdMeta::moves_cursor`'s doc for the
/// `SteelBacked`/`Lazy` blind spot this inherits unchanged: a user-bound
/// Steel motion still leaves the pins in place.
pub(in crate::editor::commands::pipeline) fn step_clear_typed_run(
    state: &mut EditorState,
    view: &EngineView,
    meta: &CmdMeta,
) {
    if state.mode() != Mode::Insert || !meta.moves_cursor() {
        return;
    }
    let pid = state.focus.id();
    let bid = focused_buffer_id(state, view);
    state.panes.state[pid][bid].typed_run = None;
}

/// The primary selection, its line, and the focused buffer — what a jump
/// entry is built from and what `step_record_jump` compares against, before
/// and after a command runs.
fn jump_position(
    state: &EditorState,
    view: &EngineView,
) -> (Selection, hume_rope::line::ContentLine, BufferId) {
    let bid = focused_buffer_id(state, view);
    let primary = current_selections(state, view).primary();
    let line = doc(state, view).text().char_to_line(primary.head());
    (primary, line, bid)
}

/// Snapshot selection recipe before body for dot-repeat recording.
///
/// The snapshot captures the selection extent the user built before the edit,
/// so `.` can re-establish it.  Inner dispatches (Steel `call!`) may overwrite
/// `selection_recipe` during the body; the snapshot is taken before they run.
pub(in crate::editor::commands::pipeline) fn step_snapshot_recipe(
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
/// past the threshold. Returns whether the cursor actually moved — `false`
/// for a command with no pre-jump snapshot at all (`step_capture_pre_jump`
/// returned `None`, i.e. `meta.moves_cursor()` was false) as well as for a
/// snapshotted one that turned out to be a no-op.
///
/// `moved` guards both branches: `JumpList::push` truncates forward history
/// unconditionally, so a `jump: true` command that happens to be a no-op on
/// this press (e.g. `#` on plain text, `goto-first-line` already on line 1)
/// must not push at all, not just skip the threshold check. Compares the
/// whole `Selection`, not just `head` — the entry being guarded stores the
/// whole thing (anchor included), and `select-all` from the buffer's own
/// last char moves only the anchor, leaving `head` unchanged.
///
/// `step_align_view` reuses this same `moved` rather than recomputing
/// `jump_position` a second time.
pub(in crate::editor::commands::pipeline) fn step_record_jump(
    state: &mut EditorState,
    view: &EngineView,
    pre_jump: Option<(Selection, hume_rope::line::ContentLine, BufferId)>,
    is_jump: bool,
) -> bool {
    let Some((pre_primary, pre_line, pre_bid)) = pre_jump else {
        return false;
    };
    let (post_primary, post_line, post_bid) = jump_position(state, view);
    let moved = post_bid != pre_bid || post_primary != pre_primary;
    if moved && (is_jump || pre_line.abs_diff(post_line) > state.settings.jump_line_threshold) {
        state.panes.jumps[state.focus.id()].push(JumpEntry::from_pre_motion(
            pre_primary,
            pre_line,
            pre_bid,
        ));
    }
    moved
}

/// Re-align the viewport after a forward object jump (`}`,
/// `goto-next-<kind>`), per `EditorSettings::object_jump_align`.
///
/// `Top`/`Center` delegate to `view_top`/`view_center` verbatim — the same
/// primitives `z k`/`z z` call — so there is exactly one implementation of
/// "put the head at this viewport row". `moved` is `step_record_jump`'s
/// result: a `}` press already on the last paragraph is a no-op on the
/// selection and must not yank the viewport around on every repeated press.
pub(in crate::editor::commands::pipeline) fn step_align_view(
    state: &mut EditorState,
    view: &mut EngineView,
    aligns_view: bool,
    moved: bool,
) {
    if !aligns_view || !moved {
        return;
    }
    match state.settings.object_jump_align {
        ObjectJumpAlign::Off => {}
        ObjectJumpAlign::Top => super::view_top(state, view),
        ObjectJumpAlign::Center => super::view_center(state, view),
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
///   Extends + extend                    → append step
///   Extends + move                      → clear (no replayable extent)
///   Establishes/Composes + no change    → leave the recipe as-is
///   Establishes + move (+ change)       → reset + push establish
///   Establishes + extend (+ change)     → append step
///   Composes (+ change)                 → append step
///   Untracked                           → clear
///
/// `Extends` is every `Motion`, including the word motions (`select-next-word`
/// et al.): their Move-mode result *looks* replayable (it lands on a selected
/// word) but isn't — replaying it would advance past the intended word rather
/// than rebuild it (see `SelectionTracking::Extends`). Extend-mode steps are
/// still recorded: extending grows an existing selection by a relative amount
/// and is safe to replay.
///
/// `selection_changed` (the pre- vs. post-body selection set, computed by the
/// caller) gates `Establishes`/`Composes`: a command that found no match
/// (`select-all-matches`) or no surrounding pair (`ms(`) established nothing
/// of its own, so it must leave whatever recipe a prior command staged
/// untouched — neither resetting it nor appending a step that would replay
/// as another no-op.
// `&Cow` not `&str`: `.clone()` must preserve Borrowed (built-ins) or Owned
// (Steel) without an unconditional heap alloc.
#[allow(clippy::ptr_arg)]
pub(in crate::editor::commands::pipeline) fn step_update_recipe(
    state: &mut EditorState,
    meta: &CmdMeta,
    name: &Cow<'static, str>,
    ctx: &CmdCtx,
    selection_changed: bool,
) {
    state.selection_recipe_writes += 1;
    if meta.selection_tracking == SelectionTracking::Untracked {
        state.selection_recipe.clear();
        return;
    }
    // A Move-mode motion has no replayable extent to restart the recipe with.
    if meta.selection_tracking == SelectionTracking::Extends && !ctx.extend {
        state.selection_recipe.clear();
        return;
    }
    if !selection_changed {
        return;
    }
    // A Move-mode establish restarts the recipe.
    if meta.selection_tracking == SelectionTracking::Establishes && !ctx.extend {
        state.selection_recipe.clear();
    }
    state.selection_recipe.push(SelectionStep {
        command: name.clone(),
        count: ctx.count.unwrap_or(1),
        extend: ctx.extend,
    });
}

/// Exit sticky Extend mode after a selection-consuming edit.
///
/// Mirrors the "done selecting" signal of `;` (collapse) and Vim's visual-mode
/// operator exit. No-op unless the editor is currently in Extend — a `change`
/// command (which already entered Insert) will not be affected because
/// `state.mode()` is `Insert` by the time the AFTER block runs.
pub(in crate::editor::commands::pipeline) fn step_clear_extend(
    state: &mut EditorState,
    clears_extend: bool,
) {
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
    // and a selection-builder (a pure cursor movement) — step_stamp_repeatable
    // and step_update_recipe would both fire. This is a property of the
    // registry, fixed at registration time, so it's checked once for every
    // command by `registry::tests::no_command_is_both_repeatable_and_selection_tracking`
    // rather than re-probed here on every dispatch.
    // BEFORE
    state.command_refused = false;
    step_paste_commit(state, view, meta.defers_paste_commit);
    step_clear_typed_run(state, view, &meta);
    let pre_jump = step_capture_pre_jump(state, view, &meta);
    let char_arg = state.pending_char;
    let pre_recipe = step_snapshot_recipe(state, meta.repeatable);
    // Only snapshot the selection when step_update_recipe could push a step —
    // a Move-mode Motion (the overwhelming majority of keypresses) always
    // clears without needing one. Cloning here, not comparing, since the body
    // below mutates the live selection set in place.
    let needs_selection_snapshot = meta.selection_tracking != SelectionTracking::Untracked
        && (ctx.extend || meta.selection_tracking != SelectionTracking::Extends);
    let pre_sels = needs_selection_snapshot.then(|| current_selections(state, view).clone());

    // BODY — cmd moved in; meta + name captured above so no further clone needed.
    run_native_body(state, view, cmd, ctx.count, ctx.extend);

    // AFTER
    let moved = step_record_jump(state, view, pre_jump, meta.is_jump);
    step_align_view(state, view, meta.aligns_view, moved);
    // A refused/errored body has nothing new to repeat — see
    // `EditorState::command_refused`. `pre_recipe` is simply dropped, not
    // restored into `state.selection_recipe`: every repeatable command is
    // `SelectionTracking::Untracked` (enforced by
    // `registry::tests::no_command_is_both_repeatable_and_selection_tracking`),
    // so `step_update_recipe` below clears it unconditionally regardless of
    // this branch — restoring it first would be immediately undone.
    if !state.command_refused {
        step_stamp_repeatable(state, &name, ctx.count.unwrap_or(1), char_arg, pre_recipe);
    }
    // A command whose own snapshot is `None` never reaches the `!selection_changed`
    // early return in step_update_recipe, so `true` here is inert filler.
    let selection_changed = match &pre_sels {
        Some(pre) => *pre != *current_selections(state, view),
        None => true,
    };
    step_update_recipe(state, &meta, &name, &ctx, selection_changed);
    step_clear_extend(state, meta.clears_extend);
}
