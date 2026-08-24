//! `align-selections` — align each selection's anchor to the primary
//! selection's anchor display column.

use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::grapheme::display_col_in_line;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;

use super::apply_edit;

/// Align selections into slots, using the primary's row as a baseline.
///
/// **Slot model** — the primary's line determines the slot count `N`: one
/// slot per single-line selection on that line (in left-to-right order). Every
/// other line participates slot-by-slot: its k-th single-line selection aligns to
/// slot `k`. Selections in slots ≥ N ("extras") and multiline selections pass
/// through unchanged (shifted by the accumulated edit delta so they don't drift).
///
/// **Target per slot** — `target[k] = max(baseline[k], fit_need[k])`:
/// - `baseline[k]` = anchor display column (`tab_width`-expanded, wide
///   graphemes counted at their true screen width) of the primary line's
///   k-th selection (the primary row's positions are a floor).
/// - `fit_need[k]` = the minimum anchor display column such that every
///   line's slot-`k` selection can reach it. A selection can only compress
///   the contiguous space/tab run immediately before its left edge (down to
///   1 display column); all other text on the line is fixed-width and sets
///   a hard floor.
/// - Slots are computed left-to-right: `fit_need[k]` depends on `target[k-1]`.
///
/// **Direction** — the anchor is direction-aware: forward → anchor is the left
/// edge (left-align); backward → anchor is the right edge (right-align). The
/// uniform anchor + removable-whitespace model works for both without
/// special-casing.
///
/// **Primary may move** — when another line forces a slot to widen past the
/// baseline, spaces are inserted before the primary line's selections too.
///
/// **Compression is char-counted, not display-counted** — the removable
/// whitespace run before a selection (`rem`) still measures in characters:
/// each space or tab in the run is one removable unit, regardless of the
/// tab's actual display width at its position. Deleting `rem` characters
/// frees at least `rem` display columns (a tab is never narrower than a
/// space), so compression never removes too little — but it can occasionally
/// remove one column *more* than strictly needed when the freed run contains
/// a tab. Insertion has no such gap: an inserted run is always spaces, each
/// exactly one display column, so `amount > 0` lands exactly on `target`.
pub fn align_selections(
    text: BufferText,
    sels: SelectionSet,
    tab_width: u8,
) -> (BufferText, SelectionSet, ChangeSet) {
    // ── Pass 1: measure ────────────────────────────────────────────────────────

    // Geometry for each selection in sorted order (matches apply_edit iteration).
    struct SelMeta {
        start_line: usize,
        is_multiline: bool,
        anchor_display_col: usize, // display col of sel.anchor() (left for forward, right for backward)
        rem: usize,                // chars removable before sel.start() while keeping ≥1 space
        slot: Option<usize>,       // None = multiline or extra (slot >= N)
    }

    let primary_line = text.char_to_line(sels.primary().anchor());
    let mut slots_on_line = rustc_hash::FxHashMap::<usize, usize>::default();

    let mut meta: Vec<SelMeta> = sels
        .iter_sorted()
        .map(|sel| {
            let start_line = text.char_to_line(sel.start());
            let is_multiline = start_line != text.char_to_line(sel.end_inclusive(&text));
            if is_multiline {
                return SelMeta {
                    start_line,
                    is_multiline: true,
                    anchor_display_col: 0,
                    rem: 0,
                    slot: None,
                };
            }
            let anchor_display_col = display_col_in_line(&text, start_line, sel.anchor(), tab_width);
            let line_start = text.line_to_char(start_line);
            let sel_start = sel.start();
            let rem = (line_start..sel_start)
                .rev()
                .take_while(|&p| matches!(text.char_at(p), Some(' ') | Some('\t')))
                .count()
                .saturating_sub(1);
            let counter = slots_on_line.entry(start_line).or_insert(0);
            let slot = *counter;
            *counter += 1;
            SelMeta {
                start_line,
                is_multiline: false,
                anchor_display_col,
                rem,
                slot: Some(slot),
            }
        })
        .collect();

    // N = number of single-line selections on the primary line.
    let n_slots = slots_on_line.get(&primary_line).copied().unwrap_or(0);

    if n_slots == 0 {
        // Primary is multiline — no slot structure, everything passes through.
        let mut b = ChangeSetBuilder::new(text.len_chars());
        b.retain_rest();
        let cs = b.finish();
        return (text, sels, cs);
    }

    // Mark slots >= n_slots as extras → pass through.
    for m in &mut meta {
        if m.slot.is_some_and(|s| s >= n_slots) {
            m.slot = None;
        }
    }

    // ── Pass 2: targets ────────────────────────────────────────────────────────

    // baseline[k] = original anchor display column of the primary line's k-th slot.
    let mut baseline = vec![0usize; n_slots];
    for m in &meta {
        if m.start_line == primary_line
            && let Some(slot) = m.slot
        {
            baseline[slot] = m.anchor_display_col;
        }
    }

    // Group participating metas by line for pair-wise constraint computation.
    // Values are in slot order (sels.iter_sorted() is ascending by start).
    let mut by_line: rustc_hash::FxHashMap<usize, Vec<&SelMeta>> = rustc_hash::FxHashMap::default();
    for m in &meta {
        if !m.is_multiline {
            by_line.entry(m.start_line).or_default().push(m);
        }
    }

    let mut targets = vec![0usize; n_slots];

    // k == 0: the only thing slot-0 can compress is its own preceding whitespace
    // (down to 1 display column). So the minimum reachable anchor is
    // anchor_display_col₀ − rem₀. Compute in isize to handle the (unlikely)
    // backward-selection case where anchor_display_col < rem; clamp to 0.
    let fit_0 = by_line
        .values()
        .filter_map(|ms| ms.iter().find(|m| m.slot == Some(0)))
        .map(|m| m.anchor_display_col as isize - m.rem as isize)
        .max()
        .unwrap_or(0)
        .max(0) as usize;
    targets[0] = baseline[0].max(fit_0);

    // k >= 1: placing target[k-1] shifts every anchor on that line by
    // (target[k-1] − anchor_display_col_{k-1}). Slot k then shifts by the
    // same amount, so its new anchor is
    // anchor_display_col_k + (target[k-1] − anchor_display_col_{k-1}). The
    // minimum feasible target[k] (leaving at least 1 space before slot k) is:
    //   target[k-1] + (anchor_display_col_k − anchor_display_col_{k-1}) − rem_k
    // where rem_k is the whitespace slot k may compress (avail − 1).
    for k in 1..n_slots {
        let fit_k = by_line
            .values()
            .filter_map(|ms| {
                let prev = ms.iter().find(|m| m.slot == Some(k - 1))?;
                let cur = ms.iter().find(|m| m.slot == Some(k))?;
                Some(
                    targets[k - 1] as isize
                        + (cur.anchor_display_col as isize - prev.anchor_display_col as isize)
                        - cur.rem as isize,
                )
            })
            .max()
            .unwrap_or(0)
            .max(0) as usize;
        targets[k] = baseline[k].max(fit_k);
    }

    // ── Pass 3: apply ──────────────────────────────────────────────────────────

    // `line_shift` tracks the net chars inserted/removed on the current line so
    // far, approximating the shift from original-buffer anchor display
    // columns to post-edit ones. An inserted run is always spaces, so an
    // insertion's char delta equals its display-column delta exactly; a
    // deletion's char delta is only a lower bound on its display-column
    // delta when the removed run contains a tab (see `align_selections`'s
    // own doc). `amount`'s sign selects which case applies on this line.
    let mut current_line = usize::MAX;
    let mut line_shift = 0isize;

    apply_edit(text, sels, |b, text, i, sel, new_sels| {
        let sel_start = sel.start();
        let sel_end = sel.end_inclusive(text);
        let content_len = sel_end + 1 - sel_start;
        let forward = sel.anchor() <= sel.head();
        let start_line = text.char_to_line(sel_start);

        if start_line != current_line {
            current_line = start_line;
            line_shift = 0;
        }

        match meta[i].slot {
            None => {
                // Extras + multiline: retain up to sel_start, capture the global
                // delta (from all edits before this position), retain the
                // content, push shifted selection.
                b.retain(sel_start - b.old_pos());
                let delta = b.new_pos() as isize - b.old_pos() as isize;
                b.retain(content_len);
                let new_anchor = (sel.anchor() as isize + delta) as usize;
                let new_head = (sel.head() as isize + delta) as usize;
                new_sels.push(Selection::new(new_anchor, new_head));
            }
            Some(slot) => {
                let target = targets[slot];
                // Adjust the original anchor display column by the net shift
                // from earlier edits on this line to get the current anchor
                // display column.
                let anchor_display_col_orig =
                    display_col_in_line(text, start_line, sel.anchor(), tab_width);
                let anchor_display_col_now =
                    (anchor_display_col_orig as isize + line_shift).max(0) as usize;
                let amount = target as isize - anchor_display_col_now as isize;

                if amount > 0 {
                    b.retain(sel_start - b.old_pos());
                    b.insert(&" ".repeat(amount as usize));
                    line_shift += amount;
                } else if amount < 0 {
                    // Remove whitespace immediately before sel_start. `rem` (= avail−1)
                    // was computed in Pass 1, so we reuse it here. Also never step past
                    // b.old_pos() (the already-consumed boundary on this line).
                    let remove = ((-amount) as usize)
                        .min(meta[i].rem)
                        .min(sel_start.saturating_sub(b.old_pos()));
                    b.retain((sel_start - remove) - b.old_pos());
                    if remove > 0 {
                        b.delete(remove);
                        line_shift -= remove as isize;
                    }
                } else {
                    b.retain(sel_start - b.old_pos());
                }

                // b.old_pos() is now at sel_start. Record the mapped start, retain
                // content, then push the new selection preserving direction.
                let new_start = b.new_pos();
                b.retain(content_len);
                // Use sel.end() (not end_inclusive) so anchor/head land on the
                // grapheme boundary rather than on a trailing combining codepoint.
                let new_end = new_start + (sel.end() - sel_start);
                new_sels.push(Selection::directed(new_start, new_end, forward));
            }
        }
    })
}
