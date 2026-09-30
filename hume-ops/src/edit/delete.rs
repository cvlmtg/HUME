//! Forward/backward char deletion, dedent-on-backspace, word-rubout, and the
//! whole-selection deletes (`d` and `c`'s content-only variant).

use hume_editing::edit::Edited;
use hume_editing::edit::Landing;
use hume_editing::grapheme::{char_pos_at_display_col, display_col_in_line};
use hume_editing::state::EditState;
use hume_editing::word::{WordChars, is_word_boundary};
use hume_rope::offset::ExclusiveRange;

use super::apply_edit;
use crate::motion::prev_word_start;

/// What `d` or `c` did: the edit, and one register entry per selection, in
/// document order, holding the text that selection took out of the buffer.
/// An entry is empty for a selection that removed nothing.
pub struct Removal {
    pub edited: Edited,
    pub yanked: Vec<String>,
}

/// Delete every selection (normal-mode `d`), and report what each removed.
///
/// A selection of whole lines removes them; on the last line the `\n` before
/// it goes too, so no empty line is left, and the register gets the lines.
/// Any other selection removes what it covers except the structural `\n`,
/// and the register gets that text. A cursor on the structural `\n` of a
/// line with text removes nothing. See `EditBuilder::remove`.
pub fn delete_selection(state: EditState) -> Removal {
    let mut yanked = Vec::new();
    let edited = apply_edit(state, |b, sel| match b.remove(sel) {
        Some(removed) => {
            yanked.push(removed.text.to_string());
            removed.cursor
        }
        None => {
            yanked.push(String::new());
            Landing::kept(sel.selection())
        }
    });
    Removal { edited, yanked }
}

/// [`delete_selection`] without the register text: the Delete key.
pub fn delete_char_forward(state: EditState) -> Edited {
    delete_selection(state).edited
}

/// Delete the grapheme cluster before each cursor, or each selection's
/// region (Backspace).
///
/// - **Cursor**: the cluster ending just before the cursor goes, and the
///   cursor moves back onto what follows it. No-op at the buffer start.
/// - **Selection**: removed as [`delete_selection`] removes it.
pub fn delete_char_backward(state: EditState) -> Edited {
    apply_edit(state, |b, sel| {
        if !sel.is_cursor() {
            return b
                .remove(sel)
                .map_or(Landing::kept(sel.selection()), |r| r.cursor);
        }
        let head = sel.head();
        let Some(prev) = hume_rope::grapheme::prev_cluster(b.text().full_slice(), head.into())
        else {
            return Landing::kept(sel.selection());
        };
        Landing::cursor(b.delete(ExclusiveRange::new(prev.offset(), head.offset())))
    })
}

/// Dedent to the previous tab stop at every selection.
///
/// For each cursor sitting in leading whitespace (caller-checked, see the
/// editor's `should_dedent_backspace`), this deletes the whitespace between
/// the cursor and the previous tab-stop column. Mixed tabs and spaces are
/// handled by walking the line forward with tab expansion to locate the char
/// offset at the target column ([`char_pos_at_display_col`]).
pub fn dedent_tab_backward(state: EditState, tab_width: u8) -> Edited {
    apply_edit(state, |b, sel| {
        debug_assert!(
            sel.is_cursor(),
            "dedent_tab_backward called on non-collapsed selection"
        );
        let text = b.text();
        let p = sel.head().offset();
        let line_idx = sel.head_line();
        let display_col = display_col_in_line(text, line_idx, p, tab_width);
        let prev_stop = hume_rope::column::BufferLineCol::new(hume_rope::width::prev_tab_stop(
            display_col.get() as usize,
            tab_width,
        ) as u32);
        let target = char_pos_at_display_col(text, line_idx, prev_stop, tab_width);
        Landing::cursor(b.delete(ExclusiveRange::new(target.offset().min(p), p)))
    })
}

/// Delete the word before each cursor (Ctrl-w in insert mode).
///
/// - **Cursor**: deletes from the word start to the cursor. No-op at the
///   buffer start. Cursors in one word delete the union of their ranges.
/// - **Selection**: removed as [`delete_selection`] removes it.
///
/// Non-yanking: Ctrl-w is readline-style word-rubout, not a kill.
pub fn delete_word_backward(state: EditState, chars: WordChars<'_>) -> Edited {
    apply_edit(state, |b, sel| {
        if !sel.is_cursor() {
            return b
                .remove(sel)
                .map_or(Landing::kept(sel.selection()), |r| r.cursor);
        }
        let p = sel.head();
        let word_start = prev_word_start(b.text(), p, is_word_boundary, chars);
        if word_start >= p {
            return Landing::kept(sel.selection());
        }
        Landing::cursor(b.delete(ExclusiveRange::new(word_start.offset(), p.offset())))
    })
}

/// Delete the content of each selection except the `\n` it ends on
/// (normal-mode `c`), and report what each removed.
///
/// A selection ending on a `\n` (after `x`, or on an empty line) keeps it:
/// `c` rewrites a line's content, not the line. Interior `\n`s go, so a
/// multi-line `c` leaves one empty line. A cursor on a lone `\n` removes
/// nothing, like `i`.
pub fn delete_selection_content(state: EditState) -> Removal {
    let mut yanked = Vec::new();
    let edited = apply_edit(state, |b, sel| match b.remove_content(sel) {
        Some(removed) => {
            yanked.push(removed.text.to_string());
            removed.cursor
        }
        None => {
            yanked.push(String::new());
            Landing::kept(sel.selection())
        }
    });
    Removal { edited, yanked }
}
