//! `G X` on a Markdown task item checks or unchecks its box.

use super::*;

/// The state after `G X` in Normal mode.
fn toggled(marked: &str) -> String {
    let mut p = editing(marked);
    p.ed.feed_keys([key('G'), key('X')]);
    state(&p.ed)
}

#[test]
fn an_unchecked_box_gets_checked() {
    assert_eq!(toggled("- [ ] -[a]>\n"), "- [x] -[a]>\n");
}

#[test]
fn a_checked_box_gets_unchecked() {
    assert_eq!(toggled("- [x] -[a]>\n"), "- [ ] -[a]>\n");
}

#[test]
fn an_uppercase_checked_box_gets_unchecked() {
    assert_eq!(toggled("- [X] -[a]>\n"), "- [ ] -[a]>\n");
}

#[test]
fn an_ordered_task_item_gets_checked() {
    assert_eq!(toggled("1. [ ] -[a]>\n"), "1. [x] -[a]>\n");
}

#[test]
fn a_nested_task_item_gets_its_own_box_checked() {
    assert_eq!(toggled("1. - [ ] -[a]>\n"), "1. - [x] -[a]>\n");
}

#[test]
fn a_checked_first_box_unchecks_every_line_the_selection_covers() {
    assert_eq!(
        toggled("-[- [x] a\n- [ ] b\n- [x] c]>\n"),
        "-[- [ ] a\n- [ ] b\n- [ ] c]>\n"
    );
}

#[test]
fn an_unchecked_first_box_checks_every_line_the_selection_covers() {
    assert_eq!(
        toggled("-[- [ ] a\n- [x] b\n- [ ] c]>\n"),
        "-[- [x] a\n- [x] b\n- [x] c]>\n"
    );
}

#[test]
fn the_primary_selection_decides_for_the_others() {
    assert_eq!(
        toggled("- [ ] -[a]>\n- [x] -{b}>\n"),
        "- [ ] -[a]>\n- [ ] -{b}>\n"
    );
}

#[test]
fn a_primary_without_a_box_leaves_the_first_box_to_decide() {
    assert_eq!(
        toggled("-{p}>lain\n\n- [x] -[a]>\n- [ ] -[b]>\n"),
        "-{p}>lain\n\n- [ ] -[a]>\n- [ ] -[b]>\n"
    );
}

#[test]
fn a_cursor_inside_the_box_stays_on_it() {
    assert_eq!(toggled("- [-[ ]>] a\n"), "- [-[x]>] a\n");
}

#[test]
fn an_empty_box_gets_checked() {
    assert_eq!(toggled("- [-[ ]>] \n"), "- [-[x]>] \n");
}

#[test]
fn an_empty_checked_box_gets_unchecked() {
    assert_eq!(toggled("- [-[x]>] \n"), "- [-[ ]>] \n");
}

#[test]
fn a_selection_starting_inside_the_box_keeps_its_start() {
    assert_eq!(toggled("- [-[ ] a]>\n"), "- [-[x] a]>\n");
}

#[test]
fn a_task_item_in_a_quote_gets_checked() {
    assert_eq!(toggled("> - [ ] -[a]>\n"), "> - [x] -[a]>\n");
}

#[test]
fn dot_repeats_the_toggle_on_the_line_it_is_on() {
    let mut p = editing("- [ ] -[a]>\n- [ ] b\n");
    p.ed.feed_keys([key('G'), key('X'), key('j'), key('.')]);
    assert_eq!(state(&p.ed), "- [x] a\n- [x] -[b]>\n");
    p.ed.feed_keys([key('.')]);
    assert_eq!(state(&p.ed), "- [x] a\n- [ ] -[b]>\n");
}

#[test]
fn an_item_without_a_box_is_left_alone() {
    assert_eq!(toggled("- -[a]>\n"), "- -[a]>\n");
}

#[test]
fn a_plain_line_is_left_alone() {
    assert_eq!(toggled("-[[ ] a]>\n"), "-[[ ] a]>\n");
}

#[test]
fn continue_lists_off_still_toggles_a_box() {
    let mut p = editing_without_lists("- [ ] -[a]>\n");
    p.ed.feed_keys([key('G'), key('X')]);
    assert_eq!(state(&p.ed), "- [x] -[a]>\n");
}

#[test]
fn g_x_in_another_language_runs_its_global_binding() {
    let mut p = editing_plain("- [ ] -[a]>\n");
    bind_global_probe(&mut p, &[("normal", "G X")]);
    p.ed.feed_keys([key('G'), key('X')]);
    assert_eq!(state(&p.ed), "!- [ ] -[a]>\n");
}
