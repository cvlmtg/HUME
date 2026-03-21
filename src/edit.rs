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

/// Delete every selection.
///
/// - **Single-character selection (cursor)**: delete the character under the
///   cursor. The cursor lands on the character that slides into that position,
///   or stays put if we are at the end of the buffer. No-op when the cursor is
///   on the structural trailing `\n` (deleting it would violate the buffer
///   invariant).
/// - **Multi-character selection**: delete the entire selected region. The
///   cursor lands at `start()`.
///
/// This is the normal-mode `d` operation. It does NOT capture the deleted text
/// into a register — the caller is responsible for that:
///
/// ```ignore
/// let yanked = yank_selections(&buf, &sels);
/// let (new_buf, new_sels) = delete_selection(buf, sels);
/// registers.write(DEFAULT_REGISTER, yanked);
/// ```
pub(crate) fn delete_selection(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let mut b = ChangeSetBuilder::new(buf.len_chars());
    let mut new_sels: Vec<Selection> = Vec::with_capacity(sels.len());
    let primary_idx = sels.primary_index();

    for sel in sels.iter_sorted() {
        if sel.is_cursor() {
            let p = sel.head;
            if p + 1 >= buf.len_chars() {
                // Cursor is on the structural trailing \n — no-op.
                b.retain(p - b.old_pos());
                new_sels.push(Selection::cursor(b.new_pos()));
                continue;
            }
            // Delete one grapheme cluster at `p`.
            let end = next_grapheme_boundary(&buf, p);
            b.retain(p - b.old_pos());
            b.delete(end - p);
            new_sels.push(Selection::cursor(b.new_pos()));
        } else {
            let start = sel.start();
            // end() is inclusive — +1 for the exclusive bound.
            let end_excl = sel.end() + 1;
            b.retain(start - b.old_pos());
            b.delete(end_excl - start);
            new_sels.push(Selection::cursor(b.new_pos()));
        }
    }

    b.retain_rest();
    let new_buf = b
        .finish()
        .apply(&buf)
        .expect("delete_selection: internal changeset is always valid");
    let new_sel_set = SelectionSet::from_vec(new_sels, primary_idx).merge_overlapping();
    new_sel_set.debug_assert_valid(new_buf.len_chars());
    (new_buf, new_sel_set)
}

/// Paste `values` after/onto each selection (normal-mode `p`).
///
/// **Cursor selections (`is_cursor()`):** insert `text` just after the cursor
/// character. The cursor lands on the last inserted character.
///
/// **Multi-char selections (`!is_cursor()`):** replace the selected region with
/// `text`. The displaced text is returned in the third tuple element so the
/// caller can write it back to the register — a swap. This eliminates the need
/// for a separate `R` keybind or Vim-style `"0` yank register.
///
/// **Multi-cursor semantics:**
/// - If `values.len() == sels.len()`: each selection gets its own slot (N-to-N).
/// - Otherwise: all `values` are joined (no separator) and used at every
///   selection (Helix fallback).
///
/// **Return value:** `(new_buf, new_sels, replaced)` where `replaced[i]` is the
/// text displaced by selection `i` — empty string for cursor selections.
///
/// An empty `values` slice is a no-op (returns the original state and an empty
/// `replaced` vec).
pub(crate) fn paste_after(
    buf: Buffer,
    sels: SelectionSet,
    values: &[String],
) -> (Buffer, SelectionSet, Vec<String>) {
    if values.is_empty() {
        return (buf, sels, Vec::new());
    }

    let n_sels = sels.len();
    let n_vals = values.len();

    // When counts mismatch, every selection gets the full joined content.
    // Compute once up front so the loop can borrow it as a plain &str.
    let joined: String = if n_sels != n_vals { values.join("") } else { String::new() };

    let mut b = ChangeSetBuilder::new(buf.len_chars());
    let mut new_sels: Vec<Selection> = Vec::with_capacity(n_sels);
    let mut replaced: Vec<String> = Vec::with_capacity(n_sels);
    let primary_idx = sels.primary_index();

    let last_char = buf.len_chars() - 1; // index of structural \n

    for (i, sel) in sels.iter_sorted().enumerate() {
        // N-to-N if counts match; full joined string otherwise.
        let text: &str = if n_sels == n_vals { &values[i] } else { &joined };

        if sel.is_cursor() {
            // Cursor: insert after this character. Nothing is replaced.
            replaced.push(String::new());
            // Clamped so we never push past the structural \n.
            let insert_at = (sel.end() + 1).min(last_char);
            b.retain(insert_at - b.old_pos());
            b.insert(text);
            // new_pos() is now just past the inserted text; -1 lands on last inserted char.
            new_sels.push(Selection::cursor(b.new_pos() - 1));
        } else {
            // Multi-char selection: replace the selected region.
            // Capture the old content so the caller can swap it into the register.
            let start = sel.start();
            let end_excl = sel.end() + 1;
            replaced.push(buf.slice(start..end_excl).to_string());
            b.retain(start - b.old_pos());
            b.delete(end_excl - start);
            b.insert(text);
            // Cursor lands on the last inserted character.
            new_sels.push(Selection::cursor(b.new_pos() - 1));
        }
    }

    b.retain_rest();
    let new_buf = b
        .finish()
        .apply(&buf)
        .expect("paste_after: internal changeset is always valid");
    let new_sel_set = SelectionSet::from_vec(new_sels, primary_idx).merge_overlapping();
    new_sel_set.debug_assert_valid(new_buf.len_chars());
    (new_buf, new_sel_set, replaced)
}

/// Paste `values` before/onto each selection (normal-mode `P`).
///
/// **Cursor selections (`is_cursor()`):** insert `text` just before the cursor
/// character. The cursor lands on the last inserted character.
///
/// **Multi-char selections (`!is_cursor()`):** same replace-and-swap semantics
/// as [`paste_after`] — the after/before distinction only applies to cursors.
/// When replacing, the selection is deleted and `text` is inserted in its place.
///
/// **Multi-cursor semantics:** identical to [`paste_after`].
///
/// **Return value:** `(new_buf, new_sels, replaced)` — same as [`paste_after`].
///
/// An empty `values` slice is a no-op.
pub(crate) fn paste_before(
    buf: Buffer,
    sels: SelectionSet,
    values: &[String],
) -> (Buffer, SelectionSet, Vec<String>) {
    if values.is_empty() {
        return (buf, sels, Vec::new());
    }

    let n_sels = sels.len();
    let n_vals = values.len();

    let joined: String = if n_sels != n_vals { values.join("") } else { String::new() };

    let mut b = ChangeSetBuilder::new(buf.len_chars());
    let mut new_sels: Vec<Selection> = Vec::with_capacity(n_sels);
    let mut replaced: Vec<String> = Vec::with_capacity(n_sels);
    let primary_idx = sels.primary_index();

    for (i, sel) in sels.iter_sorted().enumerate() {
        let text: &str = if n_sels == n_vals { &values[i] } else { &joined };

        if sel.is_cursor() {
            // Cursor: insert before this character. Nothing is replaced.
            replaced.push(String::new());
            let insert_at = sel.start();
            b.retain(insert_at - b.old_pos());
            b.insert(text);
            new_sels.push(Selection::cursor(b.new_pos() - 1));
        } else {
            // Multi-char selection: replace (same behaviour as paste_after).
            let start = sel.start();
            let end_excl = sel.end() + 1;
            replaced.push(buf.slice(start..end_excl).to_string());
            b.retain(start - b.old_pos());
            b.delete(end_excl - start);
            b.insert(text);
            new_sels.push(Selection::cursor(b.new_pos() - 1));
        }
    }

    b.retain_rest();
    let new_buf = b
        .finish()
        .apply(&buf)
        .expect("paste_before: internal changeset is always valid");
    let new_sel_set = SelectionSet::from_vec(new_sels, primary_idx).merge_overlapping();
    new_sel_set.debug_assert_valid(new_buf.len_chars());
    (new_buf, new_sel_set, replaced)
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
        assert_state!("-[h]>ello\n", |(buf, sels)| insert_char(buf, sels,'x'), "x-[h]>ello\n");
    }

    #[test]
    fn insert_char_at_cursor_middle() {
        // Cursor on second 'l' (offset 3); 'x' inserted, cursor on 'l'.
        assert_state!("hel-[l]>o\n", |(buf, sels)| insert_char(buf, sels,'x'), "helx-[l]>o\n");
    }

    #[test]
    fn insert_char_at_cursor_eof() {
        // Cursor at EOF (offset 5); 'x' appended; cursor at new EOF.
        assert_state!("hello-[\n]>", |(buf, sels)| insert_char(buf, sels,'x'), "hellox-[\n]>");
    }

    #[test]
    fn insert_char_into_empty_buffer() {
        assert_state!("-[\n]>", |(buf, sels)| insert_char(buf, sels,'x'), "x-[\n]>");
    }

    #[test]
    fn insert_char_replaces_forward_selection() {
        // Selection anchor=0, head=3 covers 'h','e','l','l' (4 chars).
        // Delete [0,4), insert 'x', cursor at 1.
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| insert_char(buf, sels,'x'),
            "x-[o]>\n"
        );
    }

    #[test]
    fn insert_char_replaces_whole_buffer() {
        assert_state!(
            "-[hello]>\n",
            |(buf, sels)| insert_char(buf, sels,'x'),
            "x-[\n]>"
        );
    }

    #[test]
    fn insert_char_replaces_backward_selection() {
        // anchor=3, head=0 covers chars 0-3 ('h','e','l','l') — "hell" (4 chars).
        // Delete [0,4), insert 'x' at 0, cursor at 1.
        // Buffer "hello" → remove "hell" → "o", insert 'x' → "xo".
        assert_state!(
            "<[hell]-o\n",
            |(buf, sels)| insert_char(buf, sels,'x'),
            "x-[o]>\n"
        );
    }

    #[test]
    fn insert_char_two_cursors() {
        // Cursors at 0 and 3. Insert 'x' at both positions.
        // Changeset: Insert("x"), Retain(3), Insert("x"), Retain(4).
        // Result: "xfoox bar", cursors at 1 and 5.
        assert_state!(
            "-[f]>oo-[ ]>bar\n",
            |(buf, sels)| insert_char(buf, sels,'x'),
            "x-[f]>oox-[ ]>bar\n"
        );
    }

    #[test]
    fn insert_char_unicode() {
        // Insert a multi-byte char (2 bytes in UTF-8, 1 char offset).
        assert_state!("caf-[é]>\n", |(buf, sels)| insert_char(buf, sels,'à'), "cafà-[é]>\n");
    }

    // ── delete_char_forward ───────────────────────────────────────────────────

    #[test]
    fn delete_forward_at_cursor_start() {
        // Cursor on 'h'; deletes 'h'; cursor stays at 0 (now on 'e').
        assert_state!("-[h]>ello\n", |(buf, sels)| delete_char_forward(buf, sels), "-[e]>llo\n");
    }

    #[test]
    fn delete_forward_at_cursor_middle() {
        assert_state!("h-[e]>llo\n", |(buf, sels)| delete_char_forward(buf, sels), "h-[l]>lo\n");
    }

    #[test]
    fn delete_forward_at_eof_is_noop() {
        assert_state!("hello-[\n]>", |(buf, sels)| delete_char_forward(buf, sels), "hello-[\n]>");
    }

    #[test]
    fn delete_forward_empty_buffer_is_noop() {
        assert_state!("-[\n]>", |(buf, sels)| delete_char_forward(buf, sels), "-[\n]>");
    }

    #[test]
    fn delete_forward_selection() {
        // Selection [0,3] inclusive → remove [0,4) → "o", cursor at 0.
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| delete_char_forward(buf, sels),
            "-[o]>\n"
        );
    }

    #[test]
    fn delete_forward_two_cursors() {
        // Cursors at 0 ('h') and 2 ('l'). Delete 'h' and first 'l'.
        // Changeset: Delete(1), Retain(1), Delete(1), Retain(2).
        // Result: "elo", cursors at 0 and 1.
        assert_state!(
            "-[h]>e-[l]>lo\n",
            |(buf, sels)| delete_char_forward(buf, sels),
            "-[e]>-[l]>o\n"
        );
    }

    #[test]
    fn delete_forward_adjacent_cursors_merge() {
        // Cursors at 2 and 3. Both delete forward; both land at 2 → merge.
        assert_state!(
            "he-[l]>-[l]>o\n",
            |(buf, sels)| delete_char_forward(buf, sels),
            "he-[o]>\n"
        );
    }

    #[test]
    fn delete_forward_grapheme_cluster() {
        // "e\u{0301}x": é is 2 chars, 1 grapheme. Cursor at 0 deletes whole cluster.
        assert_state!(
            "-[e\u{0301}]>x\n",
            |(buf, sels)| delete_char_forward(buf, sels),
            "-[x]>\n"
        );
    }

    // ── delete_char_backward ─────────────────────────────────────────────────

    #[test]
    fn delete_backward_at_cursor_end() {
        // Cursor at EOF (offset 5); backspace deletes 'o'; cursor at 4.
        assert_state!("hello-[\n]>", |(buf, sels)| delete_char_backward(buf, sels), "hell-[\n]>");
    }

    #[test]
    fn delete_backward_at_cursor_middle() {
        // Cursor at 3 ('l'); backspace deletes 'l' at 2; cursor at 2.
        assert_state!("hel-[l]>o\n", |(buf, sels)| delete_char_backward(buf, sels), "he-[l]>o\n");
    }

    #[test]
    fn delete_backward_at_start_is_noop() {
        assert_state!("-[h]>ello\n", |(buf, sels)| delete_char_backward(buf, sels), "-[h]>ello\n");
    }

    #[test]
    fn delete_backward_empty_buffer_is_noop() {
        assert_state!("-[\n]>", |(buf, sels)| delete_char_backward(buf, sels), "-[\n]>");
    }

    #[test]
    fn delete_backward_selection() {
        // Same as delete_forward for multi-char selections: removes selected region.
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| delete_char_backward(buf, sels),
            "-[o]>\n"
        );
    }

    #[test]
    fn delete_backward_two_cursors() {
        // Cursors at 2 and 4 in "hello". Backspace at 2 deletes 'e' (offset 1).
        // Backspace at 4 deletes 'l' (offset 3).
        // Changeset: Retain(1), Delete(1), Retain(1), Delete(1), Retain(1).
        // Result: "hlo", cursors at 1 and 2.
        assert_state!(
            "he-[l]>l-[o]>\n",
            |(buf, sels)| delete_char_backward(buf, sels),
            "h-[l]>-[o]>\n"
        );
    }

    #[test]
    fn delete_backward_grapheme_cluster() {
        // "e\u{0301}x": é is 2 chars (offsets 0-1). Cursor at 2 (on 'x').
        // prev_grapheme_boundary(2) = 0. Deletes entire é cluster.
        assert_state!(
            "e\u{0301}-[x]>\n",
            |(buf, sels)| delete_char_backward(buf, sels),
            "-[x]>\n"
        );
    }

    #[test]
    fn delete_backward_adjacent_cursors_merge() {
        // Cursors at 2 and 3. Backspace at 2: delete offset 1. Backspace at 3:
        // delete offset 2 in original. Both cursors land at 1 → merge.
        assert_state!(
            "he-[l]>-[l]>o\n",
            |(buf, sels)| delete_char_backward(buf, sels),
            "h-[l]>o\n"
        );
    }

    // ── delete_selection ──────────────────────────────────────────────────────

    #[test]
    fn delete_selection_cursor_deletes_char() {
        // Cursor on 'h' — deletes 'h'; cursor lands on 'e' (what was next).
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| delete_selection(buf, sels),
            "-[e]>llo\n"
        );
    }

    #[test]
    fn delete_selection_cursor_at_end_of_word() {
        // Cursor on 'o' (last word char) — deletes 'o'; cursor lands on '\n'.
        assert_state!(
            "hell-[o]>\n",
            |(buf, sels)| delete_selection(buf, sels),
            "hell-[\n]>"
        );
    }

    #[test]
    fn delete_selection_cursor_on_structural_newline_is_noop() {
        // Cursor on the trailing '\n' — buffer invariant, no-op.
        assert_state!(
            "hello-[\n]>",
            |(buf, sels)| delete_selection(buf, sels),
            "hello-[\n]>"
        );
    }

    #[test]
    fn delete_selection_empty_buffer_is_noop() {
        // Only the structural '\n' — cursor is on it, no-op.
        assert_state!(
            "-[\n]>",
            |(buf, sels)| delete_selection(buf, sels),
            "-[\n]>"
        );
    }

    #[test]
    fn delete_selection_multi_char_forward() {
        // Forward selection covering "hell" — cursor lands at start (pos 0).
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| delete_selection(buf, sels),
            "-[o]>\n"
        );
    }

    #[test]
    fn delete_selection_multi_char_backward() {
        // Backward selection — same result as forward; cursor lands at start.
        assert_state!(
            "<[hell]-o\n",
            |(buf, sels)| delete_selection(buf, sels),
            "-[o]>\n"
        );
    }

    #[test]
    fn delete_selection_two_cursors() {
        // Cursors on 'h' (pos 0) and 'l' (pos 2) — both deleted independently.
        assert_state!(
            "-[h]>el-[l]>o\n",
            |(buf, sels)| delete_selection(buf, sels),
            "-[e]>l-[o]>\n"
        );
    }

    #[test]
    fn delete_selection_adjacent_selections_merge_cursors() {
        // Cursors on 'h' (0) and 'e' (1) — after deleting both, cursors both
        // land at 0 and merge into one.
        assert_state!(
            "-[h]>-[e]>llo\n",
            |(buf, sels)| delete_selection(buf, sels),
            "-[l]>lo\n"
        );
    }

    #[test]
    fn delete_selection_grapheme_cluster() {
        // "e\u{0301}" is 2 chars (e + combining acute) but one grapheme cluster.
        // Cursor on 'e' (pos 0) deletes the entire cluster (both chars).
        assert_state!(
            "-[e]>\u{0301}x\n",
            |(buf, sels)| delete_selection(buf, sels),
            "-[x]>\n"
        );
    }

    // ── paste_after ───────────────────────────────────────────────────────────

    // Helper: call paste_after and discard the `replaced` vec for assert_state!.
    fn pa(buf: Buffer, sels: SelectionSet, values: &[String]) -> (Buffer, SelectionSet) {
        let (b, s, _) = paste_after(buf, sels, values);
        (b, s)
    }

    // Helper: call paste_before and discard the `replaced` vec for assert_state!.
    fn pb(buf: Buffer, sels: SelectionSet, values: &[String]) -> (Buffer, SelectionSet) {
        let (b, s, _) = paste_before(buf, sels, values);
        (b, s)
    }

    #[test]
    fn paste_after_single_cursor() {
        // Cursor on 'h' — insert "XY" after 'h'; cursor lands on 'Y'.
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
            "hX-[Y]>ello\n"
        );
    }

    #[test]
    fn paste_after_mid_word() {
        // Cursor on 'e' (pos 1) — insert "XY" after 'e'.
        assert_state!(
            "h-[e]>llo\n",
            |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
            "heX-[Y]>llo\n"
        );
    }

    #[test]
    fn paste_after_cursor_on_structural_newline() {
        // Cursor on the trailing '\n' — insertion is clamped to pos 5 (before '\n').
        // "hello\n" → "helloXY\n"; cursor lands on 'Y' (pos 6).
        assert_state!(
            "hello-[\n]>",
            |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
            "helloX-[Y]>\n"
        );
    }

    #[test]
    fn paste_after_two_cursors_n_to_n() {
        // Two cursors (pos 0 and 4); two values — each cursor gets its own slot.
        assert_state!(
            "-[h]>ell-[o]>\n",
            |(buf, sels)| pa(buf, sels, &["AB".to_string(), "CD".to_string()]),
            "hA-[B]>elloC-[D]>\n"
        );
    }

    #[test]
    fn paste_after_count_mismatch_uses_joined() {
        // 2 cursors, 1 value → both cursors get the full "XY".
        assert_state!(
            "-[h]>ell-[o]>\n",
            |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
            "hX-[Y]>elloX-[Y]>\n"
        );
    }

    #[test]
    fn paste_after_unicode() {
        // Paste a string with a combining character. Cursor lands on last char.
        assert_state!(
            "-[h]>i\n",
            |(buf, sels)| pa(buf, sels, &["e\u{0301}".to_string()]),
            "he-[\u{0301}]>i\n"
        );
    }

    #[test]
    fn paste_after_replaces_forward_selection() {
        // Multi-char selection "hel" is replaced by "XY". Cursor on 'Y'.
        // Replaced text "hel" is returned.
        assert_state!(
            "-[hel]>lo\n",
            |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
            "X-[Y]>lo\n"
        );
        let (buf, sels) = crate::testing::parse_state("-[hel]>lo\n");
        let (_, _, replaced) = paste_after(buf, sels, &["XY".to_string()]);
        assert_eq!(replaced, vec!["hel"]);
    }

    #[test]
    fn paste_after_replaces_backward_selection() {
        // Direction doesn't matter for replace — same result as forward.
        assert_state!(
            "<[hel]-lo\n",
            |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
            "X-[Y]>lo\n"
        );
        let (buf, sels) = crate::testing::parse_state("<[hel]-lo\n");
        let (_, _, replaced) = paste_after(buf, sels, &["XY".to_string()]);
        assert_eq!(replaced, vec!["hel"]);
    }

    #[test]
    fn paste_after_replace_swap_roundtrip() {
        // Yank "foo", paste onto selection "bar" → buffer has "foo", replaced = ["bar"].
        let (buf, sels) = crate::testing::parse_state("-[bar]>\n");
        let (new_buf, _, replaced) = paste_after(buf, sels, &["foo".to_string()]);
        assert_eq!(new_buf.to_string(), "foo\n");
        assert_eq!(replaced, vec!["bar"]);
    }

    #[test]
    fn paste_after_replace_multi_cursor_n_to_n() {
        // Two non-cursor selections; two values — each replaced independently.
        // "-[he]>l-[lo]>\n": "he" replaced by "AB", "lo" replaced by "CD".
        // Buffer: h(0)e(1)l(2)l(3)o(4)\n(5)
        // After: AB + l + CD + \n = "ABlCD\n"
        assert_state!(
            "-[he]>l-[lo]>\n",
            |(buf, sels)| pa(buf, sels, &["AB".to_string(), "CD".to_string()]),
            "A-[B]>lC-[D]>\n"
        );
        let (buf, sels) = crate::testing::parse_state("-[he]>l-[lo]>\n");
        let (_, _, replaced) = paste_after(buf, sels, &["AB".to_string(), "CD".to_string()]);
        assert_eq!(replaced, vec!["he", "lo"]);
    }

    #[test]
    fn paste_after_mixed_cursor_and_selection() {
        // One cursor (inserts) + one multi-char selection (replaces).
        // "-[h]>el-[lo]>\n": cursor at 'h' inserts "AB" after it; "lo" is replaced by "CD".
        // Buffer: h + AB + el + CD + \n = "hABelCD\n"
        // Cursors land on 'B' (pos 2) and 'D' (pos 6).
        assert_state!(
            "-[h]>el-[lo]>\n",
            |(buf, sels)| pa(buf, sels, &["AB".to_string(), "CD".to_string()]),
            "hA-[B]>elC-[D]>\n"
        );
        let (buf, sels) = crate::testing::parse_state("-[h]>el-[lo]>\n");
        let (_, _, replaced) = paste_after(buf, sels, &["AB".to_string(), "CD".to_string()]);
        // Cursor replaced nothing; selection replaced "lo".
        assert_eq!(replaced, vec!["", "lo"]);
    }

    // ── paste_before ──────────────────────────────────────────────────────────

    #[test]
    fn paste_before_single_cursor() {
        // Cursor on 'h' — insert "XY" before 'h'; cursor lands on 'Y'.
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| pb(buf, sels, &["XY".to_string()]),
            "X-[Y]>hello\n"
        );
    }

    #[test]
    fn paste_before_mid_word() {
        // Cursor on 'e' (pos 1) — insert "XY" before 'e'.
        assert_state!(
            "h-[e]>llo\n",
            |(buf, sels)| pb(buf, sels, &["XY".to_string()]),
            "hX-[Y]>ello\n"
        );
    }

    #[test]
    fn paste_before_two_cursors_n_to_n() {
        // Two cursors; two values — each cursor gets its own slot.
        // Buffer after: AB + hell + CD + o + \n
        assert_state!(
            "-[h]>ell-[o]>\n",
            |(buf, sels)| pb(buf, sels, &["AB".to_string(), "CD".to_string()]),
            "A-[B]>hellC-[D]>o\n"
        );
    }

    #[test]
    fn paste_before_count_mismatch_uses_joined() {
        // 2 cursors, 1 value → both cursors get the full "XY".
        assert_state!(
            "-[h]>ell-[o]>\n",
            |(buf, sels)| pb(buf, sels, &["XY".to_string()]),
            "X-[Y]>hellX-[Y]>o\n"
        );
    }

    #[test]
    fn paste_before_replaces_selection() {
        // Multi-char selection — paste_before also replaces (same as paste_after for selections).
        assert_state!(
            "-[hel]>lo\n",
            |(buf, sels)| pb(buf, sels, &["XY".to_string()]),
            "X-[Y]>lo\n"
        );
        let (buf, sels) = crate::testing::parse_state("-[hel]>lo\n");
        let (_, _, replaced) = paste_before(buf, sels, &["XY".to_string()]);
        assert_eq!(replaced, vec!["hel"]);
    }
}
