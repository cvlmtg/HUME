use crate::buffer::Buffer;
use crate::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use crate::selection::{Selection, SelectionSet};

// ── Core helper ───────────────────────────────────────────────────────────────

/// Apply `f` to every selection left-to-right, accumulating the net char-count
/// change from each edit so that subsequent selections see their correct
/// positions in the already-mutated buffer.
///
/// # Why left-to-right with delta?
///
/// The natural instinct is "apply right-to-left so earlier positions stay
/// valid". That is correct for the *input* positions — but the *output*
/// positions (the new cursors `f` returns) are then invalidated when a
/// subsequent left-side edit shifts the buffer. Tracking the cumulative delta
/// and adjusting each input selection before calling `f` sidesteps the
/// problem entirely: every call sees a consistently-positioned `adjusted`
/// selection and leaves a result that is already correct in the final buffer.
///
/// # Primary tracking
///
/// The primary index is carried through from the original `SelectionSet`.
/// `merge_overlapping` (called at the end) adjusts it further if adjacent
/// cursors collapse into one.
fn apply_to_each<F>(buf: &Buffer, sels: SelectionSet, mut f: F) -> (Buffer, SelectionSet)
where
    F: FnMut(&Buffer, Selection) -> (Buffer, Selection),
{
    let primary_idx = sels.primary_index();
    let sorted: Vec<Selection> = sels.iter_sorted().copied().collect();

    let mut current_buf = buf.clone();
    let mut new_sels: Vec<Selection> = Vec::with_capacity(sorted.len());
    // `delta` is the net change in char count from all edits so far.
    // Positive = characters were inserted; negative = deleted.
    let mut delta: isize = 0;

    for sel in sorted {
        // Shift this selection's offsets to account for all previous edits.
        let adjusted = sel.shift(delta);
        let old_len = current_buf.len_chars() as isize;
        let (new_buf, new_sel) = f(&current_buf, adjusted);
        delta += new_buf.len_chars() as isize - old_len;
        current_buf = new_buf;
        new_sels.push(new_sel);
    }

    // Rebuild with merge in case adjacent cursors converged (e.g. two cursors
    // on neighbouring chars both deleted forward — both land at the same spot).
    let new_set = SelectionSet::from_vec(new_sels, primary_idx).merge_overlapping();
    (current_buf, new_set)
}

// ── Public operations ─────────────────────────────────────────────────────────

/// Insert `ch` at every selection.
///
/// - **Collapsed cursor**: `ch` is inserted at the cursor position; the cursor
///   advances by one (landing on the character that follows the new one).
/// - **Non-collapsed selection**: the selected region is deleted first, then
///   `ch` is inserted at the start of the former selection. The cursor lands
///   one past the inserted character.
///
/// This covers single-cursor typing, multicursor typing, and "replace
/// selection with typed character" — all via the same loop.
pub(crate) fn insert_char(buf: &Buffer, sels: SelectionSet, ch: char) -> (Buffer, SelectionSet) {
    let ch_str = ch.to_string();
    apply_to_each(buf, sels, |buf, sel| {
        let start = sel.start();
        if sel.is_cursor() {
            // Insert at cursor; cursor advances past the new character.
            (buf.insert(start, &ch_str), Selection::cursor(start + 1))
        } else {
            // Delete the selected region (inclusive end → exclusive +1), then insert.
            let end_excl = sel.end() + 1;
            let new_buf = buf.remove(start, end_excl).insert(start, &ch_str);
            (new_buf, Selection::cursor(start + 1))
        }
    })
}

/// Delete the grapheme cluster at the cursor, or delete the selected region.
///
/// - **Collapsed cursor**: delete the grapheme cluster starting at `head`
///   (the character the cursor sits on). Cursor stays at the same offset
///   (it now points to what was the next character). No-op at end of buffer.
/// - **Non-collapsed selection**: delete the entire selected region. Cursor
///   collapses to `start()`.
pub(crate) fn delete_char_forward(
    buf: &Buffer,
    sels: SelectionSet,
) -> (Buffer, SelectionSet) {
    apply_to_each(buf, sels, |buf, sel| {
        if sel.is_cursor() {
            let p = sel.head;
            if p >= buf.len_chars() {
                // At end of buffer — nothing to delete.
                return (buf.clone(), sel);
            }
            // Delete one grapheme cluster starting at `p`.
            let end = next_grapheme_boundary(buf, p);
            (buf.remove(p, end), Selection::cursor(p))
        } else {
            // Delete the selected region (end() is inclusive).
            let start = sel.start();
            let end_excl = sel.end() + 1;
            (buf.remove(start, end_excl), Selection::cursor(start))
        }
    })
}

/// Delete the grapheme cluster before the cursor, or delete the selected region.
///
/// - **Collapsed cursor**: delete the grapheme cluster that ends just before
///   `head` (i.e. the one the user would see to the left of the cursor).
///   Cursor moves back to the start of the deleted cluster. No-op at start.
/// - **Non-collapsed selection**: delete the entire selected region. Cursor
///   collapses to `start()`. (Same as `delete_char_forward` for selections —
///   pressing Delete or Backspace on a selection both clear it.)
pub(crate) fn delete_char_backward(
    buf: &Buffer,
    sels: SelectionSet,
) -> (Buffer, SelectionSet) {
    apply_to_each(buf, sels, |buf, sel| {
        if sel.is_cursor() {
            let p = sel.head;
            if p == 0 {
                // At start of buffer — nothing to delete.
                return (buf.clone(), sel);
            }
            // Delete the grapheme cluster whose last char is at offset p-1.
            let prev = prev_grapheme_boundary(buf, p);
            (buf.remove(prev, p), Selection::cursor(prev))
        } else {
            let start = sel.start();
            let end_excl = sel.end() + 1;
            (buf.remove(start, end_excl), Selection::cursor(start))
        }
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assert_state;

    // ── insert_char ───────────────────────────────────────────────────────────

    #[test]
    fn insert_char_at_cursor_start() {
        // Cursor on 'h'; 'x' inserted before it; cursor advances to 'h'.
        assert_state!("|hello", |(buf, sels)| insert_char(&buf, sels, 'x'), "x|hello");
    }

    #[test]
    fn insert_char_at_cursor_middle() {
        // Cursor on second 'l' (offset 3); 'x' inserted, cursor on 'l'.
        assert_state!("hel|lo", |(buf, sels)| insert_char(&buf, sels, 'x'), "helx|lo");
    }

    #[test]
    fn insert_char_at_cursor_eof() {
        // Cursor at EOF (offset 5); 'x' appended; cursor at new EOF.
        assert_state!("hello|", |(buf, sels)| insert_char(&buf, sels, 'x'), "hellox|");
    }

    #[test]
    fn insert_char_into_empty_buffer() {
        assert_state!("|", |(buf, sels)| insert_char(&buf, sels, 'x'), "x|");
    }

    #[test]
    fn insert_char_replaces_forward_selection() {
        // Selection anchor=0, head=3 covers 'h','e','l','l' (4 chars).
        // Delete [0,4), insert 'x', cursor at 1.
        assert_state!(
            "#[hel|l]#o",
            |(buf, sels)| insert_char(&buf, sels, 'x'),
            "x|o"
        );
    }

    #[test]
    fn insert_char_replaces_whole_buffer() {
        assert_state!(
            "#[hell|o]#",
            |(buf, sels)| insert_char(&buf, sels, 'x'),
            "x|"
        );
    }

    #[test]
    fn insert_char_replaces_backward_selection() {
        // anchor=3, head=0 covers chars 0-3 ('h','e','l','l').
        // Delete [0,4), insert 'x' at 0, cursor at 1.
        // Buffer "hello" → remove "hell" → "o", insert 'x' → "xo".
        assert_state!(
            "#[|hel]#lo",
            |(buf, sels)| insert_char(&buf, sels, 'x'),
            "x|o"
        );
    }

    #[test]
    fn insert_char_two_cursors() {
        // Cursors at 0 and 3. Left-to-right: insert 'x' at 0, delta=1;
        // insert 'x' at adjusted(3+1=4). Result: "xfoox bar", cursors at 1 and 5.
        assert_state!(
            "|foo| bar",
            |(buf, sels)| insert_char(&buf, sels, 'x'),
            "x|foox| bar"
        );
    }

    #[test]
    fn insert_char_unicode() {
        // Insert a multi-byte char (2 bytes in UTF-8, 1 char offset).
        assert_state!("caf|é", |(buf, sels)| insert_char(&buf, sels, 'à'), "cafà|é");
    }

    // ── delete_char_forward ───────────────────────────────────────────────────

    #[test]
    fn delete_forward_at_cursor_start() {
        // Cursor on 'h'; deletes 'h'; cursor stays at 0 (now on 'e').
        assert_state!("|hello", |(buf, sels)| delete_char_forward(&buf, sels), "|ello");
    }

    #[test]
    fn delete_forward_at_cursor_middle() {
        assert_state!("h|ello", |(buf, sels)| delete_char_forward(&buf, sels), "h|llo");
    }

    #[test]
    fn delete_forward_at_eof_is_noop() {
        assert_state!("hello|", |(buf, sels)| delete_char_forward(&buf, sels), "hello|");
    }

    #[test]
    fn delete_forward_empty_buffer_is_noop() {
        assert_state!("|", |(buf, sels)| delete_char_forward(&buf, sels), "|");
    }

    #[test]
    fn delete_forward_selection() {
        // Selection [0,3] inclusive → remove [0,4) → "o", cursor at 0.
        assert_state!(
            "#[hel|l]#o",
            |(buf, sels)| delete_char_forward(&buf, sels),
            "|o"
        );
    }

    #[test]
    fn delete_forward_two_cursors() {
        // Cursors at 0 ('h') and 2 ('l'). Delete 'h' → "ello", cursor 0, delta=-1.
        // Adjust cursor 2 → 1; delete 'l' at 1 in "ello" → "elo", cursor 1.
        // Buffer "elo", cursors at 0 and 1 (distinct positions, no merge).
        assert_state!(
            "|he|llo",
            |(buf, sels)| delete_char_forward(&buf, sels),
            "|e|lo"
        );
    }

    #[test]
    fn delete_forward_adjacent_cursors_merge() {
        // Cursors at 2 and 3. Both delete forward; both land at 2 → merge.
        assert_state!(
            "he|l|lo",
            |(buf, sels)| delete_char_forward(&buf, sels),
            "he|o"
        );
    }

    #[test]
    fn delete_forward_grapheme_cluster() {
        // "e\u{0301}x": é is 2 chars, 1 grapheme. Cursor at 0 deletes whole cluster.
        assert_state!(
            "|e\u{0301}x",
            |(buf, sels)| delete_char_forward(&buf, sels),
            "|x"
        );
    }

    // ── delete_char_backward ─────────────────────────────────────────────────

    #[test]
    fn delete_backward_at_cursor_end() {
        // Cursor at EOF (offset 5); backspace deletes 'o'; cursor at 4.
        assert_state!("hello|", |(buf, sels)| delete_char_backward(&buf, sels), "hell|");
    }

    #[test]
    fn delete_backward_at_cursor_middle() {
        // Cursor at 3 ('l'); backspace deletes 'l' at 2; cursor at 2.
        assert_state!("hel|lo", |(buf, sels)| delete_char_backward(&buf, sels), "he|lo");
    }

    #[test]
    fn delete_backward_at_start_is_noop() {
        assert_state!("|hello", |(buf, sels)| delete_char_backward(&buf, sels), "|hello");
    }

    #[test]
    fn delete_backward_empty_buffer_is_noop() {
        assert_state!("|", |(buf, sels)| delete_char_backward(&buf, sels), "|");
    }

    #[test]
    fn delete_backward_selection() {
        // Same as delete_forward for non-collapsed: removes selected region.
        assert_state!(
            "#[hel|l]#o",
            |(buf, sels)| delete_char_backward(&buf, sels),
            "|o"
        );
    }

    #[test]
    fn delete_backward_two_cursors() {
        // Cursors at 2 and 4. Backspace at 2: delete offset 1 ('e'), cursor 1, delta=-1.
        // Adjust cursor 4 → 3; backspace at 3: delete offset 2 ('l'), cursor 2.
        // Buffer "hlло" → "hlo"... let me trace: "hello", cursors at 2 and 4.
        // Backspace at cursor 2: prev(2)=1, delete [1,2) → "hllo", cursor 1, delta=-1.
        // Adjusted cursor 4 → 3; backspace at 3: prev(3)=2, delete [2,3) in "hllo" → "hlo", cursor 2.
        // Result: "hlo", cursors at [1, 2].
        assert_state!(
            "he|ll|o",
            |(buf, sels)| delete_char_backward(&buf, sels),
            "h|l|o"
        );
    }

    #[test]
    fn delete_backward_grapheme_cluster() {
        // "e\u{0301}x": é is 2 chars (offsets 0-1). Cursor at 2 (on 'x').
        // prev_grapheme_boundary(2) = 0. Deletes entire é cluster.
        assert_state!(
            "e\u{0301}|x",
            |(buf, sels)| delete_char_backward(&buf, sels),
            "|x"
        );
    }

    #[test]
    fn delete_backward_adjacent_cursors_merge() {
        // Cursors at 2 and 3. Backspace at 2: delete offset 1, cursor 1, delta=-1.
        // Adjusted 3 → 2; backspace at 2: delete offset 1 in updated buf, cursor 1.
        // Both land at 1 → merge.
        assert_state!(
            "he|l|lo",
            |(buf, sels)| delete_char_backward(&buf, sels),
            "h|lo"
        );
    }
}
