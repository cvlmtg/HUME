//! Enter, `o` and `O` on a Markdown list item start the next item.

use super::*;

/// The plugin's editor on `a.md` holding `marked`'s text, selected as
/// `marked` marks it.
fn editing(marked: &str) -> Preview {
    let mut p = preview();
    p.open_marked("a.md", marked);
    p
}

/// The state after Enter in Insert mode, entered before each selection.
fn entered(marked: &str) -> String {
    let mut p = editing(marked);
    p.ed.feed_keys([key('i'), key_enter()]);
    state(&p.ed)
}

/// The state after `key` in Normal mode.
fn opened(marked: &str, key_char: char) -> String {
    let mut p = editing(marked);
    p.ed.feed_key(key(key_char));
    state(&p.ed)
}

#[test]
fn enter_after_a_bullet_item_starts_the_next_one() {
    assert_eq!(entered("- foo-[\n]>"), "- foo\n- -[\n]>");
}

#[test]
fn enter_inside_an_item_splits_it_into_two_items() {
    assert_eq!(entered("- fo-[o]>\n"), "- fo\n- -[o]>\n");
}

#[test]
fn enter_after_an_ordered_item_renumbers_the_items_after_it() {
    assert_eq!(
        entered("1. a\n2. b-[\n]>3. c\n"),
        "1. a\n2. b\n3. -[\n]>4. c\n"
    );
}

#[test]
fn enter_keeps_a_parenthesis_delimiter() {
    assert_eq!(entered("1) a-[\n]>2) b\n"), "1) a\n2) -[\n]>3) b\n");
}

#[test]
fn renumbering_past_nine_widens_the_marker() {
    assert_eq!(entered("9. a-[\n]>10. b\n"), "9. a\n10. -[\n]>11. b\n");
}

#[test]
fn a_task_item_continues_with_an_unchecked_box() {
    assert_eq!(entered("- [x] done-[\n]>"), "- [x] done\n- [ ] -[\n]>");
}

#[test]
fn a_nested_bullet_continues_at_its_own_indent() {
    assert_eq!(entered("- a\n  - b-[\n]>"), "- a\n  - b\n  - -[\n]>");
}

#[test]
fn a_nested_ordered_list_renumbers_only_itself() {
    assert_eq!(
        entered("1. a\n   1. b-[\n]>   2. c\n2. d\n"),
        "1. a\n   1. b\n   2. -[\n]>   3. c\n2. d\n"
    );
}

#[test]
fn an_item_opening_a_nested_list_continues_the_inner_one() {
    assert_eq!(entered("1. - foo-[\n]>"), "1. - foo\n   - -[\n]>");
}

#[test]
fn a_quote_line_continues_the_quote() {
    assert_eq!(entered("> foo-[\n]>"), "> foo\n> -[\n]>");
}

#[test]
fn a_list_in_a_quote_continues_and_renumbers() {
    assert_eq!(
        entered("> 1. a-[\n]>> 2. b\n"),
        "> 1. a\n> 2. -[\n]>> 3. b\n"
    );
}

#[test]
fn enter_on_an_empty_bullet_ends_the_list() {
    assert_eq!(entered("- a\n- -[\n]>"), "- a\n-[\n]>");
}

#[test]
fn enter_on_an_empty_item_in_a_quote_keeps_the_quote() {
    assert_eq!(entered("> 1. a\n> 2. -[\n]>"), "> 1. a\n> -[\n]>");
}

#[test]
fn enter_on_an_empty_quote_line_ends_the_quote() {
    assert_eq!(entered("> a\n> -[\n]>"), "> a\n-[\n]>");
}

#[test]
fn ending_an_ordered_list_renumbers_the_items_after_it() {
    assert_eq!(entered("1. a\n2. -[\n]>3. c\n"), "1. a\n-[\n]>2. c\n");
}

#[test]
fn enter_before_the_marker_breaks_the_line_as_usual() {
    assert_eq!(entered("-[-]> foo\n"), "\n-[-]> foo\n");
}

#[test]
fn enter_on_a_continuation_line_breaks_the_line_as_usual() {
    assert_eq!(entered("- a\n  b-[\n]>"), "- a\n  b\n  -[\n]>");
}

#[test]
fn enter_on_a_bullet_in_a_fenced_block_breaks_the_line_as_usual() {
    assert_eq!(entered("```\n- foo-[\n]>```\n"), "```\n- foo\n-[\n]>```\n");
}

#[test]
fn o_on_an_ordered_item_opens_the_next_item() {
    assert_eq!(
        opened("1. a\n-[2]>. b\n3. c\n", 'o'),
        "1. a\n2. b\n3. -[\n]>4. c\n"
    );
}

#[test]
fn shift_o_on_an_ordered_item_opens_an_item_before_it() {
    assert_eq!(
        opened("1. a\n-[2]>. b\n3. c\n", 'O'),
        "1. a\n2. -[\n]>3. b\n4. c\n"
    );
}

#[test]
fn o_on_a_plain_line_opens_a_plain_line() {
    assert_eq!(opened("-[f]>oo\n- a\n", 'o'), "foo\n-[\n]>- a\n");
}

#[test]
fn one_undo_after_enter_brings_back_the_list() {
    let mut p = editing("1. a-[\n]>2. b\n");
    p.ed.feed_keys([key('i'), key_enter(), key_esc(), key('u')]);
    assert_eq!(text(&p.ed), "1. a\n2. b\n");
}

#[test]
fn one_undo_after_o_brings_back_the_list() {
    let mut p = editing("-[1]>. a\n2. b\n");
    p.ed.feed_keys([key('o'), key_esc(), key('u')]);
    assert_eq!(text(&p.ed), "1. a\n2. b\n");
}

#[test]
fn two_cursors_in_one_list_each_start_an_item() {
    assert_eq!(
        entered("1. a-[\n]>2. b-{\n}>3. c\n"),
        "1. a\n2. -[\n]>3. b\n4. -{\n}>5. c\n"
    );
}

#[test]
fn enter_and_o_in_another_language_break_the_line_as_usual() {
    let mut p = preview();
    p.open("a.md", "x\n");
    p.ed.settle();
    p.open("a.txt", "- foo\n");
    p.ed.settle();
    select_marked(&mut p.ed, "- foo-[\n]>");
    p.ed.feed_keys([key('i'), key_enter()]);
    assert_eq!(state(&p.ed), "- foo\n-[\n]>");
    p.ed.feed_keys([key_esc(), key('k'), key('o')]);
    assert_eq!(state(&p.ed), "- foo\n-[\n]>\n");
}

#[test]
fn continue_lists_off_leaves_enter_as_usual() {
    let mut p = preview_with(Some("(hash 'continue-lists #f)"), true);
    p.open_marked("a.md", "- foo-[\n]>");
    p.ed.feed_keys([key('i'), key_enter()]);
    assert_eq!(state(&p.ed), "- foo\n-[\n]>");
}
