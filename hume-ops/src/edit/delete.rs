//! Forward/backward char deletion, dedent-on-backspace, word-rubout, and the
//! whole-selection deletes (`d` and `c`'s content-only variant).

use hume_editing::changeset::ChangeSet;
use hume_editing::grapheme::{
    char_pos_at_display_col, display_col_in_line, prev_grapheme_boundary,
};
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;
use hume_editing::word::{WordChars, is_word_boundary};

use super::{apply_edit, delete_one_grapheme, delete_sel_region};
use crate::motion::prev_word_start;

/// Delete the grapheme cluster at the cursor, or delete the selected region.
///
/// - **Single-character selection**: delete the grapheme cluster at `head`
///   (the character the cursor sits on). Cursor stays at the same offset
///   (it now points to what was the next character). No-op when the cursor
///   is on the trailing `\n` (the structural last character of every buffer).
/// - **Multi-character selection**: delete the entire selected region. Cursor
///   lands on `start()`.
pub fn delete_char_forward(
    text: BufferText,
    sels: SelectionSet,
) -> (BufferText, SelectionSet, ChangeSet) {
    apply_edit(text, sels, |b, text, _i, sel, new_sels| {
        if sel.is_collapsed() {
            let p = sel.head();
            // Collapsed cursor on the structural trailing '\n' means the cursor
            // is on a blank last line. Route to delete_sel_region so the
            // whole-last-line merge special-case removes it by consuming the
            // preceding '\n'. `p > 0` excludes the lone-'\n' buffer where no
            // line above exists and the structural '\n' must stay.
            if p + 1 >= text.len_chars() && p > 0 {
                delete_sel_region(b, text, sel, new_sels);
            } else {
                delete_one_grapheme(b, text, new_sels, p);
            }
        } else {
            delete_sel_region(b, text, sel, new_sels);
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
pub fn delete_char_backward(
    text: BufferText,
    sels: SelectionSet,
) -> (BufferText, SelectionSet, ChangeSet) {
    apply_edit(text, sels, |b, text, _i, sel, new_sels| {
        if sel.is_collapsed() {
            let p = sel.head();
            if p == 0 {
                // At start of buffer — nothing to delete to the left.
                let sel = Selection::collapsed(b.new_pos());
                new_sels.push(sel);
                return;
            }
            // Delete the grapheme cluster ending just before `p`.
            let prev = prev_grapheme_boundary(text, p);
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
            delete_sel_region(b, text, sel, new_sels);
        }
    })
}

/// Dedent to the previous tab stop at every selection.
///
/// For each collapsed cursor sitting in leading whitespace (caller-checked —
/// see the editor's `should_dedent_backspace`), this deletes the whitespace
/// between the cursor and the previous tab-stop column. Mixed tabs and spaces
/// are handled by walking the line forward with tab expansion to locate the
/// char offset at the target column ([`char_pos_at_display_col`]).
///
/// Non-collapsed selections and cursors not in leading whitespace are left to
/// [`delete_char_backward`] — the caller dispatches based on the
/// all-or-nothing predicate.
pub fn dedent_tab_backward(
    text: BufferText,
    sels: SelectionSet,
    tab_width: u8,
) -> (BufferText, SelectionSet, ChangeSet) {
    apply_edit(text, sels, |b, text, _i, sel, new_sels| {
        // Caller (`should_dedent_backspace`) guarantees: collapsed and sitting
        // in leading whitespace with p > line_start. Assert the invariant so
        // any future caller mismatch fails loudly in debug builds.
        debug_assert!(
            sel.is_collapsed(),
            "dedent_tab_backward called on non-collapsed selection"
        );
        let p = sel.head();
        let line_idx = text.char_to_line(p);
        let display_col = display_col_in_line(text, line_idx, p, tab_width);
        let prev_stop = hume_rope::width::prev_tab_stop(display_col, tab_width);
        // Clamp target up to the boundary already consumed by a prior same-line
        // cursor, so the second cursor still deletes whatever space remains
        // between that boundary and its own head.
        let target = char_pos_at_display_col(text, line_idx, prev_stop, tab_width).max(b.old_pos());
        if target >= p {
            // Nothing to delete (cursor already at or past its tab stop, or
            // immediately adjacent to the prior cursor's delete).
            b.retain(p - b.old_pos());
            new_sels.push(Selection::collapsed(b.new_pos()));
            return;
        }
        b.retain(target - b.old_pos());
        b.delete(p - target);
        new_sels.push(Selection::collapsed(b.new_pos()));
    })
}

/// Delete the word before each cursor (Ctrl-W in insert mode).
///
/// - **Collapsed cursor**: deletes from the word start to the cursor position,
///   using `prev_word_start` to find the boundary. No-op at buffer start.
/// - **Non-collapsed selection**: delegates to `delete_sel_region`.
///
/// Non-yanking by design: Ctrl-W is readline-style word-rubout, not a kill —
/// the deleted text is not pushed to the kill ring or any register.
pub fn delete_word_backward(
    text: BufferText,
    sels: SelectionSet,
    chars: WordChars<'_>,
) -> (BufferText, SelectionSet, ChangeSet) {
    apply_edit(text, sels, |b, text, _i, sel, new_sels| {
        if sel.is_collapsed() {
            let p = sel.head();
            // Determine how far back to delete. Three no-op cases:
            // (1) cursor at buffer start, (2) prior same-word cursor already consumed
            // past word_start — retain to `p` so this cursor lands at its own position.
            let word_start = if p > 0 {
                let ws = prev_word_start(text, p, is_word_boundary, chars);
                // `ws < b.old_pos()` means a prior cursor in the same word already
                // consumed past `ws`. Treat as no-op so the cursor lands at `p`.
                if ws >= b.old_pos() { ws } else { p }
            } else {
                p // at buffer start — nothing to delete
            };
            b.retain(word_start - b.old_pos());
            if word_start < p {
                b.delete(p - word_start);
            }
            new_sels.push(Selection::collapsed(b.new_pos()));
        } else {
            delete_sel_region(b, text, sel, new_sels);
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
/// let yanked = yank_selections(&text, &sels);
/// let (new_text, new_sels, _cs) = delete_selection(text, sels);
/// kill_ring.push(yanked);
/// ```
pub fn delete_selection(
    text: BufferText,
    sels: SelectionSet,
) -> (BufferText, SelectionSet, ChangeSet) {
    // Semantically, pressing `d` on a cursor deletes the char under it, and
    // pressing `d` on a selection deletes the selected region — exactly what
    // delete_char_forward does. There is no functional difference between the
    // two operations; the distinction is only in the key that triggered them.
    delete_char_forward(text, sels)
}

/// Exclusive upper bound for the content `c` should delete from `sel`.
///
/// Returns `(start, stop)` where `start..stop` is the range to delete.
/// A trailing `\n` at `sel.end()` is excluded — `c` clears line content but
/// keeps the line. A collapsed selection on a lone `\n` (empty line) returns
/// `(pos, pos)`, a zero-length no-op.
pub fn change_span(text: &BufferText, sel: &Selection) -> (usize, usize) {
    let start = sel.start();
    let stop = if sel.ends_on_newline(text) {
        sel.end() // stop before the '\n': `c` clears line content but keeps the line
    } else {
        // end_inclusive accounts for multi-codepoint grapheme clusters; +1 converts
        // to exclusive upper bound for deletion.
        sel.end_inclusive(text) + 1
    };
    (start, stop)
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
pub fn delete_selection_content(
    text: BufferText,
    sels: SelectionSet,
) -> (BufferText, SelectionSet, ChangeSet) {
    apply_edit(text, sels, |b, text, _i, sel, new_sels| {
        let (start, stop) = change_span(text, sel);
        b.retain(start - b.old_pos());
        if stop > start {
            b.delete(stop - start);
        }
        new_sels.push(Selection::collapsed(b.new_pos()));
    })
}
