//! Insert-mode session lifecycle: entering/exiting Insert as a repeatable
//! action, with the undo group, typed run, and autoindent state that
//! entails. What `.` replays is recorded elsewhere (`replay/`).

use hume_editing::selection::{EditView, Facing, Selection, SelectionView};
use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_editing::tracked::Tracked;
use hume_engine::pipeline::EngineView;
use hume_rope::cluster::ClusterRange;
use hume_rope::line::ContentLine;
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::editor::EditorState;
use crate::editor::doc_ops;
use crate::editor::edit_session::{self, AutoIndent, InsertSession, TypedRun};
use crate::editor::error::CommandError;
use crate::editor::input_stack::InsertEntry;
use hume_ops::edit::{ArmedIndent, clear_blank_line_indent};

use super::{FocusedPane, doc, ensure_writable, pane_view};

/// `true` if any current selection is a collapsed cursor sitting on a blank
/// line whose whitespace this session itself auto-inserted: the condition
/// under which [`clear_blank_line_indent`] would actually change the buffer.
/// Checked before calling it so the common case (exiting Insert mode away
/// from a blank line, or a blank line whose indent isn't this session's own)
/// skips the edit entirely instead of running an identity one (see
/// [`hume_ops::edit::owned_blank_indent`]'s doc comment). Takes `allowed` from
/// the caller rather than re-deriving it via `autoindent_owned` itself, so
/// [`tear_down_insert`] computes that `Vec` once and reuses it for both the
/// gate check and the edit closure.
fn has_blank_line_cursor(sels: EditView<'_>, allowed: &[ExclusiveRange<CharOffset>]) -> bool {
    sels.iter().enumerate().any(|(i, sel)| {
        sel.is_cursor()
            && hume_ops::edit::owned_blank_indent(
                sels.text(),
                sel.head().offset(),
                allowed.get(i).copied(),
            )
            .is_some()
    })
}

/// The live session's per-selection autoindent record (see
/// `InsertSession::autoindent`'s doc) for `sels`, the session pane's
/// selections. `None` if none was armed, or its length differs from the
/// live selection count (a mid-session merge; same rule
/// [`tear_down_insert`] applies to `typed_run` via `valid_run`). Every
/// reader of the record goes through this, so they agree on when it
/// applies.
fn autoindent_record<'s>(insert: &'s InsertSession, sels: &EditView<'_>) -> Option<&'s AutoIndent> {
    insert
        .autoindent
        .as_ref()
        .and_then(|record| record.get(sels.text()))
        .filter(|record| record.ranges().len() == sels.len())
}

/// The ranges of [`autoindent_record`], or empty when it is `None`. An
/// out-of-bounds index into the returned vec (via `.get(i)`) then reads as
/// "no record for this selection", same as an empty vec would.
///
/// Owned rather than borrowed: every caller needs it cloned out of the
/// session before running the edit whose `ChangeSet` will remap (or, for
/// Enter, replace) that same record.
pub(in crate::editor) fn autoindent_owned(
    insert: &InsertSession,
    sels: &EditView<'_>,
) -> Vec<ExclusiveRange<CharOffset>> {
    autoindent_record(insert, sels).map_or_else(Vec::new, |record| record.ranges().to_vec())
}

/// Each selection's [`autoindent_record`] entry, its range with the text
/// armed there, or empty when there is no record.
pub(super) fn autoindent_armed<'s>(
    insert: &'s InsertSession,
    sels: &EditView<'_>,
) -> Vec<ArmedIndent<'s>> {
    autoindent_record(insert, sels).map_or_else(Vec::new, |record| {
        record
            .ranges()
            .iter()
            .zip(record.armed())
            .map(|(&range, text)| ArmedIndent { range, text })
            .collect()
    })
}

/// Record each current selection's freshly auto-inserted indent (`[line_start,
/// head)` on the post-edit buffer, and its text) as this session's own, so
/// a later Enter or exit knows that whitespace is its own to vacate and a
/// typed closing token knows the line is still as auto-indented (see
/// `InsertSession::autoindent`'s doc). Called once, right after the
/// structural newline + indent lands, by every entry point that breaks a
/// line (`o`, `O`, Enter), into the session `entry` proves open. The record
/// ends at the head, not the line's indent end: whitespace the break
/// carried down from the line it split stays the user's.
pub(in crate::editor) fn arm_autoindent(
    state: &mut EditorState,
    view: &EngineView,
    entry: &InsertEntry,
    fp: FocusedPane,
) {
    let text = doc(state, view, fp.pane()).text();
    let entries: Vec<_> = pane_view(state, view, fp.pane())
        .iter()
        .map(|sel| {
            let head = sel.head();
            let line_start = text.lines().start(text.char_to_line(head.offset()));
            armed_entry(
                text,
                ExclusiveRange::new(line_start.offset(), head.offset()),
            )
        })
        .collect();
    let record = Tracked::new(AutoIndent::new(entries), text);
    state.input.entered_mut(entry).autoindent = Some(record);
}

/// One selection's autoindent record entry: `range` and the text it holds.
fn armed_entry(
    text: &BufferText,
    range: ExclusiveRange<CharOffset>,
) -> (ExclusiveRange<CharOffset>, String) {
    (range, String::from(text.slice(range)))
}

/// Re-arm the selections at `rewritten` (sorted by selection index, each
/// with the line the dedent rewrote) with that line's current indent, so
/// the line stays this session's own and typing on is checked against the
/// new whitespace. Every other selection keeps its record. The record is
/// the whole leading whitespace: after the rewrite it is all the rendered
/// indent.
///
/// # Panics
/// If the session has no record that applies to the pane's selections.
/// The rewritten lines came from that record, and the dedent keeps every
/// selection, so it still applies.
pub(super) fn rearm_autoindent_lines(
    state: &mut EditorState,
    view: &EngineView,
    entry: &InsertEntry,
    fp: FocusedPane,
    rewritten: &[(usize, ContentLine)],
) {
    let text = doc(state, view, fp.pane()).text();
    let sels = pane_view(state, view, fp.pane());
    let record = autoindent_record(state.input.entered(entry), &sels)
        .expect("the dedent's candidates came from this record, and it keeps every selection");
    let entries: Vec<_> = sels
        .iter()
        .zip(record.ranges().iter().zip(record.armed()))
        .map(|(sel, (&range, armed))| {
            if let Ok(at) = rewritten.binary_search_by_key(&sel.index(), |&(index, _)| index) {
                let (_, line) = rewritten[at];
                armed_entry(
                    text,
                    ExclusiveRange::new(
                        text.lines().start(line).offset(),
                        text.lines().indent_end(line).offset(),
                    ),
                )
            } else {
                (range, armed.clone())
            }
        })
        .collect();
    let record = Tracked::new(AutoIndent::new(entries), text);
    state.input.entered_mut(entry).autoindent = Some(record);
}

/// Where an *empty* typed run's cursor lands on exit. See
/// `InsertSession::step_back_on_exit`'s doc. Passed to [`begin_typed_run`]
/// so every entry command arms it in the same call that pins the run,
/// instead of a separate step that could be forgotten or reordered.
pub(super) enum ExitCursor {
    /// `a`/`A`/`o`/`O`: step one grapheme back, so e.g. `a<Esc>` round-trips.
    StepBack,
    /// `i`/`I`/`c`: leave the cursor exactly where typing left it.
    StayPut,
}

/// Pin each current selection's head as an insertion anchor, and arm where
/// the cursor lands if nothing ends up typed: the whole lifecycle of a
/// session's "typed run".
///
/// `mii` (`select-last-insertion`) recovers the span regardless of the
/// `insert-select-on-exit` setting; `tear_down_insert` reads that setting
/// itself to decide whether to auto-select on exit.
///
/// Pins into the session `entry` proves open. Call after the cursor has been
/// positioned at the insertion point (for `o`/`O`, after the structural
/// newline has been inserted), so the anchor marks the start of typed text
/// only, never the newline or the pre-edit selection.
///
/// Every later text change carries the pinned run
/// (`InsertSession::carry_marks`); a cursor-motion command during the session
/// clears it (`with_jump`, `jump_list.rs`). An entry command dispatched from
/// inside an already-open session (a Steel `call!`, an Insert-mode
/// keybinding) replaces the run and the step-back flag the previous entry
/// armed.
pub(super) fn begin_typed_run(
    state: &mut EditorState,
    view: &EngineView,
    entry: &InsertEntry,
    fp: FocusedPane,
    exit: ExitCursor,
) {
    let heads: Vec<CharOffset> = pane_view(state, view, fp.pane())
        .iter()
        .map(|s| s.head().offset())
        .collect();
    // `ends` starts equal to `anchors` (an empty run) and is pushed
    // forward only by actual insertions (see `TypedRun::ends`'s own doc).
    let run = Tracked::new(
        TypedRun {
            ends: heads.clone(),
            anchors: heads,
        },
        doc(state, view, fp.pane()).text(),
    );
    let insert = state.input.entered_mut(entry);
    insert.typed_run = Some(run);
    insert.step_back_on_exit = matches!(exit, ExitCursor::StepBack);
}

/// Enter Insert mode as a repeatable insert action.
///
/// Refused if the focused buffer is read-only. Clears any
/// pending `"<reg>` prefix: `i`/`a`/`o` aren't operators, so a register spec
/// typed just before one names nothing to write into (unlike `d`/`c`/`p`,
/// which `ensure_writable` clears it for on the assumption the command
/// consumed it). It's cleared unconditionally so a read-only refusal and a
/// normal session agree: a register spec applies only to the command
/// immediately after `"`.
///
/// `cmd_change` (`c`) is the one caller for which this would be wrong: it's
/// itself a genuine register-consuming operator that delegates its mode
/// switch here before its own `state.route_kill` reads the prefix. See
/// [`begin_insert_session_preserving_register`], which it calls instead.
pub(super) fn begin_insert_session(
    state: &mut EditorState,
    view: &EngineView,
    fp: FocusedPane,
    place: impl FnOnce(EditState) -> EditState,
) -> Result<InsertEntry, CommandError> {
    state.register_prefix = None;
    begin_insert_session_preserving_register(state, view, fp, place)
}

/// [`begin_insert_session`] without clearing `register_prefix` first, for a
/// caller that is itself about to consume it. Do not call this for a plain
/// `i`/`a`/`o` entry; use [`begin_insert_session`], which clears the prefix
/// as those commands require.
///
/// `place` moves the cursors to where typing starts. It runs after every
/// refusal (read-only, another pane's session) and before the undo group
/// opens, so a refused entry moves nothing and undo restores the placed
/// selections. Returns the entry the rest of the command's setup takes.
pub(super) fn begin_insert_session_preserving_register(
    state: &mut EditorState,
    view: &EngineView,
    fp: FocusedPane,
    place: impl FnOnce(EditState) -> EditState,
) -> Result<InsertEntry, CommandError> {
    ensure_writable(state, view, fp.pane())?;
    let (pid, bid) = (fp.pid(), fp.bid(view));
    // Already open here means this entry runs inside a live Insert session
    // (a binding re-entering Insert), whose group and layer it joins.
    // Otherwise open one or, under `replay_dot`, take over its `Replay`-kind
    // placeholder (`edit_session::open_insert`), so the replayed session
    // folds into the same undo revision as the rest of the replay.
    let joined = state
        .input
        .insert_entry()
        .filter(|entry| state.input.entered(entry).owned_by(pid, bid));
    if joined.is_none() {
        edit_session::check_can_open(state.open_session(), pid, bid)?;
    }
    super::apply_pane_motion(state, view, fp.pane(), place);
    if let Some(entry) = joined {
        return Ok(entry);
    }
    // Keep the change whose code opened this session alive for the whole
    // session, so its keystrokes record and a repeatable command mid-session
    // can't replace it. `None` outside a recorded change's code (during `.`,
    // or a session a hook opens).
    let recording = state.open_scope_source();
    let session = crate::editor::edit_session::begin_edit_group(
        &state.buffers,
        &state.panes.entries,
        state.input.insert_session(),
        &mut state.active_session,
        pid,
        bid,
        recording,
    )?;
    Ok(state.push_insert(view, session))
}

/// Exit Insert mode: truncates the `Insert` layer, running
/// [`tear_down_insert`] via `EditorState::tear_down`. A no-op if no `Insert`
/// layer is open: `cmd_exit_insert` is a registered mappable command, so
/// `(call! "exit-insert" pane)` can reach here from any mode (a hook, a timer, an
/// async LSP callback), not only from a key path that already proved
/// `Insert` is current. Truncating `state.input.mode_layer()` unconditionally
/// would cancel whatever mode layer happened to be current. A `prompt!`
/// session reached this way would lose its callback silently, since teardown
/// never fires one.
pub(in crate::editor) fn end_insert_session(state: &mut EditorState, view: &EngineView) {
    if let Some(r) = state
        .input
        .ref_of::<crate::editor::input_stack::InsertLayer>()
    {
        state.truncate_layers(view, r);
    }
}

/// The bookkeeping that runs when the `Insert` layer leaves the stack, for
/// any reason (Esc, Ctrl-c, a mouse click, a Steel-triggered mode change),
/// called from `EditorState::tear_down`'s `Insert` arm, never directly.
/// Finalises the undo and typed-run state; does not itself touch the stack (the
/// truncate that got here already removed the layer).
///
/// Acts on `session`, the one the leaving layer owned, and on the `(pane,
/// buffer)` it recorded, never on focus, which makes teardown correct
/// regardless of whether focus has already moved by the time the layer
/// pops. Works on
/// `pane_state`/`buffers` directly through `doc_ops` rather than through a
/// `CommandPane`/`FocusedPane`: this is not a command body, and those types
/// can only be minted by resolving a target, not from a bare pair.
pub(in crate::editor) fn tear_down_insert(state: &mut EditorState, mut session: InsertSession) {
    let (pid, bid) = (session.pane(), session.buffer());
    // Before the autoindent-trim edit below, which must not itself be swept
    // into a capture's net edit: it's session-teardown bookkeeping, not part
    // of whatever the capture was recorded for.
    state.end_insert_recording(&mut session);
    // An open completion session lives in its own `Completion` layer, pushed
    // above `Insert`: the top-first `truncate_layers` call that reaches
    // this function always removes `Completion` (running its own `tear_down`
    // arm) before it removes `Insert`, regardless of which of Insert's many
    // exit paths (Esc/Enter inside the session's own handler, Ctrl-c, a
    // mouse click, a Steel-triggered mode change) triggered it. Nothing
    // completion-related belongs in this function.
    // Vim autoindent parity: trim a blank auto-indented line's whitespace
    // before committing, so leaving Insert mode on one vacates it the way a
    // second Enter does. Joins the still-open session group, not a separate
    // undo step. `allowed` re-derives ownership from the buffer (via
    // `autoindent_owned`) rather than trusting a flag, and
    // `has_blank_line_cursor` also skips the edit in the common case
    // (cursor not on a line this session owns) rather than running an
    // identity one on every Insert-mode exit.
    let sels = state
        .panes
        .entries
        .seeded(pid, bid)
        .view(state.buffers.get(bid).text());
    let allowed = autoindent_owned(&session, &sels);
    if has_blank_line_cursor(sels, &allowed) {
        let (buffers, mut stores) = state.closing_insert_parts(&mut session);
        if let Err(e) = doc_ops::apply_doc_edit_grouped(buffers, &mut stores, pid, bid, move |s| {
            clear_blank_line_indent(s, &allowed)
        }) {
            state.report_error(&e);
        }
    }
    // Every insert entry pins one typed run via `begin_typed_run`, so
    // reconstruct each selection's typed span here via `typed_span`. A count
    // mismatch (selections merged mid-session, e.g. via Backspace) drops the
    // run entirely: `spans` stays `None`, so this session contributes
    // nothing to the `mii` stash and, for an empty run, falls back to
    // `exit_cursor`'s step-back handling below.
    let (typed_run, step_back, kill_opened) = (
        session.typed_run.take(),
        session.step_back_on_exit,
        session.kill_opened_session,
    );
    let sel_count = state
        .panes
        .entries
        .seeded(pid, bid)
        .view(state.buffers.get(bid).text())
        .len();
    crate::editor::edit_session::commit_insert(&mut state.buffers, &state.panes.entries, session);
    // `cmd_change` stamped `PasteStamp` right after the deletion, but every
    // keystroke since has bumped `edit_seq`. Refresh the stamp to the
    // session's final `seq` (source unchanged) so `c <text> <Esc> p` still
    // reads the ring. See `InsertSession::kill_opened_session`'s doc.
    if kill_opened && let Some(stamp) = state.paste_stamp.as_mut() {
        stamp.seq = state.buffers.edit_seq();
    }
    let valid_run = typed_run
        .and_then(|run| run.into_inner(state.buffers.get(bid).text()))
        .filter(|r| r.anchors.len() == sel_count);
    let spans: Option<Vec<Option<ClusterRange>>> = valid_run.map(|run| {
        let text = state.buffers.get(bid).text();
        run.anchors
            .iter()
            .zip(run.ends.iter())
            .map(|(&anchor, &run_end)| typed_span(text, anchor, run_end))
            .collect()
    });

    // Stash whatever was actually typed for `mii`, regardless of entry
    // command, independent of `insert-select-on-exit` below, which only
    // decides whether Esc *also* selects it immediately.
    if let Some(spans) = &spans {
        let stashed: Vec<ClusterRange> = spans.iter().flatten().copied().collect();
        if !stashed.is_empty() {
            let last = Tracked::new(stashed, state.buffers.get(bid).text());
            state.buffer_positions.last_inserts.insert(bid, last);
        }
    }

    let select_on_exit = state
        .buffers
        .get(bid)
        .overrides
        .insert_select_on_exit(&state.settings);
    let spans = spans.filter(|_| select_on_exit);
    if spans.is_some() || step_back {
        doc_ops::apply_doc_motion(
            &state.buffers,
            &mut state.panes.entries,
            pid,
            bid,
            move |st| {
                let mut spans = spans.into_iter().flatten();
                st.map(|sel| match spans.next().flatten() {
                    Some(r) => Selection::covering(r, Facing::Forward),
                    None => exit_cursor(sel, step_back),
                })
            },
        );
    }
}

/// The clusters typed in one selection's run, `[anchor, run_end)`, or
/// `None` if nothing typed survives. Reads the run's end (not the live cursor
/// head; see `TypedRun::ends`'s doc for why) and drops any trailing `\n`
/// clusters, which are line terminators, not typed content. Only clusters
/// wholly inside the run count: a typed combining mark that merged with a
/// pre-existing base char leaves nothing of its own to select.
fn typed_span(text: &BufferText, anchor: CharOffset, run_end: CharOffset) -> Option<ClusterRange> {
    let mut span = text.within(ExclusiveRange::new(anchor, run_end))?;
    while text.char_at(span.last().offset()) == Some('\n') {
        span = ClusterRange::between(text.full_slice(), span.start(), span.last().into())?;
    }
    Some(span)
}

/// Where a selection's cursor lands when its typed run is empty: the entry
/// command's own exit position. `a`/`A`/`o`/`O` step one cluster back so
/// `a<Esc>` is a round trip; the line-start guard keeps that from crossing
/// onto the previous line. `i`/`I`/`c` never set `step_back`, so the head is
/// kept.
fn exit_cursor(sel: SelectionView<'_>, step_back: bool) -> Selection {
    let head = sel.head();
    if !step_back {
        return Selection::cursor(head);
    }
    let at_line_start = head == sel.text().lines().start(sel.head_line());
    match sel.text().clusters().prev(head.into()) {
        Some(prev) if !at_line_start => Selection::cursor(prev),
        _ => Selection::cursor(head),
    }
}
