//! `pipe`: the editing half of running each selection through a shell
//! command. The process side lives in the editor; this only shapes a
//! command's output, which `replace_each_selection` or `insert_values` then
//! lands.

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
