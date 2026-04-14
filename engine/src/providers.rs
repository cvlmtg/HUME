use std::any::Any;
use std::ops::Range;

use crate::builtins::line_number::{LineNumberColumn, LineNumberStyle};
use crate::types::{EditorMode, Grapheme, RowKind, Scope, ScopeId};

// ---------------------------------------------------------------------------
// Provider ID
// ---------------------------------------------------------------------------

/// Unique identifier for a registered provider.
pub type ProviderId = u16;

// ---------------------------------------------------------------------------
// Source context
// ---------------------------------------------------------------------------

/// Context passed to providers that need to query the buffer or syntax tree.
pub struct SourceContext<'a> {
    pub rope: &'a ropey::Rope,
    /// tree-sitter parse tree, if one has been built.
    pub tree: Option<&'a tree_sitter::Tree>,
    /// Absolute byte offset of `line_idx`'s start in the file.
    /// Providers that receive byte ranges from external tools (e.g. tree-sitter)
    /// use this to convert to line-relative offsets.
    pub line_start_byte: usize,
}

// ---------------------------------------------------------------------------
// Highlight tier
// ---------------------------------------------------------------------------

/// Priority tier of a highlight source in the style cascade.
/// Higher = wins over lower. Style stage processes tiers lowest-first so later
/// calls' `layer()` results take precedence.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HighlightTier {
    Syntax = 0,
    SearchMatch = 1,
    Diagnostic = 2,
    BracketMatch = 3,
}

// ---------------------------------------------------------------------------
// Highlight source
// ---------------------------------------------------------------------------

/// A source of highlight spans for buffer lines.
///
/// Called once per visible buffer line. The caller clears `out` before the
/// first provider for each line; providers only append. Each span is
/// `(byte_start, byte_end, scope)` with byte offsets *relative to the line
/// start*. Output must be sorted by `byte_start` and non-overlapping.
pub trait HighlightSource {
    fn tier(&self) -> HighlightTier;

    /// Append highlight spans for `line_idx` to `out`.
    ///
    /// Each span is `(byte_start, byte_end, scope_id)` with byte offsets
    /// *relative to the line start*. Output must be sorted by `byte_start`
    /// and non-overlapping. Scopes must have been interned via
    /// [`crate::theme::ScopeRegistry`] before the first render.
    fn highlights_for_line(
        &self,
        line_idx: usize,
        ctx: &SourceContext,
        out: &mut Vec<(usize, usize, ScopeId)>,
    );
}

// ---------------------------------------------------------------------------
// Gutter column
// ---------------------------------------------------------------------------

/// A single column in the gutter (line numbers, git signs, diagnostics, etc.).
pub trait GutterColumn {
    /// Display width of this column in terminal cells.
    /// `last_line_idx` is the 0-based index of the last line in the file — used to
    /// size line-number columns to fit the largest line number.
    fn width(&self, last_line_idx: usize) -> u8;

    /// Produce content for one display row.
    fn render_row(&self, kind: RowKind, mode: EditorMode, primary_head_line: usize) -> GutterCell;

    /// Downcast support for per-frame config sync (e.g. updating `LineNumberStyle`).
    ///
    /// Implement as `fn as_any_mut(&mut self) -> &mut dyn Any { self }`.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

#[derive(Clone, Debug)]
pub struct GutterCell {
    pub content: GutterCellContent,
    pub scope: Scope,
}

/// What a gutter cell displays.
#[derive(Clone, Debug)]
pub enum GutterCellContent {
    Static(&'static str),
    Number(String),
    Blank,
}

impl GutterCellContent {
    pub fn from_number(n: usize) -> Self {
        Self::Number(n.to_string())
    }
}

impl GutterCell {
    pub fn blank(scope: Scope) -> Self {
        Self { content: GutterCellContent::Blank, scope }
    }

    pub fn as_str(&self) -> &str {
        match &self.content {
            GutterCellContent::Static(s) => s,
            GutterCellContent::Number(s) => s,
            GutterCellContent::Blank => " ",
        }
    }
}

// ---------------------------------------------------------------------------
// Virtual line source
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VirtualLineAnchor {
    /// Insert before the first display row of buffer line `n`.
    Before(usize),
    /// Insert after the last display row (including wraps) of buffer line `n`.
    After(usize),
}

impl VirtualLineAnchor {
    /// Sort key for ordering virtual lines: Before(n) < After(n) < Before/After(n+1).
    pub fn sort_key(self) -> (usize, u8) {
        match self {
            Self::Before(n) => (n, 0),
            Self::After(n) => (n, 1),
        }
    }
}

/// A virtual (non-buffer) display row injected by a provider.
pub struct VirtualLine {
    pub anchor: VirtualLineAnchor,
    pub provider_id: ProviderId,
    /// Pre-formatted graphemes. Virtual lines own their own layout — they are
    /// not subject to the buffer's wrap mode or tab width.
    pub graphemes: Vec<Grapheme>,
}

/// Produces virtual display rows (inline diagnostics, code lenses, git blame).
pub trait VirtualLineSource {
    fn virtual_lines(
        &self,
        visible_lines: Range<usize>,
        content_width: u16,
        out: &mut Vec<VirtualLine>,
    );
}

// ---------------------------------------------------------------------------
// Inline decoration
// ---------------------------------------------------------------------------

/// An inline decoration injected at a specific byte offset within a buffer
/// line. Participates in wrapping (unlike virtual lines). Used for inlay hints,
/// ghost text, and inline type annotations.
#[derive(Clone, Debug)]
pub struct InlineInsert {
    /// Byte offset within the buffer line at which to inject the text.
    pub byte_offset: usize,
    pub text: &'static str,
    pub scope: Scope,
}

pub trait InlineDecoration {
    /// Append inline inserts for `line_idx`. Caller sorts by `byte_offset`.
    fn decorations_for_line(&self, line_idx: usize, out: &mut Vec<InlineInsert>);
}

// ---------------------------------------------------------------------------
// Overlay provider
// ---------------------------------------------------------------------------

/// An overlay rendered on top of the content area after the main pipeline.
/// Writes directly into the ratatui buffer — last registration wins z-order.
pub trait OverlayProvider {
    fn is_active(&self) -> bool;

    fn render(
        &self,
        pane_rect: ratatui::layout::Rect,
        theme: &crate::theme::Theme,
        buf: &mut ratatui::buffer::Buffer,
    );
}

// ---------------------------------------------------------------------------
// Statusline / tab bar
// ---------------------------------------------------------------------------

/// Renders the statusline (bottom row of the terminal area).
/// The engine reserves one row at the bottom for the statusline when present.
pub trait StatuslineProvider {
    fn render(
        &self,
        area: ratatui::layout::Rect,
        theme: &crate::theme::Theme,
        buf: &mut ratatui::buffer::Buffer,
    );
}

/// Renders the tab bar (top row of the terminal area).
/// The engine reserves one row at the top for the tab bar when present.
pub trait TabBarProvider {
    fn render(
        &self,
        area: ratatui::layout::Rect,
        theme: &crate::theme::Theme,
        buf: &mut ratatui::buffer::Buffer,
    );
}

// ---------------------------------------------------------------------------
// Provider set
// ---------------------------------------------------------------------------

/// Complete set of providers for a pane. Allocated once at startup.
#[derive(Default)]
pub struct ProviderSet {
    pub(crate) highlights: Vec<Box<dyn HighlightSource>>,
    pub(crate) gutter_columns: Vec<Box<dyn GutterColumn>>,
    pub(crate) virtual_lines: Vec<Box<dyn VirtualLineSource>>,
    pub(crate) inline_decorations: Vec<Box<dyn InlineDecoration>>,
    pub(crate) overlays: Vec<Box<dyn OverlayProvider>>,
    next_id: ProviderId,
}

impl ProviderSet {
    pub fn new() -> Self {
        Self::default()
    }

    fn alloc_id(&mut self) -> ProviderId {
        let id = self.next_id;
        debug_assert!(self.next_id < ProviderId::MAX, "ProviderId overflow");
        self.next_id += 1;
        id
    }

    pub fn add_highlight_source(&mut self, p: Box<dyn HighlightSource>) -> ProviderId {
        self.highlights.push(p);
        self.highlights.sort_by_key(|h| h.tier());
        self.alloc_id()
    }

    pub fn add_gutter_column(&mut self, p: Box<dyn GutterColumn>) -> ProviderId {
        self.gutter_columns.push(p);
        self.alloc_id()
    }

    pub fn add_virtual_line_source(&mut self, p: Box<dyn VirtualLineSource>) -> ProviderId {
        self.virtual_lines.push(p);
        self.alloc_id()
    }

    pub fn add_inline_decoration(&mut self, p: Box<dyn InlineDecoration>) -> ProviderId {
        self.inline_decorations.push(p);
        self.alloc_id()
    }

    pub fn add_overlay(&mut self, p: Box<dyn OverlayProvider>) -> ProviderId {
        self.overlays.push(p);
        self.alloc_id()
    }

    pub fn gutter_columns(&self) -> &[Box<dyn GutterColumn>] {
        &self.gutter_columns
    }

    /// Push the resolved line-number style into the `LineNumberColumn`, if present.
    ///
    /// Called from `prepare_frame` each frame so `:set line-number-style` takes
    /// effect without rebuilding the provider set.
    pub fn sync_line_number_style(&mut self, style: LineNumberStyle) {
        for col in &mut self.gutter_columns {
            if let Some(ln) = col.as_any_mut().downcast_mut::<LineNumberColumn>() {
                ln.style = style;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Scope;

    struct DummyHighlight {
        tier: HighlightTier,
    }

    impl HighlightSource for DummyHighlight {
        fn tier(&self) -> HighlightTier { self.tier }
        fn highlights_for_line(&self, _: usize, _: &SourceContext, _: &mut Vec<(usize, usize, ScopeId)>) {}
    }

    struct DummyGutter;

    impl GutterColumn for DummyGutter {
        fn width(&self, _: usize) -> u8 { 0 }
        fn render_row(&self, _: crate::types::RowKind, _: crate::types::EditorMode, _: usize) -> GutterCell {
            GutterCell::blank(Scope("x"))
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }

    // ── GutterCellContent::from_number ─────────────────────────────────

    fn num_str(n: usize) -> String {
        GutterCell { content: GutterCellContent::from_number(n), scope: Scope("x") }
            .as_str()
            .to_owned()
    }

    #[test]
    fn from_number_zero() {
        assert_eq!(num_str(0), "0");
    }

    #[test]
    fn from_number_small() {
        assert_eq!(num_str(1), "1");
        assert_eq!(num_str(42), "42");
        assert_eq!(num_str(999), "999");
    }

    #[test]
    fn from_number_large() {
        assert_eq!(num_str(9_999_999), "9999999");
        assert_eq!(num_str(10_000_000), "10000000");
    }

    #[test]
    fn gutter_cell_static_and_blank() {
        let s = GutterCell { content: GutterCellContent::Static("abc"), scope: Scope("x") };
        assert_eq!(s.as_str(), "abc");
        let b = GutterCell::blank(Scope("x"));
        assert_eq!(b.as_str(), " ");
    }

    // ── sync_line_number_style ───────────────────────────────────────────

    #[test]
    fn sync_line_number_style_updates_line_number_column() {
        use crate::builtins::line_number::{LineNumberColumn, LineNumberStyle};
        let mut set = ProviderSet::new();
        set.add_gutter_column(Box::new(LineNumberColumn::with_style(LineNumberStyle::Hybrid)));
        set.sync_line_number_style(LineNumberStyle::Relative);
        let col = set.gutter_columns[0].as_any_mut().downcast_mut::<LineNumberColumn>().unwrap();
        assert_eq!(col.style, LineNumberStyle::Relative);
    }

    #[test]
    fn sync_line_number_style_skips_non_line_number_columns() {
        use crate::builtins::line_number::LineNumberStyle;
        let mut set = ProviderSet::new();
        set.add_gutter_column(Box::new(DummyGutter));
        // Should not panic — DummyGutter doesn't downcast to LineNumberColumn.
        set.sync_line_number_style(LineNumberStyle::Absolute);
    }

    #[test]
    fn sync_line_number_style_no_op_when_empty() {
        use crate::builtins::line_number::LineNumberStyle;
        let mut set = ProviderSet::new();
        set.sync_line_number_style(LineNumberStyle::Hybrid);
    }

    // ── ProviderSet ──────────────────────────────────────────────────────

    #[test]
    fn provider_set_ids_are_sequential_and_unique_across_types() {
        let mut set = ProviderSet::new();
        let id0 = set.add_highlight_source(Box::new(DummyHighlight { tier: HighlightTier::Syntax }));
        let id1 = set.add_gutter_column(Box::new(DummyGutter));
        let id2 = set.add_highlight_source(Box::new(DummyHighlight { tier: HighlightTier::Diagnostic }));
        assert_eq!(id0, 0);
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
    }

    #[test]
    fn provider_set_highlight_sorted_by_tier() {
        let mut set = ProviderSet::new();
        set.add_highlight_source(Box::new(DummyHighlight { tier: HighlightTier::BracketMatch }));
        set.add_highlight_source(Box::new(DummyHighlight { tier: HighlightTier::Syntax }));
        set.add_highlight_source(Box::new(DummyHighlight { tier: HighlightTier::Diagnostic }));

        let tiers: Vec<_> = set.highlights.iter().map(|h| h.tier()).collect();
        assert_eq!(tiers, vec![
            HighlightTier::Syntax,
            HighlightTier::Diagnostic,
            HighlightTier::BracketMatch,
        ]);
    }
}
