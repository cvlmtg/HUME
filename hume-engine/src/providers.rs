use std::any::Any;
use std::borrow::Cow;
use std::ops::Range;

use crate::builtins::line_number::{LineNumberColumn, LineNumberStyle};
use crate::builtins::sign_column::SignColumn;
use crate::types::{EditorMode, RowKind, Scope, ScopeId};

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
    /// Generic plugin-supplied spans (`set-extra-highlights!`) — beat syntax,
    /// lose to search matches, diagnostics, and bracket matches.
    Extra = 1,
    SearchMatch = 2,
    Diagnostic = 3,
    BracketMatch = 4,
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

/// Context passed to `GutterColumn::render_row` for buffer/syntax access.
///
/// A dedicated struct rather than reusing `SourceContext`: that one carries
/// `line_start_byte`, a per-line-lookup most gutters don't need, and gutter
/// rendering (~100 calls/frame) should stay cheap to build. Providers that
/// need e.g. `line_to_byte` call it themselves — this struct does not
/// precompute per-line data.
pub struct GutterRowCtx<'a> {
    pub mode: EditorMode,
    pub primary_head_line: usize,
    pub rope: &'a ropey::Rope,
    /// tree-sitter parse tree, if one has been built.
    pub tree: Option<&'a tree_sitter::Tree>,
}

/// A single column in the gutter (line numbers, git signs, diagnostics, etc.).
pub trait GutterColumn {
    /// Display width of this column in terminal cells.
    /// `last_line_idx` is the 0-based index of the last line in the file — used to
    /// size line-number columns to fit the largest line number.
    fn width(&self, last_line_idx: usize) -> u8;

    /// Produce content for one display row.
    fn render_row(&self, kind: RowKind, ctx: &GutterRowCtx) -> GutterCell;

    /// Produce content for one display row as a sequence of cells — used by
    /// columns that render multiple sub-cells within their configured width
    /// (e.g. `SignColumn` showing the top N priority-ordered signs when the
    /// user sets `signcolumn=always:N`). Default wraps `render_row` in a
    /// single-element `Vec`, so existing one-cell implementations
    /// (`LineNumberColumn`, test mocks) need no change.
    fn render_row_cells(&self, kind: RowKind, ctx: &GutterRowCtx) -> Vec<GutterCell> {
        vec![self.render_row(kind, ctx)]
    }

    /// Downcast support for per-frame config sync (e.g. updating `LineNumberStyle`).
    ///
    /// Implement as `fn as_any_mut(&mut self) -> &mut dyn Any { self }`.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

#[derive(Clone, Debug)]
pub struct GutterCell {
    pub content: GutterCellContent,
    pub scope: GutterScope,
}

/// A gutter cell's scope: either a name (`Scope`, resolved via
/// `Theme::resolve_by_name` — the slow "by string" path static builtins like
/// `LineNumberColumn` use) or an already-interned `ScopeId` (the fast O(1)
/// path — same intern-at-construction contract as `HighlightSource`/
/// `InlineInsert`, used by providers like `SignSource` that resolve their
/// scope once up front). `compose_gutter` handles both; gutter rendering is
/// ~100 calls/frame either way, so the by-name path's extra hash lookup is
/// negligible — this is about contract consistency, not performance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GutterScope {
    Name(Scope),
    Id(ScopeId),
}

impl From<Scope> for GutterScope {
    fn from(s: Scope) -> Self {
        GutterScope::Name(s)
    }
}

impl From<ScopeId> for GutterScope {
    fn from(id: ScopeId) -> Self {
        GutterScope::Id(id)
    }
}

/// What a gutter cell displays.
///
/// `Text` holds a `Cow` rather than `&'static str`: builtin columns (line
/// numbers) still borrow a `'static` literal, but plugin-supplied content
/// (Steel-configured icons, git-sign glyphs) is computed at runtime and must
/// own its string. Gutter rendering is ~100 calls/frame, not per-grapheme, so
/// the occasional owned allocation is negligible (unlike the per-cell hot
/// path, which stays index-based via `CellContent`).
#[derive(Clone, Debug)]
pub enum GutterCellContent {
    Text(Cow<'static, str>),
    Blank,
}

impl GutterCellContent {
    pub fn from_number(n: usize) -> Self {
        Self::Text(Cow::Owned(n.to_string()))
    }
}

impl GutterCell {
    pub fn blank(scope: impl Into<GutterScope>) -> Self {
        Self {
            content: GutterCellContent::Blank,
            scope: scope.into(),
        }
    }

    pub fn as_str(&self) -> &str {
        match &self.content {
            GutterCellContent::Text(s) => s,
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
///
/// Providers supply plain `text` + scoped byte-range `segments` rather than
/// pre-built `Grapheme`s: the pipeline (`emit_virtual_row`) does the grapheme
/// segmentation and width/col bookkeeping itself, the same as it does for
/// real buffer lines, so providers can't get that arithmetic wrong. Virtual
/// lines own their own layout — `text` is not subject to the buffer's wrap
/// mode or tab width.
#[derive(Clone)]
pub struct VirtualLine {
    pub anchor: VirtualLineAnchor,
    pub provider_id: ProviderId,
    pub text: String,
    /// Byte ranges into `text`, each tagged with the `ScopeId` its graphemes
    /// should resolve to. Bytes not covered by any segment get no scope
    /// (`emit_virtual_row` falls back to `ui.virtual_text`). Segments must
    /// have been interned via `ScopeRegistry` before the first render (same
    /// contract as `HighlightSource`).
    pub segments: Vec<(Range<usize>, ScopeId)>,
}

/// Produces virtual display rows (inline diagnostics, code lenses, git blame).
///
/// Implementations must be cheap per-line lookups into their own state (same
/// contract as `SignSource`): `format::display_rows_for_line` calls this for
/// a single line during scroll/cursor accounting, not just per-frame render,
/// so this can run far more often than once per frame.
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
///
/// `scope` is an already-interned [`ScopeId`], not a [`Scope`] name: providers
/// intern their scopes at construction time (same contract as
/// [`HighlightSource`] — see its `highlights_for_line` doc), since the
/// per-grapheme hot path in `format_buffer_line`/`style_row` must stay
/// index-based, never touching the raw scope-name map.
#[derive(Clone, Debug)]
pub struct InlineInsert {
    /// Byte offset within the buffer line at which to inject the text.
    pub byte_offset: usize,
    pub text: String,
    pub scope: ScopeId,
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

/// Renders the bottom drawer band, directly above the statusline row.
/// The engine reserves `height(max)` rows above the statusline when present
/// — panes shrink exactly like a terminal resize, with no separate
/// mechanism (`EngineView::pane_area` folds it into the same chrome-height
/// arithmetic as the tab bar and statusline).
pub trait DrawerProvider {
    /// Rows to reserve this frame, given `max` (the caller's ceiling — half
    /// the terminal height). Content-driven (e.g. `min(rows + 1, max)`), not
    /// a fixed constant, so a short list doesn't reserve a half-screen band.
    fn height(&self, max: u16) -> u16;

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
///
/// Each list stores `(ProviderId, Box<dyn Trait>)` pairs rather than bare
/// boxes so a provider can be looked up and removed later (`remove`) — e.g.
/// a plugin unregistering a gutter column it added, or a `:set`-style toggle
/// turning a built-in column off.
#[derive(Default)]
pub struct ProviderSet {
    pub(crate) highlights: Vec<(ProviderId, Box<dyn HighlightSource>)>,
    pub(crate) gutter_columns: Vec<(ProviderId, Box<dyn GutterColumn>)>,
    pub(crate) virtual_lines: Vec<(ProviderId, Box<dyn VirtualLineSource>)>,
    pub(crate) inline_decorations: Vec<(ProviderId, Box<dyn InlineDecoration>)>,
    pub(crate) overlays: Vec<(ProviderId, Box<dyn OverlayProvider>)>,
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
        let id = self.alloc_id();
        self.highlights.push((id, p));
        // Stable sort: within a tier, registration order is preserved —
        // later `add_highlight_source` calls layer on top of earlier ones
        // at the same tier. Deterministic layering relies on this.
        self.highlights.sort_by_key(|(_, h)| h.tier());
        id
    }

    pub fn add_gutter_column(&mut self, p: Box<dyn GutterColumn>) -> ProviderId {
        let id = self.alloc_id();
        self.gutter_columns.push((id, p));
        id
    }

    pub fn add_virtual_line_source(&mut self, p: Box<dyn VirtualLineSource>) -> ProviderId {
        let id = self.alloc_id();
        self.virtual_lines.push((id, p));
        id
    }

    pub fn add_inline_decoration(&mut self, p: Box<dyn InlineDecoration>) -> ProviderId {
        let id = self.alloc_id();
        self.inline_decorations.push((id, p));
        id
    }

    pub fn add_overlay(&mut self, p: Box<dyn OverlayProvider>) -> ProviderId {
        let id = self.alloc_id();
        self.overlays.push((id, p));
        id
    }

    /// Remove the provider registered under `id`, whichever list holds it.
    /// Returns `true` if a provider was removed, `false` for an unknown id
    /// (a no-op, not an error — callers don't need to track what they
    /// already removed).
    ///
    /// No editor call site exists yet — kept as the engine primitive for the
    /// future unregistration paths named on [`ProviderSet`] (Steel provider
    /// registration; a gutter-visibility `:set` toggle). See ROADMAP open
    /// questions.
    pub fn remove(&mut self, id: ProviderId) -> bool {
        let before = self.highlights.len()
            + self.gutter_columns.len()
            + self.virtual_lines.len()
            + self.inline_decorations.len()
            + self.overlays.len();
        self.highlights.retain(|(pid, _)| *pid != id);
        self.gutter_columns.retain(|(pid, _)| *pid != id);
        self.virtual_lines.retain(|(pid, _)| *pid != id);
        self.inline_decorations.retain(|(pid, _)| *pid != id);
        self.overlays.retain(|(pid, _)| *pid != id);
        let after = self.highlights.len()
            + self.gutter_columns.len()
            + self.virtual_lines.len()
            + self.inline_decorations.len()
            + self.overlays.len();
        before != after
    }

    pub fn gutter_columns(&self) -> impl Iterator<Item = &dyn GutterColumn> {
        self.gutter_columns.iter().map(|(_, c)| c.as_ref())
    }

    /// Push the resolved line-number style into the `LineNumberColumn`, if present.
    ///
    /// Called from `prepare_frame` each frame so `:set line-number-style` takes
    /// effect without rebuilding the provider set.
    pub fn sync_line_number_style(&mut self, style: LineNumberStyle) {
        for (_, col) in &mut self.gutter_columns {
            if let Some(ln) = col.as_any_mut().downcast_mut::<LineNumberColumn>() {
                ln.style = style;
            }
        }
    }

    /// Push a new configured width into every registered `SignColumn`, if
    /// any. Called from `prepare_frame` each frame so the gutter can collapse
    /// to `0` when no sign exists for the pane's current buffer and grow back
    /// when one appears — same downcast pattern as `sync_line_number_style`.
    pub fn sync_sign_column_width(&mut self, width: u8) {
        for (_, col) in &mut self.gutter_columns {
            if let Some(sc) = col.as_any_mut().downcast_mut::<SignColumn>() {
                sc.set_width(width);
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
        fn tier(&self) -> HighlightTier {
            self.tier
        }
        fn highlights_for_line(
            &self,
            _: usize,
            _: &SourceContext,
            _: &mut Vec<(usize, usize, ScopeId)>,
        ) {
        }
    }

    struct DummyGutter;

    impl GutterColumn for DummyGutter {
        fn width(&self, _: usize) -> u8 {
            0
        }
        fn render_row(&self, _: crate::types::RowKind, _: &GutterRowCtx) -> GutterCell {
            GutterCell::blank(Scope("x"))
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    /// Distinguishable from `DummyGutter` by width — used to prove `remove`
    /// takes down the right provider and leaves the other untouched.
    struct OtherGutter;

    impl GutterColumn for OtherGutter {
        fn width(&self, _: usize) -> u8 {
            5
        }
        fn render_row(&self, _: crate::types::RowKind, _: &GutterRowCtx) -> GutterCell {
            GutterCell::blank(Scope("y"))
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    // ── GutterCellContent::from_number ─────────────────────────────────

    fn num_str(n: usize) -> String {
        GutterCell {
            content: GutterCellContent::from_number(n),
            scope: Scope("x").into(),
        }
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
    fn gutter_cell_text_and_blank() {
        let s = GutterCell {
            content: GutterCellContent::Text(Cow::Borrowed("abc")),
            scope: Scope("x").into(),
        };
        assert_eq!(s.as_str(), "abc");
        let b = GutterCell::blank(Scope("x"));
        assert_eq!(b.as_str(), " ");
    }

    // ── sync_line_number_style ───────────────────────────────────────────

    #[test]
    fn sync_line_number_style_updates_line_number_column() {
        use crate::builtins::line_number::{LineNumberColumn, LineNumberStyle};
        let mut set = ProviderSet::new();
        set.add_gutter_column(Box::new(LineNumberColumn::with_style(
            LineNumberStyle::Hybrid,
        )));
        set.sync_line_number_style(LineNumberStyle::Relative);
        let col = set.gutter_columns[0]
            .1
            .as_any_mut()
            .downcast_mut::<LineNumberColumn>()
            .unwrap();
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

    #[test]
    fn sync_sign_column_width_updates_registered_sign_columns() {
        let mut set = ProviderSet::new();
        set.add_gutter_column(Box::new(SignColumn::new()));
        set.sync_sign_column_width(0);
        let col = set.gutter_columns[0]
            .1
            .as_any_mut()
            .downcast_mut::<SignColumn>()
            .unwrap();
        assert_eq!(col.width(0), 0);
    }

    #[test]
    fn sync_sign_column_width_skips_non_sign_columns() {
        let mut set = ProviderSet::new();
        set.add_gutter_column(Box::new(DummyGutter));
        // Should not panic — DummyGutter doesn't downcast to SignColumn.
        set.sync_sign_column_width(0);
    }

    // ── ProviderSet ──────────────────────────────────────────────────────

    #[test]
    fn provider_set_ids_are_sequential_and_unique_across_types() {
        let mut set = ProviderSet::new();
        let id0 = set.add_highlight_source(Box::new(DummyHighlight {
            tier: HighlightTier::Syntax,
        }));
        let id1 = set.add_gutter_column(Box::new(DummyGutter));
        let id2 = set.add_highlight_source(Box::new(DummyHighlight {
            tier: HighlightTier::Diagnostic,
        }));
        assert_eq!(id0, 0);
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
    }

    #[test]
    fn provider_set_highlight_sorted_by_tier() {
        let mut set = ProviderSet::new();
        set.add_highlight_source(Box::new(DummyHighlight {
            tier: HighlightTier::BracketMatch,
        }));
        set.add_highlight_source(Box::new(DummyHighlight {
            tier: HighlightTier::Syntax,
        }));
        set.add_highlight_source(Box::new(DummyHighlight {
            tier: HighlightTier::Diagnostic,
        }));

        let tiers: Vec<_> = set.highlights.iter().map(|(_, h)| h.tier()).collect();
        assert_eq!(
            tiers,
            vec![
                HighlightTier::Syntax,
                HighlightTier::Diagnostic,
                HighlightTier::BracketMatch,
            ]
        );
    }

    // ── Provider unregistration (G3) ─────────────────────────────────────

    #[test]
    fn remove_by_id_takes_down_only_that_provider() {
        let mut set = ProviderSet::new();
        let id0 = set.add_gutter_column(Box::new(DummyGutter)); // width 0
        set.add_gutter_column(Box::new(OtherGutter)); // width 5

        assert!(set.remove(id0));

        let widths: Vec<u8> = set.gutter_columns().map(|c| c.width(0)).collect();
        assert_eq!(
            widths,
            vec![5],
            "only OtherGutter (width 5) remains; render order reflects it alone"
        );
    }

    #[test]
    fn remove_unknown_id_is_a_no_op() {
        let mut set = ProviderSet::new();
        set.add_gutter_column(Box::new(DummyGutter));

        assert!(!set.remove(999), "unknown id must return false");
        assert_eq!(
            set.gutter_columns().count(),
            1,
            "removing an unknown id must not touch existing providers"
        );
    }

    #[test]
    fn remove_across_provider_types_only_touches_the_matching_list() {
        // Ids are shared across all five lists' allocator — removing a
        // gutter-column id must not accidentally hit a highlight source
        // that happens to share the same numeric id space at a different
        // index.
        let mut set = ProviderSet::new();
        let highlight_id = set.add_highlight_source(Box::new(DummyHighlight {
            tier: HighlightTier::Syntax,
        }));
        let gutter_id = set.add_gutter_column(Box::new(DummyGutter));

        assert!(set.remove(gutter_id));
        assert_eq!(set.gutter_columns().count(), 0);
        assert_eq!(
            set.highlights.len(),
            1,
            "removing the gutter column must not touch the highlight source"
        );
        let _ = highlight_id;
    }
}
