//! `G H` turns the lines a selection touches into an ATX heading, or back.

use super::*;

/// The state after `G H` in Normal mode.
fn toggled(marked: &str) -> String {
    after(marked, "GH")
}

/// The state after `<count> G H` in Normal mode.
fn set_to(marked: &str, count: usize) -> String {
    after(marked, &format!("{count}GH"))
}

#[test]
fn a_plain_line_gets_a_level_one_heading() {
    assert_eq!(toggled("-[a]>bc\n"), "# -[a]>bc\n");
}

#[test]
fn a_heading_becomes_a_plain_line() {
    assert_eq!(toggled("-[#]> abc\n"), "-[a]>bc\n");
}

#[test]
fn the_marker_follows_the_lines_indentation() {
    assert_eq!(toggled("  -[x]>\n"), "  # -[x]>\n");
}

#[test]
fn an_indented_heading_loses_only_its_marker() {
    assert_eq!(toggled("  -[#]> x\n"), "  -[x]>\n");
}

#[test]
fn the_blanks_after_a_marker_go_with_it() {
    assert_eq!(toggled("-[#]>   abc\n"), "-[a]>bc\n");
}

#[test]
fn a_marker_with_no_space_after_it_is_text() {
    // `#foo` is not an ATX heading: the marker needs a blank or the line's
    // end after its `#`s.
    assert_eq!(toggled("-[#]>foo\n"), "# -[#]>foo\n");
}

#[test]
fn seven_hashes_are_text() {
    // Only one to six `#`s make a heading.
    assert_eq!(toggled("-[#]>###### x\n"), "# -[#]>###### x\n");
}

#[test]
fn a_count_sets_the_level() {
    assert_eq!(set_to("-[a]>bc\n", 3), "### -[a]>bc\n");
}

#[test]
fn a_count_replaces_an_existing_level() {
    assert_eq!(set_to("# -[a]>bc\n", 2), "## -[a]>bc\n");
}

#[test]
fn a_count_above_six_gives_six() {
    assert_eq!(set_to("-[a]>bc\n", 7), "###### -[a]>bc\n");
}

#[test]
fn a_count_sets_the_level_of_every_line_a_selection_spans() {
    assert_eq!(set_to("-[abc\ndef]>\n", 2), "## -[abc\n## def]>\n");
}

#[test]
fn a_bare_toggle_toggles_every_line_a_selection_spans() {
    assert_eq!(toggled("-[abc\ndef]>\n"), "# -[abc\n# def]>\n");
}

#[test]
fn a_blank_line_is_left_alone() {
    assert_eq!(toggled("-[ ]>  \n"), "-[ ]>  \n");
    assert_eq!(set_to("-[ ]>  \n", 2), "-[ ]>  \n");
}

#[test]
fn a_blank_line_inside_the_selection_is_left_alone() {
    // `abc`, a blank line, and the selection's head on that blank line's
    // break, then the buffer's own final break.
    assert_eq!(toggled("-[abc\n\n]>\n"), "# -[abc\n\n]>\n");
}

#[test]
fn one_undo_takes_back_the_heading_on_every_line() {
    let mut p = editing("-[abc\ndef]>\n");
    p.ed.feed_keys("GHu".chars().map(key));
    assert_eq!(text(&p.ed), "abc\ndef\n");
}

#[test]
fn dot_repeats_the_heading_toggle() {
    assert_eq!(after("-[a]>\nlong\n", "GHj."), "# a\n# lo-[n]>g\n");
}
