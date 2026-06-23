use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use hume_editing::lines::{is_line_start, line_end_exclusive};
use hume_editing::selection::{Selection, SelectionSet, is_selection_linewise};
use hume_editing::text::Text;

use crate::ops::register;

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
// The ChangeSet is returned so the undo system can call `cs.invert(&old_buf)`
// to produce the inverse transaction. The caller (Document) holds the pre-edit
// buffer and handles the invert timing constraint.

/// Apply a `(&Text, SelectionSet) -> SelectionSet` command `count` times.
///
/// This is the count mechanism for selection commands and other operations that
/// do not produce a ChangeSet. Use [`repeat_edit`] when the composed ChangeSet
/// is needed for undo/redo bookkeeping via [`crate::editor::buffer::Buffer`].
///
/// For motions, count is handled inside `apply_motion` per-selection instead
/// (prevents premature merging of multi-cursor selections between steps).
#[cfg(test)]
pub(crate) fn repeat(
    count: usize,
    buf: &Text,
    sels: SelectionSet,
    cmd: impl Fn(&Text, SelectionSet) -> SelectionSet,
) -> SelectionSet {
    (0..count).fold(sels, |s, _| cmd(buf, s))
}

/// Apply an edit command `count` times, composing all changesets into one.
///
/// Like [`repeat`], but the command must return `(Text, SelectionSet,
/// ChangeSet)`. The N changesets are folded with [`ChangeSet::compose`] so the
/// whole repetition becomes a single undo step when passed to
/// [`crate::editor::buffer::Buffer::apply_edit`].
///
/// If `count == 0`, returns the original state with an identity ChangeSet.
#[cfg(test)]
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

/// Core loop for all editing operations.
///
/// The closure `f` receives:
///   - `b`        — the changeset builder (original-buffer coordinate space)
///   - `buf`      — shared borrow of the original buffer for read-only queries
///   - `i`        — 0-based iteration index in sorted order (N-to-N paste uses this)
///   - `sel`      — the current selection
///   - `new_sels` — accumulator for result selections; `f` must push exactly one entry
///
/// Returns the new buffer, merged selection set, and changeset.
///
/// # Why `FnMut` and not `Fn`?
///
/// Rust's closure traits form a hierarchy: `FnOnce ⊇ FnMut ⊇ Fn`.
/// `FnMut` means the closure may mutate its captured environment across calls,
/// which is the right default for a closure invoked in a loop. Even when the
/// closure only captures `Copy` values (like `char`), requiring `FnMut` keeps
/// the bound consistent and allows future closures to close over counters or
/// accumulators without changing the helper's signature.
pub(crate) fn apply_edit<F>(
    buf: Text,
    sels: SelectionSet,
    mut f: F,
) -> (Text, SelectionSet, ChangeSet)
where
    F: FnMut(&mut ChangeSetBuilder, &Text, usize, &Selection, &mut Vec<Selection>),
{
    let mut b = ChangeSetBuilder::new(buf.len_chars());
    let mut new_sels = Vec::with_capacity(sels.len());
    let primary_idx = sels.primary_index();

    for (i, sel) in sels.iter_sorted().enumerate() {
        f(&mut b, &buf, i, sel, &mut new_sels);
    }

    b.retain_rest();
    // finish() before apply() so the ChangeSet is available for undo/redo
    // bookkeeping. invert() must be called against the pre-edit buffer — the
    // caller (Buffer) holds that buffer and handles the timing constraint.
    let cs = b.finish();
    let new_buf = cs
        .apply(&buf)
        .expect("edit operation produced an invalid changeset — this is a bug");
    let new_sel_set = SelectionSet::from_vec(new_sels, primary_idx);
    new_sel_set.debug_assert_valid(&new_buf);
    (new_buf, new_sel_set, cs)
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
/// **Last-line whole-line special case**: when the selection spans the entire
/// last content line (head on the structural `\n`, anchor at the line's start)
/// *and* there is a preceding line, the preceding `\n` is consumed instead of
/// the structural one. This matches the vim `dd`-on-last-line convention:
/// rather than leaving a blank trailing line the line merges back into the one
/// above it by removing the separator newline.
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
    // Special case: whole last line with a preceding line.
    // `is_selection_linewise` confirms the selection spans full lines (starts at a
    // line boundary, ends on '\n'). `end_inclusive > last_content_char` confirms
    // the selection reaches the structural trailing '\n' (i.e. this is the last
    // line). `start > 0` confirms there is a line above to merge into.
    let on_last_line = sel.end_inclusive(buf) > buf.last_content_char();
    if on_last_line && is_selection_linewise(buf, sel) && start > 0 {
        // Consume the preceding '\n' instead of the structural one so the last
        // line disappears rather than becoming an empty trailing line (vim
        // `dd`-on-last-line convention).
        let del_start = prev_grapheme_boundary(buf, start);
        if del_start >= b.old_pos() {
            // Cursor: land at the start of the now-last line (what was the line
            // above the deleted one).
            let prev_line = buf.char_to_line(del_start);
            let cursor_line_start = buf.line_to_char(prev_line);
            // Translate to result-buffer coordinates; `saturating_sub` guards the
            // rare multi-cursor case where a prior selection overlapped this region.
            let cursor_new = b.new_pos() + cursor_line_start.saturating_sub(b.old_pos());
            b.retain(del_start - b.old_pos());
            // Delete from the preceding '\n' through the last content char,
            // keeping the structural trailing '\n'.
            b.delete(buf.last_content_char() + 1 - del_start);
            new_sels.push(Selection::collapsed(cursor_new));
            return;
        }
    }
    // Normal path: cap at the last content char so the structural '\n' is never removed.
    let end_incl = sel.content_end(buf);
    b.retain(start - b.old_pos());
    b.delete(end_incl + 1 - start); // end_incl inclusive → +1 for exclusive bound
    new_sels.push(Selection::collapsed(b.new_pos()));
}

/// Private implementation shared by [`paste_after`] and [`paste_before`].
///
/// `before` governs insert position for cursor (non-collapsed) selections:
///
/// | `before` | charwise content           | linewise content (ends `\n`)   |
/// |----------|----------------------------|--------------------------------|
/// | `false`  | one past the cursor char   | start of the next line         |
/// | `true`   | at the cursor char         | start of the cursor's line     |
///
/// Non-collapsed selections:
/// - **Charwise content**: delete the selected region, insert inline.
/// - **Linewise content**: each selection is replaced independently. The selected
///   fragment is deleted and replaced by the pasted line(s). Retained text before
///   the selection on its line is pushed onto its own line by a leading `\n`; the
///   pasted text's own trailing `\n` pushes retained text after the selection onto
///   the next line. The line's original trailing `\n` is consumed only when the
///   selection ends right before it (avoiding a spurious blank line). Multiple
///   selections on the same line or with overlapping line ranges are each replaced
///   independently — the gap between them becomes its own line.
///
/// The replaced selection is discarded; it is never pushed to the kill ring or
/// clipboard (rule: "when pasting over a selection the replaced text is not copied").
fn paste_impl(
    buf: Text,
    sels: SelectionSet,
    values: &[String],
    before: bool,
) -> (Text, SelectionSet, ChangeSet) {
    if values.is_empty() {
        let mut b = ChangeSetBuilder::new(buf.len_chars());
        b.retain_rest();
        return (buf, sels, b.finish());
    }

    let n_sels = sels.len();
    let n_vals = values.len();

    // When counts mismatch, every selection gets the full joined content.
    // Compute once up front so the closure can borrow it as `&str`.
    let joined: String = if n_sels != n_vals {
        values.join("")
    } else {
        String::new()
    };

    apply_edit(buf, sels, |b, buf, i, sel, new_sels| {
        let text: &str = if n_sels == n_vals {
            &values[i]
        } else {
            &joined
        };

        if sel.is_collapsed() {
            if register::is_register_linewise(text) {
                // Linewise cursor paste: insert as whole new line(s).
                // insert advances new_pos() by the char count of the inserted text,
                // so new_pos() - text.chars().count() is the first inserted char.
                let line = buf.char_to_line(sel.head());
                let insert_at = if before {
                    buf.line_to_char(line)
                } else {
                    line_end_exclusive(buf, line)
                };
                // saturating_sub guards against same-line multi-cursor underflow.
                b.retain(insert_at.saturating_sub(b.old_pos()));
                b.insert(text);
                let count = text.chars().count();
                new_sels.push(Selection::new(b.new_pos() - count, b.new_pos() - 1));
            } else {
                // Charwise cursor paste.
                let insert_at = if before {
                    sel.start()
                } else {
                    (sel.end_inclusive(buf) + 1).min(buf.len_chars() - 1)
                };
                b.retain(insert_at - b.old_pos());
                if text.is_empty() {
                    new_sels.push(Selection::collapsed(sel.head()));
                } else {
                    b.insert(text);
                    let count = text.chars().count();
                    new_sels.push(Selection::new(b.new_pos() - count, b.new_pos() - 1));
                }
            }
        } else if register::is_register_linewise(text) {
            // Linewise over a non-collapsed selection: replace the selected fragment
            // with the pasted line(s). Unselected text before/after on the same line
            // is retained and pushed onto its own line by the pasted '\n'.
            let start = sel.start();
            let end_incl = sel.end_inclusive(buf);

            // Prefix a '\n' only when retained text precedes the paste on this line
            // and does not already end in '\n' (i.e. we're not at a line start). When
            // the previous edit ended right at `start` (start == b.old_pos()), the
            // prior paste already supplied the separating '\n'.
            let needs_prefix = start > b.old_pos() && !is_line_start(buf, start);

            // Consume the line's trailing '\n' when the selection ends right before it,
            // so the pasted line's own '\n' doesn't create a blank line. `newline_pos`
            // is the '\n' that terminates the selection's last line.
            let last_line = buf.char_to_line(end_incl);
            let newline_pos = line_end_exclusive(buf, last_line) - 1;
            let del_end = if end_incl + 1 == newline_pos {
                newline_pos + 1
            } else {
                end_incl + 1
            };

            b.retain(start - b.old_pos());
            b.delete(del_end - start);
            if needs_prefix {
                b.insert("\n");
            }
            b.insert(text);
            let count = text.chars().count();
            new_sels.push(Selection::new(b.new_pos() - count, b.new_pos() - 1));
        } else {
            // Charwise over a non-collapsed selection: delete and inline-insert.
            let start = sel.start();
            let end_incl = sel.content_end(buf);
            let end_excl = end_incl + 1;
            b.retain(start - b.old_pos());
            b.delete(end_excl - start);
            b.insert(text);
            if text.is_empty() {
                new_sels.push(Selection::collapsed(b.new_pos()));
            } else {
                let count = text.chars().count();
                new_sels.push(Selection::new(b.new_pos() - count, b.new_pos() - 1));
            }
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
pub(crate) fn insert_char(
    buf: Text,
    sels: SelectionSet,
    ch: char,
) -> (Text, SelectionSet, ChangeSet) {
    apply_edit(buf, sels, |b, buf, _i, sel, new_sels| {
        let start = sel.start();
        b.retain(start - b.old_pos());
        if !sel.is_collapsed() {
            b.delete(sel.content_end(buf) + 1 - start);
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
            delete_one_grapheme(b, buf, new_sels, sel.head());
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
            let p = sel.head();
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
/// kill_ring.push(yanked);
/// ```
pub(crate) fn delete_selection(buf: Text, sels: SelectionSet) -> (Text, SelectionSet, ChangeSet) {
    // Semantically, pressing `d` on a cursor deletes the char under it, and
    // pressing `d` on a selection deletes the selected region — exactly what
    // delete_char_forward does. There is no functional difference between the
    // two operations; the distinction is only in the key that triggered them.
    delete_char_forward(buf, sels)
}

/// Exclusive upper bound for the content `c` should delete from `sel`.
///
/// Returns `(start, stop)` where `start..stop` is the range to delete.
/// A trailing `\n` at `sel.end()` is excluded — `c` clears line content but
/// keeps the line. A collapsed selection on a lone `\n` (empty line) returns
/// `(pos, pos)`, a zero-length no-op.
pub(crate) fn change_span(buf: &Text, sel: &Selection) -> (usize, usize) {
    let start = sel.start();
    let stop = if sel.ends_on_newline(buf) {
        sel.end() // stop before the '\n': `c` clears line content but keeps the line
    } else {
        // end_inclusive accounts for multi-codepoint grapheme clusters; +1 converts
        // to exclusive upper bound for deletion.
        sel.end_inclusive(buf) + 1
    };
    (start, stop)
}

/// Delete the content of each selection, excluding a trailing `\n` — used by `c`.
fn delete_sel_content(
    b: &mut ChangeSetBuilder,
    buf: &Text,
    sel: &Selection,
    new_sels: &mut Vec<Selection>,
) {
    let (start, stop) = change_span(buf, sel);
    b.retain(start - b.old_pos());
    if stop > start {
        b.delete(stop - start);
    }
    new_sels.push(Selection::collapsed(b.new_pos()));
}

/// Delete the content of each selection, excluding a trailing `\n` (normal-mode `c`).
///
/// Differs from [`delete_selection`] in one way: if a selection ends on a `\n`
/// (because `select-line` / `x` was used, or the line is empty), that newline
/// is kept and only the preceding content is removed. This preserves the line
/// structure — `c` rewrites a line's content, not the line itself.
///
/// - Interior `\n`s (mid-selection) are deleted normally — so a multi-line `c`
///   collapses to a single empty line.
/// - Collapsed selection on a lone `\n` (empty line) → no deletion (≡ `i`).
///
/// The caller is responsible for yanking before calling this; use
/// [`change_span`] to extract the same content range for the kill ring.
pub(crate) fn delete_selection_content(
    buf: Text,
    sels: SelectionSet,
) -> (Text, SelectionSet, ChangeSet) {
    apply_edit(buf, sels, |b, buf, _i, sel, new_sels| {
        delete_sel_content(b, buf, sel, new_sels);
    })
}

/// Paste `values` after/onto each selection (normal-mode `p`).
///
/// **Cursor selections (`is_collapsed()`):**
/// - Charwise content (not ending `\n`): inserts just after the cursor character;
///   the pasted text is selected.
/// - Linewise content (ending `\n`): inserts as new line(s) *below* the cursor's
///   line; the pasted line(s) are selected.
///
/// **Non-collapsed selections:**
/// - Charwise content: deletes the selected region, inserts inline; pasted text is selected.
/// - Linewise content: each selection replaced independently. The selected fragment
///   is deleted; retained text on the same line before/after is pushed onto its own
///   line. Multiple selections on the same line each get their own replacement, with
///   the unselected gaps between them becoming their own lines.
///
/// The replaced selection is discarded and not written to any register.
///
/// **Multi-cursor:** `values.len() == sels.len()` → N-to-N (each selection gets
/// its own slot); otherwise all values joined and applied at every selection.
///
/// An empty `values` slice is a no-op.
pub(crate) fn paste_after(
    buf: Text,
    sels: SelectionSet,
    values: &[String],
) -> (Text, SelectionSet, ChangeSet) {
    paste_impl(buf, sels, values, false)
}

/// Paste `values` before/onto each selection (normal-mode `P`).
///
/// **Cursor selections (`is_collapsed()`):**
/// - Charwise content: inserts just before the cursor character; pasted text is selected.
/// - Linewise content: inserts as new line(s) *above* the cursor's line; pasted lines are selected.
///
/// **Non-collapsed selections:** identical semantics to [`paste_after`] — the
/// before/after distinction only applies to cursor selections.
///
/// An empty `values` slice is a no-op.
pub(crate) fn paste_before(
    buf: Text,
    sels: SelectionSet,
    values: &[String],
) -> (Text, SelectionSet, ChangeSet) {
    paste_impl(buf, sels, values, true)
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
        let sel_end = sel.end(); // inclusive last-grapheme-start; equal to sel_start for cursor

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
            if pos >= sel_end {
                break;
            }
            pos = next;
        }
        // new_pos() is one past the last written char — the final grapheme of the
        // replaced range. -1 gives the cursor position (inclusive last char).
        let new_sel_end = b.new_pos() - 1;

        // Reconstruct the selection with its original direction.
        // `Selection::directed` is the canonical constructor for this pattern:
        // it takes content-aware (start, end) bounds and a direction flag.
        let forward = sel.anchor() <= sel.head();
        new_sels.push(Selection::directed(new_sel_start, new_sel_end, forward));
    })
}

#[cfg(test)]
mod tests;
