use super::*;
use crate::color::Rgb;

const WIDE: &str = "コ";

fn red() -> ResolvedStyle {
    ResolvedStyle {
        fg: Some(Rgb(255, 0, 0)),
        ..Default::default()
    }
}

fn blue() -> ResolvedStyle {
    ResolvedStyle {
        fg: Some(Rgb(0, 0, 255)),
        ..Default::default()
    }
}

/// Every cell of a row rendered as its text, with `_` standing in for a
/// continuation — an independent read of the row's shape that doesn't go
/// through the accessors under test one at a time.
fn shape(grid: &Grid, y: u16) -> String {
    grid.row(y)
        .iter()
        .map(|c| if c.is_continuation() { "_" } else { c.text() })
        .collect()
}

#[test]
fn new_grid_is_all_blanks() {
    let g = Grid::new(3, 2);
    assert_eq!(g.size(), (3, 2));
    assert_eq!(shape(&g, 0), "   ");
    assert_eq!(shape(&g, 1), "   ");
}

#[test]
fn area_is_the_whole_grid_at_the_origin() {
    assert_eq!(Grid::new(7, 4).area(), Rect::new(0, 0, 7, 4));
}

#[test]
fn narrow_write_lands_in_one_cell() {
    let mut g = Grid::new(4, 1);
    g.set_glyph(1, 0, "a", 1, red());
    assert_eq!(shape(&g, 0), " a  ");
    assert_eq!(g[(1, 0)].style(), red());
}

#[test]
fn wide_write_claims_a_continuation() {
    let mut g = Grid::new(4, 1);
    g.set_glyph(1, 0, WIDE, 2, red());
    assert_eq!(shape(&g, 0), format!(" {WIDE}_ "));
    assert_eq!(g[(1, 0)].advance(), 2);
    assert!(g[(2, 0)].is_continuation());
}

#[test]
fn a_continuation_carries_its_heads_style() {
    let mut g = Grid::new(4, 1);
    g.set_glyph(0, 0, WIDE, 2, red());
    assert_eq!(g[(1, 0)].style(), red());
}

#[test]
fn overwriting_a_continuation_demotes_its_head_to_a_blank() {
    let mut g = Grid::new(4, 1);
    g.set_glyph(0, 0, WIDE, 2, red());
    // Land a narrow glyph on the wide glyph's second column.
    g.set_glyph(1, 0, "x", 1, blue());
    assert_eq!(shape(&g, 0), " x  ");
    assert!(!g[(0, 0)].is_continuation());
    assert_eq!(g[(0, 0)].advance(), 1);
    // The demoted half keeps the colour it was painted in, not the new one.
    assert_eq!(g[(0, 0)].style(), red());
}

#[test]
fn overwriting_a_head_demotes_its_orphaned_continuation() {
    let mut g = Grid::new(4, 1);
    g.set_glyph(1, 0, WIDE, 2, red());
    // Land a narrow glyph on the wide glyph's first column.
    g.set_glyph(1, 0, "x", 1, blue());
    assert_eq!(shape(&g, 0), " x  ");
    assert!(!g[(2, 0)].is_continuation());
    assert_eq!(g[(2, 0)].style(), red());
}

#[test]
fn a_wide_write_can_straddle_two_existing_wide_glyphs() {
    let mut g = Grid::new(6, 1);
    g.set_glyph(0, 0, WIDE, 2, red());
    g.set_glyph(2, 0, WIDE, 2, red());
    // Overlaps the second column of the first and the first of the second.
    g.set_glyph(1, 0, WIDE, 2, blue());
    assert_eq!(shape(&g, 0), format!(" {WIDE}_   "));
    assert!(!g[(0, 0)].is_continuation());
    assert!(!g[(3, 0)].is_continuation());
}

#[test]
fn a_wide_glyph_at_the_right_edge_is_blanked_not_split() {
    let mut g = Grid::new(3, 1);
    g.set_glyph(2, 0, WIDE, 2, red());
    assert_eq!(shape(&g, 0), "   ");
    assert_eq!(g[(2, 0)].advance(), 1);
}

#[test]
fn clipping_at_the_right_edge_still_repairs_the_left() {
    let mut g = Grid::new(3, 1);
    g.set_glyph(1, 0, WIDE, 2, red());
    // Starts on the previous glyph's continuation and would run off the edge.
    g.set_glyph(2, 0, WIDE, 2, blue());
    assert_eq!(shape(&g, 0), "   ");
    assert!(!g[(1, 0)].is_continuation());
}

#[test]
fn out_of_bounds_writes_are_ignored() {
    let mut g = Grid::new(2, 1);
    let before = g.clone();
    g.set_glyph(2, 0, "a", 1, red());
    g.set_glyph(0, 1, "a", 1, red());
    assert_eq!(g, before);
}

#[test]
fn zero_advance_still_occupies_one_column() {
    let mut g = Grid::new(3, 1);
    g.set_glyph(0, 0, "x", 0, red());
    assert_eq!(g[(0, 0)].advance(), 1);
    assert_eq!(shape(&g, 0), "x  ");
}

#[test]
fn fill_span_writes_the_half_open_range() {
    let mut g = Grid::new(5, 1);
    g.fill_span(0, 1, 4, Cell::glyph("#", 1, red()));
    assert_eq!(shape(&g, 0), " ### ");
}

#[test]
fn fill_span_over_a_wide_glyphs_tail_demotes_its_head() {
    let mut g = Grid::new(5, 1);
    g.set_glyph(0, 0, WIDE, 2, red());
    g.fill_span(0, 1, 4, Cell::blank(blue()));
    assert_eq!(shape(&g, 0), "     ");
    assert!(!g[(0, 0)].is_continuation());
    assert_eq!(g[(0, 0)].style(), red());
}

#[test]
fn fill_span_over_a_wide_glyphs_head_demotes_its_continuation() {
    let mut g = Grid::new(5, 1);
    g.set_glyph(3, 0, WIDE, 2, red());
    g.fill_span(0, 1, 4, Cell::blank(blue()));
    assert_eq!(shape(&g, 0), "     ");
    assert!(!g[(4, 0)].is_continuation());
    assert_eq!(g[(4, 0)].style(), red());
}

#[test]
fn fill_span_clamps_past_the_right_edge() {
    let mut g = Grid::new(3, 1);
    g.fill_span(0, 1, 99, Cell::glyph("#", 1, red()));
    assert_eq!(shape(&g, 0), " ##");
}

#[test]
fn fill_span_ignores_an_empty_or_inverted_range() {
    let mut g = Grid::new(3, 1);
    let before = g.clone();
    g.fill_span(0, 2, 2, Cell::glyph("#", 1, red()));
    g.fill_span(0, 3, 1, Cell::glyph("#", 1, red()));
    g.fill_span(5, 0, 3, Cell::glyph("#", 1, red()));
    assert_eq!(g, before);
}

#[test]
fn rows_are_independent() {
    let mut g = Grid::new(3, 2);
    g.set_glyph(0, 0, "a", 1, red());
    assert_eq!(shape(&g, 0), "a  ");
    assert_eq!(shape(&g, 1), "   ");
}

#[test]
fn row_out_of_bounds_is_empty() {
    assert!(Grid::new(3, 1).row(9).is_empty());
}

#[test]
fn reset_blanks_every_cell_and_keeps_the_size() {
    let mut g = Grid::new(3, 2);
    g.set_glyph(0, 0, WIDE, 2, red());
    g.reset();
    assert_eq!(g, Grid::new(3, 2));
}

#[test]
fn resize_discards_content_and_reshapes() {
    let mut g = Grid::new(3, 2);
    g.set_glyph(0, 0, "a", 1, red());
    g.resize(5, 1);
    assert_eq!(g.size(), (5, 1));
    assert_eq!(g, Grid::new(5, 1));
}

#[test]
fn cell_returns_none_out_of_bounds() {
    let g = Grid::new(2, 2);
    assert!(g.cell(1, 1).is_some());
    assert!(g.cell(2, 0).is_none());
    assert!(g.cell(0, 2).is_none());
}
