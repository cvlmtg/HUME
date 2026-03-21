use crate::buffer::Buffer;
use crate::changeset::ChangeSetBuilder;
use crate::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use crate::selection::{Selection, SelectionSet};

// ── Edit scaffolding ──────────────────────────────────────────────────────────
//
// Every editing operation follows the same structural pattern:
//   1. Create a ChangeSetBuilder sized to the current buffer.
//   2. Walk selections in sorted order, executing per-selection logic.
//   3. Retain everything after the last selection (retain_rest).
//   4. Apply the changeset to produce the new buffer.
//   5. Assemble and merge the new SelectionSet.
//
// Rather than repeat this 7-line frame across every function, `apply_edit`
// extracts it and delegates the per-selection work to a closure. This is the
// standard higher-order-function pattern: the frame is the "algorithm", the
// closure is the "policy".
//
// Two variants exist because paste operations return extra captured data:
//   • apply_edit              → (Buffer, SelectionSet)
//   • apply_edit_with_capture → (Buffer, SelectionSet, Vec<String>)

/// Core loop for editing operations that produce `(Buffer, SelectionSet)`.
///
/// The closure `f` receives:
///   - `b`         — the changeset builder (original-buffer coordinate space)
///   - `buf`       — shared borrow of the original buffer for read-only queries
///   - `i`         — 0-based iteration index in sorted order (N-to-N paste uses this)
///   - `sel`       — the current selection
///   - `new_sels`  — accumulator for result selections; `f` must push exactly one entry
///
/// # Why `FnMut` and not `Fn`?
///
/// Rust's closure traits form a hierarchy: `FnOnce ⊇ FnMut ⊇ Fn`.
/// `FnMut` means the closure may mutate its captured environment across calls,
/// which is the right default for a closure invoked in a loop. Even when the
/// closure only captures `Copy` values (like `char`), requiring `FnMut` keeps
/// the bound consistent and allows future closures to close over counters or
/// accumulators without changing the helper's signature.
fn apply_edit<F>(buf: Buffer, sels: SelectionSet, mut f: F) -> (Buffer, SelectionSet)
where
    F: FnMut(&mut ChangeSetBuilder, &Buffer, usize, &Selection, &mut Vec<Selection>),
{
    let mut b = ChangeSetBuilder::new(buf.len_chars());
    let mut new_sels = Vec::with_capacity(sels.len());
    let primary_idx = sels.primary_index();

    for (i, sel) in sels.iter_sorted().enumerate() {
        f(&mut b, &buf, i, sel, &mut new_sels);
    }

    b.retain_rest();
    let new_buf = b
        .finish()
        .apply(&buf)
        .expect("edit operation produced an invalid changeset — this is a bug");
    let new_sel_set = SelectionSet::from_vec(new_sels, primary_idx).merge_overlapping();
    new_sel_set.debug_assert_valid(new_buf.len_chars());
    (new_buf, new_sel_set)
}

/// Core loop for editing operations that also capture per-selection output.
///
/// Identical to [`apply_edit`] except the closure receives an extra
/// `&mut Vec<String>` accumulator (`captured`) and the return type includes
/// the captured vec as a third element. Used by paste operations that return
/// the text they displaced.
fn apply_edit_with_capture<F>(
    buf: Buffer,
    sels: SelectionSet,
    mut f: F,
) -> (Buffer, SelectionSet, Vec<String>)
where
    F: FnMut(&mut ChangeSetBuilder, &Buffer, usize, &Selection, &mut Vec<Selection>, &mut Vec<String>),
{
    let mut b = ChangeSetBuilder::new(buf.len_chars());
    let mut new_sels = Vec::with_capacity(sels.len());
    let mut captured = Vec::with_capacity(sels.len());
    let primary_idx = sels.primary_index();

    for (i, sel) in sels.iter_sorted().enumerate() {
        f(&mut b, &buf, i, sel, &mut new_sels, &mut captured);
    }

    b.retain_rest();
    let new_buf = b
        .finish()
        .apply(&buf)
        .expect("edit operation produced an invalid changeset — this is a bug");
    let new_sel_set = SelectionSet::from_vec(new_sels, primary_idx).merge_overlapping();
    new_sel_set.debug_assert_valid(new_buf.len_chars());
    (new_buf, new_sel_set, captured)
}

/// Delete the grapheme cluster at `p` and push a cursor result onto `new_sels`.
///
/// No-op when `p` is the last position in the buffer (the structural trailing
/// `\n`) — deleting it would violate the buffer invariant. Used by both
/// `delete_char_forward` and `delete_selection`, whose cursor branches would
/// otherwise be character-for-character identical.
///
/// All offsets fed to `b` are in original-buffer coordinate space — the builder
/// translates them to result-buffer positions internally.
fn delete_one_grapheme(
    b: &mut ChangeSetBuilder,
    buf: &Buffer,
    new_sels: &mut Vec<Selection>,
    p: usize,
) {
    if p + 1 >= buf.len_chars() {
        // Cursor is on the structural trailing '\n' — cannot delete it.
        b.retain(p - b.old_pos());
        new_sels.push(Selection::cursor(b.new_pos()));
        return;
    }
    let end = next_grapheme_boundary(buf, p);
    b.retain(p - b.old_pos());
    b.delete(end - p);
    new_sels.push(Selection::cursor(b.new_pos()));
}

/// Delete the entire region covered by `sel` and push a cursor at `start()`.
///
/// `sel.end()` is inclusive, so the exclusive bound is `sel.end() + 1`.
/// Shared by `delete_char_forward` and `delete_char_backward`, which have
/// identical selection branches.
fn delete_sel_region(
    b: &mut ChangeSetBuilder,
    sel: &Selection,
    new_sels: &mut Vec<Selection>,
) {
    let start = sel.start();
    b.retain(start - b.old_pos());
    b.delete(sel.end() + 1 - start); // end() inclusive → +1 for exclusive bound
    new_sels.push(Selection::cursor(b.new_pos()));
}

/// Private implementation shared by [`paste_after`] and [`paste_before`].
///
/// The two public functions differ only in where a cursor selection inserts its
/// text — `paste_after` uses `(sel.end() + 1).min(last_char)`, `paste_before`
/// uses `sel.start()`. Passing that calculation as a closure (`cursor_insert_pos`)
/// captures the difference in one parameter and eliminates the duplication.
///
/// # Why `Fn` (not `FnMut`) for `cursor_insert_pos`?
///
/// The position closure is a pure calculation — it reads `buf` and `sel` but
/// never mutates anything. `Fn` names that contract precisely. `FnMut` would
/// also accept `Fn` closures (they are a strict subset), but using the weakest
/// sufficient bound makes the intent clearer to readers.
fn paste_impl<P>(
    buf: Buffer,
    sels: SelectionSet,
    values: &[String],
    cursor_insert_pos: P,
) -> (Buffer, SelectionSet, Vec<String>)
where
    P: Fn(&Buffer, &Selection) -> usize,
{
    if values.is_empty() {
        return (buf, sels, Vec::new());
    }

    let n_sels = sels.len();
    let n_vals = values.len();

    // When counts mismatch, every selection gets the full joined content.
    // Compute once up front so the closure can borrow it as `&str`.
    let joined: String = if n_sels != n_vals { values.join("") } else { String::new() };

    apply_edit_with_capture(buf, sels, |b, buf, i, sel, new_sels, replaced| {
        // N-to-N if counts match; every selection gets the full joined string otherwise.
        let text: &str = if n_sels == n_vals { &values[i] } else { &joined };

        if sel.is_cursor() {
            replaced.push(String::new()); // cursors displace nothing
            let insert_at = cursor_insert_pos(buf, sel);
            b.retain(insert_at - b.old_pos());
            b.insert(text);
            // new_pos() is one past the inserted text; -1 lands on the last
            // inserted character (the cursor sits on it — inclusive model).
            new_sels.push(Selection::cursor(b.new_pos() - 1));
        } else {
            // Multi-char selection: replace the selected region.
            // Capture the old content before the builder advances past it.
            let start = sel.start();
            let end_excl = sel.end() + 1;
            replaced.push(buf.slice(start..end_excl).to_string());
            b.retain(start - b.old_pos());
            b.delete(end_excl - start);
            b.insert(text);
            new_sels.push(Selection::cursor(b.new_pos() - 1));
        }
    })
}

// ── Public operations ─────────────────────────────────────────────────────────
//
// Each operation builds a ChangeSet via the builder, working entirely in
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
    apply_edit(buf, sels, |b, _buf, _i, sel, new_sels| {
        let start = sel.start();
        b.retain(start - b.old_pos());
        if !sel.is_cursor() {
            // Delete the selected region. end() is inclusive, so +1 for the
            // exclusive bound that the builder expects.
            b.delete(sel.end() + 1 - start);
        }
        b.insert_char(ch);
        // new_pos() is one past the inserted char — the cursor sits on the
        // character that was originally at `start` (now shifted right by 1).
        new_sels.push(Selection::cursor(b.new_pos()));
    })
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
    apply_edit(buf, sels, |b, buf, _i, sel, new_sels| {
        if sel.is_cursor() {
            delete_one_grapheme(b, buf, new_sels, sel.head);
        } else {
            delete_sel_region(b, sel, new_sels);
        }
    })
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
    apply_edit(buf, sels, |b, buf, _i, sel, new_sels| {
        if sel.is_cursor() {
            let p = sel.head;
            if p == 0 {
                // At start of buffer — nothing to delete to the left.
                new_sels.push(Selection::cursor(b.new_pos()));
                return;
            }
            // Delete the grapheme cluster ending just before `p`.
            let prev = prev_grapheme_boundary(buf, p);
            b.retain(prev - b.old_pos());
            b.delete(p - prev);
            new_sels.push(Selection::cursor(b.new_pos()));
        } else {
            delete_sel_region(b, sel, new_sels);
        }
    })
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
    // Semantically, pressing `d` on a cursor deletes the char under it, and
    // pressing `d` on a selection deletes the selected region — exactly what
    // delete_char_forward does. There is no functional difference between the
    // two operations; the distinction is only in the key that triggered them.
    delete_char_forward(buf, sels)
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
    // `last_char` = index of the structural \n. Must be read before `buf` is
    // consumed by `paste_impl`, so we capture it here and move it into the closure.
    let last_char = buf.len_chars() - 1;
    paste_impl(buf, sels, values, move |_buf, sel| (sel.end() + 1).min(last_char))
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
    paste_impl(buf, sels, values, |_buf, sel| sel.start())
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
