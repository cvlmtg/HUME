//! Register/kill-ring paste: `p`/`P`, smart paste, and `[`/`]` ring cycling.
//!
//! Distinct from terminal bracketed-paste (`Event::Paste`,
//! `input.rs`), an unrelated feature that happens to
//! share the word "paste".
//!
//! This module owns opening and closing `EditorState::paste_session`
//! (`land_register_paste`, `commit_paste_session`) via
//! `edit_session::open_paste`, which applies the same open rule as
//! `edit_session::open_insert` does for an Insert session. A `p`/`P` opens
//! its session when it lands, not when it is planned, so nothing that runs
//! while it waits for its `on-paste` handlers can end the session it lands
//! in.

use std::borrow::Cow;
use std::rc::Rc;

use hume_core::{BufferId, Count, PaneId, Severity};
use hume_engine::pipeline::EngineView;

use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::state::EditState;
use hume_ops::MotionMode;
use hume_ops::edit::{paste_after, paste_before};
use hume_ops::register::{BLACK_HOLE_REGISTER, KILL_RING_REGISTER, Piece};

use super::super::{EditorState, doc_ops, register_ops};
use super::FocusedPane;
use crate::editor::edit_session::{self, OpeningPaste};
use crate::editor::error::CommandError;
use crate::editor::paste_plan::{PasteApply, PlannedPaste, Replaced};
use crate::editor::replay::{AnswerPlace, ReplayedPaste};
use crate::editor::settings::ClipboardJoinsRing;

/// The moment a clipboard take-in is attempted at, which
/// `clipboard-joins-ring` admits or refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TakeIn {
    /// A bare smart paste.
    Paste,
    /// A kill-ring push or a clipboard-register write.
    Edit,
}

// ── EditorState helpers ───────────────────────────────────────────────────────

impl EditorState {
    /// Push a bare or `"k`-prefixed capture onto the kill ring, after taking
    /// in a clipboard HUME has not seen, so the capture is the head and the
    /// outside copy sits below it.
    pub(in crate::editor) fn capture_to_ring(&mut self, yanked: Vec<Piece>) {
        self.take_in_clipboard_at_edit();
        self.kill_ring.push(yanked);
    }

    /// [`Self::take_in_clipboard`] at [`TakeIn::Edit`]. A clipboard that
    /// cannot be read leaves the edit to proceed without a warning: arboard
    /// reports a clipboard holding an image as a failed read, and every `d`
    /// would warn.
    pub(super) fn take_in_clipboard_at_edit(&mut self) {
        let _ = self.take_in_clipboard(TakeIn::Edit);
    }

    /// Push the clipboard onto the kill ring when it holds text HUME has not
    /// seen (see [`hume_ops::register::RegisterSet::take_in_clipboard`]) and
    /// `clipboard-joins-ring` admits `at`. Nothing is read when it does not.
    ///
    /// Returns the warning when the clipboard could not be read.
    #[must_use]
    fn take_in_clipboard(&mut self, at: TakeIn) -> Option<String> {
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
    /// Reads the session's own recorded pane/buffer (`PasteSession::pane`/
    /// `buffer`), not live focus or `&EngineView`. A paste session only
    /// stays open on the focused pane (`land_register_paste`), but focus can
    /// move again before this runs, and the session's own record is what
    /// stays correct regardless. Thin wrapper around
    /// [`edit_session::commit_paste_group`], the same commit
    /// `doc_ops::apply_doc_edit` uses to close a same-pane Paste session
    /// before an unrelated direct edit.
    pub(in crate::editor) fn commit_paste_session(&mut self) {
        crate::editor::edit_session::commit_paste_group(
            &mut self.buffers,
            &self.panes.entries,
            &mut self.paste_session,
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

/// A planned `p`/`P` or `[`/`]`.
pub(in crate::editor) struct RegisterPaste {
    /// The pieces resolved for the paste, which the kill ring's cycle and
    /// `.` keep whatever the handlers answer.
    pub(in crate::editor) values: Vec<Piece>,
    /// The handlers' text, one piece per selection, when one answered.
    pub(in crate::editor) answered: Option<Vec<Piece>>,
    pub(in crate::editor) lands: RegisterLanding,
}

/// Where a register paste lands.
pub(in crate::editor) enum RegisterLanding {
    /// `p`/`P`: opens a paste session for `opening` at the selections it was
    /// planned on, which undo returns to `undo_sels`.
    Opens {
        opening: OpeningPaste,
        undo_sels: SelectionSet,
    },
    /// `[`/`]`: in the paste session it was planned in, replacing what that
    /// session has pasted so far.
    Cycles,
}

/// Land `paste` in `buf` through `pane`, which is open, at the selections
/// `replaced` holds. `Err`, with nothing changed, when the buffer's text
/// may not change, when a `p`/`P` finds a session open, and when a `[`/`]`
/// finds its session gone. A session left open on a pane that is not the
/// focused one showing `buf` is committed at once: a paste session only
/// stays open there.
pub(in crate::editor) fn land_register_paste(
    state: &mut EditorState,
    view: &EngineView,
    pane: PaneId,
    buf: BufferId,
    replaced: &Replaced,
    paste: RegisterPaste,
) -> Result<(), CommandError> {
    let RegisterPaste {
        values,
        answered,
        lands,
    } = paste;
    doc_ops::text_gate(&state.buffers, buf)?;
    let (before, answer) = match lands {
        RegisterLanding::Opens { opening, undo_sels } => {
            edit_session::check_can_open(state.open_session())?;
            let before = opening.before;
            let sels = replaced.sels().clone();
            state
                .panes
                .entries
                .seeded_mut(pane, buf)
                .set_selections(sels.clone(), state.buffers.get(buf).text());
            let buffers = &state.buffers;
            edit_session::open_paste(
                state.input.insert_session(),
                &mut state.paste_session,
                pane,
                buf,
                opening,
                || buffers.get(buf).begin_edit_group(sels, undo_sels),
            )?;
            state.kill_ring.seed_cycle(&values);
            (before, None)
        }
        RegisterLanding::Cycles => {
            let OpeningPaste { before, answer, .. } = state
                .paste_session
                .as_ref()
                .filter(|s| s.owned_by(pane, buf))
                .map(|s| s.opening().clone())
                .ok_or_else(|| {
                    CommandError::new(
                        "paste dropped: its paste session ended while it waited for its on-paste handlers",
                    )
                })?;
            (before, answer)
        }
    };
    let pasted = answered.as_deref().unwrap_or(&values);
    let paste_fn = if before { paste_before } else { paste_after };
    let (buffers, mut stores) = state.edit_parts();
    doc_ops::apply_doc_edit_regrouped(buffers, &mut stores, pane, buf, |s| paste_fn(s, pasted))?;
    if let Some(at) = &answer {
        state.replace_paste_answer(at, values);
    }
    let shown = view.focused_pane() == pane && view.panes[pane].buffer_id() == buf;
    if !shown {
        state.commit_paste_session();
    }
    Ok(())
}

/// Plan `values` to land in `buf` through `pane` as `lands` says;
/// `replaced` is what the selections covered before the paste session's first
/// paste.
fn plan_register_paste(
    state: &EditorState,
    pane: PaneId,
    buf: BufferId,
    replaced: Rc<Replaced>,
    values: Vec<Piece>,
    lands: RegisterLanding,
) -> PlannedPaste {
    let apply = PasteApply::Register(RegisterPaste {
        values,
        answered: None,
        lands,
    });
    PlannedPaste::new(&state.buffers, pane, buf, replaced, apply)
}

/// A resolved paste, ready for [`do_paste`] to execute.
struct ResolvedPaste {
    values: Vec<Piece>,
    /// No `"<reg>` prefix was given.
    bare: bool,
    /// Where `.` recorded these values, when a `.` replays them.
    recorded: Option<AnswerPlace>,
}

/// Core paste implementation: plans `resolved` at `sels`, which undo
/// returns to `undo_sels` (the selections the command saw), and records it
/// for `.`. Landing opens the paste/ring-cycle session and seeds the ring
/// cycle ([`land_register_paste`]). Does not resolve the source or apply the
/// repeat-vs-swap rule. Callers resolve the source and (for smart paste)
/// collapse `sels` before calling in.
///
/// `before`: true for `P` (paste before), false for `p` (paste after).
fn do_paste(
    state: &mut EditorState,
    focused: PaneId,
    buf: BufferId,
    before: bool,
    resolved: ResolvedPaste,
    sels: SelectionSet,
    undo_sels: SelectionSet,
) -> PlannedPaste {
    let ResolvedPaste {
        values,
        bare,
        recorded,
    } = resolved;
    let answer = recorded.or_else(|| state.record_paste_answer(&values, bare));
    let replaced = Rc::new(Replaced::new(state.buffers.get(buf).text(), sels));
    let opening = OpeningPaste {
        before,
        answer,
        replaced: Rc::clone(&replaced),
    };
    let lands = RegisterLanding::Opens { opening, undo_sels };
    plan_register_paste(state, focused, buf, replaced, values, lands)
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
) -> Result<PlannedPaste, CommandError> {
    let (pid, buf) = (fp.pid(), fp.bid(view));
    // Checked before resolving, which takes the register prefix and may
    // take in the clipboard; landing checks again.
    edit_session::check_can_open(state.open_session())?;
    super::ensure_writable(state, view, fp.pane())?;
    let resolved = resolve(state, resolve_plain_bare)?;
    let sels = state
        .panes
        .entries
        .shown(&view.panes, pid)
        .selections()
        .clone();
    Ok(do_paste(
        state,
        pid,
        buf,
        before,
        resolved,
        sels.clone(),
        sels,
    ))
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
) -> Result<PlannedPaste, CommandError> {
    let (pid, buf) = (fp.pid(), fp.bid(view));
    edit_session::check_can_open(state.open_session())?;
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
    Ok(do_paste(state, pid, buf, before, resolved, base, sels))
}

/// Paste after the selection: plain paste, kill-ring head by default.
pub(in crate::editor) fn cmd_paste_after(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<PlannedPaste, CommandError> {
    do_normal_paste(state, view, fp, false)
}

/// Paste before the selection: plain paste, kill-ring head by default.
pub(in crate::editor) fn cmd_paste_before(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<PlannedPaste, CommandError> {
    do_normal_paste(state, view, fp, true)
}

/// Smart-paste after the selection. See [`resolve_smart_bare`].
pub(in crate::editor) fn cmd_smart_paste_after(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<PlannedPaste, CommandError> {
    do_smart_paste(state, view, fp, false)
}

/// Smart-paste before the selection. See [`resolve_smart_bare`].
pub(in crate::editor) fn cmd_smart_paste_before(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<PlannedPaste, CommandError> {
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
) -> Result<PlannedPaste, CommandError> {
    let (focused, buf) = (fp.pid(), fp.bid(view));
    let Some(replaced) = state
        .paste_session
        .as_ref()
        .filter(|s| s.owned_by(focused, buf))
        .map(|s| Rc::clone(&s.opening().replaced))
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
    Ok(plan_register_paste(
        state,
        focused,
        buf,
        replaced,
        values,
        RegisterLanding::Cycles,
    ))
}

/// Cycle the kill ring one step older and re-paste from the session snapshot.
pub(in crate::editor) fn cmd_paste_ring_older(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<PlannedPaste, CommandError> {
    do_paste_cycle(state, view, fp, true)
}

/// Cycle the kill ring one step newer and re-paste from the session snapshot.
pub(in crate::editor) fn cmd_paste_ring_newer(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<PlannedPaste, CommandError> {
    do_paste_cycle(state, view, fp, false)
}
