use regex_cursor::engines::meta::Regex;

use editing::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use editing::selection::{Selection, SelectionSet};
use editing::text::Text;
use editing::helpers::{CharClass, classify_char, line_content_end};
use crate::ops::search::find_matches_in_range;
use crate::ops::MotionMode;

// ── Split on newlines ─────────────────────────────────────────────────────────

/// Split each multi-line selection into one selection per line.
///
/// Single-line selections are left unchanged. For a selection spanning lines
/// L1..L2:
/// - Line L1: from the selection's start to the last non-`\n` char on L1
///   (or the `\n` itself if the line is empty).
/// - Lines L1+1..L2-1: full lines from start to last non-`\n` char.
/// - Line L2: from the line start to the selection's end.
///
/// The direction (forward/backward) of the original selection is preserved on
/// every piece. The primary becomes the first piece of the original primary.
pub(crate) fn cmd_split_selection_on_newlines(
    buf: &Text,
    sels: SelectionSet,
    _mode: MotionMode,
) -> SelectionSet {
    let primary_idx = sels.primary_index();
    let mut new_sels: Vec<Selection> = Vec::new();
    // Maps each old selection (by sorted index) to the first index of its
    // pieces in `new_sels`.
    let mut piece_start: Vec<usize> = Vec::new();

    for sel in sels.iter_sorted() {
        let start = sel.start();
        let end = sel.end();
        let start_line = buf.char_to_line(start);
        let end_line = buf.char_to_line(end);
        let forward = sel.anchor() <= sel.head();

        let first_piece_idx = new_sels.len();

        if start_line == end_line {
            // Single-line: keep as-is.
            new_sels.push(*sel);
        } else {
            // First line piece: from selection start to end of line content.
            let first_end = line_content_end(buf, start_line);
            let sel = Selection::directed(start, first_end, forward);
            new_sels.push(sel);

            // Middle lines: full lines.
            for line in (start_line + 1)..end_line {
                let ls = buf.line_to_char(line);
                let le = line_content_end(buf, line);
                let sel = Selection::directed(ls, le, forward);
                new_sels.push(sel);
            }

            // Last line piece: from line start to selection end.
            let last_ls = buf.line_to_char(end_line);
            let sel = Selection::directed(last_ls, end, forward);
            new_sels.push(sel);
        }

        piece_start.push(first_piece_idx);
    }

    // The new primary is the first piece of the original primary.
    let new_primary = piece_start[primary_idx];
    // Split selections cover disjoint line ranges and can't overlap, so no
    // merge is needed. `from_vec` preserves the sorted order we built.
    let new_set = SelectionSet::from_vec(new_sels, new_primary);
    new_set.debug_assert_valid(buf);
    new_set
}

// ── Select matches within ─────────────────────────────────────────────────────

/// Replace each selection with the regex matches found within it.
///
/// For every selection in `sels`, finds all non-overlapping matches of `regex`
/// bounded to that selection's range. Each match becomes a new forward
/// `Selection`. The new primary is the first match within the original primary
/// selection's range.
///
/// Returns `None` when no matches are found in any selection — the caller
/// should keep the original selections unchanged.
pub(crate) fn select_matches_within(
    buf: &Text,
    sels: &SelectionSet,
    regex: &Regex,
) -> Option<SelectionSet> {
    let primary_idx = sels.primary_index();
    let mut new_sels: Vec<Selection> = Vec::new();
    let mut new_primary = 0;

    for (i, sel) in sels.iter_sorted().enumerate() {
        let piece_start = new_sels.len();
        let matches = find_matches_in_range(buf, regex, sel.start(), sel.end_inclusive(buf));

        for (s, e) in matches {
            new_sels.push(Selection::new(s, e));
        }

        // Primary = first match within the original primary selection.
        if i == primary_idx && piece_start < new_sels.len() {
            new_primary = piece_start;
        }
    }

    if new_sels.is_empty() {
        return None;
    }

    // Matches within non-overlapping selections can't overlap each other,
    // so no merge is needed.
    let new_set = SelectionSet::from_vec(new_sels, new_primary);
    new_set.debug_assert_valid(buf);
    Some(new_set)
}

// ── Trim whitespace ───────────────────────────────────────────────────────────

/// Trim leading and trailing whitespace from every selection's range.
///
/// "Whitespace" here means space (` `), tab (`\t`), and newline (`\n`). The
/// range shrinks inward until both ends sit on non-whitespace characters. If
/// the entire selection is whitespace the selection collapses to a cursor at
/// the original `head`.
pub(crate) fn cmd_trim_selection_whitespace(
    buf: &Text,
    sels: SelectionSet,
    _mode: MotionMode,
) -> SelectionSet {
    let new_sels = sels.map_and_merge(|sel| {
        let mut start = sel.start();
        let end = sel.end();
        let forward = sel.anchor() <= sel.head();

        // Walk forward from start, skipping whitespace (grapheme boundary steps).
        // `classify_char` is the authoritative whitespace definition for this
        // codebase — Space covers ' '/'\t', Eol covers '\n'.
        while start <= end
            && matches!(
                buf.char_at(start).map(classify_char),
                Some(CharClass::Space | CharClass::Eol)
            )
        {
            start = next_grapheme_boundary(buf, start);
        }

        // If we consumed everything, the selection is all whitespace.
        if start > end {
            return Selection::collapsed(sel.head());
        }

        // Walk backward from end, skipping whitespace (grapheme boundary steps).
        let mut new_end = end;
        while new_end > start
            && matches!(
                buf.char_at(new_end).map(classify_char),
                Some(CharClass::Space | CharClass::Eol)
            )
        {
            new_end = prev_grapheme_boundary(buf, new_end);
        }

        Selection::directed(start, new_end, forward)
    });
    new_sels.debug_assert_valid(buf);
    new_sels
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assert_state;
    use crate::testing::parse_state;
    use pretty_assertions::assert_eq;

    // ── cmd_split_selection_on_newlines ────────────────────────────────────

    #[test]
    fn split_single_line_is_noop() {
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| cmd_split_selection_on_newlines(&buf, sels, MotionMode::Move),
            "-[hell]>o\n"
        );
    }

    #[test]
    fn split_two_line_selection() {
        // "foo\nbar\n", selection from 'f'(0) to 'r'(6) (cross-line forward).
        // "#[foo\nba|r]#\n" → anchor=0, head=6 (cursor on 'r').
        // After split: "foo" on line 0, "bar" on line 1.
        let (buf, sels) = parse_state("-[foo\nbar]>\n");
        let sels_out = cmd_split_selection_on_newlines(&buf, sels, MotionMode::Move);
        // Text unchanged (pure op).
        assert_eq!(buf.to_string(), "foo\nbar\n");
        // Two selections.
        assert_eq!(sels_out.len(), 2);
        let s: Vec<_> = sels_out.iter_sorted().copied().collect();
        // First: covers "foo" on line 0 (offsets 0–2).
        assert_eq!(s[0].start(), 0);
        assert_eq!(s[0].end(), 2);
        // Second: covers "bar" on line 1 (offsets 4–6).
        assert_eq!(s[1].start(), 4);
        assert_eq!(s[1].end(), 6);
        // Primary is first piece of original primary (index 0).
        assert_eq!(sels_out.primary_index(), 0);
    }

    #[test]
    fn split_three_line_selection() {
        // "a\nb\nc\n" — forward selection from 'a' to 'c'.
        let (buf, sels) = parse_state("-[a\nb\nc]>\n");
        let sels_out = cmd_split_selection_on_newlines(&buf, sels, MotionMode::Move);
        assert_eq!(sels_out.len(), 3);
        let s: Vec<_> = sels_out.iter_sorted().copied().collect();
        // Line 0: just 'a' at offset 0.
        assert_eq!(s[0].start(), 0);
        assert_eq!(s[0].end(), 0);
        // Line 1: just 'b' at offset 2.
        assert_eq!(s[1].start(), 2);
        assert_eq!(s[1].end(), 2);
        // Line 2: just 'c' at offset 4.
        assert_eq!(s[2].start(), 4);
        assert_eq!(s[2].end(), 4);
    }

    #[test]
    fn split_cursor_at_newline_is_noop() {
        // A cursor sitting on a newline character is a single-line selection
        // (the \n is part of its line).
        let (buf, sels) = parse_state("foo-[\n]>bar\n");
        let sels_out = cmd_split_selection_on_newlines(&buf, sels, MotionMode::Move);
        assert_eq!(sels_out.len(), 1);
        assert_eq!(sels_out.primary().head(), 3); // still on \n
    }

    #[test]
    fn split_empty_line_in_middle() {
        // "foo\n\nbar\n" — selection from 'f'(0) to 'r'(7) spans 3 lines.
        // Line 0: "foo\n", line 1: "\n" (empty), line 2: "bar\n".
        // Middle piece should be a cursor on the lone '\n' at offset 4.
        let (buf, sels) = parse_state("-[foo\n\nbar]>\n");
        let sels_out = cmd_split_selection_on_newlines(&buf, sels, MotionMode::Move);
        assert_eq!(sels_out.len(), 3);
        let s: Vec<_> = sels_out.iter_sorted().copied().collect();
        // Line 0: "foo" → offsets 0–2.
        assert_eq!(s[0].start(), 0);
        assert_eq!(s[0].end(), 2);
        // Line 1: empty → cursor on '\n' at offset 4.
        assert_eq!(s[1].start(), 4);
        assert_eq!(s[1].end(), 4);
        // Line 2: "bar" → offsets 5–7.
        assert_eq!(s[2].start(), 5);
        assert_eq!(s[2].end(), 7);
    }

    #[test]
    fn split_backward_multi_line_with_empty_line_preserves_direction() {
        // "foo\n\nbar\n" — backward selection spanning 3 lines including an
        // empty one. All 3 pieces must be backward, and the empty-line piece
        // must be a cursor on the '\n'.
        let (buf, sels) = parse_state("<[foo\n\nbar]-\n");
        let sels_out = cmd_split_selection_on_newlines(&buf, sels, MotionMode::Move);
        assert_eq!(sels_out.len(), 3);
        let s: Vec<_> = sels_out.iter_sorted().copied().collect();
        // All pieces must be backward (anchor >= head; cursor is anchor == head).
        assert!(s[0].anchor() >= s[0].head(), "line 0 should be backward");
        assert!(
            s[1].anchor() >= s[1].head(),
            "empty line should be cursor/backward"
        );
        assert!(s[2].anchor() >= s[2].head(), "line 2 should be backward");
        // Empty line: cursor on the lone '\n' at offset 4.
        assert_eq!(s[1].head(), 4);
    }

    #[test]
    fn split_backward_multi_line_preserves_direction() {
        // "foo\nbar\n" — backward selection: anchor=6('r'), head=0('f').
        // Each piece should be backward (anchor > head).
        let (buf, sels) = parse_state("<[foo\nbar]-\n");
        let sels_out = cmd_split_selection_on_newlines(&buf, sels, MotionMode::Move);
        assert_eq!(sels_out.len(), 2);
        let s: Vec<_> = sels_out.iter_sorted().copied().collect();
        // Both pieces should be backward selections.
        assert!(s[0].anchor() > s[0].head(), "line 0 piece should be backward");
        assert!(s[1].anchor() > s[1].head(), "line 1 piece should be backward");
    }

    #[test]
    fn split_selection_on_newlines_empty_buffer_is_noop() {
        // Empty buffer: cursor on the single structural '\n'. The cursor's
        // start_line == end_line → single-line branch → kept as-is.
        assert_state!(
            "-[\n]>",
            |(buf, sels)| cmd_split_selection_on_newlines(&buf, sels, MotionMode::Move),
            "-[\n]>"
        );
    }

    // ── cmd_trim_selection_whitespace ──────────────────────────────────────

    #[test]
    fn trim_leading_spaces() {
        // "  hello\n", forward selection covering the whole word + leading spaces.
        // "#[  hell|o]#\n" → anchor=0, head=6 (cursor on 'o', offsets:  (0) (1) h(2) e(3) l(4) l(5) o(6)).
        // After trim: start advances past the 2 spaces → start=2, end=6.
        let (buf, sels) = parse_state("-[  hello]>\n");
        let sels_out = cmd_trim_selection_whitespace(&buf, sels, MotionMode::Move);
        assert_eq!(sels_out.primary().start(), 2); // after the two spaces
        assert_eq!(sels_out.primary().end(), 6); // 'o' at offset 6
    }

    #[test]
    fn trim_trailing_spaces() {
        // "hello  \n", forward selection covering "hello  " (with trailing spaces).
        // "#[hello | ]#\n" → anchor=0, head=6 (cursor on second space).
        // After trim: end walks back past 2 spaces → end=4 ('o').
        let (buf, sels) = parse_state("-[hello  ]>\n");
        let sels_out = cmd_trim_selection_whitespace(&buf, sels, MotionMode::Move);
        assert_eq!(sels_out.primary().start(), 0);
        assert_eq!(sels_out.primary().end(), 4); // 'o' at offset 4
    }

    #[test]
    fn trim_all_whitespace_collapses_to_cursor_at_head() {
        // Selection covering only spaces — should collapse to cursor at head.
        let (buf, sels) = parse_state("-[    ]>\n");
        let sels_out = cmd_trim_selection_whitespace(&buf, sels, MotionMode::Move);
        assert!(sels_out.primary().is_collapsed());
        // Head was at offset 3 (the `|` position in DSL).
        assert_eq!(sels_out.primary().head(), 3);
    }

    #[test]
    fn trim_no_whitespace_is_noop() {
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| cmd_trim_selection_whitespace(&buf, sels, MotionMode::Move),
            "-[hell]>o\n"
        );
    }

    #[test]
    fn trim_tab_characters() {
        // "\thello\t\n" — selection from tab(0) to tab(6) inclusive.
        // After trim: start=1 ('h'), end=5 ('o').
        // "\thello\t\n": \t(0),h(1),e(2),l(3),l(4),o(5),\t(6),\n(7).
        let (buf, sels) = parse_state("-[\thello]>\t\n");
        let sels_out = cmd_trim_selection_whitespace(&buf, sels, MotionMode::Move);
        assert_eq!(sels_out.primary().start(), 1); // past leading tab
        assert_eq!(sels_out.primary().end(), 5); // 'o'
    }

    #[test]
    fn trim_backward_selection_preserves_direction() {
        // Backward selection covering "  hello\n": anchor=7('\n'), head=0.
        // After trim: spans 'h'(2) to 'o'(6), still backward.
        assert_state!(
            "<[  hello\n]-",
            |(buf, sels)| cmd_trim_selection_whitespace(&buf, sels, MotionMode::Move),
            "  <[hello]-\n"
        );
    }

    #[test]
    fn trim_empty_buffer_collapses() {
        // Only char is '\n' (whitespace) — all-whitespace selection collapses.
        assert_state!(
            "-[\n]>",
            |(buf, sels)| cmd_trim_selection_whitespace(&buf, sels, MotionMode::Move),
            "-[\n]>"
        );
    }

    // ── select_matches_within ─────────────────────────────────────────────

    #[test]
    fn select_matches_basic() {
        // Select "ab" within a selection that spans "aababab".
        let (buf, sels) = parse_state("-[aababab]>\n");
        let regex = regex_cursor::engines::meta::Regex::new("ab").unwrap();
        let result = select_matches_within(&buf, &sels, &regex).unwrap();
        // Expect 3 selections: (1,2), (3,4), (5,6)
        assert_eq!(result.len(), 3);
        assert_eq!((result.primary().anchor(), result.primary().head()), (1, 2));
    }

    #[test]
    fn select_matches_no_hits_returns_none() {
        let (buf, sels) = parse_state("-[hello]>\n");
        let regex = regex_cursor::engines::meta::Regex::new("xyz").unwrap();
        assert!(select_matches_within(&buf, &sels, &regex).is_none());
    }

    #[test]
    fn select_matches_bounded_to_selection() {
        // Only matches within the selection range should be found.
        // "ab" appears at (0,1) and (4,5) in "abcdab\n", but selection
        // covers only chars 2..3 ("cd") — no matches.
        let buf = Text::from("abcdab\n");
        let sels = SelectionSet::single(Selection::new(2, 3));
        let regex = regex_cursor::engines::meta::Regex::new("ab").unwrap();
        assert!(select_matches_within(&buf, &sels, &regex).is_none());
    }

    #[test]
    fn select_matches_multiple_selections() {
        // Two selections, each containing one "ab".
        let buf = Text::from("ab cd ab\n");
        let sel0 = Selection::new(0, 1); // "ab"
        let sel1 = Selection::new(6, 7); // "ab"
        let sels = SelectionSet::from_vec(vec![sel0, sel1], 0);
        let regex = regex_cursor::engines::meta::Regex::new("ab").unwrap();
        let result = select_matches_within(&buf, &sels, &regex).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn select_matches_backward_selection() {
        // Backward selection (anchor > head) should work identically.
        let buf = Text::from("aababab\n");
        let sels = SelectionSet::single(Selection::new(6, 0)); // backward
        let regex = regex_cursor::engines::meta::Regex::new("ab").unwrap();
        let result = select_matches_within(&buf, &sels, &regex).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!((result.primary().anchor(), result.primary().head()), (1, 2));
    }

    #[test]
    fn select_matches_single_char_match() {
        // Single-char regex matches produce cursor-sized selections.
        let (buf, sels) = parse_state("-[abc]>\n");
        let regex = regex_cursor::engines::meta::Regex::new("b").unwrap();
        let result = select_matches_within(&buf, &sels, &regex).unwrap();
        assert_eq!(result.len(), 1);
        let sel = result.primary();
        assert_eq!(sel.anchor(), 1);
        assert_eq!(sel.head(), 1);
        assert!(sel.is_collapsed());
    }

    #[test]
    fn select_matches_combining_grapheme() {
        // "café\n" where 'é' is e + U+0301 (2 codepoints at chars 3,4).
        // Selection covers the whole word. Matching "é" should produce a
        // selection spanning both codepoints (3,4).
        let buf = Text::from("caf\u{0065}\u{0301}\n");
        let sels = SelectionSet::single(Selection::new(0, 4));
        let regex = regex_cursor::engines::meta::Regex::new("\u{0065}\u{0301}").unwrap();
        let result = select_matches_within(&buf, &sels, &regex).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!((result.primary().anchor(), result.primary().head()), (3, 4));
    }
}
