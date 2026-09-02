use super::super::*;
use hume_test_fixtures::assert_state;

// Stand-ins for `hume_treesitter::textobjects::ObjectSpans` — this crate
// cannot depend on that crate, so these tests exercise `apply_object_motion`
// against a fixed list of spans rather than a real query result. Buffer
// text throughout is "abcdefghijklmnopqrstuvwxyz\n" (positions 0..=25 are
// 'a'..='z'). OBJ_INNER is nested inside OBJ_OUTER, proving the "search
// from the far edge" skip needs no special-casing.
const OBJ1: (usize, usize) = (2, 5); // "cdef"
const OBJ_OUTER: (usize, usize) = (10, 14); // "klmno"
const OBJ_INNER: (usize, usize) = (11, 12); // "lm", nested in OBJ_OUTER
const OBJ3: (usize, usize) = (20, 23); // "uvwx"
const SPANS: [(usize, usize); 4] = [OBJ1, OBJ_OUTER, OBJ_INNER, OBJ3];

/// Mirrors `ObjectSpans::adjacent(pos, Forward)`: smallest `start > pos`,
/// ties -> largest `end`.
fn find_forward(pos: usize) -> Option<(usize, usize)> {
    SPANS
        .iter()
        .copied()
        .filter(|&(start, _)| start > pos)
        .min_by_key(|&(start, end)| (start, std::cmp::Reverse(end)))
}

/// Mirrors `ObjectSpans::adjacent(pos, Backward)`: largest `start < pos`,
/// ties -> largest `end`.
fn find_backward(pos: usize) -> Option<(usize, usize)> {
    SPANS
        .iter()
        .copied()
        .filter(|&(start, _)| start < pos)
        .max_by_key(|&(start, end)| (start, end))
}

fn cmd_goto(
    text: &BufferText,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
    backward: bool,
) -> SelectionSet {
    let finder = if backward {
        find_backward
    } else {
        find_forward
    };
    apply_object_motion(text, sels, mode, count, backward, |_, pos| finder(pos))
}

#[test]
fn move_forward_from_cursor() {
    assert_state!(
        "-[a]>bcdefghijklmnopqrstuvwxyz\n",
        |(text, sels)| cmd_goto(&text, sels, 1, MotionMode::Move, false),
        "ab<[cdef]-ghijklmnopqrstuvwxyz\n"
    );
}

#[test]
fn move_forward_skips_object_nested_in_the_one_just_selected() {
    // Origin is the selected object's own end, so a sibling nested inside it
    // (OBJ_INNER, start 11) never qualifies as `start > origin`.
    assert_state!(
        "abcdefghij<[klmno]-pqrstuvwxyz\n",
        |(text, sels)| cmd_goto(&text, sels, 1, MotionMode::Move, false),
        "abcdefghijklmnopqrst<[uvwx]-yz\n"
    );
}

#[test]
fn move_backward_from_inside_an_object_lands_on_its_own_start() {
    // Origin is `current.start()`; a collapsed cursor's start is itself, and
    // OBJ1's own start (2) is `< 3` — so backward search from inside an
    // object finds that object before walking further back.
    assert_state!(
        "abc-[d]>efghijklmnopqrstuvwxyz\n",
        |(text, sels)| cmd_goto(&text, sels, 1, MotionMode::Move, true),
        "ab<[cdef]-ghijklmnopqrstuvwxyz\n"
    );
}

#[test]
fn move_forward_count_two() {
    assert_state!(
        "-[a]>bcdefghijklmnopqrstuvwxyz\n",
        |(text, sels)| cmd_goto(&text, sels, 2, MotionMode::Move, false),
        "abcdefghij<[klmno]-pqrstuvwxyz\n"
    );
}

#[test]
fn move_forward_no_next_object_is_unchanged() {
    assert_state!(
        "abcdefghijklmnopqrstuvwxy-[z]>\n",
        |(text, sels)| cmd_goto(&text, sels, 1, MotionMode::Move, false),
        "abcdefghijklmnopqrstuvwxy-[z]>\n"
    );
}

#[test]
fn extend_forward_keeps_anchor() {
    assert_state!(
        "-[a]>bcdefghijklmnopqrstuvwxyz\n",
        |(text, sels)| cmd_goto(&text, sels, 1, MotionMode::Extend, false),
        "-[abcdef]>ghijklmnopqrstuvwxyz\n"
    );
}

#[test]
fn extend_backward_keeps_anchor() {
    assert_state!(
        "abcdefghijklmnopqrstuvwxy-[z]>\n",
        |(text, sels)| cmd_goto(&text, sels, 1, MotionMode::Extend, true),
        "abcdefghijklmnopqrst<[uvwxyz]-\n"
    );
}

#[test]
fn multi_cursor_convergence_merges() {
    // Both cursors' forward search lands on OBJ1 — `map`'s always-merge
    // collapses the two resulting selections into one.
    assert_state!(
        "-[a]>-[b]>cdefghijklmnopqrstuvwxyz\n",
        |(text, sels)| cmd_goto(&text, sels, 1, MotionMode::Move, false),
        "ab<[cdef]-ghijklmnopqrstuvwxyz\n"
    );
}
