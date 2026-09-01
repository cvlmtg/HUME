//! `p`/`P` — paste register contents after/before each selection.

use std::borrow::Cow;

use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::lines::{is_line_start, line_break_char, line_end_exclusive};
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::{BufferText, normalize_line_endings};

use super::apply_edit;
use crate::register;

/// Private implementation shared by [`paste_after`] and [`paste_before`].
///
/// `before` governs insert position for cursor (non-collapsed) selections:
///
/// | `before` | charwise content           | linewise content (ends `\n`)   |
/// |----------|----------------------------|--------------------------------|
/// | `false`  | one past the cursor char, clamped to the line's own `\n` | start of the next line |
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
    text: BufferText,
    sels: SelectionSet,
    values: &[String],
    before: bool,
) -> (BufferText, SelectionSet, ChangeSet) {
    if values.is_empty() {
        let mut b = ChangeSetBuilder::new(text.len_chars());
        b.retain_rest();
        return (text, sels, b.finish());
    }

    // Not for the rope's sake — the changeset builder normalizes every
    // insertion — but for `is_register_linewise` below, which asks whether a
    // value ends in `\n`. A register written by a plugin or filled from the
    // OS clipboard can end in `\r\n` or a bare `\r`, and would otherwise be
    // classified charwise and pasted inline despite being whole lines.
    // Each `Cow` borrows (no allocation) when its own value has no `\r`.
    let values: Vec<Cow<'_, str>> = values.iter().map(|v| normalize_line_endings(v)).collect();
    let values = &values[..];

    let n_sels = sels.len();
    let n_vals = values.len();

    // When counts mismatch, every selection gets the full joined content.
    // Compute once up front so the closure can borrow it as `&str`.
    let joined: String = if n_sels != n_vals {
        values.join("")
    } else {
        String::new()
    };

    apply_edit(text, sels, |b, text, i, sel, new_sels| {
        let content: &str = if n_sels == n_vals {
            &values[i]
        } else {
            &joined
        };

        if sel.is_collapsed() {
            if register::is_register_linewise(content) {
                // Linewise cursor paste: insert as whole new line(s).
                // `new_pos()` before the insert is where the pasted text
                // starts; after it, one past where it ends.
                let line = text.char_to_line(sel.head());
                let insert_at = if before {
                    text.line_to_char(line)
                } else {
                    line_end_exclusive(text, line)
                };
                // saturating_sub guards against same-line multi-cursor underflow.
                b.retain(insert_at.saturating_sub(b.old_pos()));
                let paste_start = b.new_pos();
                b.insert(content);
                new_sels.push(Selection::new(paste_start, b.new_pos() - 1));
            } else {
                // Charwise cursor paste.
                let insert_at = if before {
                    sel.start()
                } else {
                    // "After the cursor" must not cross the line break: on an
                    // empty line the cursor sits on the '\n' itself (there is
                    // no other char to land on), so stepping one past it
                    // would drop the text at the start of the next line.
                    let end_incl = sel.end_inclusive(text);
                    (end_incl + 1).min(line_break_char(text, text.char_to_line(end_incl)))
                };
                b.retain(insert_at - b.old_pos());
                if content.is_empty() {
                    new_sels.push(Selection::collapsed(sel.head()));
                } else {
                    let paste_start = b.new_pos();
                    b.insert(content);
                    new_sels.push(Selection::new(paste_start, b.new_pos() - 1));
                }
            }
        } else if register::is_register_linewise(content) {
            // Linewise over a non-collapsed selection: replace the selected fragment
            // with the pasted line(s). Unselected text before/after on the same line
            // is retained and pushed onto its own line by the pasted '\n'.
            let start = sel.start();
            let end_incl = sel.end_inclusive(text);

            // Prefix a '\n' only when retained text precedes the paste on this line
            // and does not already end in '\n' (i.e. we're not at a line start). When
            // the previous edit ended right at `start` (start == b.old_pos()), the
            // prior paste already supplied the separating '\n'.
            let needs_prefix = start > b.old_pos() && !is_line_start(text, sel);

            // Consume the line's trailing '\n' when the selection ends right before it,
            // so the pasted line's own '\n' doesn't create a blank line.
            let last_line = text.char_to_line(end_incl);
            let newline_pos = line_break_char(text, last_line);
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
            // Captured after the separating '\n' so the selection covers the
            // pasted content alone, not the prefix.
            let paste_start = b.new_pos();
            b.insert(content);
            new_sels.push(Selection::new(paste_start, b.new_pos() - 1));
        } else {
            // Charwise over a non-collapsed selection: delete and inline-insert.
            let start = sel.start();
            let end_incl = sel.content_end(text);
            let end_excl = end_incl + 1;
            b.retain(start - b.old_pos());
            b.delete(end_excl - start);
            let paste_start = b.new_pos();
            b.insert(content);
            if content.is_empty() {
                new_sels.push(Selection::collapsed(b.new_pos()));
            } else {
                new_sels.push(Selection::new(paste_start, b.new_pos() - 1));
            }
        }
    })
}

/// Paste `values` after/onto each selection (normal-mode `p`). See
/// `paste_impl` for the cursor/non-collapsed × charwise/linewise matrix;
/// the replaced selection is discarded and not written to any register.
///
/// **Multi-cursor:** `values.len() == sels.len()` → N-to-N (each selection
/// gets its own slot); otherwise all values joined and applied at every
/// selection. An empty `values` slice is a no-op.
pub fn paste_after(
    text: BufferText,
    sels: SelectionSet,
    values: &[String],
) -> (BufferText, SelectionSet, ChangeSet) {
    paste_impl(text, sels, values, false)
}

/// Paste `values` before/onto each selection (normal-mode `P`) — mirrors
/// [`paste_after`]; the before/after distinction only applies to cursor
/// selections (see `paste_impl`'s matrix). An empty `values` slice is a
/// no-op.
pub fn paste_before(
    text: BufferText,
    sels: SelectionSet,
    values: &[String],
) -> (BufferText, SelectionSet, ChangeSet) {
    paste_impl(text, sels, values, true)
}
