use super::*;
use pretty_assertions::assert_eq;

// ── Visual-line movement ──────────────────────────────────────────────────────
//
// `visual_test_editor` pins settings to `WrapMode::Indent { width: 76 }` with
// tab_width=4 and an 80×24 viewport. For a line with no leading indent, Indent
// wrap is equivalent to Soft wrap (indent_cols = 0), so the wrap boundary is
// simply at column 76.
//
// Test layout:
//   Line 0: 'a' × 80  →  sub-row 0: chars  0..76 (cols 0..75)
//                         sub-row 1: chars 76..80 (cols 0..3) + '\n' at col 4
//   Line 1: "short\n"  →  chars 81..86
//
// Char offsets:
//   0      = first 'a'
//   76     = first 'a' on sub-row 1
//   80     = '\n' at end of line 0
//   81     = 's' (start of "short")
//   85     = 't'
//   86     = '\n' at end of line 1

fn visual_test_editor(head: usize) -> Editor {
    let line0: String = "a".repeat(80);
    let content = format!("{}\nshort\n", line0);
    // Build manually so we can place the cursor at an exact char offset.
    use crate::core::selection::{Selection, SelectionSet};
    use crate::core::text::Text;
    let buf = Text::from(content.as_str());
    let sels = SelectionSet::single(Selection::collapsed(head));
    let mut ed = Editor::for_testing(Buffer::new(buf, sels));
    // Pin to 76-column indent-wrap so the char-offset expectations in the tests
    // are stable regardless of terminal size.
    ed.settings.wrap_mode = engine::pane::WrapMode::Indent { width: 76 };
    ed
}

/// j moves from sub-row 0 to sub-row 1 of the same buffer line.
#[test]
fn visual_move_down_within_wrapped_line() {
    let mut ed = visual_test_editor(0);
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head,
        76,
        "j: sub-row 0 → sub-row 1, col 0 → char 76"
    );
    assert_eq!(
        ed.current_selections().primary().horiz,
        Some(0),
        "sticky col latched on first j"
    );
}

/// j on the last sub-row crosses to the next buffer line.
#[test]
fn visual_move_down_crosses_buffer_line() {
    let mut ed = visual_test_editor(76); // sub-row 1 of line 0
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head,
        81,
        "j: last sub-row → first char of next buffer line"
    );
}

/// k from the first row of a buffer line enters the last sub-row of the previous line.
#[test]
fn visual_move_up_enters_last_subrow_of_previous_line() {
    let mut ed = visual_test_editor(81); // start of "short"
    ed.handle_key(key('k'));
    assert_eq!(
        ed.current_selections().primary().head,
        76,
        "k: buffer line n+1 → last sub-row of line n, col 0 → char 76"
    );
}

/// k on sub-row 1 retreats to sub-row 0 of the same buffer line.
#[test]
fn visual_move_up_within_wrapped_line() {
    let mut ed = visual_test_editor(76); // sub-row 1 of line 0
    ed.handle_key(key('k'));
    assert_eq!(
        ed.current_selections().primary().head,
        0,
        "k: sub-row 1 → sub-row 0, col 0 → char 0"
    );
}

/// k on the first sub-row of the first line stays put.
#[test]
fn visual_move_up_at_top_stays_put() {
    let mut ed = visual_test_editor(0);
    ed.handle_key(key('k'));
    assert_eq!(
        ed.current_selections().primary().head,
        0,
        "k at first row: no-op"
    );
}

/// j on the last sub-row of the last line stays put.
#[test]
fn visual_move_down_at_bottom_stays_put() {
    // Place cursor at "short" (line 1 is last). Line 1 has only 1 sub-row.
    let mut ed = visual_test_editor(81);
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head,
        81,
        "j at last row: no-op"
    );
}

/// The preferred display column is preserved across consecutive j/k presses
/// and used to find the closest grapheme when the target row is shorter.
#[test]
fn visual_preferred_col_stickiness() {
    // Cursor at char 40 (display col 40) in sub-row 0 of the long line.
    let mut ed = visual_test_editor(40);

    // j: target_col = 40, sub-row 1 has only 4 chars (cols 0..3).
    // Closest to col 40 is char 79 (col 3, last 'a' on sub-row 1).
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head,
        79,
        "j: clamped to last char on short sub-row"
    );
    assert_eq!(
        ed.current_selections().primary().horiz,
        Some(40),
        "sticky col stays at 40"
    );

    // j again: cross to "short\n" (line 1). target_col=40, "short" has cols 0..4.
    // Closest to 40 is 't' at col 4, char 85.
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head,
        85,
        "j: clamped to last char on short second line"
    );
    assert_eq!(
        ed.current_selections().primary().horiz,
        Some(40),
        "sticky col still 40"
    );
}

/// Any non-vertical command resets preferred_display_col.
#[test]
fn visual_preferred_col_reset_on_horizontal_motion() {
    let mut ed = visual_test_editor(40);
    ed.handle_key(key('j')); // latches horiz on the selection
    assert!(
        ed.current_selections().primary().horiz.is_some(),
        "j latches sticky col"
    );
    ed.handle_key(key('l')); // horizontal motion — Selection::new() clears horiz
    assert!(
        ed.current_selections().primary().horiz.is_none(),
        "l resets sticky col"
    );
}

/// WrapMode::None falls back to buffer-line movement.
#[test]
fn visual_move_no_wrap_falls_back_to_buffer_line() {
    let mut ed = visual_test_editor(0);
    // Override via buffer: apply_visual_vertical reads overrides at call time.
    ed.doc_mut().overrides.wrap_mode = Some(engine::pane::WrapMode::None);

    ed.handle_key(key('j'));
    // With no wrapping: j moves by one buffer line (0 → 81 "short").
    assert_eq!(
        ed.current_selections().primary().head,
        81,
        "WrapMode::None: j moves by buffer line"
    );
    assert!(
        ed.current_selections().primary().horiz.is_none(),
        "no sticky col in non-wrap mode"
    );
}

/// count prefix: 2j moves two visual rows.
#[test]
fn visual_move_down_with_count() {
    let mut ed = visual_test_editor(0);
    ed.handle_key(key('2'));
    ed.handle_key(key('j'));
    // 2j from char 0: first j → char 76 (sub-row 1), second j → char 81 (next line).
    assert_eq!(
        ed.current_selections().primary().head,
        81,
        "2j: two visual rows from sub-row 0"
    );
}

/// Each cursor uses its own sticky column in multi-cursor j/k.
///
/// Text layout (visual_test_editor):
///   sub-row 0: chars  0..76 (cols 0..75)
///   sub-row 1: chars 76..80 (cols 0..3)  ← two cursors placed here
///   line 1:    chars 81..86 "short\n"
///
/// Cursor A at char 76 (col 0), cursor B at char 79 (col 3, primary).
/// j → line 1: A should land at col 0 = char 81, B at col 3 = char 84.
/// k → sub-row 1: A should return to col 0 = char 76, B to col 3 = char 79.
#[test]
fn visual_move_per_selection_sticky_col() {
    use crate::core::selection::{Selection, SelectionSet};

    let line0: String = "a".repeat(80);
    let content = format!("{}\nshort\n", line0);
    let buf = crate::core::text::Text::from(content.as_str());
    // A at col 0, B at col 3 (primary).
    let sels = SelectionSet::from_vec(
        vec![
            Selection::collapsed(76), // A — col 0 on sub-row 1
            Selection::collapsed(79), // B — col 3 on sub-row 1
        ],
        1, // primary is B
    );
    let mut ed = Editor::for_testing(Buffer::new(buf, sels));
    ed.settings.wrap_mode = engine::pane::WrapMode::Indent { width: 76 };

    // j: each cursor should use its own column, not the primary's.
    ed.handle_key(key('j'));
    let sels = ed.current_selections().clone();
    assert_eq!(sels.len(), 2, "two cursors remain distinct");
    // Sorted by start(): A is first.
    let heads: Vec<usize> = sels.iter_sorted().map(|s| s.head).collect();
    assert_eq!(heads[0], 81, "A (col 0) → char 81 on line 1");
    assert_eq!(heads[1], 84, "B (col 3) → char 84 on line 1");

    // k: sticky cols should bring each cursor back to its original column.
    ed.handle_key(key('k'));
    let sels = ed.current_selections().clone();
    assert_eq!(sels.len(), 2, "two cursors remain distinct");
    let heads: Vec<usize> = sels.iter_sorted().map(|s| s.head).collect();
    assert_eq!(heads[0], 76, "A returns to col 0 = char 76 on sub-row 1");
    assert_eq!(heads[1], 79, "B returns to col 3 = char 79 on sub-row 1");
}

// ── Visual-line extend variants ───────────────────────────────────────────────
//
// Extend mode is toggled with `e`. In extend mode `j`/`k` resolve to
// extend-down/extend-up: the anchor stays fixed and only the head moves.

/// extend-down (e+j) within a wrapped line: anchor stays at sub-row 0, head
/// advances to sub-row 1 of the same buffer line.
#[test]
fn visual_extend_down_within_wrapped_line() {
    let mut ed = visual_test_editor(0);
    ed.handle_key(key('e')); // enter extend mode
    ed.handle_key(key('j'));
    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor, 0, "anchor fixed at sub-row 0 col 0");
    assert_eq!(sel.head, 76, "head extends to sub-row 1 col 0");
}

/// extend-down crosses to the next buffer line when already on the last sub-row.
#[test]
fn visual_extend_down_crosses_buffer_line() {
    let mut ed = visual_test_editor(76); // last sub-row of line 0
    ed.handle_key(key('e'));
    ed.handle_key(key('j'));
    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor, 76, "anchor fixed at last sub-row");
    assert_eq!(
        sel.head, 81,
        "head crosses to first char of next buffer line"
    );
}

/// extend-up (e+k) within a wrapped line: head retreats from sub-row 1 to sub-row 0.
#[test]
fn visual_extend_up_within_wrapped_line() {
    let mut ed = visual_test_editor(76); // sub-row 1 of line 0
    ed.handle_key(key('e'));
    ed.handle_key(key('k'));
    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor, 76, "anchor fixed at sub-row 1");
    assert_eq!(sel.head, 0, "head retreats to sub-row 0 col 0");
}

/// extend-up enters the last sub-row of the previous buffer line.
#[test]
fn visual_extend_up_enters_previous_line_last_subrow() {
    let mut ed = visual_test_editor(81); // start of "short"
    ed.handle_key(key('e'));
    ed.handle_key(key('k'));
    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor, 81, "anchor fixed at line 1 start");
    assert_eq!(
        sel.head, 76,
        "head enters last sub-row of previous buffer line"
    );
}
