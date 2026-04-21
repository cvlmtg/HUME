use crate::core::text::Text;
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

/// Apply a `(&Text, SelectionSet) -> SelectionSet` command `count` times.
///
#[allow(dead_code)]
/// This is the count mechanism for selection commands and other operations that
/// do not produce a ChangeSet. Use [`repeat_edit`] when the composed ChangeSet
/// is needed for undo/redo bookkeeping via [`crate::editor::buffer::Buffer`].
///
/// For motions, count is handled inside `apply_motion` per-selection instead
/// (prevents premature merging of multi-cursor selections between steps).
pub(crate) fn repeat(
    count: usize,
    buf: &Text,
    sels: SelectionSet,
    cmd: impl Fn(&Text, SelectionSet) -> SelectionSet,
) -> SelectionSet {
    (0..count).fold(sels, |s, _| cmd(buf, s))
}

#[allow(dead_code)]
/// Apply an edit command `count` times, composing all changesets into one.
///
/// Like [`repeat`], but the command must return `(Text, SelectionSet,
/// ChangeSet)`. The N changesets are folded with [`ChangeSet::compose`] so the
/// whole repetition becomes a single undo step when passed to
/// [`crate::editor::buffer::Buffer::apply_edit`].
///
/// If `count == 0`, returns the original state with an identity ChangeSet.
pub(crate) fn repeat_edit(
    count: usize,
    buf: Text,
    sels: SelectionSet,
    cmd: impl Fn(Text, SelectionSet) -> (Text, SelectionSet, ChangeSet),
) -> (Text, SelectionSet, ChangeSet) {
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
    buf: Text,
    sels: SelectionSet,
    mut f: F,
) -> (Text, SelectionSet, ChangeSet, Vec<String>)
where
    F: FnMut(&mut ChangeSetBuilder, &Text, usize, &Selection, &mut Vec<Selection>, &mut Vec<String>),
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
pub(crate) fn apply_edit<F>(buf: Text, sels: SelectionSet, mut f: F) -> (Text, SelectionSet, ChangeSet)
where
    F: FnMut(&mut ChangeSetBuilder, &Text, usize, &Selection, &mut Vec<Selection>),
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
    buf: &Text,
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
    buf: &Text,
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
    buf: Text,
    sels: SelectionSet,
    values: &[String],
    cursor_insert_pos: P,
) -> (Text, SelectionSet, ChangeSet, Vec<String>)
where
    P: Fn(&Text, &Selection) -> usize,
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
pub(crate) fn insert_char(buf: Text, sels: SelectionSet, ch: char) -> (Text, SelectionSet, ChangeSet) {
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
    buf: Text,
    sels: SelectionSet,
) -> (Text, SelectionSet, ChangeSet) {
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
    buf: Text,
    sels: SelectionSet,
) -> (Text, SelectionSet, ChangeSet) {
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
pub(crate) fn delete_selection(buf: Text, sels: SelectionSet) -> (Text, SelectionSet, ChangeSet) {
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
    buf: Text,
    sels: SelectionSet,
    values: &[String],
) -> (Text, SelectionSet, ChangeSet, Vec<String>) {
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
    buf: Text,
    sels: SelectionSet,
    values: &[String],
) -> (Text, SelectionSet, ChangeSet, Vec<String>) {
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
    buf: Text,
    sels: SelectionSet,
    ch: char,
) -> (Text, SelectionSet, ChangeSet) {
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


#[cfg(test)]
mod tests;
