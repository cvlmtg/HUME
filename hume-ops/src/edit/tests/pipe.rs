use super::super::*;
use test_fixtures::assert_state;
use test_fixtures::testing::{parse_state, state};
use test_fixtures::unicode::ALL;

fn values(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

// ── replace_each_selection ──────────────────────────────────────────────────

#[test]
fn replace_each_selection_covers_the_output() {
    assert_state!(
        "-[hello]> world\n",
        |(text, sels)| replace_each_selection(state(text, sels), &values(&["HI"])),
        "-[HI]> world\n"
    );
}

#[test]
fn replace_each_selection_gives_each_selection_its_own_value() {
    assert_state!(
        "-[ab]> -{cd}> -[ef]>\n",
        |(text, sels)| replace_each_selection(state(text, sels), &values(&["1", "22", "333"])),
        "-[1]> -{22}> -[333]>\n"
    );
}

#[test]
fn replace_each_selection_keeps_a_backward_selection_backward() {
    assert_state!(
        "<[ab]-\n",
        |(text, sels)| replace_each_selection(state(text, sels), &values(&["xyz"])),
        "<[xyz]-\n"
    );
}

#[test]
fn replace_each_selection_with_empty_output_leaves_a_cursor_after_the_removed_text() {
    assert_state!(
        "a-[bc]>d\n",
        |(text, sels)| replace_each_selection(state(text, sels), &values(&[""])),
        "a-[d]>\n"
    );
}

#[test]
fn replace_each_selection_takes_multi_line_output() {
    assert_state!(
        "-[ab]>\n",
        |(text, sels)| replace_each_selection(state(text, sels), &values(&["x\ny"])),
        "-[x\ny]>\n"
    );
}

#[test]
fn replace_each_selection_over_the_whole_text_keeps_the_final_break() {
    assert_state!(
        "-[b\na\n]>",
        |(text, sels)| replace_each_selection(state(text, sels), &values(&["a\nb\n"])),
        "-[a\nb\n]>"
    );
}

#[test]
fn replace_each_selection_with_identical_output_changes_nothing() {
    let (text, sels) = parse_state("-[ab]> -{cd}>\n");
    let edited = replace_each_selection(state(text, sels), &values(&["ab", "cd"]));
    assert!(edited.changes().is_identity());
}

#[test]
fn replace_each_selection_replaces_every_corpus_sample_whole() {
    for s in ALL {
        assert_state!(
            &format!("x\n-[{s}]>b\n"),
            |(text, sels)| replace_each_selection(state(text, sels), &values(&["Z"])),
            "x\n-[Z]>b\n"
        );
    }
}

#[test]
#[should_panic(expected = "one value per selection")]
fn replace_each_selection_refuses_a_count_mismatch() {
    let (text, sels) = parse_state("-[ab]> -{cd}>\n");
    replace_each_selection(state(text, sels), &values(&["x"]));
}

// ── fit_pipe_output ─────────────────────────────────────────────────────────

#[test]
fn fit_pipe_output_drops_one_break_only_when_the_input_had_none() {
    let cases = [
        (false, "ABC\n", "ABC"),
        (false, "ABC\r\n", "ABC"),
        (false, "ABC\n\n", "ABC\n"),
        (false, "ABC", "ABC"),
        (false, "ABC\r", "ABC\r"),
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
