use std::any::Any;
use std::borrow::Cow;

use bitflags::bitflags;
use hume_grid::Rect;

use crate::render::Canvas;

use crate::builtins::line_number::{LineNumberColumn, LineNumberStyle};
use crate::builtins::sign_column::SignColumn;
use crate::types::{EditorMode, RowKind, Scope, ScopeId};

// ---------------------------------------------------------------------------
// Provider ID
// ---------------------------------------------------------------------------

/// Unique identifier for a registered provider.
pub type ProviderId = u16;

// ---------------------------------------------------------------------------
// Highlight tier
// ---------------------------------------------------------------------------

/// Priority tier of a highlight span in the style cascade.
/// Higher = wins over lower. Style stage processes tiers lowest-first so later
/// calls' `layer()` results take precedence. Data on `Decoration::Highlight`,
/// not a per-provider property — one source can emit spans at different tiers.
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
// Syntax spans
// ---------------------------------------------------------------------------

/// Per-buffer source of syntax highlight spans, implemented outside the
/// engine (hume-treesitter's `Syntax`). Same span shape as
/// `Decoration::Highlight`: `(byte_start, byte_end, scope_id)` relative to
/// the line start, sorted, non-overlapping, appended to `out`. Kept as its
/// own trait rather than folded into `DecorationSource`: parse state is
/// per-buffer while `DecorationSource` providers are per-pane, and this is
/// the dependency-inversion seam to `hume-treesitter` — same span shape,
/// different lifecycle. The engine consumes only these spans — it has no
/// knowledge of parse trees, grammars, or tree-sitter.
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
/// Gutter rendering (~100 calls/frame) should stay cheap to build, so this
/// struct does not precompute per-line data — providers that need e.g.
/// `line_to_byte` call it themselves.
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
    /// Multi-cell columns (like `SignColumn` sized past one slot, `signcolumn=
    /// always`/`auto` included) return multiple cells, one per sign slot.
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
/// nothing more specific to say. `pub`, not `pub(crate)`: one source so the
/// literal can't drift between `builtins::line_number`, `builtins::sign_column`,
/// `EngineView`'s own interned fallback for `compose_gutter`, and
/// `hume-editor`'s `build_pane`, which interns this same constant to hand
/// `LineNumberColumn`/`SignColumn` their scopes at construction. Every
/// caller interns this once (at pane/view construction) and carries the
/// resulting `ScopeId` — same intern-at-construction contract as
/// `DecorationSource`, so the per-cell hot path in `compose_gutter` never
/// falls back to a by-name lookup.
pub const DEFAULT_GUTTER_SCOPE: Scope = Scope("ui.linenr");

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
// Virtual line
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
/// width/display-column bookkeeping itself, the same as it does for real buffer lines, so
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
    /// segment fall back to `base_scope`, then to `ui.virtual_text`. Scopes
    /// must have been interned via `ScopeRegistry` before the first render
    /// (same contract as every `DecorationSource`).
    ///
    /// Same span shape as `Decoration::Highlight`/`SyntaxSpans`: sorted by
    /// `byte_start`, non-overlapping. Providers are plugin code, so the
    /// engine does not trust this — it re-sorts at intake (`RowMap::block`)
    /// before resolving scopes with a monotonic cursor, the same posture
    /// `style::rebuild_line_decorations` takes for highlight spans.
    pub segments: Vec<(usize, usize, ScopeId)>,
    /// Scope for bytes no `segments` entry covers, and the row's background:
    /// its `bg` fills the row's gutter and trailing cells past the text (the
    /// virtual-row counterpart of `Decoration::LineBg`). `None` → the render
    /// stage's `ui.virtual_text` fallback, and no row fill.
    pub base_scope: Option<ScopeId>,
}

// ---------------------------------------------------------------------------
// Inline insert
// ---------------------------------------------------------------------------

/// An inline decoration injected at a specific byte offset within a buffer
/// line. Participates in wrapping (unlike virtual lines). Used for inlay hints,
/// ghost text, and inline type annotations.
///
/// `scope` is an already-interned [`ScopeId`], not a [`Scope`] name: providers
/// intern their scopes at construction time (same contract as every
/// `DecorationSource`), since the per-grapheme hot path in
/// `format_buffer_line`/`style_row` must stay index-based, never touching the
/// raw scope-name map.
#[derive(Clone, Debug)]
pub struct InlineInsert {
    /// Byte offset within the buffer line at which to inject the text.
    pub byte_offset: usize,
    pub text: String,
    pub scope: ScopeId,
}

// ---------------------------------------------------------------------------
// Decoration source
// ---------------------------------------------------------------------------

/// One piece of per-line decoration data a [`DecorationSource`] can produce.
/// A single query result covering every kind (highlight spans, virtual
/// lines, inline inserts, line backgrounds), so a provider that wants to
/// emit more than one kind (or a kind that varies by line) only implements
/// one trait.
pub enum Decoration {
    /// `(byte_start, byte_end)` relative to the line start, plus the tier
    /// this span layers at — tier is data here, not a per-provider property,
    /// so one source can emit spans at different tiers.
    Highlight {
        byte_start: usize,
        byte_end: usize,
        scope: ScopeId,
        tier: HighlightTier,
    },
    VirtualLine(VirtualLine),
    Inline(InlineInsert),
    /// Full-row background tint for the queried line.
    LineBg(ScopeId),
}

bitflags! {
    /// Which [`Decoration`] kinds a [`DecorationSource`] can produce. Cached
    /// at registration (`ProviderSet::add_decoration_source`) so a querying
    /// stage skips providers whose output it would discard, without calling
    /// `kinds()` again per line.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub struct DecorationKinds: u8 {
        const HIGHLIGHT    = 0b0001;
        const VIRTUAL_LINE = 0b0010;
        const INLINE       = 0b0100;
        const LINE_BG      = 0b1000;
    }
}

impl DecorationKinds {
    /// Kinds the paint stage queries in one pass (`style::rebuild_line_decorations`)
    /// — render-only, never consulted by layout. The layout stage has no
    /// analogous combined constant: `rows::RowMap` queries `VIRTUAL_LINE` and
    /// `INLINE` separately, at different points in the row walk (`block()`
    /// for virtual lines, `format_line()` for inline inserts), so a `LAYOUT`
    /// union would have no correct caller.
    pub const PAINT: Self = Self::HIGHLIGHT.union(Self::LINE_BG);
}

/// A source of per-line decorations (highlight spans, virtual lines, inline
/// inserts, line backgrounds). Called once per queried buffer line; the
/// caller clears `out` before the first provider for each line (or per
/// provider, when order matters — see call sites), providers only append.
///
/// Implementations must be cheap per-line lookups into their own state:
/// `rows::RowMap` queries a single line whenever it needs that line's block
/// shape, which is scroll, cursor, and movement math as well as render — so
/// this can run far more often than once per frame.
pub trait DecorationSource {
    /// The [`Decoration`] kinds this source can produce — fixed for the
    /// source's lifetime, cached at registration.
    fn kinds(&self) -> DecorationKinds;

    fn decorations_for_line(&self, line_idx: usize, out: &mut Vec<Decoration>);
}

// ---------------------------------------------------------------------------
// Overlay provider
// ---------------------------------------------------------------------------

/// An overlay rendered on top of the content area after the main pipeline.
/// Last registration wins z-order.
pub trait OverlayProvider {
    fn is_active(&self) -> bool;

    fn render(&self, pane_rect: Rect, theme: &crate::theme::Theme, canvas: &mut Canvas);
}

// ---------------------------------------------------------------------------
// Statusline / tab bar
// ---------------------------------------------------------------------------

/// Renders the statusline (bottom row of the terminal area).
/// The engine reserves one row at the bottom for the statusline when present.
pub trait StatuslineProvider {
    fn render(&self, area: Rect, theme: &crate::theme::Theme, canvas: &mut Canvas);
}

/// Renders the tab bar (top row of the terminal area).
/// The engine reserves one row at the top for the tab bar when present.
pub trait TabBarProvider {
    fn render(&self, area: Rect, theme: &crate::theme::Theme, canvas: &mut Canvas);
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

    fn render(&self, area: Rect, theme: &crate::theme::Theme, canvas: &mut Canvas);
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
    pub(crate) decorations: Vec<(ProviderId, DecorationKinds, Box<dyn DecorationSource>)>,
    pub(crate) gutter_columns: Vec<(ProviderId, Box<dyn GutterColumn>)>,
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

    pub fn add_decoration_source(&mut self, p: Box<dyn DecorationSource>) -> ProviderId {
        let id = self.alloc_id();
        let kinds = p.kinds();
        self.decorations.push((id, kinds, p));
        id
    }

    pub fn add_gutter_column(&mut self, p: Box<dyn GutterColumn>) -> ProviderId {
        let id = self.alloc_id();
        self.gutter_columns.push((id, p));
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

    /// Decoration sources whose declared [`DecorationKinds`] intersect
    /// `want` — the kind-routing chokepoint: the layout stage (`rows::RowMap`)
    /// queries `VIRTUAL_LINE` and `INLINE` separately, the paint stage
    /// (`style::rebuild_line_decorations`) queries `DecorationKinds::PAINT`
    /// (`HIGHLIGHT | LINE_BG`) in one pass, so no stage pays for a provider
    /// whose output it would discard.
    pub(crate) fn decoration_sources(
        &self,
        want: DecorationKinds,
    ) -> impl Iterator<Item = (ProviderId, &dyn DecorationSource)> {
        self.decorations
            .iter()
            .filter(move |(_, kinds, _)| kinds.intersects(want))
            .map(|(id, _, p)| (*id, p.as_ref()))
    }

    /// Push the resolved line-number style into the `LineNumberColumn`, if present.
    ///
    /// Called from `prepare_frame` each frame so `:set line-number-style` takes
    /// effect without rebuilding the provider set.
    pub fn sync_line_number_style(&mut self, style: LineNumberStyle) {
        for (_, lane) in &mut self.gutter_columns {
            if let Some(ln) = lane.as_any_mut().downcast_mut::<LineNumberColumn>() {
                ln.style = style;
            }
        }
    }

    /// Push a new configured width into every registered `SignColumn`, if
    /// any. Called from `prepare_frame` each frame so the gutter can collapse
    /// to `0` when no sign exists for the pane's current buffer and grow back
    /// when one appears — same downcast pattern as `sync_line_number_style`.
    pub fn sync_sign_column_width(&mut self, width: u8) {
        for (_, lane) in &mut self.gutter_columns {
            if let Some(sc) = lane.as_any_mut().downcast_mut::<SignColumn>() {
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
