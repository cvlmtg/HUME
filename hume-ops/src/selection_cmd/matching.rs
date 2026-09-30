use regex_cursor::engines::meta::Regex;

use crate::MotionMode;
use crate::search::find_matches_in_range;
use crate::text_object::trim_blank;
use hume_editing::lines::line_last_char;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;
use hume_rope::offset::InclusiveRange;

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
pub fn cmd_split_selection_on_newlines(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    _mode: MotionMode,
) -> SelectionSet {
    let primary_idx = sels.primary_index();
    let mut new_sels: Vec<Selection> = Vec::new();
    // Maps each old selection (by sorted index) to the first index of its
    // pieces in `new_sels`.
    let mut piece_start: Vec<usize> = Vec::new();

    for sel in sels.iter_sorted() {
        let span = sel.span(text);
        let start_line = text.char_to_line(span.start);
        let end_line = text.char_to_line(span.end);
        let forward = sel.anchor() <= sel.head();

        let first_piece_idx = new_sels.len();

        if start_line == end_line {
            // Single-line: keep as-is.
            new_sels.push(*sel);
        } else {
            // First line piece: from selection start to end of line content.
            let first_end = line_last_char(text, start_line);
            let sel =
                Selection::from_span(InclusiveRange::new(span.start, first_end), forward, text);
            new_sels.push(sel);

            // Middle lines: full lines. Bare-`usize` range, `ContentLine`
            // re-minted each iteration: `ContentLine` has no `Step`/`Range`
            // impl to loop over directly (see CLAUDE.md's "Line counts and
            // ranges"). Sound here: both endpoints are already-valid
            // `ContentLine`s.
            for line_idx in start_line.advance(1).index()..end_line.index() {
                let line = hume_rope::line::ContentLine::new(line_idx);
                let ls = text.line_to_char(line.into());
                let le = line_last_char(text, line);
                let sel = Selection::from_span(InclusiveRange::new(ls, le), forward, text);
                new_sels.push(sel);
            }

            // Last line piece: from line start to selection end.
            let last_ls = text.line_to_char(end_line.into());
            let sel = Selection::from_span(InclusiveRange::new(last_ls, span.end), forward, text);
            new_sels.push(sel);
        }

        piece_start.push(first_piece_idx);
    }

    // The new primary is the first piece of the original primary.
    let new_primary = piece_start[primary_idx];
    // Split selections cover disjoint line ranges and can't overlap. `from_vec`
    // sorts and merges, but the input is already sorted and disjoint, so both
    // are no-ops here and the primary index is preserved.
    let new_set = SelectionSet::from_vec(new_sels, new_primary);
    new_set.debug_assert_valid(text);
    new_set
}

// ── Sift matches within ────────────────────────────────────────────────────────

/// Replace each selection with the regex matches found within it.
///
/// For every selection in `sels`, finds all non-overlapping matches of `regex`
/// bounded to that selection's range. Each match becomes a new forward
/// `Selection`. The new primary is the first match within the original primary
/// selection's range.
///
/// Returns `None` when no matches are found in any selection; the caller
/// should keep the original selections unchanged.
pub fn sift_matches_within(
    text: &BufferText,
    sels: &SelectionSet,
    regex: &Regex,
) -> Option<SelectionSet> {
    let primary_idx = sels.primary_index();
    let mut new_sels: Vec<Selection> = Vec::new();
    let mut new_primary = 0;

    for (i, sel) in sels.iter_sorted().enumerate() {
        let piece_start = new_sels.len();
        let matches = find_matches_in_range(
            text,
            regex,
            InclusiveRange::new(sel.start(), sel.end_inclusive(text)),
        );

        for span in matches {
            new_sels.push(Selection::from_span(span, true, text));
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
    new_set.debug_assert_valid(text);
    Some(new_set)
}

// ── Trim whitespace ───────────────────────────────────────────────────────────

/// Trim leading and trailing whitespace from every selection's range.
///
/// "Whitespace" here means space (` `), tab (`\t`), and newline (`\n`). The
/// range shrinks inward until both ends sit on non-whitespace characters. If
/// the entire selection is whitespace the selection collapses to a cursor at
/// the original `head`.
pub fn cmd_trim_selection_whitespace(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    _mode: MotionMode,
) -> SelectionSet {
    let new_sels = sels.map(|sel| {
        let forward = sel.anchor() <= sel.head();

        match trim_blank(text, sel.span(text)) {
            Some(range) => Selection::from_span(range, forward, text),
            None => Selection::collapsed(sel.head()),
        }
    });
    new_sels.debug_assert_valid(text);
    new_sels
}

#[cfg(test)]
mod tests;
