use super::doubles::InlineHint;
use super::*;
use hume_editing::selection::StickyDisplayCol;
use hume_rope::column::{BufferLineCol, DisplayLineCol};
use pretty_assertions::assert_eq;

/// A `DisplayLine`-tagged latch — what `j`/`k` write while wrapping, which
/// is how every fixture in this file (except the `WrapMode::None` ones) is
/// pinned. `wrap_width: Some(76)` matches `visual_test_editor`'s fixed
/// `WrapMode::Indent { width: 76 }` — an explicit, non-sentinel width, so it
/// stays 76 regardless of the fixture's 80×24 viewport.
fn sticky_display_line(display_col: u32) -> StickyDisplayCol {
    StickyDisplayCol::DisplayLine {
        display_col: DisplayLineCol::new(display_col),
        wrap_width: Some(76),
    }
}

/// A `BufferLine`-tagged latch — what `9j`/`9k` write, and what `j`/`k`
/// write too once wrapping is off (a display line IS the buffer line there
/// — see `StickyDisplayCol`'s own doc). Carries no `wrap_width` — the
/// `BufferLine` variant has none, unlike `DisplayLine`.
fn sticky_buffer_line(display_col: u32) -> StickyDisplayCol {
    StickyDisplayCol::BufferLine {
        display_col: BufferLineCol::new(display_col),
    }
}

// ── Visual-line movement ──────────────────────────────────────────────────────
//
// `visual_test_editor` pins settings to `WrapMode::Indent { width: 76 }` with
// tab_width=4 and an 80×24 viewport. For a line with no leading indent, Indent
// wrap is equivalent to Soft wrap (indent_display_cols = 0), so the wrap boundary is
// simply at column 76.
//
// Test layout:
//   Line 0: 'a' × 80  →  display line 0: chars  0..76 (cols 0..75)
//                         display line 1: chars 76..80 (cols 0..3) + '\n' at col 4
//   Line 1: "short\n"  →  chars 81..86
//
// Char offsets:
//   0      = first 'a'
//   76     = first 'a' on display line 1
//   80     = '\n' at end of line 0
//   81     = 's' (start of "short")
//   85     = 't'
//   86     = '\n' at end of line 1

fn visual_test_editor(head: usize) -> Editor {
    let line0: String = "a".repeat(80);
    let content = format!("{}\nshort\n", line0);
    // Build manually so we can place the cursor at an exact char offset.
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::BufferText;
    let text = BufferText::from(content.as_str());
    let sels = SelectionSet::single(Selection::collapsed(co(head)));
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    // Pin to 76-column indent-wrap so the char-offset expectations in the tests
    // are stable regardless of terminal size.
    ed.view.panes[ed.state.focus.id()].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Indent { width: 76 }),
        saved: None,
    });
    ed
}

/// j moves from display line 0 to display line 1 of the same buffer line.
#[test]
fn visual_move_down_within_wrapped_line() {
    let mut ed = visual_test_editor(0);
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(76),
        "j: display line 0 → display line 1, col 0 → char 76"
    );
    assert_eq!(
        ed.current_selections().primary().sticky_display_col(),
        Some(sticky_display_line(0)),
        "sticky col latched on first j"
    );
}

/// j on the last display line crosses to the next buffer line.
#[test]
fn visual_move_down_crosses_buffer_line() {
    let mut ed = visual_test_editor(76); // display line 1 of line 0
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(81),
        "j: last display line → first char of next buffer line"
    );
}

/// k from the first display line of a buffer line enters the last display line of the previous line.
#[test]
fn visual_move_up_enters_last_display_line_of_previous_line() {
    let mut ed = visual_test_editor(81); // start of "short"
    ed.handle_key(key('k'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(76),
        "k: buffer line n+1 → last display line of line n, col 0 → char 76"
    );
}

/// k on display line 1 retreats to display line 0 of the same buffer line.
#[test]
fn visual_move_up_within_wrapped_line() {
    let mut ed = visual_test_editor(76); // display line 1 of line 0
    ed.handle_key(key('k'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(0),
        "k: display line 1 → display line 0, col 0 → char 0"
    );
}

/// k on the first display line of the first line stays put.
#[test]
fn visual_move_up_at_top_stays_put() {
    let mut ed = visual_test_editor(0);
    ed.handle_key(key('k'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(0),
        "k at first display line: no-op"
    );
}

/// j on the last display line of the last line stays put.
#[test]
fn visual_move_down_at_bottom_stays_put() {
    // Place cursor at "short" (line 1 is last). Line 1 has only 1 display line.
    let mut ed = visual_test_editor(81);
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(81),
        "j at last display line: no-op"
    );
}

/// The preferred display column is preserved across consecutive j/k presses
/// and used to find the closest grapheme when the target display line is shorter.
#[test]
fn visual_preferred_display_col_stickiness() {
    // Cursor at char 40 (display col 40) in display line 0 of the long line.
    let mut ed = visual_test_editor(40);

    // j: target_display_col = 40, display line 1 has only 4 chars (cols 0..3).
    // Closest to col 40 is char 79 (col 3, last 'a' on display line 1).
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(79),
        "j: clamped to last char on short display line"
    );
    assert_eq!(
        ed.current_selections().primary().sticky_display_col(),
        Some(sticky_display_line(40)),
        "sticky col stays at 40"
    );

    // j again: cross to "short\n" (line 1). target_display_col=40, "short" has cols 0..4.
    // Closest to 40 is 't' at col 4, char 85.
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(85),
        "j: clamped to last char on short second line"
    );
    assert_eq!(
        ed.current_selections().primary().sticky_display_col(),
        Some(sticky_display_line(40)),
        "sticky col still 40"
    );
}

/// Any non-vertical command resets preferred_display_col.
#[test]
fn visual_preferred_display_col_reset_on_horizontal_motion() {
    let mut ed = visual_test_editor(40);
    ed.handle_key(key('j')); // latches sticky_display_col on the selection
    assert!(
        ed.current_selections()
            .primary()
            .sticky_display_col()
            .is_some(),
        "j latches sticky col"
    );
    ed.handle_key(key('l')); // horizontal motion — Selection::new() clears sticky_display_col
    assert!(
        ed.current_selections()
            .primary()
            .sticky_display_col()
            .is_none(),
        "l resets sticky col"
    );
}

/// WrapMode::None: a no-wrap content display line *is* a buffer line, so bare `j`
/// lands on the same char a buffer-line hop would (0 → 81 "short") — but it
/// still goes through the sticky *display*-column model (`move_vertical`),
/// matching page/half-page/wheel scroll in the same mode, so
/// `sticky_display_col` latches here too.
#[test]
fn visual_move_no_wrap_content_display_line_is_a_buffer_line() {
    let mut ed = visual_test_editor(0);
    // Pin off, overriding `visual_test_editor`'s indent-wrap pin.
    pin_no_wrap(&mut ed);

    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(81),
        "WrapMode::None: a content display line is a buffer line"
    );
    assert_eq!(
        ed.current_selections().primary().sticky_display_col(),
        Some(sticky_buffer_line(0)),
        "sticky display column latches even in no-wrap mode"
    );
}

/// count prefix: 2j moves two BUFFER lines (the second hop is a no-op here —
/// buffer line 2 is the phantom trailing empty line — so it lands on buffer
/// line 1, same char offset a bare `j`,`j` would reach by two visual display lines in
/// this particular buffer; see `visual_move_down_with_explicit_count_moves_buffer_lines`
/// for a case where the two paths diverge).
#[test]
fn visual_move_down_with_count() {
    let mut ed = visual_test_editor(0);
    ed.handle_key(key('2'));
    ed.handle_key(key('j'));
    // 2j from char 0: first hop → char 81 (buffer line 1); second hop is a
    // no-op (buffer line 2 is the phantom trailing line).
    assert_eq!(
        ed.current_selections().primary().head(),
        co(81),
        "2j: buffer-line movement, clamped at the last real line"
    );
}

/// A count prefix means "N buffer lines", not "N display lines" — even while
/// wrapping is on. `1j` skips straight to the start of buffer line 1, bypassing
/// the display-line-1 stop that a bare `j` (no count) lands on. The buffer-line
/// path latches a `BufferLine`-tagged sticky column (Q29b) — distinct from
/// the `DisplayLine` one bare `j` latches while wrapping, so a following `2j`
/// reuses it and a following bare `j` re-derives instead of reading it as a
/// display-line-relative column.
#[test]
fn visual_move_down_with_explicit_count_moves_buffer_lines() {
    let mut ed = visual_test_editor(0); // display line 0, col 0
    ed.handle_key(key('1'));
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(81),
        "1j: one buffer line skips the display-line-1 stop entirely"
    );
    assert_eq!(
        ed.current_selections().primary().sticky_display_col(),
        Some(sticky_buffer_line(0)),
        "buffer-line path latches a Line-variant sticky display column"
    );
}

/// A larger explicit count also moves by buffer lines: `2j` from line 0 lands
/// on the (only) next buffer line, not two display lines past it.
#[test]
fn visual_move_up_with_explicit_count_moves_buffer_lines() {
    let mut ed = visual_test_editor(81); // start of "short" (buffer line 1)
    ed.handle_key(key('1'));
    ed.handle_key(key('k'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(0),
        "1k: one buffer line lands on line 0 col 0, not the last display line (char 76)"
    );
}

// ── Explicit-count (BufferLine) vertical motion ───────────────────────────
//
// `9j`/`9k` (`VerticalMove::BufferLine`, `editor::visual_move::move_buffer_line`)
// resolve their column through `DisplayLineMap::buffer_line_col`/
// `char_at_buffer_line_col`, same as bare `j`/`k`'s `ContentRow`/`ScreenRow`
// units, rather than a rope-only column blind to the decoration layer.
// `WrapMode::None` throughout: these cases pin the buffer-line column model
// itself, not its interaction with wrapping (that's the "family switch"
// suite below).

#[test]
fn explicit_count_move_down_basic() {
    let mut ed = unwrapped_editor("hello\nworld\n", 0); // 'h'
    ed.handle_key(key('1'));
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(6),
        "lands on 'w'"
    );
}

#[test]
fn explicit_count_move_down_preserves_display_column() {
    let mut ed = unwrapped_editor("hello\nworld\n", 2); // 'l', col 2
    ed.handle_key(key('1'));
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(8),
        "col 2 of \"world\" is 'r'"
    );
}

#[test]
fn explicit_count_move_down_clamps_to_shorter_line() {
    let mut ed = unwrapped_editor("hello\nab\n", 2); // 'l', col 2
    ed.handle_key(key('1'));
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(7),
        "clamps to 'b'"
    );
}

#[test]
fn explicit_count_move_down_clamp_at_document_edge() {
    let mut ed = unwrapped_editor("hello\nworld\n", 6); // already on the last line
    ed.handle_key(key('1'));
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(6),
        "head stays exactly put, not re-clamped onto the same line"
    );
}

#[test]
fn explicit_count_move_up_clamp_at_document_edge() {
    let mut ed = unwrapped_editor("hello\nworld\n", 0); // already on the first line
    ed.handle_key(key('1'));
    ed.handle_key(key('k'));
    assert_eq!(ed.current_selections().primary().head(), co(0));
}

/// `j` at the document's last line must still collapse a non-empty selection
/// onto its head, exactly as every other `MotionMode::Move` motion does at
/// its own edge (`hume_ops::apply_motion` collapses unconditionally).
/// `apply_visual_vertical` only ever backs motions now — the no-collapse
/// guard a view scroll needs lives in `commands::scroll_view`'s own
/// per-selection `carry` pass instead, so there is no shared guard here that
/// could apply too broadly to plain `j`/`k`.
#[test]
fn move_down_at_document_edge_still_collapses_a_selection() {
    let mut ed = editor_from("hello\n-[world]>\n");
    pin_no_wrap(&mut ed);
    ed.handle_key(key('j'));
    assert_eq!(
        state(&ed),
        "hello\nworl-[d]>\n",
        "already the last line: anchor collapses onto head, head stays put"
    );
}

/// `k`'s mirror of the test above, at the document's first line.
#[test]
fn move_up_at_document_edge_still_collapses_a_selection() {
    let mut ed = editor_from("-[hello]>\nworld\n");
    pin_no_wrap(&mut ed);
    ed.handle_key(key('k'));
    assert_eq!(
        state(&ed),
        "hell-[o]>\nworld\n",
        "already the first line: anchor collapses onto head, head stays put"
    );
}

#[test]
fn explicit_count_move_down_to_empty_line() {
    let mut ed = unwrapped_editor("hello\n\nworld\n", 0); // 'h'
    ed.handle_key(key('1'));
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(6),
        "the empty line's only cell is its own '\\n'"
    );
}

#[test]
fn explicit_count_move_down_multi_cursor_merge() {
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::BufferText;

    // Two cursors on line 0 at different columns (2 and 4), both past line
    // 1's width (2) — both clamp to its last char and converge there.
    // `SelectionSet::map` must still merge them through the new
    // `move_buffer_line` path, same as it does for every other motion.
    let text = BufferText::from("hello\nab\n");
    let sels = SelectionSet::from_vec(
        vec![Selection::collapsed(co(2)), Selection::collapsed(co(4))],
        0,
    );
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    pin_no_wrap(&mut ed);
    ed.handle_key(key('1'));
    ed.handle_key(key('j'));
    let sels = ed.current_selections().clone();
    assert_eq!(sels.len(), 1, "both cursors clamp onto 'b' and merge");
    assert_eq!(sels.primary().head(), co(7));
}

#[test]
fn explicit_count_move_down_preserves_display_column_across_a_tab() {
    // "\tworld" (tab_width 4): 'o' (char 2) sits at display column 5 (tab
    // expands to 4, 'w' is 1 more). Landing must use display column 5 on the
    // target line — 'f' (char offset 5 of "abcdefgh") — not char-offset
    // column 2, which would be 'c'.
    let mut ed = unwrapped_editor("\tworld\nabcdefgh\n", 2); // 'o'
    ed.handle_key(key('1'));
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(12),
        "lands on 'f'"
    );
}

#[test]
fn explicit_count_move_down_preserves_display_column_across_a_wide_cjk_char() {
    // 漢 (East Asian Wide) is 2 display columns but 1 char, so 'b' (char 1)
    // sits at display column 2. Landing must use display column 2 — 'c' —
    // not char-offset column 1, which would be 'b'.
    let mut ed = unwrapped_editor("\u{6F22}bc\nabcdefgh\n", 1); // 'b'
    ed.handle_key(key('1'));
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(6),
        "lands on 'c'"
    );
}

#[test]
fn explicit_count_move_up_basic() {
    let mut ed = unwrapped_editor("hello\nworld\n", 6); // 'w'
    ed.handle_key(key('1'));
    ed.handle_key(key('k'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(0),
        "lands on 'h'"
    );
}

#[test]
fn explicit_count_move_up_preserves_display_column() {
    let mut ed = unwrapped_editor("hello\nworld\n", 9); // 'l' of "world", col 3
    ed.handle_key(key('1'));
    ed.handle_key(key('k'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(3),
        "col 3 of \"hello\" is 'l'"
    );
}

#[test]
fn explicit_count_move_up_clamps_to_shorter_line() {
    let mut ed = unwrapped_editor("ab\nhello\n", 6); // 'l' of "hello", col 3
    ed.handle_key(key('1'));
    ed.handle_key(key('k'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(1),
        "clamps to 'b'"
    );
}

// ── Sticky display column across a count (Q29b) ───────────────────────────
//
// A count-fold must hold its goal column across the whole hop instead of
// re-deriving it from each intermediate landing — otherwise a short line
// partway through the hop truncates it, same failure the sticky column
// exists to prevent for repeated bare `j`/`k`. `move_buffer_line` computes
// `target_line` directly rather than stepping through it, so this holds
// structurally — the test guards the shortcut from regressing to a per-line
// step loop.

#[test]
fn explicit_count_move_down_holds_display_column_through_a_short_line() {
    let mut ed = unwrapped_editor("abcdef\nx\nabcdef\n", 3); // 'd', col 3
    ed.handle_key(key('2'));
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(12),
        "lands on line 2's 'd' — landing on 'x' first would truncate the column to 0"
    );
}

#[test]
fn explicit_count_move_up_holds_display_column_through_a_short_line() {
    let mut ed = unwrapped_editor("abcdef\nx\nabcdef\n", 12); // 'd' of line 2, col 3
    ed.handle_key(key('2'));
    ed.handle_key(key('k'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(3),
        "lands on line 0's 'd'"
    );
}

#[test]
fn explicit_count_move_down_reuses_a_buffer_line_latch_but_rederives_a_display_line_one() {
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::BufferText;

    let text = BufferText::from("abcdefgh\nABCDEFGH\n");

    // A `BufferLine`-tagged latch is this call's own domain and seeds the
    // hop directly.
    let seeded = SelectionSet::single(Selection::with_sticky_display_col(
        co(2),
        co(2),
        StickyDisplayCol::BufferLine {
            display_col: BufferLineCol::new(6),
        },
    ));
    let mut ed = Editor::for_testing(Buffer::new(text.clone(), seeded));
    pin_no_wrap(&mut ed);
    ed.handle_key(key('1'));
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(15),
        "BufferLine latch (6) is reused: lands on 'G'"
    );

    // A `DisplayLine`-tagged one is a different quantity under wrap (see
    // `StickyDisplayCol`'s own doc) and must be re-derived from `head`
    // instead — reusing it as a buffer-line column would be a sideways jump.
    let ignored = SelectionSet::single(Selection::with_sticky_display_col(
        co(2),
        co(2),
        StickyDisplayCol::DisplayLine {
            display_col: DisplayLineCol::new(6),
            wrap_width: None,
        },
    ));
    let mut ed = Editor::for_testing(Buffer::new(text, ignored));
    pin_no_wrap(&mut ed);
    ed.handle_key(key('1'));
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(11),
        "DisplayLine latch is ignored: re-derives from head (col 2), lands on 'C'"
    );
}

#[test]
fn resize_invalidates_a_display_line_latch_measured_at_the_old_wrap_width() {
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::BufferText;

    // Line 0 ("0123456789ABCDE", 15 chars) wraps under a content-width-driven
    // `Soft { width: 0 }`. Line 1 ("FGHIJ") gives `j` somewhere to land after
    // line 0's own wrap display lines are exhausted, so the second press below crosses
    // out of the resized block entirely — the case a stale sticky column
    // would misplace worst.
    let text = BufferText::from("0123456789ABCDE\nFGHIJ\n");
    let sels = SelectionSet::single(Selection::collapsed(co(2))); // '2', display line 0 col 2
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    let pid = ed.state.focus.id();
    ed.view.panes[pid].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Soft { width: 0 }),
        saved: None,
    });
    ed.view.panes[pid].viewport.width = 10; // content_width 10 (no gutter in this harness)
    ed.view.panes[pid].viewport.height = 24;

    // At width 10: display line 0 = "0123456789" (chars 0-9), display line 1
    // = "ABCDE" (chars 10-14). `j` from col 2 of display line 0 lands on
    // display line 1's col 2 = 'C' (char 12), latching a `DisplayLine`
    // column of 2 measured against width 10.
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(12),
        "lands on 'C'"
    );

    // Resize to width 8: line 0 re-flows to display line 0 = "01234567" (chars 0-7),
    // display line 1 = "89ABCDE" (chars 8-14) — 'C' (char 12) is now display line 1's col 4,
    // not col 2. A `j` from here must re-derive from head's *current* column
    // (4) rather than reuse the stale latch (2) measured for width 10.
    ed.view.panes[pid].viewport.width = 8;

    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(20),
        "lands on line 1's col 4 ('J'), not col 2 ('H') from the stale latch"
    );
}

#[test]
fn explicit_count_move_down_emits_a_buffer_line_tagged_sticky_column() {
    let mut ed = unwrapped_editor("hello\nworld\n", 2); // 'l', col 2
    ed.handle_key(key('1'));
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(8),
        "lands on 'r' (col 2 of \"world\")"
    );
    assert_eq!(
        ed.current_selections().primary().sticky_display_col(),
        Some(sticky_buffer_line(2)),
        "output must latch a Line-tagged sticky column"
    );
}

#[test]
fn explicit_count_move_down_past_last_content_line_leaves_head_exactly_where_it_was() {
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::BufferText;

    // Already on the buffer's last content line; a further count must leave
    // `head` untouched rather than landing on whatever this line's own width
    // resolves the (absurdly large) latched column to.
    let text = BufferText::from("ab\ncdefgh\n");
    let sels = SelectionSet::single(Selection::with_sticky_display_col(
        co(5),
        co(5),
        sticky_buffer_line(200),
    ));
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    pin_no_wrap(&mut ed);
    ed.handle_key(key('3'));
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(5),
        "head must stay exactly on 'e'"
    );
}

// ── Family switch across the fork (Q29b) ──────────────────────────────────
//
// Bare `j`/`k` (display-line domain, `editor::visual_move`) and an explicit
// count (buffer-line domain, `editor::visual_move::move_buffer_line`) now
// share `Selection::sticky_display_col`, a
// `StickyDisplayCol::DisplayLine`/`BufferLine` enum. With wrap off the two
// domains coincide (a display line IS the buffer line), so the column
// survives a switch; while wrapping they're different quantities, so a
// switch re-derives instead of misreading one as the other.

/// With wrap off, the `Line`-tagged latch bare `j` writes (a display line IS the
/// buffer line there) is the same latch `2j` reads, so the column survives the
/// switch from the display-line-domain path to the buffer-line one.
#[test]
fn no_wrap_j_then_count_2_holds_display_column_across_the_family_switch() {
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::BufferText;

    // line0 = "\tfoo" (tab_width 4: 'f' at display col 4), line1 = "x" (1
    // char), line2 = "abcdefgh" (8 chars, cols 0..7).
    let content = "\tfoo\nx\nabcdefgh\n";
    let text = BufferText::from(content);
    let sels = SelectionSet::single(Selection::collapsed(co(1))); // 'f', display col 4
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    pin_no_wrap(&mut ed);

    ed.handle_key(key('j')); // bare j: display-line-domain path, latches Line(4)
    assert_eq!(
        ed.current_selections().primary().head(),
        co(5),
        "j clamps to the only char on line 1 ('x')"
    );

    ed.handle_key(key('2'));
    ed.handle_key(key('j')); // 2j: hume-ops buffer-line path
    assert_eq!(
        ed.current_selections().primary().head(),
        co(11),
        "2j reuses the col-4 latch, landing on 'e' — re-deriving from the \
         drifted col-0 landing on 'x' would land on 'a' (char 7) instead"
    );
}

/// While wrapping, bare `j` onto a continuation display line latches a
/// `DisplayLine`-variant column — the display line's own, not the buffer line's
/// (see `StickyDisplayCol`). `2j` must re-derive from `head`'s buffer-line
/// column instead of misreading that display-line-relative number as one;
/// this is the trap the naive fix (share the field without tagging it)
/// would fall into.
#[test]
fn wrapped_j_then_count_2_rederives_instead_of_reading_the_display_line_latch_as_a_line_column() {
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::BufferText;

    // line0 = 80 'a's (wraps at col 76, same layout as `visual_test_editor`);
    // line1 = 100 'b's, long enough that display col 40 (the display-line latch) and
    // col 79 (head's real buffer-line column) land on different characters.
    let line0: String = "a".repeat(80);
    let line1: String = "b".repeat(100);
    let content = format!("{line0}\n{line1}\n");
    let text = BufferText::from(content.as_str());
    let sels = SelectionSet::single(Selection::collapsed(co(40))); // display line 0, display col 40
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    ed.view.panes[ed.state.focus.id()].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Indent { width: 76 }),
        saved: None,
    });

    ed.handle_key(key('j')); // bare j: display line 0 -> display line 1, clamped to col 3
    assert_eq!(ed.current_selections().primary().head(), co(79));
    assert_eq!(
        ed.current_selections().primary().sticky_display_col(),
        Some(sticky_display_line(40)),
        "sticky col latches the display-line-relative column, 40"
    );

    ed.handle_key(key('2'));
    ed.handle_key(key('j')); // 2j: buffer-line path
    assert_eq!(
        ed.current_selections().primary().head(),
        co(160),
        "must re-derive from head's buffer-line column (79), landing on \
         line1's 80th 'b' (char 160) — misreading the display-line latch (40) as a \
         buffer-line column would land on char 121 instead"
    );
}

/// No-wrap `j` (`apply_visual_vertical`'s `ContentDisplayLine`) and a
/// screen-relative scroll of the same display-line count
/// (`commands::scroll_view`, what page/half-page/the mouse wheel use) must
/// land on the *same* character — both resolve through the same
/// `DisplayLineMap` display-column authority, just via different call
/// paths since `commands::scroll_view` carries its cursor with
/// `hume_engine::display_lines::carry` rather than `apply_visual_vertical`.
/// Line 0 has a leading tab (tab width 4): 'f' sits at char index 1 but
/// display column 4. Landing by char column would put both on line 1's char
/// index 1 ('b'); landing by display column — the model every vertical path
/// shares — puts both on char index 4 ('e').
#[test]
fn no_wrap_bare_j_and_view_scroll_agree_on_display_column() {
    use crate::editor::commands::scroll_view;
    use crate::editor::visual_move::{VerticalMove, apply_visual_vertical};
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::BufferText;
    use hume_ops::MotionMode;

    let no_wrap_editor_at_f = || {
        let content = "\tfoo\nabcdefgh\n";
        let text = BufferText::from(content);
        let sels = SelectionSet::single(Selection::collapsed(co(1))); // 'f', display col 4
        let mut ed = Editor::for_testing(Buffer::new(text, sels));
        pin_no_wrap(&mut ed);
        ed
    };

    let mut bare_j = no_wrap_editor_at_f();
    let bare_j_pid = bare_j.state.focus.id();
    apply_visual_vertical(
        &mut bare_j.state,
        &mut bare_j.view,
        bare_j_pid,
        1,
        true,
        MotionMode::Move,
        VerticalMove::ContentDisplayLine,
    );
    let bare_j_head = bare_j.current_selections().primary().head();

    let mut view_scroll = no_wrap_editor_at_f();
    let view_scroll_pid = view_scroll.state.focus.id();
    scroll_view(
        &mut view_scroll.state,
        &mut view_scroll.view,
        view_scroll_pid,
        1,
        true,
        MotionMode::Move,
    );
    let view_scroll_head = view_scroll.current_selections().primary().head();

    assert_eq!(
        bare_j_head, view_scroll_head,
        "ContentDisplayLine and a view scroll must land on the same char"
    );
    assert_eq!(
        view_scroll_head,
        co(9),
        "display col 4 on line 1 (\"abcdefgh\") is char index 4 → 'e', absolute offset 9"
    );
}

/// Scroll commands (page/half-page) always move by display lines, regardless of
/// `explicit_count` — the buffer-vs-visual choice is a parameter passed by the
/// caller (`unit`), not a global-state read inside the shared core. This
/// guards against `apply_visual_vertical` accidentally reading
/// `state.explicit_count` itself instead of trusting its parameter.
#[test]
fn apply_visual_vertical_ignores_explicit_count_when_caller_forces_visual() {
    use crate::editor::visual_move::{VerticalMove, apply_visual_vertical};
    use hume_ops::MotionMode;

    let mut ed = visual_test_editor(0);
    ed.state.explicit_count = true; // simulate "a count was typed"
    let pid = ed.state.focus.id();
    apply_visual_vertical(
        &mut ed.state,
        &mut ed.view,
        pid,
        1,
        true,
        MotionMode::Move,
        VerticalMove::ContentDisplayLine,
    );
    assert_eq!(
        ed.current_selections().primary().head(),
        co(76),
        "VerticalMove::ContentDisplayLine must move one display line even with explicit_count=true"
    );
}

/// Each cursor uses its own sticky column in multi-cursor j/k.
///
/// BufferText layout (visual_test_editor):
///   display line 0: chars  0..76 (cols 0..75)
///   display line 1: chars 76..80 (cols 0..3)  ← two cursors placed here
///   line 1:    chars 81..86 "short\n"
///
/// Cursor A at char 76 (col 0), cursor B at char 79 (col 3, primary).
/// j → line 1: A should land at col 0 = char 81, B at col 3 = char 84.
/// k → display line 1: A should return to col 0 = char 76, B to col 3 = char 79.
#[test]
fn visual_move_per_selection_sticky_display_col() {
    use hume_editing::selection::{Selection, SelectionSet};

    let line0: String = "a".repeat(80);
    let content = format!("{}\nshort\n", line0);
    let text = hume_editing::text::BufferText::from(content.as_str());
    // A at col 0, B at col 3 (primary).
    let sels = SelectionSet::from_vec(
        vec![
            Selection::collapsed(co(76)), // A — col 0 on display line 1
            Selection::collapsed(co(79)), // B — col 3 on display line 1
        ],
        1, // primary is B
    );
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    ed.view.panes[ed.state.focus.id()].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Indent { width: 76 }),
        saved: None,
    });

    // j: each cursor should use its own column, not the primary's.
    ed.handle_key(key('j'));
    let sels = ed.current_selections().clone();
    assert_eq!(sels.len(), 2, "two cursors remain distinct");
    // Sorted by start(): A is first.
    let heads: Vec<_> = sels.iter_sorted().map(|s| s.head()).collect();
    assert_eq!(heads[0], co(81), "A (col 0) → char 81 on line 1");
    assert_eq!(heads[1], co(84), "B (col 3) → char 84 on line 1");

    // k: sticky cols should bring each cursor back to its original column.
    ed.handle_key(key('k'));
    let sels = ed.current_selections().clone();
    assert_eq!(sels.len(), 2, "two cursors remain distinct");
    let heads: Vec<_> = sels.iter_sorted().map(|s| s.head()).collect();
    assert_eq!(
        heads[0],
        co(76),
        "A returns to col 0 = char 76 on display line 1"
    );
    assert_eq!(
        heads[1],
        co(79),
        "B returns to col 3 = char 79 on display line 1"
    );
}

// ── Inline decorations and the display-column model (regression) ─────────
//
// `DisplayLineMap::buffer_line_col`/`char_at_buffer_line_col` and their `9j`/`9k`
// wiring must count through the decoration layer, not just buffer text and
// tab expansion — so vertical motion agrees with `DisplayLineMap` (the display
// authority `j`/`k` and page/wheel scroll already use) whenever an inline
// decoration (an inlay hint, say) sits on a line a buffer-line move touches.

/// `9j`/`9k` pressed with no prior latch resolves its column by re-deriving
/// from `head` — the path the rope-only mirror used to own. A 3-column hint
/// sitting before the cursor on its own line shifts the on-screen column by
/// 3; the rope-only mirror never saw it and would land 3 columns short.
#[test]
fn explicit_count_first_press_resolves_column_through_a_preceding_hint() {
    // line 0: hint "HHH" (3 cols) before "abc" — cursor on 'c' (char 2,
    // display col 5: 3 hint cols + 'a','b'). line 1: hint-free "abcdefgh".
    let text = hume_editing::text::BufferText::from("abc\nabcdefgh\n");
    let sels = hume_editing::selection::SelectionSet::single(
        hume_editing::selection::Selection::collapsed(co(2)),
    );
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    pin_no_wrap(&mut ed);
    ed.view.panes[ed.state.focus.id()]
        .providers
        .add_decoration_source(Box::new(InlineHint::new(0, 0, "HHH")));

    ed.handle_key(key('1'));
    ed.handle_key(key('j'));
    assert_eq!(
        ed.current_selections().primary().head(),
        co(9),
        "col 5 (hint-inclusive) of \"abcdefgh\" is 'f' — the rope-only mirror \
         would have derived col 2 and landed on 'c' (char 6) instead"
    );
}

/// Bare `j` (wrap on) latches a `DisplayLine`-variant column — already
/// hint-aware, since `move_vertical` always went through `DisplayLineMap`. A
/// following `2j` crosses families (`DisplayLine` → `BufferLine`), so it
/// can't reuse that latch (see `StickyDisplayCol`) and must re-derive from
/// `head` instead — on a line with a hint before `head`, that re-derivation
/// is exactly where the retired rope-only mirror went blind.
#[test]
fn buffer_line_family_switch_rederives_through_a_hint_not_around_it() {
    // line 0: "xyz" (plain). line 1: hint "HHH" before "abc" — bare j lands
    // on 'a' (char 4). line 2: hint-free "abcdefgh" — 2j's target.
    let text = hume_editing::text::BufferText::from("xyz\nabc\nabcdefgh\n");
    let sels = hume_editing::selection::SelectionSet::single(
        hume_editing::selection::Selection::collapsed(co(0)),
    );
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    // Wide enough that nothing actually wraps — only `is_wrapping()` matters,
    // to force bare `j` to tag `DisplayLine` instead of `BufferLine`.
    ed.view.panes[ed.state.focus.id()].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Soft { width: 200 }),
        saved: None,
    });
    ed.view.panes[ed.state.focus.id()]
        .providers
        .add_decoration_source(Box::new(InlineHint::new(1, 0, "HHH")));

    ed.handle_key(key('j')); // bare j: col 0 target, clamps onto 'a' (virtual hint cells excluded)
    assert_eq!(
        ed.current_selections().primary().head(),
        co(4),
        "lands on 'a'"
    );
    assert_eq!(
        ed.current_selections().primary().sticky_display_col(),
        // Not `sticky_display_line(0)`: that helper's `wrap_width` matches
        // `visual_test_editor`'s width-76 fixture, not this test's own
        // `WrapMode::Soft { width: 200 }`.
        Some(StickyDisplayCol::DisplayLine {
            display_col: DisplayLineCol::new(0),
            wrap_width: Some(200),
        }),
        "latches the DisplayLine-tagged target column (0), not the landed one"
    );

    ed.handle_key(key('2'));
    ed.handle_key(key('j')); // 2j: crosses families, re-derives from 'a' through the hint
    assert_eq!(
        ed.current_selections().primary().head(),
        co(11),
        "re-derives head's line-relative column as 3 (the hint's width) and \
         lands on 'd' — a rope-only re-derivation would compute column 0 \
         (blind to the hint) and land on 'a' (char 8) instead"
    );
}

// ── Visual-line extend variants ───────────────────────────────────────────────
//
// Extend mode is toggled with `e`. In extend mode `j`/`k` resolve to
// extend-down/extend-up: the anchor stays fixed and only the head moves.

/// extend-down (e+j) within a wrapped line: anchor stays at display line 0, head
/// advances to display line 1 of the same buffer line.
#[test]
fn visual_extend_down_within_wrapped_line() {
    let mut ed = visual_test_editor(0);
    ed.handle_key(key('e')); // enter extend mode
    ed.handle_key(key('j'));
    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor(), co(0), "anchor fixed at display line 0 col 0");
    assert_eq!(sel.head(), co(76), "head extends to display line 1 col 0");
}

/// extend-down crosses to the next buffer line when already on the last display line.
#[test]
fn visual_extend_down_crosses_buffer_line() {
    let mut ed = visual_test_editor(76); // last display line of line 0
    ed.handle_key(key('e'));
    ed.handle_key(key('j'));
    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor(), co(76), "anchor fixed at last display line");
    assert_eq!(
        sel.head(),
        co(81),
        "head crosses to first char of next buffer line"
    );
}

/// extend-up (e+k) within a wrapped line: head retreats from display line 1 to display line 0.
#[test]
fn visual_extend_up_within_wrapped_line() {
    let mut ed = visual_test_editor(76); // display line 1 of line 0
    ed.handle_key(key('e'));
    ed.handle_key(key('k'));
    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor(), co(76), "anchor fixed at display line 1");
    assert_eq!(sel.head(), co(0), "head retreats to display line 0 col 0");
}

/// extend-up enters the last display line of the previous buffer line.
#[test]
fn visual_extend_up_enters_previous_line_last_display_line() {
    let mut ed = visual_test_editor(81); // start of "short"
    ed.handle_key(key('e'));
    ed.handle_key(key('k'));
    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor(), co(81), "anchor fixed at line 1 start");
    assert_eq!(
        sel.head(),
        co(76),
        "head enters last display line of previous buffer line"
    );
}

// ── select-word-nearest-on-line: wrap-aware bounds ───────────────────────────
//
// Buffer layout (wrap=76):
//   Line 0: 75 'a's + "+ ratatui\n"  (total 85 chars, 0..84)
//            display line 0: chars  0..75  (75 'a's and '+' at col 75)
//            display line 1: chars 76..84  (' ' at col 0, "ratatui", '\n')
//   Line 1: "short\n"  (chars 85..90)
//
// Char map:
//   75  = '+'
//   76  = ' '  (leading whitespace of display line 1 — the wrap-breaking space)
//   77  = 'r'  (start of "ratatui")
//   83  = 'i'  (end of "ratatui")
//   84  = '\n'
//   85  = 's'  (start of "short")

fn word_wrap_editor() -> Editor {
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::BufferText;
    let content = format!("{}+ ratatui\nshort\n", "a".repeat(75));
    let text = BufferText::from(content.as_str());
    let sels = SelectionSet::single(Selection::collapsed(co(0)));
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    ed.view.panes[ed.state.focus.id()].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Indent { width: 76 }),
        saved: None,
    });
    ed
}

/// `select-word-nearest-on-line` in wrap mode must snap to the word *on the
/// current visual display line*, not across the wrap boundary.
///
/// After `move-down` from col 0 of display line 0, head lands on the leading space
/// of display line 1 (char 76). The nearest-word scan must find "ratatui" (forward,
/// same display line), NOT '+' (backward, previous display line).
#[test]
fn select_word_nearest_scopes_to_visual_display_line() {
    let mut ed = word_wrap_editor();

    // j: head moves to char 76 (leading space of display line 1).
    ed.handle_key(key('j'));
    assert_eq!(ed.current_selections().primary().head(), co(76));

    ed.execute_keymap_command(
        std::borrow::Cow::Borrowed("select-word-nearest-on-line"),
        Some(1),
        false,
    );

    let sel = ed.current_selections().primary();
    assert_ne!(
        sel.head(),
        co(75),
        "must NOT snap to '+' across the wrap boundary"
    );
    assert_eq!(
        sel.head(),
        co(83),
        "must snap to 'ratatui' (last char = 'i' at char 83)"
    );
    assert_eq!(
        sel.sticky_display_col(),
        Some(sticky_display_line(0)),
        "sticky_display_col preserved through snap"
    );
}

/// Two consecutive `j` + `select-word-nearest-on-line` sequences must advance
/// the head forward — no oscillation. The bug this guards against was:
///   j → head=76 (space); select → head=75 ('+', wrong display line);
///   j → head=76 again;   select → head=75 again. (oscillation)
/// With the fix the second select must land strictly past the first.
#[test]
fn select_word_nearest_no_oscillation_on_repeated_j() {
    let mut ed = word_wrap_editor();

    let call_select = |ed: &mut Editor| {
        ed.execute_keymap_command(
            std::borrow::Cow::Borrowed("select-word-nearest-on-line"),
            Some(1),
            false,
        )
    };

    // First j + select: lands on "ratatui" (head = 83).
    ed.handle_key(key('j'));
    call_select(&mut ed);
    let head_after_first_select = ed.current_selections().primary().head();
    assert_eq!(head_after_first_select, co(83));

    // Second j: must advance past 83 (crosses to line 1, display line 0 → 's' at 85).
    ed.handle_key(key('j'));
    let head_after_second_j = ed.current_selections().primary().head();
    assert!(
        head_after_second_j > head_after_first_select,
        "second j must advance past {head_after_first_select:?}; got {head_after_second_j:?}"
    );

    // Second select: must land strictly past the first select — never back.
    call_select(&mut ed);
    let head_after_second_select = ed.current_selections().primary().head();
    assert!(
        head_after_second_select > head_after_first_select,
        "second select must advance past {head_after_first_select:?}; got {head_after_second_select:?} (oscillation)"
    );
}

/// With the default `word-selects-whitespace = true`, the snapped word's
/// leading whitespace bookend is absorbed into the selection (matching `mm`)
/// — even in wrap mode, since the absorbed space (char 76) is buffer-adjacent
/// to "ratatui", not a crossing into a different word.
#[test]
fn select_word_nearest_absorbs_whitespace_bookend_by_default() {
    let mut ed = word_wrap_editor();

    ed.handle_key(key('j')); // head -> char 76 (leading space of display line 1)
    ed.execute_keymap_command(
        std::borrow::Cow::Borrowed("select-word-nearest-on-line"),
        Some(1),
        false,
    );

    let sel = ed.current_selections().primary();
    assert_eq!(
        sel.anchor(),
        co(76),
        "leading space must be absorbed, matching mm's word_unit_at rule"
    );
    assert_eq!(sel.head(), co(83), "still snaps to 'ratatui'");
}

/// A word beginning exactly at a wrapped display line's start (no leading space
/// within that display line — the space is the *previous* display line's trailing char) must
/// not have that space pulled into its around-selection. Before the fix,
/// `expand_word_unit`'s leading scan ignored the display-line bound
/// `nearest_word_on_line` was given and walked straight through it into the
/// previous visual display line.
#[test]
fn select_word_nearest_does_not_absorb_previous_display_line_whitespace() {
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::BufferText;
    // "hello wordB\n": wrap at column 6 puts "hello " (space included) on
    // display line 0, so "wordB" starts display line 1 with no leading space on it.
    let text = BufferText::from("hello wordB\n");
    let sels = SelectionSet::single(Selection::collapsed(co(8))); // 'r' inside "wordB"
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    ed.view.panes[ed.state.focus.id()].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Indent { width: 6 }),
        saved: None,
    });

    ed.execute_keymap_command(
        std::borrow::Cow::Borrowed("select-word-nearest-on-line"),
        Some(1),
        false,
    );

    let sel = ed.current_selections().primary();
    assert_eq!(
        sel.anchor(),
        co(6),
        "must not absorb the space at char 5 — it belongs to the previous visual display line"
    );
    assert_eq!(sel.head(), co(10), "still selects all of 'wordB'");
}

/// `mm` (`select-word`) is NOT wrap-aware — unlike
/// `select-word-nearest-on-line` (see
/// `select_word_nearest_does_not_absorb_previous_display_line_whitespace` above, same
/// buffer/wrap setup), it has no display-line floor to stop the leading-whitespace
/// absorption at a wrap boundary, because it dispatches straight to
/// `word_unit_at` with `min_start = 0` rather than through
/// `nearest_word_on_line`. So on a word that starts a wrapped continuation
/// display line, `mm` pulls in the previous display line's trailing space while the on-cursor
/// snap command does not — a real behavioral difference between the two
/// commands, not a bug in either. This pins the current, deliberate
/// asymmetry so a future change to either command's wrap-awareness is a
/// visible, intentional decision rather than a silent regression.
#[test]
fn select_word_absorbs_previous_display_line_whitespace_unlike_nearest_on_line() {
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::BufferText;
    // Same buffer/wrap as `select_word_nearest_does_not_absorb_previous_display_line_whitespace`:
    // "hello " (space included) wraps onto display line 0, so "wordB" starts
    // display line 1 with no leading space on it.
    let text = BufferText::from("hello wordB\n");
    let sels = SelectionSet::single(Selection::collapsed(co(8))); // 'r' inside "wordB"
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    ed.view.panes[ed.state.focus.id()].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Indent { width: 6 }),
        saved: None,
    });

    ed.execute_keymap_command(std::borrow::Cow::Borrowed("select-word"), Some(1), false);

    let sel = ed.current_selections().primary();
    assert_eq!(
        sel.anchor(),
        co(5),
        "mm DOES absorb the previous display line's space (char 5) — no display-line floor"
    );
    assert_eq!(sel.head(), co(10), "still selects all of 'wordB'");
}

/// With `word-selects-whitespace` off, the selection stays a bare inner word
/// — no whitespace bookend — even in wrap mode.
#[test]
fn select_word_nearest_respects_word_selects_whitespace_off() {
    let mut ed = word_wrap_editor();
    ed.state.settings.word_selects_whitespace = false;

    ed.handle_key(key('j')); // head -> char 76 (leading space of display line 1)
    ed.execute_keymap_command(
        std::borrow::Cow::Borrowed("select-word-nearest-on-line"),
        Some(1),
        false,
    );

    let sel = ed.current_selections().primary();
    assert_eq!(
        sel.anchor(),
        co(77),
        "inner word only — leading space must not be absorbed"
    );
    assert_eq!(sel.head(), co(83), "still snaps to 'ratatui'");
}

// ── Dispatch-origin count semantics ────────────────────────────────────────
//
// `move-down`/`move-up`'s buffer-line-vs-visual-display-line choice comes from
// `CmdCtx.count: Option<usize>` — `None` means "as if no count was typed".
// The keymap trie leaves / WaitChar arm produce `None` for a bare keypress.
// Steel can produce the same `None` explicitly: a script passes a count of
// `0` (the Scheme spelling of "no count"), which `parse_count_extend` decodes
// to `None` before it ever reaches `CmdCtx`. A no-arg `call!`/typed `:move-down`
// still defaults to `Some(1)` (buffer line) — the script has to opt into
// visual-display-line movement by name (passing `0`), it never happens implicitly.

/// Scripted dispatch (`run_command_sync`, the path behind Steel's `call!`) with
/// an explicit `Some(count)` moves by buffer line — unlike a bare keyboard `j`,
/// which stops at the wrap boundary (see `visual_move_down_within_wrapped_line`,
/// char 76). A script can pass `None` (Steel count `0`) to get visual-display-line
/// movement instead — see `steel_call_move_down_zero_count_moves_visual_display_line`.
#[test]
fn run_command_sync_some_count_moves_buffer_line() {
    use hume_scripting::host::CommandHost;

    let mut ed = visual_test_editor(0);
    {
        let mut host = live_host!(ed);
        host.run_command_sync("move-down", Some(1), false, None)
            .expect("run_command_sync must not error for move-down");
    }
    assert_eq!(
        ed.current_selections().primary().head(),
        co(81),
        "scripted move-down (Some(1)) must move a full buffer line, not stop at the wrap boundary"
    );
}

/// A Steel command's own internal `(call! "move-down")` always moves by
/// buffer line regardless of the *outer* key's typed count — the two are
/// dispatched separately, each through its own `run_native_body` call, so
/// the inner one can't inherit the outer's explicitness. This also proves
/// `state.explicit_count` is restored (not left `true`) once the whole
/// dispatch — outer Steel command plus its nested native call — completes.
#[test]
fn steel_call_move_down_ignores_outer_keystrokes_count() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::ScriptingHost;

    let mut ed = visual_test_editor(0);

    let names: Vec<String> = ed
        .state
        .config
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    // The body passes no count to `move-down` — if it inherited the outer
    // key's count-or-lack-thereof, this would move a visual display line instead.
    host.eval_source(
        r#"(define-command! "steel-move-down" ""
                 (lambda () (call! "move-down")))"#,
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    // Simulates `5<key>` bound to "steel-move-down": the outer count (5) must
    // have no bearing on the inner call's buffer-line-vs-visual-display-line choice.
    ed.execute_keymap_command("steel-move-down".into(), Some(5), false);

    assert_eq!(
        ed.current_selections().primary().head(),
        co(81),
        "inner (call! \"move-down\") must move one buffer line, not one visual display line \
         and not the outer key's count of 5 buffer lines"
    );
    assert!(
        !ed.state.explicit_count,
        "explicit_count must be restored to its pre-dispatch value (false) after \
         the outer Steel command and its nested native call both complete"
    );
}

/// A Steel wrapper that forwards its own `count`/`extend` params straight into
/// `(call! "move-down" count extend)` must preserve bare-press visual-display-line
/// movement: dispatching it with `None` (as a keymap trie leaf would for a
/// bare keypress) injects `count = 0` into the lambda, which round-trips back
/// to `None` through `parse_count_extend` — the count-forwarding contract
/// documented in `plugins.md`'s "Calling other commands" section.
///
/// Fail oracle: a `run_steel_command` that injects `ctx.count.unwrap_or(1)`
/// instead of `0` would make the lambda see `1` and always move the buffer
/// line (head 81), never 76.
#[test]
fn steel_wrapper_bare_dispatch_moves_visual_display_line() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::ScriptingHost;

    let mut ed = visual_test_editor(0);
    let mut host = ScriptingHost::new();
    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source(
        r#"(define-command! "steel-jk" ""
                 (lambda (count extend) (call! "move-down" count extend)))"#,
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    // Simulates a bare `<key>` bound to "steel-jk": no count was typed.
    ed.execute_keymap_command("steel-jk".into(), None, false);

    assert_eq!(
        ed.current_selections().primary().head(),
        co(76),
        "bare dispatch through a forwarding Steel wrapper must move one visual \
         display line (char 76), not one buffer line (char 81)"
    );
}

/// The same wrapper with an explicit count still moves by buffer line —
/// forwarding preserves both behaviors, not just the visual-display-line one.
///
/// Buffer: wrapped 80-char line 0, then three short lines "b"/"c"/"d" (chars
/// 81/83/85). From char 0, 3 buffer lines lands on 'd' (85); 3 *visual* display
/// lines (display line 1, then "b", then "c") would land on 'c' (83) instead — the two
/// outcomes are distinguishable, so this pins `VerticalMove::BufferLine`,
/// not just count.
#[test]
fn steel_wrapper_explicit_count_moves_buffer_lines() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::BufferText;
    use hume_scripting::ScriptingHost;

    let line0: String = "a".repeat(80);
    let content = format!("{line0}\nb\nc\nd\n");
    let text = BufferText::from(content.as_str());
    let sels = SelectionSet::single(Selection::collapsed(co(0)));
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    ed.view.panes[ed.state.focus.id()].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Indent { width: 76 }),
        saved: None,
    });

    let mut host = ScriptingHost::new();
    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source(
        r#"(define-command! "steel-jk" ""
                 (lambda (count extend) (call! "move-down" count extend)))"#,
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    ed.execute_keymap_command("steel-jk".into(), Some(3), false);

    assert_eq!(
        ed.current_selections().primary().head(),
        co(85),
        "3<key> through the forwarding wrapper must move 3 buffer lines (char \
         85), not 3 visual display lines (char 83)"
    );
}

/// `(call! "move-down" 0)` from inside a Steel command body moves by visual
/// display line regardless of the *outer* dispatch's count — a script can ask for
/// visual-display-line movement explicitly, not just by forwarding a bare keypress.
/// Also confirms `explicit_count` is restored afterward (mirrors
/// `steel_call_move_down_ignores_outer_keystrokes_count`).
#[test]
fn steel_call_move_down_zero_count_moves_visual_display_line() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::ScriptingHost;

    let mut ed = visual_test_editor(0);
    let mut host = ScriptingHost::new();
    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source(
        r#"(define-command! "steel-vis" ""
                 (lambda () (call! "move-down" 0)))"#,
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    // Outer count is Some(1) — irrelevant, since the body hardcodes 0.
    ed.execute_keymap_command("steel-vis".into(), Some(1), false);

    assert_eq!(
        ed.current_selections().primary().head(),
        co(76),
        "(call! \"move-down\" 0) must move one visual display line (char 76), not one \
         buffer line (char 81)"
    );
    assert!(
        !ed.state.explicit_count,
        "explicit_count must be restored to its pre-dispatch value (false) \
         after the command completes"
    );
}

/// The bare-name wrapper generated by `register_command_names` is variadic —
/// `(move-down 0)` (no `call!`, no wrapper lambda) must also decode `0` to
/// visual-display-line movement. This exercises the generated
/// `(lambda args (%dispatch-command "move-down" args))` binding directly,
/// distinct from the `call!`-macro path the other tests use.
#[test]
fn generated_bare_name_wrapper_accepts_zero_count() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::ScriptingHost;

    let mut ed = visual_test_editor(0);
    let names: Vec<String> = ed
        .state
        .config
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source(
        r#"(define-command! "steel-vis-direct" ""
                 (lambda () (move-down 0)))"#,
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    ed.execute_keymap_command("steel-vis-direct".into(), Some(1), false);

    assert_eq!(
        ed.current_selections().primary().head(),
        co(76),
        "(move-down 0) via the generated variadic wrapper must move one \
         visual display line (char 76), not one buffer line (char 81)"
    );
}
