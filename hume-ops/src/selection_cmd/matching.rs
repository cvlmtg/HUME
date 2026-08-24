use regex_cursor::engines::meta::Regex;

use crate::MotionMode;
use crate::search::find_matches_in_range;
use hume_editing::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use hume_editing::lines::line_content_end;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;
use hume_editing::word::{CharClass, classify_char};

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
    buf: &BufferText,
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
    // Split selections cover disjoint line ranges and can't overlap. `from_vec`
    // sorts and merges, but the input is already sorted and disjoint, so both
    // are no-ops here and the primary index is preserved.
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
pub fn select_matches_within(
    buf: &BufferText,
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
pub fn cmd_trim_selection_whitespace(
    buf: &BufferText,
    sels: SelectionSet,
    _count: usize,
    _mode: MotionMode,
) -> SelectionSet {
    let new_sels = sels.map(|sel| {
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
mod tests;
