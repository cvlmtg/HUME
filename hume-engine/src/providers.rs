use std::borrow::Cow;
use std::sync::Arc;

use bitflags::bitflags;
use hume_grid::Rect;
use hume_rope::column::ByteCol;
use hume_rope::offset::ExclusiveRange;
use hume_rope::spanned::SpannedText;

use crate::render::Canvas;

use crate::builtins::line_number::LineNumberStyle;
use crate::types::{DisplayLineKind, EditorMode, Scope, ScopeId};

// Provider ID

/// Unique identifier for a registered provider.
pub type ProviderId = u16;

// Highlight tier

/// Priority tier of a highlight span in the style cascade.
/// Higher = wins over lower. Style stage processes tiers lowest-first so later
/// calls' `layer()` results take precedence. Data on `Decoration::Highlight`,
/// not a per-provider property: one source can emit spans at different tiers.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HighlightTier {
    Syntax = 0,
    /// Generic plugin-supplied spans (`set-extra-highlights!`): beat syntax,
    /// lose to search matches, diagnostics, and bracket matches.
    Extra = 1,
    SearchMatch = 2,
    Diagnostic = 3,
    BracketMatch = 4,
}

// Syntax source

/// Per-buffer source of syntax-derived per-line facts, implemented outside
/// the engine (hume-treesitter's `Syntax`). Kept as its own trait rather
/// than folded into `DecorationSource`: parse state is per-buffer while
/// `DecorationSource` providers are per-pane, and this is the
/// dependency-inversion seam to `hume-treesitter`. The engine consumes only
/// these per-line answers; it has no knowledge of parse trees, grammars, or
/// tree-sitter.
pub trait SyntaxSource {
    /// Highlight spans in `Decoration::Highlight`'s shape: `(range,
    /// scope_id)` with byte columns relative to the line start, sorted,
    /// non-overlapping, appended to `out`.
    fn spans_for_line(
        &self,
        line_idx: hume_rope::line::ContentLine,
        rope: &ropey::Rope,
        out: &mut Vec<(ExclusiveRange<ByteCol>, ScopeId)>,
    );

    /// The header lines of every scope containing `line` whose header is
    /// above it, ascending (outer to inner), deduplicated, written to `out`,
    /// replacing its contents.
    fn header_lines(
        &self,
        line: hume_rope::line::ContentLine,
        rope: &ropey::Rope,
        out: &mut Vec<hume_rope::line::ContentLine>,
    );

    /// Push the first line of every foldable region starting in `lines`
    /// onto `out`, in any order, possibly more than once. One call covers a
    /// pane's whole run of drawn lines, a query per call rather than per line.
    fn fold_starts(
        &self,
        lines: hume_rope::offset::InclusiveRange<hume_rope::line::ContentLine>,
        rope: &ropey::Rope,
        out: &mut Vec<hume_rope::line::ContentLine>,
    );
}

// Gutter column

/// Context passed to `GutterColumn::render_cells` for buffer/syntax access.
///
/// Built once per drawn row (~100 per frame) from per-frame inputs, so it
/// stays cheap to build. Its one per-line field, `fold`, is a lookup in the
/// fold marks the render pass reads before its walk, one query per run of
/// shown lines. Providers that need other per-line data (e.g.
/// `line_to_byte`) compute it themselves.
pub struct GutterCtx<'a> {
    pub mode: EditorMode,
    pub row: GutterRow,
    pub rope: &'a ropey::Rope,
    /// The gutter's settings for the pane being drawn.
    pub inputs: GutterInputs,
    /// The fold mark of the line being drawn, [`FoldMark::None`] on every
    /// row but a line's first, and while the fold column is hidden.
    pub fold: FoldMark,
}

/// Whether a fold starts on a line, and whether it is closed.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FoldMark {
    None,
    Open,
    Closed,
}

/// The kind of pane row a gutter cell is drawn for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GutterRow {
    /// A row of the pane's text, whose primary selection head is on
    /// `primary_head_line`.
    Body {
        primary_head_line: hume_rope::line::ContentLine,
    },
    /// A scope header row, which shows its line's number, absolute and never
    /// marked as the cursor's, and its fold caret, with no sign.
    Header,
}

impl GutterRow {
    /// The primary selection head's line, `None` on a header row. Ropey
    /// domain, to compare against [`DisplayLineKind::LineStart`]'s own
    /// field; a render walk never emits that for the phantom line, so the
    /// widening changes no answer.
    pub fn primary_head_line(self) -> Option<hume_rope::line::RopeyLine> {
        match self {
            Self::Body { primary_head_line } => Some(primary_head_line.into()),
            Self::Header => None,
        }
    }
}

/// The settings the built-in gutter columns size and draw by, resolved for
/// one pane by the host on every width or draw call, so a width read between
/// frames reflects the settings as they are.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct GutterInputs {
    /// How [`LineNumberColumn`](crate::builtins::line_number::LineNumberColumn)
    /// numbers its lines.
    pub line_numbers: LineNumberStyle,
    /// [`SignColumn`](crate::builtins::sign_column::SignColumn)'s width in
    /// cells, its slots plus one padding cell, `0` when collapsed.
    pub sign_width: u8,
    /// Whether [`FoldColumn`](crate::builtins::fold_column::FoldColumn)
    /// shows.
    pub folds: bool,
}

/// A single column in the gutter (line numbers, git signs, diagnostics, etc.).
pub trait GutterColumn {
    /// Display width of this column in terminal cells.
    /// `last_line_idx` is the 0-based index of the last line in the file, used to
    /// size line-number columns to fit the largest line number. Ropey domain,
    /// phantom trailing line included: deliberate, so the gutter stays one
    /// digit wider than content strictly requires (see `layout::compute_viewport`).
    fn width(&self, last_line_idx: hume_rope::line::RopeyLine, inputs: GutterInputs) -> u8;

    /// Produce content for one display line as a sequence of cells.
    /// Single-cell columns (like `LineNumberColumn`) return a `Vec` with one element.
    /// Multi-cell columns (like `SignColumn` sized past one slot, `gutter-signs=
    /// always`/`auto` included) return multiple cells, one per sign slot.
    fn render_cells(&self, kind: DisplayLineKind, ctx: &GutterCtx) -> Vec<GutterCell>;
}

#[derive(Clone, Debug)]
pub struct GutterCell {
    pub content: GutterCellContent,
    pub scope: ScopeId,
}

/// Default/blank gutter scope name: the fallback every built-in gutter
/// column (line numbers, unfilled sign slots) renders under when it has
/// nothing more specific to say. `pub`, not `pub(crate)`: one source so the
/// literal can't drift across crates. Every
/// caller interns this once (at pane/view construction) and carries the
/// resulting `ScopeId`, the same intern-at-construction contract as
/// `DecorationSource`, so the per-cell hot path in `compose_gutter` never
/// falls back to a by-name lookup.
pub const DEFAULT_GUTTER_SCOPE: Scope = Scope(crate::theme::ui_scopes::LINENR);

/// What a gutter cell displays.
///
/// `Text` holds a `Cow` rather than `&'static str`: builtin columns (line
/// numbers) still borrow a `'static` literal, but plugin-supplied content
/// (Steel-configured icons, git-sign glyphs) is computed at runtime and must
/// own its string. Gutter rendering is ~100 calls/frame, not per-grapheme, so
/// the occasional owned allocation is negligible (unlike the per-cell hot
/// path, which stays index-based via `CellContent`). `Shared` is the one
/// exception worth avoiding the allocation for: `SignColumn` already holds
/// its glyph as `Arc<str>` (`Sign::text`), so cloning it into a cell is a
/// refcount bump rather than a fresh `to_string()` for every visible sign,
/// every frame.
#[derive(Clone, Debug)]
pub enum GutterCellContent {
    Text(Cow<'static, str>),
    Shared(Arc<str>),
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
            GutterCellContent::Shared(s) => s,
            GutterCellContent::Blank => " ",
        }
    }
}

// Virtual line

/// A virtual (non-buffer) display line injected by a provider.
///
/// Providers supply text with scoped byte-range segments rather than
/// pre-built `Grapheme`s: `display_lines::DisplayLineMap` does the grapheme segmentation and
/// width/display-column bookkeeping itself, the same as it does for real buffer lines, so
/// providers can't get that arithmetic wrong. Like a buffer line, the text wraps
/// under the pane's wrap mode and expands tabs against its tab width; every
/// display line it wraps into carries this line's `base_scope`.
#[derive(Clone)]
pub struct VirtualLine {
    /// The buffer line this virtual line is anchored to.
    pub line: hume_rope::line::ContentLine,
    /// `Before`: above `line`'s first display line. `After`: below its last,
    /// wraps included.
    pub side: hume_core::Side,
    pub provider_id: ProviderId,
    /// The text, and the scope each segment of it resolves its graphemes
    /// to. Bytes not covered by any segment fall back to `base_scope`, then
    /// to `ui.virtual_text`. Scopes must have been interned via
    /// `ScopeRegistry` before the first render (same contract as every
    /// `DecorationSource`).
    pub content: SpannedText<ScopeId>,
    /// Scope for bytes no segment covers, and the display line's
    /// background: its `bg` fills the display line's gutter and trailing
    /// cells past the text (the virtual-display-line counterpart of
    /// `Decoration::LineBg`). `None` → the render stage's `ui.virtual_text`
    /// fallback, and no fill.
    pub base_scope: Option<ScopeId>,
}

// Inline insert

/// An inline decoration injected at a specific byte offset within a buffer
/// line. Participates in wrapping, as a unit. Used for inlay hints,
/// ghost text, and inline type annotations.
///
/// `scope` is an already-interned [`ScopeId`], not a [`Scope`] name: providers
/// intern their scopes at construction time (same contract as every
/// `DecorationSource`), since the per-grapheme hot path in
/// `format_buffer_line`/`style_display_line` must stay index-based, never touching the
/// raw scope-name map.
#[derive(Clone, Debug)]
pub struct InlineInsert {
    /// Byte offset within the buffer line at which to inject the text.
    pub byte_offset: ByteCol,
    pub text: String,
    pub scope: ScopeId,
}

// Decoration source

/// One piece of per-line decoration data a [`DecorationSource`] can produce.
/// A single query result covering every kind (highlight spans, virtual
/// lines, inline inserts, line backgrounds), so a provider that wants to
/// emit more than one kind (or a kind that varies by line) only implements
/// one trait.
pub enum Decoration {
    /// A byte range relative to the line start, plus the tier
    /// this span layers at. Tier is data here, not a per-provider property,
    /// so one source can emit spans at different tiers.
    Highlight {
        range: ExclusiveRange<ByteCol>,
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
    /// (render-only, never consulted by layout). The layout stage has no
    /// analogous combined constant: `display_lines::DisplayLineMap` queries `VIRTUAL_LINE` and
    /// `INLINE` separately, at different points in the display-line walk (`block()`
    /// for virtual lines, `ensure_formatted()` for inline inserts), so a `LAYOUT`
    /// union would have no correct caller.
    pub const PAINT: Self = Self::HIGHLIGHT.union(Self::LINE_BG);
}

/// A source of per-line decorations (highlight spans, virtual lines, inline
/// inserts, line backgrounds). Called once per queried buffer line; the
/// caller clears `out` before the first provider for each line (or per
/// provider, when order matters), providers only append.
///
/// Implementations must be cheap per-line lookups into their own state:
/// `display_lines::DisplayLineMap` queries a single line whenever it needs that line's block
/// shape, which is scroll, cursor, and movement math as well as render, so
/// this can run far more often than once per frame.
pub trait DecorationSource {
    /// The [`Decoration`] kinds this source can produce, fixed for the
    /// source's lifetime, cached at registration.
    fn kinds(&self) -> DecorationKinds;

    fn decorations_for_line(
        &self,
        line_idx: hume_rope::line::ContentLine,
        out: &mut Vec<Decoration>,
    );
}

// Overlay provider

/// An overlay rendered on top of the content area after the main pipeline
/// (see `EngineView::overlays`).
pub trait OverlayProvider {
    fn is_active(&self) -> bool;

    /// Paint at the geometry the overlay's own state was placed at this
    /// frame.
    fn render(&self, theme: &crate::theme::Theme, canvas: &mut Canvas);
}

// Chrome

/// Paints one chrome element (the statusline, the tab bar, a bottom band)
/// into the rect `EngineView::render` placed it at.
pub trait ChromeProvider {
    fn render(&self, area: Rect, theme: &crate::theme::Theme, canvas: &mut Canvas);
}

// Provider set

/// Complete set of providers for a pane. Allocated once at startup.
///
/// Each list stores `(ProviderId, Box<dyn Trait>)` pairs. The id is still
/// needed even with no unregistration path: virtual display lines are stamped
/// with their producing provider's id (`display_lines::DisplayLineMap::block`) so
/// `DisplayLineKind::Virtual { provider_id }` can be attributed back to it (e.g. by a
/// gutter column rendering which provider owns a display line).
#[derive(Default)]
pub struct ProviderSet {
    pub(crate) decorations: Vec<(ProviderId, DecorationKinds, Box<dyn DecorationSource>)>,
    pub(crate) gutter_columns: Vec<(ProviderId, Box<dyn GutterColumn>)>,
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

    pub fn gutter_columns(&self) -> impl Iterator<Item = &dyn GutterColumn> {
        self.gutter_columns.iter().map(|(_, c)| c.as_ref())
    }

    /// The gutter column drawn over pane-relative cell `x`, laid out as
    /// [`crate::layout::lane_widths`] sizes it; `None` past the gutter.
    pub fn gutter_lane_at(
        &self,
        max_line: hume_rope::line::RopeyLine,
        inputs: GutterInputs,
        x: u16,
    ) -> Option<ProviderId> {
        let widths = crate::layout::lane_widths(self.gutter_columns(), max_line, inputs);
        let mut end = 0;
        for ((id, _), width) in self.gutter_columns.iter().zip(widths) {
            end += width;
            if x < end {
                return Some(*id);
            }
        }
        None
    }

    /// Decoration sources whose declared [`DecorationKinds`] intersect
    /// `want`, the kind-routing chokepoint: the layout stage (`display_lines::DisplayLineMap`)
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
}

#[cfg(test)]
mod tests;
