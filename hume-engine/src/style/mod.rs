pub(crate) mod highlight;
use highlight::HighlightStack;
pub use highlight::TierBufs;
pub(crate) use highlight::rebuild_line_decorations;

use hume_rope::column::{ByteCol, DisplayLineCol};
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::providers::Decoration;
use crate::theme::Theme;
use crate::types::{DisplayLine, EditorMode, Grapheme, ResolvedStyle, ScopeId, Selection};

// ---------------------------------------------------------------------------
// Scratch storage
// ---------------------------------------------------------------------------

/// Reusable scratch buffers for the Style stage (Stage 3).
///
/// Owned by [`crate::pipeline::FrameScratch`] so capacity is retained across
/// frames — no heap allocation after the first frame warms up the `Vec`s.
pub struct StyleScratch {
    /// Per-grapheme resolved styles (parallel to the graphemes slice).
    pub styles: Vec<ResolvedStyle>,
    /// Raw spans from the buffer's `SyntaxSpans` source, reused each call.
    pub syntax_spans: Vec<(ByteCol, ByteCol, ScopeId)>,
    /// Raw decorations from the `PAINT`-kind `DecorationSource` providers,
    /// reused across providers.
    pub decorations: Vec<Decoration>,
    /// Sorted highlight intervals split by tier; built once per buffer line.
    pub tier_bufs: TierBufs,
    /// Selection display-column spans for the current display line (all selections, including primary).
    pub sel_spans: Vec<(DisplayLineCol, DisplayLineCol)>,
    /// Display columns of each selection head on the current display line (all selections, including primary).
    pub head_display_cols: Vec<DisplayLineCol>,
    /// Sorted copy of selections; populated once per frame or batch call.
    pub sorted_sels: Vec<Selection>,
    /// Index of the primary selection within `sorted_sels`. `None` if empty.
    ///
    /// The primary is always `selections[0]` by convention (the selection the viewport follows).
    /// We track it by post-sort index rather than adding an `is_primary: bool` field on
    /// `Selection`, because `Selection` is a pure data type (anchor + head) and "primary" is a
    /// display concern — it would bleed UI logic into the core model. Using an index also avoids
    /// fragile DocPos equality: two distinct selections could share the same head position.
    pub primary_idx_in_sorted: Option<usize>,
    /// Display column of the primary selection's head on the current display line. `None` if not on this display line.
    pub primary_head_display_col: Option<DisplayLineCol>,
    /// Display-column span of the primary selection on the current display line. `None` if not on this display line.
    pub primary_sel_span: Option<(DisplayLineCol, DisplayLineCol)>,
}

impl StyleScratch {
    pub fn new() -> Self {
        Self {
            styles: Vec::with_capacity(512),
            syntax_spans: Vec::with_capacity(256),
            decorations: Vec::with_capacity(256),
            tier_bufs: TierBufs::default(),
            sel_spans: Vec::new(),
            head_display_cols: Vec::new(),
            sorted_sels: Vec::new(),
            primary_idx_in_sorted: None,
            primary_head_display_col: None,
            primary_sel_span: None,
        }
    }

    /// Copy `selections` (already sorted in ascending document order) into
    /// `sorted_sels`. No sort is performed — the caller guarantees order.
    pub fn populate_sorted_sels(&mut self, selections: &[Selection], primary_idx: usize) {
        debug_assert!(
            selections.windows(2).all(|w| w[0].head <= w[1].head),
            "selections must be sorted by head position",
        );
        self.sorted_sels.clear();
        self.sorted_sels.extend_from_slice(selections);
        self.primary_idx_in_sorted = if selections.is_empty() {
            None
        } else {
            Some(primary_idx)
        };
    }

    /// Reset all buffers to empty, retaining allocated capacity.
    pub fn clear(&mut self) {
        self.styles.clear();
        self.syntax_spans.clear();
        self.decorations.clear();
        self.tier_bufs.clear();
        self.sel_spans.clear();
        self.head_display_cols.clear();
        self.sorted_sels.clear();
        self.primary_idx_in_sorted = None;
        self.primary_head_display_col = None;
        self.primary_sel_span = None;
    }
}

impl Default for StyleScratch {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Resolve per-grapheme styles for one display line.
///
/// `styles_out` must be pre-sized to at least `display_line.graphemes.end` (parallel
/// to `graphemes`). Writes into the display line's slice of `styles_out`; entries
/// outside `display_line.graphemes` are untouched.
///
/// Call [`rebuild_line_decorations`] for the current buffer line before
/// this, and pass its returned tint through as `line_tint`.
/// `scratch.sorted_sels` must be pre-populated and sorted by the caller.
#[allow(clippy::too_many_arguments)]
pub(crate) fn style_display_line(
    display_line: &DisplayLine,
    graphemes: &[Grapheme],
    line_chars: ExclusiveRange<CharOffset>,
    is_head_line: bool,
    line_tint: Option<ScopeId>,
    mode: EditorMode,
    cursor_is_block: bool,
    theme: &Theme,
    scratch: &mut StyleScratch,
) {
    let primary_idx = scratch.primary_idx_in_sorted;
    // Whether the primary selection runs backward (head before anchor) — the
    // one piece of per-display-line context the unpainted-primary-head
    // carve-out below needs. A property of the selection itself, not of
    // this display line, so it's computed once here rather than per grapheme.
    let primary_is_reverse = primary_idx
        .map(|idx| scratch.sorted_sels[idx].head < scratch.sorted_sels[idx].anchor)
        .unwrap_or(false);
    collect_selection_spans(
        line_chars,
        &scratch.sorted_sels,
        primary_idx,
        graphemes,
        &display_line.graphemes,
        &mut scratch.sel_spans,
        &mut scratch.primary_sel_span,
    );
    collect_head_display_cols(
        line_chars,
        &scratch.sorted_sels,
        primary_idx,
        graphemes,
        &display_line.graphemes,
        &mut scratch.head_display_cols,
        &mut scratch.primary_head_display_col,
    );

    let mut hl = HighlightStack::new(&scratch.tier_bufs);

    // Tiers 4 and 3 are properties of the *display line*, not of any
    // grapheme in it, so they resolve once here rather than per grapheme below.
    //
    // Tier 4: provider line-background tint (lowest) — a full-row
    // *background* a `DecorationSource` requested for this line (e.g.
    // git-diff's changed-line highlight). Only `bg` is layered, not the
    // scope's whole resolved style: the row-fill paint site
    // (`pane_render.rs`'s `row_bg`) can only ever contribute a background —
    // it has no per-grapheme fg/modifiers to paint — so a `LineBg`-scoped fg
    // or modifier applied here would only ever show up on content cells,
    // never on the gutter or the row's trailing fill past end-of-line.
    // Constraining both paint sites to `bg` is what keeps them in agreement
    // "by construction" instead of by convention. Layered below cursorline so
    // the cursor's own line always reads clearly even inside a tinted block;
    // a theme whose cursorline has no `bg` falls through to the tint
    // automatically (`ResolvedStyle::layer` only overrides on `Some(bg)`).
    //
    // Tier 3: selection-head-line background tint, applied to every grapheme
    // on the line that contains a selection head. `theme.ui` fields are O(1)
    // struct-field reads — no HashMap lookup.
    let mut base_style = theme.default;
    if let Some(scope) = line_tint {
        base_style = base_style.layer(ResolvedStyle {
            bg: theme.resolve(scope).bg,
            ..ResolvedStyle::default()
        });
    }
    if is_head_line {
        base_style = base_style.layer(theme.ui.cursorline);
    }

    for (g_idx, g) in graphemes[display_line.graphemes.clone()].iter().enumerate() {
        let g_idx = display_line.graphemes.start + g_idx;

        // WidthContinuation cells get the same style as their primary cell.
        if matches!(g.content, crate::types::CellContent::WidthContinuation) {
            if g_idx > 0 {
                scratch.styles[g_idx] = scratch.styles[g_idx - 1];
            }
            continue;
        }

        let mut style = base_style;

        // Tier 2a–2d: highlights layered in ascending priority.
        // Each theme.resolve(id) is an O(1) Vec index.
        style = hl.layer_at(g.byte_range.start, style, theme);

        // Tier 2d½: an unrenderable cluster's stand-in, or an opted-in
        // whitespace glyph. Layered over the syntax highlight so `<202e>`
        // reads as a placeholder rather than as whatever token it sits
        // inside — these are the characters a reader most needs to notice
        // — and so a whitespace glyph takes the theme's dedicated colour
        // rather than the token colour underneath it. Under Tier 2e so a
        // decoration that carries its own scope still wins. `TabFill` gets
        // neither scope: a tab's indicator being off must leave it unstyled.
        match g.content {
            crate::types::CellContent::Placeholder { .. } => {
                style = style.layer(theme.ui.invisible);
            }
            crate::types::CellContent::Whitespace { .. } => {
                style = style.layer(theme.ui.whitespace);
            }
            _ => {}
        }

        // Tier 2e: the cell's own scope (inline-insert decorations). Layered
        // after syntax/search/diagnostic/bracket highlights so a decoration's
        // scope wins over whatever highlight tier would otherwise apply at
        // this column, but still under selection/cursor tiers below.
        if let Some(id) = g.scope {
            style = style.layer(theme.resolve(id));
        }

        // Tiers 1 and 0: selection and selection-head, as mutually exclusive
        // spans — matching Helix's own non-overlapping selection/cursor spans
        // in `doc_selection_highlights` rather than layering a (possibly
        // partial) head style over a selection style underneath it. Both
        // heads are painted only when the resolved cursor shape for the live
        // mode is `Block`; for Bar/Underline the real terminal cursor is the
        // primary's sole indicator, and HUME extends that same rule to
        // secondary heads (a deliberate departure from Helix, which paints
        // secondary cursors unconditionally — HUME has no second hardware
        // cursor for a themed block to stand in for either). This is the one
        // implementing site for that rule; every other mention in this
        // codebase is a pointer back here, not a second copy.
        //
        // The primary's unpainted head keeps its selection styling only for a
        // reverse selection (head before anchor), Helix's own carve-out for
        // that case; a forward or collapsed primary selection leaves the cell
        // bare, since the real terminal cursor already marks it. A secondary
        // head has no such marker, so its unpainted head instead falls
        // through to the plain span checks below: a ranged secondary keeps an
        // unbroken `ui.selection` run across its head cell, and a collapsed
        // one (no span to fall through to) goes bare.
        //
        // `head_display_cols` holds every head, the primary's included (see
        // `collect_head_display_cols`), so one `cursor_is_block` gate covers
        // both heads; `is_primary_head` only then picks which scope ladder.
        // The `is_primary_head` arm below is not folded into that gate's
        // condition: a primary head that fails it must stay bare, never fall
        // through to the span checks meant for the secondary case.
        let is_primary_head = scratch.primary_head_display_col == Some(g.display_col);
        let in_primary_span = scratch
            .primary_sel_span
            .is_some_and(|(s, e)| g.display_col >= s && g.display_col < e);
        if cursor_is_block && scratch.head_display_cols.contains(&g.display_col) {
            style = style.layer(cursor_cell_style(theme, mode, is_primary_head));
        } else if is_primary_head {
            if primary_is_reverse && in_primary_span {
                style = style.layer(theme.ui.selection_primary);
            }
        } else if in_primary_span {
            style = style.layer(theme.ui.selection_primary);
        } else if scratch
            .sel_spans
            .iter()
            .any(|&(s, e)| g.display_col >= s && g.display_col < e)
        {
            style = style.layer(theme.ui.selection);
        }

        scratch.styles[g_idx] = style;
    }
}

/// Pick the Tier-0 cursor cell style for a selection head, by mode and
/// primary-ness. This is the themed cell color/attrs a head is painted with —
/// unrelated to `hume_editor`'s internal `CursorShape` setting, which is the
/// real terminal hardware cursor's appearance during Insert mode.
///
/// `Insert` uses the insert chain; `Extend` — HUME's name for Helix's Select
/// mode — uses the select chain; every other mode, including HUME's own
/// Command/Search/Sift prompt modes (which have no Helix equivalent — Helix
/// keeps the underlying document mode while a prompt is open, and HUME's
/// prompts have no cursor-shape option of their own), uses the plain Normal
/// chain.
fn cursor_cell_style(theme: &Theme, mode: EditorMode, is_primary: bool) -> ResolvedStyle {
    let (primary, secondary) = match mode {
        EditorMode::Insert => (theme.ui.cursor_insert_primary, theme.ui.cursor_insert),
        EditorMode::Extend => (theme.ui.cursor_select_primary, theme.ui.cursor_select),
        EditorMode::Normal | EditorMode::Command | EditorMode::Search | EditorMode::Sift => {
            (theme.ui.cursor_primary, theme.ui.cursor)
        }
    };
    if is_primary { primary } else { secondary }
}

// ---------------------------------------------------------------------------
// Selection helpers
// ---------------------------------------------------------------------------

/// Collect (start_display_col, end_display_col_exclusive) spans for the given line within `grapheme_range`.
///
/// `line_chars` is the half-open absolute-char range of the buffer line
/// being rendered (from `rope.line_to_char`). Selections use absolute char
/// offsets.
///
/// Also sets `primary_sel_span` when the primary selection (at `primary_idx` in
/// `sorted_sels`) has a visible span on this display line.
///
/// Rescans all of `sorted_sels` on every call — O(display_lines × selections)
/// per frame. Intentional: realistic selection counts are single digits, so
/// this is nil in practice. The alternative (binding the window of selections
/// overlapping one line via two `partition_point` calls, hoisted per buffer
/// line) requires translating `primary_idx` into window-local coordinates and
/// threading that through `StyleScratch`'s primary-span/primary-head
/// bookkeeping — a second index space on top of the existing
/// head-sorted-vs-start-sorted subtlety around `pane.selections`, which has
/// bitten this project before. Not worth it for microseconds; do not
/// "optimize" this into the windowed form without re-deriving that trade-off.
fn collect_selection_spans(
    line_chars: ExclusiveRange<CharOffset>,
    sorted_sels: &[Selection],
    primary_idx: Option<usize>,
    graphemes: &[Grapheme],
    grapheme_range: &std::ops::Range<usize>,
    out: &mut Vec<(DisplayLineCol, DisplayLineCol)>,
    primary_sel_span: &mut Option<(DisplayLineCol, DisplayLineCol)>,
) {
    out.clear();
    *primary_sel_span = None;

    let gs = &graphemes[grapheme_range.clone()];
    // This display line has real content only when it has graphemes at all,
    // and its first and last don't collapse to the same empty point — an
    // empty line's sole grapheme (the EOL sentinel) has an empty `byte_range`.
    // The style stage never runs on a virtual display line (whose cells carry
    // `Grapheme::char_offset`'s no-buffer-position sentinel — see its doc),
    // so `first`/`last`'s own char offsets are always genuine positions here.
    let content_chars = match (gs.first(), gs.last()) {
        (Some(first), Some(last)) if first.byte_range.start < last.byte_range.end => Some((
            CharOffset::new(first.char_offset),
            CharOffset::new(last.char_offset),
        )),
        _ => None,
    };

    for (idx, sel) in sorted_sels.iter().enumerate() {
        // A collapsed selection (anchor == head) has no extent to paint — the
        // cursor at Tier 0 is its sole representation, and a 1-cell span here
        // would claim the head cell is *selected* rather than merely where the
        // cursor sits.
        if sel.is_collapsed() {
            continue;
        }
        let span = sel.range(); // absolute char offsets
        let (start, end) = (span.start, span.end);

        // Skip if the selection doesn't overlap this line at all.
        if start >= line_chars.end || end < line_chars.start {
            continue;
        }

        // Clamp the selection to this line's char range.
        let sel_char_start = start.max(line_chars.start);
        // `None` signals "extends past the end of this display line" — the
        // `display_col` fallback below will then use the last grapheme's
        // trailing column.
        let sel_char_end = (end < line_chars.end).then_some(end);

        // For display lines with real content, skip if the selection doesn't
        // intersect this wrap segment. Without this check a selection on
        // wrap segment N would incorrectly highlight all other wrap segments
        // of the same line.
        if let Some((first_char, last_char)) = content_chars {
            let ends_before = sel_char_end.is_some_and(|end| end <= first_char);
            let starts_after = sel_char_start > last_char;
            if ends_before || starts_after {
                continue;
            }
        }

        let display_col_start =
            char_offset_to_display_col(sel_char_start, graphemes, grapheme_range)
                .unwrap_or(DisplayLineCol::new(0));
        // Selections are inclusive at both ends, so the exclusive upper bound is
        // the right edge of the end grapheme (display_col + width), not its
        // left edge (display_col). Using the left edge caused backward
        // selections to silently drop their anchor cell from the highlighted span.
        let display_col_end = sel_char_end
            .and_then(|end| char_offset_to_end_display_col(end, graphemes, grapheme_range))
            .unwrap_or_else(|| {
                gs.last().map_or(DisplayLineCol::new(0), |g| {
                    g.display_col.advance_saturating(g.width as u32)
                })
            });
        if display_col_end > display_col_start {
            out.push((display_col_start, display_col_end));
            if Some(idx) == primary_idx {
                *primary_sel_span = Some((display_col_start, display_col_end));
            }
        }
    }
}

/// Collect the display column of each selection head on this line within `grapheme_range`.
///
/// `line_chars` is the half-open absolute-char range of the buffer line.
/// Heads outside this range are skipped.
///
/// Also sets `primary_head_display_col` when the primary selection (identified
/// by `primary_idx`) has its head on this display line.
fn collect_head_display_cols(
    line_chars: ExclusiveRange<CharOffset>,
    sorted_sels: &[Selection],
    primary_idx: Option<usize>,
    graphemes: &[Grapheme],
    grapheme_range: &std::ops::Range<usize>,
    out: &mut Vec<DisplayLineCol>,
    primary_head_display_col: &mut Option<DisplayLineCol>,
) {
    out.clear();
    *primary_head_display_col = None;
    for (idx, sel) in sorted_sels.iter().enumerate() {
        if !line_chars.contains(sel.head) {
            continue;
        }
        if let Some(display_col) = char_offset_to_display_col(sel.head, graphemes, grapheme_range) {
            out.push(display_col);
            if Some(idx) == primary_idx {
                *primary_head_display_col = Some(display_col);
            }
        }
    }
}

/// Binary-search for the grapheme in `grapheme_range` whose `char_offset` equals or
/// immediately follows `char_offset`, returning `(display_col, width)`.
///
/// Returns `None` when `char_offset` falls before this display line's first
/// grapheme (it belongs to an earlier wrap segment and must not be claimed
/// for this display line).
///
/// A display line's graphemes are non-decreasing in `char_offset` (inline-insert `Virtual` cells
/// carry the offset of the real grapheme they precede, pushed just before it),
/// so `partition_point` can land on an insert rather than the real grapheme at
/// that offset — the loop below skips forward past any such ties.
///
/// `pub(crate)`: also the resolver `display_lines::DisplayLineMap::locate_in_line` uses, so the
/// two column-lookup paths (selection styling, cursor placement) can't drift
/// on how they treat a `Virtual` tie.
pub(crate) fn resolve_grapheme_display_col(
    char_offset: CharOffset,
    graphemes: &[Grapheme],
    grapheme_range: &std::ops::Range<usize>,
) -> Option<(DisplayLineCol, u32)> {
    let char_offset = char_offset.index();
    let gs = &graphemes[grapheme_range.clone()];
    let idx = gs.partition_point(|g| g.char_offset < char_offset);
    // If char_offset falls before this display line's first grapheme, the
    // position belongs to an earlier wrap segment — don't claim it for this
    // display line.
    if idx == 0 && gs.first().is_some_and(|g| char_offset < g.char_offset) {
        return None;
    }
    // The cursor/selection must land on the real character, not an inline-insert
    // decoration sharing its offset — skip forward past any `Virtual` cells.
    let mut idx = idx;
    while gs.get(idx).is_some_and(|g| {
        g.char_offset == char_offset
            && matches!(g.content, crate::types::CellContent::Virtual { .. })
    }) {
        idx += 1;
    }
    gs.get(idx).map(|g| (g.display_col, g.width as u32))
}

/// Left edge (`g.display_col`) of the grapheme at `char_offset` in this display line.
///
/// Returns `None` for a position on an earlier wrap segment. Callers use a
/// fallback when `None`.
fn char_offset_to_display_col(
    char_offset: CharOffset,
    graphemes: &[Grapheme],
    grapheme_range: &std::ops::Range<usize>,
) -> Option<DisplayLineCol> {
    resolve_grapheme_display_col(char_offset, graphemes, grapheme_range)
        .map(|(display_col, _)| display_col)
}

/// Exclusive right edge (`g.display_col + g.width`) of the grapheme at `char_offset`.
///
/// Used for inclusive selection-span upper bounds: the span
/// `[display_col_start, display_col_end)` must cover the end grapheme
/// itself, which requires `display_col_end = display_col + width`.
fn char_offset_to_end_display_col(
    char_offset: CharOffset,
    graphemes: &[Grapheme],
    grapheme_range: &std::ops::Range<usize>,
) -> Option<DisplayLineCol> {
    resolve_grapheme_display_col(char_offset, graphemes, grapheme_range)
        .map(|(display_col, width)| display_col.advance_saturating(width))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
