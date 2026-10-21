use super::super::*;

// ── fit_pipe_output ─────────────────────────────────────────────────────────

#[test]
fn fit_pipe_output_drops_one_break_only_when_the_input_had_none() {
    let cases = [
        (false, "ABC\n", "ABC"),
        (false, "ABC\r\n", "ABC"),
        (false, "ABC\n\n", "ABC\n"),
        (false, "ABC", "ABC"),
        (false, "ABC\r", "ABC"),
        (false, "ABC\n\r", "ABC\n"),
        (true, "ABC\n", "ABC\n"),
        (true, "ABC", "ABC"),
        (false, "", ""),
    ];
    for (input_ends_line, output, expected) in cases {
        assert_eq!(
            fit_pipe_output(input_ends_line, output.to_string()),
            expected,
            "input ends a line: {input_ends_line}, output {output:?}"
        );
    }
}
