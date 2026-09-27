//! Register/kill-ring paste: `p`/`P`, smart paste, and `[`/`]` ring cycling.
//!
//! Distinct from terminal bracketed-paste (`Event::Paste`,
//! `mappings/bracketed_paste.rs`), an unrelated feature that happens to
//! share the word "paste".
//!
//! This module owns opening/closing the `EditSessionKind::Paste` variant of
//! `EditorState::active_session` (`do_paste`, `commit_paste_session`) via
//! `edit_session::open_or_retarget` (the same primitive
//! `doc_ops::begin_edit_group` uses for the `Insert` variant) rather than
//! `doc_ops::begin_edit_group`/`commit_edit_group` themselves, which are
//! typed to `Insert` alone: a paste session's own `before` direction has
//! nowhere to live in those. `PaneBufferState::kill_opened_session`
//! (`pane_state.rs`) stays put; pane state is the SSOT for per-(pane, buffer)
//! facts unrelated to session ownership, and this module reaches into it
//! rather than owning it.

use hume_engine::pipeline::{BufferId, EngineView, PaneId};

use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;
use hume_ops::MotionMode;
use hume_ops::edit::{paste_after, paste_before};
use hume_ops::register::{BLACK_HOLE_REGISTER, CLIPBOARD_REGISTER, KILL_RING_REGISTER};

use super::super::{EditorState, Severity, doc_ops, register_ops};
use super::FocusedPane;
use crate::editor::edit_session::{self, EditSessionKind};
use crate::editor::error::CommandError;

/// Which source a bare paste (no `"<reg>` prefix) reads, valid only while
/// [`crate::editor::buffer::store::BufferStore::edit_seq`] is still `seq`. The moment any
/// buffer is edited (or undone/redone), the stamped `seq` falls behind and a
/// bare `smart-paste-*` falls through to the clipboard instead.
///
/// Written by every capture that pushes onto the kill ring (`d`/`c`/`y`, bare
/// or `"k`-prefixed; see `EditorState::capture_to_ring`) and by every
/// completed bare paste (plain or smart) and ring cycle (`[`/`]`), each
/// re-stamping with the *post*-edit `seq` and whatever source it actually
/// used. The re-stamp on completion is load-bearing, not cosmetic: a paste is
/// itself an edit, so without it the stamp a capture wrote would go stale on
/// the very first paste that reads it, and `d p p p` would paste the kill
/// once and the clipboard twice. An explicit register read (`"5p`, `"cp`, …)
/// does not stamp: it is a plain edit as far as this mechanism is concerned.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PasteStamp {
    pub(in crate::editor) seq: u64,
    pub(in crate::editor) source: PasteSource,
}

/// See [`PasteStamp`].
#[derive(Debug, Clone, Copy)]
pub(in crate::editor) enum PasteSource {
    /// Kill-ring slot (`0` = head). Looked up fresh via `KillRing::slot` at
    /// read time rather than snapshotting the text, so a stamp always
    /// reflects the ring's current contents at that slot.
    Ring(usize),
    Clipboard,
}

// ── EditorState helpers ───────────────────────────────────────────────────────

impl EditorState {
    /// Push a bare or `"k`-prefixed capture onto the kill ring and record it
    /// as the freshest capture, for a following bare paste to read. Push and
    /// stamp are one operation: a ring push without the stamp silently
    /// breaks smart-paste routing, so no call site gets to do them
    /// separately. Never used for an explicit named register, which bare
    /// paste never reads. See [`PasteStamp`]'s doc for the full mechanism.
    ///
    /// `pub(in crate::editor)`, not `pub(super)`: `host_impl.rs`'s
    /// `RegisterHost::write_register` also reaches this, so `(write-register!
    /// "k" …)` gets the same stamped ring push as `"ky`.
    pub(in crate::editor) fn capture_to_ring(&mut self, yanked: Vec<String>) {
        self.kill_ring.push(yanked);
        self.paste_stamp = Some(PasteStamp {
            seq: self.buffers.edit_seq(),
            source: PasteSource::Ring(0),
        });
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
    /// regardless. Thin wrapper around [`doc_ops::commit_paste_group`], the
    /// same commit `doc_ops::apply_doc_edit` uses to close a same-pane Paste
    /// session before an unrelated direct edit.
    pub(in crate::editor) fn commit_paste_session(&mut self) {
        doc_ops::commit_paste_group(
            &mut self.buffers,
            &self.panes.state,
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
fn collapse_if_repeat(
    text: &BufferText,
    sels: SelectionSet,
    values: &[String],
    before: bool,
) -> SelectionSet {
    let repeats = values.len() == sels.len()
        && sels
            .iter_sorted()
            .zip(values)
            .all(|(sel, v)| sel.slice(text) == *v);
    if !repeats {
        return sels;
    }
    sels.map(|s| {
        if before {
            Selection::collapsed(s.start())
        } else {
            Selection::collapsed(s.end_inclusive(text))
        }
    })
}

/// A resolved paste, ready for [`do_paste`] to execute.
struct ResolvedPaste {
    values: Vec<String>,
    /// Where the values came from. Drives the two things that depend on it
    /// after the fact: seeding `[`/`]`'s cycle position (`Ring` seeds it) and,
    /// for a bare paste only, stamping [`PasteStamp`] (see `do_paste`).
    /// `None` = an explicit named/digit register, which does neither.
    from: Option<PasteSource>,
    /// No `"<reg>` prefix was given; only a bare paste stamps [`PasteStamp`].
    bare: bool,
}

/// Core paste implementation, shared by the plain and smart variants: applies
/// `resolved` at `sels`, opens the paste/ring-cycle session, and stamps
/// [`PasteStamp`]/seeds the ring cycle for bare pastes. Carries no knowledge
/// of where `resolved` came from or of the repeat-vs-swap rule. Callers
/// resolve the source and (for smart paste) collapse `sels` before calling in.
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
) -> Result<(), CommandError> {
    let ResolvedPaste { values, from, bare } = resolved;

    let pre_sels = sels.clone();
    state.panes.state[focused][buf].set_selections(sels);
    edit_session::open_or_retarget(
        &mut state.active_session,
        focused,
        buf,
        EditSessionKind::Paste { before },
        || state.buffers.get(buf).begin_edit_group(pre_sels),
    )?;
    let paste_fn = if before { paste_before } else { paste_after };
    doc_ops::apply_doc_edit_regrouped(
        &mut state.buffers,
        &state.config.decorations,
        &mut state.panes.state,
        &mut state.panes.jumps,
        &mut state.active_session,
        focused,
        buf,
        |b, s| paste_fn(b, s, &values),
    );

    // An explicit register prefix opts out of the stamp entirely; see
    // PasteStamp's doc for why.
    if bare && let Some(source) = from {
        state.paste_stamp = Some(PasteStamp {
            seq: state.buffers.edit_seq(),
            source,
        });
    }

    // Every completed paste opens a fresh session (any prior one was already
    // committed in BEFORE by `step_paste_commit`), so every one reseeds the
    // cycle.
    let ring_seed = match from {
        Some(PasteSource::Ring(slot)) => Some(slot),
        Some(PasteSource::Clipboard) | None => None,
    };
    state.kill_ring.seed_cycle(ring_seed);
    Ok(())
}

/// Resolve values for a fresh paste against an explicit `"<reg>` prefix.
/// Shared by plain and smart paste: an explicit register bypasses the
/// smart-paste heuristic entirely, so both variants resolve it identically.
/// Returns `None` for a no-op paste: black-hole or an empty register.
fn resolve_explicit_register(state: &mut EditorState, reg: char) -> Option<ResolvedPaste> {
    let (values, from) = match reg {
        KILL_RING_REGISTER => (state.kill_ring.head()?.to_vec(), Some(PasteSource::Ring(0))),
        BLACK_HOLE_REGISTER => return None,
        c => {
            // Digits and clipboard. Digits read in-memory RegisterSet (symmetric
            // with "Ny writes). Clipboard routes through the OS clipboard.
            let (cow, warn) =
                register_ops::read_register_text(&state.registers, &mut state.clipboard, c);
            let values = cow.map(|c| c.to_vec()); // end borrow of state.registers
            if let Some(w) = warn {
                state.report(Severity::Warning, w);
            }
            (values?, None)
        }
    };
    Some(ResolvedPaste {
        values,
        from,
        bare: false,
    })
}

/// Resolve values for a fresh plain (`paste-after`/`-before`) paste. Returns
/// `None` to signal a no-op paste.
fn resolve_plain(state: &mut EditorState) -> Option<ResolvedPaste> {
    match state.take_register_prefix() {
        // Bare source is always the kill-ring head, with no clipboard
        // fallback and no stamp consultation, but `do_paste` still writes
        // the stamp for it, so an immediately following bare *smart* paste
        // (no capture in between) continues from the same ring slot instead
        // of jumping to the clipboard just because a paste is itself an edit.
        None => Some(ResolvedPaste {
            values: state.kill_ring.head()?.to_vec(),
            from: Some(PasteSource::Ring(0)),
            bare: true,
        }),
        Some(reg) => resolve_explicit_register(state, reg),
    }
}

/// Resolve values for a fresh smart (`smart-paste-after`/`-before`) paste.
/// Returns `None` to signal a no-op paste.
fn resolve_smart(state: &mut EditorState) -> Option<ResolvedPaste> {
    match state.take_register_prefix() {
        None => resolve_smart_bare(state),
        Some(reg) => resolve_explicit_register(state, reg),
    }
}

/// Bare `smart-paste-*` resolution: the kill-ring slot the stamp points at
/// while it is still fresh (`PasteStamp::seq == BufferStore::edit_seq()`),
/// the clipboard otherwise. When the clipboard yields nothing, a *fresh*
/// paste falls back to the ring head silently, but a *repeat* (fresh
/// `Clipboard` stamp) refuses to substitute; see the `None` arm below.
fn resolve_smart_bare(state: &mut EditorState) -> Option<ResolvedPaste> {
    let fresh_source = state
        .paste_stamp
        .as_ref()
        .filter(|s| s.seq == state.buffers.edit_seq())
        .map(|s| s.source);
    if let Some(PasteSource::Ring(slot)) = fresh_source {
        let values = state.kill_ring.slot(slot)?.to_vec();
        return Some(ResolvedPaste {
            values,
            from: Some(PasteSource::Ring(slot)),
            bare: true,
        });
    }
    // Stale/no stamp, or a fresh stamp pointing at the clipboard (re-read:
    // the OS clipboard may have changed externally since).
    let (cow, warn) = register_ops::read_register_text(
        &state.registers,
        &mut state.clipboard,
        CLIPBOARD_REGISTER,
    );
    match cow.map(|c| c.to_vec()) {
        Some(values) => {
            if let Some(w) = warn {
                state.report(Severity::Warning, w);
            }
            Some(ResolvedPaste {
                values,
                from: Some(PasteSource::Clipboard),
                bare: true,
            })
        }
        None => {
            // A repeat press (fresh Clipboard stamp) must repeat the
            // clipboard value or do nothing: substituting the ring head
            // would feed `collapse_if_repeat` unrelated text and replace
            // what the previous press just pasted. Warn and no-op.
            if matches!(fresh_source, Some(PasteSource::Clipboard)) {
                if let Some(w) = warn {
                    state.report(Severity::Warning, w);
                }
                return None;
            }
            // A fresh paste with no readable clipboard falls back to the
            // ring head silently. Only emit the warning when the fallback
            // also fails. Otherwise the user sees a warning alongside a
            // successful paste.
            if let Some(head) = state.kill_ring.head() {
                return Some(ResolvedPaste {
                    values: head.to_vec(),
                    from: Some(PasteSource::Ring(0)),
                    bare: true,
                });
            }
            if let Some(w) = warn {
                state.report(Severity::Warning, w);
            }
            None
        }
    }
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
    // Checked before `take_selections` below, not left to `do_paste`'s own
    // `open_or_retarget` call, so a refusal here leaves the live selection
    // untouched instead of stranding it in `SelectionSet::default()`.
    edit_session::check_can_open(&state.active_session, pid, buf)?;
    if super::refuse_if_read_only(state, view, fp.pane()) {
        return Ok(());
    }
    let Some(resolved) = resolve_plain(state) else {
        return Ok(());
    };
    let sels = state.panes.state[pid][buf].take_selections();
    do_paste(state, pid, buf, before, resolved, sels)
}

/// Smart paste: resolve from the stamp-driven source (ring while nothing has
/// been edited since the last capture, clipboard otherwise; see
/// [`PasteStamp`]), apply the repeat-vs-swap collapse rule to the
/// selections (bare paste only; see [`collapse_if_repeat`]), then hand off
/// to [`do_paste`].
fn do_smart_paste(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    before: bool,
) -> Result<(), CommandError> {
    let (pid, buf) = (fp.pid(), fp.bid(view));
    edit_session::check_can_open(&state.active_session, pid, buf)?;
    if super::refuse_if_read_only(state, view, fp.pane()) {
        return Ok(());
    }
    let Some(resolved) = resolve_smart(state) else {
        return Ok(());
    };
    let mut sels = state.panes.state[pid][buf].take_selections();
    if resolved.bare {
        let text = state.buffers.get(buf).text();
        sels = collapse_if_repeat(text, sels, &resolved.values, before);
    }
    do_paste(state, pid, buf, before, resolved, sels)
}

/// Paste after the selection: plain paste, kill-ring head by default.
pub(in crate::editor) fn cmd_paste_after(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_normal_paste(state, view, fp, false)
}

/// Paste before the selection: plain paste, kill-ring head by default.
pub(in crate::editor) fn cmd_paste_before(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_normal_paste(state, view, fp, true)
}

/// Smart-paste after the selection: ring while nothing has been edited since
/// the last capture, clipboard otherwise. See [`PasteStamp`].
pub(in crate::editor) fn cmd_smart_paste_after(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_smart_paste(state, view, fp, false)
}

/// Smart-paste before the selection: ring while nothing has been edited since
/// the last capture, clipboard otherwise. See [`PasteStamp`].
pub(in crate::editor) fn cmd_smart_paste_before(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_smart_paste(state, view, fp, true)
}

/// Shared implementation for `[` and `]`: advance/retreat the kill-ring cycle
/// cursor and re-paste from the session snapshot.
///
/// Noop when no paste session is open or when the cycle is already at a boundary.
fn do_paste_cycle(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    older: bool,
) -> Result<(), CommandError> {
    let (focused, buf) = (fp.pid(), fp.bid(view));
    let Some(EditSessionKind::Paste { before }) = state
        .active_session
        .as_ref()
        .filter(|s| s.owned_by(focused, buf))
        .map(|s| s.kind())
    else {
        return Ok(());
    };
    // Eagerly convert to owned Vec so the borrow of state.kill_ring ends before
    // state.buffers and state.panes.state are borrowed mutably below.
    let values = if older {
        state.kill_ring.cycle_older()
    } else {
        state.kill_ring.cycle_newer()
    }
    .map(|v| v.to_vec());
    if let Some(values) = values {
        let paste_fn = if before { paste_before } else { paste_after };
        doc_ops::apply_doc_edit_regrouped(
            &mut state.buffers,
            &state.config.decorations,
            &mut state.panes.state,
            &mut state.panes.jumps,
            &mut state.active_session,
            focused,
            buf,
            |b, s| paste_fn(b, s, &values),
        );
        // Cycling always lands on a ring slot, so reflect it in the stamp so a
        // following bare paste (of either variant) continues from here.
        if let Some(slot) = state.kill_ring.cycle_position() {
            state.paste_stamp = Some(PasteStamp {
                seq: state.buffers.edit_seq(),
                source: PasteSource::Ring(slot),
            });
        }
    }
    Ok(())
}

/// Cycle the kill ring one step older and re-paste from the session snapshot.
pub(in crate::editor) fn cmd_paste_ring_older(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste_cycle(state, view, fp, true)
}

/// Cycle the kill ring one step newer and re-paste from the session snapshot.
pub(in crate::editor) fn cmd_paste_ring_newer(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste_cycle(state, view, fp, false)
}
