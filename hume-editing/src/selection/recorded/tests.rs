use pretty_assertions::assert_eq;

use super::*;
use crate::error::InvariantViolation;
use crate::marked::{parse, render};

#[test]
fn a_recorded_set_binds_to_a_text_with_its_content() {
    let state = parse("a-[bc]>\n");
    let rebound = state
        .recorded()
        .bind(BufferText::from("abc\n"))
        .expect("the content is the recorded one");
    assert_eq!(render(rebound.view()), "a-[bc]>\n");
}

#[test]
fn a_recorded_set_is_refused_by_a_text_it_does_not_fit() {
    let state = parse("abc-[d]>\n");
    let refused = state.recorded().bind(BufferText::from("a\n"));
    assert_eq!(
        refused.err(),
        Some(InvariantViolation::OutOfBounds { index: 0 })
    );
}

#[test]
fn a_recorded_set_refits_into_a_shorter_text() {
    let state = parse("abc-[d]>\n");
    let refitted = state.recorded().refit(BufferText::from("ab\n"));
    assert_eq!(render(refitted.view()), "ab-[\n]>");
}
