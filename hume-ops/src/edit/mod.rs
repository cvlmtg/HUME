use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::grapheme::next_grapheme_boundary;
use hume_editing::selection::{Selection, SelectionSet, is_selection_linewise};
use hume_editing::text::Text;

mod align;
mod case;
mod delete;
mod insert;
mod join;
mod paste;
mod replace;
mod sort;

pub use align::align_selections;
pub use case::{make_text_capitalized, make_text_lowercase, make_text_uppercase};
pub use delete::{
    change_span, dedent_tab_backward, delete_char_backward, delete_char_forward, delete_selection,
    delete_selection_content, delete_word_backward,
};
pub use insert::{
    blank_line_ws_range, clear_blank_line_indent, insert_char, insert_newline_indent, insert_str,
    insert_tab,
};
pub use join::join_lines_select_spaces;
pub use paste::{paste_after, paste_before};
pub use replace::{
    replace_around_cursors, replace_selections, replace_span_around_cursors, word_start_before,
};
pub use sort::{SortOpts, SortRefusal, sort_rows};

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
// extracts it and delegates the per-selection work to a closure. Every
// sibling in this directory (insert/delete/paste/replace/case/join/align)
// builds on it.
//
// The ChangeSet is returned so the undo system can call `cs.invert(&old_buf)`
// to produce the inverse transaction. The caller (Document) holds the pre-edit
// buffer and handles the invert timing constraint.

/// Apply an edit command `count` times, composing all changesets into one.
///
/// The command must return `(Text, SelectionSet, ChangeSet)`. The N
/// changesets are folded with [`ChangeSet::compose`] so the whole repetition
/// becomes a single undo step when passed to the editor buffer's own
/// `apply_edit`.
///
/// For motions, count is handled inside `apply_motion` per-selection instead
/// (prevents premature merging of multi-cursor selections between steps).
///
/// If `count == 0`, returns the original state with an identity ChangeSet.
///
/// Test-only, but used from `hume-editor`'s test suite too (a downstream
/// crate) — see the `test-util` feature.
#[cfg(any(test, feature = "test-util"))]
pub fn repeat_edit(
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
pub fn apply_edit<F>(buf: Text, sels: SelectionSet, mut f: F) -> (Text, SelectionSet, ChangeSet)
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
        let del_start = hume_editing::grapheme::prev_grapheme_boundary(buf, start);
        if del_start >= b.old_pos() {
            // Cursor: land at the start of the merged line (what was the line
            // above the deleted one). Compute as (del_start's new_pos) minus
            // del_start's column within its original line — this stays correct
            // in the multi-cursor case where b.new_pos() != b.old_pos().
            let prev_line = buf.char_to_line(del_start);
            let col_in_line = del_start - buf.line_to_char(prev_line);
            b.retain(del_start - b.old_pos());
            let cursor_new = b.new_pos().saturating_sub(col_in_line);
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

#[cfg(test)]
mod tests;
