//! `>` / `<` — shift every line touched by a selection by whole indent levels.

use hume_editing::changeset::{Assoc, ChangeSet, ChangeSetBuilder, PosMapCursor};
use hume_editing::lines::leading_indent;
use hume_editing::selection::{Selection, SelectionSet, is_selection_linewise};
use hume_editing::tab_style::TabStyle;
use hume_editing::text::BufferText;
use hume_rope::width::indent_stop;

/// Indent every line touched by a selection by `levels` indent levels (`>`).
pub fn indent_lines(
    text: BufferText,
    sels: SelectionSet,
    style: TabStyle,
    tab_width: u8,
    levels: usize,
) -> (BufferText, SelectionSet, ChangeSet) {
    let delta_display_col = indent_stop(levels as u32, tab_width) as isize;
    shift_indent(text, sels, style, tab_width, delta_display_col)
}

/// Unindent every line touched by a selection by `levels` indent levels (`<`).
pub fn unindent_lines(
    text: BufferText,
    sels: SelectionSet,
    style: TabStyle,
    tab_width: u8,
    levels: usize,
) -> (BufferText, SelectionSet, ChangeSet) {
    let delta_display_col = -(indent_stop(levels as u32, tab_width) as isize);
    shift_indent(text, sels, style, tab_width, delta_display_col)
}

/// Render a leading-whitespace run of exactly `width` display columns in
/// `style`. Lives here rather than beside `hume_rope::width`'s `indent_depth`/
/// `indent_stop` because it needs `TabStyle`, which sits in `hume-editing`,
/// above `hume-rope` in the dependency graph.
fn render_indent(width: usize, style: TabStyle, tab_width: u8) -> String {
    match style {
        TabStyle::Soft => " ".repeat(width),
        TabStyle::Hard => {
            let tw = (tab_width as usize).max(1);
            std::iter::repeat_n('\t', width / tw)
                .chain(std::iter::repeat_n(' ', width % tw))
                .collect()
        }
    }
}

/// Shared implementation for [`indent_lines`]/[`unindent_lines`]: one signed
/// display-column delta (positive indents, negative unindents), since the two
/// are otherwise identical. Callers pass columns — via
/// [`indent_stop`] — rather than levels, so this function never re-derives
/// "how many columns is a level" itself.
///
/// **Width-preserving, not level-snapping**: each touched line's indent
/// display-width shifts by `delta_display_col`, then that exact width is
/// re-rendered in `style` — an indent that isn't already a whole number of
/// levels (e.g. a continuation line hand-aligned to an open paren) shifts by
/// the requested amount without being rounded onto a tab stop first. This
/// also makes `<` exactly invert `>` (Vim's default, no `shiftround`);
/// snapping to levels first would make round-tripping lossy.
///
/// Iterates lines directly rather than going through [`super::apply_edit`]
/// (built for one edit per *selection*, not per *line*) — same reason
/// `sort_rows` drives a [`ChangeSetBuilder`] by hand instead.
fn shift_indent(
    text: BufferText,
    sels: SelectionSet,
    style: TabStyle,
    tab_width: u8,
    delta_display_col: isize,
) -> (BufferText, SelectionSet, ChangeSet) {
    // Every distinct line touched by any selection, ascending — `iter_sorted()`
    // is ascending/non-overlapping and each selection's own line range is
    // ascending, so a plain consecutive-dedup is enough (mirrors
    // `sort::collect_rows`).
    let mut lines: Vec<usize> = sels
        .iter_sorted()
        .flat_map(|sel| text.char_to_line(sel.start())..=text.char_to_line(sel.content_end(&text)))
        .collect();
    lines.dedup();

    let mut b = ChangeSetBuilder::new(text.len_chars());
    let mut touched_any = false;

    for line in lines {
        let line_start = text.line_to_char(line);
        let (ws_end, old_width) = leading_indent(&text, line, tab_width);
        // Blank line (empty, or whitespace-only): every line char up to the
        // structural/line '\n' is whitespace, so `leading_indent`'s scan runs
        // off the end without finding a non-whitespace char. Skipped
        // untouched — matches Vim's `>>`, so a blank separator line never
        // collects trailing whitespace.
        if text.char_at(ws_end) == Some('\n') {
            continue;
        }
        let new_width = old_width.saturating_add_signed(delta_display_col);
        if new_width == old_width {
            // Reachable at `delta_display_col == 0` (a `levels == 0` call — never
            // issued by the editor's own count dispatch, but this crate's ops
            // are a public API), or via the saturating clamp when unindenting
            // an already-flush line past width 0. Nothing to rewrite or remap.
            continue;
        }
        let new_indent = render_indent(new_width, style, tab_width);
        let old_len = ws_end - line_start;

        b.retain(line_start - b.old_pos());
        // Insert before delete (not the delete-then-insert order every other
        // `hume-ops` edit uses): it puts the new indent's `Insert` op at
        // exactly the old-doc position of the touched line's start, so
        // `PosMapCursor` resolves an endpoint sitting there by `Assoc`
        // instead of unconditionally collapsing it into the following
        // `Delete` — that's what lets the remap below reuse the ChangeSet
        // itself rather than a hand-kept table of before/after line offsets.
        b.insert(&new_indent);
        b.delete(old_len);
        touched_any = true;
    }

    b.retain_rest();
    let cs = b.finish();
    if !touched_any {
        // No touched line changed width (all blank, or a `<` saturating at
        // an already-flush indent) — every position is already correct as
        // is. `cs` is `ChangeSet::identity` here (only `retain_rest` ran).
        // Not needed for undo bookkeeping (an identity `ChangeSet` already
        // short-circuits before a revision is recorded — see
        // `Buffer::apply_edit`/`doc_ops::finish_edit`); this just skips the
        // no-op rope clone/apply and the selection remap below.
        debug_assert!(cs.is_identity());
        return (text, sels, cs);
    }
    let new_text = cs
        .apply(&text)
        .expect("indent/unindent produced an invalid changeset — this is a bug");

    // One monotone `PosMapCursor` pass over every selection endpoint, same
    // shape as `SelectionSet::translate_in_place_with`. A linewise
    // selection's start is exactly a rewritten line's start by definition
    // (`is_selection_linewise`) and must stay there — `Assoc::Before` sticks
    // to what was left of the insertion point, landing on the new line
    // start. Every other endpoint — an ordinary cursor that merely happens
    // to sit at column 0, or one buried in the old indent — clamps past the
    // new indent instead: `Assoc::After` moves past the `Insert`, and any
    // deeper old-indent position falls into the following `Delete` and
    // collapses to that same point regardless of `Assoc` (a position inside
    // a deletion is never ambiguous).
    let mut mapper = PosMapCursor::new(cs.ops());
    let new_sels: Vec<Selection> = sels
        .iter_sorted()
        .map(|sel| {
            let assoc = if is_selection_linewise(&text, sel) {
                Assoc::Before
            } else {
                Assoc::After
            };
            let forward = sel.anchor() <= sel.head();
            let lo = mapper.map(sel.start(), assoc);
            let hi = mapper.map(sel.end(), assoc);
            if forward {
                Selection::new(lo, hi)
            } else {
                Selection::new(hi, lo)
            }
        })
        .collect();
    let new_sel_set = SelectionSet::from_vec(new_sels, sels.primary_index());
    new_sel_set.debug_assert_valid(&new_text);
    (new_text, new_sel_set, cs)
}
