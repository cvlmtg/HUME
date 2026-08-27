//! Light box-drawing glyphs, shared by every crate that draws a border or a
//! divider: `hume-engine`'s pane-seam dividers and `hume-editor`'s popup/menu
//! boxes. One set of named escapes rather than one per drawing site, so a
//! grep for a glyph finds every use, and so a terminal or editor that renders
//! one of them ambiguously can't have some call sites silently drawing a
//! different (but visually similar) character than the rest.
//!
//! Written as `\u{...}` escapes rather than literal characters for the same
//! grep-ability reason — a literal box-drawing glyph pasted into a diff is
//! indistinguishable by eye from a lookalike Unicode character.

pub const HORIZONTAL: &str = "\u{2500}";
pub const VERTICAL: &str = "\u{2502}";
/// A heavier vertical, used for a scrollbar thumb overdrawing a plain
/// [`VERTICAL`] border.
pub const THICK_VERTICAL: &str = "\u{2503}";
pub const CROSS: &str = "\u{253c}";
pub const HORIZONTAL_DOWN: &str = "\u{252c}";
pub const HORIZONTAL_UP: &str = "\u{2534}";
pub const VERTICAL_RIGHT: &str = "\u{251c}";
pub const VERTICAL_LEFT: &str = "\u{2524}";
pub const TOP_LEFT: &str = "\u{250c}";
pub const TOP_RIGHT: &str = "\u{2510}";
pub const BOTTOM_LEFT: &str = "\u{2514}";
pub const BOTTOM_RIGHT: &str = "\u{2518}";
