//! The native command dispatch pipeline: `run_native_body` executes a
//! command's own effect; the `step_*` functions are the bookkeeping
//! (dot-repeat, jump list, selection recipe, sticky extend) that wraps every
//! dispatch, composed into one pipeline by `run_dispatch_pipeline`.

use std::borrow::Cow;

use hume_editing::selection::Selection;
use hume_engine::pipeline::{BufferId, EngineView, PaneId};
use hume_scripting::PaneHandle;
use slotmap::SecondaryMap;

use crate::editor::pane_state::PaneBufferState;

use crate::editor::dispatch::CmdCtx;
use crate::editor::jump_list::JumpEntry;
use crate::editor::registry::{
    CmdMeta, EditorCmdBody, MappableCommand, SelectionBody, SelectionTracking, TargetCategory,
};
use crate::editor::replay::{RepeatableAction, SelectionStep};
use crate::editor::settings::ObjectJumpAlign;
use crate::editor::{EditorState, Mode};
use hume_ops::{MotionMode, WordCtx};

use crate::editor::syntax::ensure_syntax_current;

use super::structural::object_spans;
use super::{apply_pane_edit, apply_pane_motion, effective_word_chars, pane_selections};

// ── Command targets ─────────────────────────────────────────────────────────

/// A pane resolved as the target of a native command's Pane-category body,
/// or a kind-B builtin — the pane a `(call! "cmd" pane)` or a
/// pane-taking builtin acts through, which need not be the focused pane. The
/// private field means the only way to get one is through this module's own
/// resolution ([`resolve_focus`]/[`resolve_pane`]) or by narrowing a
/// [`FocusedPane`] — never by wrapping an arbitrary `PaneId` from outside.
///
/// `bid` is read live rather than cached at resolution time: a jump or
/// `goto-*-buffer` body switches the pane's own buffer, and the AFTER steps
/// (`step_record_jump`, `step_align_view`) must see that switch, not the
/// buffer the target was resolved for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) struct CommandPane(PaneId);

impl CommandPane {
    pub(in crate::editor) fn pid(self) -> PaneId {
        self.0
    }

    pub(in crate::editor) fn bid(self, view: &EngineView) -> BufferId {
        view.panes[self.0].buffer_id
    }

    /// `t`'s own seeded state — the shared tail every
    /// `pane_state[t.pid()][t.bid(view)]` hand-index reduces to. Panics with
    /// slotmap's own message if unseeded, which never happens for a
    /// resolved `CommandPane`: every pane creation or buffer switch seeds
    /// its `PaneBufferState` first.
    pub(in crate::editor) fn state<'a>(
        self,
        pane_state: &'a SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
        view: &EngineView,
    ) -> &'a PaneBufferState {
        &pane_state[self.0][self.bid(view)]
    }

    /// [`Self::state`]'s mutable counterpart.
    pub(in crate::editor) fn state_mut<'a>(
        self,
        pane_state: &'a mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
        view: &EngineView,
    ) -> &'a mut PaneBufferState {
        let bid = self.bid(view);
        &mut pane_state[self.0][bid]
    }
}

/// A [`CommandPane`] proven, by construction, to be the focused pane at the
/// moment it was minted. The single admission point for focus-bound state —
/// the open Insert/paste session, the sticky Extend flag, dot-repeat — so a
/// body that only received a `CommandPane` has no way to reach any of it: it
/// would need a `FocusedPane`, which it was never given.
///
/// [`Self::current`] is the *only* mint outside this pipeline's own
/// resolution — every non-body caller that legitimately needs "the focused
/// pane" (a typed command, an input layer, LSP goto, `Editor::
/// focused_buffer_id`) calls it explicitly at its own call site, rather than
/// reaching for `state.focus.id()` directly.
///
/// `Copy`, and valid only until the next focus change (`focus::focus_pane`) —
/// never held across one.
#[derive(Clone, Copy, Debug)]
pub(in crate::editor) struct FocusedPane(CommandPane);

impl FocusedPane {
    /// The one mint for focus-context code — see this type's own doc.
    pub(in crate::editor) fn current(state: &EditorState) -> Self {
        Self(CommandPane(state.focus.id()))
    }

    pub(in crate::editor) fn target(self) -> CommandPane {
        self.0
    }

    pub(in crate::editor) fn pid(self) -> PaneId {
        self.0.pid()
    }

    pub(in crate::editor) fn bid(self, view: &EngineView) -> BufferId {
        self.0.bid(view)
    }

    /// This pane as the `pane` half of a value handed back to Steel (a hook
    /// argument, a completion source's own pane) — the tuple always carries
    /// the focused pid, never bare focus state a callee would have to
    /// re-derive.
    pub(in crate::editor) fn handle(self, view: &EngineView) -> PaneHandle {
        PaneHandle::with_pane(self.bid(view), self.pid())
    }
}

/// A native command's target, resolved per [`TargetCategory`] before the
/// body runs. Pairs 1:1 with the [`EditorCmdBody`] variant (or, for
/// Motion/Selection/Edit, always [`Self::Pane`] — those variants have no
/// category field because they have exactly one) so `run_native_body`'s
/// match on `(body, target)` can treat a mismatch as unreachable: resolution
/// and dispatch both derive the category from the same source.
#[derive(Clone, Copy)]
pub(in crate::editor) enum ResolvedTarget {
    Pane(CommandPane),
    Focused(FocusedPane),
}

/// Why [`resolve_pane`] could not produce a [`ResolvedTarget`] for a
/// requested [`PaneHandle`]. `Display`ed by `EditorHostImpl::run_command_sync`
/// and by every kind-A/B builtin's host method — the places these reach a
/// user (via `%call-native!`'s `Err`, or a builtin's own raise).
#[derive(Debug, Clone, Copy)]
pub(in crate::editor) enum TargetError {
    /// The handle's buffer names no live buffer.
    Closed,
    /// The command/builtin needs a pane, and the handle carried none.
    NoPane,
    /// The handle's pane no longer exists.
    PaneClosed,
    /// The handle's pane exists but no longer shows its buffer.
    PaneShowsOther,
    /// The command/builtin needs the *focused* pane, and the handle's pane
    /// isn't it.
    NotFocused,
}

impl std::fmt::Display for TargetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => write!(f, "invalid buffer id"),
            Self::NoPane => write!(f, "needs a pane, but was given none"),
            Self::PaneClosed => write!(f, "pane has been closed"),
            Self::PaneShowsOther => write!(f, "pane no longer shows this buffer"),
            Self::NotFocused => write!(f, "acts on the focused pane, which this pane isn't"),
        }
    }
}

/// Whether a resolved target is the focused pane/buffer ([`Self::Focus`]) or
/// not ([`Self::Remote`]) — decides which BEFORE/AFTER bookkeeping steps in
/// [`run_resolved`] run. A `Remote` dispatch (a `call!` to a pane other than
/// what's focused) never touches the focused pane's paste session, typed
/// run, dot-repeat recipe, or Extend flag — those describe what the user is
/// doing in the pane they're looking at, not what a hook just did elsewhere.
#[derive(Clone, Copy)]
pub(in crate::editor) enum Scope {
    Focus(FocusedPane),
    Remote,
}

/// Resolve `cmd`'s target against the currently focused pane — used by the
/// keypress dispatch path, where "the buffer to act on" is always whatever
/// is focused. Infallible: every category has an answer when the target is
/// focus itself.
fn resolve_focus(
    state: &EditorState,
    _view: &EngineView,
    cmd: &MappableCommand,
) -> (ResolvedTarget, Scope) {
    let fp = FocusedPane::current(state);
    let target = match cmd.target_category() {
        TargetCategory::Pane => ResolvedTarget::Pane(fp.target()),
        TargetCategory::FocusedPane => ResolvedTarget::Focused(fp),
    };
    (target, Scope::Focus(fp))
}

/// Resolve `handle`'s pane, requiring it to be live and to still show
/// `handle`'s buffer — the check `resolve_pane`'s `Pane`/`FocusedPane`
/// categories share, and the shape every kind-B `CursorHost`/`EditHost`/…
/// builtin host method needs outright (as [`resolve_command_pane`]).
fn checked_pane(
    state: &EditorState,
    view: &EngineView,
    handle: PaneHandle,
) -> Result<CommandPane, TargetError> {
    let bid = handle.buffer();
    if state.buffers.try_get(bid).is_none() {
        return Err(TargetError::Closed);
    }
    let pid = handle.pane().ok_or(TargetError::NoPane)?;
    let Some(pane) = view.panes.get(pid) else {
        return Err(TargetError::PaneClosed);
    };
    if pane.buffer_id != bid {
        return Err(TargetError::PaneShowsOther);
    }
    Ok(CommandPane(pid))
}

/// Resolve a [`TargetCategory`] against an explicit [`PaneHandle`] — used by
/// `EditorHostImpl::run_command_sync` (`(call! "cmd" pane)`) and by every
/// kind-A/B builtin's host method. Per-category rule:
/// - `Global` ignores `handle` entirely, even if closed.
/// - `Pane`/`FocusedPane` go through [`checked_pane`] — `handle`'s buffer
///   must be live, `handle.pane()` must be `Some`, that pane must still
///   exist, and it must still show `handle.buffer()`. `FocusedPane` further
///   requires the resolved pane to be the focused one.
/// - `Buffer` requires `handle`'s buffer to be live, no pane involved.
pub(in crate::editor) fn resolve_pane(
    state: &EditorState,
    view: &EngineView,
    handle: PaneHandle,
    category: TargetCategory,
) -> Result<(ResolvedTarget, Scope), TargetError> {
    let focused = FocusedPane::current(state);
    match category {
        TargetCategory::Pane => {
            let target = checked_pane(state, view, handle)?;
            let scope = if target.pid() == focused.pid() {
                Scope::Focus(focused)
            } else {
                Scope::Remote
            };
            Ok((ResolvedTarget::Pane(target), scope))
        }
        TargetCategory::FocusedPane => {
            let target = checked_pane(state, view, handle)?;
            if target.pid() != focused.pid() {
                return Err(TargetError::NotFocused);
            }
            Ok((ResolvedTarget::Focused(focused), Scope::Focus(focused)))
        }
    }
}

/// [`checked_pane`], the shape every kind-B `CursorHost`/`EditHost`/… builtin
/// host method needs: just the resolved pane, not `resolve_pane`'s `Scope`
/// bookkeeping (only `run_resolved` cares about that).
pub(in crate::editor) fn resolve_command_pane(
    state: &EditorState,
    view: &EngineView,
    handle: PaneHandle,
) -> Result<CommandPane, TargetError> {
    checked_pane(state, view, handle)
}

/// [`checked_pane`] plus the focused-pane check — the shape every kind-A
/// builtin host method (a popup/menu/drawer/picker opener) needs.
pub(in crate::editor) fn resolve_focused_pane(
    state: &EditorState,
    view: &EngineView,
    handle: PaneHandle,
) -> Result<FocusedPane, TargetError> {
    let target = checked_pane(state, view, handle)?;
    let focused = FocusedPane::current(state);
    if target.pid() != focused.pid() {
        return Err(TargetError::NotFocused);
    }
    Ok(focused)
}

// ── Native dispatch funnel ──────────────────────────────────────────────────

/// Wraps a native `MappableCommand` variant's body (`SelectionBody`, the
/// `Edit` fn pointer, or an
/// [`EditorCmdBody`](crate::editor::registry::EditorCmdBody))
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

impl NativeBody<EditorCmdBody> {
    /// `self.0`'s own [`EditorCmdBody::category`] — the one place outside
    /// `registry/defaults/builder.rs` that can read `.0` at all, so
    /// `MappableCommand::EditorCmd::target_category` derives from this
    /// instead of caching a separate field that could drift from it.
    pub(in crate::editor) fn category(&self) -> TargetCategory {
        self.0.category()
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
fn run_native_body(
    state: &mut EditorState,
    view: &mut EngineView,
    cmd: MappableCommand,
    target: ResolvedTarget,
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
    match cmd {
        MappableCommand::Motion { fun, .. } | MappableCommand::Selection { fun, .. } => {
            let ResolvedTarget::Pane(t) = target else {
                unreachable!("Motion/Selection always resolve to a Pane target")
            };
            let buf = t.bid(view);
            match fun.0 {
                SelectionBody::Plain(fun) => {
                    apply_pane_motion(state, view, t, |b, s| fun(b, s, count, motion_mode));
                }
                SelectionBody::Word(fun) => {
                    let doc = state.buffers.get(buf);
                    let ctx = WordCtx {
                        mode: motion_mode,
                        around: doc.overrides.word_selects_whitespace(&state.settings),
                        chars: effective_word_chars(doc, &state.settings),
                    };
                    // Can't route through `apply_pane_motion` (takes `&mut
                    // EditorState` wholesale): `ctx.chars` borrows out of
                    // `state.buffers`, which must stay borrowed alongside
                    // the `&mut state.panes.state` the motion itself needs —
                    // exactly the disjoint-borrow case `apply_doc_motion`
                    // exists to take directly.
                    crate::editor::doc_ops::apply_doc_motion(
                        &state.buffers,
                        &mut state.panes.state,
                        t.pid(),
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
                    // Collected before `apply_pane_motion`'s call below, which
                    // needs `&state.buffers` and `&mut state.panes.state` at
                    // once — `ObjectSpans` is owned precisely so its tree borrow
                    // ends here, before that call.
                    let spans = object_spans(state.buffers.get(buf), body);
                    apply_pane_motion(state, view, t, |t2, s| {
                        body.apply(t2, s, count, motion_mode, &spans)
                    });
                }
            }
        }
        MappableCommand::Edit { fun, .. } => {
            let ResolvedTarget::Pane(t) = target else {
                unreachable!("Edit always resolves to a Pane target")
            };
            // `apply_pane_edit` itself routes into the grouped path when an
            // edit group is already open (insert session or dot-repeat
            // replay), so the edit composes into the open group rather than
            // creating a standalone undo revision. `Err` when another pane
            // holds one instead — same reporting shape as `EditorCmd`'s
            // `Result` two arms below.
            if let Err(e) = apply_pane_edit(state, view, t, fun.0) {
                state.report(e.severity(), e.message().to_owned());
                state.command_refused = true;
            }
        }
        MappableCommand::EditorCmd { fun, .. } => {
            // Every arm pairs a body with the target its own `EditorCmdBody`
            // variant demands — `resolve_focus`/`resolve_pane` derive
            // `target`'s variant from the same `target_category()` the
            // registry used to choose which `EditorCmdBody` arm to build, so
            // the two can't disagree. The catch-all is a debug invariant, not
            // load-bearing dispatch logic.
            let result = match (fun.0, target) {
                (EditorCmdBody::Pane(f), ResolvedTarget::Pane(t)) => {
                    f(state, view, t, count, motion_mode)
                }
                (EditorCmdBody::FocusedPane(f), ResolvedTarget::Focused(fp)) => {
                    f(state, view, fp, count, motion_mode)
                }
                _ => unreachable!(
                    "resolve_focus/resolve_pane derive the target from the same \
                     category the command was registered with"
                ),
            };
            if let Err(e) = result {
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

/// [`run_native_body`], resolved against the focused pane — the shape dot-repeat
/// replay ([`crate::editor::Editor::replay_dot`]) and the Insert-mode `Edit`
/// short-circuit (`input_stack/insert.rs`) both need: neither goes through
/// [`run_resolved`]'s bookkeeping, but both always act on focus.
pub(in crate::editor) fn run_native_body_on_focus(
    state: &mut EditorState,
    view: &mut EngineView,
    cmd: MappableCommand,
    count: Option<usize>,
    extend: bool,
) {
    let (target, _) = resolve_focus(state, view, &cmd);
    run_native_body(state, view, cmd, target, count, extend);
}

// ── Dispatch step functions ─────────────────────────────────────────────────

// ── Shared steps (used by both native and Steel dispatch paths) ──────────────

/// Commit paste session unless the command defers it (ring-cycle pastes).
pub(in crate::editor) fn step_paste_commit(state: &mut EditorState, defers: bool) {
    if !defers {
        state.commit_paste_session();
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
    t: CommandPane,
    meta: &CmdMeta,
) -> Option<(Selection, hume_rope::line::ContentLine, BufferId)> {
    meta.moves_cursor().then(|| jump_position(state, view, t))
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
/// (`input_stack/insert.rs`) has a meta that hardcodes all three motion flags
/// `false`, so `moves_cursor()` would answer `false` here too; dot-repeat
/// replay (`replay.rs`) calls `run_native_body_on_focus` directly, but
/// reopens an edit group first, which clears the pins itself
/// (`doc_ops::begin_edit_group`). See `CmdMeta::moves_cursor`'s doc for the
/// `SteelBacked`/`Lazy` blind spot this inherits unchanged: a user-bound
/// Steel motion still leaves the pins in place.
pub(in crate::editor::commands::pipeline) fn step_clear_typed_run(
    state: &mut EditorState,
    view: &EngineView,
    fp: FocusedPane,
    meta: &CmdMeta,
) {
    if state.mode() != Mode::Insert || !meta.moves_cursor() {
        return;
    }
    fp.target()
        .state_mut(&mut state.panes.state, view)
        .typed_run = None;
}

/// The primary selection, its line, and `t`'s buffer — what a jump entry is
/// built from and what `step_record_jump` compares against, before and after
/// a command runs. Reads `t`'s pane directly, not the focused pane: a Remote
/// dispatch's jump list belongs to the pane it actually ran in.
fn jump_position(
    state: &EditorState,
    view: &EngineView,
    t: CommandPane,
) -> (Selection, hume_rope::line::ContentLine, BufferId) {
    let bid = t.bid(view);
    let primary = t.state(&state.panes.state, view).selections().primary();
    let line = state.buffers.get(bid).text().char_to_line(primary.head());
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
    t: CommandPane,
) -> bool {
    let Some((pre_primary, pre_line, pre_bid)) = pre_jump else {
        return false;
    };
    let (post_primary, post_line, post_bid) = jump_position(state, view, t);
    let moved = post_bid != pre_bid || post_primary != pre_primary;
    if moved && (is_jump || pre_line.abs_diff(post_line) > state.settings.jump_line_threshold) {
        state.panes.jumps[t.pid()].push(JumpEntry::from_pre_motion(pre_primary, pre_line, pre_bid));
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
    pane: Option<CommandPane>,
    aligns_view: bool,
    moved: bool,
) {
    let (Some(t), true, true) = (pane, aligns_view, moved) else {
        return;
    };
    match state.settings.object_jump_align {
        ObjectJumpAlign::Off => {}
        ObjectJumpAlign::Top => super::view_top(state, view, t.pid()),
        ObjectJumpAlign::Center => super::view_center(state, view, t.pid()),
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
/// operator exit. `set_extend(false)` only ever writes `Base`'s own flag — a
/// `change` command (which already entered Insert by the time the AFTER
/// block runs) is unaffected either way, visibly or otherwise: `Base` isn't
/// the current mode layer while Insert is open, and `push_mode_layer`
/// already cleared the flag on the way in.
pub(in crate::editor::commands::pipeline) fn step_clear_extend(
    state: &mut EditorState,
    clears_extend: bool,
) {
    if clears_extend {
        state.input.set_extend(false);
    }
}

// ── Native dispatch pipeline (composed from step functions) ────────────────

/// Execute a native command through the full dispatch pipeline, targeting
/// the focused pane — the keypress path's shape, where "the buffer to act
/// on" is always whatever is focused.
///
/// Returns `false` if the body refused outright (see
/// `EditorState::command_refused`), `true` otherwise. Delegates to
/// [`run_resolved`]; see its doc for the pipeline itself.
pub(in crate::editor) fn run_dispatch_pipeline(
    state: &mut EditorState,
    view: &mut EngineView,
    cmd: MappableCommand,
    ctx: CmdCtx,
) -> bool {
    let (target, scope) = resolve_focus(state, view, &cmd);
    run_resolved(state, view, cmd, target, scope, ctx)
}

/// Execute a native command through the full dispatch pipeline against an
/// already-[`resolve_pane`]-resolved target — `EditorHostImpl::
/// run_command_sync`'s shape, where `(call! "cmd" bid)` may target a pane
/// other than the focused one.
///
/// Returns `false` if the body refused outright (see
/// `EditorState::command_refused`), `true` otherwise — `run_command_sync`
/// forwards this to Steel's `call!` as the outcome of the dispatch; the
/// keypress path (via [`run_dispatch_pipeline`]) ignores it, since a refusal
/// already reported itself via `state.report` inside the body.
///
/// Composed from the step functions above. Every BEFORE/AFTER step that
/// touches focus-bound state (paste session, typed run, dot-repeat recipe,
/// Extend flag) runs only under [`Scope::Focus`] — see this module's own doc
/// on [`Scope`]. Jump-list capture/record and view alignment run against
/// `target`'s own pane regardless of scope, when it has one.
pub(in crate::editor) fn run_resolved(
    state: &mut EditorState,
    view: &mut EngineView,
    cmd: MappableCommand,
    target: ResolvedTarget,
    scope: Scope,
    ctx: CmdCtx,
) -> bool {
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
    let pane = match target {
        ResolvedTarget::Pane(t) => Some(t),
        ResolvedTarget::Focused(fp) => Some(fp.target()),
    };

    // BEFORE
    state.command_refused = false;
    if let Scope::Focus(fp) = scope {
        step_paste_commit(state, meta.defers_paste_commit);
        step_clear_typed_run(state, view, fp, &meta);
    }
    let pre_jump = pane.and_then(|t| step_capture_pre_jump(state, view, t, &meta));
    let char_arg = state.pending_char;
    let pre_recipe = match scope {
        Scope::Focus(_) => step_snapshot_recipe(state, meta.repeatable),
        Scope::Remote => None,
    };
    // Only snapshot the selection when step_update_recipe could push a step —
    // a Move-mode Motion (the overwhelming majority of keypresses) always
    // clears without needing one. Cloning here, not comparing, since the body
    // below mutates the live selection set in place. `Scope::Remote` never
    // snapshots — the recipe belongs to the focused pane's dot-repeat state,
    // which a Remote dispatch must not touch (see this fn's own doc).
    let needs_selection_snapshot = meta.selection_tracking != SelectionTracking::Untracked
        && (ctx.extend || meta.selection_tracking != SelectionTracking::Extends);
    let pre_sels = match scope {
        Scope::Focus(fp) if needs_selection_snapshot => {
            Some(pane_selections(state, view, fp.target()).clone())
        }
        _ => None,
    };

    // BODY — cmd moved in; meta + name captured above so no further clone needed.
    run_native_body(state, view, cmd, target, ctx.count, ctx.extend);

    // AFTER
    let moved = pane
        .map(|t| step_record_jump(state, view, pre_jump, meta.is_jump, t))
        .unwrap_or(false);
    step_align_view(state, view, pane, meta.aligns_view, moved);
    if let Scope::Focus(fp) = scope {
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
            Some(pre) => *pre != *pane_selections(state, view, fp.target()),
            None => true,
        };
        step_update_recipe(state, &meta, &name, &ctx, selection_changed);
        step_clear_extend(state, meta.clears_extend);
    }
    !state.command_refused
}
