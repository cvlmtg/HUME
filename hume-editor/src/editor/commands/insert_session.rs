//! Insert-mode session lifecycle: entering/exiting Insert as a repeatable
//! action, with the undo group and dot-repeat bookkeeping that entails.

use hume_editing::selection::Selection;
use hume_editing::text::BufferText;
use hume_engine::pipeline::EngineView;

use crate::editor::buffer::LastInsert;
use crate::editor::pane_state::TypedRun;
use crate::editor::replay::InsertSession;
use crate::editor::{EditorState, Mode};
use hume_ops::edit::clear_blank_line_indent;

use super::{
    apply_focused_edit_grouped, apply_focused_motion, begin_edit_group_current,
    commit_edit_group_current, current_selections, doc, focused_buffer_id, refuse_if_read_only,
};

/// `true` when the focused (pane, buffer) has an open edit group.
fn is_group_open_current(state: &EditorState, view: &EngineView) -> bool {
    let bid = focused_buffer_id(state, view);
    state.panes.state[state.focused_pane_id][bid]
        .edit_group
        .is_some()
}

/// `true` if any current selection is a collapsed cursor sitting on a blank,
/// auto-indented line — the condition under which [`clear_blank_line_indent`]
/// would actually change the buffer. Checked before calling it so the common
/// case (exiting Insert mode away from a blank line) skips the edit entirely
/// instead of running an identity one (see
/// [`hume_ops::edit::blank_line_ws_range`]'s doc comment).
pub(super) fn has_blank_line_cursor(state: &EditorState, view: &EngineView) -> bool {
    let text = doc(state, view).text();
    current_selections(state, view).iter_sorted().any(|sel| {
        sel.is_collapsed() && hume_ops::edit::blank_line_ws_range(text, sel.head()).is_some()
    })
}

/// Where an *empty* typed run's cursor lands on exit — see
/// `PaneBufferState::step_back_on_exit`'s doc. Passed to [`begin_typed_run`]
/// so every entry command arms it in the same call that pins the run,
/// instead of a separate step that could be forgotten or reordered.
pub(super) enum ExitCursor {
    /// `a`/`A`/`o`/`O`: step one grapheme back, so e.g. `a<Esc>` round-trips.
    StepBack,
    /// `i`/`I`/`c`: leave the cursor exactly where typing left it.
    StayPut,
}

/// Pin each current selection's head as an insertion anchor, and arm where
/// the cursor lands if nothing ends up typed — the whole lifecycle of a
/// session's "typed run".
///
/// `mii` (`select-last-insertion`) recovers the span regardless of the
/// `select-inserted-text` setting; `end_insert_session` reads that setting
/// itself to decide whether to auto-select on exit.
///
/// No-op if no edit group is open (read-only buffer, where
/// `begin_insert_session` already refused to enter Insert) — `exit` is
/// correctly not armed either in that case. Call after the cursor has been
/// positioned at the insertion point — for `o`/`O`, after the structural
/// newline has been inserted — so the anchor marks the start of typed text
/// only, never the newline or the pre-edit selection.
///
/// `apply_doc_edit_grouped` (doc_ops.rs) maps the pinned run through every
/// subsequent grouped edit; a cursor-motion command during the session
/// clears it (`step_clear_typed_run`, `commands/pipeline.rs`); a fresh
/// `begin_edit_group` clears it (and resets `step_back_on_exit`) too, so a
/// later session never inherits a stale run or a stale step-back flag — the
/// entry command dispatched from inside an already-open session (a Steel
/// `call!`, an Insert-mode keybinding) would otherwise inherit whatever the
/// previous entry armed.
pub(super) fn begin_typed_run(state: &mut EditorState, view: &EngineView, exit: ExitCursor) {
    if !is_group_open_current(state, view) {
        return;
    }
    let heads: Vec<usize> = current_selections(state, view)
        .iter_sorted()
        .map(|s| s.head())
        .collect();
    let pid = state.focused_pane_id;
    let bid = focused_buffer_id(state, view);
    let pbs = &mut state.panes.state[pid][bid];
    // `ends` starts equal to `anchors` — an empty run — and is pushed
    // forward only by actual insertions (see `TypedRun::ends`'s own doc).
    pbs.typed_run = Some(TypedRun {
        ends: heads.clone(),
        anchors: heads,
    });
    pbs.step_back_on_exit = matches!(exit, ExitCursor::StepBack);
}

/// Enter Insert mode as a repeatable insert action.
///
/// No-op (with a warning) if the focused buffer is read-only. Clears any
/// pending `"<reg>` prefix: `i`/`a`/`o` aren't operators, so a register spec
/// typed just before one names nothing to write into (unlike `d`/`c`/`p`,
/// which `refuse_if_read_only` clears it for on the assumption the command
/// consumed it) — cleared unconditionally so a read-only refusal and a
/// normal session agree, matching Vim, where the spec applies only to the
/// operator immediately after `"`.
///
/// `cmd_change` (`c`) is the one caller for which this would be wrong: it's
/// itself a genuine register-consuming operator that delegates its mode
/// switch here before its own `state.route_kill` reads the prefix — see
/// [`begin_insert_session_preserving_register`], which it calls instead.
pub(super) fn begin_insert_session(state: &mut EditorState, view: &EngineView) {
    state.register_prefix = None;
    begin_insert_session_preserving_register(state, view);
}

/// [`begin_insert_session`] without clearing `register_prefix` first — for a
/// caller that is itself about to consume it. Do not call this for a plain
/// `i`/`a`/`o` entry; use [`begin_insert_session`], which clears the prefix
/// as those commands require.
pub(super) fn begin_insert_session_preserving_register(state: &mut EditorState, view: &EngineView) {
    if refuse_if_read_only(state, view) {
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
        });
    }
    // Outside the guard above (unlike `insert_session`) so replay — which
    // skips session creation — still starts each replayed session with no
    // pending auto-indent to vacate, matching a fresh interactive session.
    state.autoindent_pending = false;
    state.set_mode(Mode::Insert);
}

/// Exit Insert mode and finalise the undo/repeat state.
pub(in crate::editor) fn end_insert_session(state: &mut EditorState, view: &EngineView) {
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
    // Every insert entry pins one typed run via `begin_typed_run` —
    // reconstruct each selection's typed span here via `typed_span`. A count
    // mismatch (selections merged mid-session, e.g. via Backspace) drops the
    // run entirely — `spans` stays `None`, so this session contributes
    // nothing to the `mii` stash and, for an empty run, falls back to
    // `exit_cursor`'s step-back handling below.
    let (typed_run, step_back, kill_opened) = {
        let pid = state.focused_pane_id;
        let bid = focused_buffer_id(state, view);
        let pbs = &mut state.panes.state[pid][bid];
        (
            pbs.typed_run.take(),
            std::mem::take(&mut pbs.step_back_on_exit),
            std::mem::take(&mut pbs.kill_opened_session),
        )
    };
    // `cmd_change` stamped `PasteStamp` right after the deletion, but every
    // keystroke since has bumped `edit_seq` — refresh the stamp to the
    // session's final `seq` (source unchanged) so `c <text> <Esc> p` still
    // reads the ring. See `PaneBufferState::kill_opened_session`'s doc.
    if kill_opened && let Some(stamp) = state.paste_stamp.as_mut() {
        stamp.seq = state.buffers.edit_seq();
    }
    let sel_count = current_selections(state, view).len();
    let valid_run = typed_run.filter(|r| r.anchors.len() == sel_count);
    let spans: Option<Vec<Option<(usize, usize)>>> = valid_run.map(|run| {
        let text = doc(state, view).text();
        run.anchors
            .iter()
            .zip(run.ends.iter())
            .map(|(&anchor, &run_end)| typed_span(text, anchor, run_end))
            .collect()
    });

    // Stash whatever was actually typed for `mii`, regardless of entry
    // command — independent of `select-inserted-text` below, which only
    // decides whether Esc *also* selects it immediately.
    if let Some(spans) = &spans {
        let stashed: Vec<(usize, usize)> = spans.iter().flatten().copied().collect();
        if !stashed.is_empty() {
            let bid = focused_buffer_id(state, view);
            let buf = state.buffers.get_mut(bid);
            let text_gen = buf.text_gen;
            buf.last_insert = Some(LastInsert {
                spans: stashed,
                text_gen,
            });
        }
    }

    let select_on_exit = doc(state, view)
        .overrides
        .select_inserted_text(&state.settings);
    let spans = spans.filter(|_| select_on_exit);
    if spans.is_some() || step_back {
        apply_focused_motion(state, view, move |b, sels| {
            let mut spans = spans.into_iter().flatten();
            sels.map(|sel| match spans.next().flatten() {
                Some((anchor, end)) => Selection::new(anchor, end),
                None => exit_cursor(b, sel.head(), step_back),
            })
        });
    }
    state.set_mode(Mode::Normal);
}

/// The selected typed span `(anchor, end]` — inclusive of `end` — for one
/// selection, or `None` if nothing typed survives. Walks back from `run_end`
/// (not the live cursor head — see `TypedRun::ends`'s doc for why) over any
/// trailing `\n` graphemes, which are line terminators, not typed content,
/// to the grapheme immediately before whatever's left. Walking off the start
/// of the run (nothing typed, or only newlines were) yields `None`, never a
/// backwards or zero-width range.
fn typed_span(text: &BufferText, anchor: usize, run_end: usize) -> Option<(usize, usize)> {
    let mut cursor = run_end;
    loop {
        if cursor <= anchor {
            return None;
        }
        let prev = hume_editing::grapheme::prev_grapheme_boundary(text, cursor);
        // A typed combining mark can merge with a PRE-EXISTING base char
        // into one grapheme cluster, so the boundary before `cursor` can
        // land behind `anchor` in a single step rather than landing on it —
        // the loop guard above only catches `cursor <= anchor`, not a jump
        // past it. Nothing wholly inside the run is left to select.
        if prev < anchor {
            return None;
        }
        if text.char_at(prev) != Some('\n') {
            return Some((anchor, prev));
        }
        cursor = prev;
    }
}

/// Where a selection's cursor lands when its typed run is empty — the entry
/// command's own exit position. `a`/`A`/`o`/`O` step one grapheme back so
/// `a<Esc>` is a round trip; the line-start guard keeps that from crossing
/// onto the previous line. `i`/`I`/`c` never set `step_back`, so `head` is
/// returned unchanged.
fn exit_cursor(b: &BufferText, head: usize, step_back: bool) -> Selection {
    if !step_back {
        return Selection::collapsed(head);
    }
    let line_start = b.line_to_char(b.char_to_line(head).into());
    let new_head = if head > line_start {
        hume_editing::grapheme::prev_grapheme_boundary(b, head)
    } else {
        head
    };
    Selection::collapsed(new_head)
}
