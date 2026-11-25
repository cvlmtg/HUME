//! Enter, `o` and `O` on a Markdown list item start the next item.

use super::*;

/// The state after Enter in Insert mode, entered before each selection,
/// once the hooks the edit fires have run.
fn entered(marked: &str) -> String {
    let mut p = editing(marked);
    p.ed.feed_keys([key('i'), key_enter()]);
    p.ed.settle();
    state(&p.ed)
}

/// The state after `key` in Normal mode, once the hooks the edit fires
/// have run.
fn opened(marked: &str, key_char: char) -> String {
    let mut p = editing(marked);
    p.ed.feed_key(key(key_char));
    p.ed.settle();
    state(&p.ed)
}

/// Feeds each of `keys` and settles after it, as the run loop does.
fn feed_settled(p: &mut Preview, keys: impl IntoIterator<Item = KeyEvent>) {
    for k in keys {
        p.ed.feed_event(k);
    }
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
fn an_ordered_task_item_continues_numbered_with_an_unchecked_box() {
    assert_eq!(entered("1. [x] a-[\n]>"), "1. [x] a\n2. [ ] -[\n]>");
}

#[test]
fn a_task_bullet_after_an_ordered_marker_continues_the_inner_list_with_a_box() {
    assert_eq!(entered("1. - [ ] a-[\n]>"), "1. - [ ] a\n   - [ ] -[\n]>");
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
fn enter_on_an_empty_task_item_ends_the_list() {
    assert_eq!(entered("- [ ] a\n- [ ] -[\n]>"), "- [ ] a\n-[\n]>");
}

#[test]
fn enter_on_an_empty_checked_ordered_task_item_ends_the_list_and_renumbers() {
    assert_eq!(
        entered("1. [ ] a\n2. [x] -[\n]>3. [ ] c\n"),
        "1. [ ] a\n-[\n]>2. [ ] c\n"
    );
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
fn ending_an_ordered_item_with_no_blank_after_its_marker_renumbers() {
    assert_eq!(entered("1. a\n2.-[\n]>3. c\n"), "1. a\n-[\n]>2. c\n");
}

#[test]
fn ending_a_nested_item_on_an_ordered_line_renumbers_the_outer_list() {
    assert_eq!(entered("1. - -[\n]>2. b\n3. c\n"), "-[\n]>1. b\n2. c\n");
}

#[test]
fn a_quote_after_a_bullet_continues_the_quote_in_the_item() {
    assert_eq!(entered("- > foo-[\n]>"), "- > foo\n  > -[\n]>");
}

#[test]
fn a_quote_after_an_ordered_marker_adds_no_item() {
    assert_eq!(
        entered("1. > foo-[\n]>2. b\n"),
        "1. > foo\n   > -[\n]>2. b\n"
    );
}

#[test]
fn ending_an_empty_quote_after_a_marker_renumbers_the_list() {
    assert_eq!(entered("1. a\n2. > -[\n]>3. c\n"), "1. a\n-[\n]>2. c\n");
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
fn o_with_two_selections_on_one_item_opens_one_item() {
    assert_eq!(opened("- -[f]>oo -{b}>ar\n", 'o'), "- foo bar\n- -[\n]>");
}

#[test]
fn o_with_two_selections_on_one_ordered_item_renumbers_once() {
    assert_eq!(
        opened("1. -[a]> -{b}>\n2. c\n", 'o'),
        "1. a b\n2. -[\n]>3. c\n"
    );
}

#[test]
fn o_on_an_empty_task_item_opens_a_task_item() {
    assert_eq!(opened("- [ ] -[\n]>", 'o'), "- [ ] \n- [ ] -[\n]>");
}

#[test]
fn o_on_a_plain_line_opens_a_plain_line() {
    assert_eq!(opened("-[f]>oo\n- a\n", 'o'), "foo\n-[\n]>- a\n");
}

#[test]
fn one_undo_after_enter_brings_back_the_list() {
    let mut p = editing("1. a-[\n]>2. b\n");
    feed_settled(&mut p, [key('i'), key_enter(), key_esc(), key('u')]);
    assert_eq!(text(&p.ed), "1. a\n2. b\n");
}

#[test]
fn one_undo_after_o_brings_back_the_list() {
    let mut p = editing("-[1]>. a\n2. b\n");
    feed_settled(&mut p, [key('o'), key_esc(), key('u')]);
    assert_eq!(text(&p.ed), "1. a\n2. b\n");
}

#[test]
fn dot_repeats_o_with_its_list_item() {
    let mut p = editing("-[1]>. a\n2. b\n");
    feed_settled(&mut p, [key('o'), key('x'), key_esc(), key('.')]);
    assert_eq!(text(&p.ed), "1. a\n2. x\n3. x\n4. b\n");
}

#[test]
fn a_dot_repeated_open_undoes_its_renumbering_first() {
    let mut p = editing("-[1]>. a\n2. b\n");
    feed_settled(&mut p, [key('o'), key('x'), key_esc(), key('.'), key('u')]);
    assert_eq!(text(&p.ed), "1. a\n2. x\n3. x\n3. b\n");
    feed_settled(&mut p, [key('u')]);
    assert_eq!(text(&p.ed), "1. a\n2. x\n3. b\n");
}

#[test]
fn two_cursors_on_one_item_line_each_start_an_item() {
    assert_eq!(entered("- a-[b]>c-{d}>\n"), "- a\n- -[b]>c\n- -{d}>\n");
}

#[test]
fn two_cursors_in_one_list_each_start_an_item() {
    assert_eq!(
        entered("1. a-[\n]>2. b-{\n}>3. c\n"),
        "1. a\n2. -[\n]>3. b\n4. -{\n}>5. c\n"
    );
}

#[test]
fn enter_with_every_cursor_on_an_empty_item_ends_each_one() {
    assert_eq!(
        entered("- a\n- -[\n]>- b\n- -{\n}>"),
        "- a\n-[\n]>- b\n-{\n}>"
    );
}

#[test]
fn enter_with_a_cursor_on_a_full_item_continues_every_item() {
    assert_eq!(entered("- a-[\n]>- -{\n}>"), "- a\n- -[\n]>- \n- -{\n}>");
}

#[test]
fn shift_o_on_a_bullet_opens_an_item_before_it() {
    assert_eq!(opened("- -[a]>\n", 'O'), "- -[\n]>- a\n");
}

#[test]
fn o_and_shift_o_on_a_quote_line_continue_the_quote() {
    assert_eq!(opened("> -[a]>\n", 'o'), "> a\n> -[\n]>");
    assert_eq!(opened("> -[a]>\n", 'O'), "> -[\n]>> a\n");
}

#[test]
fn enter_and_o_in_another_language_run_their_global_bindings() {
    let mut p = editing_plain("- foo-[\n]>");
    bind_global_probe(&mut p, &[("insert", "enter"), ("normal", "o")]);
    p.ed.feed_keys([key('i'), key_enter()]);
    assert_eq!(state(&p.ed), "!- foo-[\n]>");
    p.ed.feed_keys([key_esc(), key('o')]);
    assert_eq!(state(&p.ed), "!!- foo-[\n]>");
}

#[test]
fn continue_lists_off_leaves_enter_as_usual() {
    let mut p = editing_without_lists("- foo-[\n]>");
    p.ed.feed_keys([key('i'), key_enter()]);
    assert_eq!(state(&p.ed), "- foo\n-[\n]>");
}

#[test]
fn continue_lists_off_leaves_o_and_shift_o_as_usual() {
    for (key_char, opened) in [('o', "- foo\n-[\n]>"), ('O', "-[\n]>- foo\n")] {
        let mut p = editing_without_lists("- -[f]>oo\n");
        p.ed.feed_key(key(key_char));
        p.ed.settle();
        assert_eq!(state(&p.ed), opened, "{key_char}");
    }
}

/// The plugin's editor on `marked` after `typed` is pressed, once the
/// edited text's tree has landed and the hooks it fires have run.
fn edited(marked: &str, typed: &str) -> Preview {
    let mut p = editing(marked);
    press(&mut p.ed, typed);
    p.render();
    p
}

/// Two ordered lists with a paragraph between them, the second one's first
/// number selected.
const SPLIT_LISTS: &str = "1. a\n2. b\n\nx\n\n-[3]>. c\n4. d\n";

#[test]
fn editing_an_item_number_renumbers_the_items_after_it() {
    let p = edited("1. a\n-[5]>. b\n6. c\n7. d\n", "r2");
    assert_eq!(text(&p.ed), "1. a\n2. b\n3. c\n4. d\n");
}

#[test]
fn a_list_after_a_paragraph_renumbers_from_its_new_first_number() {
    let p = edited(SPLIT_LISTS, "r1");
    assert_eq!(text(&p.ed), "1. a\n2. b\n\nx\n\n1. c\n2. d\n");
}

#[test]
fn typing_in_an_items_text_renumbers_nothing() {
    let mut p = edited("1. a\n1. b-[\n]>1. c\n", "ix");
    p.ed.feed_key(key_esc());
    p.render();
    assert_eq!(text(&p.ed), "1. a\n1. bx\n1. c\n");
}

#[test]
fn a_renumbering_while_typing_undoes_with_the_edit() {
    let mut p = editing(SPLIT_LISTS);
    press(&mut p.ed, "c1");
    p.ed.feed_event(key_esc());
    p.render();
    assert_eq!(text(&p.ed), "1. a\n2. b\n\nx\n\n1. c\n2. d\n");
    press(&mut p.ed, "u");
    p.render();
    assert_eq!(text(&p.ed), "1. a\n2. b\n\nx\n\n3. c\n4. d\n");
}

#[test]
fn saving_right_after_an_edit_keeps_its_renumbering() {
    let mut p = editing(SPLIT_LISTS);
    p.ed.feed_keys([key('r'), key('1'), key(':'), key('w'), key_enter()]);
    p.render();
    assert_eq!(text(&p.ed), "1. a\n2. b\n\nx\n\n1. c\n2. d\n");
}

#[test]
fn a_normal_mode_edit_undoes_its_renumbering_first() {
    let mut p = edited(SPLIT_LISTS, "r1");
    press(&mut p.ed, "u");
    p.render();
    assert_eq!(text(&p.ed), "1. a\n2. b\n\nx\n\n1. c\n4. d\n");
    press(&mut p.ed, "u");
    p.render();
    assert_eq!(text(&p.ed), "1. a\n2. b\n\nx\n\n3. c\n4. d\n");
}

#[test]
fn continue_lists_off_renumbers_nothing_after_an_edit() {
    let mut p = editing_without_lists(SPLIT_LISTS);
    press(&mut p.ed, "r1");
    p.render();
    assert_eq!(text(&p.ed), "1. a\n2. b\n\nx\n\n1. c\n4. d\n");
}

#[test]
fn a_pasted_run_of_items_keeps_its_numbers() {
    let mut p = editing("1. a\n-[2]>. b\n3. c\n");
    press(&mut p.ed, "x");
    seed_ring(&mut p.ed, &["7. x\n8. y\n"]);
    p.ed.feed_key(key('p'));
    p.render();
    assert_eq!(text(&p.ed), "1. a\n7. x\n8. y\n9. c\n");
}

#[test]
fn deleting_an_item_renumbers_the_items_after_it() {
    let p = edited("1. a\n-[2]>. b\n3. c\n4. d\n", "xd");
    assert_eq!(text(&p.ed), "1. a\n2. c\n3. d\n");
}

#[test]
fn deleting_a_nested_item_renumbers_only_its_list() {
    let p = edited("1. a\n   1. x\n   -[2]>. y\n   3. z\n2. b\n", "xd");
    assert_eq!(text(&p.ed), "1. a\n   1. x\n   2. z\n2. b\n");
}

#[test]
fn deleting_the_first_item_renumbers_the_rest_from_its_number() {
    let p = edited("-[1]>. a\n2. b\n3. c\n", "xd");
    assert_eq!(text(&p.ed), "1. b\n2. c\n");
}

#[test]
fn blanking_the_first_items_text_renumbers_the_rest() {
    let p = edited("-[1. a]>\n2. b\n3. c\n", "d");
    assert_eq!(text(&p.ed), "\n1. b\n2. c\n");
}

#[test]
fn deleting_the_first_item_of_a_loose_list_renumbers_the_rest() {
    let p = edited("-[1]>. a\n\n2. b\n\n3. c\n", "xd");
    assert_eq!(text(&p.ed), "\n1. b\n\n2. c\n");
}

#[test]
fn deleting_a_list_leaves_the_next_list_with_another_delimiter_alone() {
    let p = edited("-[3]>. a\n1) b\n2) c\n", "xd");
    assert_eq!(text(&p.ed), "1) b\n2) c\n");
}

#[test]
fn editing_a_number_leaves_the_next_list_with_another_delimiter_alone() {
    let p = edited("-[3]>. a\n1) b\n", "r5");
    assert_eq!(text(&p.ed), "5. a\n1) b\n");
}

#[test]
fn a_deletion_undoes_its_renumbering_first() {
    let mut p = edited("1. a\n-[2]>. b\n3. c\n4. d\n", "xd");
    press(&mut p.ed, "u");
    p.render();
    assert_eq!(text(&p.ed), "1. a\n3. c\n4. d\n");
    press(&mut p.ed, "u");
    p.render();
    assert_eq!(text(&p.ed), "1. a\n2. b\n3. c\n4. d\n");
}

#[test]
fn deleting_a_word_in_an_items_text_renumbers_nothing() {
    let p = edited("1. a\n3. b -[c]>\n7. d\n", "d");
    assert_eq!(text(&p.ed), "1. a\n3. b \n7. d\n");
}

#[test]
fn indenting_an_item_numbers_it_in_its_new_list_and_renumbers_the_list_it_left() {
    let p = edited("1. a\n-[2]>. b\n3. c\n", ">");
    assert_eq!(text(&p.ed), "1. a\n\t1. b\n2. c\n");
}

/// [`edited`] with soft indentation `width` columns wide, each key of
/// `typed` pressed and rendered after in turn.
fn indented(marked: &str, width: usize, typed: &str) -> Preview {
    let mut p = editing(marked);
    let tmp = p.tmp.path().to_path_buf();
    run(
        &mut p.ed,
        &tmp,
        &format!(r#"(set-option! "indent-style" "soft") (set-option! "indent-width" {width})"#),
    );
    for k in typed.chars() {
        press(&mut p.ed, &k.to_string());
        p.render();
    }
    p
}

#[test]
fn indenting_items_into_a_new_nested_list_numbers_them_from_one() {
    let p = indented("1. a\n2. b\n-[3. c\n4. d]>\n5. e\n", 2, ">>");
    assert_eq!(text(&p.ed), "1. a\n2. b\n    1. c\n    2. d\n3. e\n");
}

#[test]
fn indenting_an_item_into_a_nested_list_counts_on_from_its_items() {
    let p = indented("1. a\n   1. x\n-[3]>. b\n4. c\n", 3, ">");
    assert_eq!(text(&p.ed), "1. a\n   1. x\n   2. b\n2. c\n");
}

#[test]
fn indenting_an_item_into_a_list_numbered_the_same_gives_it_that_number() {
    let p = indented("1. a\n   1. x\n   1. y\n-[7]>. b\n", 3, ">");
    assert_eq!(text(&p.ed), "1. a\n   1. x\n   1. y\n   1. b\n");
}

#[test]
fn outdenting_an_item_counts_on_in_the_outer_list() {
    let p = indented("1. a\n   1. x\n   -[5]>. y\n2. b\n", 3, "<");
    assert_eq!(text(&p.ed), "1. a\n   1. x\n2. y\n3. b\n");
}

#[test]
fn deleting_an_item_indented_unlike_the_rest_renumbers_its_list() {
    let p = edited("1. a\n  -[2]>. b\n3. c\n", "xd");
    assert_eq!(text(&p.ed), "1. a\n2. c\n");
}

/// A list whose items all carry one number.
const SAME_NUMBER: &str = "1. a\n-[1]>. b\n1. c\n1. d\n";

#[test]
fn deleting_from_a_list_numbered_the_same_keeps_its_numbers() {
    let p = edited(SAME_NUMBER, "xd");
    assert_eq!(text(&p.ed), "1. a\n1. c\n1. d\n");
}

#[test]
fn undoing_a_deletion_from_a_list_numbered_the_same_restores_it() {
    let mut p = edited(SAME_NUMBER, "xd");
    press(&mut p.ed, "u");
    p.render();
    assert_eq!(text(&p.ed), "1. a\n1. b\n1. c\n1. d\n");
}

#[test]
fn enter_in_a_list_numbered_the_same_gives_the_new_item_its_number() {
    assert_eq!(entered("1. a-[\n]>1. b\n"), "1. a\n1. -[\n]>1. b\n");
}

#[test]
fn o_in_a_list_numbered_the_same_gives_the_new_item_its_number() {
    assert_eq!(opened("-[1]>. a\n1. b\n", 'o'), "1. a\n1. -[\n]>1. b\n");
}

/// An item indented past its parent's text: under `2. b`, whose text starts
/// at column 3, the parser counts the fourth blank as part of the marker.
const PAST_THE_PARENT: &str = "1. a\n2. b\n    3. c-[\n]>";

#[test]
fn enter_after_an_item_indented_past_its_parent_continues_it() {
    assert_eq!(
        entered(PAST_THE_PARENT),
        "1. a\n2. b\n    3. c\n    4. -[\n]>"
    );
}

#[test]
fn editing_the_number_of_an_item_indented_past_its_parent_renumbers_its_list() {
    let p = edited("1. a\n    -[5]>. b\n    6. c\n", "r3");
    assert_eq!(text(&p.ed), "1. a\n    3. b\n    4. c\n");
}
