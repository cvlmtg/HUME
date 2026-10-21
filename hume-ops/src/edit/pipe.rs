//! `pipe`: the editing half of running each selection through a shell
//! command. The process side lives in the editor; these functions only
//! shape a command's output and land it.

use hume_editing::edit::{Edited, Landing};
use hume_editing::state::EditState;

use super::apply_edit;

/// Replace selection `i` with `values[i]`, in document order. Each result
/// covers its replacement and keeps its facing; an empty value leaves a
/// cursor on the cluster after the removed text.
///
/// # Panics
/// Panics if `values` and the selections differ in number.
pub fn replace_each_selection(state: EditState, values: &[String]) -> Edited {
    assert_eq!(
        values.len(),
        state.view().len(),
        "replace_each_selection: needs one value per selection"
    );
    apply_edit(state, |b, sel| {
        let mark = b.replace(sel.covered(), &values[sel.index()]);
        Landing::covering(mark, sel.facing())
    })
}

/// A command's `output`, minus the line break a filter adds to text that
/// had none: when its input did not end with `\n` (`input_ends_line` is
/// false) and `output` ends with a line break, that one goes: `\r\n`, `\n`,
/// or a lone `\r`, which the buffer would read as a line break too.
pub fn fit_pipe_output(input_ends_line: bool, mut output: String) -> String {
    if !input_ends_line {
        let cut = if output.ends_with("\r\n") {
            2
        } else if output.ends_with(['\n', '\r']) {
            1
        } else {
            0
        };
        output.truncate(output.len() - cut);
    }
    output
}
