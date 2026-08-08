//! `p`/`P` — paste register contents after/before each selection.

use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::lines::{is_line_start, line_break_char, line_end_exclusive};
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::Text;

use super::apply_edit;
use crate::register;

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
            let needs_prefix = start > b.old_pos() && !is_line_start(buf, sel);

            // Consume the line's trailing '\n' when the selection ends right before it,
            // so the pasted line's own '\n' doesn't create a blank line.
            let last_line = buf.char_to_line(end_incl);
            let newline_pos = line_break_char(buf, last_line);
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

/// Paste `values` after/onto each selection (normal-mode `p`). See
/// `paste_impl` for the cursor/non-collapsed × charwise/linewise matrix;
/// the replaced selection is discarded and not written to any register.
///
/// **Multi-cursor:** `values.len() == sels.len()` → N-to-N (each selection
/// gets its own slot); otherwise all values joined and applied at every
/// selection. An empty `values` slice is a no-op.
pub fn paste_after(
    buf: Text,
    sels: SelectionSet,
    values: &[String],
) -> (Text, SelectionSet, ChangeSet) {
    paste_impl(buf, sels, values, false)
}

/// Paste `values` before/onto each selection (normal-mode `P`) — mirrors
/// [`paste_after`]; the before/after distinction only applies to cursor
/// selections (see `paste_impl`'s matrix). An empty `values` slice is a
/// no-op.
pub fn paste_before(
    buf: Text,
    sels: SelectionSet,
    values: &[String],
) -> (Text, SelectionSet, ChangeSet) {
    paste_impl(buf, sels, values, true)
}
