use hume_editing::edit::{EditBuilder, Edited, Landing, Landings, edit};
use hume_editing::selection::SelectionView;
use hume_editing::state::EditState;
#[cfg(test)]
use hume_editing::{selection::SelectionSet, text::BufferText};

mod align;
mod case;
mod delete;
mod indent;
mod insert;
mod join;
mod paste;
mod replace;
mod sort;

pub use align::align_selections;
pub use case::{make_text_capitalized, make_text_lowercase, make_text_uppercase};
pub use delete::{
    Removal, dedent_tab_backward, delete_char_backward, delete_char_forward, delete_selection,
    delete_selection_content, delete_word_backward,
};
pub use indent::{indent_lines, unindent_lines};
pub use insert::{
    clear_blank_line_indent, insert_char, insert_newline_indent, insert_str, insert_tab,
    open_line_above, owned_blank_indent,
};
pub use join::join_lines_select_spaces;
pub use paste::{paste_after, paste_before};
pub use replace::{
    replace_around_cursors, replace_selections, replace_span_around_cursors, word_start_before,
};
pub use sort::{SortOpts, SortRefusal, sort_lines};

/// Apply an edit command `count` times as one edit, so the repetition is a
/// single undo step. `count == 0` leaves `state` unchanged.
///
/// Test-only, but used from `hume-editor`'s test suite too (a downstream
/// crate); see the `test-util` feature.
#[cfg(any(test, feature = "test-util"))]
pub fn repeat_edit(count: usize, state: EditState, cmd: impl Fn(EditState) -> Edited) -> Edited {
    (0..count).fold(Edited::unchanged(state), |edited, _| edited.then(&cmd))
}

/// One edit made selection by selection. `f` receives the builder and the
/// selection, and returns where that selection lands; the primary stays on
/// the primary's result. The builder applies its operations in position
/// order, so the order `f` records them in does not matter.
pub fn apply_edit<F>(state: EditState, mut f: F) -> Edited
where
    F: for<'a, 'id> FnMut(&mut EditBuilder<'a, 'id>, SelectionView<'a>) -> Landing<'id>,
{
    let primary = state.view().primary().index();
    edit(&state, |plan| {
        let landings = state.view().iter().map(|sel| f(plan, sel)).collect();
        Landings::new(landings, primary)
    })
}

#[cfg(test)]
mod tests;
