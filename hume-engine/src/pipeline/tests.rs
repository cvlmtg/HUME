use super::*;
use ratatui::layout::Rect;

use super::layout::split_rect;
use super::pane_render::emit_virtual_row;
use crate::providers::VirtualLineAnchor;
use crate::render::{self, ComposeCtx};
use crate::types::{ResolvedStyle, RowKind};

fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

// ── emit_virtual_row scope styling (B3/B4) ──────────────────────────

fn make_compose_ctx<'a>(
    visible: &'a crate::layout::VisibleRange,
    viewport: &'a crate::pane::ViewportState,
    theme: &'a Theme,
    pane_rect: Rect,
    rope: &'a ropey::Rope,
) -> ComposeCtx<'a> {
    ComposeCtx {
        gutter_columns: &[],
        visible,
        viewport,
        mode: EditorMode::Normal,
        primary_head_line: 0,
        tab_width: 4,
        tilde_style: theme.ui.virtual_text.into(),
        indent_guide_style: theme.ui.indent_guide.into(),
        pane_rect,
        theme,
        pane_bg: None,
        rope,
        tree: None,
    }
}

#[test]
fn emit_virtual_row_resolves_grapheme_scope_and_falls_back_to_virtual_text() {
    // Two graphemes: one carries an interned scope (must resolve to that
    // scope's fg), one carries no scope (must fall back to ui.virtual_text).
    let mut registry = ScopeRegistry::new();
    let hint_scope = registry.intern("hint");
    let mut styles_map = HashMap::new();
    styles_map.insert(
        "hint",
        ResolvedStyle {
            fg: Some(ratatui::style::Color::Red),
            ..Default::default()
        },
    );
    styles_map.insert(
        "ui.virtual",
        ResolvedStyle {
            fg: Some(ratatui::style::Color::Blue),
            ..Default::default()
        },
    );
    let mut theme = Theme::new(styles_map, ResolvedStyle::default());
    theme.bake(&registry);

    let mut scratch = FrameScratch::new();
    scratch
        .format
        .virtual_lines
        .push(crate::providers::VirtualLine {
            anchor: VirtualLineAnchor::Before(0),
            provider_id: 0,
            text: "H~".to_string(),
            // "H" (byte 0..1) carries the scope; "~" (byte 1..2) carries
            // none and must fall back to `ui.virtual_text`.
            segments: vec![(0..1, hint_scope)],
        });

    let visible = crate::layout::VisibleRange {
        line_range: 0..1,
        top_skip_rows: 0,
        content_height: 5,
        content_width: 20,
        gutter_width: 0,
        last_line_idx: 0,
    };
    let viewport = crate::pane::ViewportState::new(20, 5);
    let pane_rect = rect(0, 0, 20, 5);
    let rope = ropey::Rope::new();
    let compose_ctx = make_compose_ctx(&visible, &viewport, &theme, pane_rect, &rope);
    let mut buf = ratatui::buffer::Buffer::empty(pane_rect);
    let mut canvas = render::PaneCanvas::new(&mut buf, None);

    emit_virtual_row(0, 0, 0, &mut scratch, &compose_ctx, &mut canvas);

    let scoped_cell = buf.cell(ratatui::layout::Position { x: 0, y: 0 }).unwrap();
    assert_eq!(
        scoped_cell.fg,
        ratatui::style::Color::Red,
        "grapheme with Some(scope) resolves that scope's style"
    );
    let fallback_cell = buf.cell(ratatui::layout::Position { x: 1, y: 0 }).unwrap();
    assert_eq!(
        fallback_cell.fg,
        ratatui::style::Color::Blue,
        "grapheme with no scope falls back to ui.virtual_text"
    );
}

// ── top_skip vs virtual lines (B5) ──────────────────────────────────

/// Emits one virtual line anchored to a fixed line index, only when that
/// line is in the visible range — smoke-tests the whole virtual-line path
/// (`ProviderSet::add_virtual_line_source` → `render_pane`) for the first
/// time, per the plan's note that no real `VirtualLineSource` exists yet.
struct FixedVirtualLineSource {
    anchor: VirtualLineAnchor,
}

impl crate::providers::VirtualLineSource for FixedVirtualLineSource {
    fn virtual_lines(
        &self,
        visible_lines: std::ops::Range<usize>,
        _content_width: u16,
        out: &mut Vec<crate::providers::VirtualLine>,
    ) {
        let line = match self.anchor {
            VirtualLineAnchor::Before(n) | VirtualLineAnchor::After(n) => n,
        };
        if visible_lines.contains(&line) {
            out.push(crate::providers::VirtualLine {
                anchor: self.anchor,
                provider_id: 0,
                text: "V".to_string(),
                segments: Vec::new(),
            });
        }
    }
}

/// Build a pane viewing a rope whose line 0 is "aaaabbbbcccc" — under
/// `WrapMode::Soft { width: 4 }` this wraps into exactly 3 rows: "aaaa",
/// "bbbb", "cccc" (each grapheme is 1 column, so Soft splits at the exact
/// column with no backtracking). `top_row_offset` controls how many of
/// those wrap rows are already scrolled past.
fn render_wrapped_pane_with_virtual_line(
    top_row_offset: u16,
    anchor: VirtualLineAnchor,
) -> ratatui::buffer::Buffer {
    let rope = ropey::Rope::from_str("aaaabbbbcccc\nz\n");
    let mut bids: SlotMap<BufferId, ()> = SlotMap::with_key();
    let bid = bids.insert(());

    let mut pane = Pane::new(bid, WrapMode::Soft { width: 4 });
    pane.viewport = crate::pane::ViewportState::new(10, 6);
    pane.viewport.top_row_offset = top_row_offset;
    pane.providers
        .add_virtual_line_source(Box::new(FixedVirtualLineSource { anchor }));

    let theme = Theme::default();
    let pane_rect = rect(0, 0, 10, 6);
    let pane_ctx = PaneRenderCtx {
        pane: &pane,
        rope: &rope,
        syntax: None,
        theme: &theme,
        rect: pane_rect,
        settings: PaneRenderSettings {
            mode: EditorMode::Normal,
            wrap_mode: WrapMode::Soft { width: 4 },
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
        },
        dim: None,
    };
    let mut scratch = FrameScratch::new();
    let mut buf = ratatui::buffer::Buffer::empty(pane_rect);
    render_pane(&pane_ctx, &mut scratch, &mut buf);
    buf
}

fn cell_symbol(buf: &ratatui::buffer::Buffer, x: u16, y: u16) -> String {
    buf.cell(ratatui::layout::Position { x, y })
        .unwrap()
        .symbol()
        .to_string()
}

#[test]
fn before_virtual_line_dropped_without_stealing_skip_budget() {
    // top_row_offset=1 skips wrap row 0 ("aaaa"). The Before(0) virtual
    // line must be dropped too, but WITHOUT consuming another unit of the
    // skip budget — screen row 0 must show wrap row 1 ("bbbb"), not wrap
    // row 2 ("cccc") and not the virtual line.
    let buf = render_wrapped_pane_with_virtual_line(1, VirtualLineAnchor::Before(0));
    assert_eq!(cell_symbol(&buf, 0, 0), "b", "screen row 0 is wrap row 1");
    assert_eq!(cell_symbol(&buf, 1, 0), "b");
    assert_eq!(cell_symbol(&buf, 2, 0), "b");
    assert_eq!(cell_symbol(&buf, 3, 0), "b");
    assert_eq!(cell_symbol(&buf, 0, 1), "c", "screen row 1 is wrap row 2");
}

#[test]
fn before_virtual_line_renders_when_not_skipped() {
    // top_row_offset=0: the Before(0) virtual line renders at screen row
    // 0, pushing wrap row 0 ("aaaa") down to screen row 1.
    let buf = render_wrapped_pane_with_virtual_line(0, VirtualLineAnchor::Before(0));
    assert_eq!(cell_symbol(&buf, 0, 0), "V", "virtual line at screen row 0");
    assert_eq!(
        cell_symbol(&buf, 0, 1),
        "a",
        "wrap row 0 pushed to screen row 1"
    );
}

#[test]
fn after_virtual_line_renders_below_skipped_rows() {
    // top_row_offset=1 skips wrap row 0. The After(0) virtual line sits
    // below all of line 0's wrap rows, which are not skipped (the budget
    // is exhausted by wrap row 0 alone) — it must still render, after
    // wrap rows 1 and 2.
    let buf = render_wrapped_pane_with_virtual_line(1, VirtualLineAnchor::After(0));
    assert_eq!(cell_symbol(&buf, 0, 0), "b", "wrap row 1");
    assert_eq!(cell_symbol(&buf, 0, 1), "c", "wrap row 2");
    assert_eq!(
        cell_symbol(&buf, 0, 2),
        "V",
        "After(0) virtual line still renders"
    );
}

// ── Provider id stamping (G3) ────────────────────────────────────────

/// Reports a deliberately wrong `provider_id` — the pipeline must not
/// trust it.
struct SpoofingVirtualLineSource {
    anchor: VirtualLineAnchor,
}

impl crate::providers::VirtualLineSource for SpoofingVirtualLineSource {
    fn virtual_lines(
        &self,
        visible_lines: std::ops::Range<usize>,
        _content_width: u16,
        out: &mut Vec<crate::providers::VirtualLine>,
    ) {
        let line = match self.anchor {
            VirtualLineAnchor::Before(n) | VirtualLineAnchor::After(n) => n,
        };
        if visible_lines.contains(&line) {
            out.push(crate::providers::VirtualLine {
                anchor: self.anchor,
                provider_id: 9999, // spoofed — must be overwritten by the pipeline
                text: "V".to_string(),
                segments: Vec::new(),
            });
        }
    }
}

/// Renders the `RowKind::Virtual` provider_id as gutter text, so the test
/// can observe what actually reached `compose` after collection.
struct ProviderIdReportingGutter;

impl crate::providers::GutterColumn for ProviderIdReportingGutter {
    fn width(&self, _: usize) -> u8 {
        5
    }
    fn render_row_cells(
        &self,
        kind: RowKind,
        _: &crate::providers::GutterRowCtx,
    ) -> Vec<crate::providers::GutterCell> {
        let cell = match kind {
            RowKind::Virtual { provider_id, .. } => crate::providers::GutterCell {
                content: crate::providers::GutterCellContent::Text(std::borrow::Cow::Owned(
                    provider_id.to_string(),
                )),
                scope: crate::types::Scope("ui.linenr").into(),
            },
            _ => crate::providers::GutterCell::blank(crate::types::Scope("ui.linenr")),
        };
        vec![cell]
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[test]
fn virtual_line_provider_id_is_stamped_by_pipeline_not_self_reported() {
    let rope = ropey::Rope::from_str("a\n");
    let mut bids: SlotMap<BufferId, ()> = SlotMap::with_key();
    let bid = bids.insert(());
    let mut pane = Pane::new(bid, WrapMode::None);
    pane.viewport = crate::pane::ViewportState::new(10, 3);
    pane.providers
        .add_gutter_column(Box::new(ProviderIdReportingGutter));
    let real_id = pane
        .providers
        .add_virtual_line_source(Box::new(SpoofingVirtualLineSource {
            anchor: VirtualLineAnchor::Before(0),
        }));

    let theme = Theme::default();
    let pane_rect = rect(0, 0, 10, 3);
    let pane_ctx = PaneRenderCtx {
        pane: &pane,
        rope: &rope,
        syntax: None,
        theme: &theme,
        rect: pane_rect,
        settings: PaneRenderSettings {
            mode: EditorMode::Normal,
            wrap_mode: WrapMode::None,
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
        },
        dim: None,
    };
    let mut scratch = FrameScratch::new();
    let mut buf = ratatui::buffer::Buffer::empty(pane_rect);
    render_pane(&pane_ctx, &mut scratch, &mut buf);

    // Gutter width 5 -> usable 4 columns; a small real_id fits comfortably.
    let gutter_text: String = (0..4).map(|x| cell_symbol(&buf, x, 0)).collect();
    assert_eq!(
        gutter_text.trim(),
        real_id.to_string(),
        "gutter must show the pipeline-stamped id ({real_id}), not the spoofed 9999"
    );
}

// ── CJK line-row estimate feeds the viewport (B6) ───────────────────

#[test]
fn cjk_heavy_viewport_fills_every_row_no_premature_filler() {
    // Two lines of 20 '中' chars each (true width 40 per line). At
    // WrapMode::Soft { width: 20 } each line wraps into exactly 2 rows,
    // so the two lines together supply exactly 4 real rows — matching
    // a 4-row viewport with nothing left over for tilde fillers.
    let line: String = "中".repeat(20);
    let rope = ropey::Rope::from_str(&format!("{line}\n{line}\n"));
    let mut bids: SlotMap<BufferId, ()> = SlotMap::with_key();
    let bid = bids.insert(());

    let mut pane = Pane::new(bid, WrapMode::Soft { width: 20 });
    pane.viewport = crate::pane::ViewportState::new(20, 4);

    let theme = Theme::default();
    let pane_rect = rect(0, 0, 20, 4);
    let pane_ctx = PaneRenderCtx {
        pane: &pane,
        rope: &rope,
        syntax: None,
        theme: &theme,
        rect: pane_rect,
        settings: PaneRenderSettings {
            mode: EditorMode::Normal,
            wrap_mode: WrapMode::Soft { width: 20 },
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
        },
        dim: None,
    };
    let mut scratch = FrameScratch::new();
    let mut buf = ratatui::buffer::Buffer::empty(pane_rect);
    render_pane(&pane_ctx, &mut scratch, &mut buf);

    for y in 0..4u16 {
        let sym = cell_symbol(&buf, 0, y);
        assert_eq!(
            sym, "中",
            "row {y} must be real CJK content, not a tilde filler"
        );
    }
}

// ── Dual compose path migration (B10) ───────────────────────────────

#[test]
fn scrolled_pane_renders_from_top_line_onward() {
    // "ab\ncd\n": scrolling to top_line=1 must show "cd" at screen row 0
    // — migrated from render.rs's old top_skip_rows_skips_first_row,
    // which drove the deleted batch `compose()` path directly. The
    // fused pipeline's equivalent scroll mechanism in `WrapMode::None`
    // is `viewport.top_line`, not `top_row_offset` (which only sub-scrolls
    // within a wrapped `top_line`'s own rows).
    let rope = ropey::Rope::from_str("ab\ncd\n");
    let mut bids: SlotMap<BufferId, ()> = SlotMap::with_key();
    let bid = bids.insert(());

    let mut pane = Pane::new(bid, WrapMode::None);
    pane.viewport = crate::pane::ViewportState::new(20, 5);
    pane.viewport.top_line = 1;

    let theme = Theme::default();
    let pane_rect = rect(0, 0, 20, 5);
    let pane_ctx = PaneRenderCtx {
        pane: &pane,
        rope: &rope,
        syntax: None,
        theme: &theme,
        rect: pane_rect,
        settings: PaneRenderSettings {
            mode: EditorMode::Normal,
            wrap_mode: WrapMode::None,
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
        },
        dim: None,
    };
    let mut scratch = FrameScratch::new();
    let mut buf = ratatui::buffer::Buffer::empty(pane_rect);
    render_pane(&pane_ctx, &mut scratch, &mut buf);

    assert_eq!(cell_symbol(&buf, 0, 0), "c");
    assert_eq!(cell_symbol(&buf, 1, 0), "d");
}

#[test]
fn filler_row_gutter_shows_gutter_content_not_stale_blank() {
    // Filler rows past EOF must still get their gutter column consulted
    // — before B10's fix, only compose_row's gutter loop ran for real
    // rows; render_tilde_fillers never called it, so a filler row's
    // gutter area was silently blank regardless of what a custom
    // GutterColumn would render for RowKind::Filler.
    struct MarkerGutter;
    impl crate::providers::GutterColumn for MarkerGutter {
        fn width(&self, _: usize) -> u8 {
            3
        }
        fn render_row_cells(
            &self,
            kind: RowKind,
            _: &crate::providers::GutterRowCtx,
        ) -> Vec<crate::providers::GutterCell> {
            let text = if matches!(kind, RowKind::Filler) {
                "~g"
            } else {
                "ln"
            };
            vec![crate::providers::GutterCell {
                content: crate::providers::GutterCellContent::Text(std::borrow::Cow::Borrowed(
                    text,
                )),
                scope: crate::types::Scope("ui.linenr").into(),
            }]
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    let rope = ropey::Rope::from_str("x\n");
    let mut bids: SlotMap<BufferId, ()> = SlotMap::with_key();
    let bid = bids.insert(());

    let mut pane = Pane::new(bid, WrapMode::None);
    pane.viewport = crate::pane::ViewportState::new(20, 3); // 1 real row + 2 filler rows
    pane.providers.add_gutter_column(Box::new(MarkerGutter));

    let theme = Theme::default();
    let pane_rect = rect(0, 0, 20, 3);
    let pane_ctx = PaneRenderCtx {
        pane: &pane,
        rope: &rope,
        syntax: None,
        theme: &theme,
        rect: pane_rect,
        settings: PaneRenderSettings {
            mode: EditorMode::Normal,
            wrap_mode: WrapMode::None,
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
        },
        dim: None,
    };
    let mut scratch = FrameScratch::new();
    let mut buf = ratatui::buffer::Buffer::empty(pane_rect);
    render_pane(&pane_ctx, &mut scratch, &mut buf);

    // Row 1 is a Filler row (past the single real line) — its gutter
    // must show the column's own Filler rendering ("~g"), not blank.
    assert_eq!(cell_symbol(&buf, 0, 1), "~");
    assert_eq!(cell_symbol(&buf, 1, 1), "g");
}

// ── split_rect ───────────────────────────────────────────────────────

#[test]
fn split_rect_horizontal_half() {
    // width=100 → usable=99, w1 = (99*0.5) as u16 = 49 (truncates, not rounds).
    let (a, seam, b) = split_rect(rect(0, 0, 100, 50), false, 0.5, true);
    assert_eq!(a, rect(0, 0, 49, 50));
    assert_eq!(seam, rect(49, 0, 1, 50));
    assert_eq!(b, rect(50, 0, 50, 50));
}

#[test]
fn split_rect_vertical_half() {
    // height=50 → usable=49, h1 = (49*0.5) as u16 = 24 (truncates, not rounds).
    let (a, seam, b) = split_rect(rect(0, 0, 100, 50), true, 0.5, true);
    assert_eq!(a, rect(0, 0, 100, 24));
    assert_eq!(seam, rect(0, 24, 100, 1));
    assert_eq!(b, rect(0, 25, 100, 25));
}

#[test]
fn split_rect_ratio_zero_gives_remainder_to_second() {
    // ratio 0.0 → first gets nothing; second gets everything but the seam.
    let (a, seam, b) = split_rect(rect(0, 0, 100, 50), false, 0.0, true);
    assert_eq!(a.width, 0);
    assert_eq!(seam.width, 1);
    assert_eq!(b.width, 99);
}

#[test]
fn split_rect_ratio_one_gives_remainder_to_first() {
    // ratio 1.0 → first gets everything but the seam; second gets nothing.
    let (a, seam, b) = split_rect(rect(0, 0, 100, 50), false, 1.0, true);
    assert_eq!(a.width, 99);
    assert_eq!(seam.width, 1);
    assert_eq!(b.width, 0);
}

#[test]
fn split_rect_zero_area_no_panic() {
    let (a, seam, b) = split_rect(rect(0, 0, 0, 0), false, 0.5, true);
    assert_eq!(a.width, 0);
    assert_eq!(seam.width, 0);
    assert_eq!(b.width, 0);
}

#[test]
fn split_rect_children_and_seam_tile_parent() {
    let area = rect(10, 5, 100, 40);
    let (a, seam, b) = split_rect(area, false, 0.3, true);
    assert_eq!(a.x, area.x);
    assert_eq!(seam.x, a.x + a.width);
    assert_eq!(b.x, seam.x + seam.width);
    assert_eq!(a.width + seam.width + b.width, area.width);
    assert_eq!(a.height, area.height);
    assert_eq!(seam.height, area.height);
    assert_eq!(b.height, area.height);
}

#[test]
fn split_rect_no_reserve_seam_tiles_edge_to_edge() {
    // pane-dividers=false: no seam reserved, children tile the parent exactly.
    let area = rect(10, 5, 100, 40);
    let (a, seam, b) = split_rect(area, false, 0.3, false);
    assert_eq!(seam.width, 0);
    assert_eq!(a.x, area.x);
    assert_eq!(b.x, a.x + a.width);
    assert_eq!(a.width + b.width, area.width);
}

// ── LayoutTree ───────────────────────────────────────────────────────

#[test]
fn layout_tree_leaf_returns_single_rect() {
    let tree = LayoutTree::Leaf(PaneId::default());
    let mut out = Vec::new();
    tree.collect_rects_into(rect(0, 0, 80, 24), true, &mut out);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].1, rect(0, 0, 80, 24));
}

#[test]
fn layout_tree_horizontal_split() {
    let id_a = PaneId::default();
    let id_b = PaneId::default();
    let tree = LayoutTree::Split {
        direction: Direction::Horizontal,
        ratio: 0.5,
        children: Box::new((LayoutTree::Leaf(id_a), LayoutTree::Leaf(id_b))),
    };
    let mut out = Vec::new();
    tree.collect_rects_into(rect(0, 0, 100, 50), true, &mut out);
    assert_eq!(out.len(), 2);
    // Seam eats 1 column from the first child; the second child's
    // geometry is unaffected (see split_rect_horizontal_half).
    assert_eq!(out[0].1.width, 49);
    assert_eq!(out[1].1.x, 50);
    assert_eq!(out[1].1.width, 50);
}

#[test]
fn layout_tree_vertical_split() {
    let tree = LayoutTree::Split {
        direction: Direction::Vertical,
        ratio: 0.5,
        children: Box::new((
            LayoutTree::Leaf(PaneId::default()),
            LayoutTree::Leaf(PaneId::default()),
        )),
    };
    let mut out = Vec::new();
    tree.collect_rects_into(rect(0, 0, 100, 50), true, &mut out);
    assert_eq!(out.len(), 2);
    // Seam eats 1 row from the first child; the second child's geometry
    // is unaffected (see split_rect_vertical_half).
    assert_eq!(out[0].1.height, 24);
    assert_eq!(out[1].1.y, 25);
    assert_eq!(out[1].1.height, 25);
}

#[test]
fn layout_tree_no_reserve_seam_children_tile_edge_to_edge() {
    // pane-dividers=false: no seam between the two panes.
    let tree = LayoutTree::Split {
        direction: Direction::Horizontal,
        ratio: 0.5,
        children: Box::new((
            LayoutTree::Leaf(PaneId::default()),
            LayoutTree::Leaf(PaneId::default()),
        )),
    };
    let mut out = Vec::new();
    tree.collect_rects_into(rect(0, 0, 100, 50), false, &mut out);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].1.width + out[1].1.width, 100);
    assert_eq!(out[1].1.x, out[0].1.width);
}

#[test]
fn layout_tree_collect_appends_without_clearing() {
    let tree = LayoutTree::Leaf(PaneId::default());
    let mut out = vec![(PaneId::default(), rect(99, 99, 1, 1))]; // pre-existing entry
    tree.collect_rects_into(rect(0, 0, 80, 24), true, &mut out);
    assert_eq!(out.len(), 2); // appended, not replaced
}

// ── Seams ────────────────────────────────────────────────────────────

#[test]
fn collect_seams_leaf_has_no_seams() {
    let tree = LayoutTree::Leaf(PaneId::default());
    let mut out = Vec::new();
    tree.collect_seams_into(rect(0, 0, 80, 24), &mut out);
    assert!(out.is_empty());
}

#[test]
fn collect_seams_one_split_yields_one_seam() {
    let tree = LayoutTree::Split {
        direction: Direction::Horizontal,
        ratio: 0.5,
        children: Box::new((
            LayoutTree::Leaf(PaneId::default()),
            LayoutTree::Leaf(PaneId::default()),
        )),
    };
    let mut out = Vec::new();
    tree.collect_seams_into(rect(0, 0, 100, 50), &mut out);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].rect, rect(49, 0, 1, 50));
    assert_eq!(out[0].direction, Direction::Horizontal);
}

#[test]
fn collect_seams_nested_splits_yield_one_seam_per_split_node() {
    let [a, b, c] = pane_ids();
    let tree = LayoutTree::Split {
        direction: Direction::Horizontal,
        ratio: 0.5,
        children: Box::new((
            LayoutTree::Leaf(a),
            LayoutTree::Split {
                direction: Direction::Vertical,
                ratio: 0.5,
                children: Box::new((LayoutTree::Leaf(b), LayoutTree::Leaf(c))),
            },
        )),
    };
    let mut out = Vec::new();
    tree.collect_seams_into(rect(0, 0, 100, 100), &mut out);
    assert_eq!(out.len(), 2, "one seam per split node — root + nested");
}

#[test]
fn focused_seam_segment_full_overlap_horizontal_adjacency() {
    // Two panes side by side with a 1-col seam between them at x=49.
    // Each pane spans the seam's full height, so the highlighted segment
    // is the whole seam.
    let seam = rect(49, 0, 1, 50);
    let left_pane = rect(0, 0, 49, 50);
    let right_pane = rect(50, 0, 50, 50);
    assert_eq!(focused_seam_segment(seam, left_pane), Some(seam));
    assert_eq!(focused_seam_segment(seam, right_pane), Some(seam));
}

#[test]
fn focused_seam_segment_full_overlap_vertical_adjacency() {
    // Two panes stacked with a 1-row seam between them at y=24. Each
    // pane spans the seam's full width, so the highlighted segment is
    // the whole seam.
    let seam = rect(0, 24, 100, 1);
    let top_pane = rect(0, 0, 100, 24);
    let bottom_pane = rect(0, 25, 100, 25);
    assert_eq!(focused_seam_segment(seam, top_pane), Some(seam));
    assert_eq!(focused_seam_segment(seam, bottom_pane), Some(seam));
}

#[test]
fn focused_seam_segment_none_for_non_adjacent_pane() {
    // A 3-pane row: a | seam | b | seam | c. The first seam does not
    // touch the third pane.
    let first_seam = rect(32, 0, 1, 50);
    let pane_c = rect(66, 0, 34, 50);
    assert_eq!(focused_seam_segment(first_seam, pane_c), None);
}

#[test]
fn focused_seam_segment_partial_for_shared_seam() {
    // A over B|C: A spans the full width; the horizontal seam below A
    // is shared by B (left half) and C (right half). Focusing B or C
    // should only highlight the half of the seam above that pane, not
    // the whole seam — this is the bug this function fixes.
    let seam = rect(0, 24, 100, 1);
    let pane_b = rect(0, 25, 50, 25);
    let pane_c = rect(50, 25, 50, 25);
    assert_eq!(
        focused_seam_segment(seam, pane_b),
        Some(rect(0, 24, 50, 1)),
        "focusing B highlights only the left half of the shared seam"
    );
    assert_eq!(
        focused_seam_segment(seam, pane_c),
        Some(rect(50, 24, 50, 1)),
        "focusing C highlights only the right half of the shared seam"
    );
}

#[test]
fn focused_seam_segment_full_for_full_width_pane_above_shared_seam() {
    // Same seam as above, but focusing A (the full-width pane on the
    // other side) still highlights the entire seam.
    let seam = rect(0, 24, 100, 1);
    let pane_a = rect(0, 0, 100, 24);
    assert_eq!(focused_seam_segment(seam, pane_a), Some(seam));
}

// ── focused_pane_corners ─────────────────────────────────────────────

#[test]
fn focused_pane_corners_interior_pane_yields_all_four() {
    // A pane at (5, 5, 10, 10) has its corners one cell diagonally outside
    // each corner: top-left (4,4), top-right (15,4), bottom-left (4,15),
    // bottom-right (15,15). Expected values derived from the geometry
    // (pane.x-1, pane.y-1) etc., independent of the helper.
    let pane = rect(5, 5, 10, 10);
    let corners = focused_pane_corners(pane);
    assert_eq!(corners[0], Some((4, 4)), "top-left");
    assert_eq!(corners[1], Some((15, 4)), "top-right");
    assert_eq!(corners[2], Some((4, 15)), "bottom-left");
    assert_eq!(corners[3], Some((15, 15)), "bottom-right");
}

#[test]
fn focused_pane_corners_at_screen_origin_drops_origin_corners() {
    // A pane flush with the top-left screen edge has no seam above its
    // top edge or to the left of its left edge (the screen edge carries
    // no seam), so every corner touching one of those edges is `None`;
    // only the bottom-right corner — bounded by seams on both sides —
    // survives.
    let pane = rect(0, 0, 10, 10);
    let corners = focused_pane_corners(pane);
    assert_eq!(corners[0], None, "top-left off-origin on both axes");
    assert_eq!(corners[1], None, "top-right off-origin on y");
    assert_eq!(corners[2], None, "bottom-left off-origin on x");
    assert_eq!(corners[3], Some((10, 10)), "bottom-right");
}

/// `PaneId::default()` is the slotmap null key — every default is equal,
/// so tests that assert on distinct ids mint real ones off a throwaway map.
fn pane_ids<const N: usize>() -> [PaneId; N] {
    let mut sm: SlotMap<PaneId, ()> = SlotMap::with_key();
    std::array::from_fn(|_| sm.insert(()))
}

// ── junction_glyph ───────────────────────────────────────────────────

#[test]
fn junction_glyph_resolves_every_reachable_mask() {
    // Expected glyphs derived by hand from the compass-bit meaning of
    // each mask, independent of `junction_glyph`'s own match arms.
    assert_eq!(junction_glyph(ARM_N | ARM_S), "│", "vertical line");
    assert_eq!(junction_glyph(ARM_E | ARM_W), "─", "horizontal line");
    assert_eq!(
        junction_glyph(ARM_E | ARM_S | ARM_W),
        "┬",
        "horizontal line with a downward stem"
    );
    assert_eq!(
        junction_glyph(ARM_N | ARM_E | ARM_W),
        "┴",
        "horizontal line with an upward stem"
    );
    assert_eq!(
        junction_glyph(ARM_N | ARM_E | ARM_S),
        "├",
        "vertical line with a rightward stem"
    );
    assert_eq!(
        junction_glyph(ARM_N | ARM_S | ARM_W),
        "┤",
        "vertical line with a leftward stem"
    );
    assert_eq!(
        junction_glyph(ARM_N | ARM_E | ARM_S | ARM_W),
        "┼",
        "full cross"
    );
}

// ── collect_seam_arms ────────────────────────────────────────────────

#[test]
fn collect_seam_arms_t_junction() {
    // A over (B|C): the seam below A meets the seam between B and C in a
    // T. Root split is vertical (height split, ratio 0.5) over a 100x100
    // area, landing the horizontal seam at row 49 (see
    // split_rect_vertical_half); the nested horizontal split of the
    // 100x50 bottom half lands its vertical seam at column 49 (see
    // split_rect_horizontal_half applied to a 100-wide area).
    let [a, b, c] = pane_ids();
    let tree = LayoutTree::Split {
        direction: Direction::Vertical,
        ratio: 0.5,
        children: Box::new((
            LayoutTree::Leaf(a),
            LayoutTree::Split {
                direction: Direction::Horizontal,
                ratio: 0.5,
                children: Box::new((LayoutTree::Leaf(b), LayoutTree::Leaf(c))),
            },
        )),
    };
    let mut seams = Vec::new();
    tree.collect_seams_into(rect(0, 0, 100, 100), &mut seams);

    let mut arms = HashMap::new();
    collect_seam_arms(&seams, &mut arms);

    // The B|C seam starts one row below the A|BC seam, so it contributes
    // a southward arm to the cell where it meets the horizontal line.
    assert_eq!(arms.get(&(49, 49)), Some(&ARM_S));
}

#[test]
fn collect_seam_arms_cross_junction() {
    // (A|D) over (B|C), both rows split at the same ratio so their
    // vertical seams land in the same column (49) — the horizontal seam
    // between the rows is sandwiched by both, producing a full cross.
    let [a, b, c, d] = pane_ids();
    let tree = LayoutTree::Split {
        direction: Direction::Vertical,
        ratio: 0.5,
        children: Box::new((
            LayoutTree::Split {
                direction: Direction::Horizontal,
                ratio: 0.5,
                children: Box::new((LayoutTree::Leaf(a), LayoutTree::Leaf(d))),
            },
            LayoutTree::Split {
                direction: Direction::Horizontal,
                ratio: 0.5,
                children: Box::new((LayoutTree::Leaf(b), LayoutTree::Leaf(c))),
            },
        )),
    };
    let mut seams = Vec::new();
    tree.collect_seams_into(rect(0, 0, 100, 100), &mut seams);

    let mut arms = HashMap::new();
    collect_seam_arms(&seams, &mut arms);

    // The top row's seam ends just above the crossing (northward arm);
    // the bottom row's seam starts just below it (southward arm).
    assert_eq!(arms.get(&(49, 49)), Some(&(ARM_N | ARM_S)));
}

#[test]
fn junction_glyph_at_t_and_cross_scenarios_matches_collect_seam_arms() {
    // Integration check bridging the two unit-tested pieces: feed real
    // arms-map output through `junction_glyph` and confirm the resolved
    // glyphs match what a human reading the layout would expect.
    let [a, b, c] = pane_ids();
    let t_tree = LayoutTree::Split {
        direction: Direction::Vertical,
        ratio: 0.5,
        children: Box::new((
            LayoutTree::Leaf(a),
            LayoutTree::Split {
                direction: Direction::Horizontal,
                ratio: 0.5,
                children: Box::new((LayoutTree::Leaf(b), LayoutTree::Leaf(c))),
            },
        )),
    };
    let mut seams = Vec::new();
    t_tree.collect_seams_into(rect(0, 0, 100, 100), &mut seams);
    let mut arms = HashMap::new();
    collect_seam_arms(&seams, &mut arms);
    // The A|BC seam is horizontal (Direction::Vertical), base E|W.
    let base = ARM_E | ARM_W;
    let mask = base | arms.get(&(49, 49)).copied().unwrap_or(0);
    assert_eq!(junction_glyph(mask), "┬");
}

#[test]
fn split_leaf_on_root() {
    let [a, b] = pane_ids();
    let mut tree = LayoutTree::Leaf(a);
    assert!(tree.split_leaf(a, b, Direction::Vertical, 0.5));
    assert_eq!(
        tree,
        LayoutTree::Split {
            direction: Direction::Vertical,
            ratio: 0.5,
            children: Box::new((LayoutTree::Leaf(a), LayoutTree::Leaf(b))),
        }
    );
}

#[test]
fn split_leaf_missing_target_is_noop() {
    let [a, b, missing] = pane_ids();
    let mut tree = LayoutTree::Leaf(a);
    assert!(!tree.split_leaf(missing, b, Direction::Vertical, 0.5));
    assert_eq!(tree, LayoutTree::Leaf(a));
}

#[test]
fn split_leaf_on_nested_target() {
    let [a, b, c] = pane_ids();
    let mut tree = LayoutTree::Split {
        direction: Direction::Horizontal,
        ratio: 0.5,
        children: Box::new((LayoutTree::Leaf(a), LayoutTree::Leaf(b))),
    };
    assert!(tree.split_leaf(b, c, Direction::Vertical, 0.5));
    assert_eq!(
        tree,
        LayoutTree::Split {
            direction: Direction::Horizontal,
            ratio: 0.5,
            children: Box::new((
                LayoutTree::Leaf(a),
                LayoutTree::Split {
                    direction: Direction::Vertical,
                    ratio: 0.5,
                    children: Box::new((LayoutTree::Leaf(b), LayoutTree::Leaf(c))),
                },
            )),
        }
    );
    let mut out = Vec::new();
    tree.collect_rects_into(rect(0, 0, 100, 100), true, &mut out);
    assert_eq!(out.len(), 3);
}

#[test]
fn remove_leaf_collapses_parent() {
    let [a, b] = pane_ids();
    let mut tree = LayoutTree::Split {
        direction: Direction::Horizontal,
        ratio: 0.5,
        children: Box::new((LayoutTree::Leaf(a), LayoutTree::Leaf(b))),
    };
    assert_eq!(tree.remove_leaf(a), Some(b));
    assert_eq!(tree, LayoutTree::Leaf(b));
}

#[test]
fn remove_leaf_promotes_subtree_and_returns_its_leftmost_leaf() {
    let [a, b, c] = pane_ids();
    let mut tree = LayoutTree::Split {
        direction: Direction::Horizontal,
        ratio: 0.5,
        children: Box::new((
            LayoutTree::Leaf(a),
            LayoutTree::Split {
                direction: Direction::Vertical,
                ratio: 0.5,
                children: Box::new((LayoutTree::Leaf(b), LayoutTree::Leaf(c))),
            },
        )),
    };
    assert_eq!(tree.remove_leaf(a), Some(b));
    assert_eq!(
        tree,
        LayoutTree::Split {
            direction: Direction::Vertical,
            ratio: 0.5,
            children: Box::new((LayoutTree::Leaf(b), LayoutTree::Leaf(c))),
        }
    );
}

#[test]
fn remove_leaf_sole_leaf_returns_none() {
    let [a] = pane_ids();
    let mut tree = LayoutTree::Leaf(a);
    assert_eq!(tree.remove_leaf(a), None);
    assert_eq!(tree, LayoutTree::Leaf(a));
}

#[test]
fn remove_leaf_missing_target_is_noop() {
    let [a, b, missing] = pane_ids();
    let mut tree = LayoutTree::Split {
        direction: Direction::Horizontal,
        ratio: 0.5,
        children: Box::new((LayoutTree::Leaf(a), LayoutTree::Leaf(b))),
    };
    let before = tree.clone();
    assert_eq!(tree.remove_leaf(missing), None);
    assert_eq!(tree, before);
}

// ── Drawer band partition (U6) ───────────────────────────────────────

/// A drawer that always reports a fixed height, regardless of `max` — lets
/// tests probe `pane_area`'s chrome arithmetic without a real `DrawerModel`.
struct FixedHeightDrawer(u16);

impl crate::providers::DrawerProvider for FixedHeightDrawer {
    fn height(&self, max: u16) -> u16 {
        self.0.min(max)
    }

    fn render(&self, _area: Rect, _theme: &Theme, _buf: &mut ratatui::buffer::Buffer) {}
}

/// A no-op tab bar — only its `is_some()` presence matters to `pane_area`.
struct NoopTabBar;

impl crate::providers::TabBarProvider for NoopTabBar {
    fn render(&self, _area: Rect, _theme: &Theme, _buf: &mut ratatui::buffer::Buffer) {}
}

#[test]
fn pane_area_reserves_drawer_height_above_statusline() {
    let mut view = EngineView::new(Theme::default());
    view.drawer = Some(Box::new(FixedHeightDrawer(3)));

    let area = view.pane_area(rect(0, 0, 40, 20));

    // 20 rows total - 1 (statusline) - 3 (drawer) = 16 rows for panes,
    // starting at the top (no tab bar registered).
    assert_eq!(area.y, 0);
    assert_eq!(area.height, 16);
}

#[test]
fn pane_area_folds_tabbar_and_drawer_together() {
    let mut view = EngineView::new(Theme::default());
    view.tabbar = Some(Box::new(NoopTabBar));
    view.drawer = Some(Box::new(FixedHeightDrawer(3)));

    let area = view.pane_area(rect(0, 0, 40, 20));

    // 20 - 1 (tab bar) - 1 (statusline) - 3 (drawer) = 15, offset by the
    // tab bar's 1 row.
    assert_eq!(area.y, 1);
    assert_eq!(area.height, 15);
}

#[test]
fn pane_area_drawer_height_is_capped_by_half_the_terminal_height() {
    let mut view = EngineView::new(Theme::default());
    // Wants 50 rows — way more than half of a 20-row terminal (max = 10).
    view.drawer = Some(Box::new(FixedHeightDrawer(50)));

    let area = view.pane_area(rect(0, 0, 40, 20));

    // 20 - 1 (statusline) - 10 (capped drawer) = 9.
    assert_eq!(area.height, 9);
}

#[test]
fn pane_area_degenerate_when_terminal_too_small_for_chrome_plus_drawer() {
    let mut view = EngineView::new(Theme::default());
    view.drawer = Some(Box::new(FixedHeightDrawer(3)));

    // chrome_height = 1 (statusline) + 3 (drawer, capped at height/2=1) = 2,
    // which is NOT less than a 2-row terminal — degenerate.
    let area = view.pane_area(rect(0, 0, 40, 2));

    assert_eq!(area.height, 0);
}

// ── FrameScratch ─────────────────────────────────────────────────────

#[test]
fn frame_scratch_clear_retains_capacity() {
    let mut s = FrameScratch::new();
    for _ in 0..100 {
        s.format.graphemes.push(crate::types::Grapheme {
            byte_range: 0..1,
            char_offset: 0,
            col: 0,
            width: 1,
            content: crate::types::CellContent::Empty,
            indent_depth: 0,
            scope: None,
        });
    }
    let cap_before = s.format.graphemes.capacity();
    s.clear();
    assert_eq!(s.format.graphemes.len(), 0);
    assert!(s.format.graphemes.capacity() >= cap_before);
}
