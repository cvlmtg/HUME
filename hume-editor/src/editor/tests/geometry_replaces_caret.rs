//! A resize or a wrap-mode change must still replace the caret even though
//! the cursor itself is unmoved between the two frames each test drives.
//! Neither fixture moves the primary head, so `PaneBufferState::reveal_pending`
//! would stay `false` if resize and wrap-mode change did not each explicitly
//! raise it themselves — these tests pin that both do, closing exactly the
//! gap a purely selection-driven reveal signal would otherwise have.

use super::*;

/// A resize with the cursor unmoved must still replace the caret:
/// `sync_viewport_dims` raises `reveal_pending` itself when a pane's height
/// actually changes, so `frame.rs`'s scroll step runs `Viewport::reveal` and
/// scrolls the (unchanged) viewport top far enough for the shrunk height to
/// still cover the cursor, rather than trusting a stale top left over from
/// the taller frame.
#[test]
fn a_height_change_replaces_the_caret_even_when_the_cursor_has_not_moved() {
    let content: String = numbered_lines(60);
    let mut ed = unwrapped_editor(&content, 0);
    seek_to_line(&mut ed, 40);

    assert!(
        frame(&mut ed, 80, 50).cursor_content_pos.is_some(),
        "sanity: the cursor is visible in the tall viewport"
    );

    assert!(
        frame(&mut ed, 80, 10).cursor_content_pos.is_some(),
        "the caret must still be placed after a resize, even though the \
         cursor itself did not move"
    );
}

/// A wrap-mode change with the cursor unmoved must still replace the caret:
/// `set_focused_wrap_override`/`toggle_focused_wrap` raise `reveal_pending`
/// themselves on an actual mode change, so `frame.rs`'s scroll step
/// re-places the caret against wrapping's now-different display-line layout
/// rather than trusting the unwrapped frame's screen position.
#[test]
fn a_wrap_mode_change_replaces_the_caret_even_when_the_cursor_has_not_moved() {
    let content: String = (0..60)
        .map(|i| format!("{i} {}\n", "x".repeat(120)))
        .collect();
    let mut ed = unwrapped_editor(&content, 0);
    seek_to_line(&mut ed, 20);

    assert!(
        frame(&mut ed, 80, 15).cursor_content_pos.is_some(),
        "sanity: the cursor is visible unwrapped"
    );

    // Through the real production path (`:set pane wrap-mode=…`), not a raw
    // `Pane::set_wrap` poke: only `set_focused_wrap_override`/
    // `toggle_focused_wrap` ever change wrap mode in a running editor, and
    // both raise `PaneBufferState::reveal_pending` — a poke that bypasses
    // them bypasses the signal too, which is not a gap this test should
    // exercise.
    run_set(&mut ed, "pane wrap-mode=soft:0").expect(":set pane wrap-mode=soft:0 failed");

    assert!(
        frame(&mut ed, 80, 15).cursor_content_pos.is_some(),
        "the caret must still be placed after a wrap-mode change, even \
         though the cursor's DisplayLinePos is unchanged"
    );
}
