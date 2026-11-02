use std::ops::Range;

use hume_rope::cluster::ClusterStart;
use hume_rope::column::{ByteCol, DisplayLineCol};
use hume_rope::offset::ExclusiveRange;

// Theme & Style

/// A semantic scope name emitted by providers, which never emit raw colors.
/// Dot-notation with fallback: `keyword.function` → `keyword` → default.
///
/// Used at construction time; the hot path uses [`ScopeId`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Scope(pub &'static str);

/// An interned scope identifier produced by [`crate::theme::ScopeRegistry`]:
/// a direct index into [`crate::theme::Theme`]'s resolved styles.
///
/// Stable within a session only; never persist one.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScopeId(pub u16);

/// The style vocabulary the frame is composed in. A `Cell` stores a
/// [`ResolvedStyle`] directly, with no narrower backend style, so the
/// underline shape and "no colour" survive all the way to the terminal.
pub use hume_grid::{Modifiers, ResolvedStyle, UnderlineStyle};

// Grapheme: the atom of the formatter

/// One grapheme cluster laid out by the Format stage, the unit that flows
/// through Style into Render.
#[derive(Copy, Clone, Debug)]
pub struct Grapheme {
    /// Byte range within the materialized line buffer (empty for virtual content).
    pub byte_range: ExclusiveRange<ByteCol>,
    /// The buffer cluster this cell shows or stands at, so the style stage
    /// needs no rope lookups. A content line's inline-insert (`Virtual`)
    /// cells and newline indicator carry the cluster they sit at or precede.
    /// `None` for a virtual display line's cells.
    pub pos: Option<ClusterStart>,
    /// 0-based display column within this grapheme's display line. A wrap
    /// continuation renumbers from its own left edge (its indent, under
    /// `WrapMode::Indent`).
    ///
    /// With wrapping off this may run far past the viewport's width; the
    /// render path subtracts `Viewport::horizontal_offset` to get a cell.
    pub display_col: DisplayLineCol,
    /// Display width: 1 for ASCII/most Unicode, 2 for CJK, >1 for tabs.
    pub width: u8,
    /// What to render.
    pub content: CellContent,
    /// Indent depth at this display column, used for indent guide rendering.
    pub indent_depth: u8,
    /// Scope this cell's decoration was interned with. `None` for a real
    /// buffer grapheme, styled by the highlight tiers instead.
    pub scope: Option<ScopeId>,
}

/// What a grapheme cell displays.
///
/// `Whitespace`, `Placeholder`, and `Virtual` hold a `(start, len)` range
/// into a per-line text arena (`LineFormat::virtual_texts` or
/// `VirtualFormat::texts`) rather than a string: their text is not
/// `'static`, and `CellContent` must stay `Copy` on the per-cell hot path.
#[derive(Copy, Clone, Debug)]
pub enum CellContent {
    /// A real grapheme cluster, read from the line text via `byte_range`.
    Grapheme,
    /// A whitespace indicator glyph (`·`, `→`, `⏎`, `⍽`) the user opted into
    /// seeing, styled `ui.virtual.whitespace`.
    Whitespace { start: u32, len: u16 },
    /// One space of a tab whose indicator is off. Unstyled, so
    /// `ui.virtual.whitespace` does not tint a tab the user chose not to see.
    TabFill,
    /// The stand-in for a cluster the terminal must not be shown as itself:
    /// a control character it would act on, or an invisible one it would
    /// collapse. Styled `ui.virtual.invisible`, since these (bidi overrides
    /// among them) are what a reader most needs to notice.
    Placeholder { start: u32, len: u16 },
    /// The right-hand padding cell of a double-width character.
    WidthContinuation,
    /// Empty: tilde filler past EOF, or padding past end of line.
    Empty,
    /// An inline virtual decoration (inlay hint, ghost text) or a
    /// virtual-line cell.
    Virtual { start: u32, len: u16 },
}

// Display Line

/// One horizontal line in the content area. A wrapped buffer line produces
/// several.
#[derive(Clone, Debug)]
pub struct DisplayLine {
    /// What kind of content this display line represents.
    pub kind: DisplayLineKind,
    /// Index range into its `LineFormat::graphemes` buffer.
    pub graphemes: Range<usize>,
}

/// Classifies a display line's origin.
///
/// `line_idx`/`anchor_line` are ropey domain: [`crate::format::format_buffer_line`]'s
/// unit tests format the trailing phantom line, which a `ContentLine` cannot
/// name. A production display line never passes
/// [`crate::display_lines::DisplayLineMap::last_line`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DisplayLineKind {
    /// The first display line of a buffer line.
    LineStart {
        line_idx: hume_rope::line::RopeyLine,
    },
    /// A continuation display line produced by wrapping.
    Wrap {
        line_idx: hume_rope::line::RopeyLine,
        wrap_index: u16,
    },
    /// A virtual display line injected by a provider (no buffer line).
    Virtual {
        provider_id: u16,
        anchor_line: hume_rope::line::RopeyLine,
    },
    /// A tilde filler display line past end of buffer.
    Filler,
}

impl DisplayLineKind {
    /// The buffer line this display line belongs to, if any.
    pub fn line_idx(self) -> Option<hume_rope::line::RopeyLine> {
        match self {
            DisplayLineKind::LineStart { line_idx } | DisplayLineKind::Wrap { line_idx, .. } => {
                Some(line_idx)
            }
            DisplayLineKind::Virtual { .. } | DisplayLineKind::Filler => None,
        }
    }
}

// Selections & Cursor

/// A selection as the renderer paints it, built by the editor each frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PaintedSelection {
    /// The first covered cluster.
    pub first: ClusterStart,
    /// The last covered cluster, at or after `first`.
    pub last: ClusterStart,
    /// The cluster the cursor sits on: the selection's head, which is
    /// `first` or `last`.
    pub cursor: ClusterStart,
}

impl PaintedSelection {
    /// The selection covers one cluster and has no extent to paint.
    pub fn is_cursor(&self) -> bool {
        self.first == self.last
    }

    /// The head comes before the anchor.
    pub fn is_reverse(&self) -> bool {
        !self.is_cursor() && self.cursor == self.first
    }
}

/// A pane's selections as the renderer paints them: at least one, in cursor
/// order, and one of them primary, the one the viewport follows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaintedSelections {
    items: Vec<PaintedSelection>,
    primary: usize,
}

impl PaintedSelections {
    /// # Panics
    /// Panics if `items` is empty, not in cursor order, or has no selection
    /// at `primary`.
    pub fn new(items: Vec<PaintedSelection>, primary: usize) -> Self {
        Self::check(&items, primary);
        Self { items, primary }
    }

    /// Replace the selections, reusing this mirror's storage.
    ///
    /// # Panics
    /// Same as [`new`](Self::new).
    pub fn rewrite(&mut self, items: impl IntoIterator<Item = PaintedSelection>, primary: usize) {
        self.items.clear();
        self.items.extend(items);
        self.primary = primary;
        Self::check(&self.items, primary);
    }

    fn check(items: &[PaintedSelection], primary: usize) {
        assert!(
            !items.is_empty(),
            "a selection mirror holds at least one selection"
        );
        assert!(
            primary < items.len(),
            "the primary index names a selection of the mirror"
        );
        assert!(
            items.windows(2).all(|w| w[0].cursor <= w[1].cursor),
            "a selection mirror is in cursor order"
        );
    }

    pub fn iter(&self) -> impl Iterator<Item = &PaintedSelection> {
        self.items.iter()
    }

    pub fn primary(&self) -> &PaintedSelection {
        &self.items[self.primary]
    }

    /// The primary selection's position in [`iter`](Self::iter)'s order.
    pub fn primary_index(&self) -> usize {
        self.primary
    }
}

/// Editor mode: selects the cursor scope ladder and highlight behavior. The
/// cursor's shape comes from `PaneRenderSettings::cursor_is_block` instead.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum EditorMode {
    #[default]
    Normal,
    Insert,
    Sift,
    Extend,
    Command,
    Search,
}

#[cfg(test)]
mod tests;
