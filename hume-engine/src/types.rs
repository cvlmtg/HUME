use std::ops::Range;

// ---------------------------------------------------------------------------
// Theme & Style
// ---------------------------------------------------------------------------

/// A semantic scope name emitted by providers. All style decisions go through
/// the Theme — providers never emit raw colors.
///
/// Built-in scopes use `&'static str`. The scope format follows dot-notation
/// with automatic fallback: `keyword.function` → `keyword` → default.
///
/// Use `Scope` at construction time (theme maps, scope_map slices, gutter
/// cells). Use [`ScopeId`] on the hot path — it is an O(1) Vec index into the
/// theme's baked style array, with no hashing.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Scope(pub &'static str);

/// An interned scope identifier produced by [`crate::theme::ScopeRegistry`].
///
/// Resolved once at provider-construction time; used on the per-grapheme hot
/// path to look up [`ResolvedStyle`] from [`crate::theme::Theme`] in O(1) via
/// a direct `Vec` index.
///
/// The mapping is stable within a session but not persistent — do not store
/// `ScopeId` values across sessions.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScopeId(pub u16);

/// The style vocabulary the frame is composed in, re-exported from
/// `hume-grid` so engine and editor code keeps naming it here.
///
/// It lives in `hume-grid` because a `Cell` stores a [`ResolvedStyle`]
/// directly. There is no separate backend style for it to be narrowed into
/// at the end of the pipeline, which is what keeps the underline *shape* and
/// the distinction between "no colour" and "some default colour" intact all
/// the way to the terminal.
pub use hume_grid::{Modifiers, ResolvedStyle, UnderlineStyle};

// ---------------------------------------------------------------------------
// Grapheme — the atom of the formatter
// ---------------------------------------------------------------------------

/// One grapheme cluster laid out by the Format stage.
/// This is the unit that flows through Style into Render.
#[derive(Clone, Debug)]
pub struct Grapheme {
    /// Byte range within the materialized line buffer (empty for virtual content).
    ///
    /// Used by the highlight system (tree-sitter intervals are byte-native) and
    /// by the wrap-segment intersection check in the style stage.
    pub byte_range: Range<usize>,
    /// Absolute char offset from the start of the buffer.
    ///
    /// Populated by the format stage so the style stage can resolve selection
    /// head positions without any rope lookups. A content row's inline-insert
    /// (`Virtual`) cells and its newline indicator still carry the real char
    /// offset of the buffer position they sit at or precede — `usize::MAX` is
    /// reserved for a virtual-*row*'s cells, which have no buffer position at
    /// all (see [`crate::rows::RowMap::render_row`]).
    pub char_offset: usize,
    /// Display column within the row this grapheme ended up on (0-based,
    /// accounting for the widths before it). Row-relative: when a line wraps,
    /// the graphemes carried onto the continuation row are renumbered from
    /// that row's own left edge (its indent, under `WrapMode::Indent`).
    ///
    /// With wrapping off a row *is* the whole line, so the same value is also
    /// the line's own display column and may run far past the viewport's
    /// width — which is why the render path subtracts
    /// `ViewportState::horizontal_offset` from it rather than treating it as
    /// a screen cell.
    pub display_col: u32,
    /// Display width: 1 for ASCII/most Unicode, 2 for CJK, >1 for tabs.
    pub width: u8,
    /// What to render.
    pub content: CellContent,
    /// Indent depth at this display column — used for indent guide rendering.
    pub indent_depth: u8,
    /// Scope this cell's decoration was interned with, if any. `None` for
    /// every real buffer grapheme — their style comes from the highlight
    /// tiers, not a per-cell scope. `Some` for inline-insert (`Virtual`)
    /// cells and virtual-line cells that carry their own styling.
    pub scope: Option<ScopeId>,
}

/// What a grapheme cell displays.
///
/// `Indicator` and `Virtual` reference a range in a per-frame text arena in
/// `FormatScratch` (`virtual_texts` for a content line's inline decorations,
/// `virtual_row.texts` for a provider's virtual row — see
/// `rows::RenderRow::virtual_texts`) rather than borrowing a string directly
/// — their source text (Steel-configured whitespace glyphs, LSP inlay hints,
/// provider-built virtual lines) is never truly `'static`, and `CellContent`
/// must stay `Copy` on the per-cell hot path (pushed and matched once per
/// grapheme in `format_buffer_line`/`style_row`). `(start: u32, len: u16)`
/// keeps the variant small; a single line's arena realistically never
/// approaches either bound (see `format::push_arena_text`).
#[derive(Copy, Clone, Debug)]
pub enum CellContent {
    /// A real grapheme cluster. The text is read from the rope via `byte_range`.
    /// Avoids copying grapheme strings during formatting.
    Grapheme,
    /// A substitution: whitespace indicator, tab fill character.
    Indicator { start: u32, len: u16 },
    /// The stand-in for a cluster the terminal must not be shown as itself —
    /// a control character it would act on, or an invisible one it would
    /// collapse. Drawn exactly like an [`CellContent::Indicator`], but a
    /// distinct variant because the style stage gives it its own scope
    /// (`ui.virtual.invisible`): these are the characters a reader most needs
    /// to notice, bidi overrides among them.
    Placeholder { start: u32, len: u16 },
    /// The right-hand padding cell of a double-width character.
    WidthContinuation,
    /// Empty: tilde filler past EOF, or padding past end of line.
    Empty,
    /// An inline virtual decoration (inlay hint, ghost text) or a
    /// virtual-line cell.
    Virtual { start: u32, len: u16 },
}

// ---------------------------------------------------------------------------
// Display Row
// ---------------------------------------------------------------------------

/// One horizontal row in the content area.
/// A single buffer line may produce multiple DisplayRows when wrapping.
#[derive(Clone, Debug)]
pub struct DisplayRow {
    /// What kind of content this row represents.
    pub kind: RowKind,
    /// Index range into the frame's `FrameScratch::graphemes` buffer.
    pub graphemes: Range<usize>,
}

/// Classifies a display row's origin.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// The first display row of a buffer line.
    LineStart { line_idx: usize },
    /// A continuation row produced by wrapping.
    Wrap { line_idx: usize, wrap_row: u16 },
    /// A virtual row injected by a provider (no buffer line).
    Virtual {
        provider_id: u16,
        anchor_line: usize,
    },
    /// A tilde filler row past end of buffer.
    Filler,
}

impl RowKind {
    /// Returns the buffer line index if this row corresponds to a real line.
    pub fn line_idx(self) -> Option<usize> {
        match self {
            RowKind::LineStart { line_idx } | RowKind::Wrap { line_idx, .. } => Some(line_idx),
            RowKind::Virtual { .. } | RowKind::Filler => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Selections & Cursor
// ---------------------------------------------------------------------------

/// An editor selection: an anchor and a head, both as absolute char offsets
/// from the start of the buffer.
///
/// Anchor == head is a single-character selection covering the char at that
/// index (the editor's inclusive selection invariant). The selection spans
/// [min(anchor, head), max(anchor, head)] inclusive.
///
/// Using char offsets avoids per-frame rope lookups at the editor→engine
/// boundary: the editor simply copies its char-offset selections directly.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    /// Returns the selection range as (start, end) with start <= end.
    pub fn range(self) -> (usize, usize) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// True if this selection is collapsed (anchor == head, no range).
    pub fn is_collapsed(self) -> bool {
        self.anchor == self.head
    }
}

/// Ceiling on a command's numeric count prefix (e.g. the `3` in `3w`).
///
/// Shared by the keyboard accumulator (`hume-editor`'s `Editor::handle_normal`,
/// which builds a count one digit at a time and would otherwise overflow
/// `usize` past ~20 digits) and Steel's `parse_count_extend`, which decodes a
/// script-supplied count with no digit-by-digit limit of its own. Without a
/// shared ceiling, a count from either origin can still make a command loop
/// `count` times with no fixed-point exit (e.g. macro replay growing its
/// replay queue by `count × macro length`) run long enough to hang the editor.
pub const MAX_COUNT: usize = 10_000;

/// Editor mode — determines cursor shape and highlight behavior.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum EditorMode {
    #[default]
    Normal,
    Insert,
    Select,
    Extend,
    Command,
    Search,
}

impl EditorMode {
    /// Whether the cursor should render as a bar (Insert/Command/Search/Select)
    /// or a block (Normal/Extend).
    pub fn cursor_is_bar(self) -> bool {
        matches!(
            self,
            EditorMode::Insert | EditorMode::Command | EditorMode::Search | EditorMode::Select
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
