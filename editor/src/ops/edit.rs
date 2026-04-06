use crate::core::buffer::Buffer;
use crate::core::changeset::{ChangeSet, ChangeSetBuilder};
use crate::core::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use crate::core::selection::{Selection, SelectionSet};

// ── Edit scaffolding ──────────────────────────────────────────────────────────
//
// Every editing operation follows the same structural pattern:
//   1. Create a ChangeSetBuilder sized to the current buffer.
//   2. Walk selections in sorted order, executing per-selection logic.
//   3. Retain everything after the last selection (retain_rest).
//   4. Apply the changeset to produce the new buffer.
//   5. Assemble and merge the new SelectionSet.
//
// Rather than repeat this 5-step frame across every function, `apply_edit`
// extracts it and delegates the per-selection work to a closure. This is the
// standard higher-order-function pattern: the frame is the "algorithm", the
// closure is the "policy".
//
// `apply_edit_with_capture` is the core: it runs the 5-step frame and also
// collects per-selection output (e.g. yanked text for paste). `apply_edit` is
// a thin wrapper for the common case where captured output is not needed.
//
// The ChangeSet is returned so the undo system can call `cs.invert(&old_buf)`
// to produce the inverse transaction. The caller (Document) holds the pre-edit
// buffer and handles the invert timing constraint.

/// Apply a `(&Buffer, SelectionSet) -> SelectionSet` command `count` times.
///
#[allow(dead_code)]
/// This is the count mechanism for selection commands and other operations that
/// do not produce a ChangeSet. Use [`repeat_edit`] when the composed ChangeSet
/// is needed for undo/redo bookkeeping via [`crate::core::document::Document`].
///
/// For motions, count is handled inside `apply_motion` per-selection instead
/// (prevents premature merging of multi-cursor selections between steps).
pub(crate) fn repeat(
    count: usize,
    buf: &Buffer,
    sels: SelectionSet,
    cmd: impl Fn(&Buffer, SelectionSet) -> SelectionSet,
) -> SelectionSet {
    (0..count).fold(sels, |s, _| cmd(buf, s))
}

#[allow(dead_code)]
/// Apply an edit command `count` times, composing all changesets into one.
///
/// Like [`repeat`], but the command must return `(Buffer, SelectionSet,
/// ChangeSet)`. The N changesets are folded with [`ChangeSet::compose`] so the
/// whole repetition becomes a single undo step when passed to
/// [`crate::core::document::Document::apply_edit`].
///
/// If `count == 0`, returns the original state with an identity ChangeSet.
pub(crate) fn repeat_edit(
    count: usize,
    buf: Buffer,
    sels: SelectionSet,
    cmd: impl Fn(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet),
) -> (Buffer, SelectionSet, ChangeSet) {
    let mut current_buf = buf;
    let mut current_sels = sels;
    let mut composed: Option<ChangeSet> = None;

    for _ in 0..count {
        let (new_buf, new_sels, cs) = cmd(current_buf, current_sels);
        // ChangeSet::compose(A, B) produces A→C from A→B and B→C, combining
        // N individual edits into one for purposes of undo/redo granularity.
        composed = Some(match composed {
            None => cs,
            Some(prev) => prev.compose(cs),
        });
        current_buf = new_buf;
        current_sels = new_sels;
    }

    let cs = composed.unwrap_or_else(|| {
        // count == 0: produce an identity changeset (all Retain).
        let mut b = ChangeSetBuilder::new(current_buf.len_chars());
        b.retain_rest();
        b.finish()
    });
    (current_buf, current_sels, cs)
}

/// Core loop for editing operations that also capture per-selection output.
///
/// The closure `f` receives:
///   - `b`         — the changeset builder (original-buffer coordinate space)
///   - `buf`       — shared borrow of the original buffer for read-only queries
///   - `i`         — 0-based iteration index in sorted order (N-to-N paste uses this)
///   - `sel`       — the current selection
///   - `new_sels`  — accumulator for result selections; `f` must push exactly one entry
///   - `captured`  — accumulator for per-selection output (e.g. displaced text)
///
/// Returns the new buffer, merged selection set, changeset, and captured strings.
/// Use [`apply_edit`] when captured output is not needed.
///
/// # Why `FnMut` and not `Fn`?
///
/// Rust's closure traits form a hierarchy: `FnOnce ⊇ FnMut ⊇ Fn`.
/// `FnMut` means the closure may mutate its captured environment across calls,
/// which is the right default for a closure invoked in a loop. Even when the
/// closure only captures `Copy` values (like `char`), requiring `FnMut` keeps
/// the bound consistent and allows future closures to close over counters or
/// accumulators without changing the helper's signature.
pub(crate) fn apply_edit_with_capture<F>(
    buf: Buffer,
    sels: SelectionSet,
    mut f: F,
) -> (Buffer, SelectionSet, ChangeSet, Vec<String>)
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
    // Split finish() from apply() so the ChangeSet can be returned to the
    // caller for undo/redo bookkeeping. invert() must be called against the
    // pre-edit buffer — the caller (Document) holds that buffer and handles
    // the timing constraint.
    let cs = b.finish();
    let new_buf = cs
        .apply(&buf)
        .expect("edit operation produced an invalid changeset — this is a bug");
    let new_sel_set = SelectionSet::from_vec(new_sels, primary_idx).merge_overlapping();
    new_sel_set.debug_assert_valid(&new_buf);
    (new_buf, new_sel_set, cs, captured)
}

/// Convenience wrapper around [`apply_edit_with_capture`] for edits that
/// don't need to capture per-selection output.
pub(crate) fn apply_edit<F>(buf: Buffer, sels: SelectionSet, mut f: F) -> (Buffer, SelectionSet, ChangeSet)
where
    F: FnMut(&mut ChangeSetBuilder, &Buffer, usize, &Selection, &mut Vec<Selection>),
{
    let (new_buf, new_sels, cs, _) =
        apply_edit_with_capture(buf, sels, |b, buf, i, sel, new_sels, _captured| {
            f(b, buf, i, sel, new_sels);
        });
    (new_buf, new_sels, cs)
}

/// Delete the grapheme cluster at `p` and push a cursor result onto `new_sels`.
///
/// No-op when `p` is the last position in the buffer (the structural trailing
/// `\n`) — deleting it would violate the buffer invariant. Used by
/// `delete_char_forward` (cursor branch).
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
        let sel = Selection::collapsed(b.new_pos());
        new_sels.push(sel);
        return;
    }
    let end = next_grapheme_boundary(buf, p);
    b.retain(p - b.old_pos());
    b.delete(end - p);
    let sel = Selection::collapsed(b.new_pos());
    new_sels.push(sel);
}

/// Delete the entire region covered by `sel` and push a cursor at `start()`.
///
/// Uses `sel.end_inclusive()` so that multi-codepoint grapheme clusters
/// (e.g. `e + \u{0301}`) are deleted atomically. The deletion is capped at
/// the last content character (`buf.len_chars() - 2`) so that the structural
/// trailing `\n` is never removed — matching the protection in
/// `delete_one_grapheme`.
///
/// Shared by `delete_char_forward` and `delete_char_backward`, which have
/// identical selection branches.
fn delete_sel_region(
    b: &mut ChangeSetBuilder,
    buf: &Buffer,
    sel: &Selection,
    new_sels: &mut Vec<Selection>,
) {
    let start = sel.start();
    // Cap at the last content char so the structural trailing '\n' is never removed.
    let end_incl = sel.end_inclusive(buf).min(buf.last_content_char());
    b.retain(start - b.old_pos());
    b.delete(end_incl + 1 - start); // end_incl inclusive → +1 for exclusive bound
    let sel = Selection::collapsed(b.new_pos());
    new_sels.push(sel);
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
) -> (Buffer, SelectionSet, ChangeSet, Vec<String>)
where
    P: Fn(&Buffer, &Selection) -> usize,
{
    if values.is_empty() {
        // Nothing to paste — return an identity ChangeSet (all Retain).
        let mut b = ChangeSetBuilder::new(buf.len_chars());
        b.retain_rest();
        return (buf, sels, b.finish(), Vec::new());
    }

    let n_sels = sels.len();
    let n_vals = values.len();

    // When counts mismatch, every selection gets the full joined content.
    // Compute once up front so the closure can borrow it as `&str`.
    let joined: String = if n_sels != n_vals { values.join("") } else { String::new() };

    apply_edit_with_capture(buf, sels, |b, buf, i, sel, new_sels, replaced| {
        // N-to-N if counts match; every selection gets the full joined string otherwise.
        let text: &str = if n_sels == n_vals { &values[i] } else { &joined };

        if sel.is_collapsed() {
            replaced.push(String::new()); // cursors displace nothing
            let insert_at = cursor_insert_pos(buf, sel);
            b.retain(insert_at - b.old_pos());
            if text.is_empty() {
                // Nothing to insert — cursor stays where it is.
                new_sels.push(Selection::collapsed(sel.head));
            } else {
                b.insert(text);
                // new_pos() is one past the inserted text; -1 lands on the last
                // inserted character (the cursor sits on it — inclusive model).
                new_sels.push(Selection::collapsed(b.new_pos() - 1));
            }
        } else {
            // Multi-char selection: replace the selected region.
            // Cap end at the last content char so the structural trailing '\n'
            // is never deleted.
            let start = sel.start();
            let end_incl = sel.end_inclusive(buf).min(buf.last_content_char());
            let end_excl = end_incl + 1;
            replaced.push(buf.slice(start..end_excl).to_string());
            b.retain(start - b.old_pos());
            b.delete(end_excl - start);
            b.insert(text);
            // When text is empty the delete leaves new_pos() at the start of
            // the deleted region; -1 would underflow. Use saturating_sub so
            // the cursor lands at start (the first char after the deletion).
            let pos = if text.is_empty() { b.new_pos() } else { b.new_pos() - 1 };
            new_sels.push(Selection::collapsed(pos));
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
pub(crate) fn insert_char(buf: Buffer, sels: SelectionSet, ch: char) -> (Buffer, SelectionSet, ChangeSet) {
    apply_edit(buf, sels, |b, buf, _i, sel, new_sels| {
        let start = sel.start();
        b.retain(start - b.old_pos());
        if !sel.is_collapsed() {
            // Delete the selected region. Cap at the last content char to protect
            // the structural trailing '\n'.
            let end_incl = sel.end_inclusive(buf).min(buf.last_content_char());
            b.delete(end_incl + 1 - start);
        }
        b.insert_char(ch);
        // new_pos() is one past the inserted char — the cursor sits on the
        // character that was originally at `start` (now shifted right by 1).
        let sel = Selection::collapsed(b.new_pos());
        new_sels.push(sel);
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
) -> (Buffer, SelectionSet, ChangeSet) {
    apply_edit(buf, sels, |b, buf, _i, sel, new_sels| {
        if sel.is_collapsed() {
            delete_one_grapheme(b, buf, new_sels, sel.head);
        } else {
            delete_sel_region(b, buf, sel, new_sels);
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
) -> (Buffer, SelectionSet, ChangeSet) {
    apply_edit(buf, sels, |b, buf, _i, sel, new_sels| {
        if sel.is_collapsed() {
            let p = sel.head;
            if p == 0 {
                // At start of buffer — nothing to delete to the left.
                let sel = Selection::collapsed(b.new_pos());
                new_sels.push(sel);
                return;
            }
            // Delete the grapheme cluster ending just before `p`.
            let prev = prev_grapheme_boundary(buf, p);
            if prev < b.old_pos() {
                // A previous selection already consumed `prev` — the character
                // we'd delete is gone. Treat as a no-op; the cursor stays put.
                let sel = Selection::collapsed(b.new_pos());
                new_sels.push(sel);
                return;
            }
            b.retain(prev - b.old_pos());
            b.delete(p - prev);
            let sel = Selection::collapsed(b.new_pos());
            new_sels.push(sel);
        } else {
            delete_sel_region(b, buf, sel, new_sels);
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
/// ```text
/// let yanked = yank_selections(&buf, &sels);
/// let (new_buf, new_sels, _cs) = delete_selection(buf, sels);
/// registers.write(DEFAULT_REGISTER, yanked);
/// ```
pub(crate) fn delete_selection(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet, ChangeSet) {
    // Semantically, pressing `d` on a cursor deletes the char under it, and
    // pressing `d` on a selection deletes the selected region — exactly what
    // delete_char_forward does. There is no functional difference between the
    // two operations; the distinction is only in the key that triggered them.
    delete_char_forward(buf, sels)
}

/// Paste `values` after/onto each selection (normal-mode `p`).
///
/// **Cursor selections (`is_collapsed()`):** insert `text` just after the cursor
/// character. The cursor lands on the last inserted character.
///
/// **Multi-char selections (`!is_collapsed()`):** replace the selected region with
/// `text`. The displaced text is returned in the third tuple element so the
/// caller can write it back to the register — a swap. This eliminates the need
/// for a separate `R` keybind or Vim-style `"0` yank register.
///
/// **Multi-cursor semantics:**
/// - If `values.len() == sels.len()`: each selection gets its own slot (N-to-N).
/// - Otherwise: all `values` are joined (no separator) and used at every
///   selection (Helix fallback).
///
/// **Return value:** `(new_buf, new_sels, changeset, replaced)` where
/// `replaced[i]` is the text displaced by selection `i` — empty string for
/// cursor selections.
///
/// An empty `values` slice is a no-op (returns the original state and an empty
/// `replaced` vec).
pub(crate) fn paste_after(
    buf: Buffer,
    sels: SelectionSet,
    values: &[String],
) -> (Buffer, SelectionSet, ChangeSet, Vec<String>) {
    // `last_char` = index of the structural \n. Must be read before `buf` is
    // consumed by `paste_impl`, so we capture it here and move it into the closure.
    let last_char = buf.len_chars() - 1;
    paste_impl(buf, sels, values, move |buf, sel| (sel.end_inclusive(buf) + 1).min(last_char))
}

/// Paste `values` before/onto each selection (normal-mode `P`).
///
/// **Cursor selections (`is_collapsed()`):** insert `text` just before the cursor
/// character. The cursor lands on the last inserted character.
///
/// **Multi-char selections (`!is_collapsed()`):** same replace-and-swap semantics
/// as [`paste_after`] — the after/before distinction only applies to cursors.
/// When replacing, the selection is deleted and `text` is inserted in its place.
///
/// **Multi-cursor semantics:** identical to [`paste_after`].
///
/// **Return value:** `(new_buf, new_sels, changeset, replaced)` — same as [`paste_after`].
///
/// An empty `values` slice is a no-op.
pub(crate) fn paste_before(
    buf: Buffer,
    sels: SelectionSet,
    values: &[String],
) -> (Buffer, SelectionSet, ChangeSet, Vec<String>) {
    paste_impl(buf, sels, values, |_buf, sel| sel.start())
}

/// Replace every grapheme in every selection with `ch` (normal-mode `r`).
///
/// - **Cursor selection**: the single character under the cursor is replaced.
///   The cursor remains on the replacement character.
/// - **Multi-character selection**: every grapheme in the selected region is
///   replaced with `ch`, preserving the selection direction. Multi-codepoint
///   grapheme clusters (e.g. `é` = U+0065 + U+0301) are replaced atomically —
///   the replacement shrinks the cluster down to one char without orphaning
///   combining marks.
/// - **Newline skipping**: `\n` graphemes are never replaced — they are
///   retained as-is. This preserves line structure when the selection spans
///   multiple lines. The structural trailing `\n` is protected by the same
///   rule.
pub(crate) fn replace_selections(
    buf: Buffer,
    sels: SelectionSet,
    ch: char,
) -> (Buffer, SelectionSet, ChangeSet) {
    apply_edit(buf, sels, |b, buf, i, sel, new_sels| {
        let sel_start = sel.start();
        let sel_end   = sel.end(); // inclusive last-grapheme-start; equal to sel_start for cursor

        // Smart replace: when replacing a single character (cursor selection)
        // and the replacement is a pair character, resolve open/close based on
        // what's currently under the cursor.  See `surround::smart_replace_char`.
        let effective_ch = if sel.is_collapsed() {
            if let Some(current) = buf.char_at(sel_start) {
                crate::ops::surround::smart_replace_char(ch, current, i)
            } else {
                ch
            }
        } else {
            ch
        };

        // Retain everything up to this selection (handles the gap from the
        // previous selection or the buffer start). Record the start position
        // in result-buffer coordinates for later selection reconstruction.
        b.retain(sel_start - b.old_pos());
        let new_sel_start = b.new_pos();
        // The loop always executes at least once (pos starts at sel_start ≤ sel_end),
        // so new_sel_end is always overwritten before use. Rust cannot prove
        // the loop runs, so we initialise to new_sel_start as a safe sentinel.
        #[allow(unused_assignments)]
        let mut new_sel_end = new_sel_start;

        let mut pos = sel_start;
        loop {
            let next = next_grapheme_boundary(buf, pos);
            // `\n` graphemes are skipped (retained) to preserve line structure.
            // This also naturally protects the structural trailing '\n'.
            if buf.char_at(pos) == Some('\n') {
                b.retain(next - pos);
            } else {
                // After the initial `retain` above, b.old_pos() == sel_start == pos.
                // Each subsequent delete advances b.old_pos() by the cluster size,
                // landing exactly at the next grapheme start — so the builder stays
                // in sync without additional retain calls between graphemes.
                b.delete(next - pos);
                b.insert_char(effective_ch);
            }
            // Track the last position processed (whether replaced or retained)
            // so the reconstructed selection covers the full original range.
            new_sel_end = b.new_pos() - 1;

            if pos >= sel_end { break; }
            pos = next;
        }

        // Reconstruct the selection with its original direction.
        // `Selection::directed` is the canonical constructor for this pattern:
        // it takes content-aware (start, end) bounds and a direction flag.
        let forward = sel.anchor <= sel.head;
        new_sels.push(Selection::directed(new_sel_start, new_sel_end, forward));
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assert_state;
    use pretty_assertions::assert_eq;

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
    fn insert_char_replaces_selection_grapheme_base() {
        // Selection head lands on the base codepoint 'e' of {e\u{0301}} = é.
        // The fix extends the delete to include the combining mark, so typing
        // 'Z' fully replaces "café" rather than leaving an orphaned accent.
        // Buffer: "cafe\u{0301} x\n". Selection anchor=0, head=3 ('e').
        // Result: chars 0-4 deleted, 'Z' inserted → "Z x\n", cursor at 1 (' ').
        assert_state!(
            "-[cafe]>\u{0301} x\n",
            |(buf, sels)| insert_char(buf, sels, 'Z'),
            "Z-[ ]>x\n"
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

    #[test]
    fn delete_selection_multi_char_ends_at_grapheme_base() {
        // Multi-char selection whose head (sel.end()) lands on the base codepoint
        // 'e' of the grapheme {e\u{0301}} = é. The fix extends the delete to
        // include the combining mark at position 4, so no orphaned accent remains.
        // Buffer: "cafe\u{0301} x\n". Selection anchor=0, head=3 ('e').
        // Without the fix: only chars 0-3 deleted → "\u{0301} x\n" (broken).
        // With the fix: chars 0-4 deleted → " x\n" (correct).
        assert_state!(
            "-[cafe]>\u{0301} x\n",
            |(buf, sels)| delete_selection(buf, sels),
            "-[ ]>x\n"
        );
    }

    // ── paste_after ───────────────────────────────────────────────────────────

    // Helper: call paste_after and discard changeset + replaced vec for assert_state!.
    fn pa(buf: Buffer, sels: SelectionSet, values: &[String]) -> (Buffer, SelectionSet) {
        let (b, s, _, _) = paste_after(buf, sels, values);
        (b, s)
    }

    // Helper: call paste_before and discard changeset + replaced vec for assert_state!.
    fn pb(buf: Buffer, sels: SelectionSet, values: &[String]) -> (Buffer, SelectionSet) {
        let (b, s, _, _) = paste_before(buf, sels, values);
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
        let (_, _, _, replaced) = paste_after(buf, sels, &["XY".to_string()]);
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
        let (_, _, _, replaced) = paste_after(buf, sels, &["XY".to_string()]);
        assert_eq!(replaced, vec!["hel"]);
    }

    #[test]
    fn paste_after_replace_swap_roundtrip() {
        // Yank "foo", paste onto selection "bar" → buffer has "foo", replaced = ["bar"].
        let (buf, sels) = crate::testing::parse_state("-[bar]>\n");
        let (new_buf, _, _, replaced) = paste_after(buf, sels, &["foo".to_string()]);
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
        let (_, _, _, replaced) = paste_after(buf, sels, &["AB".to_string(), "CD".to_string()]);
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
        let (_, _, _, replaced) = paste_after(buf, sels, &["AB".to_string(), "CD".to_string()]);
        // Cursor replaced nothing; selection replaced "lo".
        assert_eq!(replaced, vec!["", "lo"]);
    }

    #[test]
    fn paste_after_empty_string_cursor_is_noop() {
        // B4 regression: b.new_pos() - 1 underflows when text is "".
        // For a cursor selection with empty text, buffer and cursor must be unchanged.
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| {
                let (b, s, _, _) = paste_after(buf, sels, &["".to_string()]);
                (b, s)
            },
            "-[h]>ello\n"
        );
    }

    #[test]
    fn paste_after_empty_string_over_selection_deletes_and_lands_at_start() {
        // Empty text with a multi-char selection: the selection is deleted,
        // cursor lands at the start of the deleted region (not new_pos() - 1).
        assert_state!(
            "-[hel]>lo\n",
            |(buf, sels)| {
                let (b, s, _, _) = paste_after(buf, sels, &["".to_string()]);
                (b, s)
            },
            "-[l]>o\n"
        );
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
        let (_, _, _, replaced) = paste_before(buf, sels, &["XY".to_string()]);
        assert_eq!(replaced, vec!["hel"]);
    }

    // ── paste empty-values (no-op path) ──────────────────────────────────────

    #[test]
    fn paste_after_empty_values_is_noop() {
        let (buf, sels) = crate::testing::parse_state("-[h]>ello\n");
        let buf_str = buf.to_string();
        let (new_buf, new_sels, _, replaced) = paste_after(buf, sels.clone(), &[]);
        assert_eq!(new_buf.to_string(), buf_str);
        assert_eq!(new_sels, sels);
        assert!(replaced.is_empty());
    }

    #[test]
    fn paste_before_empty_values_is_noop() {
        let (buf, sels) = crate::testing::parse_state("-[h]>ello\n");
        let buf_str = buf.to_string();
        let (new_buf, new_sels, _, replaced) = paste_before(buf, sels.clone(), &[]);
        assert_eq!(new_buf.to_string(), buf_str);
        assert_eq!(new_sels, sels);
        assert!(replaced.is_empty());
    }

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

    // ── insert_char edge cases ────────────────────────────────────────────────

    #[test]
    fn insert_char_newline() {
        // Inserting '\n' is mechanically identical to any other char: it goes
        // before the cursor character, cursor stays on the original char (now shifted).
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| insert_char(buf, sels, '\n'),
            "\n-[h]>ello\n"
        );
    }

    #[test]
    fn insert_char_combining_codepoint() {
        // Inserting a bare combining accent (U+0301) before 'h'. Mechanically
        // fine — the accent is stored as its own codepoint at position 0, and
        // the cursor lands on 'h' (now at position 1).
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| insert_char(buf, sels, '\u{0301}'),
            "\u{0301}-[h]>ello\n"
        );
    }

    // ── paste with multiline text ─────────────────────────────────────────────

    #[test]
    fn paste_after_multiline_text() {
        // Paste "foo\nbar" after 'h'. Buffer: "h" + "foo\nbar" + "ello\n".
        // Cursor lands on the last pasted char 'r'(7).
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| {
                let (b, s, cs, _) = paste_after(buf, sels, &["foo\nbar".to_string()]);
                (b, s, cs)
            },
            "hfoo\nba-[r]>ello\n"
        );
    }

    #[test]
    fn paste_before_multiline_text() {
        // Paste "foo\nbar" before 'h'. Buffer: "foo\nbar" + "hello\n".
        // Cursor lands on the last pasted char 'r'(6).
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| {
                let (b, s, cs, _) = paste_before(buf, sels, &["foo\nbar".to_string()]);
                (b, s, cs)
            },
            "foo\nba-[r]>hello\n"
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

    // ── yank → paste round-trip ───────────────────────────────────────────────

    #[test]
    fn yank_then_paste_after_round_trip() {
        use crate::ops::register::yank_selections;
        // Yank "ello" from selection, then paste it after the cursor.
        // Initial: cursor on 'h', selection covers "ello".
        // After yank: yanked = ["ello"]
        // After paste_after: "h" + "ello" + "\n" — cursor on last pasted 'o'.
        let (buf, sels) = crate::testing::parse_state("-[h]>ello\n");
        let yanked = yank_selections(&buf, &sels);
        assert_eq!(yanked, vec!["h"], "yank captures the cursor char");

        // Now paste the yanked text after the cursor (which is on 'h').
        // paste_after inserts "h" after 'h': "hh|ello\n"
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| {
                let values = yank_selections(&buf, &sels);
                let (b, s, cs, _) = paste_after(buf, sels, &values);
                (b, s, cs)
            },
            "h-[h]>ello\n"
        );
    }

    #[test]
    fn yank_multi_cursor_then_paste_after_n_to_n() {
        use crate::ops::register::yank_selections;
        // Two cursors: one on 'h', one on 'o'. Yank both, paste after each.
        // Expected yanked: ["h", "o"]
        // After paste: "hh" at pos 0-1, "oo" at pos 4-5 (with shift).
        let (buf, sels) = crate::testing::parse_state("-[h]>ell-[o]>\n");
        let yanked = yank_selections(&buf, &sels);
        assert_eq!(yanked, vec!["h", "o"]);

        assert_state!(
            "-[h]>ell-[o]>\n",
            |(buf, sels)| {
                let values = yank_selections(&buf, &sels);
                let (b, s, cs, _) = paste_after(buf, sels, &values);
                (b, s, cs)
            },
            "h-[h]>ello-[o]>\n"
        );
    }

    // ── replace_selections ────────────────────────────────────────────────────

    #[test]
    fn replace_cursor_single_char() {
        // Cursor on 'h'; replace with 'x' → cursor stays on 'x'.
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| replace_selections(buf, sels, 'x'),
            "-[x]>ello\n"
        );
    }

    #[test]
    fn replace_cursor_middle() {
        // Cursor on 'l' at offset 2; replace with 'x'.
        assert_state!(
            "he-[l]>lo\n",
            |(buf, sels)| replace_selections(buf, sels, 'x'),
            "he-[x]>lo\n"
        );
    }

    #[test]
    fn replace_cursor_on_structural_newline_is_noop() {
        // Structural trailing '\n' is skipped like any other '\n'.
        assert_state!(
            "hello-[\n]>",
            |(buf, sels)| replace_selections(buf, sels, 'x'),
            "hello-[\n]>"
        );
    }

    #[test]
    fn replace_cursor_on_mid_buffer_newline_is_noop() {
        // Cursor on the '\n' between two lines — preserved, not replaced.
        assert_state!(
            "hello-[\n]>world\n",
            |(buf, sels)| replace_selections(buf, sels, 'x'),
            "hello-[\n]>world\n"
        );
    }

    #[test]
    fn replace_empty_buffer_is_noop() {
        // Buffer is just the structural '\n'.
        assert_state!(
            "-[\n]>",
            |(buf, sels)| replace_selections(buf, sels, 'x'),
            "-[\n]>"
        );
    }

    #[test]
    fn replace_forward_selection() {
        // Forward selection covers "hell" (offsets 0-3); replace each with 'x'.
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| replace_selections(buf, sels, 'x'),
            "-[xxxx]>o\n"
        );
    }

    #[test]
    fn replace_backward_selection() {
        // Backward selection anchor=3, head=0 covers "hell"; direction preserved.
        assert_state!(
            "<[hell]-o\n",
            |(buf, sels)| replace_selections(buf, sels, 'x'),
            "<[xxxx]-o\n"
        );
    }

    #[test]
    fn replace_whole_line() {
        // Forward selection covers all content chars (not the structural '\n').
        assert_state!(
            "-[hello]>\n",
            |(buf, sels)| replace_selections(buf, sels, 'x'),
            "-[xxxxx]>\n"
        );
    }

    #[test]
    fn replace_two_cursors() {
        // Two cursors; each independently replaced.
        assert_state!(
            "-[h]>ell-[o]>\n",
            |(buf, sels)| replace_selections(buf, sels, 'x'),
            "-[x]>ell-[x]>\n"
        );
    }

    #[test]
    fn replace_two_selections() {
        // Two non-overlapping selections each get all their chars replaced.
        assert_state!(
            "-[he]>l-[lo]>\n",
            |(buf, sels)| replace_selections(buf, sels, 'x'),
            "-[xx]>l-[xx]>\n"
        );
    }

    #[test]
    fn replace_grapheme_cluster_cursor() {
        // Cursor on 'é' (e + U+0301, 2 codepoints). Replaced with 'x' (1 codepoint).
        // Buffer shrinks by 1 char; cursor lands on 'x'.
        assert_state!(
            "caf-[e]>\u{0301}z\n",
            |(buf, sels)| replace_selections(buf, sels, 'x'),
            "caf-[x]>z\n"
        );
    }

    #[test]
    fn replace_multiline_selection_skips_newline() {
        // Selection spans two lines. The '\n' between them is retained;
        // only the visible characters are replaced. Lines stay separate.
        assert_state!(
            "-[hello\nworld]>\n",
            |(buf, sels)| replace_selections(buf, sels, 'x'),
            "-[xxxxx\nxxxxx]>\n"
        );
    }

    #[test]
    fn replace_selection_including_structural_trailing_newline_preserves_newline() {
        // When the selection reaches the structural trailing '\n', that newline
        // must be preserved — replace_selections skips '\n' graphemes entirely.
        // Before the fix this path existed but had no explicit test.
        assert_state!(
            "-[hello\n]>",
            |(buf, sels)| replace_selections(buf, sels, 'x'),
            "-[xxxxx\n]>"
        );
    }

    // ── Smart replace (pair-aware) ───────────────────────────────────────────

    #[test]
    fn smart_replace_opening_bracket_to_opening() {
        // Two cursors on `(` and `)`, replace with `[` → `[` and `]`.
        assert_state!(
            "-[(]>hello-[)]>\n",
            |(buf, sels)| replace_selections(buf, sels, '['),
            "-[[]>hello-[]]>\n"
        );
    }

    #[test]
    fn smart_replace_asym_to_sym() {
        // `(` and `)` replaced with `"` → both become `"`.
        assert_state!(
            "-[(]>hello-[)]>\n",
            |(buf, sels)| replace_selections(buf, sels, '"'),
            "-[\"]>hello-[\"]>\n"
        );
    }

    #[test]
    fn smart_replace_sym_to_asym_uses_index() {
        // Two cursors on `"` and `"`, replace with `(` → `(` and `)`.
        assert_state!(
            "-[\"]>hello-[\"]>\n",
            |(buf, sels)| replace_selections(buf, sels, '('),
            "-[(]>hello-[)]>\n"
        );
    }

    #[test]
    fn smart_replace_sym_to_sym() {
        // Two cursors on `"` and `"`, replace with `'` → both `'`.
        assert_state!(
            "-[\"]>hello-[\"]>\n",
            |(buf, sels)| replace_selections(buf, sels, '\''),
            "-[']>hello-[']>\n"
        );
    }

    #[test]
    fn smart_replace_non_delimiter_is_literal() {
        // Cursor on `x`, replace with `[` → literal `[` (no smart logic).
        assert_state!(
            "-[x]>hello\n",
            |(buf, sels)| replace_selections(buf, sels, '['),
            "-[[]>hello\n"
        );
    }

    #[test]
    fn smart_replace_range_selection_no_smart_logic() {
        // Range selection (not a cursor) — all chars become `[`, no smart logic.
        assert_state!(
            "-[(he]>llo)\n",
            |(buf, sels)| replace_selections(buf, sels, '['),
            "-[[[[]>llo)\n"
        );
    }

    #[test]
    fn smart_replace_non_pair_replacement_is_literal() {
        // Replacement is not a pair char — always literal, even on delimiters.
        assert_state!(
            "-[(]>hello-[)]>\n",
            |(buf, sels)| replace_selections(buf, sels, 'x'),
            "-[x]>hello-[x]>\n"
        );
    }
}
