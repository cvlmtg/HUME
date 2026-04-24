use crate::providers::{HighlightSource, HighlightTier, SourceContext};
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
    /// Raw highlight intervals from one provider, reused across providers.
    pub highlights: Vec<(usize, usize, ScopeId)>,
    /// Sorted highlight intervals split by tier; built once per buffer line.
    pub tier_bufs: TierBufs,
    /// Selection column spans for the current row (all selections, including primary).
    pub sel_spans: Vec<(u16, u16)>,
    /// Display columns of each selection head on the current row (all selections, including primary).
    pub head_cols: Vec<u16>,
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
    pub primary_head_col: Option<u16>,
    /// Column span of the primary selection on the current row. `None` if not on this row.
    pub primary_sel_span: Option<(u16, u16)>,
}

impl StyleScratch {
    pub fn new() -> Self {
        Self {
            styles: Vec::with_capacity(512),
            highlights: Vec::with_capacity(256),
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
        self.highlights.clear();
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
// Interval cursor — O(1) amortised forward lookup
// ---------------------------------------------------------------------------

/// Walks a sorted, non-overlapping slice of `(byte_start, byte_end, ScopeId)`
/// intervals in order. Queries must be monotonically non-decreasing.
struct IntervalCursor<'a> {
    intervals: &'a [(usize, usize, ScopeId)],
    pos: usize,
}

impl<'a> IntervalCursor<'a> {
    fn new(intervals: &'a [(usize, usize, ScopeId)]) -> Self {
        Self { intervals, pos: 0 }
    }

    /// Return the scope id active at `byte_offset`, or `None`.
    /// Advances the internal cursor forward; never goes backward.
    fn scope_at(&mut self, byte_offset: usize) -> Option<ScopeId> {
        // Skip intervals that have already ended.
        while self.pos < self.intervals.len() && self.intervals[self.pos].1 <= byte_offset {
            self.pos += 1;
        }
        // Check if the current interval covers `byte_offset`.
        if self.pos < self.intervals.len() {
            let (start, end, id) = self.intervals[self.pos];
            if start <= byte_offset && byte_offset < end {
                return Some(id);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Per-line highlight stack
// ---------------------------------------------------------------------------

/// Aggregated highlight intervals for one buffer line, one cursor per tier.
/// Built once before iterating graphemes, queried per grapheme in O(1) amortised.
struct HighlightStack<'a> {
    syntax: IntervalCursor<'a>,
    search: IntervalCursor<'a>,
    diagnostic: IntervalCursor<'a>,
    bracket: IntervalCursor<'a>,
}

impl<'a> HighlightStack<'a> {
    fn new(tiers: &'a TierBufs) -> Self {
        Self {
            syntax: IntervalCursor::new(&tiers.syntax),
            search: IntervalCursor::new(&tiers.search),
            diagnostic: IntervalCursor::new(&tiers.diagnostic),
            bracket: IntervalCursor::new(&tiers.bracket),
        }
    }

    /// Layer all active highlight tiers at `byte_offset` into `base`.
    ///
    /// Each `theme.resolve(id)` call is an O(1) `Vec` index into the baked
    /// style array — no hashing on the per-grapheme hot path.
    fn layer_at(
        &mut self,
        byte_offset: usize,
        mut base: ResolvedStyle,
        theme: &Theme,
    ) -> ResolvedStyle {
        // Syntax (lowest)
        if let Some(id) = self.syntax.scope_at(byte_offset) {
            base = base.layer(theme.resolve(id));
        }
        // Search match
        if let Some(id) = self.search.scope_at(byte_offset) {
            base = base.layer(theme.resolve(id));
        }
        // Diagnostic
        if let Some(id) = self.diagnostic.scope_at(byte_offset) {
            base = base.layer(theme.resolve(id));
        }
        // Bracket match (highest highlight)
        if let Some(id) = self.bracket.scope_at(byte_offset) {
            base = base.layer(theme.resolve(id));
        }
        base
    }
}

/// Scratch buffer holding sorted highlight intervals split by tier.
/// Owned by `FrameScratch` so capacity is retained across frames.
///
/// Each interval is `(byte_start, byte_end, ScopeId)` — the `ScopeId` maps to
/// a pre-baked [`ResolvedStyle`] via an O(1) `Vec` index in [`Theme::resolve`].
#[derive(Default)]
pub struct TierBufs {
    syntax: Vec<(usize, usize, ScopeId)>,
    search: Vec<(usize, usize, ScopeId)>,
    diagnostic: Vec<(usize, usize, ScopeId)>,
    bracket: Vec<(usize, usize, ScopeId)>,
}

impl TierBufs {
    pub fn clear(&mut self) {
        self.syntax.clear();
        self.search.clear();
        self.diagnostic.clear();
        self.bracket.clear();
    }

    fn push(&mut self, tier: HighlightTier, interval: (usize, usize, ScopeId)) {
        match tier {
            HighlightTier::Syntax => self.syntax.push(interval),
            HighlightTier::SearchMatch => self.search.push(interval),
            HighlightTier::Diagnostic => self.diagnostic.push(interval),
            HighlightTier::BracketMatch => self.bracket.push(interval),
        }
    }

    fn sort_all(&mut self) {
        self.syntax.sort_by_key(|i| i.0);
        self.search.sort_by_key(|i| i.0);
        self.diagnostic.sort_by_key(|i| i.0);
        self.bracket.sort_by_key(|i| i.0);
    }
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Gather highlight intervals from all providers for one buffer line.
///
/// Must be called once per buffer line before calling [`style_row`] for that
/// line's display rows. Clears and re-fills `tier_bufs` and `raw_highlights`.
pub(crate) fn rebuild_tier_bufs(
    line_idx: usize,
    providers: &[Box<dyn HighlightSource>],
    rope: &ropey::Rope,
    tree: Option<&tree_sitter::Tree>,
    scratch: &mut StyleScratch,
) {
    scratch.tier_bufs.clear();
    scratch.highlights.clear();
    let ctx = SourceContext {
        rope,
        tree,
        line_start_byte: rope.line_to_byte(line_idx),
    };
    for provider in providers {
        provider.highlights_for_line(line_idx, &ctx, &mut scratch.highlights);
        for &interval in scratch.highlights.iter() {
            scratch.tier_bufs.push(provider.tier(), interval);
        }
        scratch.highlights.clear();
    }
    scratch.tier_bufs.sort_all();
}

/// Resolve per-grapheme styles for one display row.
///
/// `styles_out` must be pre-sized to at least `row.graphemes.end` (parallel
/// to `graphemes`). Writes into the row's slice of `styles_out`; entries
/// outside `row.graphemes` are untouched.
///
/// Call [`rebuild_tier_bufs`] for the current buffer line before this.
/// `scratch.sorted_sels` must be pre-populated and sorted by the caller.
#[allow(clippy::too_many_arguments)]
pub(crate) fn style_row(
    row: &DisplayRow,
    graphemes: &[Grapheme],
    line_start_char: usize,
    line_end_char: usize,
    is_head_line: bool,
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

        // Tier 3: selection-head-line background tint (lowest).
        // Applied to every grapheme on the line that contains a selection head.
        // theme.ui fields are O(1) struct-field reads — no HashMap lookup.
        if is_head_line {
            style = style.layer(theme.ui.cursorline);
        }

        // Tier 2a–2d: highlights layered in ascending priority.
        // Each theme.resolve(id) is an O(1) Vec index.
        style = hl.layer_at(g.byte_range.start, style, theme);

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

/// Stage 3 — resolve per-grapheme styles for all display rows.
///
/// Convenience wrapper over [`rebuild_tier_bufs`] + [`style_row`] that
/// processes the full set of visible rows in one call. Used by the
/// non-fused pipeline path and tests.
#[allow(clippy::too_many_arguments)]
pub fn resolve_styles(
    rows: &[DisplayRow],
    graphemes: &[Grapheme],
    selections: &[Selection],
    primary_idx: usize,
    mode: EditorMode,
    highlight_providers: &[Box<dyn HighlightSource>],
    theme: &Theme,
    rope: &ropey::Rope,
    tree: Option<&tree_sitter::Tree>,
    scratch: &mut StyleScratch,
) {
    scratch.populate_sorted_sels(selections, primary_idx);

    let mut current_line: Option<usize> = None;
    scratch
        .styles
        .resize(graphemes.len(), ResolvedStyle::default());

    for row in rows {
        let line_idx = match row.kind.line_idx() {
            Some(l) => l,
            None => {
                // Virtual row: styles are already default-initialised.
                continue;
            }
        };

        // Rebuild highlight tiers once per buffer line (multiple rows can share a line
        // when wrapping is enabled).
        if current_line != Some(line_idx) {
            current_line = Some(line_idx);
            rebuild_tier_bufs(line_idx, highlight_providers, rope, tree, scratch);
        }

        // Char range for this line: selections are char-offset based.
        let line_start_char = rope.line_to_char(line_idx);
        let line_end_char = rope.line_to_char(line_idx + 1);
        let is_head_line = scratch
            .sorted_sels
            .iter()
            .any(|s| s.head >= line_start_char && s.head < line_end_char);
        style_row(
            row,
            graphemes,
            line_start_char,
            line_end_char,
            is_head_line,
            mode,
            theme,
            scratch,
        );
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
#[allow(clippy::too_many_arguments)]
fn collect_selection_spans(
    line_start_char: usize,
    line_end_char: usize,
    sorted_sels: &[Selection],
    primary_idx: Option<usize>,
    graphemes: &[Grapheme],
    row_range: &std::ops::Range<usize>,
    out: &mut Vec<(u16, u16)>,
    primary_sel_span: &mut Option<(u16, u16)>,
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
        let col_end = char_offset_to_col(sel_char_end, graphemes, row_range)
            .unwrap_or_else(|| row_gs.last().map_or(0, |g| g.col + g.width as u16));
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
    out: &mut Vec<u16>,
    primary_head_col: &mut Option<u16>,
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
/// immediately follows `char_offset`, returning its display column.
///
/// Returns `None` when `char_offset` is before this row's first grapheme (the
/// head belongs to an earlier wrap segment and should not be claimed for this row).
/// Returns `None` when `char_offset` is past all graphemes (caller uses a
/// fallback such as end-of-row column).
fn char_offset_to_col(
    char_offset: usize,
    graphemes: &[Grapheme],
    row_range: &std::ops::Range<usize>,
) -> Option<u16> {
    if char_offset == usize::MAX {
        // Sentinel value meaning "extend to end of row" — let the caller use the fallback.
        return None;
    }
    let row_graphemes = &graphemes[row_range.clone()];
    let idx = row_graphemes.partition_point(|g| g.char_offset < char_offset);
    // If char_offset falls before this row's first grapheme, the head belongs
    // to an earlier wrap segment — don't claim it for this row.
    row_graphemes.get(idx).and_then(|g| {
        if idx == 0 && char_offset < g.char_offset {
            None
        } else {
            Some(g.col)
        }
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ScopeRegistry, Theme};
    use crate::types::{
        CellContent, DisplayRow, Grapheme, ResolvedStyle, RowKind, ScopeId, Selection,
    };
    use std::collections::HashMap;

    fn make_graphemes(count: usize) -> Vec<Grapheme> {
        (0..count)
            .map(|i| Grapheme {
                byte_range: i..i + 1,
                char_offset: i,
                col: i as u16,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
            })
            .collect()
    }

    fn make_row(graphemes: std::ops::Range<usize>) -> DisplayRow {
        DisplayRow {
            kind: RowKind::LineStart { line_idx: 0 },
            graphemes,
        }
    }

    fn default_theme() -> Theme {
        Theme::default()
    }

    #[test]
    fn no_selections_yields_default_style() {
        let rope = ropey::Rope::from_str("abc");
        let graphemes = make_graphemes(3);
        let rows = vec![make_row(0..3)];
        let mut scratch = StyleScratch::new();
        resolve_styles(
            &rows,
            &graphemes,
            &[],
            0,
            EditorMode::Normal,
            &[],
            &default_theme(),
            &rope,
            None,
            &mut scratch,
        );

        assert_eq!(scratch.styles.len(), 3);
        assert!(
            scratch
                .styles
                .iter()
                .all(|s| *s == ResolvedStyle::default())
        );
    }

    #[test]
    fn selection_head_overrides_default() {
        let rope = ropey::Rope::from_str("abcde");
        let graphemes = make_graphemes(5);
        let rows = vec![make_row(0..5)];
        let selections = vec![Selection { anchor: 2, head: 2 }];

        // Theme with a cursor style so we can detect the override.
        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        resolve_styles(
            &rows,
            &graphemes,
            &selections,
            0,
            EditorMode::Normal,
            &[],
            &theme,
            &rope,
            None,
            &mut scratch,
        );

        // Grapheme at col 2 (index 2) should have the cursor style.
        assert_eq!(scratch.styles[2].fg, Some(ratatui::style::Color::Red));
        // Other graphemes should not.
        assert_eq!(scratch.styles[0].fg, None);
    }

    /// Build graphemes for "hello\n": 5 content graphemes + 1 eol sentinel.
    fn make_graphemes_with_sentinel() -> Vec<Grapheme> {
        let mut gs = (0..5usize)
            .map(|i| Grapheme {
                byte_range: i..i + 1,
                char_offset: i,
                col: i as u16,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
            })
            .collect::<Vec<_>>();
        // eol sentinel at char_offset=5, col=5 (the `\n` position).
        gs.push(Grapheme {
            byte_range: 5..5,
            char_offset: 5,
            col: 5,
            width: 1,
            content: CellContent::Empty,
            indent_depth: 0,
        });
        gs
    }

    /// After `x` (select-line), the selection head lands on the `\n` char.
    /// The eol sentinel grapheme must receive cursor styling so the cursor is visible.
    #[test]
    fn selection_head_on_newline_is_visible() {
        let rope = ropey::Rope::from_str("hello\n");
        let graphemes = make_graphemes_with_sentinel();
        let rows = vec![make_row(0..6)]; // all 6 graphemes in one row

        let mut styles_map = std::collections::HashMap::new();
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        // Line selection: anchor=0, head=5 (the '\n').
        let selections = vec![Selection { anchor: 0, head: 5 }];
        let mut scratch = StyleScratch::new();
        resolve_styles(
            &rows,
            &graphemes,
            &selections,
            0,
            EditorMode::Normal,
            &[],
            &theme,
            &rope,
            None,
            &mut scratch,
        );

        // The eol sentinel at index 5 must have the cursor style.
        assert_eq!(
            scratch.styles[5].fg,
            Some(ratatui::style::Color::Red),
            "eol sentinel (head on \\n) must receive cursor styling"
        );
        // The 'o' grapheme (index 4) must NOT have cursor styling (it's in selection, not head).
        assert_ne!(
            scratch.styles[4].fg,
            Some(ratatui::style::Color::Red),
            "grapheme before \\n must not have cursor styling"
        );
    }

    fn make_scope_ids(names: &[&'static str]) -> (ScopeRegistry, Vec<ScopeId>) {
        let mut reg = ScopeRegistry::new();
        let ids = names.iter().map(|&n| reg.intern(n)).collect();
        (reg, ids)
    }

    #[test]
    fn interval_cursor_basic() {
        let (_reg, ids) = make_scope_ids(&["kw", "fn"]);
        let (kw, fn_) = (ids[0], ids[1]);
        let intervals = vec![(2, 5, kw), (7, 9, fn_)];
        let mut cursor = IntervalCursor::new(&intervals);
        assert_eq!(cursor.scope_at(0), None);
        assert_eq!(cursor.scope_at(2), Some(kw));
        assert_eq!(cursor.scope_at(4), Some(kw));
        assert_eq!(cursor.scope_at(5), None);
        assert_eq!(cursor.scope_at(7), Some(fn_));
        assert_eq!(cursor.scope_at(9), None);
    }

    #[test]
    fn interval_cursor_empty() {
        let mut cursor = IntervalCursor::<'_>::new(&[]);
        assert_eq!(cursor.scope_at(0), None);
        assert_eq!(cursor.scope_at(100), None);
    }

    #[test]
    fn interval_cursor_adjacent_intervals() {
        // (2,5) and (5,8) are adjacent — byte 5 must match the second.
        let (_reg, ids) = make_scope_ids(&["kw", "fn"]);
        let (kw, fn_) = (ids[0], ids[1]);
        let intervals = vec![(2, 5, kw), (5, 8, fn_)];
        let mut cursor = IntervalCursor::new(&intervals);
        assert_eq!(cursor.scope_at(4), Some(kw));
        assert_eq!(cursor.scope_at(5), Some(fn_));
        assert_eq!(cursor.scope_at(7), Some(fn_));
        assert_eq!(cursor.scope_at(8), None);
    }

    #[test]
    fn selection_range_highlighted() {
        // Graphemes at cols 0,1,2. Selection spans chars 1..3 (cols 1 and 2).
        let rope = ropey::Rope::from_str("abc");
        let graphemes = make_graphemes(3);
        let rows = vec![make_row(0..3)];
        let selections = vec![Selection { anchor: 1, head: 3 }];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.selection",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        resolve_styles(
            &rows,
            &graphemes,
            &selections,
            0,
            EditorMode::Normal,
            &[],
            &theme,
            &rope,
            None,
            &mut scratch,
        );

        assert_eq!(scratch.styles[0].bg, None, "col 0 outside selection");
        assert_eq!(
            scratch.styles[1].bg,
            Some(ratatui::style::Color::Red),
            "col 1 inside selection"
        );
        assert_eq!(
            scratch.styles[2].bg,
            Some(ratatui::style::Color::Red),
            "col 2 inside selection"
        );
    }

    #[test]
    fn cursorline_background_applied_to_cursor_line_only() {
        // Two lines; cursor on line 0.
        // "ab\ncd": a=char0, b=char1, \n=char2, c=char3, d=char4
        let rope = ropey::Rope::from_str("ab\ncd");
        let g0 = Grapheme {
            byte_range: 0..1,
            char_offset: 0,
            col: 0,
            width: 1,
            content: crate::types::CellContent::Grapheme,
            indent_depth: 0,
        };
        let g1 = Grapheme {
            byte_range: 1..2,
            char_offset: 1,
            col: 1,
            width: 1,
            content: crate::types::CellContent::Grapheme,
            indent_depth: 0,
        };
        let g2 = Grapheme {
            byte_range: 0..1,
            char_offset: 3,
            col: 0,
            width: 1,
            content: crate::types::CellContent::Grapheme,
            indent_depth: 0,
        };
        let g3 = Grapheme {
            byte_range: 1..2,
            char_offset: 4,
            col: 1,
            width: 1,
            content: crate::types::CellContent::Grapheme,
            indent_depth: 0,
        };
        let graphemes = vec![g0, g1, g2, g3];
        let rows = vec![
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 0 },
                graphemes: 0..2,
            },
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 1 },
                graphemes: 2..4,
            },
        ];
        let selections = vec![Selection { anchor: 0, head: 0 }];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursorline",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Green),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        resolve_styles(
            &rows,
            &graphemes,
            &selections,
            0,
            EditorMode::Normal,
            &[],
            &theme,
            &rope,
            None,
            &mut scratch,
        );

        assert_eq!(
            scratch.styles[0].bg,
            Some(ratatui::style::Color::Green),
            "line 0 has cursorline bg"
        );
        assert_eq!(
            scratch.styles[1].bg,
            Some(ratatui::style::Color::Green),
            "line 0 has cursorline bg"
        );
        assert_eq!(scratch.styles[2].bg, None, "line 1 has no cursorline bg");
        assert_eq!(scratch.styles[3].bg, None, "line 1 has no cursorline bg");
    }

    #[test]
    fn insert_mode_uses_insert_cursor_scope() {
        let rope = ropey::Rope::from_str("ab");
        let graphemes = make_graphemes(2);
        let rows = vec![make_row(0..2)];
        let selections = vec![Selection { anchor: 0, head: 0 }];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursor.insert",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Green),
                ..Default::default()
            },
        );
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        resolve_styles(
            &rows,
            &graphemes,
            &selections,
            0,
            EditorMode::Insert,
            &[],
            &theme,
            &rope,
            None,
            &mut scratch,
        );

        assert_eq!(
            scratch.styles[0].fg,
            Some(ratatui::style::Color::Green),
            "Insert uses ui.cursor.insert scope"
        );
    }

    #[test]
    fn multi_head_all_lines_get_cursorline() {
        // Two selection heads on lines 0 and 2; line 1 should not get cursorline.
        // "a\nb\nc": a=char0, \n=char1, b=char2, \n=char3, c=char4
        let rope = ropey::Rope::from_str("a\nb\nc");
        let graphemes = vec![
            Grapheme {
                byte_range: 0..1,
                char_offset: 0,
                col: 0,
                width: 1,
                content: crate::types::CellContent::Grapheme,
                indent_depth: 0,
            },
            Grapheme {
                byte_range: 0..1,
                char_offset: 2,
                col: 0,
                width: 1,
                content: crate::types::CellContent::Grapheme,
                indent_depth: 0,
            },
            Grapheme {
                byte_range: 0..1,
                char_offset: 4,
                col: 0,
                width: 1,
                content: crate::types::CellContent::Grapheme,
                indent_depth: 0,
            },
        ];
        let rows = vec![
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 0 },
                graphemes: 0..1,
            },
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 1 },
                graphemes: 1..2,
            },
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 2 },
                graphemes: 2..3,
            },
        ];
        let selections = vec![
            Selection { anchor: 0, head: 0 },
            Selection { anchor: 4, head: 4 },
        ];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursorline",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Blue),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        resolve_styles(
            &rows,
            &graphemes,
            &selections,
            0,
            EditorMode::Normal,
            &[],
            &theme,
            &rope,
            None,
            &mut scratch,
        );

        assert_eq!(
            scratch.styles[0].bg,
            Some(ratatui::style::Color::Blue),
            "line 0 head line"
        );
        assert_eq!(scratch.styles[1].bg, None, "line 1 no head line");
        assert_eq!(
            scratch.styles[2].bg,
            Some(ratatui::style::Color::Blue),
            "line 2 head line"
        );
    }

    #[test]
    fn virtual_rows_keep_default_style() {
        let rope = ropey::Rope::from_str("ab");
        let graphemes = vec![
            Grapheme {
                byte_range: 0..1,
                char_offset: 0,
                col: 0,
                width: 1,
                content: crate::types::CellContent::Grapheme,
                indent_depth: 0,
            },
            Grapheme {
                byte_range: 0..0,
                char_offset: usize::MAX,
                col: 0,
                width: 1,
                content: crate::types::CellContent::Virtual("hint"),
                indent_depth: 0,
            },
        ];
        let rows = vec![
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 0 },
                graphemes: 0..1,
            },
            DisplayRow {
                kind: RowKind::Virtual {
                    provider_id: 0,
                    anchor_line: 0,
                },
                graphemes: 1..2,
            },
        ];
        let selections = vec![Selection { anchor: 0, head: 0 }];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursorline",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Blue),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        resolve_styles(
            &rows,
            &graphemes,
            &selections,
            0,
            EditorMode::Normal,
            &[],
            &theme,
            &rope,
            None,
            &mut scratch,
        );

        // Virtual row grapheme stays at default style.
        assert_eq!(scratch.styles[1], ResolvedStyle::default());
    }

    // ── Primary vs secondary selection head ─────────────────────────────────

    #[test]
    fn primary_head_gets_primary_style() {
        // Two selection heads on the same line (cols 0 and 2). Primary is first in the
        // selections slice (col 0). Theme has distinct styles for primary vs secondary.
        let rope = ropey::Rope::from_str("abcde");
        let graphemes = make_graphemes(5);
        let rows = vec![make_row(0..5)];
        let selections = vec![
            Selection { anchor: 0, head: 0 }, // primary (col 0)
            Selection { anchor: 2, head: 2 }, // secondary (col 2)
        ];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursor.primary",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Yellow),
                ..Default::default()
            },
        );
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        resolve_styles(
            &rows,
            &graphemes,
            &selections,
            0,
            EditorMode::Normal,
            &[],
            &theme,
            &rope,
            None,
            &mut scratch,
        );

        assert_eq!(
            scratch.styles[0].fg,
            Some(ratatui::style::Color::Yellow),
            "primary head gets ui.cursor.primary"
        );
        assert_eq!(
            scratch.styles[2].fg,
            Some(ratatui::style::Color::Red),
            "secondary head gets ui.cursor"
        );
        assert_eq!(scratch.styles[1].fg, None, "non-head grapheme unchanged");
    }

    #[test]
    fn primary_selection_gets_primary_style() {
        // Two selections on the same line. Primary is first (bytes 0..2), secondary is bytes 3..5.
        let rope = ropey::Rope::from_str("abcde");
        let graphemes = make_graphemes(5);
        let rows = vec![make_row(0..5)];
        let selections = vec![
            Selection { anchor: 0, head: 2 }, // primary
            Selection { anchor: 3, head: 5 }, // secondary
        ];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.selection.primary",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Cyan),
                ..Default::default()
            },
        );
        styles_map.insert(
            "ui.selection",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Blue),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        resolve_styles(
            &rows,
            &graphemes,
            &selections,
            0,
            EditorMode::Normal,
            &[],
            &theme,
            &rope,
            None,
            &mut scratch,
        );

        // Primary selection: cols 0 and 1 (bytes 0..2)
        assert_eq!(
            scratch.styles[0].bg,
            Some(ratatui::style::Color::Cyan),
            "col 0 in primary selection"
        );
        assert_eq!(
            scratch.styles[1].bg,
            Some(ratatui::style::Color::Cyan),
            "col 1 in primary selection"
        );
        // Secondary selection: cols 3 and 4 (bytes 3..5)
        assert_eq!(
            scratch.styles[3].bg,
            Some(ratatui::style::Color::Blue),
            "col 3 in secondary selection"
        );
        assert_eq!(
            scratch.styles[4].bg,
            Some(ratatui::style::Color::Blue),
            "col 4 in secondary selection"
        );
        // Col 2 is between selections
        assert_eq!(scratch.styles[2].bg, None, "col 2 outside both selections");
    }

    #[test]
    fn primary_head_falls_back_when_no_primary_scope() {
        // Theme does not define ui.cursor.primary — both heads should get ui.cursor.
        let rope = ropey::Rope::from_str("abcde");
        let graphemes = make_graphemes(5);
        let rows = vec![make_row(0..5)];
        let selections = vec![
            Selection { anchor: 0, head: 0 }, // primary
            Selection { anchor: 2, head: 2 }, // secondary
        ];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        resolve_styles(
            &rows,
            &graphemes,
            &selections,
            0,
            EditorMode::Normal,
            &[],
            &theme,
            &rope,
            None,
            &mut scratch,
        );

        // Both heads get ui.cursor via dot-notation fallback.
        assert_eq!(
            scratch.styles[0].fg,
            Some(ratatui::style::Color::Red),
            "primary falls back to ui.cursor"
        );
        assert_eq!(
            scratch.styles[2].fg,
            Some(ratatui::style::Color::Red),
            "secondary uses ui.cursor"
        );
    }

    #[test]
    fn head_on_wrapped_line_only_on_correct_segment() {
        // Simulate a wrapped line: line 0 has two display rows.
        // First segment: graphemes at byte ranges 0..1 (col 0), 1..2 (col 1), 2..3 (col 2).
        // Second segment: graphemes at byte ranges 3..4 (col 0), 4..5 (col 1).
        // Cursor head is at char_offset=1 (first segment). It must appear only on row 0.
        // "abcde" has no newlines so all chars are on line 0 with absolute char offsets 0..5.
        let rope = ropey::Rope::from_str("abcde");
        let graphemes = vec![
            Grapheme {
                byte_range: 0..1,
                char_offset: 0,
                col: 0,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
            },
            Grapheme {
                byte_range: 1..2,
                char_offset: 1,
                col: 1,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
            },
            Grapheme {
                byte_range: 2..3,
                char_offset: 2,
                col: 2,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
            },
            Grapheme {
                byte_range: 3..4,
                char_offset: 3,
                col: 0,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
            }, // wrap segment
            Grapheme {
                byte_range: 4..5,
                char_offset: 4,
                col: 1,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
            },
        ];
        let rows = vec![
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 0 },
                graphemes: 0..3,
            },
            DisplayRow {
                kind: RowKind::Wrap {
                    line_idx: 0,
                    wrap_row: 1,
                },
                graphemes: 3..5,
            },
        ];
        let selections = vec![Selection { anchor: 1, head: 1 }];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        resolve_styles(
            &rows,
            &graphemes,
            &selections,
            0,
            EditorMode::Normal,
            &[],
            &theme,
            &rope,
            None,
            &mut scratch,
        );

        // Selection head at byte 1 → col 1 in the first segment.
        assert_eq!(
            scratch.styles[1].fg,
            Some(ratatui::style::Color::Red),
            "selection head at col 1 in first segment"
        );
        // Second segment graphemes must NOT have the head style.
        assert_eq!(
            scratch.styles[3].fg, None,
            "wrap segment col 0 must not show head style"
        );
        assert_eq!(
            scratch.styles[4].fg, None,
            "wrap segment col 1 must not show head style"
        );
    }

    #[test]
    fn selection_on_wrapped_line_does_not_highlight_other_segments() {
        // Same wrapped line layout as head_on_wrapped_line_only_on_correct_segment.
        // A selection spanning chars 0..2 (cols 0–1 in segment 0) must not
        // produce a selection highlight on segment 1 at all.
        let rope = ropey::Rope::from_str("abcde");
        let graphemes = vec![
            Grapheme {
                byte_range: 0..1,
                char_offset: 0,
                col: 0,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
            },
            Grapheme {
                byte_range: 1..2,
                char_offset: 1,
                col: 1,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
            },
            Grapheme {
                byte_range: 2..3,
                char_offset: 2,
                col: 2,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
            },
            Grapheme {
                byte_range: 3..4,
                char_offset: 3,
                col: 0,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
            },
            Grapheme {
                byte_range: 4..5,
                char_offset: 4,
                col: 1,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
            },
        ];
        let rows = vec![
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 0 },
                graphemes: 0..3,
            },
            DisplayRow {
                kind: RowKind::Wrap {
                    line_idx: 0,
                    wrap_row: 1,
                },
                graphemes: 3..5,
            },
        ];
        let selections = vec![Selection { anchor: 0, head: 2 }];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.selection",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Blue),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        resolve_styles(
            &rows,
            &graphemes,
            &selections,
            0,
            EditorMode::Normal,
            &[],
            &theme,
            &rope,
            None,
            &mut scratch,
        );

        // Segment 0: cols 0 and 1 should be highlighted (selection spans bytes 0..2).
        assert_eq!(
            scratch.styles[0].bg,
            Some(ratatui::style::Color::Blue),
            "col 0 in selection"
        );
        assert_eq!(
            scratch.styles[1].bg,
            Some(ratatui::style::Color::Blue),
            "col 1 in selection"
        );
        assert_eq!(scratch.styles[2].bg, None, "col 2 outside selection");
        // Segment 1: no selection highlight at all.
        assert_eq!(
            scratch.styles[3].bg, None,
            "wrap segment col 0 must not show selection"
        );
        assert_eq!(
            scratch.styles[4].bg, None,
            "wrap segment col 1 must not show selection"
        );
    }
}
