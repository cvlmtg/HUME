//! `>` / `<` — shift every line touched by a selection by whole indent levels.

use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::grapheme::display_col_in_line;
use hume_editing::lines::leading_whitespace_end;
use hume_editing::selection::{Selection, SelectionSet, is_selection_linewise};
use hume_editing::tab_style::TabStyle;
use hume_editing::text::BufferText;

/// Indent every line touched by a selection by `levels` indent levels (`>`).
pub fn indent_lines(
    text: BufferText,
    sels: SelectionSet,
    style: TabStyle,
    tab_width: u8,
    levels: usize,
) -> (BufferText, SelectionSet, ChangeSet) {
    shift_indent(text, sels, style, tab_width, levels as isize)
}

/// Unindent every line touched by a selection by `levels` indent levels (`<`).
pub fn unindent_lines(
    text: BufferText,
    sels: SelectionSet,
    style: TabStyle,
    tab_width: u8,
    levels: usize,
) -> (BufferText, SelectionSet, ChangeSet) {
    shift_indent(text, sels, style, tab_width, -(levels as isize))
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
            let tabs = width / tw;
            let spaces = width % tw;
            let mut s = String::with_capacity(tabs + spaces);
            s.extend(std::iter::repeat_n('\t', tabs));
            s.extend(std::iter::repeat_n(' ', spaces));
            s
        }
    }
}

/// One rewritten line's before/after geometry, in ascending line order —
/// enough to both drive the [`ChangeSetBuilder`] and remap selections
/// afterward without re-scanning the buffer.
struct LineEdit {
    line: usize,
    line_start: usize,
    /// End of the line's *old* leading-whitespace run — the clamp target for
    /// a selection endpoint that sat inside the old indent.
    ws_end: usize,
    new_len: usize,
    /// Net char delta from every rewritten line strictly before this one.
    cum_delta_before: isize,
    /// Net char delta including this line's own rewrite — the shift that
    /// applies to every position from here to the next rewritten line.
    delta_after: isize,
}

/// Shared implementation for [`indent_lines`]/[`unindent_lines`]: one levels
/// value (sign encodes direction), since the two are otherwise identical.
///
/// **Width-preserving, not level-snapping**: each touched line's indent
/// display-width shifts by `delta_levels * tab_width`, then that exact width
/// is re-rendered in `style` — an indent that isn't already a whole number of
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
    delta_levels: isize,
) -> (BufferText, SelectionSet, ChangeSet) {
    // Every distinct line touched by any selection, ascending and
    // deduplicated by construction: `iter_sorted()` is ascending and
    // non-overlapping, and each selection's own line range is ascending, so
    // only a same-as-last-pushed check is needed (mirrors `sort::collect_rows`).
    let mut lines: Vec<usize> = Vec::new();
    for sel in sels.iter_sorted() {
        let start_line = text.char_to_line(sel.start());
        let end_line = text.char_to_line(sel.content_end(&text));
        for line in start_line..=end_line {
            if lines.last() != Some(&line) {
                lines.push(line);
            }
        }
    }

    let tw_isize = tab_width as isize;
    let mut b = ChangeSetBuilder::new(text.len_chars());
    let mut edits: Vec<LineEdit> = Vec::new();
    let mut cum_delta: isize = 0;

    for line in lines {
        let line_start = text.line_to_char(line);
        let ws_end = leading_whitespace_end(&text, line);
        // Blank line (empty, or whitespace-only): every line char up to the
        // structural/line '\n' is whitespace, so `leading_whitespace_end`'s
        // scan runs off the end without finding a non-whitespace char.
        // Skipped untouched — matches Vim's `>>`, so a blank separator line
        // never collects trailing whitespace.
        if text.char_at(ws_end) == Some('\n') {
            continue;
        }
        let old_width = display_col_in_line(&text, line, ws_end, tab_width);
        let new_width = old_width.saturating_add_signed(delta_levels.saturating_mul(tw_isize));
        if new_width == old_width {
            // Only reachable via the clamp above: unindenting an
            // already-flush line. Nothing to rewrite or remap.
            continue;
        }
        let new_indent = render_indent(new_width, style, tab_width);
        let old_len = ws_end - line_start;
        let new_len = new_indent.chars().count();

        b.retain(line_start - b.old_pos());
        b.delete(old_len);
        b.insert(&new_indent);

        let delta_after = cum_delta + (new_len as isize - old_len as isize);
        edits.push(LineEdit {
            line,
            line_start,
            ws_end,
            new_len,
            cum_delta_before: cum_delta,
            delta_after,
        });
        cum_delta = delta_after;
    }

    if edits.is_empty() {
        // A distinct identity return (not just an edit with a no-op
        // ChangeSet) matters here for the same reason `sort_rows` returns
        // `Err`: `Buffer::apply_edit` records an undo revision unconditionally,
        // so applying an identity ChangeSet would still dirty a clean buffer.
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
    // past the old indent, or any other line entirely — is a uniform shift by
    // the total delta accumulated up to that point.
    let map_pos = |p: usize, keep_at_line_start: bool| -> usize {
        let line = text.char_to_line(p);
        match edits.binary_search_by_key(&line, |e| e.line) {
            Ok(idx) => {
                let e = &edits[idx];
                if keep_at_line_start && p == e.line_start {
                    (e.line_start as isize + e.cum_delta_before) as usize
                } else if p < e.ws_end {
                    (e.line_start as isize + e.cum_delta_before + e.new_len as isize) as usize
                } else {
                    (p as isize + e.delta_after) as usize
                }
            }
            Err(idx) => {
                let delta = if idx == 0 {
                    0
                } else {
                    edits[idx - 1].delta_after
                };
                (p as isize + delta) as usize
            }
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
