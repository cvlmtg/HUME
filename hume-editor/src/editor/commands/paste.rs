//! Register/kill-ring paste: `p`/`P`, smart paste, and `[`/`]` ring cycling.
//!
//! Distinct from terminal bracketed-paste (`Event::Paste`,
//! `input.rs`), an unrelated feature that happens to
//! share the word "paste".
//!
//! This module owns opening/closing the `EditSessionKind::Paste` variant of
//! `EditorState::active_session` (`do_paste`, `commit_paste_session`) via
//! `edit_session::open_or_retarget`, which applies the same open rule as
//! `edit_session::open_insert` does for an Insert session.

use std::borrow::Cow;

use hume_core::{BufferId, Count, PaneId, Severity};
use hume_engine::pipeline::EngineView;

use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::state::EditState;
use hume_ops::MotionMode;
use hume_ops::edit::{paste_after, paste_before};
use hume_ops::register::{BLACK_HOLE_REGISTER, KILL_RING_REGISTER, Piece};

use super::super::{EditorState, doc_ops, register_ops};
use super::FocusedPane;
use crate::editor::edit_session::{self, EditSessionKind};
use crate::editor::error::CommandError;
use crate::editor::replay::{AnswerPlace, ReplayedPaste};
use crate::editor::settings::ClipboardJoinsRing;

/// The moment a clipboard take-in is attempted at, which
/// `clipboard-joins-ring` admits or refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum TakeIn {
    /// A bare smart paste.
    Paste,
    /// A kill-ring push or a clipboard-register write.
    Edit,
}

// ── EditorState helpers ───────────────────────────────────────────────────────

impl EditorState {
    /// Push a bare or `"k`-prefixed capture onto the kill ring, after taking
    /// in a clipboard HUME has not seen, so the capture is the head and the
    /// outside copy sits below it. A clipboard that cannot be read leaves
    /// the capture to proceed without a warning: arboard reports a clipboard
    /// holding an image as a failed read, and every `d` would warn.
    pub(in crate::editor) fn capture_to_ring(&mut self, yanked: Vec<Piece>) {
        self.take_in_clipboard(TakeIn::Edit);
        self.kill_ring.push(yanked);
    }

    /// Push the clipboard onto the kill ring when it holds text HUME has not
    /// seen (see [`hume_ops::register::RegisterSet::take_in_clipboard`]) and
    /// `clipboard-joins-ring` admits `at`. Nothing is read when it does not.
    ///
    /// Returns the warning when the clipboard could not be read.
    pub(in crate::editor) fn take_in_clipboard(&mut self, at: TakeIn) -> Option<String> {
        let admitted = match self.settings.clipboard_joins_ring {
            ClipboardJoinsRing::Always => true,
            ClipboardJoinsRing::Paste => at == TakeIn::Paste,
            ClipboardJoinsRing::Never => false,
        };
        if !admitted {
            return None;
        }
        match register_ops::take_in_clipboard(&mut self.registers, &mut self.clipboard) {
            Ok(new) => {
                if let Some(pieces) = new {
                    self.kill_ring.push(pieces);
                }
                None
            }
            Err(warn) => Some(warn),
        }
    }

    /// Commit the open paste session, if any.
    ///
    /// Records exactly one history revision for the entire paste + all cycles.
    /// Called before any non-`[`/`]` dispatch so the session is committed
    /// before undo, motions, or the next `p`/`P`.
    ///
    /// Reads the session's own recorded pane/buffer (`EditSession::pane`/
    /// `buffer`), not live focus or `&EngineView`. A paste session only
    /// ever opens on the focused pane (`do_paste`), but focus can move again
    /// before this runs, and the session's own record is what stays correct
    /// regardless. Thin wrapper around [`edit_session::commit_paste_group`], the
    /// same commit `doc_ops::apply_doc_edit` uses to close a same-pane Paste
    /// session before an unrelated direct edit.
    pub(in crate::editor) fn commit_paste_session(&mut self) {
        crate::editor::edit_session::commit_paste_group(
            &mut self.buffers,
            &self.panes.entries,
            &mut self.active_session,
        );
    }
}

// ── Paste ────────────────────────────────────────────────────────────────────

/// For a bare smart paste (`do_smart_paste` only): if every selection's text
/// matches `values` one-to-one, collapse the selections so the paste lands
/// next to the existing text instead of replacing it. Repeat-paste and
/// swap-paste are thus decided by content, not by the previous command.
///
/// Plain paste and register-explicit smart paste (`"Xp`) always replace, so
/// scripts get a predictable contract. The check is all-or-nothing: the paste
/// op applies one mode to every selection, so partial agreement falls back to
/// replace. Already-collapsed selections are unaffected either way.
fn collapse_if_repeat(state: EditState, values: &[Piece], before: bool) -> EditState {
    let view = state.view();
    let repeats = values.len() == view.len()
        && view
            .iter()
            .zip(values)
            .all(|(sel, v)| sel.slice() == v.text());
    if !repeats {
        return state;
    }
    state.map(|s| {
        if before {
            Selection::cursor(s.start())
        } else {
            Selection::cursor(s.last())
        }
    })
}

/// Paste `values` before or after `focused`'s selections in `buf`, replacing
/// what the open Paste session has pasted so far.
fn paste_values(
    state: &mut EditorState,
    focused: PaneId,
    buf: BufferId,
    before: bool,
    values: &[Piece],
) -> Result<(), CommandError> {
    let paste_fn = if before { paste_before } else { paste_after };
    let (buffers, mut stores) = state.edit_parts();
    doc_ops::apply_doc_edit_regrouped(buffers, &mut stores, focused, buf, |s| paste_fn(s, values))
}

/// A resolved paste, ready for [`do_paste`] to execute.
struct ResolvedPaste {
    values: Vec<Piece>,
    /// No `"<reg>` prefix was given.
    bare: bool,
    /// Where `.` recorded these values, when a `.` replays them.
    recorded: Option<AnswerPlace>,
}

/// Core paste implementation: applies `resolved` at `sels`, which undo
/// returns to `undo_sels` (the selections the command saw), opens the
/// paste/ring-cycle session, and seeds the ring cycle. Does not resolve the
/// source or apply the repeat-vs-swap rule. Callers resolve the source and
/// (for smart paste) collapse `sels` before calling in.
///
/// `before`: true for `P` (paste before), false for `p` (paste after).
///
/// `Err` when a real, already-open `Insert`/`Paste` session blocks this
/// (pane, buffer), reachable only through a `call!` (a hook or timer
/// firing mid-typing) or an Insert-mode binding, since `step_paste_commit`
/// closes a prior *paste* session before every ordinary dispatch. A
/// dot-repeat replay's own pre-opened `Replay`-kind placeholder (the common
/// case for a Steel `#:repeatable` wrapper that calls native paste) is not
/// a conflict: [`edit_session::open_or_retarget`] retargets it to `Paste`
/// in place instead.
fn do_paste(
    state: &mut EditorState,
    focused: PaneId,
    buf: BufferId,
    before: bool,
    resolved: ResolvedPaste,
    sels: SelectionSet,
    undo_sels: SelectionSet,
) -> Result<(), CommandError> {
    let ResolvedPaste {
        values,
        bare,
        recorded,
    } = resolved;
    let answer = recorded.or_else(|| state.record_paste_answer(&values, bare));

    let base = sels.clone();
    state
        .panes
        .entries
        .seeded_mut(focused, buf)
        .set_selections(sels, state.buffers.get(buf).text());
    let buffers = &state.buffers;
    edit_session::open_or_retarget(
        state.input.insert_session(),
        &mut state.active_session,
        focused,
        buf,
        EditSessionKind::Paste { before, answer },
        || buffers.get(buf).begin_edit_group(base, undo_sels),
    )?;
    paste_values(state, focused, buf, before, &values)?;

    // Every completed paste opens a fresh session (any prior one was already
    // committed in BEFORE by `step_paste_commit`), so every one reseeds the
    // cycle.
    state.kill_ring.seed_cycle(&values);
    Ok(())
}

/// The kill-ring head, or the refusal for an empty ring.
fn kill_ring_head(state: &EditorState) -> Result<Vec<Piece>, CommandError> {
    state
        .kill_ring
        .head()
        .map(<[Piece]>::to_vec)
        .ok_or_else(|| CommandError::transient("kill ring is empty"))
}

/// The refusal for a register with nothing in it.
fn empty_register(reg: char) -> CommandError {
    CommandError::transient(format!("register '{reg}' is empty"))
}

/// Resolve values for a fresh paste against an explicit `"<reg>` prefix.
/// An explicit register bypasses the smart-paste heuristic entirely.
/// `Err` when there is nothing to paste: the black-hole register, an empty
/// register, or a system clipboard that could not be read with nothing in
/// its in-memory mirror.
fn resolve_explicit_register(
    state: &mut EditorState,
    reg: char,
) -> Result<ResolvedPaste, CommandError> {
    let values = match reg {
        KILL_RING_REGISTER => kill_ring_head(state)?,
        BLACK_HOLE_REGISTER => return Err(empty_register(reg)),
        c => {
            // Digits and clipboard. Digits read in-memory RegisterSet (symmetric
            // with "Ny writes). Clipboard routes through the OS clipboard.
            let (cow, warn) =
                register_ops::read_register_text(&state.registers, &mut state.clipboard, c);
            let values = cow.map(Cow::into_owned); // end borrow of state.registers
            match (values, warn) {
                (Some(values), warn) => {
                    if let Some(w) = warn {
                        state.report(Severity::Warning, w);
                    }
                    values
                }
                (None, Some(w)) => return Err(CommandError::warning(w)),
                (None, None) => return Err(empty_register(c)),
            }
        }
    };
    Ok(ResolvedPaste {
        values,
        bare: false,
        recorded: None,
    })
}

/// During `.`, what the replayed paste pasted live, in place of its source:
/// a paste replays its text, never its register or the kill ring as they are
/// by then. `None` outside `.`, and when a register typed before `.` makes
/// the paste read that register live.
fn resolve_replayed(
    state: &mut EditorState,
    typed: bool,
) -> Option<Result<ResolvedPaste, CommandError>> {
    match state.replayed_paste(typed)? {
        Ok(ReplayedPaste::Recorded { pieces, bare, at }) => Some(Ok(ResolvedPaste {
            values: pieces,
            bare,
            recorded: Some(at),
        })),
        Ok(ReplayedPaste::Typed) => None,
        Err(refusal) => Some(Err(CommandError::new(format!("paste: {refusal}")))),
    }
}

/// Resolve values for a fresh paste: what a `.` replay recorded, the
/// `"<reg>` prefix's register, or `bare`'s values when no prefix was given.
/// `Err` when there is nothing to paste.
fn resolve(
    state: &mut EditorState,
    bare: fn(&mut EditorState) -> Result<ResolvedPaste, CommandError>,
) -> Result<ResolvedPaste, CommandError> {
    let (prefix, typed) = state.take_register_prefix_at_dot();
    if let Some(replayed) = resolve_replayed(state, typed) {
        return replayed;
    }
    match prefix {
        None => bare(state),
        Some(reg) => resolve_explicit_register(state, reg),
    }
}

/// Bare `paste-*` resolution: the kill-ring head. The clipboard is not taken
/// in.
fn resolve_plain_bare(state: &mut EditorState) -> Result<ResolvedPaste, CommandError> {
    Ok(ResolvedPaste {
        values: kill_ring_head(state)?,
        bare: true,
        recorded: None,
    })
}

/// Bare `smart-paste-*` resolution: the kill-ring head, after taking in a
/// clipboard HUME has not seen. With nothing on the ring, the refusal gives
/// the clipboard's own warning when it could not be read.
fn resolve_smart_bare(state: &mut EditorState) -> Result<ResolvedPaste, CommandError> {
    let warn = state.take_in_clipboard(TakeIn::Paste);
    let values = state
        .kill_ring
        .head()
        .map(<[Piece]>::to_vec)
        .ok_or_else(|| match warn {
            Some(w) => CommandError::warning(w),
            None => CommandError::transient("nothing to paste"),
        })?;
    Ok(ResolvedPaste {
        values,
        bare: true,
        recorded: None,
    })
}

/// Plain paste: resolve from the register (kill-ring head when bare, honoring
/// `"<reg>` otherwise), then hand off to [`do_paste`] unconditionally, which
/// always replaces a non-collapsed selection. See [`collapse_if_repeat`]'s
/// doc for why smart paste alone needs the extra step.
fn do_normal_paste(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    before: bool,
) -> Result<(), CommandError> {
    let (pid, buf) = (fp.pid(), fp.bid(view));
    // Checked here, not left to `do_paste`'s own `open_or_retarget` call,
    // which runs after `do_paste` has written the selections it pastes at:
    // a refusal here leaves the pane untouched.
    edit_session::check_can_open(state.open_session(), pid, buf)?;
    super::ensure_writable(state, view, fp.pane())?;
    let resolved = resolve(state, resolve_plain_bare)?;
    let sels = state
        .panes
        .entries
        .shown(&view.panes, pid)
        .selections()
        .clone();
    do_paste(state, pid, buf, before, resolved, sels.clone(), sels)
}

/// Smart paste: resolve the kill-ring head, after taking in a new clipboard
/// (see [`resolve_smart_bare`]), apply the repeat-vs-swap collapse rule to the
/// selections (bare paste only; see [`collapse_if_repeat`]), then hand off
/// to [`do_paste`].
fn do_smart_paste(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    before: bool,
) -> Result<(), CommandError> {
    let (pid, buf) = (fp.pid(), fp.bid(view));
    edit_session::check_can_open(state.open_session(), pid, buf)?;
    super::ensure_writable(state, view, fp.pane())?;
    let resolved = resolve(state, resolve_smart_bare)?;
    let pbs = state.panes.entries.shown(&view.panes, pid);
    let sels = pbs.selections().clone();
    let base = if resolved.bare {
        collapse_if_repeat(
            pbs.state(state.buffers.get(buf).text()),
            &resolved.values,
            before,
        )
        .into_selections()
    } else {
        sels.clone()
    };
    do_paste(state, pid, buf, before, resolved, base, sels)
}

/// Paste after the selection: plain paste, kill-ring head by default.
pub(in crate::editor) fn cmd_paste_after(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_normal_paste(state, view, fp, false)
}

/// Paste before the selection: plain paste, kill-ring head by default.
pub(in crate::editor) fn cmd_paste_before(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_normal_paste(state, view, fp, true)
}

/// Smart-paste after the selection. See [`resolve_smart_bare`].
pub(in crate::editor) fn cmd_smart_paste_after(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_smart_paste(state, view, fp, false)
}

/// Smart-paste before the selection. See [`resolve_smart_bare`].
pub(in crate::editor) fn cmd_smart_paste_before(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_smart_paste(state, view, fp, true)
}

/// Shared implementation for `[` and `]`: advance/retreat the kill-ring cycle
/// cursor and re-paste from the session snapshot.
///
/// Refused when no paste session is open in `fp` or when the cycle is
/// already at a boundary.
fn do_paste_cycle(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    older: bool,
) -> Result<(), CommandError> {
    let (focused, buf) = (fp.pid(), fp.bid(view));
    let Some((before, answer)) = state
        .active_session
        .as_ref()
        .filter(|s| s.owned_by(focused, buf))
        .and_then(|s| match s.kind() {
            EditSessionKind::Paste { before, answer } => Some((*before, answer.clone())),
            EditSessionKind::Replay => None,
        })
    else {
        return Err(CommandError::transient("no paste to cycle"));
    };
    // Eagerly convert to owned Vec so the borrow of state.kill_ring ends before
    // state.buffers and state.panes.entries are borrowed mutably below.
    let values = if older {
        state.kill_ring.cycle_older()
    } else {
        state.kill_ring.cycle_newer()
    }
    .map(|v| v.to_vec())
    .ok_or_else(|| {
        CommandError::transient(if older {
            "no older entry"
        } else {
            "no newer entry"
        })
    })?;
    paste_values(state, focused, buf, before, &values)?;
    if let Some(at) = &answer {
        state.replace_paste_answer(at, values);
    }
    Ok(())
}

/// Cycle the kill ring one step older and re-paste from the session snapshot.
pub(in crate::editor) fn cmd_paste_ring_older(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste_cycle(state, view, fp, true)
}

/// Cycle the kill ring one step newer and re-paste from the session snapshot.
pub(in crate::editor) fn cmd_paste_ring_newer(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste_cycle(state, view, fp, false)
}
