use super::*;
use crate::color::Rgb;
use crate::style::UnderlineStyle;

fn styled() -> ResolvedStyle {
    ResolvedStyle {
        fg: Some(Rgb(1, 2, 3)),
        ..Default::default()
    }
}

#[test]
fn glyph_stores_text_and_advance() {
    let c = Cell::glyph("コ", 2, styled());
    assert_eq!(c.text(), "コ");
    assert_eq!(c.advance(), 2);
    assert_eq!(c.style(), styled());
    assert!(!c.is_continuation());
}

#[test]
fn glyph_clamps_zero_advance_to_one() {
    assert_eq!(Cell::glyph("x", 0, ResolvedStyle::default()).advance(), 1);
}

#[test]
fn blank_is_one_space_column() {
    let c = Cell::blank(styled());
    assert_eq!(c.text(), " ");
    assert_eq!(c.advance(), 1);
    assert!(!c.is_continuation());
}

#[test]
fn continuation_has_no_text_and_no_advance() {
    let c = Cell::continuation(styled());
    assert_eq!(c.text(), "");
    assert_eq!(c.advance(), 0);
    assert!(c.is_continuation());
}

#[test]
fn continuation_carries_the_heads_style() {
    // Load-bearing for the diff: a continuation differs between frames
    // exactly when its head does.
    assert_eq!(Cell::continuation(styled()).style(), styled());
}

#[test]
fn default_is_a_blank_in_terminal_colours() {
    let c = Cell::default();
    assert_eq!(c.text(), " ");
    assert_eq!(c.advance(), 1);
    assert_eq!(c.style(), ResolvedStyle::default());
}

#[test]
fn constructors_normalize_the_style() {
    // An underline colour with no underline is invisible, so two cells that
    // render the same must compare equal.
    let dormant = ResolvedStyle {
        underline: UnderlineStyle::None,
        underline_color: Some(Rgb(9, 9, 9)),
        ..Default::default()
    };
    assert_eq!(
        Cell::glyph("a", 1, dormant),
        Cell::glyph("a", 1, ResolvedStyle::default())
    );
    assert_eq!(Cell::blank(dormant), Cell::blank(ResolvedStyle::default()));
}

#[test]
fn cells_differing_only_in_text_are_unequal() {
    assert_ne!(
        Cell::glyph("a", 1, ResolvedStyle::default()),
        Cell::glyph("b", 1, ResolvedStyle::default())
    );
}
