//! `align-selections` — align each selection's anchor to the primary
//! selection's anchor display column.

use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::grapheme::{char_pos_at_display_col, display_col_in_line};
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;
use hume_rope::column::BufferLineCol;
use hume_rope::line::ContentLine;
use hume_rope::offset::CharOffset;

use super::apply_edit;

/// Align selections into slots, using the primary's line as a baseline.
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
///   k-th selection (the primary line's positions are a floor).
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
/// **Compression is measured in display cells** — the removable run before a
/// selection is tracked as two numbers: `rem` (chars — how many the run has
/// to spare, keeping ≥1) caps how much can be *deleted*, and `rem_cells`
/// (that run's tab-aware display width) is what every target/fit computation
/// actually operates on. Removing the whole `rem`-char run frees exactly
/// `rem_cells` display columns, so `fit_need` is exact, not a lower bound. A
/// tab's whole-unit granularity can still force removing more than the exact
/// cell need in one step (deleting a tab that straddles the target frees more
/// than requested) — the surplus is padded back with spaces so every
/// selection still lands precisely on `target`. Insertion has no granularity
/// gap of its own: an inserted run is always spaces, each exactly one display
/// column, so `amount > 0` lands exactly on `target` without padding.
pub fn align_selections(
    text: BufferText,
    sels: SelectionSet,
    tab_width: u8,
) -> (BufferText, SelectionSet, ChangeSet) {
    // ── Pass 1: measure ────────────────────────────────────────────────────────

    // Geometry for each selection in sorted order (matches apply_edit iteration).
    struct SelMeta {
        start_line: ContentLine,
        is_multiline: bool,
        anchor_display_col: BufferLineCol, // display col of sel.anchor() (left for forward, right for backward)
        start_display_col: BufferLineCol, // display col of sel.start() — same value pass 3 re-derives from the same unedited text, cached here to avoid the second walk
        rem: usize,          // chars removable before sel.start() while keeping ≥1 space
        rem_cells: u32,      // display-cell width of the `rem`-char run (tab-aware)
        slot: Option<usize>, // None = multiline or extra (slot >= N)
    }

    let primary_line = text.char_to_line(sels.primary().anchor());
    let mut slots_on_line = rustc_hash::FxHashMap::<ContentLine, usize>::default();

    let mut meta: Vec<SelMeta> = sels
        .iter_sorted()
        .map(|sel| {
            let start_line = text.char_to_line(sel.start());
            let is_multiline = start_line != text.char_to_line(sel.end_inclusive(&text));
            if is_multiline {
                return SelMeta {
                    start_line,
                    is_multiline: true,
                    anchor_display_col: BufferLineCol::new(0),
                    start_display_col: BufferLineCol::new(0),
                    rem: 0,
                    rem_cells: 0,
                    slot: None,
                };
            }
            let anchor_display_col =
                display_col_in_line(&text, start_line, sel.anchor(), tab_width);
            let line_start = text.line_to_char(start_line.into());
            let sel_start = sel.start();
            // `sel.start()` is `anchor.min(head)`, so for a forward selection
            // (anchor <= head) it's the anchor itself — reuse the column just
            // walked above rather than walking the same prefix again. Only a
            // backward selection (head == start, anchor == end) needs its own walk.
            let start_display_col = if sel.anchor() <= sel.head() {
                anchor_display_col
            } else {
                display_col_in_line(&text, start_line, sel_start, tab_width)
            };
            let rem = (line_start.index()..sel_start.index())
                .rev()
                .take_while(|&p| matches!(text.char_at(CharOffset::new(p)), Some(' ') | Some('\t')))
                .count()
                .saturating_sub(1);
            // The `rem`-char run's display width, tab-aware. Measured from
            // `sel_start`, not `anchor_display_col`: the run always ends at
            // the selection's left edge, which for a backward multi-char
            // selection is the head, not the (right-edge) anchor — using
            // the anchor's column here would fold the selection's own
            // content width into the run's width.
            let run_start = sel_start.retreat(rem);
            let rem_cells = start_display_col
                .cells_since(display_col_in_line(&text, start_line, run_start, tab_width));
            let counter = slots_on_line.entry(start_line).or_insert(0);
            let slot = *counter;
            *counter += 1;
            SelMeta {
                start_line,
                is_multiline: false,
                anchor_display_col,
                start_display_col,
                rem,
                rem_cells,
                slot: Some(slot),
            }
        })
        .collect();

    // N = number of single-line selections on the primary line.
    let n_slots = slots_on_line.get(&primary_line).copied().unwrap_or(0);

    if n_slots == 0 {
        // Primary is multiline — no slot structure, everything passes through.
        let mut b = ChangeSetBuilder::new(text.end());
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
    let mut baseline = vec![BufferLineCol::new(0); n_slots];
    for m in &meta {
        if m.start_line == primary_line
            && let Some(slot) = m.slot
        {
            baseline[slot] = m.anchor_display_col;
        }
    }

    // Group participating metas by line for pair-wise constraint computation.
    // Values are in slot order (sels.iter_sorted() is ascending by start).
    let mut by_line: rustc_hash::FxHashMap<ContentLine, Vec<&SelMeta>> =
        rustc_hash::FxHashMap::default();
    for m in &meta {
        if !m.is_multiline {
            by_line.entry(m.start_line).or_default().push(m);
        }
    }

    let mut targets = vec![BufferLineCol::new(0); n_slots];

    // k == 0: the only thing slot-0 can compress is its own preceding
    // whitespace run, down to its display-cell width `rem_cells₀`. So the
    // minimum reachable anchor is anchor_display_col₀ − rem_cells₀.
    // `retreat_saturating` (not a bare subtraction) for the (unlikely)
    // backward-selection case where anchor_display_col < rem_cells, which it
    // clamps to 0 rather than wrapping on.
    let fit_0 = by_line
        .values()
        .filter_map(|ms| ms.iter().find(|m| m.slot == Some(0)))
        .map(|m| m.anchor_display_col.retreat_saturating(m.rem_cells))
        .max()
        .unwrap_or(BufferLineCol::new(0));
    targets[0] = baseline[0].max(fit_0);

    // k >= 1: placing target[k-1] shifts every anchor on that line by
    // (target[k-1] − anchor_display_col_{k-1}). Slot k then shifts by the
    // same amount, so its new anchor is
    // anchor_display_col_k + (target[k-1] − anchor_display_col_{k-1}). The
    // minimum feasible target[k] (leaving at least the one kept separator
    // char before slot k) is:
    //   target[k-1] + (anchor_display_col_k − anchor_display_col_{k-1}) − rem_cells_k
    // where rem_cells_k is the display width slot k may compress (`rem_k`
    // chars' worth, one char short of the whole run).
    for k in 1..n_slots {
        let fit_k = by_line
            .values()
            .filter_map(|ms| {
                let prev = ms.iter().find(|m| m.slot == Some(k - 1))?;
                let cur = ms.iter().find(|m| m.slot == Some(k))?;
                let delta = cur.anchor_display_col.get() as isize
                    - prev.anchor_display_col.get() as isize
                    - cur.rem_cells as isize;
                Some(targets[k - 1].shift_saturating(delta))
            })
            .max()
            .unwrap_or(BufferLineCol::new(0));
        targets[k] = baseline[k].max(fit_k);
    }

    // ── Pass 3: apply ──────────────────────────────────────────────────────────

    // `line_shift` tracks the net display-cell delta on the current line so
    // far, approximating the shift from original-buffer anchor display
    // columns to post-edit ones. Both branches now measure cells exactly
    // (insertion is always spaces; removal resolves its char count from the
    // exact cell need, padding any tab-overshoot). The one residual
    // imprecision: `line_shift` sums *original-buffer* cell deltas applied to
    // *original-buffer* columns, so a tab sitting between two slots on the
    // same line is still weighed at its pre-edit stop rather than its
    // post-edit one — zero for a line's first slot, and unchanged by this fix.
    let mut current_line: Option<ContentLine> = None;
    let mut line_shift = 0isize;

    apply_edit(text, sels, |b, text, i, sel, new_sels| {
        let sel_start = sel.start();
        let content_len = sel.end_exclusive(text).chars_since(sel_start);
        let forward = sel.anchor() <= sel.head();
        let start_line = text.char_to_line(sel_start);

        if Some(start_line) != current_line {
            current_line = Some(start_line);
            line_shift = 0;
        }

        match meta[i].slot {
            None => {
                // Extras + multiline: retain up to sel_start, capture the global
                // delta (from all edits before this position), retain the
                // content, push shifted selection.
                b.retain(sel_start.chars_since(b.old_pos()));
                let delta = b.new_pos().index() as isize - b.old_pos().index() as isize;
                b.retain(content_len);
                let new_anchor = sel.anchor().shift(delta);
                let new_head = sel.head().shift(delta);
                new_sels.push(Selection::new(new_anchor, new_head));
            }
            Some(slot) => {
                let target = targets[slot];
                // Adjust the original anchor display column by the net shift
                // from earlier edits on this line to get the current anchor
                // display column.
                // Measured in pass 1 from the same (still unedited) text —
                // a `Some(slot)` meta is exactly one that took pass 1's
                // single-line branch, which is what populates this field.
                let anchor_display_col_now =
                    meta[i].anchor_display_col.shift_saturating(line_shift);
                let amount = target.get() as isize - anchor_display_col_now.get() as isize;

                if amount > 0 {
                    b.retain(sel_start.chars_since(b.old_pos()));
                    b.insert(&" ".repeat(amount as usize));
                    line_shift += amount;
                } else if amount < 0 {
                    // Remove whitespace immediately before sel_start, resolving
                    // the needed cell count back to a char count. Measured in
                    // original-buffer columns throughout (`start_display_col`,
                    // `threshold`, `freed`) — the same origin `need` (derived
                    // from `amount`, itself anchor-based) already assumes;
                    // mixing origins across this subtraction would be worse
                    // than the approximation `line_shift` already makes below.
                    let need = (-amount) as u32;
                    // Same unedited-text walk pass 1 already did for this
                    // selection (see `SelMeta::start_display_col`'s doc) —
                    // reused rather than repeated.
                    let start_display_col = meta[i].start_display_col;
                    let max_remove = meta[i].rem.min(sel_start.chars_since(b.old_pos()));
                    // Largest position whose column is still `need` cells left
                    // of sel_start: char_pos_at_display_col stops *before* a
                    // grapheme that would overshoot, so a tab straddling the
                    // threshold is deleted whole and the surplus padded back.
                    let threshold = start_display_col.retreat_saturating(need);
                    let cut = char_pos_at_display_col(text, start_line, threshold, tab_width);
                    let remove = sel_start.chars_since(cut).min(max_remove);
                    let cut_pos = sel_start.retreat(remove);
                    let freed = start_display_col
                        .cells_since(display_col_in_line(text, start_line, cut_pos, tab_width));
                    // 0 unless a tab's granularity overshot the exact target.
                    let pad = freed.saturating_sub(need);
                    b.retain(cut_pos.chars_since(b.old_pos()));
                    if remove > 0 {
                        b.delete(remove);
                    }
                    if pad > 0 {
                        b.insert(&" ".repeat(pad as usize));
                    }
                    line_shift += pad as isize - freed as isize;
                } else {
                    b.retain(sel_start.chars_since(b.old_pos()));
                }

                // b.old_pos() is now at sel_start. Record the mapped start, retain
                // content, then push the new selection preserving direction.
                let new_start = b.new_pos();
                b.retain(content_len);
                // Use sel.end() (not end_inclusive) so anchor/head land on the
                // grapheme boundary rather than on a trailing combining codepoint.
                let new_end = new_start.shift(sel.end().chars_since(sel_start) as isize);
                new_sels.push(Selection::directed(new_start, new_end, forward));
            }
        }
    })
}
