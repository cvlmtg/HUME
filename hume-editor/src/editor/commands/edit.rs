use hume_engine::pipeline::{BufferId, EngineView, PaneId};

use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::Text;
use hume_ops::MotionMode;
use hume_ops::edit::{
    align_selections, change_span, delete_selection, delete_selection_content,
    join_lines_select_spaces, paste_after, paste_before, replace_selections,
};
use hume_ops::register::{
    BLACK_HOLE_REGISTER, CLIPBOARD_REGISTER, KILL_RING_REGISTER, yank_selections,
};
use hume_ops::surround::wrap_each_selection;

use super::super::{EditorState, PasteSource, PasteStamp, Severity, doc_ops, register_ops};
use super::{
    apply_focused_edit, apply_focused_edit_grouped, apply_focused_motion, begin_insert_session,
    doc, focused_buffer_id, pin_insert_anchors,
};
use crate::editor::error::CommandError;

// ── Edit composites ───────────────────────────────────────────────────────────

/// Yank selections into the active register, then delete them.
///
/// **Bare default** (no `"<reg>` prefix): pushes to the kill ring only.
/// **Explicit register**: routes through `write_register`.
pub fn cmd_delete(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let yanked = yank_selections(
        super::doc(state, view).text(),
        super::current_selections(state, view),
    );
    apply_focused_edit(state, view, delete_selection);
    state.route_kill(yanked);
    Ok(())
}

/// Yank, delete, then enter insert mode — all in one undo group.
///
/// **Bare default**: pushes to kill ring only. **Explicit register**: routes through
/// `write_register` — same as `cmd_delete`.
///
/// Unlike `d`, a trailing `\n` at the end of a selection is not deleted — `c`
/// clears line content but keeps the line. The yank is trimmed accordingly so
/// the kill-ring entry matches what was removed (no trailing `\n`).
pub fn cmd_change(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let yanked = {
        let doc = super::doc(state, view);
        let sels = super::current_selections(state, view);
        sels.iter_sorted()
            .map(|sel| {
                let (start, stop) = change_span(doc.text(), sel);
                doc.text().slice(start..stop).to_string()
            })
            .collect::<Vec<_>>()
    };
    begin_insert_session(state, view);
    apply_focused_edit_grouped(state, view, delete_selection_content);
    pin_insert_anchors(state, view);
    // Auto-select the typed replacement on exit only when the setting is on
    // (`mii` can still recover it later regardless — see `pin_insert_anchors`).
    // Gated on the group actually being open (skips read-only buffers, and
    // re-captures correctly on dot-repeat replay, which pre-opens the group).
    if doc(state, view)
        .overrides
        .select_changed_text(&state.settings)
        && super::is_group_open_current(state, view)
    {
        let pid = state.focused_pane_id;
        let bid = focused_buffer_id(state, view);
        state.panes.state[pid][bid].select_on_exit = true;
    }
    // Kill-opened only when the yank actually captured to the ring: the
    // capture stamped `PasteStamp`, but every keystroke about to be typed in
    // the session bumps `edit_seq` and would strand it — the flag makes
    // `end_insert_session` refresh the stamp's `seq` once typing stops. An
    // explicit-register change (`"5c`) writes no stamp, and refreshing
    // whatever stale stamp might pre-exist would wrongly resurrect it. Lives
    // on `PaneBufferState`, not `InsertSession`, for the same reason
    // `select_on_exit` does (see its doc): a no-op on a read-only buffer,
    // where `begin_insert_session` refused and no group opened, since
    // `end_insert_session` never runs to read it back.
    if state.route_kill(yanked) {
        let pid = state.focused_pane_id;
        let bid = focused_buffer_id(state, view);
        state.panes.state[pid][bid].kill_opened_session = true;
    }
    Ok(())
}

/// Select the span(s) typed during the most recently completed insert
/// session (`i`/`a`/`o`/`O`/`A`/`I`/`c`/…), bound at `mii`.
///
/// Like every other object in the `mi`/`ma` trie, honors [`MotionMode`]:
/// `Move` replaces the current selection with just the insertion spans;
/// `Extend` unions them into the current selection set instead of
/// discarding it, matching the `.extendable()` contract.
///
/// Reports [`Severity::Info`] and leaves selections untouched if there is no
/// stashed insertion, or if a later mutation (any edit, undo, or redo) has
/// moved the buffer's `text_gen` past the stamp — see
/// [`crate::editor::buffer::LastInsert`].
pub fn cmd_select_last_insertion(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let buf = doc(state, view);
    let fresh = buf
        .last_insert
        .as_ref()
        .filter(|last| last.text_gen == buf.text_gen)
        .map(|last| last.spans.clone());
    let Some(spans) = fresh else {
        state.report(Severity::Info, "no last insertion".to_string());
        return Ok(());
    };
    // Non-empty by construction: `end_insert_session` only ever stashes a
    // non-empty `spans` vec (see `pin_insert_anchors`'s caller). The last
    // span is spatially last (stashed in ascending-start order) — primary
    // there, matching the entry command's own cursor placement.
    let insertion_primary = spans.len() - 1;
    let insertion_sels: Vec<Selection> = spans
        .into_iter()
        .map(|(anchor, head)| Selection::new(anchor, head))
        .collect();
    apply_focused_motion(state, view, move |_b, sels| match mode {
        MotionMode::Move => SelectionSet::from_vec(insertion_sels, insertion_primary),
        MotionMode::Extend => {
            // `from_vec` sorts and merges genuinely overlapping selections,
            // so this is a plain union — no need to zip against current
            // selections one-to-one (their counts can differ freely, e.g.
            // `mii` invoked after the selection count changed since the
            // insert). Merely-adjacent (touching, non-overlapping) spans
            // stay separate selections, same as everywhere else in the
            // codebase. The pre-existing primary stays primary, consistent
            // with how every other `mi*` object behaves in Extend mode.
            let primary = sels.primary_index();
            let mut combined: Vec<Selection> = sels.iter_sorted().copied().collect();
            combined.extend(insertion_sels);
            SelectionSet::from_vec(combined, primary)
        }
    });
    Ok(())
}

/// Yank selections without deleting.
///
/// **Bare default**: writes to the system clipboard AND pushes to the kill ring.
/// **Explicit register**: routes through `write_register`.
pub fn cmd_yank(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let yanked = yank_selections(
        super::doc(state, view).text(),
        super::current_selections(state, view),
    );
    match state.take_register_prefix() {
        None => {
            state.write_register(CLIPBOARD_REGISTER, yanked.clone());
            state.capture_to_ring(yanked);
        }
        // "ky: push to ring only (no clipboard).
        Some(KILL_RING_REGISTER) => state.capture_to_ring(yanked),
        Some(reg) => state.write_register(reg, yanked),
    }
    Ok(())
}

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

    state.panes.state[focused][buf].selections = sels;
    state.panes.state[focused][buf].paste_before = before;

    let pre_sels = state.panes.state[focused][buf].selections.clone();
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
pub fn cmd_paste_after(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_normal_paste(state, view, false);
    Ok(())
}

/// Paste before the selection: plain paste, kill-ring head by default.
pub fn cmd_paste_before(
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
pub fn cmd_smart_paste_after(
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
pub fn cmd_smart_paste_before(
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
pub fn cmd_paste_ring_older(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste_cycle(state, view, true)
}

/// Cycle the kill ring one step newer and re-paste from the session snapshot.
pub fn cmd_paste_ring_newer(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste_cycle(state, view, false)
}

pub fn cmd_undo(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    for _ in 0..count {
        if !state.buffers.get(buf).can_undo() {
            state.report(Severity::Info, "Already at oldest change".to_string());
            break;
        }
        doc_ops::apply_doc_undo(
            &mut state.buffers,
            &state.config.decorations,
            &mut state.panes.state,
            focused,
            buf,
        );
    }
    Ok(())
}

pub fn cmd_redo(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    for _ in 0..count {
        if !state.buffers.get(buf).can_redo() {
            state.report(Severity::Info, "Already at newest change".to_string());
            break;
        }
        doc_ops::apply_doc_redo(
            &mut state.buffers,
            &state.config.decorations,
            &mut state.panes.state,
            focused,
            buf,
        );
    }
    Ok(())
}

// ── Replace / surround ────────────────────────────────────────────────────────

/// Replace every character in each selection with the next typed character.
pub fn cmd_replace(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if let Some(ch) = state.pending_char.take() {
        apply_focused_edit(state, view, |b, s| replace_selections(b, s, ch));
    }
    Ok(())
}

/// Join lines inside each selection and select the inserted spaces.
pub fn cmd_join_lines_select_spaces(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    apply_focused_edit(state, view, join_lines_select_spaces);
    Ok(())
}

/// Align each selection's anchor to the primary selection's anchor column.
pub fn cmd_align_selections(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    apply_focused_edit(state, view, align_selections);
    Ok(())
}

/// Wrap every selection with a pair determined by the next typed character.
pub fn cmd_surround_add(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let Some(ch) = state.pending_char.take() else {
        return Ok(());
    };
    let (_ap_enabled, ap_pairs) = super::doc(state, view)
        .overrides
        .auto_pairs_ref(&state.settings);
    let (open, close) = ap_pairs
        .iter()
        .find(|p| p.open == ch || p.close == ch)
        .map(|p| (p.open, p.close))
        .unwrap_or((ch, ch));
    apply_focused_edit(state, view, |b, s| wrap_each_selection(b, s, open, close));
    Ok(())
}
