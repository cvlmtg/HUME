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
// Syntax spans
// ---------------------------------------------------------------------------

/// Per-buffer source of syntax highlight spans, implemented outside the
/// engine (hume-treesitter's `Syntax`). Same span contract as
/// `HighlightSource::highlights_for_line`: `(byte_start, byte_end, scope_id)`
/// relative to the line start, sorted, non-overlapping, appended to `out`.
/// The engine consumes only these spans — it has no knowledge of parse
/// trees, grammars, or tree-sitter.
pub trait SyntaxSpans {
    fn spans_for_line(
        &self,
        line_idx: usize,
        rope: &ropey::Rope,
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
}

/// A single column in the gutter (line numbers, git signs, diagnostics, etc.).
pub trait GutterColumn {
    /// Display width of this column in terminal cells.
    /// `last_line_idx` is the 0-based index of the last line in the file — used to
    /// size line-number columns to fit the largest line number.
    fn width(&self, last_line_idx: usize) -> u8;

    /// Produce content for one display row as a sequence of cells.
    /// Single-cell columns (like `LineNumberColumn`) return a `Vec` with one element.
    /// Multi-cell columns (like `SignColumn` with `signcolumn=always:N`) return
    /// multiple cells, one per sign slot.
    fn render_row_cells(&self, kind: RowKind, ctx: &GutterRowCtx) -> Vec<GutterCell>;

    /// Downcast support for per-frame config sync (e.g. updating `LineNumberStyle`).
    ///
    /// Implement as `fn as_any_mut(&mut self) -> &mut dyn Any { self }`.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

#[derive(Clone, Debug)]
pub struct GutterCell {
    pub content: GutterCellContent,
    pub scope: ScopeId,
}

/// Default/blank gutter scope name — the fallback every built-in gutter
/// column (line numbers, unfilled sign slots) renders under when it has
/// nothing more specific to say. One source so the literal can't drift
/// between `builtins::line_number`, `builtins::sign_column`, and
/// `EngineView`'s own interned fallback for `compose_gutter`. Callers intern
/// this once (at pane/view construction) and carry the resulting `ScopeId` —
/// same intern-at-construction contract as `HighlightSource`/`InlineInsert`,
/// so the per-cell hot path in `compose_gutter` never falls back to a
/// by-name lookup.
pub(crate) const DEFAULT_GUTTER_SCOPE: Scope = Scope("ui.linenr");

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
    pub fn blank(scope: ScopeId) -> Self {
        Self {
            content: GutterCellContent::Blank,
            scope,
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
/// pre-built `Grapheme`s: `rows::RowMap` does the grapheme segmentation and
/// width/col bookkeeping itself, the same as it does for real buffer lines, so
/// providers can't get that arithmetic wrong. Virtual
/// lines own their own layout — `text` is not subject to the buffer's wrap
/// mode or tab width.
#[derive(Clone)]
pub struct VirtualLine {
    pub anchor: VirtualLineAnchor,
    pub provider_id: ProviderId,
    pub text: String,
    /// `(byte_start, byte_end, scope_id)` offsets into `text`, each naming the
    /// scope its graphemes should resolve to. Bytes not covered by any
    /// segment get no scope (the render stage falls back to
    /// `ui.virtual_text`). Scopes must have been interned via `ScopeRegistry`
    /// before the first render (same contract as `HighlightSource`).
    ///
    /// Same span shape as `HighlightSource`/`SyntaxSpans`: sorted by
    /// `byte_start`, non-overlapping. Providers are plugin code, so the
    /// engine does not trust this — it re-sorts at intake (`RowMap::block`)
    /// before resolving scopes with a monotonic cursor, the same posture
    /// `rebuild_tier_bufs` takes for highlight spans.
    pub segments: Vec<(usize, usize, ScopeId)>,
}

/// Produces virtual display rows (inline diagnostics, code lenses, git blame).
///
/// Implementations must be cheap per-line lookups into their own state (same
/// contract as `SignSource`): `rows::RowMap` queries a single line whenever it
/// needs that line's block shape, which is scroll, cursor and movement math as
/// well as render — so this can run far more often than once per frame.
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

/// Renders a bottom chrome band, directly above the statusline row (or
/// stacked above a sibling band that already claimed that row — see
/// `EngineView::render`). The engine reserves `height(max)` rows per band
/// when present — panes shrink exactly like a terminal resize, with no
/// separate mechanism (`EngineView::pane_area` folds every band into the
/// same chrome-height arithmetic as the tab bar and statusline).
///
/// Two independent callers implement this: the pick-list drawer
/// (`show-drawer-list!`) and the docked hover popup (`show-popup! #:anchor
/// 'bottom`) — only one is ever non-empty at a time in practice, so
/// `EngineView` carries both as a flat list rather than special-casing
/// mutual exclusion.
pub trait BottomBandProvider {
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
/// Each list stores `(ProviderId, Box<dyn Trait>)` pairs — the id is still
/// load-bearing even with no unregistration path: virtual rows are stamped
/// with their producing provider's id (`rows::RowMap::block`) so
/// `RowKind::Virtual { provider_id }` can be attributed back to it (e.g. by a
/// gutter column rendering which provider owns a row).
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
mod tests;
