//! Register/kill-ring paste: `p`/`P`, smart paste, and `[`/`]` ring cycling.
//!
//! Distinct from terminal bracketed-paste (`Event::Paste`,
//! `mappings/bracketed_paste.rs`) — an unrelated feature that happens to
//! share the word "paste".
//!
//! `PaneBufferState`'s `paste_group`/`paste_before`/`kill_opened_session`
//! fields (`pane_state.rs`) deliberately stay put rather than moving here:
//! pane state is the SSOT for per-(pane, buffer) facts, and this module
//! reaches into it rather than owning it.

use hume_engine::pipeline::{BufferId, EngineView, PaneId};

use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::Text;
use hume_ops::MotionMode;
use hume_ops::edit::{paste_after, paste_before};
use hume_ops::register::{BLACK_HOLE_REGISTER, CLIPBOARD_REGISTER, KILL_RING_REGISTER};

use super::super::{EditorState, Severity, doc_ops, register_ops};
use super::focused_buffer_id;
use crate::editor::error::CommandError;

// ── PasteStamp ──────────────────────────────────────────────────────────────

/// Which source a bare paste (no `"<reg>` prefix) reads, valid only while
/// [`crate::editor::buffer::store::BufferStore::edit_seq`] is still `seq` — the moment any
/// buffer is edited (or undone/redone), the stamped `seq` falls behind and a
/// bare `smart-paste-*` falls through to the clipboard instead.
///
/// Written by every capture that pushes onto the kill ring (`d`/`c`/`y`, bare
/// or `"k`-prefixed — see `EditorState::capture_to_ring`) and by every
/// completed bare paste (plain or smart) and ring cycle (`[`/`]`), each
/// re-stamping with the *post*-edit `seq` and whatever source it actually
/// used. The re-stamp on completion is load-bearing, not cosmetic: a paste is
/// itself an edit, so without it the stamp a capture wrote would go stale on
/// the very first paste that reads it, and `d p p p` would paste the kill
/// once and the clipboard twice. An explicit register read (`"5p`, `"cp`, …)
/// does not stamp — it is a plain edit as far as this mechanism is concerned.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PasteStamp {
    pub(crate) seq: u64,
    pub(crate) source: PasteSource,
}

/// See [`PasteStamp`].
#[derive(Debug, Clone, Copy)]
pub(crate) enum PasteSource {
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
    /// stamp are one operation — a ring push without the stamp silently
    /// breaks smart-paste routing, so no call site gets to do them
    /// separately. Never used for an explicit named register, which bare
    /// paste never reads. See [`PasteStamp`]'s doc for the full mechanism.
    pub(super) fn capture_to_ring(&mut self, yanked: Vec<String>) {
        self.kill_ring.push(yanked);
        self.paste_stamp = Some(PasteStamp {
            seq: self.buffers.edit_seq(),
            source: PasteSource::Ring(0),
        });
    }

    /// Commit the open paste session on the focused (pane, buffer) pair, if any.
    ///
    /// Records exactly one history revision for the entire paste + all cycles.
    /// Called before any non-`[`/`]` dispatch so the session is committed
    /// before undo, motions, or the next `p`/`P`.
    ///
    /// Invariant: an open paste session can only exist on the focused (pane,
    /// buffer) pair — sessions are opened only there (`do_paste`),
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

// ── Paste ────────────────────────────────────────────────────────────────────

/// Bare smart-paste only — only `do_smart_paste` calls this, and only for a
/// bare paste: if every selection's current text matches `values` one-to-one,
/// collapse the selections so the paste below lands next to the existing text
/// instead of replacing it — repeat-paste ("stack another copy") and
/// swap-paste ("replace what's different") are the same rule, decided by
/// content, not by what command ran last.
///
/// Plain paste never calls this — it always replaces a non-collapsed
/// selection, unconditionally, so a script driving it never has to check
/// what's already selected before pasting; to append, the script collapses
/// the selection itself first. An explicit-register smart paste (`"Xp`)
/// skips it too: that's a direct "paste register X here" order, sharing the
/// plain-paste replace contract. Bare smart-paste's own goal is different:
/// staying predictable to a human who just wants "paste, however that's
/// smart to do" means noticing when a repeat would clobber what it just
/// pasted.
///
/// All-or-nothing across the whole set: a length mismatch, or any single
/// selection whose text differs from its value, means replace. The
/// underlying op (`paste_after`/`paste_before`) applies one uniform
/// charwise/linewise mode to every selection, so there is no way to replace
/// selection 1 while appending next to selection 2 — partial agreement can't
/// be represented, so it falls back to the always-safe replace. A selection
/// that is already collapsed (a bare cursor) is untouched either way:
/// `paste_after`/`paste_before` already insert next to a collapsed cursor
/// rather than replacing, so this rule only has visible effect on a real
/// (non-collapsed) selection.
fn collapse_if_repeat(
    buf: &Text,
    sels: SelectionSet,
    values: &[String],
    before: bool,
) -> SelectionSet {
    let repeats = values.len() == sels.len()
        && sels
            .iter_sorted()
            .zip(values)
            .all(|(sel, v)| buf.slice(sel.start()..sel.end_inclusive(buf) + 1) == *v);
    if !repeats {
        return sels;
    }
    sels.map(|s| {
        if before {
            Selection::collapsed(s.start())
        } else {
            Selection::collapsed(s.end_inclusive(buf))
        }
    })
}

/// A resolved paste, ready for [`do_paste`] to execute.
struct ResolvedPaste {
    values: Vec<String>,
    /// Where the values came from — drives the two things that depend on it
    /// after the fact: seeding `[`/`]`'s cycle position (`Ring` seeds it) and,
    /// for a bare paste only, stamping [`PasteStamp`] (see `do_paste`).
    /// `None` = an explicit named/digit register, which does neither.
    from: Option<PasteSource>,
    /// No `"<reg>` prefix was given — only a bare paste stamps [`PasteStamp`].
    bare: bool,
}

/// Core paste implementation, shared by the plain and smart variants: applies
/// `resolved` at `sels`, opens the paste/ring-cycle session, and stamps
/// [`PasteStamp`]/seeds the ring cycle for bare pastes. Carries no knowledge
/// of where `resolved` came from or of the repeat-vs-swap rule — callers
/// resolve the source and (for smart paste) collapse `sels` before calling in.
///
/// `before`: true for `P` (paste before), false for `p` (paste after).
fn do_paste(
    state: &mut EditorState,
    focused: PaneId,
    buf: BufferId,
    before: bool,
    resolved: ResolvedPaste,
    sels: SelectionSet,
) {
    let ResolvedPaste { values, from, bare } = resolved;

    let pre_sels = sels.clone();
    state.panes.state[focused][buf].selections = sels;
    state.panes.state[focused][buf].paste_before = before;
    state
        .buffers
        .get(buf)
        .begin_edit_group(&mut state.panes.state[focused][buf].paste_group, pre_sels);
    let paste_fn = if before { paste_before } else { paste_after };
    doc_ops::apply_doc_edit_regrouped(
        &mut state.buffers,
        &state.config.decorations,
        &mut state.panes.state,
        focused,
        buf,
        |b, s| paste_fn(b, s, &values),
    );

    // An explicit register prefix opts out of the stamp entirely — see
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
}

/// Resolve values for a fresh paste against an explicit `"<reg>` prefix.
/// Shared by plain and smart paste — an explicit register bypasses the
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
        // fallback and no stamp consultation — but `do_paste` still writes
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
/// `Clipboard` stamp) refuses to substitute — see the `None` arm below.
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
    // Stale/no stamp, or a fresh stamp pointing at the clipboard (re-read —
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
            // also fails — otherwise the user sees a warning alongside a
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
/// `"<reg>` otherwise), then hand off to [`do_paste`] unconditionally —
/// always replaces a non-collapsed selection. See [`collapse_if_repeat`]'s
/// doc for why smart paste alone needs the extra step.
fn do_normal_paste(state: &mut EditorState, view: &mut EngineView, before: bool) {
    if super::focused_buffer_read_only(state, view) {
        state.report(Severity::Info, "Buffer is read-only".to_string());
        return;
    }
    let Some(resolved) = resolve_plain(state) else {
        return;
    };
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    let sels = std::mem::take(&mut state.panes.state[focused][buf].selections);
    do_paste(state, focused, buf, before, resolved, sels);
}

/// Smart paste: resolve from the stamp-driven source (ring while nothing has
/// been edited since the last capture, clipboard otherwise — see
/// [`PasteStamp`]), apply the repeat-vs-swap collapse rule to the
/// selections (bare paste only — see [`collapse_if_repeat`]), then hand off
/// to [`do_paste`].
fn do_smart_paste(state: &mut EditorState, view: &mut EngineView, before: bool) {
    if super::focused_buffer_read_only(state, view) {
        state.report(Severity::Info, "Buffer is read-only".to_string());
        return;
    }
    let Some(resolved) = resolve_smart(state) else {
        return;
    };
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    let mut sels = std::mem::take(&mut state.panes.state[focused][buf].selections);
    if resolved.bare {
        let text = state.buffers.get(buf).text();
        sels = collapse_if_repeat(text, sels, &resolved.values, before);
    }
    do_paste(state, focused, buf, before, resolved, sels);
}

/// Paste after the selection: plain paste, kill-ring head by default.
pub(crate) fn cmd_paste_after(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_normal_paste(state, view, false);
    Ok(())
}

/// Paste before the selection: plain paste, kill-ring head by default.
pub(crate) fn cmd_paste_before(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_normal_paste(state, view, true);
    Ok(())
}

/// Smart-paste after the selection: ring while nothing has been edited since
/// the last capture, clipboard otherwise. See [`PasteStamp`].
pub(crate) fn cmd_smart_paste_after(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_smart_paste(state, view, false);
    Ok(())
}

/// Smart-paste before the selection: ring while nothing has been edited since
/// the last capture, clipboard otherwise. See [`PasteStamp`].
pub(crate) fn cmd_smart_paste_before(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_smart_paste(state, view, true);
    Ok(())
}

/// Shared implementation for `[` and `]`: advance/retreat the kill-ring cycle
/// cursor and re-paste from the session snapshot.
///
/// Noop when no paste session is open or when the cycle is already at a boundary.
fn do_paste_cycle(
    state: &mut EditorState,
    view: &mut EngineView,
    older: bool,
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    if state.panes.state[focused][buf].paste_group.is_none() {
        return Ok(());
    }
    // Eagerly convert to owned Vec so the borrow of state.kill_ring ends before
    // state.buffers and state.panes.state are borrowed mutably below.
    let values = if older {
        state.kill_ring.cycle_older()
    } else {
        state.kill_ring.cycle_newer()
    }
    .map(|v| v.to_vec());
    if let Some(values) = values {
        let before = state.panes.state[focused][buf].paste_before;
        let paste_fn = if before { paste_before } else { paste_after };
        doc_ops::apply_doc_edit_regrouped(
            &mut state.buffers,
            &state.config.decorations,
            &mut state.panes.state,
            focused,
            buf,
            |b, s| paste_fn(b, s, &values),
        );
        // Cycling always lands on a ring slot — reflect it in the stamp so a
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
pub(crate) fn cmd_paste_ring_older(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste_cycle(state, view, true)
}

/// Cycle the kill ring one step newer and re-paste from the session snapshot.
pub(crate) fn cmd_paste_ring_newer(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste_cycle(state, view, false)
}
