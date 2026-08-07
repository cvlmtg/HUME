pub(crate) mod highlight;
use highlight::HighlightStack;
pub use highlight::TierBufs;
pub(crate) use highlight::rebuild_line_decorations;

use crate::providers::Decoration;
use crate::theme::Theme;
use crate::types::{DisplayRow, EditorMode, Grapheme, ResolvedStyle, ScopeId, Selection};

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
    pub syntax_spans: Vec<(usize, usize, ScopeId)>,
    /// Raw decorations from the `PAINT`-kind `DecorationSource` providers,
    /// reused across providers.
    pub decorations: Vec<Decoration>,
    /// Sorted highlight intervals split by tier; built once per buffer line.
    pub tier_bufs: TierBufs,
    /// Selection column spans for the current row (all selections, including primary).
    pub sel_spans: Vec<(u32, u32)>,
    /// Display columns of each selection head on the current row (all selections, including primary).
    pub head_cols: Vec<u32>,
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
    /// Display column of the primary selection's head on the current row. `None` if not on this row.
    pub primary_head_col: Option<u32>,
    /// Column span of the primary selection on the current row. `None` if not on this row.
    pub primary_sel_span: Option<(u32, u32)>,
}

impl StyleScratch {
    pub fn new() -> Self {
        Self {
            styles: Vec::with_capacity(512),
            syntax_spans: Vec::with_capacity(256),
            decorations: Vec::with_capacity(256),
            tier_bufs: TierBufs::default(),
            sel_spans: Vec::new(),
            head_cols: Vec::new(),
            sorted_sels: Vec::new(),
            primary_idx_in_sorted: None,
            primary_head_col: None,
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
        self.head_cols.clear();
        self.sorted_sels.clear();
        self.primary_idx_in_sorted = None;
        self.primary_head_col = None;
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

/// Resolve per-grapheme styles for one display row.
///
/// `styles_out` must be pre-sized to at least `row.graphemes.end` (parallel
/// to `graphemes`). Writes into the row's slice of `styles_out`; entries
/// outside `row.graphemes` are untouched.
///
/// Call [`rebuild_line_decorations`] for the current buffer line before
/// this, and pass its returned tint through as `line_tint`.
/// `scratch.sorted_sels` must be pre-populated and sorted by the caller.
#[allow(clippy::too_many_arguments)]
pub(crate) fn style_row(
    row: &DisplayRow,
    graphemes: &[Grapheme],
    line_start_char: usize,
    line_end_char: usize,
    is_head_line: bool,
    line_tint: Option<ScopeId>,
    mode: EditorMode,
    theme: &Theme,
    scratch: &mut StyleScratch,
) {
    let primary_idx = scratch.primary_idx_in_sorted;
    collect_selection_spans(
        line_start_char,
        line_end_char,
        &scratch.sorted_sels,
        primary_idx,
        graphemes,
        &row.graphemes,
        &mut scratch.sel_spans,
        &mut scratch.primary_sel_span,
    );
    collect_head_cols(
        line_start_char,
        line_end_char,
        &scratch.sorted_sels,
        primary_idx,
        graphemes,
        &row.graphemes,
        &mut scratch.head_cols,
        &mut scratch.primary_head_col,
    );

    let mut hl = HighlightStack::new(&scratch.tier_bufs);

    for (g_idx, g) in graphemes[row.graphemes.clone()].iter().enumerate() {
        let g_idx = row.graphemes.start + g_idx;

        // WidthContinuation cells get the same style as their primary cell.
        if matches!(g.content, crate::types::CellContent::WidthContinuation) {
            if g_idx > 0 {
                scratch.styles[g_idx] = scratch.styles[g_idx - 1];
            }
            continue;
        }

        let mut style = theme.default;

        // Tier 4: provider line-background tint (lowest) — a full-row
        // *background* a `DecorationSource` requested for this line (e.g.
        // git-diff's changed-line highlight). Only `bg` is layered, not the
        // scope's whole resolved style: the row-fill paint site
        // (`pane_render.rs`'s `row_bg`) can only ever contribute a
        // background — it has no per-grapheme fg/modifiers to paint — so a
        // `LineBg`-scoped fg or modifier applied here would only ever show
        // up on content cells, never on the gutter or the row's trailing
        // fill past end-of-line. Constraining both paint sites to `bg` is
        // what keeps them in agreement "by construction" instead of by
        // convention (GIT-DIFF.md Phase 3.2). Layered below cursorline so
        // the cursor's own line always reads clearly even inside a tinted
        // block; a theme whose cursorline has no `bg` falls through to the
        // tint automatically (`ResolvedStyle::layer` only overrides on
        // `Some(bg)`).
        if let Some(scope) = line_tint {
            style = style.layer(ResolvedStyle {
                bg: theme.resolve(scope).bg,
                ..ResolvedStyle::default()
            });
        }

        // Tier 3: selection-head-line background tint.
        // Applied to every grapheme on the line that contains a selection head.
        // theme.ui fields are O(1) struct-field reads — no HashMap lookup.
        if is_head_line {
            style = style.layer(theme.ui.cursorline);
        }

        // Tier 2a–2d: highlights layered in ascending priority.
        // Each theme.resolve(id) is an O(1) Vec index.
        style = hl.layer_at(g.byte_range.start, style, theme);

        // Tier 2e: the cell's own scope (inline-insert decorations). Layered
        // after syntax/search/diagnostic/bracket highlights so a decoration's
        // scope wins over whatever highlight tier would otherwise apply at
        // this column, but still under selection/cursor tiers below.
        if let Some(id) = g.scope {
            style = style.layer(theme.resolve(id));
        }

        // Tier 1: selection (primary wins over secondary for style; both are highlighted)
        let in_primary_sel = scratch
            .primary_sel_span
            .is_some_and(|(s, e)| g.col >= s && g.col < e);
        if in_primary_sel {
            style = style.layer(theme.ui.selection_primary);
        } else if scratch
            .sel_spans
            .iter()
            .any(|&(s, e)| g.col >= s && g.col < e)
        {
            style = style.layer(theme.ui.selection);
        }

        // Tier 0: selection head (highest priority).
        // The grapheme at each selection's head gets `ui.cursor*` styling so it
        // visually looks like a cursor. In bar-cursor modes (Insert, Command, …)
        // the terminal cursor overlaps this cell; in block modes it is the sole
        // visual indicator.
        let is_primary_head = scratch.primary_head_col == Some(g.col);
        if is_primary_head {
            let head_style = if mode.cursor_is_bar() {
                theme.ui.cursor_insert_primary
            } else {
                theme.ui.cursor_primary
            };
            style = style.layer(head_style);
        } else if scratch.head_cols.contains(&g.col) {
            let head_style = if mode.cursor_is_bar() {
                theme.ui.cursor_insert
            } else {
                theme.ui.cursor
            };
            style = style.layer(head_style);
        }

        scratch.styles[g_idx] = style;
    }
}

// ---------------------------------------------------------------------------
// Selection helpers
// ---------------------------------------------------------------------------

/// Collect (start_col, end_col_exclusive) spans for the given line within `row`.
///
/// `line_start_char` / `line_end_char` are the half-open absolute-char range of
/// the buffer line being rendered (from `rope.line_to_char`). Selections use
/// absolute char offsets.
///
/// Also sets `primary_sel_span` when the primary selection (at `primary_idx` in
/// `sorted_sels`) has a visible span on this row.
///
/// Rescans all of `sorted_sels` on every call — O(display_rows × selections)
/// per frame. Intentional: realistic selection counts are single digits, so
/// this is nil in practice. The alternative (binding the window of selections
/// overlapping one line via two `partition_point` calls, hoisted per buffer
/// line) requires translating `primary_idx` into window-local coordinates and
/// threading that through `StyleScratch`'s primary-span/primary-head
/// bookkeeping — a second index space on top of the existing
/// head-sorted-vs-start-sorted subtlety around `pane.selections`, which has
/// bitten this project before. Not worth it for microseconds; do not
/// "optimize" this into the windowed form without re-deriving that trade-off.
#[allow(clippy::too_many_arguments)]
fn collect_selection_spans(
    line_start_char: usize,
    line_end_char: usize,
    sorted_sels: &[Selection],
    primary_idx: Option<usize>,
    graphemes: &[Grapheme],
    row_range: &std::ops::Range<usize>,
    out: &mut Vec<(u32, u32)>,
    primary_sel_span: &mut Option<(u32, u32)>,
) {
    out.clear();
    *primary_sel_span = None;

    let row_gs = &graphemes[row_range.clone()];
    // Use byte_range to detect the empty-line sentinel (byte_range 0..0 = no real content).
    let row_first_byte = row_gs.first().map_or(usize::MAX, |g| g.byte_range.start);
    let row_last_byte = row_gs.last().map_or(0, |g| g.byte_range.end);
    // Char-based wrap-segment boundaries for the intersection check below.
    let row_first_char = row_gs.first().map_or(usize::MAX, |g| g.char_offset);
    // row_last_char_excl: char immediately after the last grapheme on this row.
    // Adding 1 is exact because cursor positions always land on grapheme-cluster
    // boundaries — a selection can never start inside a multi-char cluster.
    let row_last_char_excl = row_gs.last().map_or(0, |g| g.char_offset.saturating_add(1));

    for (idx, sel) in sorted_sels.iter().enumerate() {
        // A collapsed selection (anchor == head) has no extent to paint — the
        // cursor at Tier 0 is its sole representation. Emitting a 1-cell span
        // here is invisible under the Normal block cursor but leaks through the
        // transparent Insert bar cursor as a spuriously highlighted cell.
        if sel.is_collapsed() {
            continue;
        }
        let (start, end) = sel.range(); // (usize, usize) absolute char offsets

        // Skip if the selection doesn't overlap this line at all.
        if start >= line_end_char || end < line_start_char {
            continue;
        }

        // Clamp the selection to this line's char range.
        let sel_char_start = start.max(line_start_char);
        // `usize::MAX` signals "extends past the end of this row" — the col fallback
        // below will then use the last grapheme's trailing column.
        let sel_char_end = if end < line_end_char { end } else { usize::MAX };

        // For rows with real content, skip if the selection doesn't intersect
        // this wrap segment. Without this check a selection on wrap segment N
        // would incorrectly highlight all other wrap segments of the same line.
        if row_first_byte < row_last_byte {
            let ends_before_row = sel_char_end != usize::MAX && sel_char_end <= row_first_char;
            let starts_after_row = sel_char_start >= row_last_char_excl;
            if ends_before_row || starts_after_row {
                continue;
            }
        }

        let col_start = char_offset_to_col(sel_char_start, graphemes, row_range).unwrap_or(0);
        // Selections are inclusive at both ends, so the exclusive upper bound is
        // the right edge of the end grapheme (col + width), not its left edge
        // (col). Using the left edge caused backward selections to silently drop
        // their anchor cell from the highlighted span.
        let col_end = char_offset_to_end_col(sel_char_end, graphemes, row_range)
            .unwrap_or_else(|| row_gs.last().map_or(0, |g| g.col + g.width as u32));
        if col_end > col_start {
            out.push((col_start, col_end));
            if Some(idx) == primary_idx {
                *primary_sel_span = Some((col_start, col_end));
            }
        }
    }
}

/// Collect the display column of each selection head on this line within `row_range`.
///
/// `line_start_char` / `line_end_char` are the half-open absolute-char range of
/// the buffer line. Heads outside this range are skipped.
///
/// Also sets `primary_head_col` when the primary selection (identified by
/// `primary_idx`) has its head on this row.
#[allow(clippy::too_many_arguments)]
fn collect_head_cols(
    line_start_char: usize,
    line_end_char: usize,
    sorted_sels: &[Selection],
    primary_idx: Option<usize>,
    graphemes: &[Grapheme],
    row_range: &std::ops::Range<usize>,
    out: &mut Vec<u32>,
    primary_head_col: &mut Option<u32>,
) {
    out.clear();
    *primary_head_col = None;
    for (idx, sel) in sorted_sels.iter().enumerate() {
        if sel.head < line_start_char || sel.head >= line_end_char {
            continue;
        }
        if let Some(col) = char_offset_to_col(sel.head, graphemes, row_range) {
            out.push(col);
            if Some(idx) == primary_idx {
                *primary_head_col = Some(col);
            }
        }
    }
}

/// Binary-search for the grapheme in `row_range` whose `char_offset` equals or
/// immediately follows `char_offset`, returning `(col, width)`.
///
/// Returns `None` when `char_offset` is the sentinel `usize::MAX` (meaning
/// "extend to end of row"), or when it falls before this row's first grapheme
/// (it belongs to an earlier wrap segment and must not be claimed for this row).
///
/// Rows are non-decreasing in `char_offset` (inline-insert `Virtual` cells
/// carry the offset of the real grapheme they precede, pushed just before it),
/// so `partition_point` can land on an insert rather than the real grapheme at
/// that offset — the loop below skips forward past any such ties.
///
/// `pub(crate)`: also the resolver `rows::RowMap::locate_in_line` uses, so the
/// two column-lookup paths (selection styling, cursor placement) can't drift
/// on how they treat a `Virtual` tie.
pub(crate) fn resolve_grapheme_col(
    char_offset: usize,
    graphemes: &[Grapheme],
    row_range: &std::ops::Range<usize>,
) -> Option<(u32, u32)> {
    if char_offset == usize::MAX {
        // Sentinel: "extend to end of row" — let the caller use the fallback.
        return None;
    }
    let row_graphemes = &graphemes[row_range.clone()];
    let idx = row_graphemes.partition_point(|g| g.char_offset < char_offset);
    // If char_offset falls before this row's first grapheme, the position
    // belongs to an earlier wrap segment — don't claim it for this row.
    if idx == 0
        && row_graphemes
            .first()
            .is_some_and(|g| char_offset < g.char_offset)
    {
        return None;
    }
    // The cursor/selection must land on the real character, not an inline-insert
    // decoration sharing its offset — skip forward past any `Virtual` cells.
    let mut idx = idx;
    while row_graphemes.get(idx).is_some_and(|g| {
        g.char_offset == char_offset
            && matches!(g.content, crate::types::CellContent::Virtual { .. })
    }) {
        idx += 1;
    }
    row_graphemes.get(idx).map(|g| (g.col, g.width as u32))
}

/// Left edge (`g.col`) of the grapheme at `char_offset` in this row.
///
/// Returns `None` for the usize::MAX sentinel or for positions on an earlier
/// wrap segment. Callers use a fallback when `None`.
fn char_offset_to_col(
    char_offset: usize,
    graphemes: &[Grapheme],
    row_range: &std::ops::Range<usize>,
) -> Option<u32> {
    resolve_grapheme_col(char_offset, graphemes, row_range).map(|(col, _)| col)
}

/// Exclusive right edge (`g.col + g.width`) of the grapheme at `char_offset`.
///
/// Used for inclusive selection-span upper bounds: the span `[col_start, col_end)`
/// must cover the end grapheme itself, which requires `col_end = col + width`.
fn char_offset_to_end_col(
    char_offset: usize,
    graphemes: &[Grapheme],
    row_range: &std::ops::Range<usize>,
) -> Option<u32> {
    resolve_grapheme_col(char_offset, graphemes, row_range).map(|(col, width)| col + width)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
