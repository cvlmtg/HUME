use hume_rope::offset::CharOffset;
use pretty_assertions::assert_eq;

use super::*;
use crate::changeset::ChangeSetBuilder;
use crate::edit::TextChange;

/// A hunk's line ranges and lines, words left out: `change_hunks` owns those.
type Lines = (usize, usize, usize, usize, Vec<String>, Vec<String>);

fn lines(hunks: Vec<ChangeHunk>) -> Vec<Lines> {
    hunks
        .into_iter()
        .map(|h| {
            let (old, new) = (h.old_start.index(), h.new_start.index());
            (
                old,
                old + h.old_lines.len(),
                new,
                new + h.new_lines.len(),
                h.old_lines,
                h.new_lines,
            )
        })
        .collect()
}

fn hunk(old: (usize, usize), new: (usize, usize), old_lines: &[&str], new_lines: &[&str]) -> Lines {
    let owned = |ls: &[&str]| ls.iter().map(|l| (*l).to_owned()).collect();
    (
        old.0,
        old.1,
        new.0,
        new.1,
        owned(old_lines),
        owned(new_lines),
    )
}

/// Applies `edits` (`(start, end, inserted)`, char offsets in increasing
/// order) to `text`, records the step in `log`, and returns the new text.
fn step(log: &mut ChangeLog, text: &BufferText, edits: &[(usize, usize, &str)]) -> BufferText {
    let mut b = ChangeSetBuilder::new(CharOffset::new(text.len_chars()));
    for &(start, end, inserted) in edits {
        b.retain_to(CharOffset::new(start));
        b.delete_to(CharOffset::new(end));
        b.insert(inserted);
    }
    b.retain_rest();
    let cs = b.finish();
    let after = cs.apply(text).expect("the edit fits its text");
    log.record(&TextChange::new(text, &after, &cs));
    after
}

#[test]
fn an_insert_inside_a_line_is_one_hunk_on_that_line() {
    let mut log = ChangeLog::default();
    let text = BufferText::from("one\ntwo\nthree\n");
    let live = step(&mut log, &text, &[(5, 5, "X")]);
    assert_eq!(
        log.since(text.generation(), &live).map(lines),
        Some(vec![hunk((1, 2), (1, 2), &["two"], &["tXwo"])])
    );
}

#[test]
fn two_cursors_far_apart_give_two_hunks() {
    let mut log = ChangeLog::default();
    let text = BufferText::from("a\nb\nc\nd\n");
    let live = step(&mut log, &text, &[(0, 0, "X"), (6, 6, "Y")]);
    assert_eq!(
        log.since(text.generation(), &live).map(lines),
        Some(vec![
            hunk((0, 1), (0, 1), &["a"], &["Xa"]),
            hunk((3, 4), (3, 4), &["d"], &["Yd"]),
        ])
    );
}

#[test]
fn deleting_whole_lines_leaves_an_empty_new_range_at_the_join() {
    let mut log = ChangeLog::default();
    let text = BufferText::from("a\nb\nc\nd\n");
    let live = step(&mut log, &text, &[(2, 6, "")]);
    assert_eq!(
        log.since(text.generation(), &live).map(lines),
        Some(vec![hunk((1, 3), (1, 1), &["b", "c"], &[])])
    );
}

#[test]
fn steps_since_a_mark_combine_into_hunks_against_the_live_text() {
    let mut log = ChangeLog::default();
    let text = BufferText::from("a\nb\nc\n");
    let mid = step(&mut log, &text, &[(0, 0, "new\n")]);
    let live = step(&mut log, &mid, &[(8, 9, "C")]);
    assert_eq!(
        log.since(text.generation(), &live).map(lines),
        Some(vec![
            hunk((0, 0), (0, 1), &[], &["new"]),
            hunk((2, 3), (3, 4), &["c"], &["C"]),
        ])
    );
    assert_eq!(
        log.since(mid.generation(), &live).map(lines),
        Some(vec![hunk((3, 4), (3, 4), &["c"], &["C"])])
    );
}

#[test]
fn steps_that_cancel_out_give_no_hunks() {
    let mut log = ChangeLog::default();
    let text = BufferText::from("a\nb\n");
    let typed = step(&mut log, &text, &[(2, 2, "x")]);
    let live = step(&mut log, &typed, &[(2, 3, "")]);
    assert_eq!(log.since(text.generation(), &live), Some(vec![]));
}

#[test]
fn a_mark_at_the_live_text_gives_no_hunks() {
    let mut log = ChangeLog::default();
    let text = BufferText::from("a\n");
    let live = step(&mut log, &text, &[(0, 0, "x")]);
    assert_eq!(log.since(live.generation(), &live), Some(vec![]));
}

#[test]
fn a_replacement_since_the_mark_cannot_be_described() {
    let mut log = ChangeLog::default();
    let text = BufferText::from("a\n");
    let edited = step(&mut log, &text, &[(0, 0, "x")]);
    let live = edited.replaced_with("whole new text\n");
    log.record_replace(&edited);
    assert_eq!(log.since(text.generation(), &live), None);
    assert_eq!(log.since(edited.generation(), &live), None);
    assert_eq!(log.since(live.generation(), &live), Some(vec![]));
}

#[test]
fn a_mark_past_the_entry_bound_cannot_be_described() {
    let mut log = ChangeLog::default();
    let text = BufferText::from("a\n");
    let mut live = step(&mut log, &text, &[(0, 0, "x")]);
    let kept = live.clone();
    for _ in 1..MAX_ENTRIES {
        live = step(&mut log, &live, &[(0, 0, "x")]);
    }
    assert_eq!(
        log.since(text.generation(), &live).map(|h| h.len()),
        Some(1)
    );
    live = step(&mut log, &live, &[(0, 0, "x")]);
    assert_eq!(log.since(text.generation(), &live), None);
    assert_eq!(
        log.since(kept.generation(), &live).map(|h| h.len()),
        Some(1)
    );
}

#[test]
fn a_mark_past_the_deleted_text_bound_cannot_be_described() {
    let mut log = ChangeLog::default();
    let text = BufferText::from("xxxxxxx\n".repeat(MAX_HELD_CHARS / 8 + 1).as_str());
    let half = step(&mut log, &text, &[(0, MAX_HELD_CHARS / 2, "")]);
    assert_eq!(
        log.since(text.generation(), &half).map(|h| h.len()),
        Some(1)
    );
    let live = step(&mut log, &half, &[(0, MAX_HELD_CHARS / 2 + 1, "")]);
    assert_eq!(log.since(text.generation(), &live), None);
    assert_eq!(
        log.since(half.generation(), &live).map(|h| h.len()),
        Some(1)
    );
}

#[test]
fn a_change_whose_lines_outgrow_the_report_bound_cannot_be_described() {
    let mut log = ChangeLog::default();
    let text = BufferText::from("a\n");
    let live = step(&mut log, &text, &[(0, 0, &"x".repeat(MAX_REPORTED_CHARS))]);
    assert_eq!(log.since(text.generation(), &live), None);
}

#[test]
fn a_generation_the_log_never_saw_cannot_be_described() {
    let mut log = ChangeLog::default();
    let text = BufferText::from("a\n");
    let live = step(&mut log, &text, &[(0, 0, "x")]);
    assert_eq!(log.since(live.generation() + 1, &live), None);
}
