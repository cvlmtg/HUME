use pretty_assertions::assert_eq;

use super::*;

fn col(width: u32) -> BufferLineCol {
    BufferLineCol::new(width)
}

#[test]
fn hard_renders_tabs_then_spaces() {
    assert_eq!(TabStyle::Hard.render_indent(col(0), 4), "");
    assert_eq!(TabStyle::Hard.render_indent(col(6), 4), "\t  ");
}

#[test]
fn soft_renders_spaces_only() {
    assert_eq!(TabStyle::Soft.render_indent(col(0), 4), "");
    assert_eq!(TabStyle::Soft.render_indent(col(6), 4), "      ");
}

#[test]
fn hard_with_tab_width_zero_renders_one_tab_per_column() {
    assert_eq!(TabStyle::Hard.render_indent(col(3), 0), "\t\t\t");
}
