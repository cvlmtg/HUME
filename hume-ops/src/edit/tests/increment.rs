use super::super::*;
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use test_fixtures::assert_state;
use test_fixtures::testing::{parse_state, state};
use test_fixtures::unicode::{CJK, COMBINING};

fn stepped(step: i128, stepping: Stepping) -> impl Fn((BufferText, SelectionSet)) -> Edited {
    move |(text, sels)| {
        let state = state(text, sels);
        let plan =
            plan_increment(state.view(), step, stepping).expect("the selections hold a target");
        apply_increment_plan(state, plan)
    }
}

fn by(step: i128) -> impl Fn((BufferText, SelectionSet)) -> Edited {
    stepped(step, Stepping::Uniform)
}

fn sequence(step: i128) -> impl Fn((BufferText, SelectionSet)) -> Edited {
    stepped(step, Stepping::Sequence)
}

fn refusal(input: &str, step: i128) -> IncrementRefusal {
    let (text, sels) = parse_state(input);
    let state = state(text, sels);
    plan_increment(state.view(), step, Stepping::Uniform).expect_err("no plan")
}

/// Whether stepping `input` by `step` leaves its text as it was, with an
/// identity changeset.
fn changes_nothing(input: &str, step: i128) -> bool {
    let (_, _, cs) = test_fixtures::testing::parts(by(step)(parse_state(input)));
    cs.is_identity()
}

// ── Finding the target ────────────────────────────────────────────────────

#[test]
fn a_cursor_on_a_digit_steps_its_number_and_selects_it() {
    assert_state!("x = 4-[1]>;\n", by(1), "x = -[42]>;\n");
}

#[test]
fn a_cursor_before_a_number_reaches_it_on_the_same_line() {
    assert_state!("-[x]> = 41;\n", by(1), "x = -[42]>;\n");
}

#[test]
fn a_cursor_does_not_reach_a_number_on_the_next_line() {
    assert_eq!(refusal("-[x]>\n1\n", 1), IncrementRefusal::NoTarget);
}

#[test]
fn a_cursor_after_the_last_number_has_no_target() {
    assert_eq!(refusal("1 -[x]>\n", 1), IncrementRefusal::NoTarget);
}

#[test]
fn a_wide_selection_steps_the_number_touching_its_start() {
    assert_state!("1-[2 x]> 7\n", by(1), "-[13]> x 7\n");
}

#[test]
fn a_wide_selection_steps_the_first_number_inside_it() {
    assert_state!("-[a 1 b 2]>\n", by(1), "a -[2]> b 2\n");
}

#[test]
fn a_wide_selection_reaches_a_number_on_a_later_line_it_covers() {
    assert_state!("-[a\nb 5]>\n", by(1), "a\nb -[6]>\n");
}

#[test]
fn a_wide_selection_does_not_reach_past_its_end() {
    assert_eq!(refusal("-[ab]> 1\n", 1), IncrementRefusal::NoTarget);
}

#[test]
fn a_backward_selection_keeps_its_facing() {
    assert_state!("<[12]-\n", by(1), "<[13]-\n");
}

#[test]
fn a_selection_without_a_target_is_left_as_it_is() {
    assert_state!("-{1}>\nx-[y]>\n", by(1), "-{2}>\nx-[y]>\n");
}

#[test]
fn two_selections_on_one_number_step_it_once() {
    assert_state!("-{1}>-[2]>\n", by(1), "-[13]>\n");
}

// ── Decimal numbers ───────────────────────────────────────────────────────

#[test]
fn the_step_is_the_count() {
    assert_state!("-[1]>0\n", by(5), "-[15]>\n");
    assert_state!("-[1]>0\n", by(-15), "-[-5]>\n");
}

#[test]
fn a_minus_after_a_non_word_char_is_a_sign() {
    assert_state!("a -[=]> -1\n", by(1), "a = -[0]>\n");
    assert_state!("-[x]> = -1\n", by(3), "x = -[2]>\n");
    assert_state!("f(-[-]>3)\n", by(1), "f(-[-2]>)\n");
}

#[test]
fn a_minus_after_a_word_char_is_a_separator() {
    assert_state!("x-[-]>1\n", by(1), "x--[2]>\n");
    assert_state!("1-[-]>1\n", by(1), "1--[2]>\n");
}

#[test]
fn a_minus_after_a_closing_bracket_is_a_separator() {
    assert_state!("f(x)-[-]>1\n", by(1), "f(x)--[2]>\n");
    assert_state!("a[i]-[-]>1\n", by(1), "a[i]--[2]>\n");
    assert_state!("{a}-[-]>1\n", by(1), "{a}--[2]>\n");
}

#[test]
fn a_number_after_a_word_char_or_closing_bracket_stops_at_zero() {
    assert_state!("ite-[m]>3\n", by(-5), "item-[0]>\n");
    assert_state!("f(x)-[1]>\n", by(-2), "f(x)-[0]>\n");
    assert!(changes_nothing("-[v]>0\n", -1));
}

#[test]
fn digits_inside_an_identifier_step() {
    assert_state!("ite-[m]>1\n", by(1), "item-[2]>\n");
}

#[test]
fn leading_zeros_keep_the_width() {
    assert_state!("-[0]>07\n", by(1), "-[008]>\n");
    assert_state!("-[0]>99\n", by(1), "-[100]>\n");
    assert_state!("-[-]>001\n", by(2), "-[001]>\n");
}

#[test]
fn no_leading_zero_means_no_padding() {
    assert_state!("-[1]>0\n", by(-1), "-[9]>\n");
}

#[test]
fn underscore_groups_follow_the_last_group() {
    assert_state!("-[9]>99_999\n", by(1), "-[1_000_000]>\n");
    assert_state!("-[1]>_0000\n", by(-1), "-[9999]>\n");
}

#[test]
fn a_dot_separates_two_numbers() {
    assert_state!("1.-[5]>\n", by(1), "1.-[6]>\n");
    assert_state!("-[1]>.2.3\n", by(1), "-[2]>.2.3\n");
}

#[test]
fn a_result_past_the_integer_range_is_refused() {
    let max = format!("-[{}]>\n", i128::MAX);
    assert_eq!(refusal(&max, 1), IncrementRefusal::OutOfRange);
    let huge = format!("-[1]>{}\n", "0".repeat(40));
    assert_eq!(refusal(&huge, 1), IncrementRefusal::OutOfRange);
}

// ── Prefixed numbers ──────────────────────────────────────────────────────

#[test]
fn hex_octal_and_binary_step_in_their_base() {
    assert_state!("-[0]>xff\n", by(1), "-[0x100]>\n");
    assert_state!("-[0]>o7\n", by(1), "-[0o10]>\n");
    assert_state!("-[0]>b11\n", by(1), "-[0b100]>\n");
}

#[test]
fn a_prefixed_number_keeps_its_width_and_case() {
    assert_state!("-[0]>x00ff\n", by(1), "-[0x0100]>\n");
    assert_state!("-[0]>XFF\n", by(1), "-[0X100]>\n");
    assert_state!("-[0]>xAb\n", by(1), "-[0xAC]>\n");
}

#[test]
fn a_prefixed_number_keeps_its_underscore_groups() {
    assert_state!("-[0]>xff_ff\n", by(1), "-[0x1_00_00]>\n");
}

#[test]
fn a_prefixed_number_stops_at_zero() {
    assert_state!("-[0]>x1\n", by(-5), "-[0x0]>\n");
}

#[test]
fn a_prefixed_number_at_zero_changes_nothing_going_down() {
    assert!(changes_nothing("-[0]>x0\n", -1));
}

#[test]
fn a_minus_before_a_prefixed_number_is_not_its_sign() {
    assert_state!("a -[-]>0x10\n", by(1), "a --[0x11]>\n");
}

#[test]
fn a_prefix_inside_a_word_is_not_a_prefix() {
    assert_state!("a-[0]>x1\n", by(1), "a-[1]>x1\n");
    assert_state!("-[0]>xfg\n", by(1), "-[1]>xfg\n");
}

// ── Dates and times ───────────────────────────────────────────────────────

#[test]
fn a_cursor_steps_the_date_field_it_is_on() {
    assert_state!("-[2]>026-10-18\n", by(1), "-[2027]>-10-18\n");
    assert_state!("2026-1-[0]>-18\n", by(1), "2026--[11]>-18\n");
    assert_state!("2026-10-1-[8]>\n", by(1), "2026-10--[19]>\n");
}

#[test]
fn a_separator_belongs_to_the_field_before_it() {
    assert_state!("2026-[-]>10-18\n", by(1), "-[2027]>-10-18\n");
}

#[test]
fn a_day_step_carries_into_the_month_and_year() {
    assert_state!("2026-10-3-[1]>\n", by(1), "2026-11--[01]>\n");
    assert_state!("2026/12/3-[1]>\n", by(1), "2027/01/-[01]>\n");
    assert_state!("2026-03-0-[1]>\n", by(-1), "2026-02--[28]>\n");
}

#[test]
fn a_month_or_year_step_clamps_the_day() {
    assert_state!("2024-0-[1]>-31\n", by(1), "2024--[02]>-29\n");
    assert_state!("-[2]>024-02-29\n", by(1), "-[2025]>-02-28\n");
}

#[test]
fn a_date_reached_after_the_cursor_steps_its_day() {
    assert_state!("-[d]>ue 2026-10-18\n", by(1), "due 2026-10--[19]>\n");
}

#[test]
fn a_date_outside_four_digit_years_is_refused() {
    assert_eq!(refusal("-[9]>999-12-31\n", 1), IncrementRefusal::OutOfRange);
}

#[test]
fn an_invalid_date_is_a_run_of_numbers() {
    assert_state!("2026-13-4-[5]>\n", by(1), "2026-13--[46]>\n");
}

#[test]
fn a_datetime_time_field_carries_into_the_date() {
    assert_state!("2026-10-18 23:5-[9]>\n", by(1), "2026-10-19 00:-[00]>\n");
    assert_state!(
        "2026-12-31T23:59:5-[9]>\n",
        by(1),
        "2027-01-01T00:00:-[00]>\n"
    );
}

#[test]
fn a_datetime_date_field_keeps_the_time() {
    assert_state!("2026-01-3-[1]> 10:30\n", by(1), "2026-02--[01]> 10:30\n");
}

#[test]
fn a_datetime_with_an_invalid_time_is_a_date() {
    assert_state!("2026-10-1-[8]> 25:00\n", by(1), "2026-10--[19]> 25:00\n");
}

#[test]
fn a_time_wraps_around_midnight() {
    assert_state!("23:5-[9]>\n", by(1), "00:-[00]>\n");
    assert_state!("-[0]>0:00:00\n", by(-1), "-[23]>:00:00\n");
}

#[test]
fn a_time_stepped_by_a_whole_day_changes_nothing() {
    assert!(changes_nothing("10:3-[0]>\n", 24 * 60));
}

#[test]
fn a_stepped_field_steps_again() {
    assert_state!("2026-10--[19]>\n", by(1), "2026-10--[20]>\n");
    assert_state!("10:-[30]>\n", by(1), "10:-[31]>\n");
}

#[test]
fn selections_on_one_date_each_step_their_own_field() {
    assert_state!("-{2}>026-10-1-[8]>\n", by(1), "-{2027}>-10--[19]>\n");
    assert_state!("-[2]>024-0-{1}>-31\n", by(1), "-[2025]>--{02}>-28\n");
}

#[test]
fn a_time_reached_after_the_cursor_steps_its_minutes() {
    assert_state!("-[a]>t 10:30\n", by(15), "at 10:-[45]>\n");
}

#[test]
fn a_time_after_a_colon_is_a_run_of_numbers() {
    assert_state!("1:23:4-[5]>\n", by(1), "1:23:-[46]>\n");
}

#[test]
fn a_minus_before_a_date_is_not_its_sign() {
    assert_state!("-[x]> -2026-10-18\n", by(1), "x -2026-10--[19]>\n");
}

// ── Sequences ─────────────────────────────────────────────────────────────

#[test]
fn a_sequence_steps_the_nth_target_n_times() {
    assert_state!(
        "-{1}>\n-[1]>\n-[1]>\n",
        sequence(1),
        "-{2}>\n-[3]>\n-[4]>\n"
    );
    assert_state!(
        "-{1}>\n-[1]>\n-[1]>\n",
        sequence(5),
        "-{6}>\n-[11]>\n-[16]>\n"
    );
    assert_state!("-{5}>\n-[5]>\n", sequence(-1), "-{4}>\n-[3]>\n");
}

#[test]
fn a_uniform_step_changes_every_target_alike() {
    assert_state!("-{1}>\n-[1]>\n-[1]>\n", by(1), "-{2}>\n-[2]>\n-[2]>\n");
}

#[test]
fn a_sequence_counts_only_selections_with_a_target() {
    assert_state!(
        "-{1}>\nx-[y]>\n-[1]>\n",
        sequence(1),
        "-{2}>\nx-[y]>\n-[3]>\n"
    );
}

#[test]
fn a_sequence_counts_a_shared_target_once() {
    assert_state!("-{1}>-[2]> -[1]>\n", sequence(1), "-{13}> -[3]>\n");
}

// ── Clusters ──────────────────────────────────────────────────────────────

#[test]
fn a_digit_carrying_a_combining_mark_is_not_a_digit() {
    assert_state!("-[x]> 1\u{301} 2\n", by(1), "x 1\u{301} -[3]>\n");
}

#[test]
fn a_digit_after_a_prepend_mark_is_not_a_digit() {
    assert_state!("-[x]> \u{600}7 8\n", by(1), "x \u{600}7 -[9]>\n");
}

#[test]
fn wide_and_combined_letters_before_a_number_keep_their_place() {
    let input = format!("{CJK}{COMBINING} -[4]>\n");
    let expected = format!("{CJK}{COMBINING} -[5]>\n");
    assert_state!(&input, by(1), &expected);
}

#[test]
fn a_minus_after_a_combined_letter_is_a_separator() {
    let input = format!("{COMBINING}-[-]>1\n");
    let expected = format!("{COMBINING}--[2]>\n");
    assert_state!(&input, by(1), &expected);
}
