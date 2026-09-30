//! Every text object runs over the shared unicode corpus: the sample sits
//! inside delimiters or on a line of its own.

use super::super::*;
use test_fixtures::assert_state;
use test_fixtures::unicode::{ALL, standalone};

#[test]
fn inner_paren_selects_the_sample() {
    for s in standalone() {
        assert_state!(
            &format!("x\n(-[{s}]>)\n"),
            |(text, sels)| cmd_inner_paren(
                test_fixtures::testing::state(text, sels),
                0,
                MotionMode::Move
            ),
            &format!("x\n(-[{s}]>)\n")
        );
    }
}

#[test]
fn around_paren_selects_the_delimiters_and_the_sample() {
    for s in standalone() {
        assert_state!(
            &format!("x\n(-[{s}]>)\n"),
            |(text, sels)| cmd_around_paren(
                test_fixtures::testing::state(text, sels),
                0,
                MotionMode::Move
            ),
            &format!("x\n-[({s})]>\n")
        );
    }
}

#[test]
fn inner_double_quote_selects_the_sample() {
    for s in standalone() {
        assert_state!(
            &format!("x\n\"-[{s}]>\"\n"),
            |(text, sels)| cmd_inner_double_quote(
                test_fixtures::testing::state(text, sels),
                0,
                MotionMode::Move
            ),
            &format!("x\n\"-[{s}]>\"\n")
        );
    }
}

#[test]
fn around_double_quote_selects_the_quotes_and_the_sample() {
    for s in standalone() {
        assert_state!(
            &format!("x\n\"-[{s}]>\"\n"),
            |(text, sels)| cmd_around_double_quote(
                test_fixtures::testing::state(text, sels),
                0,
                MotionMode::Move
            ),
            &format!("x\n-[\"{s}\"]>\n")
        );
    }
}

#[test]
fn inner_line_covers_the_sample_and_the_rest_of_the_line() {
    for s in ALL {
        assert_state!(
            &format!("x\n-[{s}]>b\n"),
            |(text, sels)| cmd_inner_line(
                test_fixtures::testing::state(text, sels),
                0,
                MotionMode::Move
            ),
            &format!("x\n-[{s}b]>\n")
        );
    }
}

#[test]
fn around_line_covers_the_sample_and_the_newline() {
    for s in ALL {
        assert_state!(
            &format!("x\n-[{s}]>b\n"),
            |(text, sels)| cmd_around_line(
                test_fixtures::testing::state(text, sels),
                0,
                MotionMode::Move
            ),
            &format!("x\n-[{s}b\n]>")
        );
    }
}

fn is_word_sample(s: &str) -> bool {
    s.chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
}

#[test]
fn inner_word_groups_a_sample_with_its_neighbours_only_when_it_is_a_word_char() {
    for s in standalone() {
        let input = format!("x\nab-[{s}]>cd\n");
        let expected = if is_word_sample(s) {
            format!("x\n-[ab{s}cd]>\n")
        } else {
            format!("x\nab-[{s}]>cd\n")
        };
        assert_state!(
            &input,
            |(text, sels)| cmd_inner_word(
                test_fixtures::testing::state(text, sels),
                0,
                crate::WordCtx::bare(MotionMode::Move)
            ),
            &expected
        );
    }
}

#[test]
fn select_next_word_lands_on_the_whole_sample() {
    for s in standalone().filter(|s| {
        !s.chars()
            .next()
            .is_some_and(|c| c.is_whitespace() || c == '\u{3000}')
    }) {
        assert_state!(
            &format!("x\n-[a]> {s} b\n"),
            |(text, sels)| crate::motion::cmd_select_next_word(
                test_fixtures::testing::state(text, sels),
                1,
                crate::WordCtx::bare(MotionMode::Move)
            ),
            &format!("x\na -[{s}]> b\n")
        );
    }
}
