mod highlight;
use highlight::HighlightStack;
pub use highlight::TierBufs;
pub(crate) use highlight::rebuild_tier_bufs;

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
// Public entry points
// ---------------------------------------------------------------------------

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
fn resolve_grapheme_col(
    char_offset: usize,
    graphemes: &[Grapheme],
    row_range: &std::ops::Range<usize>,
) -> Option<(u16, u16)> {
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
        g.char_offset == char_offset && matches!(g.content, crate::types::CellContent::Virtual { .. })
    }) {
        idx += 1;
    }
    row_graphemes.get(idx).map(|g| (g.col, g.width as u16))
}

/// Left edge (`g.col`) of the grapheme at `char_offset` in this row.
///
/// Returns `None` for the usize::MAX sentinel or for positions on an earlier
/// wrap segment. Callers use a fallback when `None`.
fn char_offset_to_col(
    char_offset: usize,
    graphemes: &[Grapheme],
    row_range: &std::ops::Range<usize>,
) -> Option<u16> {
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
) -> Option<u16> {
    resolve_grapheme_col(char_offset, graphemes, row_range).map(|(col, width)| col + width)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use crate::types::{CellContent, DisplayRow, Grapheme, ResolvedStyle, RowKind, Selection};
    use std::collections::HashMap;

    /// Test driver mirroring the live pipeline's Style-stage orchestration
    /// (`pipeline.rs::render_buffer_line`): primary-based `is_head_line`,
    /// `rebuild_tier_bufs` once per buffer line, `style_row` per display row.
    /// No highlight providers or tree — these tests cover cursor/selection styling only.
    fn apply_styles(
        rows: &[DisplayRow],
        graphemes: &[Grapheme],
        selections: &[Selection],
        mode: EditorMode,
        theme: &Theme,
        rope: &ropey::Rope,
        scratch: &mut StyleScratch,
    ) {
        scratch.populate_sorted_sels(selections, 0);
        scratch
            .styles
            .resize(graphemes.len(), ResolvedStyle::default());
        let mut current_line: Option<usize> = None;
        for row in rows {
            let Some(line_idx) = row.kind.line_idx() else {
                continue; // virtual row: styles stay default
            };
            if current_line != Some(line_idx) {
                current_line = Some(line_idx);
                rebuild_tier_bufs(line_idx, None, &[], rope, None, scratch);
            }
            let line_start_char = rope.line_to_char(line_idx);
            let line_end_char = rope.line_to_char(line_idx + 1);
            let is_head_line = scratch
                .primary_idx_in_sorted
                .and_then(|i| scratch.sorted_sels.get(i))
                .is_some_and(|s| s.head >= line_start_char && s.head < line_end_char);
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

    fn make_graphemes(count: usize) -> Vec<Grapheme> {
        (0..count)
            .map(|i| Grapheme {
                byte_range: i..i + 1,
                char_offset: i,
                col: i as u16,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
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
        apply_styles(
            &rows,
            &graphemes,
            &[],
            EditorMode::Normal,
            &default_theme(),
            &rope,
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
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
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
                scope: None,
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
            scope: None,
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
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
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
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
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

    /// Regression test: backward selections (head < anchor, e.g. after flip-selections)
    /// must highlight their full inclusive range. Before the fix, the anchor cell at
    /// the high end of the range was excluded from the selection span and rendered plain.
    #[test]
    fn backward_selection_anchor_cell_highlighted() {
        // "foo": chars 0,1,2. Backward selection: head=0, anchor=2.
        // Expected: col 0 painted as cursor (head), cols 1 and 2 painted as selection.
        let rope = ropey::Rope::from_str("foo");
        let graphemes = make_graphemes(3);
        let rows = vec![make_row(0..3)];
        let selections = vec![Selection { anchor: 2, head: 0 }];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.selection",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Blue),
                ..Default::default()
            },
        );
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::White),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        assert_eq!(
            scratch.styles[0].fg,
            Some(ratatui::style::Color::White),
            "col 0 is the head — must have cursor fg"
        );
        assert_eq!(
            scratch.styles[1].bg,
            Some(ratatui::style::Color::Blue),
            "col 1 is inside selection — must have selection bg"
        );
        // Regression: col 2 is the anchor (highest char), was rendered plain before fix.
        assert_eq!(
            scratch.styles[2].bg,
            Some(ratatui::style::Color::Blue),
            "col 2 is the anchor — must have selection bg (regression)"
        );
    }

    /// Regression: a collapsed selection (anchor == head, i.e. bare cursor) must
    /// not emit a selection-highlight span. In Insert mode the bar cursor is
    /// transparent, so a spurious 1-cell span shows through as a highlighted char.
    #[test]
    fn insert_mode_collapsed_selection_not_highlighted() {
        let rope = ropey::Rope::from_str("foo");
        let graphemes = make_graphemes(3);
        let rows = vec![make_row(0..3)];
        // Collapsed selection: head == anchor == char 1 (the 'o').
        let selections = vec![Selection { anchor: 1, head: 1 }];

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
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Insert,
            &theme,
            &rope,
            &mut scratch,
        );

        // The cursor cell itself carries Tier-0 cursor styling, not selection bg.
        assert_ne!(
            scratch.styles[1].bg,
            Some(ratatui::style::Color::Blue),
            "col 1 is the collapsed cursor — must NOT have selection bg"
        );
        // Neighboring cells are also not highlighted.
        assert_ne!(
            scratch.styles[0].bg,
            Some(ratatui::style::Color::Blue),
            "col 0 not highlighted"
        );
        assert_ne!(
            scratch.styles[2].bg,
            Some(ratatui::style::Color::Blue),
            "col 2 not highlighted"
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
            scope: None,
        };
        let g1 = Grapheme {
            byte_range: 1..2,
            char_offset: 1,
            col: 1,
            width: 1,
            content: crate::types::CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        };
        let g2 = Grapheme {
            byte_range: 0..1,
            char_offset: 3,
            col: 0,
            width: 1,
            content: crate::types::CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        };
        let g3 = Grapheme {
            byte_range: 1..2,
            char_offset: 4,
            col: 1,
            width: 1,
            content: crate::types::CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
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
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
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
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Insert,
            &theme,
            &rope,
            &mut scratch,
        );

        assert_eq!(
            scratch.styles[0].fg,
            Some(ratatui::style::Color::Green),
            "Insert uses ui.cursor.insert scope"
        );
    }

    #[test]
    fn insert_head_is_transparent_without_insert_scope() {
        // Theme defines ui.cursor with a block bg but NOT ui.cursor.insert.
        // In Insert mode the head cell must NOT inherit the block bg so the real
        // terminal bar cursor shows through.
        let rope = ropey::Rope::from_str("abcde");
        let graphemes = make_graphemes(5);
        let rows = vec![make_row(0..5)];
        // Two selections: head 0 = primary, head 2 = secondary.
        let selections = vec![
            Selection { anchor: 0, head: 0 },
            Selection { anchor: 2, head: 2 },
        ];

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let theme = Theme::new(styles_map, ResolvedStyle::default());

        let mut scratch = StyleScratch::new();
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Insert,
            &theme,
            &rope,
            &mut scratch,
        );

        assert_eq!(
            scratch.styles[0].bg, None,
            "primary insert head has no block bg"
        );
        assert_eq!(
            scratch.styles[2].bg, None,
            "secondary insert head has no block bg"
        );
    }

    #[test]
    fn cursorline_applies_only_to_primary_head_line() {
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
                scope: None,
            },
            Grapheme {
                byte_range: 0..1,
                char_offset: 2,
                col: 0,
                width: 1,
                content: crate::types::CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
            Grapheme {
                byte_range: 0..1,
                char_offset: 4,
                col: 0,
                width: 1,
                content: crate::types::CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
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
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        assert_eq!(
            scratch.styles[0].bg,
            Some(ratatui::style::Color::Blue),
            "line 0 head line"
        );
        assert_eq!(scratch.styles[1].bg, None, "line 1 no head line");
        // line 2 has a non-primary selection head: primary-based is_head_line = false,
        // so the live pipeline does NOT apply cursorline there.
        assert_eq!(
            scratch.styles[2].bg, None,
            "line 2 non-primary head: no cursorline"
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
                scope: None,
            },
            Grapheme {
                byte_range: 0..0,
                char_offset: usize::MAX,
                col: 0,
                width: 1,
                content: crate::types::CellContent::Virtual { start: 0, len: 4 },
                indent_depth: 0,
                scope: None,
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
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
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
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
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
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
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
        // Col 2 is the head of the primary selection — included in the span, so it gets primary bg.
        assert_eq!(
            scratch.styles[2].bg,
            Some(ratatui::style::Color::Cyan),
            "col 2 is primary head — must have primary selection bg"
        );
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
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
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
                scope: None,
            },
            Grapheme {
                byte_range: 1..2,
                char_offset: 1,
                col: 1,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
            Grapheme {
                byte_range: 2..3,
                char_offset: 2,
                col: 2,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
            Grapheme {
                byte_range: 3..4,
                char_offset: 3,
                col: 0,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            }, // wrap segment
            Grapheme {
                byte_range: 4..5,
                char_offset: 4,
                col: 1,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
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
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
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
                scope: None,
            },
            Grapheme {
                byte_range: 1..2,
                char_offset: 1,
                col: 1,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
            Grapheme {
                byte_range: 2..3,
                char_offset: 2,
                col: 2,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
            Grapheme {
                byte_range: 3..4,
                char_offset: 3,
                col: 0,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            },
            Grapheme {
                byte_range: 4..5,
                char_offset: 4,
                col: 1,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
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
        apply_styles(
            &rows,
            &graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
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
        // Col 2 is the head of the selection (char 2 is included in [0,2]); it gets selection bg.
        assert_eq!(
            scratch.styles[2].bg,
            Some(ratatui::style::Color::Blue),
            "col 2 is selection head — included in inclusive span"
        );
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

    // ── Inline-insert scope styling (B3) ─────────────────────────────────

    #[test]
    fn inline_insert_scope_is_layered_but_neighbour_is_not() {
        // Insert with an interned scope mapped to fg: Red. The insert cell's
        // resolved style must carry that scope; the real grapheme next to it
        // must not.
        let rope = ropey::Rope::from_str("ab");
        let mut registry = crate::theme::ScopeRegistry::new();
        let hint_scope = registry.intern("hint");
        let inserts = vec![crate::providers::InlineInsert {
            byte_offset: 0,
            text: "H".into(),
            scope: hint_scope,
        }];
        let mut fmt = crate::format::FormatScratch::new();
        crate::format::format_buffer_line(
            &rope,
            0,
            4,
            &crate::pane::WhitespaceConfig::default(),
            &crate::pane::WrapMode::None,
            None,
            &inserts,
            &mut fmt,
        );

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "hint",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let mut theme = Theme::new(styles_map, ResolvedStyle::default());
        theme.bake(&registry);

        let mut scratch = StyleScratch::new();
        apply_styles(
            &fmt.display_rows,
            &fmt.graphemes,
            &[],
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        let insert_idx = fmt
            .graphemes
            .iter()
            .position(|g| matches!(g.content, CellContent::Virtual { .. }))
            .expect("insert grapheme present");
        let a_idx = fmt
            .graphemes
            .iter()
            .position(|g| g.char_offset == 0 && matches!(g.content, CellContent::Grapheme))
            .expect("'a' grapheme present");

        assert_eq!(
            scratch.styles[insert_idx].fg,
            Some(ratatui::style::Color::Red),
            "insert cell must carry its own scope's style"
        );
        assert_eq!(
            scratch.styles[a_idx].fg, None,
            "neighbouring real grapheme must not inherit the insert's scope"
        );
    }

    // ── Inline-insert char_offset partition invariant (B2) ────────────────

    /// Drive the real formatter with a mid-row insert, then style the result —
    /// end-to-end coverage that `resolve_grapheme_col`'s partition_point lands
    /// on the real grapheme, not the insert sharing its char_offset.
    #[test]
    fn insert_mid_row_head_resolves_to_real_grapheme_col() {
        // "abcdef", width-2 insert before 'c' (byte offset 2). Layout by hand:
        // a(col0) b(col1) [insert XY](col2..4) c(col4) d(col5) e(col6) f(col7).
        // The insert and 'c' share char_offset 2 (the insert is pushed first,
        // at the offset of the grapheme it precedes) — the exact tie
        // `resolve_grapheme_col` must break in favour of the real grapheme.
        // Cursor at char 2 ('c') must land at col 4, not the insert's col 2.
        let rope = ropey::Rope::from_str("abcdef");
        let mut registry = crate::theme::ScopeRegistry::new();
        let insert_scope = registry.intern("test");
        let inserts = vec![crate::providers::InlineInsert {
            byte_offset: 2,
            text: "XY".into(),
            scope: insert_scope,
        }];
        let mut fmt = crate::format::FormatScratch::new();
        crate::format::format_buffer_line(
            &rope,
            0,
            4,
            &crate::pane::WhitespaceConfig::default(),
            &crate::pane::WrapMode::None,
            None,
            &inserts,
            &mut fmt,
        );

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.cursor",
            ResolvedStyle {
                fg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let mut theme = Theme::new(styles_map, ResolvedStyle::default());
        theme.bake(&registry);
        let selections = vec![Selection { anchor: 2, head: 2 }];
        let mut scratch = StyleScratch::new();
        apply_styles(
            &fmt.display_rows,
            &fmt.graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        let c_idx = fmt
            .graphemes
            .iter()
            .position(|g| g.char_offset == 2 && matches!(g.content, CellContent::Grapheme))
            .expect("'c' grapheme present");
        assert_eq!(
            fmt.graphemes[c_idx].col, 4,
            "'c' shifts right by the insert's width"
        );
        assert_eq!(
            scratch.styles[c_idx].fg,
            Some(ratatui::style::Color::Red),
            "cursor head must land on 'c', not the insert sharing its char_offset"
        );

        let insert_idx = fmt
            .graphemes
            .iter()
            .position(|g| matches!(g.content, CellContent::Virtual { .. }))
            .expect("insert grapheme present");
        assert_ne!(
            scratch.styles[insert_idx].fg,
            Some(ratatui::style::Color::Red),
            "the insert cell itself must not receive cursor styling"
        );
    }

    #[test]
    fn selection_spanning_row_start_insert_begins_at_first_real_grapheme() {
        // Insert at byte 0 — the row starts with a virtual cell at col 0,
        // then 'a' at col 1, 'b' at col 2, etc. A selection over chars 0..1
        // ('a','b') must start its highlighted span at 'a's col (1), not the
        // insert's col (0).
        let rope = ropey::Rope::from_str("abcdef");
        let mut registry = crate::theme::ScopeRegistry::new();
        let insert_scope = registry.intern("test");
        let inserts = vec![crate::providers::InlineInsert {
            byte_offset: 0,
            text: "Z".into(),
            scope: insert_scope,
        }];
        let mut fmt = crate::format::FormatScratch::new();
        crate::format::format_buffer_line(
            &rope,
            0,
            4,
            &crate::pane::WhitespaceConfig::default(),
            &crate::pane::WrapMode::None,
            None,
            &inserts,
            &mut fmt,
        );

        let mut styles_map = HashMap::new();
        styles_map.insert(
            "ui.selection",
            ResolvedStyle {
                bg: Some(ratatui::style::Color::Blue),
                ..Default::default()
            },
        );
        let mut theme = Theme::new(styles_map, ResolvedStyle::default());
        theme.bake(&registry);
        let selections = vec![Selection { anchor: 0, head: 1 }]; // 'a' and 'b'
        let mut scratch = StyleScratch::new();
        apply_styles(
            &fmt.display_rows,
            &fmt.graphemes,
            &selections,
            EditorMode::Normal,
            &theme,
            &rope,
            &mut scratch,
        );

        let insert_idx = fmt
            .graphemes
            .iter()
            .position(|g| matches!(g.content, CellContent::Virtual { .. }))
            .expect("insert grapheme present");
        assert_eq!(fmt.graphemes[insert_idx].col, 0);
        assert_eq!(
            scratch.styles[insert_idx].bg, None,
            "the row-start insert cell must not be painted as part of the selection"
        );

        let a_idx = fmt
            .graphemes
            .iter()
            .position(|g| g.char_offset == 0 && matches!(g.content, CellContent::Grapheme))
            .expect("'a' grapheme present");
        let b_idx = fmt
            .graphemes
            .iter()
            .position(|g| g.char_offset == 1 && matches!(g.content, CellContent::Grapheme))
            .expect("'b' grapheme present");
        assert_eq!(fmt.graphemes[a_idx].col, 1);
        assert_eq!(
            scratch.styles[a_idx].bg,
            Some(ratatui::style::Color::Blue),
            "'a' is the first real grapheme — selection span must start here"
        );
        assert_eq!(scratch.styles[b_idx].bg, Some(ratatui::style::Color::Blue));
    }
}
