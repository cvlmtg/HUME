use hume_rope::column::CharCol;
use hume_rope::line::ContentLine;
use hume_rope::offset::CharOffset;
use pretty_assertions::assert_eq;

use super::*;
use crate::changeset::{ChangeSetBuilder, changesets_from_line_diff};
use crate::text::BufferText;

const TEXT: &str = "one\ntwo\nthree\n";

fn cs_of(len: usize, edits: &[(usize, usize, &str)]) -> ChangeSet {
    let mut b = ChangeSetBuilder::new(CharOffset::new(len));
    for &(start, end, inserted) in edits {
        b.retain_to(CharOffset::new(start));
        b.delete_to(CharOffset::new(end));
        b.insert(inserted);
    }
    b.retain_rest();
    b.finish()
}

fn lines_of(text: &str) -> Vec<String> {
    text.strip_suffix('\n')
        .unwrap_or(text)
        .split('\n')
        .map(str::to_owned)
        .collect()
}

fn strs(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|l| (*l).to_owned()).collect()
}

fn hunk(old_start: usize, new_start: usize, old: &[&str], new: &[&str]) -> ChangeHunk {
    ChangeHunk {
        old_start: ContentLine::new(old_start),
        new_start: ContentLine::new(new_start),
        old_lines: strs(old),
        new_lines: strs(new),
        words: WordSpans {
            old: Vec::new(),
            new: Vec::new(),
        },
    }
}

fn span(line: usize, start: usize, end: usize) -> LineSpan {
    LineSpan {
        line,
        start: CharCol::new(start),
        end: CharCol::new(end),
    }
}

fn with_words(mut h: ChangeHunk, old: Vec<LineSpan>, new: Vec<LineSpan>) -> ChangeHunk {
    h.words = WordSpans { old, new };
    h
}

/// Replays `hunks` over `text`'s lines: each hunk's new-side lines give way
/// to its old-side lines, and every old-side start must land where the
/// output has got to.
fn replay(text: &str, hunks: &[ChangeHunk]) -> Vec<String> {
    let lines = lines_of(text);
    let mut out = Vec::new();
    let mut next = 0;
    for h in hunks {
        let at = h.new_start.index();
        out.extend_from_slice(&lines[next..at]);
        assert_eq!(out.len(), h.old_start.index(), "old_start of {h:?}");
        out.extend(h.old_lines.iter().cloned());
        next = at + h.new_lines.len();
    }
    out.extend_from_slice(&lines[next..]);
    out
}

fn check(text: &str, edits: &[(usize, usize, &str)], expected: &[ChangeHunk]) {
    let buffer = BufferText::from(text);
    let cs = cs_of(buffer.len_chars(), edits);
    let hunks = change_hunks(&cs, &buffer);
    assert_eq!(hunks, expected);
    let after = cs.apply(&buffer).expect("changeset fits its text");
    assert_eq!(replay(text, &hunks), lines_of(&after.to_string()));
}

#[test]
fn word_change_inside_a_line() {
    check(
        TEXT,
        &[(5, 6, "W")],
        &[with_words(
            hunk(1, 1, &["tWo"], &["two"]),
            vec![span(0, 1, 2)],
            vec![span(0, 1, 2)],
        )],
    );
}

#[test]
fn whole_line_replacement_has_no_word_spans() {
    check(TEXT, &[(4, 7, "xyz")], &[hunk(1, 1, &["xyz"], &["two"])]);
}

#[test]
fn inserted_line_in_the_middle() {
    check(TEXT, &[(4, 4, "new\n")], &[hunk(1, 1, &["new"], &[])]);
}

#[test]
fn deleted_line_in_the_middle() {
    check(TEXT, &[(4, 8, "")], &[hunk(1, 1, &[], &["two"])]);
}

#[test]
fn inserted_line_at_the_start() {
    check(TEXT, &[(0, 0, "zero\n")], &[hunk(0, 0, &["zero"], &[])]);
}

#[test]
fn deleted_line_at_the_start() {
    check(TEXT, &[(0, 4, "")], &[hunk(0, 0, &[], &["one"])]);
}

#[test]
fn inserted_line_at_the_end() {
    check(TEXT, &[(14, 14, "four\n")], &[hunk(3, 3, &["four"], &[])]);
}

#[test]
fn deleted_line_at_the_end() {
    check(TEXT, &[(8, 14, "")], &[hunk(2, 2, &[], &["three"])]);
}

#[test]
fn inserted_line_break_splits_a_line() {
    check(TEXT, &[(6, 6, "\n")], &[hunk(1, 1, &["tw", "o"], &["two"])]);
}

#[test]
fn deleted_line_break_joins_two_lines() {
    check(
        TEXT,
        &[(7, 8, "")],
        &[hunk(1, 1, &["twothree"], &["two", "three"])],
    );
}

#[test]
fn regions_on_adjacent_lines_form_one_hunk() {
    check(
        TEXT,
        &[(0, 1, "O"), (4, 5, "T")],
        &[with_words(
            hunk(0, 0, &["One", "Two"], &["one", "two"]),
            vec![span(0, 0, 1), span(1, 0, 1)],
            vec![span(0, 0, 1), span(1, 0, 1)],
        )],
    );
}

#[test]
fn distant_regions_are_separate_hunks_and_the_second_old_start_follows_the_first() {
    check(
        TEXT,
        &[(0, 4, ""), (12, 13, "E")],
        &[
            hunk(0, 0, &[], &["one"]),
            with_words(
                hunk(1, 2, &["threE"], &["three"]),
                vec![span(0, 4, 5)],
                vec![span(0, 4, 5)],
            ),
        ],
    );
}

#[test]
fn an_edit_that_restores_its_own_text_leaves_no_hunk() {
    check(TEXT, &[(5, 6, "w")], &[]);
}

#[test]
fn identity_changeset_has_no_hunks() {
    check(TEXT, &[], &[]);
}

#[test]
fn spans_are_char_columns_across_a_zwj_cluster() {
    let family = "x\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}y\n";
    check(
        family,
        &[(6, 7, "Y")],
        &[with_words(
            hunk(
                0,
                0,
                &["x\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}Y"],
                &["x\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}y"],
            ),
            vec![span(0, 6, 7)],
            vec![span(0, 6, 7)],
        )],
    );
}

#[test]
fn a_whole_line_diff_revision_has_no_word_spans() {
    let old = BufferText::from(TEXT);
    let new = BufferText::from("one\nxyz\nthree\n");
    let (forward, _) = changesets_from_line_diff(&old, &new);
    let hunks = change_hunks(&forward, &old);
    assert_eq!(hunks, vec![hunk(1, 1, &["xyz"], &["two"])]);
}

#[test]
fn text_hunk_words_marks_the_edit_not_the_unpaired_line() {
    let words = text_hunk_words(
        &strs(&["## core:plum", "", "**PLUM** a commands b"]),
        &strs(&["**PLUM** a b"]),
    );
    assert_eq!(words.old, vec![span(2, 10, 19)]);
    assert_eq!(words.new, Vec::new());
}

#[test]
fn text_hunk_words_marks_both_sides_of_a_changed_word() {
    let words = text_hunk_words(&strs(&["let x = 1;"]), &strs(&["let y = 1;"]));
    assert_eq!(words.old, vec![span(0, 4, 5)]);
    assert_eq!(words.new, vec![span(0, 4, 5)]);
}

#[test]
fn text_hunk_words_is_empty_for_a_pure_insertion_or_deletion() {
    let empty = WordSpans {
        old: Vec::new(),
        new: Vec::new(),
    };
    assert_eq!(text_hunk_words(&[], &strs(&["a b"])), empty);
    assert_eq!(text_hunk_words(&strs(&["a b"]), &[]), empty);
}

#[test]
fn text_hunk_words_splits_a_span_across_lines() {
    let words = text_hunk_words(&strs(&["a one", "two b"]), &strs(&["a|b"]));
    assert_eq!(words.old, vec![span(0, 1, 5), span(1, 0, 4)]);
    assert_eq!(words.new, vec![span(0, 1, 2)]);
}
