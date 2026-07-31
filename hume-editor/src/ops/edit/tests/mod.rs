mod align;
mod case;
mod delete;
mod insert;
mod join;
mod paste;
mod replace;
mod sort;

use super::*;
use crate::assert_state;

// ── repeat_edit (count prefix for edits) ──────────────────────────────────

#[test]
fn repeat_delete_forward_count_3() {
    // 3x: delete 'h', then 'e', then 'l' — cursor lands on the second 'l'.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| repeat_edit(3, buf, sels, delete_char_forward),
        "-[l]>o\n"
    );
}

#[test]
fn repeat_delete_forward_count_exceeds_buffer() {
    // count=100 on a 3-char buffer ("hi\n"). Deletes 'h' and 'i', then
    // 98 no-ops on the structural '\n' (cannot be deleted).
    assert_state!(
        "-[h]>i\n",
        |(buf, sels)| repeat_edit(100, buf, sels, delete_char_forward),
        "-[\n]>"
    );
}

#[test]
fn repeat_delete_backward_count_2() {
    // 2<BS>: delete 'l' (offset 3), then 'e' (offset 2) from "hello\n".
    // Cursor was on 'l'(3); after first delete it sits on 'l'(2→now 'l'),
    // after second delete it sits on 'l' which is now at offset 2.
    assert_state!(
        "hel-[l]>o\n",
        |(buf, sels)| repeat_edit(2, buf, sels, delete_char_backward),
        "h-[l]>o\n"
    );
}

// ── repeat_edit count=0 ───────────────────────────────────────────────────

#[test]
fn repeat_edit_count_zero_is_noop() {
    // count=0 produces an identity ChangeSet and leaves buf+sels unchanged.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| repeat_edit(0, buf, sels, delete_char_forward),
        "-[h]>ello\n"
    );
}
