use super::*;
use hume_test_fixtures::assert_state;
use hume_test_fixtures::testing::parse_state;
use pretty_assertions::assert_eq;

// ── cmd_collapse_selection_to_head ─────────────────────────────────────────────

#[test]
fn collapse_cursor_is_noop() {
    // A cursor (anchor == head) collapsing to itself — no change.
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_collapse_selection_to_head(&text, sels, 0, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn collapse_forward_selection() {
    assert_state!(
        "-[hell]>o\n",
        |(text, sels)| cmd_collapse_selection_to_head(&text, sels, 0, MotionMode::Move),
        // head was at 'l' (offset 3)
        "hel-[l]>o\n"
    );
}

#[test]
fn collapse_backward_selection() {
    // Backward: anchor=3, head=0, selects "hell" (4 chars). Collapses to cursor at head=0.
    assert_state!(
        "<[hell]-o\n",
        |(text, sels)| cmd_collapse_selection_to_head(&text, sels, 0, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn collapse_merges_coincident_heads() {
    // Two cursors at different positions stay separate after collapse —
    // they only merge if their heads land on the exact same position.
    let (text, sels) = parse_state("-[h]>el-[l]>o\n");
    let result = cmd_collapse_selection_to_head(&text, sels, 0, MotionMode::Move);
    assert_eq!(result.len(), 2); // still 2 — they don't converge
}

// ── cmd_flip_selections ────────────────────────────────────────────────

#[test]
fn flip_forward_becomes_backward() {
    // Forward: anchor=0, head=3, selects "hell". After flip: anchor=3, head=0.
    assert_state!(
        "-[hell]>o\n",
        |(text, sels)| cmd_flip_selections(&text, sels, 0, MotionMode::Move),
        "<[hell]-o\n"
    );
}

#[test]
fn flip_backward_becomes_forward() {
    // Backward: anchor=3, head=0, selects "hell". After flip: anchor=0, head=3.
    assert_state!(
        "<[hell]-o\n",
        |(text, sels)| cmd_flip_selections(&text, sels, 0, MotionMode::Move),
        "-[hell]>o\n"
    );
}

#[test]
fn flip_cursor_is_noop() {
    // anchor == head → flip does nothing observable.
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_flip_selections(&text, sels, 0, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn flip_is_involution() {
    // Flipping twice is the identity.
    assert_state!(
        "-[hell]>o\n",
        |(text, sels)| {
            let sels = cmd_flip_selections(&text, sels, 0, MotionMode::Move);
            cmd_flip_selections(&text, sels, 0, MotionMode::Move)
        },
        "-[hell]>o\n"
    );
}

// ── cmd_select_all ─────────────────────────────────────────────────────

#[test]
fn select_all_spans_entire_buffer() {
    // Cursor at 'e'; after select-all, anchor=0 head=last char ('\n').
    assert_state!(
        "h-[e]>llo\n",
        |(text, sels)| cmd_select_all(&text, sels, 0, MotionMode::Move),
        "-[hello\n]>"
    );
}

#[test]
fn select_all_multi_line() {
    assert_state!(
        "foo\nb-[a]>r\nbaz\n",
        |(text, sels)| cmd_select_all(&text, sels, 0, MotionMode::Move),
        "-[foo\nbar\nbaz\n]>"
    );
}

#[test]
fn select_all_empty_buffer() {
    // Minimal buffer is just '\n'. select-all produces a cursor at 0.
    assert_state!(
        "-[\n]>",
        |(text, sels)| cmd_select_all(&text, sels, 0, MotionMode::Move),
        "-[\n]>"
    );
}

// ── cmd_keep_primary_selection ─────────────────────────────────────────

#[test]
fn keep_primary_drops_all_others() {
    // Three cursors; primary (first yielded by DSL) is at 0. Others dropped.
    assert_state!(
        "-[h]>el-[l]>-[o]>\n",
        |(text, sels)| cmd_keep_primary_selection(&text, sels, 0, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn keep_primary_single_unchanged() {
    assert_state!(
        "-[hell]>o\n",
        |(text, sels)| cmd_keep_primary_selection(&text, sels, 0, MotionMode::Move),
        "-[hell]>o\n"
    );
}

// ── cmd_remove_primary_selection ───────────────────────────────────────

#[test]
fn remove_primary_single_is_noop() {
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_remove_primary_selection(&text, sels, 1, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn remove_primary_two_selections() {
    // Two cursors at 0 and 4. Primary is first (index 0).
    // After removal: only the cursor at 4 remains, becomes primary.
    assert_state!(
        "-[h]>ell-[o]>\n",
        |(text, sels)| cmd_remove_primary_selection(&text, sels, 1, MotionMode::Move),
        "hell-[o]>\n"
    );
}

// ── cmd_cycle_primary_forward ──────────────────────────────────────────

#[test]
fn cycle_forward_advances_primary() {
    // Three cursors. After cycling forward, primary should be the next one.
    let (text, sels) = parse_state("-[h]>el-[l]>o\n"); // two cursors, primary at 0
    assert_eq!(sels.primary().head(), 0);
    let sels = cmd_cycle_primary_forward(&text, sels, 0, MotionMode::Move);
    assert_eq!(sels.primary().head(), 3);
    // Cycle again — wraps back to first.
    let sels = cmd_cycle_primary_forward(&text, sels, 0, MotionMode::Move);
    assert_eq!(sels.primary().head(), 0);
}

// ── cmd_cycle_primary_backward ─────────────────────────────────────────

#[test]
fn cycle_backward_wraps_to_last() {
    let (text, sels) = parse_state("-[h]>el-[l]>o\n"); // primary at 0
    let sels = cmd_cycle_primary_backward(&text, sels, 0, MotionMode::Move);
    assert_eq!(sels.primary().head(), 3); // wraps to last
}

// ── cmd_collapse_selection_to_anchor ──────────────────────────────────

#[test]
fn collapse_to_anchor_cursor_is_noop() {
    // anchor == head → collapsing to anchor is the same as collapsing to head.
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_collapse_selection_to_anchor(&text, sels, 0, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn collapse_to_anchor_forward_selection() {
    // Forward: anchor=0 (h), head=3 (l). Collapse to anchor → cursor at h.
    assert_state!(
        "-[hell]>o\n",
        |(text, sels)| cmd_collapse_selection_to_anchor(&text, sels, 0, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn collapse_to_anchor_backward_selection() {
    // Backward: anchor=3 (l), head=0 (h). Collapse to anchor → cursor at l.
    assert_state!(
        "<[hell]-o\n",
        |(text, sels)| cmd_collapse_selection_to_anchor(&text, sels, 0, MotionMode::Move),
        "hel-[l]>o\n"
    );
}

#[test]
fn collapse_to_anchor_merges_coincident_anchors() {
    // Two selections with different heads but the same anchor collapse to
    // the same cursor and must be merged.
    let text = hume_editing::text::BufferText::from("hello\n");
    let sels = hume_editing::selection::SelectionSet::from_vec(
        vec![
            hume_editing::selection::Selection::new(0, 2), // anchor=0
            hume_editing::selection::Selection::new(0, 4), // anchor=0
        ],
        0,
    );
    let result = cmd_collapse_selection_to_anchor(&text, sels, 0, MotionMode::Move);
    assert_eq!(result.len(), 1); // merged — both collapsed to cursor at 0
    assert_eq!(result.primary().head(), 0);
}

// ── additional collapse edge cases ─────────────────────────────────────

#[test]
fn collapse_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| cmd_collapse_selection_to_head(&text, sels, 0, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn collapse_two_selections_same_head_merges() {
    // Two selections with different anchors but the same head collapse to
    // one cursor — map (which always merges) must reduce the count.
    let text = hume_editing::text::BufferText::from("hello\n");
    let sels = hume_editing::selection::SelectionSet::from_vec(
        vec![
            hume_editing::selection::Selection::new(0, 3), // head at 3
            hume_editing::selection::Selection::new(1, 3), // head at 3
        ],
        0,
    );
    let result = cmd_collapse_selection_to_head(&text, sels, 0, MotionMode::Move);
    assert_eq!(result.len(), 1); // merged — both collapsed to cursor at 3
    assert_eq!(result.primary().head(), 3);
}

// ── additional flip edge cases ─────────────────────────────────────────

#[test]
fn flip_multiple_selections() {
    // Two forward selections both flip to backward.
    assert_state!(
        "-[hell]>o -[worl]>d\n",
        |(text, sels)| cmd_flip_selections(&text, sels, 0, MotionMode::Move),
        "<[hell]-o <[worl]-d\n"
    );
}

// ── additional keep_primary edge cases ─────────────────────────────────

#[test]
fn keep_primary_when_primary_is_not_first() {
    // Cycle primary to the second cursor, then keep — should keep that one.
    let (text, sels) = parse_state("-[h]>el-[l]>o\n"); // primary at index 0 (head=0)
    let sels = cmd_cycle_primary_forward(&text, sels, 0, MotionMode::Move); // primary now at index 1 (head=3)
    let sels_out = cmd_keep_primary_selection(&text, sels, 0, MotionMode::Move);
    assert_eq!(sels_out.len(), 1);
    assert_eq!(sels_out.primary().head(), 3); // kept the second one
}

// ── additional remove_primary edge cases ───────────────────────────────

#[test]
fn remove_primary_at_end_wraps_to_first() {
    // Three cursors at 0, 3, 6. Cycle to last, then remove — should wrap
    // to the first remaining cursor (index 0 of the new set).
    let (text, sels) = parse_state("-[h]>el-[l]>o-[\n]>"); // 3 cursors, primary at 0
    let sels = cmd_cycle_primary_backward(&text, sels, 0, MotionMode::Move); // primary at last (head=6)
    let sels_out = cmd_remove_primary_selection(&text, sels, 1, MotionMode::Move);
    assert_eq!(sels_out.len(), 2);
    assert_eq!(sels_out.primary().head(), 0); // wrapped to first
}
