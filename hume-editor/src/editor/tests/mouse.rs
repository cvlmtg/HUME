use super::*;
use pretty_assertions::assert_eq;
use termina::event::{Event, Modifiers, MouseButton, MouseEvent, MouseEventKind};

fn mouse_left_down(col: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: col,
        row,
        modifiers: Modifiers::NONE,
    })
}

/// Regression: `end_insert_session` can mutate the buffer (the blank-line
/// indent trim, code review fix #3) — a mouse click that exits Insert mode
/// must recompute its char offset AFTER that mutation, not before, or a
/// stale offset can land past the shrunk buffer's end (fix #2).
#[test]
fn click_after_blank_line_trim_lands_on_correct_char() {
    // "  x\ncd\n": enter Insert with the cursor on line 0's own trailing '\n'.
    let mut ed = editor_from("  x-[\n]>cd\n");
    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    // Enter copies "  " onto a new line and lands the cursor on *that* line's
    // trailing '\n' — a blank, auto-indented line (buffer is now
    // "  x\n  \ncd\n", cursor at char 6). `autoindent_pending` is set, so
    // exiting Insert now will trim that "  ".
    assert_eq!(state(&ed), "  x\n  -[\n]>cd\n");

    // Click on 'd' (line 2, column 1, no gutter in test harness) to exit
    // Insert mode via the mouse.
    ed.handle_event(mouse_left_down(1, 2));

    assert_eq!(ed.state.mode, Mode::Normal);
    // The blank line's "  " is trimmed on exit (buffer shrinks to
    // "  x\n\ncd\n"), and the click must land on 'd' in the *new* buffer —
    // not at the stale pre-trim offset, which would land 2 chars past 'd'
    // (out of bounds before the fix, since the buffer is now 2 chars
    // shorter than it was when the click coordinates were captured).
    assert_eq!(state(&ed), "  x\n\nc-[d]>\n");
}
