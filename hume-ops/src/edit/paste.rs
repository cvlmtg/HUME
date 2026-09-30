//! `p`/`P`: paste register contents after/before each selection.

use hume_editing::edit::Edited;
use hume_editing::edit::Landing;
use hume_editing::lines::next_line_start;
use hume_editing::selection::Facing;
use hume_editing::state::EditState;
use hume_rope::offset::ExclusiveRange;

use super::apply_edit;
use crate::register;

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
///   independently; the gap between them becomes its own line.
///
/// The replaced selection is discarded; it is never pushed to the kill ring or
/// clipboard (rule: "when pasting over a selection the replaced text is not copied").
fn paste_impl(state: EditState, values: &[String], before: bool) -> Edited {
    if values.is_empty() {
        return Edited::unchanged(state);
    }

    let n_sels = state.view().len();
    let n_vals = values.len();

    // When counts mismatch, every selection gets the full joined content.
    // Computed once so the closure can borrow it as `&str`.
    let joined: String = if n_sels != n_vals {
        values.join("")
    } else {
        String::new()
    };

    let content_of = |i: usize| -> &str {
        if n_sels == n_vals {
            &values[i]
        } else {
            &joined
        }
    };
    // Whether the text right before selection `i` is a linewise paste over
    // the selection touching it: that paste already ended its line, so `i`
    // needs no leading '\n' of its own.
    let follows_pasted_line: Vec<bool> = {
        let view = state.view();
        let sels: Vec<_> = view.iter().collect();
        (0..sels.len())
            .map(|i| {
                i > 0
                    && !sels[i - 1].is_cursor()
                    && register::is_register_linewise(content_of(i - 1))
                    && sels[i - 1].covered().end().offset() == sels[i].start().offset()
            })
            .collect()
    };

    apply_edit(state, |b, sel| {
        let content = content_of(sel.index());
        let text = b.text();

        if sel.is_cursor() {
            if register::is_register_linewise(content) {
                // Linewise cursor paste: whole new line(s) above or below.
                let line = sel.head_line();
                let insert_at = if before {
                    text.line_to_char(line.into())
                } else {
                    next_line_start(text, line.into())
                };
                let mark = b.insert(insert_at, content);
                return Landing::covering(mark, Facing::Forward);
            }
            if content.is_empty() {
                return Landing::kept(sel.selection());
            }
            // Charwise: before the cursor, or after it without crossing its
            // line break.
            let insert_at = if before {
                sel.start()
            } else {
                sel.append_point()
            };
            let mark = b.insert(insert_at, content);
            return Landing::covering(mark, Facing::Forward);
        }

        if register::is_register_linewise(content) {
            // Linewise over a selection: the pasted lines replace the selected
            // fragment. Text before it on its line keeps its own line through
            // a leading '\n'; the pasted text's own '\n' pushes text after it
            // onto the next line. A selection ending right before its line's
            // '\n' takes that '\n' too, so no blank line is left.
            let covered = sel.covered();
            let last_line = sel.lines().end;
            let line_break = hume_rope::lines::line_break(text.rope(), last_line);
            let range = if covered.end() == line_break.into() {
                ExclusiveRange::new(
                    covered.start().offset(),
                    hume_rope::grapheme::cluster_end(text.full_slice(), line_break).offset(),
                )
            } else {
                covered.chars()
            };
            if !sel.starts_line() && !follows_pasted_line[sel.index()] {
                b.insert(sel.start(), "\n");
            }
            let mark = b.replace(range, content);
            return Landing::covering(mark, Facing::Forward);
        }

        // Charwise over a selection: delete it, insert in its place.
        b.delete(sel.covered());
        let mark = b.insert(sel.start(), content);
        Landing::covering(mark, Facing::Forward)
    })
}

/// Paste `values` after/onto each selection (normal-mode `p`). See
/// `paste_impl` for the cursor/non-collapsed × charwise/linewise matrix;
/// the replaced selection is discarded and not written to any register.
///
/// **Multi-cursor:** `values.len() == sels.len()` → N-to-N (each selection
/// gets its own slot); otherwise all values joined and applied at every
/// selection. An empty `values` slice is a no-op.
pub fn paste_after(state: EditState, values: &[String]) -> Edited {
    paste_impl(state, values, false)
}

/// Paste `values` before/onto each selection (normal-mode `P`). Mirrors
/// [`paste_after`]; the before/after distinction only applies to cursor
/// selections (see `paste_impl`'s matrix). An empty `values` slice is a
/// no-op.
pub fn paste_before(state: EditState, values: &[String]) -> Edited {
    paste_impl(state, values, true)
}
