//! Forward/backward char deletion, dedent-on-backspace, word-rubout, and the
//! whole-selection deletes (`d` and `c`'s content-only variant).

use hume_editing::edit::{EditBuilder, Edited, Landing};
use hume_editing::grapheme::{char_pos_at_display_col, clusters_before, display_col_in_line};
use hume_editing::selection::SelectionView;
use hume_editing::state::EditState;
use hume_editing::word::{WordChars, is_word_boundary};
use hume_rope::cluster::ClusterRange;
use hume_rope::offset::ExclusiveRange;
use ropey::RopeSlice;

use super::apply_edit;
use crate::motion::prev_word_start;
use crate::register::{Piece, Shape};

/// What `d` or `c` did: the edit, and one register entry per selection, in
/// document order, holding the text that selection took out of the buffer.
/// An entry is empty for a selection that removed nothing.
pub struct Removal {
    pub edited: Edited,
    pub yanked: Vec<Piece>,
}

/// Delete every selection (normal-mode `d`), and report what each removed.
///
/// Every selection removes what it covers, and the text keeps ending with a
/// `\n`: removing the last line leaves no empty line, and a cursor on the
/// structural `\n` of a line with text removes nothing. The register gets
/// whole lines as lines and anything else short of the structural `\n`.
pub fn delete_selection(state: EditState) -> Removal {
    let mut yanked = Vec::new();
    let edited = remove_each(state, remove, |removed, linewise| {
        yanked.push(removed_piece(removed, linewise))
    });
    Removal { edited, yanked }
}

/// What [`remove`] or [`remove_content`] took out of a selection: the text
/// a register gets, `None` when it took nothing, whether that text is whole
/// lines, and where the selection lands.
struct Removed<'a, 'id> {
    text: Option<RopeSlice<'a>>,
    linewise: bool,
    cursor: Landing<'id>,
}

/// Take `sel` out of the text as `d` does: everything it covers goes, and
/// the register gets [`removal`]. A selection of whole lines lands on the
/// start of the line after them, or of the last line when they ran to the
/// end.
fn remove<'a, 'id>(b: &mut EditBuilder<'a, 'id>, sel: SelectionView<'a>) -> Removed<'a, 'id> {
    let covered = sel.covered();
    let linewise = sel.is_linewise();
    let at = b.delete(covered);
    let cursor = if linewise {
        Landing::line_start_of(at)
    } else {
        Landing::cursor(at)
    };
    Removed {
        text: removal_of(sel, covered, linewise),
        linewise,
        cursor,
    }
}

/// Take what `sel` covers out of the text as `c` does: everything but the
/// `\n` it ends on. Takes nothing when that `\n` is all it covers.
fn remove_content<'a, 'id>(
    b: &mut EditBuilder<'a, 'id>,
    sel: SelectionView<'a>,
) -> Removed<'a, 'id> {
    match sel.content() {
        Some(content) => Removed {
            text: Some(b.text().slice(content.chars())),
            linewise: false,
            cursor: Landing::cursor(b.delete(content)),
        },
        None => Removed {
            text: None,
            linewise: false,
            cursor: Landing::kept(sel.selection()),
        },
    }
}

/// The text a register receives when `d` removes `sel`, or `None` when `d`
/// removes nothing of it. Whole lines give the whole covered text; anything
/// else gives it short of the structural `\n`, which `d` keeps.
pub(crate) fn removal(sel: SelectionView<'_>) -> Option<RopeSlice<'_>> {
    removal_of(sel, sel.covered(), sel.is_linewise())
}

/// [`removal`] for a caller already holding `sel`'s covered range and its
/// linewise classification.
fn removal_of(
    sel: SelectionView<'_>,
    covered: ClusterRange,
    linewise: bool,
) -> Option<RopeSlice<'_>> {
    let text = sel.text();
    let kept_break = ExclusiveRange::new(
        covered.start().offset(),
        covered.end().offset().min(text.last_char()),
    );
    if linewise {
        (!kept_break.is_empty() || sel.lines().start.index() != 0).then(|| sel.slice())
    } else {
        (!kept_break.is_empty()).then(|| text.slice(kept_break))
    }
}

/// Remove what `remove` takes out of each selection. `on_removed` sees each
/// selection's removed text, `None` when it removed nothing, and whether that
/// text is whole lines.
fn remove_each(
    state: EditState,
    remove: impl for<'a, 'id> Fn(&mut EditBuilder<'a, 'id>, SelectionView<'a>) -> Removed<'a, 'id>,
    mut on_removed: impl FnMut(Option<RopeSlice<'_>>, bool),
) -> Edited {
    apply_edit(state, |b, sel| {
        let removed = remove(b, sel);
        on_removed(removed.text, removed.linewise);
        removed.cursor
    })
}

/// [`delete_selection`] without the register text: the Delete key.
pub fn delete_char_forward(state: EditState) -> Edited {
    remove_each(state, remove, |_, _| {})
}

/// Delete the grapheme cluster before each cursor, or each selection's
/// region (Backspace).
///
/// - **Cursor**: the cluster ending just before the cursor goes, and the
///   cursor moves back onto what follows it. No-op at the buffer start.
/// - **Selection**: removed as [`delete_selection`] removes it.
pub fn delete_char_backward(state: EditState) -> Edited {
    delete_back_to(state, |sel| {
        clusters_before(sel.text(), sel.head().into())
            .next()
            .map(|cluster| cluster.range())
    })
}

/// The backward deletes' shared shape: each cursor deletes the range
/// `range_of` names, which ends at its head, and keeps its place when there
/// is nothing to delete; each selection is removed as [`delete_selection`]
/// removes it.
fn delete_back_to(
    state: EditState,
    range_of: impl Fn(SelectionView<'_>) -> Option<ClusterRange>,
) -> Edited {
    apply_edit(state, |b, sel| {
        if !sel.is_cursor() {
            return remove(b, sel).cursor;
        }
        match range_of(sel) {
            Some(range) => Landing::cursor(b.delete(range)),
            None => Landing::kept(sel.selection()),
        }
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
    debug_assert!(
        state.view().iter().all(|sel| sel.is_cursor()),
        "dedent_tab_backward called on non-collapsed selection"
    );
    delete_back_to(state, |sel| {
        let text = sel.text();
        let line_idx = sel.head_line();
        let display_col = display_col_in_line(text, line_idx, sel.head().offset(), tab_width);
        let prev_stop = hume_rope::column::BufferLineCol::new(hume_rope::width::prev_tab_stop(
            display_col.get() as usize,
            tab_width,
        ) as u32);
        let start = char_pos_at_display_col(text, line_idx, prev_stop, tab_width).min(sel.head());
        ClusterRange::between(text.full_slice(), start, sel.head().into())
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
    delete_back_to(state, |sel| {
        let text = sel.text();
        let start = prev_word_start(text, sel.head(), is_word_boundary, chars);
        ClusterRange::between(text.full_slice(), start, sel.head().into())
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
    let edited = remove_each(state, remove_content, |removed, _| {
        yanked.push(piece(removed, Shape::Charwise));
    });
    Removal { edited, yanked }
}

/// What `d` puts in the register for the text it removed: whole lines paste
/// as lines, anything else as characters. Empty when `d` removed nothing.
fn removed_piece(removed: Option<RopeSlice<'_>>, linewise: bool) -> Piece {
    let shape = if removed.is_some() && linewise {
        Shape::Linewise
    } else {
        Shape::Charwise
    };
    piece(removed, shape)
}

/// The register entry for `removed`, pasting as `shape`; empty when nothing
/// was removed.
fn piece(removed: Option<RopeSlice<'_>>, shape: Shape) -> Piece {
    Piece::new(removed.map_or_else(String::new, |r| r.to_string()), shape)
}

/// What `d` would put in the register for each selection, in document order,
/// without changing the text: the one rule for what a selection's text is in
/// a register, and the shape it pastes in. An entry is empty for a selection
/// `d` would remove nothing from.
pub fn yank_selections(state: &EditState) -> Vec<Piece> {
    state
        .view()
        .iter()
        .map(|sel| removed_piece(removal(sel), sel.is_linewise()))
        .collect()
}
