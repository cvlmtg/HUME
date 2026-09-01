//! `>` / `<` — shift every line touched by a selection by whole indent levels.

use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
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

/// One rewritten line's before/after geometry — enough to remap selections
/// afterward via a binary search on `line`, without re-scanning the buffer.
/// The `new_*` fields are read straight off [`ChangeSetBuilder::new_pos`] at
/// the moment each is known, rather than carried through a hand-kept delta
/// accumulator — the builder already tracks that (its own doc: "no separate
/// delta accumulator needed").
#[derive(Clone, Copy)]
struct LineEdit {
    line: usize,
    line_start: usize,
    /// End of the line's *old* leading-whitespace run — the clamp target for
    /// a selection endpoint that sat inside the old indent.
    ws_end: usize,
    /// `line_start`'s position in the new buffer.
    new_line_start: usize,
    /// End of the line's *new* leading-whitespace run.
    new_ws_end: usize,
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
    let mut edits: Vec<LineEdit> = Vec::new();

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
        let new_line_start = b.new_pos();
        b.delete(old_len);
        b.insert(&new_indent);
        let new_ws_end = b.new_pos();

        edits.push(LineEdit {
            line,
            line_start,
            ws_end,
            new_line_start,
            new_ws_end,
        });
    }

    if edits.is_empty() {
        // No touched line changed width (all blank, or a `<` saturating at
        // an already-flush indent) — every position is already correct as
        // is. Not needed for undo bookkeeping (an identity `ChangeSet`
        // already short-circuits before a revision is recorded — see
        // `Buffer::apply_edit`/`doc_ops::finish_edit`); this just skips the
        // no-op rope clone/apply and the selection remap below.
        let len = text.len_chars();
        return (text, sels, ChangeSet::identity(len));
    }

    b.retain_rest();
    let cs = b.finish();
    let new_text = cs
        .apply(&text)
        .expect("indent/unindent produced an invalid changeset — this is a bug");

    // Maps one original-buffer position through every rewritten line at or
    // before it. A position strictly inside a rewritten line's old indent
    // clamps to the end of the new indent — shifting it by the line's raw
    // char delta would walk it onto the wrong line when the indent shrinks by
    // more than the position's own offset into it. `keep_at_line_start` is
    // the one exception: a linewise selection's start is exactly the line
    // start by definition (`is_selection_linewise`), and must stay there
    // (rather than clamp forward past the new indent) so the selection still
    // covers the whole rewritten line, indent included — an ordinary cursor
    // that merely happens to sit at column 0 gets no such exception, and
    // clamps like any other in-indent position. Everywhere else — content
    // past the old indent, or any other line entirely — shifts by the
    // nearest preceding rewritten line's own net width change.
    let shift = |p: usize, e: &LineEdit| p - e.ws_end + e.new_ws_end;
    let map_pos = |p: usize, keep_at_line_start: bool| -> usize {
        let line = text.char_to_line(p);
        match edits.binary_search_by_key(&line, |e| e.line) {
            Ok(idx) => {
                let e = edits[idx];
                if keep_at_line_start && p == e.line_start {
                    e.new_line_start
                } else if p < e.ws_end {
                    e.new_ws_end
                } else {
                    shift(p, &e)
                }
            }
            Err(idx) => idx.checked_sub(1).map_or(p, |j| shift(p, &edits[j])),
        }
    };

    let new_sels: Vec<Selection> = sels
        .iter_sorted()
        .map(|sel| {
            let linewise = is_selection_linewise(&text, sel);
            Selection::new(
                map_pos(sel.anchor(), linewise),
                map_pos(sel.head(), linewise),
            )
        })
        .collect();
    let new_sel_set = SelectionSet::from_vec(new_sels, sels.primary_index());
    new_sel_set.debug_assert_valid(&new_text);
    (new_text, new_sel_set, cs)
}
