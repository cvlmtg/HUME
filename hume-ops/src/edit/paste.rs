//! `p`/`P`: paste register contents after/before each selection.

use hume_editing::edit::Edited;
use hume_editing::edit::Landing;
use hume_editing::lines::{line_break, line_range, line_start};
use hume_editing::selection::Facing;
use hume_editing::state::EditState;
use hume_rope::cluster::ClusterRange;

use super::apply_edit;
use crate::register::{Piece, Shape};

/// `before` governs insert position for cursors (collapsed selections):
///
/// | `before` | charwise piece             | linewise piece                 |
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
fn paste_impl(state: EditState, values: &[Piece], before: bool) -> Edited {
    if values.is_empty() {
        return Edited::unchanged(state);
    }

    let n_sels = state.view().len();
    let n_vals = values.len();

    // When counts mismatch, every selection gets the full joined content,
    // pasting in the shape of the last piece. Computed once so the closure can
    // borrow it.
    let joined = (n_sels != n_vals).then(|| {
        let shape = values.last().map_or(Shape::Charwise, Piece::shape);
        Piece::new(values.iter().map(Piece::text).collect::<String>(), shape)
    });
    let piece_of = |i: usize| -> &Piece { joined.as_ref().unwrap_or_else(|| &values[i]) };
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
                    && piece_of(i - 1).is_linewise()
                    && sels[i - 1].covered().end().offset() == sels[i].start().offset()
            })
            .collect()
    };

    apply_edit(state, |b, sel| {
        let piece = piece_of(sel.index());
        let content = piece.text();
        let text = b.text();

        if sel.is_cursor() {
            if piece.is_linewise() {
                // Linewise cursor paste: whole new line(s) above or below.
                let line = sel.head_line();
                let insert_at = if before {
                    line_start(text, line).into()
                } else {
                    line_range(text, line).end()
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

        if piece.is_linewise() {
            // Linewise over a selection: the pasted lines replace the selected
            // fragment. Text before it on its line keeps its own line through
            // a leading '\n'; the pasted text's own '\n' pushes text after it
            // onto the next line. A selection ending right before its line's
            // '\n' takes that '\n' too, so no blank line is left.
            let covered = sel.covered();
            let last_line = sel.lines().end;
            let line_break = line_break(text, last_line);
            let range = if covered.end() == line_break.into() {
                ClusterRange::through(text.full_slice(), covered.start(), line_break)
                    .expect("a selection starts before the break after it")
            } else {
                covered
            };
            if !sel.starts_line() && !follows_pasted_line[sel.index()] {
                b.insert(sel.start(), "\n");
            }
            let mark = b.replace(range, content);
            return Landing::covering(mark, Facing::Forward);
        }

        // Charwise over a selection: the content takes its place.
        let mark = b.replace(sel.covered(), content);
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
pub fn paste_after(state: EditState, values: &[Piece]) -> Edited {
    paste_impl(state, values, false)
}

/// Paste `values` before/onto each selection (normal-mode `P`). Mirrors
/// [`paste_after`]; the before/after distinction only applies to cursor
/// selections (see `paste_impl`'s matrix). An empty `values` slice is a
/// no-op.
pub fn paste_before(state: EditState, values: &[Piece]) -> Edited {
    paste_impl(state, values, true)
}
