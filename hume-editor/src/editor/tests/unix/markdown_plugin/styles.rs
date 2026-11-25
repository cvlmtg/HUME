//! `M B`, `M I`, `M C` and `M S` toggle a Markdown inline style: they
//! remove it around the selections inside it, or else wrap each selection.

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
fn m_b_inside_bold_removes_it() {
    assert_eq!(after("**b-[o]>ld**\n", "MB"), "b-[o]>ld\n");
}

#[test]
fn m_i_inside_italic_removes_it() {
    assert_eq!(after("*i-[t]>*\n", "MI"), "i-[t]>\n");
}

#[test]
fn m_c_inside_code_removes_it() {
    assert_eq!(after("`-[x]>`\n", "MC"), "-[x]>\n");
}

#[test]
fn m_s_inside_strikethrough_removes_both_tildes() {
    assert_eq!(after("~~go-[n]>e~~\n", "MS"), "go-[n]>e\n");
}

#[test]
fn m_s_on_an_outer_tilde_removes_both_tildes() {
    assert_eq!(after("-[~]>~a~~\n", "MS"), "-[a]>\n");
    assert_eq!(after("~~a~-[~]>\n", "MS"), "-[a]>\n");
}

#[test]
fn m_s_on_the_whole_strikethrough_removes_both_tildes() {
    assert_eq!(after("-[~~a~~]>\n", "MS"), "-[a]>\n");
}

#[test]
fn m_b_inside_bold_italic_keeps_the_italic() {
    assert_eq!(after("***-[a]>***\n", "MB"), "*-[a]>*\n");
}

#[test]
fn m_i_inside_bold_italic_keeps_the_bold() {
    assert_eq!(after("***-[a]>***\n", "MI"), "**-[a]>**\n");
}

#[test]
fn m_b_twice_lands_on_the_last_character() {
    assert_eq!(after("-[bold]>\n", "MBMB"), "bol-[d]>\n");
}

#[test]
fn m_b_twice_on_cursors_gives_them_back() {
    assert_eq!(after("-[a]> -{b}>\n", "MBMB"), "-[a]> -{b}>\n");
}

#[test]
fn m_s_twice_lands_on_the_struck_character() {
    assert_eq!(after("-[s]>\n", "MSMS"), "-[s]>\n");
}

#[test]
fn a_cursor_inside_the_text_keeps_the_others_where_they_land() {
    assert_eq!(after("**a-{*}>* **x-[y]>z**\n", "MB"), "a-{ }>x-[y]>z\n");
}

#[test]
fn a_primary_inside_bold_removes_only_the_bold_around_selections() {
    assert_eq!(after("**-{a}>** -[b]>\n", "MB"), "-{a}> -[b]>\n");
}

#[test]
fn a_primary_outside_bold_wraps_every_selection() {
    assert_eq!(
        after("**-[a]>** -{b}>\n", "MB"),
        "****a-[*]>*** **b-{*}>*\n"
    );
}

#[test]
fn one_undo_puts_a_removed_style_back() {
    assert_eq!(after("**b-[o]>ld**\n", "MBu"), "**b-[o]>ld**\n");
}

#[test]
fn m_m_still_selects_the_word() {
    assert_eq!(after("a-[b]>c d\n", "MM"), "-[abc ]>d\n");
}
