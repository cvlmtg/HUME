use crate::buffer::Buffer;
use crate::changeset::ChangeSetBuilder;
use crate::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use crate::selection::{Selection, SelectionSet};

// ── Public operations ─────────────────────────────────────────────────────────
//
// Each operation builds a `ChangeSet` via the builder, working entirely in
// **original-buffer coordinates**. The builder's `new_pos()` gives cursor
// positions directly in the result buffer's coordinate space — no cumulative
// delta tracking, no intermediate buffer clones.

/// Insert `ch` at every selection.
///
/// - **Single-character selection**: `ch` is inserted before the cursor
///   character; the cursor advances to land on the character that follows it.
/// - **Multi-character selection**: the selected region is deleted first, then
///   `ch` is inserted at the start of the former selection. The cursor lands
///   one past the inserted character.
///
/// This covers single-cursor typing, multicursor typing, and "replace
/// selection with typed character" — all via the same loop.
pub(crate) fn insert_char(buf: Buffer, sels: SelectionSet, ch: char) -> (Buffer, SelectionSet) {
    let mut b = ChangeSetBuilder::new(buf.len_chars());
    let mut new_sels: Vec<Selection> = Vec::with_capacity(sels.len());
    let primary_idx = sels.primary_index();

    for sel in sels.iter_sorted() {
        let start = sel.start();

        // Retain everything between the builder's current position and this
        // selection — these chars are untouched by this edit.
        b.retain(start - b.old_pos());

        if !sel.is_cursor() {
            // Delete the selected region. end() is inclusive, so +1 for the
            // exclusive bound that the builder expects.
            b.delete(sel.end() + 1 - start);
        }

        // Insert the character. The cursor lands right after it.
        b.insert_char(ch);
        new_sels.push(Selection::cursor(b.new_pos()));
    }

    b.retain_rest();
    let new_buf = b.finish().apply(&buf).expect("insert_char: internal changeset is always valid");
    let new_sel_set = SelectionSet::from_vec(new_sels, primary_idx).merge_overlapping();
    new_sel_set.debug_assert_valid(new_buf.len_chars());
    (new_buf, new_sel_set)
}

/// Delete the grapheme cluster at the cursor, or delete the selected region.
///
/// - **Single-character selection**: delete the grapheme cluster at `head`
///   (the character the cursor sits on). Cursor stays at the same offset
///   (it now points to what was the next character). No-op when the cursor
///   is on the trailing `\n` (the structural last character of every buffer).
/// - **Multi-character selection**: delete the entire selected region. Cursor
///   lands on `start()`.
pub(crate) fn delete_char_forward(
    buf: Buffer,
    sels: SelectionSet,
) -> (Buffer, SelectionSet) {
    let mut b = ChangeSetBuilder::new(buf.len_chars());
    let mut new_sels: Vec<Selection> = Vec::with_capacity(sels.len());
    let primary_idx = sels.primary_index();

    for sel in sels.iter_sorted() {
        if sel.is_cursor() {
            let p = sel.head;
            if p + 1 >= buf.len_chars() {
                // Cursor is on the last character (the structural trailing \n)
                // — deleting it would violate the buffer invariant. No-op.
                b.retain(p - b.old_pos());
                new_sels.push(Selection::cursor(b.new_pos()));
                continue;
            }
            // Delete one grapheme cluster starting at `p`. We call
            // next_grapheme_boundary on the *original* buffer — all
            // positions in the builder are in original-buffer space.
            let end = next_grapheme_boundary(&buf, p);
            b.retain(p - b.old_pos());
            b.delete(end - p);
            new_sels.push(Selection::cursor(b.new_pos()));
        } else {
            let start = sel.start();
            let end_excl = sel.end() + 1;
            b.retain(start - b.old_pos());
            b.delete(end_excl - start);
            new_sels.push(Selection::cursor(b.new_pos()));
        }
    }

    b.retain_rest();
    let new_buf = b.finish().apply(&buf).expect("delete_char_forward: internal changeset is always valid");
    let new_sel_set = SelectionSet::from_vec(new_sels, primary_idx).merge_overlapping();
    new_sel_set.debug_assert_valid(new_buf.len_chars());
    (new_buf, new_sel_set)
}

/// Delete the grapheme cluster before the cursor, or delete the selected region.
///
/// - **Single-character selection**: delete the grapheme cluster that ends
///   just before `head` (the character to the left of the cursor). Cursor
///   moves back to the start of the deleted cluster. No-op at start.
/// - **Multi-character selection**: delete the entire selected region. Cursor
///   lands on `start()`. (Same as `delete_char_forward` for selections —
///   Delete and Backspace both clear a selection.)
pub(crate) fn delete_char_backward(
    buf: Buffer,
    sels: SelectionSet,
) -> (Buffer, SelectionSet) {
    let mut b = ChangeSetBuilder::new(buf.len_chars());
    let mut new_sels: Vec<Selection> = Vec::with_capacity(sels.len());
    let primary_idx = sels.primary_index();

    for sel in sels.iter_sorted() {
        if sel.is_cursor() {
            let p = sel.head;
            if p == 0 {
                // At start of buffer — nothing to delete.
                new_sels.push(Selection::cursor(b.new_pos()));
                continue;
            }
            // Delete the grapheme cluster ending just before `p`.
            let prev = prev_grapheme_boundary(&buf, p);
            b.retain(prev - b.old_pos());
            b.delete(p - prev);
            new_sels.push(Selection::cursor(b.new_pos()));
        } else {
            let start = sel.start();
            let end_excl = sel.end() + 1;
            b.retain(start - b.old_pos());
            b.delete(end_excl - start);
            new_sels.push(Selection::cursor(b.new_pos()));
        }
    }

    b.retain_rest();
    let new_buf = b.finish().apply(&buf).expect("delete_char_backward: internal changeset is always valid");
    let new_sel_set = SelectionSet::from_vec(new_sels, primary_idx).merge_overlapping();
    new_sel_set.debug_assert_valid(new_buf.len_chars());
    (new_buf, new_sel_set)
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
        assert_state!("|hello\n", |(buf, sels)| insert_char(buf, sels,'x'), "x|hello\n");
    }

    #[test]
    fn insert_char_at_cursor_middle() {
        // Cursor on second 'l' (offset 3); 'x' inserted, cursor on 'l'.
        assert_state!("hel|lo\n", |(buf, sels)| insert_char(buf, sels,'x'), "helx|lo\n");
    }

    #[test]
    fn insert_char_at_cursor_eof() {
        // Cursor at EOF (offset 5); 'x' appended; cursor at new EOF.
        assert_state!("hello|\n", |(buf, sels)| insert_char(buf, sels,'x'), "hellox|\n");
    }

    #[test]
    fn insert_char_into_empty_buffer() {
        assert_state!("|\n", |(buf, sels)| insert_char(buf, sels,'x'), "x|\n");
    }

    #[test]
    fn insert_char_replaces_forward_selection() {
        // Selection anchor=0, head=3 covers 'h','e','l','l' (4 chars).
        // Delete [0,4), insert 'x', cursor at 1.
        assert_state!(
            "#[hel|l]#o\n",
            |(buf, sels)| insert_char(buf, sels,'x'),
            "x|o\n"
        );
    }

    #[test]
    fn insert_char_replaces_whole_buffer() {
        assert_state!(
            "#[hell|o]#\n",
            |(buf, sels)| insert_char(buf, sels,'x'),
            "x|\n"
        );
    }

    #[test]
    fn insert_char_replaces_backward_selection() {
        // anchor=3, head=0 covers chars 0-3 ('h','e','l','l').
        // Delete [0,4), insert 'x' at 0, cursor at 1.
        // Buffer "hello" → remove "hell" → "o", insert 'x' → "xo".
        assert_state!(
            "#[|hel]#lo\n",
            |(buf, sels)| insert_char(buf, sels,'x'),
            "x|o\n"
        );
    }

    #[test]
    fn insert_char_two_cursors() {
        // Cursors at 0 and 3. Insert 'x' at both positions.
        // Changeset: Insert("x"), Retain(3), Insert("x"), Retain(4).
        // Result: "xfoox bar", cursors at 1 and 5.
        assert_state!(
            "|foo| bar\n",
            |(buf, sels)| insert_char(buf, sels,'x'),
            "x|foox| bar\n"
        );
    }

    #[test]
    fn insert_char_unicode() {
        // Insert a multi-byte char (2 bytes in UTF-8, 1 char offset).
        assert_state!("caf|é\n", |(buf, sels)| insert_char(buf, sels,'à'), "cafà|é\n");
    }

    // ── delete_char_forward ───────────────────────────────────────────────────

    #[test]
    fn delete_forward_at_cursor_start() {
        // Cursor on 'h'; deletes 'h'; cursor stays at 0 (now on 'e').
        assert_state!("|hello\n", |(buf, sels)| delete_char_forward(buf, sels), "|ello\n");
    }

    #[test]
    fn delete_forward_at_cursor_middle() {
        assert_state!("h|ello\n", |(buf, sels)| delete_char_forward(buf, sels), "h|llo\n");
    }

    #[test]
    fn delete_forward_at_eof_is_noop() {
        assert_state!("hello|\n", |(buf, sels)| delete_char_forward(buf, sels), "hello|\n");
    }

    #[test]
    fn delete_forward_empty_buffer_is_noop() {
        assert_state!("|\n", |(buf, sels)| delete_char_forward(buf, sels), "|\n");
    }

    #[test]
    fn delete_forward_selection() {
        // Selection [0,3] inclusive → remove [0,4) → "o", cursor at 0.
        assert_state!(
            "#[hel|l]#o\n",
            |(buf, sels)| delete_char_forward(buf, sels),
            "|o\n"
        );
    }

    #[test]
    fn delete_forward_two_cursors() {
        // Cursors at 0 ('h') and 2 ('l'). Delete 'h' and first 'l'.
        // Changeset: Delete(1), Retain(1), Delete(1), Retain(2).
        // Result: "elo", cursors at 0 and 1.
        assert_state!(
            "|he|llo\n",
            |(buf, sels)| delete_char_forward(buf, sels),
            "|e|lo\n"
        );
    }

    #[test]
    fn delete_forward_adjacent_cursors_merge() {
        // Cursors at 2 and 3. Both delete forward; both land at 2 → merge.
        assert_state!(
            "he|l|lo\n",
            |(buf, sels)| delete_char_forward(buf, sels),
            "he|o\n"
        );
    }

    #[test]
    fn delete_forward_grapheme_cluster() {
        // "e\u{0301}x": é is 2 chars, 1 grapheme. Cursor at 0 deletes whole cluster.
        assert_state!(
            "|e\u{0301}x\n",
            |(buf, sels)| delete_char_forward(buf, sels),
            "|x\n"
        );
    }

    // ── delete_char_backward ─────────────────────────────────────────────────

    #[test]
    fn delete_backward_at_cursor_end() {
        // Cursor at EOF (offset 5); backspace deletes 'o'; cursor at 4.
        assert_state!("hello|\n", |(buf, sels)| delete_char_backward(buf, sels), "hell|\n");
    }

    #[test]
    fn delete_backward_at_cursor_middle() {
        // Cursor at 3 ('l'); backspace deletes 'l' at 2; cursor at 2.
        assert_state!("hel|lo\n", |(buf, sels)| delete_char_backward(buf, sels), "he|lo\n");
    }

    #[test]
    fn delete_backward_at_start_is_noop() {
        assert_state!("|hello\n", |(buf, sels)| delete_char_backward(buf, sels), "|hello\n");
    }

    #[test]
    fn delete_backward_empty_buffer_is_noop() {
        assert_state!("|\n", |(buf, sels)| delete_char_backward(buf, sels), "|\n");
    }

    #[test]
    fn delete_backward_selection() {
        // Same as delete_forward for multi-char selections: removes selected region.
        assert_state!(
            "#[hel|l]#o\n",
            |(buf, sels)| delete_char_backward(buf, sels),
            "|o\n"
        );
    }

    #[test]
    fn delete_backward_two_cursors() {
        // Cursors at 2 and 4 in "hello". Backspace at 2 deletes 'e' (offset 1).
        // Backspace at 4 deletes 'l' (offset 3).
        // Changeset: Retain(1), Delete(1), Retain(1), Delete(1), Retain(1).
        // Result: "hlo", cursors at 1 and 2.
        assert_state!(
            "he|ll|o\n",
            |(buf, sels)| delete_char_backward(buf, sels),
            "h|l|o\n"
        );
    }

    #[test]
    fn delete_backward_grapheme_cluster() {
        // "e\u{0301}x": é is 2 chars (offsets 0-1). Cursor at 2 (on 'x').
        // prev_grapheme_boundary(2) = 0. Deletes entire é cluster.
        assert_state!(
            "e\u{0301}|x\n",
            |(buf, sels)| delete_char_backward(buf, sels),
            "|x\n"
        );
    }

    #[test]
    fn delete_backward_adjacent_cursors_merge() {
        // Cursors at 2 and 3. Backspace at 2: delete offset 1. Backspace at 3:
        // delete offset 2 in original. Both cursors land at 1 → merge.
        assert_state!(
            "he|l|lo\n",
            |(buf, sels)| delete_char_backward(buf, sels),
            "h|lo\n"
        );
    }
}
