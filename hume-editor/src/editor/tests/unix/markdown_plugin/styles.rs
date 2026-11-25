//! `M B`, `M I`, `M C` and `M S` wrap each selection in a Markdown inline
//! style.

use super::*;

#[test]
fn m_b_makes_the_selection_bold() {
    assert_eq!(after("-[bold]>\n", "MB"), "**bold-[*]>*\n");
}

#[test]
fn m_b_makes_the_character_under_a_cursor_bold() {
    assert_eq!(after("-[a]>\n", "MB"), "**a-[*]>*\n");
}

#[test]
fn m_i_makes_the_selection_italic() {
    assert_eq!(after("-[it]>\n", "MI"), "*it-[*]>\n");
}

#[test]
fn m_c_makes_the_selection_code() {
    assert_eq!(after("-[x]>\n", "MC"), "`x-[`]>\n");
}

#[test]
fn m_s_strikes_the_selection_through() {
    assert_eq!(after("-[gone]>\n", "MS"), "~~gone-[~]>~\n");
}

#[test]
fn m_s_strikes_the_character_under_a_cursor_through() {
    assert_eq!(after("-[a]>\n", "MS"), "~~a-[~]>~\n");
}

#[test]
fn every_selection_gets_the_style() {
    assert_eq!(after("-[ab]> -{cd}>\n", "MB"), "**ab-[*]>* **cd-{*}>*\n");
}

#[test]
fn every_cursor_gets_the_style() {
    assert_eq!(after("-[a]> -{b}>\n", "MB"), "**a-[*]>* **b-{*}>*\n");
}

#[test]
fn dot_repeats_the_style_on_a_cursor() {
    assert_eq!(after("-[a]>\nb\n", "MBjgh."), "**a**\n**b-[*]>*\n");
}

#[test]
fn m_m_still_selects_the_word() {
    assert_eq!(after("a-[b]>c d\n", "MM"), "-[abc ]>d\n");
}
