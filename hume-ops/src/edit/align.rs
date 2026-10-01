//! `align-selections`: align each selection's anchor to the primary
//! selection's anchor display column.

use hume_editing::edit::Edited;
use hume_editing::edit::Landing;
use hume_editing::state::EditState;
use hume_rope::column::BufferLineCol;
use hume_rope::line::ContentLine;
use hume_rope::offset::{CharOffset, ExclusiveRange};

use super::apply_edit;

/// Align selections into slots, using the primary's line as a baseline.
///
/// The primary's line has `N` single-line selections, one slot each, left to
/// right. On every other line the k-th single-line selection aligns to slot
/// `k`. Extras (slot >= N) and multiline selections are only shifted by the
/// accumulated edit delta.
///
/// Slots are solved left to right: `target[k] = max(baseline[k], fit_need[k])`.
/// `baseline[k]` is the anchor display column of the primary line's k-th
/// selection. `fit_need[k]` is the smallest column every line's slot-`k`
/// selection can reach, given `target[k-1]`: a selection can only shrink the
/// space/tab run just before it (down to one column), so the primary line
/// may move too. The anchor is the left edge going forward and the right
/// edge going backward, which gives left- and right-alignment.
///
/// The removable run is tracked as `rem` chars (what may be deleted) and
/// `rem_cells` display cells (what the target math uses), so `fit_need` is
/// exact. Deleting a tab can free more cells than needed; the surplus is
/// padded back with spaces so every selection lands on `target`.
pub fn align_selections(state: EditState, tab_width: u8) -> Edited {
    let text = state.text();
    let view = state.view();
    // ── Pass 1: measure ────────────────────────────────────────────────────────

    // Geometry for each selection in sorted order.
    struct SelMeta {
        start_line: ContentLine,
        is_multiline: bool,
        anchor_display_col: BufferLineCol, // display col of sel.anchor() (left for forward, right for backward)
        start_display_col: BufferLineCol, // display col of sel.start(), the same value pass 3 re-derives from the same unedited text, cached here to avoid the second walk
        rem: usize, // chars removable before sel.start(): the whitespace run back to the previous selection's end or the line start, less one kept space
        rem_cells: u32, // display-cell width of the `rem`-char run (tab-aware)
        slot: Option<usize>, // None = multiline or extra (slot >= N)
    }

    let primary_line = text.char_to_line(view.primary().anchor().offset());
    let mut slots_on_line = rustc_hash::FxHashMap::<ContentLine, usize>::default();

    let mut previous_end = CharOffset::new(0);
    let mut meta: Vec<SelMeta> = view
        .iter()
        .map(|sel| {
            let run_floor = previous_end;
            previous_end = sel.covered().end().offset();
            let lines = sel.lines();
            let start_line = lines.start;
            let is_multiline = lines.start != lines.end;
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
                text.columns()
                    .display_col(start_line, sel.anchor().offset(), tab_width);
            let line_start = text.line_to_char(start_line.into());
            let sel_start = sel.start().offset();
            // `sel.start()` is `anchor.min(head)`, so for a forward selection
            // (anchor <= head) it's the anchor itself, so reuse the column just
            // walked above rather than walking the same prefix again. Only a
            // backward selection (head == start, anchor == end) needs its own walk.
            let start_display_col = if sel.anchor() <= sel.head() {
                anchor_display_col
            } else {
                text.columns().display_col(start_line, sel_start, tab_width)
            };
            let rem = (run_floor.max(line_start).index()..sel_start.index())
                .rev()
                .take_while(|&p| matches!(text.char_at(CharOffset::new(p)), Some(' ') | Some('\t')))
                .count()
                .saturating_sub(1);
            // The `rem`-char run's display width, tab-aware. Measured from
            // `sel_start`, not `anchor_display_col`: the run always ends at
            // the selection's left edge, which for a backward multi-char
            // selection is the head, not the (right-edge) anchor. Using
            // the anchor's column here would fold the selection's own
            // content width into the run's width.
            let run_start = sel_start.retreat(rem);
            let rem_cells = start_display_col
                .cells_since(text.columns().display_col(start_line, run_start, tab_width));
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
        // Primary is multiline: no slot structure, everything passes through.
        return Edited::unchanged(state);
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
    // Values are in slot order (selections are in document order).
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
    // columns to post-edit ones. Both branches measure cells exactly
    // (insertion is always spaces; removal resolves its char count from the
    // exact cell need, padding any tab-overshoot). The one residual
    // imprecision: `line_shift` sums *original-buffer* cell deltas applied to
    // *original-buffer* columns, so a tab sitting between two slots on the
    // same line is still weighed at its pre-edit stop rather than its
    // post-edit one. That error is zero for a line's first slot.
    let mut current_line: Option<ContentLine> = None;
    let mut line_shift = 0isize;

    apply_edit(state, |b, sel| {
        let i = sel.index();
        let text = b.text();
        let sel_start = sel.start().offset();
        let start_line = sel.lines().start;

        if Some(start_line) != current_line {
            current_line = Some(start_line);
            line_shift = 0;
        }

        if let Some(slot) = meta[i].slot {
            let target = targets[slot];
            // The original anchor display column moved by the net shift from
            // earlier edits on this line.
            let anchor_display_col_now = meta[i].anchor_display_col.shift_saturating(line_shift);
            let amount = target.get() as isize - anchor_display_col_now.get() as isize;

            if amount > 0 {
                b.insert(sel.start(), &" ".repeat(amount as usize));
                line_shift += amount;
            } else if amount < 0 {
                // Remove whitespace immediately before sel_start, resolving
                // the needed cell count back to a char count. Measured in
                // original-buffer columns throughout (`start_display_col`,
                // `threshold`, `freed`), the same origin `need` (derived
                // from `amount`, itself anchor-based) already assumes;
                // mixing origins across this subtraction would be worse
                // than the approximation `line_shift` already makes.
                let need = (-amount) as u32;
                let start_display_col = meta[i].start_display_col;
                // Largest position whose column is still `need` cells left
                // of sel_start: char_pos_at_display_col stops *before* a
                // grapheme that would overshoot, so a tab straddling the
                // threshold is deleted whole and the surplus padded back.
                let threshold = start_display_col.retreat_saturating(need);
                let cut = text
                    .columns()
                    .pos_at_display_col(start_line, threshold, tab_width);
                let remove = sel_start.chars_since(cut.offset()).min(meta[i].rem);
                let run = ExclusiveRange::new(sel_start.retreat(remove), sel_start);
                if let Some(removed) = text.within(run) {
                    let freed = start_display_col.cells_since(text.columns().display_col(
                        start_line,
                        removed.start().offset(),
                        tab_width,
                    ));
                    // 0 unless a tab's granularity overshot the exact target.
                    let pad = freed.saturating_sub(need);
                    b.delete(removed);
                    if pad > 0 {
                        b.insert(sel.start(), &" ".repeat(pad as usize));
                    }
                    line_shift += pad as isize - freed as isize;
                }
            }
        }

        // Extras and multiline selections only move with the edits before
        // them.
        let mark = b.keep(sel.covered());
        Landing::covering(mark, sel.facing())
    })
}
