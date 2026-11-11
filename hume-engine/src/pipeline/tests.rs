use hume_grid::{Grid, Position, Rect, Rgb};
use std::collections::HashMap;

use crate::test_doubles::VirtualLineBlock;
use crate::test_support::{fg, theme_with};
use hume_rope::spanned::SpannedText;

use super::*;

use crate::display_lines::DisplayLinePos;
use crate::pane::{WhitespaceConfig, WrapMode};
use crate::providers::{Decoration, DecorationKinds, DecorationSource, VirtualLine};
use crate::types::{DisplayLineKind, ResolvedStyle};
use hume_rope::line::{ContentLine, RopeyLine};

fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// Draw `pane`'s view of `rope` into a grid the size of `pane_rect`.
///
/// The render tests below vary only these six things; everything else a
/// `PaneRenderCtx` carries is fixed (no syntax, no dimming, 4-column tabs, no
/// whitespace indicators, indent guides on), so it is spelled once here
/// rather than at each call.
///
/// The line store is built per call and so is always cold. These tests assert
/// on drawn cells, and a store shared between them would make one test's
/// output depend on which ran first. The sharing itself is covered in
/// `display_lines::tests`, against the store directly.
fn render_test_pane(
    pane: &mut Pane,
    rope: &ropey::Rope,
    theme: &Theme,
    pane_rect: Rect,
    wrap_mode: WrapMode,
    default_gutter_scope: ScopeId,
) -> Grid {
    let settings = PaneRenderSettings {
        mode: EditorMode::Normal,
        format: crate::display_lines::line_store::FormatKey {
            buffer_tag: [0; 3],
            wrap_mode,
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
        },
        hidden: crate::folds::HiddenLines::default(),
        indent_guides: true,
        cursor_is_block: true,
        gutter: crate::providers::GutterInputs::default(),
    };
    // The pane's rows as the host's scroll pass settles them: a map laid
    // out as the render pass lays this pane out, over a store of its own.
    let content_width = crate::layout::compute_viewport(
        rope,
        PaneSize::of(pane_rect),
        pane.providers.gutter_columns(),
        settings.gutter,
    )
    .content_width;
    let mut rows_store = crate::display_lines::line_store::PaneLineStore::new();
    let mut rows_dlm = crate::display_lines::DisplayLineMap::new(
        rope,
        &pane.providers,
        content_width,
        settings.format,
        settings.hidden.clone(),
        &mut rows_store,
    );
    let rows = crate::test_support::plain_geo(PaneSize::of(pane_rect), 0)
        .rows(&mut rows_dlm, &mut pane.viewport);
    // The host writes a mirror before every frame; these tests stand in for
    // it with a primary cursor at the text start unless a test wrote its own.
    let selections = pane
        .selections
        .get_or_insert_with(|| crate::test_support::cursor_mirror(rope, 0));
    let mut pane_ctx = PaneRenderCtx {
        viewport: &mut pane.viewport,
        providers: &pane.providers,
        selections,
        rope,
        syntax: None,
        theme,
        rect: pane_rect,
        settings: &settings,
        rows,
        dim: None,
        default_gutter_scope,
    };
    let mut buf = Grid::new(pane_rect.width, pane_rect.height);
    render_pane(
        &mut pane_ctx,
        &mut FrameScratch::new(),
        &mut crate::display_lines::line_store::PaneLineStore::new(),
        &mut buf,
    );
    buf
}

// ── Virtual display line scope styling ──────────────────────────────

/// Emits one `Before(0)` display line whose first grapheme carries `scope`
/// and whose second carries none.
struct ScopedVirtualLine {
    scope: crate::types::ScopeId,
}

impl DecorationSource for ScopedVirtualLine {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }
    fn decorations_for_line(&self, line_idx: ContentLine, out: &mut Vec<Decoration>) {
        if line_idx == ContentLine::new(0) {
            out.push(Decoration::VirtualLine(VirtualLine {
                line: ContentLine::new(0),
                side: hume_core::Side::Before,
                provider_id: 0,
                // "H" carries the scope; "~" carries none and must fall
                // back to `ui.virtual_text`.
                content: SpannedText::new("H~".to_string(), vec![(0, 1, self.scope)])
                    .expect("the span fits"),
                base_scope: None,
            }));
        }
    }
}

#[test]
fn virtual_display_line_resolves_grapheme_scope_and_falls_back_to_virtual_text() {
    // Two graphemes: one carries an interned scope (must resolve to that
    // scope's fg), one carries no scope (must fall back to ui.virtual_text).
    let mut registry = ScopeRegistry::new();
    let hint_scope = registry.intern("hint");
    let mut theme = theme_with([
        ("hint", fg(Rgb(255, 0, 0))),
        ("ui.virtual", fg(Rgb(0, 0, 255))),
    ]);
    theme.bake(&registry);

    let rope = ropey::Rope::from_str("z\n");
    let [bid] = hume_core::fresh_ids();
    let mut pane = Pane::new(bid);
    pane.providers
        .add_decoration_source(Box::new(ScopedVirtualLine { scope: hint_scope }));

    let pane_rect = rect(0, 0, 20, 5);
    let buf = render_test_pane(
        &mut pane,
        &rope,
        &theme,
        pane_rect,
        WrapMode::None,
        crate::types::ScopeId(0),
    );

    let scoped_cell = buf.cell(0, 0).unwrap();
    assert_eq!(
        scoped_cell.style().fg,
        Some(Rgb(255, 0, 0)),
        "grapheme with Some(scope) resolves that scope's style"
    );
    let fallback_cell = buf.cell(1, 0).unwrap();
    assert_eq!(
        fallback_cell.style().fg,
        Some(Rgb(0, 0, 255)),
        "grapheme with no scope falls back to ui.virtual_text"
    );
}

/// Emits one `Before(0)` display line whose four graphemes each carry a
/// distinct scope, the segments given in the reverse of text order.
/// `format_virtual_lines`'s cursor scan never backs up, so it resolves every
/// grapheme only from segments in text order.
struct UnsortedScopedVirtualLine {
    scopes: [crate::types::ScopeId; 4],
}

impl DecorationSource for UnsortedScopedVirtualLine {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }
    fn decorations_for_line(&self, line_idx: ContentLine, out: &mut Vec<Decoration>) {
        if line_idx == ContentLine::new(0) {
            out.push(Decoration::VirtualLine(VirtualLine {
                line: ContentLine::new(0),
                side: hume_core::Side::Before,
                provider_id: 0,
                content: SpannedText::new(
                    "ABCD".to_string(),
                    vec![
                        (3, 4, self.scopes[3]),
                        (2, 3, self.scopes[2]),
                        (1, 2, self.scopes[1]),
                        (0, 1, self.scopes[0]),
                    ],
                )
                .expect("the spans fit"),
                base_scope: None,
            }));
        }
    }
}

#[test]
fn virtual_display_line_resolves_scopes_from_segments_given_out_of_order() {
    let mut registry = ScopeRegistry::new();
    let scope_names = ["scope-a", "scope-b", "scope-c", "scope-d"];
    let scopes: [crate::types::ScopeId; 4] =
        std::array::from_fn(|i| registry.intern(scope_names[i]));
    let colors = [
        Rgb(255, 0, 0),
        Rgb(0, 255, 0),
        Rgb(255, 255, 0),
        Rgb(255, 0, 255),
    ];
    // Built in a loop, so `theme_with`'s array form doesn't fit, but the
    // per-entry `fg` helper still applies.
    let mut styles_map = HashMap::new();
    for (name, color) in scope_names.iter().zip(colors) {
        styles_map.insert(*name, crate::test_support::fg(color));
    }
    let mut theme = Theme::new(styles_map, ResolvedStyle::default());
    theme.bake(&registry);

    let rope = ropey::Rope::from_str("z\n");
    let [bid] = hume_core::fresh_ids();
    let mut pane = Pane::new(bid);
    pane.providers
        .add_decoration_source(Box::new(UnsortedScopedVirtualLine { scopes }));

    let pane_rect = rect(0, 0, 20, 5);
    let buf = render_test_pane(
        &mut pane,
        &rope,
        &theme,
        pane_rect,
        WrapMode::None,
        crate::types::ScopeId(0),
    );

    for (x, color) in colors.into_iter().enumerate() {
        let cell = buf.cell(x as u16, 0).unwrap();
        assert_eq!(
            cell.style().fg,
            Some(color),
            "grapheme {x} resolves its scope regardless of segment emission order"
        );
    }
}

// ── top_skip vs virtual lines ─────────────────────────────────────────

/// Build a pane viewing a rope whose line 0 is "aaaabbbbcccc". Under
/// `WrapMode::Soft { width: 4 }` this wraps into exactly 3 display lines:
/// "aaaa", "bbbb", "cccc" (each grapheme is 1 column, so Soft splits at the
/// exact column with no backtracking). `top_slot` controls how many of
/// those wrap display lines are already scrolled past.
fn render_wrapped_pane_with_virtual_line(
    top_slot: u16,
    side: hume_core::Side,
    line: ContentLine,
) -> Grid {
    render_wrapped_pane_with_virtual_text(top_slot, side, line, "V")
}

/// [`render_wrapped_pane_with_virtual_line`] with the virtual line's own text.
fn render_wrapped_pane_with_virtual_text(
    top_slot: u16,
    side: hume_core::Side,
    line: ContentLine,
    text: &'static str,
) -> Grid {
    let rope = ropey::Rope::from_str("aaaabbbbcccc\nz\n");
    let [bid] = hume_core::fresh_ids();

    let mut pane = Pane::new(bid);
    pane.viewport
        .seed_top_for_test(DisplayLinePos::new(ContentLine::new(0), top_slot as usize));
    pane.providers
        .add_decoration_source(Box::new(VirtualLineBlock::uniform(side, line, 1, text)));

    let theme = Theme::default();
    let pane_rect = rect(0, 0, 10, 6);
    render_test_pane(
        &mut pane,
        &rope,
        &theme,
        pane_rect,
        WrapMode::Soft { width: 4 },
        crate::types::ScopeId(0),
    )
}

fn cell_symbol(buf: &Grid, x: u16, y: u16) -> String {
    buf.cell(x, y).unwrap().text().to_string()
}

#[test]
fn before_virtual_line_skipped_one_display_line_at_a_time() {
    // Line 0's block under Before(0) is [V, aaaa, bbbb, cccc, <eol>], 5
    // display lines: "aaaabbbbcccc" wraps into three content display lines
    // at width 4, and "cccc" exactly fills the last one, so the trailing
    // '\n's own sentinel wraps onto a display line of its own. `top_slot`
    // walks through the whole
    // block uniformly, the same as it would a plain wrap display line: each
    // unit of offset drops exactly one block display line, virtual or real.
    let offset1 =
        render_wrapped_pane_with_virtual_line(1, hume_core::Side::Before, ContentLine::new(0));
    assert_eq!(
        cell_symbol(&offset1, 0, 0),
        "a",
        "offset 1 skips V only, wrap display line 0 shows"
    );
    assert_eq!(
        cell_symbol(&offset1, 0, 1),
        "b",
        "wrap display line 1 follows"
    );

    let offset2 =
        render_wrapped_pane_with_virtual_line(2, hume_core::Side::Before, ContentLine::new(0));
    assert_eq!(
        cell_symbol(&offset2, 0, 0),
        "b",
        "offset 2 skips V and wrap display line 0"
    );

    let offset3 =
        render_wrapped_pane_with_virtual_line(3, hume_core::Side::Before, ContentLine::new(0));
    assert_eq!(
        cell_symbol(&offset3, 0, 0),
        "c",
        "offset 3 reaches wrap display line 2"
    );

    let offset4 =
        render_wrapped_pane_with_virtual_line(4, hume_core::Side::Before, ContentLine::new(0));
    assert_eq!(
        cell_symbol(&offset4, 0, 0),
        " ",
        "offset 4 reaches the block's last display line, the wrapped eol sentinel"
    );

    // Offset 5 is past the end of a 5-display-line block: not an address in
    // the document, so it clamps to the block's last display line: the same
    // address `Viewport::top_at` resolves to, which is the point (production
    // never reaches this case directly, since the render pass resolves the
    // top itself every frame). Skipping display lines by the raw offset
    // would carry over into line 1 instead.
    let past_end =
        render_wrapped_pane_with_virtual_line(5, hume_core::Side::Before, ContentLine::new(0));
    assert_eq!(
        cell_symbol(&past_end, 0, 0),
        " ",
        "an offset past the block clamps to its last display line, as the clamp does"
    );
}

#[test]
fn before_virtual_line_renders_when_not_skipped() {
    // top_slot=0: the Before(0) virtual line renders at screen row
    // 0, pushing wrapped display line 0 ("aaaa") down to screen row 1.
    let buf =
        render_wrapped_pane_with_virtual_line(0, hume_core::Side::Before, ContentLine::new(0));
    assert_eq!(cell_symbol(&buf, 0, 0), "V", "virtual line at screen row 0");
    assert_eq!(
        cell_symbol(&buf, 0, 1),
        "a",
        "wrap display line 0 pushed to screen row 1"
    );
}

#[test]
fn long_after_virtual_line_wraps_onto_following_rows() {
    // Line 0 occupies rows 0..=3 ("aaaa", "bbbb", "cccc", the sentinel's
    // wrapped row). The 9-column After(0) text wraps at width 4 into
    // "VVVV" / "VVVV" / "V" on rows 4..=6, then line 1 follows.
    let buf = render_wrapped_pane_with_virtual_text(
        0,
        hume_core::Side::After,
        ContentLine::new(0),
        "VVVVVVVVV",
    );
    let row = |y: u16| (0..5).map(|x| cell_symbol(&buf, x, y)).collect::<String>();
    assert_eq!(row(4), "VVVV ");
    assert_eq!(row(5), "VVVV ");
}

#[test]
fn after_virtual_line_renders_below_skipped_display_lines() {
    // top_slot=1 skips wrap display line 0. The After(0) virtual line sits
    // below all of line 0's wrap display lines (two more content display
    // lines plus the trailing '\n's own wrapped sentinel display line), none
    // of which are skipped (the budget is exhausted by wrap display line 0
    // alone). It must still render, after wrap display lines 1, 2, and the
    // sentinel.
    let buf = render_wrapped_pane_with_virtual_line(1, hume_core::Side::After, ContentLine::new(0));
    assert_eq!(cell_symbol(&buf, 0, 0), "b", "wrap display line 1");
    assert_eq!(cell_symbol(&buf, 0, 1), "c", "wrap display line 2");
    assert_eq!(
        cell_symbol(&buf, 0, 2),
        " ",
        "the trailing newline's wrapped sentinel row"
    );
    assert_eq!(
        cell_symbol(&buf, 0, 3),
        "V",
        "After(0) virtual line still renders"
    );
}

/// Line 0 is "x" (one unwrapped display line) with `n` `Before(0)` virtual
/// display lines stacked above it, viewed through a viewport `height` rows
/// tall.
fn render_pane_with_n_before_lines(top_slot: u16, n: usize, height: u16) -> Grid {
    let rope = ropey::Rope::from_str("x\ny\n");
    let [bid] = hume_core::fresh_ids();

    let mut pane = Pane::new(bid);
    pane.viewport
        .seed_top_for_test(DisplayLinePos::new(ContentLine::new(0), top_slot as usize));
    pane.providers
        .add_decoration_source(Box::new(VirtualLineBlock::numbered(
            hume_core::Side::Before,
            ContentLine::new(0),
            n,
        )));

    let theme = Theme::default();
    let pane_rect = rect(0, 0, 10, height);
    render_test_pane(
        &mut pane,
        &rope,
        &theme,
        pane_rect,
        WrapMode::Soft { width: 10 },
        crate::types::ScopeId(0),
    )
}

#[test]
fn virtual_before_block_taller_than_viewport_exposes_every_display_line() {
    // Line 0's block is [1, 2, 3, x] (4 display lines) in a viewport only
    // 2 rows tall, so the block can never be shown in full. Every display
    // line must still be individually reachable: walking `top_slot` through
    // the block one unit at a time surfaces each display line at the top of
    // the viewport in turn,
    // with none permanently stuck outside it.
    let expected = ["1", "2", "3", "x"];
    for (offset, expect) in expected.iter().enumerate() {
        let buf = render_pane_with_n_before_lines(offset as u16, 3, 2);
        assert_eq!(
            cell_symbol(&buf, 0, 0),
            *expect,
            "offset {offset}: block display line {offset} must be the top visible row"
        );
    }
}

// ── Provider id stamping ─────────────────────────────────────────────

/// Reports a wrong `provider_id`; the pipeline must not
/// trust it.
struct SpoofingVirtualLineSource {
    side: hume_core::Side,
    line: ContentLine,
}

impl DecorationSource for SpoofingVirtualLineSource {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }
    fn decorations_for_line(&self, line_idx: ContentLine, out: &mut Vec<Decoration>) {
        if line_idx == self.line {
            out.push(Decoration::VirtualLine(VirtualLine {
                line: self.line,
                side: self.side,
                provider_id: 9999, // spoofed, must be overwritten by the pipeline
                content: SpannedText::plain("V".to_string()),
                base_scope: None,
            }));
        }
    }
}

/// Renders the `DisplayLineKind::Virtual` provider_id as gutter text, so the test
/// can observe what actually reached `compose` after collection.
struct ProviderIdReportingGutter;

impl crate::providers::GutterColumn for ProviderIdReportingGutter {
    fn width(&self, _: RopeyLine, _: crate::providers::GutterInputs) -> u8 {
        5
    }
    fn render_cells(
        &self,
        kind: DisplayLineKind,
        _: &crate::providers::GutterCtx,
    ) -> Vec<crate::providers::GutterCell> {
        let cell = match kind {
            DisplayLineKind::Virtual { provider_id, .. } => crate::providers::GutterCell {
                content: crate::providers::GutterCellContent::Text(std::borrow::Cow::Owned(
                    provider_id.to_string(),
                )),
                scope: crate::types::ScopeId(0),
            },
            _ => crate::providers::GutterCell::blank(crate::types::ScopeId(0)),
        };
        vec![cell]
    }
}

#[test]
fn virtual_line_provider_id_is_stamped_by_pipeline_not_self_reported() {
    let rope = ropey::Rope::from_str("a\n");
    let [bid] = hume_core::fresh_ids();
    let mut pane = Pane::new(bid);
    pane.providers
        .add_gutter_column(Box::new(ProviderIdReportingGutter));
    let real_id = pane
        .providers
        .add_decoration_source(Box::new(SpoofingVirtualLineSource {
            side: hume_core::Side::Before,
            line: ContentLine::new(0),
        }));

    let mut registry = crate::theme::ScopeRegistry::new();
    let default_gutter_scope = registry.intern("ui.linenr");
    let mut theme = Theme::default();
    theme.bake(&registry);
    let pane_rect = rect(0, 0, 10, 3);
    let buf = render_test_pane(
        &mut pane,
        &rope,
        &theme,
        pane_rect,
        WrapMode::None,
        default_gutter_scope,
    );

    // Gutter width 5 -> usable 4 columns; a small real_id fits comfortably.
    let gutter_text: String = (0..4).map(|x| cell_symbol(&buf, x, 0)).collect();
    assert_eq!(
        gutter_text.trim(),
        real_id.to_string(),
        "gutter must show the pipeline-stamped id ({real_id}), not the spoofed 9999"
    );
}

// ── CJK display-line estimate feeds the viewport ───────────────────────

#[test]
fn cjk_heavy_viewport_fills_every_row_no_premature_filler() {
    // Two lines of 20 '中' chars each (true width 40 per line). At
    // WrapMode::Soft { width: 20 } each line wraps into two content display
    // lines of exactly 20 columns, and since the second display line
    // exactly fills the wrap width, the trailing '\n's own sentinel wraps
    // onto a display line of its own rather than landing past the pane's
    // edge. Line 0 therefore supplies 3 display lines (content, content,
    // blank sentinel) before line 1 begins, so a 4-row viewport shows those
    // three plus line 1's own first display line, with nothing left over
    // for tilde fillers.
    let line: String = "中".repeat(20);
    let rope = ropey::Rope::from_str(&format!("{line}\n{line}\n"));
    let [bid] = hume_core::fresh_ids();

    let mut pane = Pane::new(bid);

    let theme = Theme::default();
    let pane_rect = rect(0, 0, 20, 4);
    let buf = render_test_pane(
        &mut pane,
        &rope,
        &theme,
        pane_rect,
        WrapMode::Soft { width: 20 },
        crate::types::ScopeId(0),
    );

    let expected = ["中", "中", " ", "中"];
    for y in 0..4u16 {
        let sym = cell_symbol(&buf, 0, y);
        assert_eq!(
            sym, expected[y as usize],
            "row {y} must be real content (a CJK glyph or the wrapped sentinel), not a tilde filler"
        );
    }
}

#[test]
fn scrolled_pane_renders_from_top_line_onward() {
    // "ab\ncd\n": scrolling to top_line=1 must show "cd" at screen row 0.
    // The fused pipeline's scroll mechanism in `WrapMode::None` is
    // `viewport.top_line`, not `top_slot` (which only sub-scrolls
    // within a wrapped `top_line`'s own display lines).
    let rope = ropey::Rope::from_str("ab\ncd\n");
    let [bid] = hume_core::fresh_ids();

    let mut pane = Pane::new(bid);
    pane.viewport
        .seed_top_for_test(DisplayLinePos::new(ContentLine::new(1), 0));

    let theme = Theme::default();
    let pane_rect = rect(0, 0, 20, 5);
    let buf = render_test_pane(
        &mut pane,
        &rope,
        &theme,
        pane_rect,
        WrapMode::None,
        crate::types::ScopeId(0),
    );

    assert_eq!(cell_symbol(&buf, 0, 0), "c");
    assert_eq!(cell_symbol(&buf, 1, 0), "d");
}

#[test]
fn filler_display_line_gutter_shows_gutter_content_not_stale_blank() {
    // Filler display lines past EOF must still get their gutter column
    // consulted, the same as compose_display_line's gutter loop does for
    // real display lines. Otherwise a filler display line's gutter area
    // would be silently blank regardless of what a custom GutterColumn
    // renders for DisplayLineKind::Filler.
    struct MarkerGutter;
    impl crate::providers::GutterColumn for MarkerGutter {
        fn width(&self, _: RopeyLine, _: crate::providers::GutterInputs) -> u8 {
            3
        }
        fn render_cells(
            &self,
            kind: DisplayLineKind,
            _: &crate::providers::GutterCtx,
        ) -> Vec<crate::providers::GutterCell> {
            let text = if matches!(kind, DisplayLineKind::Filler) {
                "~g"
            } else {
                "ln"
            };
            vec![crate::providers::GutterCell {
                content: crate::providers::GutterCellContent::Text(std::borrow::Cow::Borrowed(
                    text,
                )),
                scope: crate::types::ScopeId(0),
            }]
        }
    }

    let rope = ropey::Rope::from_str("x\n");
    let [bid] = hume_core::fresh_ids();

    let mut pane = Pane::new(bid);
    pane.providers.add_gutter_column(Box::new(MarkerGutter));

    let mut registry = crate::theme::ScopeRegistry::new();
    let default_gutter_scope = registry.intern("ui.linenr");
    let mut theme = Theme::default();
    theme.bake(&registry);
    let pane_rect = rect(0, 0, 20, 3);
    let buf = render_test_pane(
        &mut pane,
        &rope,
        &theme,
        pane_rect,
        WrapMode::None,
        default_gutter_scope,
    );

    // Display line 1 is a Filler display line (past the single real line):
    // its gutter must show the column's own Filler rendering ("~g"), not
    // blank.
    assert_eq!(cell_symbol(&buf, 0, 1), "~");
    assert_eq!(cell_symbol(&buf, 1, 1), "g");
}

// ── LayoutTree ───────────────────────────────────────────────────────

fn width_split(members: Vec<LayoutTree>) -> LayoutTree {
    LayoutTree::split(Direction::Horizontal, members)
}

fn height_split(members: Vec<LayoutTree>) -> LayoutTree {
    LayoutTree::split(Direction::Vertical, members)
}

fn width_split_of_three(a: PaneId, b: PaneId, c: PaneId) -> LayoutTree {
    width_split(vec![
        LayoutTree::Leaf(a),
        LayoutTree::Leaf(b),
        LayoutTree::Leaf(c),
    ])
}

fn width_split_of_four(a: PaneId, b: PaneId, c: PaneId, d: PaneId) -> LayoutTree {
    width_split(vec![
        LayoutTree::Leaf(a),
        LayoutTree::Leaf(b),
        LayoutTree::Leaf(c),
        LayoutTree::Leaf(d),
    ])
}

/// `top` stacked above `bottom`.
fn height_split_of_two(top: PaneId, bottom: PaneId) -> LayoutTree {
    height_split(vec![LayoutTree::Leaf(top), LayoutTree::Leaf(bottom)])
}

/// Every leaf's rect in `area`, with dividers.
fn rects(tree: &LayoutTree, area: Rect) -> Vec<(PaneId, Rect)> {
    let mut out = Vec::new();
    tree.collect_rects_into(area, true, &mut out);
    out
}

fn rect_of(tree: &LayoutTree, pid: PaneId, area: Rect) -> Rect {
    tree.find_rect(pid, area, true).expect("a leaf of the tree")
}

#[test]
fn layout_tree_leaf_returns_single_rect() {
    let tree = LayoutTree::Leaf(PaneId::default());
    assert_eq!(
        rects(&tree, rect(0, 0, 80, 24)),
        vec![(PaneId::default(), rect(0, 0, 80, 24))]
    );
}

#[test]
fn an_equal_pair_gives_the_seam_and_the_odd_cell_after_the_first_half() {
    // 100 columns: 1 for the seam, 99 left, the first half rounds down.
    let [a, b] = hume_core::fresh_ids();
    let tree = width_split(vec![LayoutTree::Leaf(a), LayoutTree::Leaf(b)]);
    let area = rect(0, 0, 100, 50);
    assert_eq!(
        rects(&tree, area),
        vec![(a, rect(0, 0, 49, 50)), (b, rect(50, 0, 50, 50))]
    );
    let mut seams = Vec::new();
    tree.collect_seams_into(area, &mut seams);
    assert_eq!(seams.len(), 1);
    assert_eq!(seams[0].rect, rect(49, 0, 1, 50));
    assert_eq!(seams[0].direction(), Direction::Horizontal);
}

#[test]
fn an_equal_height_split_divides_the_height() {
    let [a, b] = hume_core::fresh_ids();
    assert_eq!(
        rects(&height_split_of_two(a, b), rect(0, 0, 100, 50)),
        vec![(a, rect(0, 0, 100, 24)), (b, rect(0, 25, 100, 25))]
    );
}

#[test]
fn without_dividers_members_tile_edge_to_edge() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut out = Vec::new();
    width_split_of_three(a, b, c).collect_rects_into(rect(10, 5, 100, 40), false, &mut out);
    assert_eq!(
        out,
        vec![
            (a, rect(10, 5, 33, 40)),
            (b, rect(43, 5, 33, 40)),
            (c, rect(76, 5, 34, 40)),
        ]
    );
}

#[test]
fn an_equal_width_split_and_its_seams_tile_the_area_within_a_cell_of_each_other() {
    let [a, b, c, d] = hume_core::fresh_ids();
    let tree = width_split_of_four(a, b, c, d);
    let area = rect(3, 0, 101, 10);
    let panes = rects(&tree, area);
    let mut seams = Vec::new();
    tree.collect_seams_into(area, &mut seams);

    let mut x = area.x;
    for (k, (_, pane)) in panes.iter().enumerate() {
        assert_eq!(pane.x, x, "member {k} starts where the last cell ended");
        x += pane.width;
        if let Some(seam) = seams.get(k) {
            assert_eq!(seam.rect.x, x, "seam {k} follows member {k}");
            x += 1;
        }
    }
    assert_eq!(x, area.right());
    let widths: Vec<u16> = panes.iter().map(|(_, r)| r.width).collect();
    let (min, max) = (
        widths.iter().copied().fold(u16::MAX, u16::min),
        widths.iter().copied().fold(0, u16::max),
    );
    assert!(max - min <= 1, "widths {widths:?}");
}

#[test]
fn a_zero_area_gives_empty_rects() {
    let [a, b] = hume_core::fresh_ids();
    let tree = width_split(vec![LayoutTree::Leaf(a), LayoutTree::Leaf(b)]);
    for (_, pane) in rects(&tree, rect(0, 0, 0, 0)) {
        assert_eq!(pane.width, 0);
    }
    let mut seams = Vec::new();
    tree.collect_seams_into(rect(0, 0, 0, 0), &mut seams);
    assert!(seams.is_empty());
}

#[test]
fn layout_tree_collect_appends_without_clearing() {
    let tree = LayoutTree::Leaf(PaneId::default());
    let mut out = vec![(PaneId::default(), rect(99, 99, 1, 1))]; // pre-existing entry
    tree.collect_rects_into(rect(0, 0, 80, 24), true, &mut out);
    assert_eq!(out.len(), 2); // appended, not replaced
}

#[test]
fn find_rect_matches_collect_rects_into_for_each_pane() {
    // A 4th id from the same slotmap as id_a/id_b/id_c (never inserted into
    // the tree below). Per `pane_ids`'s own doc, a *fresh* map's first key can
    // coincide with another fresh map's first key, so "unknown" must come
    // from this same call, not a separate one.
    let [id_a, id_b, id_c, unknown] = hume_core::fresh_ids();
    let tree = width_split(vec![
        LayoutTree::Leaf(id_a),
        height_split_of_two(id_b, id_c),
    ]);
    let area = rect(0, 0, 100, 50);

    for (pid, expected_rect) in rects(&tree, area) {
        assert_eq!(
            tree.find_rect(pid, area, true),
            Some(expected_rect),
            "find_rect must agree with collect_rects_into for {pid:?}"
        );
    }
    assert_eq!(tree.find_rect(unknown, area, true), None);
}

#[test]
fn collect_seams_leaf_has_no_seams() {
    let tree = LayoutTree::Leaf(PaneId::default());
    let mut out = Vec::new();
    tree.collect_seams_into(rect(0, 0, 80, 24), &mut out);
    assert!(out.is_empty());
}

#[test]
fn collect_seams_yields_one_seam_between_each_two_members() {
    let [a, b, c, d] = hume_core::fresh_ids();
    let tree = width_split(vec![
        LayoutTree::Leaf(a),
        height_split_of_two(b, c),
        LayoutTree::Leaf(d),
    ]);
    let mut out = Vec::new();
    tree.collect_seams_into(rect(0, 0, 100, 100), &mut out);
    let directions: Vec<Direction> = out.iter().map(Seam::direction).collect();
    assert_eq!(
        directions,
        vec![
            Direction::Horizontal,
            Direction::Horizontal,
            Direction::Vertical
        ]
    );
}

// ── FocusedLayout ────────────────────────────────────────────────────

const AREA: Rect = Rect {
    x: 0,
    y: 0,
    width: 100,
    height: 50,
};

fn widths(layout: &FocusedLayout, panes: &[PaneId]) -> Vec<u16> {
    widths_in(AREA, layout, panes)
}

fn resize(layout: &mut FocusedLayout, axis: Direction, how: Resize, cells: u16) {
    assert_eq!(
        layout.resize_focused(AREA, true, axis, how, cells),
        Ok(()),
        "setup: the resize moves a seam"
    );
}

#[test]
fn a_new_layout_is_its_pane_alone_and_focused() {
    let [a] = hume_core::fresh_ids();
    let layout = FocusedLayout::new(UnattachedPane::for_test(a));
    assert_eq!(layout.tree(), &LayoutTree::Leaf(a));
    assert_eq!(layout.focused(), a);
}

#[test]
fn focus_moves_to_a_leaf() {
    let [a, b] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(height_split_of_two(a, b), a);

    layout.focus(b);

    assert_eq!(layout.focused(), b);
}

#[test]
#[should_panic(expected = "a focus target is a leaf of the tree")]
fn focus_refuses_a_pane_outside_the_tree() {
    let [a, b, missing] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(height_split_of_two(a, b), a);

    layout.focus(missing);
}

#[test]
fn next_cycled_pane_follows_leaf_order_and_wraps() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(width_split_of_three(a, b, c), a);
    assert_eq!(layout.pane_cycled_from_focused(Cycle::Next), Some(b));

    layout.focus(c);

    assert_eq!(layout.pane_cycled_from_focused(Cycle::Next), Some(a));
}

#[test]
fn prev_cycled_pane_follows_reverse_leaf_order_and_wraps() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(width_split_of_three(a, b, c), c);
    assert_eq!(layout.pane_cycled_from_focused(Cycle::Prev), Some(b));

    layout.focus(a);

    assert_eq!(layout.pane_cycled_from_focused(Cycle::Prev), Some(c));
}

#[test]
fn cycled_pane_is_none_for_a_sole_pane() {
    let [a] = hume_core::fresh_ids();
    let layout = FocusedLayout::new(UnattachedPane::for_test(a));
    assert_eq!(layout.pane_cycled_from_focused(Cycle::Next), None);
    assert_eq!(layout.pane_cycled_from_focused(Cycle::Prev), None);
}

/// `a` fills the left half; the right half stacks a short `b` (center row 7)
/// over a tall `c` (center row 32). Both touch `a`'s right edge, so they tie
/// on gap, and `a`'s center row 25 is nearer `c`'s center. Ranking by the
/// panes' top rows instead (0 for `b`, 15 for `c`) would pick `b`.
#[test]
fn focused_neighbor_tie_breaks_on_center_distance() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(
        width_split(vec![LayoutTree::Leaf(a), height_split_of_two(b, c)]),
        b,
    );
    resize(&mut layout, Direction::Vertical, Resize::Shrink, 10);
    assert_eq!(rect_of(layout.tree(), b, AREA), rect(50, 0, 50, 14));
    layout.focus(a);

    assert_eq!(layout.focused_neighbor(AREA, true, Side::Right), Some(c));
}

#[test]
#[should_panic(expected = "FocusKey from another EngineView")]
fn a_focus_key_from_another_view_is_refused() {
    let (mut view, _) = EngineView::new(Theme::default(), hume_grid::Rect::default());
    let (_, other_key) = EngineView::new(Theme::default(), hume_grid::Rect::default());
    let pid = view.focused_pane();

    view.focus(&other_key, pid);
}

#[test]
fn split_focused_adds_the_pane_beside_the_focus_and_keeps_focus() {
    let [a, b] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::new(UnattachedPane::for_test(a));

    let new_pane =
        layout.splice_beside_focused(UnattachedPane::for_test(b), Direction::Vertical, None);

    assert_eq!(new_pane, b);
    assert_eq!(layout.tree(), &height_split_of_two(a, b));
    assert_eq!(layout.focused(), a);
}

#[test]
fn a_split_along_a_split_joins_it_after_the_focused_pane() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::new(UnattachedPane::for_test(a));
    layout.splice_beside_focused(UnattachedPane::for_test(b), Direction::Horizontal, None);
    layout.splice_beside_focused(UnattachedPane::for_test(c), Direction::Horizontal, None);

    assert_eq!(layout.tree(), &width_split_of_three(a, c, b));
}

#[test]
fn a_split_across_a_split_stacks_the_focused_pane_with_the_new_one() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(
        width_split(vec![LayoutTree::Leaf(a), LayoutTree::Leaf(b)]),
        b,
    );

    layout.splice_beside_focused(UnattachedPane::for_test(c), Direction::Vertical, None);

    assert_eq!(
        layout.tree(),
        &width_split(vec![LayoutTree::Leaf(a), height_split_of_two(b, c)])
    );
}

#[test]
fn a_split_in_a_resized_split_halves_only_the_focused_pane() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(
        width_split(vec![LayoutTree::Leaf(a), LayoutTree::Leaf(b)]),
        a,
    );
    resize(&mut layout, Direction::Horizontal, Resize::Grow, 10);
    assert_eq!(widths(&layout, &[a, b]), vec![59, 40]);

    layout.splice_beside_focused(UnattachedPane::for_test(c), Direction::Horizontal, None);

    // The new seam takes a cell of `a`'s 59, and `a` and `c` share the rest.
    assert_eq!(widths(&layout, &[a, c, b]), vec![29, 29, 40]);
}

#[test]
fn a_split_and_a_close_in_a_resized_split_leave_it_as_it_was() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(
        width_split(vec![LayoutTree::Leaf(a), LayoutTree::Leaf(b)]),
        a,
    );
    resize(&mut layout, Direction::Horizontal, Resize::Grow, 10);
    let before = layout.tree().clone();
    let new_pane =
        layout.splice_beside_focused(UnattachedPane::for_test(c), Direction::Horizontal, None);
    layout.focus(new_pane);

    let detached = layout.close_focused().expect("three panes");

    assert_eq!(detached.pane_id(), c);
    assert_eq!(layout.tree(), &before);
    assert_eq!(
        layout.focused(),
        a,
        "focus goes to the pane that took the space"
    );
}

#[test]
fn closing_the_first_pane_of_a_resized_split_gives_its_space_to_the_next() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(width_split_of_three(a, b, c), b);
    resize(&mut layout, Direction::Horizontal, Resize::Grow, 6);
    assert_eq!(widths(&layout, &[a, b, c]), vec![32, 39, 27]);
    layout.focus(a);

    let detached = layout.close_focused().expect("three panes");

    assert_eq!(detached.pane_id(), a);
    // `b` takes `a`'s 32 columns and the seam between them; `c` keeps its 27.
    assert_eq!(widths(&layout, &[b, c]), vec![72, 27]);
    assert_eq!(layout.focused(), b);
}

#[test]
fn a_split_across_a_resized_split_leaves_its_seams() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(
        width_split(vec![LayoutTree::Leaf(a), LayoutTree::Leaf(b)]),
        a,
    );
    resize(&mut layout, Direction::Horizontal, Resize::Grow, 10);
    layout.focus(b);

    layout.splice_beside_focused(UnattachedPane::for_test(c), Direction::Vertical, None);

    assert_eq!(widths(&layout, &[a, b, c]), vec![59, 40, 40]);
}

#[test]
fn a_split_with_a_share_places_the_seam_in_the_focused_pane_s_space() {
    let [a, b] = hume_core::fresh_ids();
    let quarter = SplitShare::from_bits(std::num::NonZeroU32::new(1 << 30).expect("not zero"));
    let mut layout = FocusedLayout::new(UnattachedPane::for_test(a));

    layout.splice_beside_focused(
        UnattachedPane::for_test(b),
        Direction::Horizontal,
        Some(quarter),
    );

    // A quarter of the 99 columns left once the seam takes one, rounded down.
    assert_eq!(widths(&layout, &[a, b]), vec![24, 75]);
}

/// A one-pane view on a `width`-column, 25-row terminal, with its screen.
#[allow(clippy::disallowed_methods)]
fn one_pane_view(width: u16) -> (EngineView, ScreenLayout) {
    let (mut view, key) = EngineView::new(Theme::default(), rect(0, 0, width, 25));
    let bid = view.buffers.insert(());
    let pane = view.insert_pane(crate::pane::Pane::new(bid));
    let _placeholder = view.replace_layout(&key, FocusedLayout::new(pane));
    let screen = view.screen_layout(ChromeRows::default(), true);
    (view, screen)
}

/// Split `view`'s focused pane side by side, counting the panes the split
/// opens.
fn split_beside(
    view: &mut EngineView,
    screen: &ScreenLayout,
    share: Option<SplitShare>,
    opened: &mut usize,
) -> Result<PaneId, SplitTooSmall> {
    view.split_focused(screen, Direction::Horizontal, share, |view| {
        *opened += 1;
        let bid = view.buffers.insert(());
        Ok(view.insert_pane(crate::pane::Pane::new(bid)))
    })
}

#[test]
fn a_split_that_fits_opens_its_pane_beside_the_focus() {
    let (mut view, screen) = one_pane_view(80);
    let focus = view.focused_pane();
    let mut opened = 0;

    let new_pane = split_beside(&mut view, &screen, None, &mut opened).expect("80 columns fit");

    assert_eq!(opened, 1);
    assert_eq!(
        view.layout(),
        &width_split(vec![LayoutTree::Leaf(focus), LayoutTree::Leaf(new_pane)])
    );
    assert_eq!(view.focused_pane(), focus);
}

#[test]
fn a_split_too_small_for_two_panes_opens_nothing() {
    // Two 10-column panes and a seam need 21 columns.
    let (mut view, screen) = one_pane_view(20);
    let before = view.layout().clone();
    let mut opened = 0;

    assert_eq!(
        split_beside(&mut view, &screen, None, &mut opened),
        Err(SplitTooSmall)
    );
    assert_eq!(opened, 0);
    assert_eq!(view.layout(), &before);
}

#[test]
fn a_share_that_leaves_a_pane_no_room_is_refused() {
    let (mut view, screen) = one_pane_view(80);
    let mut opened = 0;
    let at = |bits| {
        Some(SplitShare::from_bits(
            std::num::NonZeroU32::new(bits).expect("not zero"),
        ))
    };
    // A share of 1 puts the seam at the start of the focused pane, and the
    // largest one at its end.
    for share in [at(1), at(u32::MAX)] {
        assert_eq!(
            split_beside(&mut view, &screen, share, &mut opened),
            Err(SplitTooSmall)
        );
    }
    assert_eq!(opened, 0);
}

#[test]
fn split_and_close_in_an_equal_split_keep_it_equal() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(
        width_split(vec![LayoutTree::Leaf(a), LayoutTree::Leaf(b)]),
        b,
    );
    layout.splice_beside_focused(UnattachedPane::for_test(c), Direction::Horizontal, None);
    assert_eq!(widths(&layout, &[a, b, c]), vec![32, 33, 33]);
    layout.focus(c);

    let detached = layout.close_focused().expect("three panes");

    assert_eq!(detached.pane_id(), c);
    assert_eq!(
        layout.tree(),
        &width_split(vec![LayoutTree::Leaf(a), LayoutTree::Leaf(b)])
    );
    assert_eq!(layout.focused(), b);
}

#[test]
fn a_perpendicular_member_counts_as_one_share_of_an_equal_split() {
    let [a, b, c, d] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(
        width_split(vec![LayoutTree::Leaf(a), height_split_of_two(b, c)]),
        a,
    );

    layout.splice_beside_focused(UnattachedPane::for_test(d), Direction::Horizontal, None);

    assert_eq!(widths(&layout, &[a, d, b, c]), vec![32, 33, 33, 33]);
}

#[test]
fn close_focused_collapses_the_parent_onto_the_sibling() {
    let [a, b] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(height_split_of_two(a, b), a);
    let detached = layout.close_focused().expect("two panes");
    assert_eq!(detached.pane_id(), a);
    assert_eq!(layout.tree(), &LayoutTree::Leaf(b));
    assert_eq!(layout.focused(), b);
}

#[test]
fn close_focused_moves_focus_to_the_neighbor_subtree_leftmost_leaf() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(
        width_split(vec![LayoutTree::Leaf(a), height_split_of_two(b, c)]),
        a,
    );
    let detached = layout.close_focused().expect("three panes");
    assert_eq!(detached.pane_id(), a);
    assert_eq!(layout.tree(), &height_split_of_two(b, c));
    assert_eq!(layout.focused(), b);
}

#[test]
fn close_focused_refuses_a_sole_pane() {
    let [a] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::new(UnattachedPane::for_test(a));
    assert_eq!(layout.close_focused().err(), Some(SinglePane));
    assert_eq!(layout.tree(), &LayoutTree::Leaf(a));
    assert_eq!(layout.focused(), a);
}

/// `a | (b over (c | d))`, focused on `b`.
fn width_split_around_a_height_split_holding_a_width_split(
    a: PaneId,
    b: PaneId,
    c: PaneId,
    d: PaneId,
) -> FocusedLayout {
    FocusedLayout::from_parts(
        width_split(vec![
            LayoutTree::Leaf(a),
            height_split(vec![
                LayoutTree::Leaf(b),
                width_split(vec![LayoutTree::Leaf(c), LayoutTree::Leaf(d)]),
            ]),
        ]),
        b,
    )
}

#[test]
fn a_collapse_into_a_split_of_the_same_direction_joins_it() {
    let [a, b, c, d] = hume_core::fresh_ids();
    let mut layout = width_split_around_a_height_split_holding_a_width_split(a, b, c, d);

    let detached = layout.close_focused().expect("four panes");

    assert_eq!(detached.pane_id(), b);
    assert_eq!(layout.tree(), &width_split_of_three(a, c, d));
    assert_eq!(layout.focused(), c);
}

#[test]
fn a_collapse_into_a_resized_split_keeps_every_seam_where_it_was() {
    let [a, b, c, d] = hume_core::fresh_ids();
    let mut layout = width_split_around_a_height_split_holding_a_width_split(a, b, c, d);
    layout.focus(a);
    resize(&mut layout, Direction::Horizontal, Resize::Grow, 10);
    assert_eq!(widths(&layout, &[a, c, d]), vec![59, 19, 20]);
    layout.focus(b);

    let detached = layout.close_focused().expect("four panes");

    assert_eq!(detached.pane_id(), b);
    // Each seam keeps its fraction of the split. The split now has one more
    // seam, so one column less to share, and `a` ends a column sooner.
    assert_eq!(widths(&layout, &[a, c, d]), vec![58, 20, 20]);
}

#[test]
fn a_collapse_into_a_resized_split_before_its_last_member_keeps_every_seam_where_it_was() {
    let [a, b, c, d, e] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(
        width_split(vec![
            LayoutTree::Leaf(a),
            height_split(vec![
                LayoutTree::Leaf(b),
                width_split(vec![LayoutTree::Leaf(c), LayoutTree::Leaf(d)]),
            ]),
            LayoutTree::Leaf(e),
        ]),
        a,
    );
    resize(&mut layout, Direction::Horizontal, Resize::Grow, 10);
    assert_eq!(widths(&layout, &[a, c, d, e]), vec![42, 11, 11, 33]);
    layout.focus(b);

    let detached = layout.close_focused().expect("five panes");

    assert_eq!(detached.pane_id(), b);
    let LayoutTree::Split(split) = layout.tree() else {
        panic!("four panes stay split");
    };
    assert_eq!(
        split.members(),
        &[a, c, d, e].map(LayoutTree::Leaf),
        "the collapsed split joins its parent"
    );
    // Each seam keeps its fraction of the split. The split now has one more
    // seam, so one column less to share, every seam lands a column sooner
    // and `e` takes the column left over.
    assert_eq!(widths(&layout, &[a, c, d, e]), vec![41, 11, 11, 34]);
}

#[test]
fn rotate_forward_moves_each_pane_one_place_along_its_split() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(width_split_of_three(a, b, c), b);

    assert_eq!(layout.rotate_focused(Rotation::Forward, 1), Ok(()));

    assert_eq!(layout.tree(), &width_split_of_three(c, a, b));
    assert_eq!(layout.focused(), b, "focus stays on the pane it was on");
}

#[test]
fn rotate_keeps_each_place_s_size_in_a_resized_row() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(width_split_of_three(a, b, c), a);
    resize(&mut layout, Direction::Horizontal, Resize::Grow, 9);
    let member_rects = |layout: &FocusedLayout| {
        rects(layout.tree(), AREA)
            .into_iter()
            .map(|(_, r)| r)
            .collect::<Vec<_>>()
    };
    let before = member_rects(&layout);

    assert_eq!(layout.rotate_focused(Rotation::Forward, 1), Ok(()));

    assert_eq!(member_rects(&layout), before);
    assert_eq!(rect_of(layout.tree(), c, AREA), before[0]);
}

#[test]
fn rotate_backward_moves_each_pane_one_place_back_along_its_split() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(width_split_of_three(a, b, c), b);
    assert_eq!(layout.rotate_focused(Rotation::Backward, 1), Ok(()));
    assert_eq!(layout.tree(), &width_split_of_three(b, c, a));
}

#[test]
fn rotate_by_a_count_moves_each_pane_that_many_places() {
    let [a, b, c, d] = hume_core::fresh_ids();
    let mut forward = FocusedLayout::from_parts(width_split_of_four(a, b, c, d), a);
    assert_eq!(forward.rotate_focused(Rotation::Forward, 2), Ok(()));
    assert_eq!(forward.tree(), &width_split_of_four(c, d, a, b));

    let mut backward = FocusedLayout::from_parts(width_split_of_four(a, b, c, d), a);
    assert_eq!(backward.rotate_focused(Rotation::Backward, 3), Ok(()));
    assert_eq!(backward.tree(), &width_split_of_four(d, a, b, c));
}

#[test]
fn rotate_by_the_member_count_leaves_the_split_unchanged() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(width_split_of_three(a, b, c), a);
    assert_eq!(layout.rotate_focused(Rotation::Forward, 3), Ok(()));
    assert_eq!(layout.tree(), &width_split_of_three(a, b, c));
}

#[test]
fn rotate_moves_a_perpendicular_split_as_one_member() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(
        width_split(vec![LayoutTree::Leaf(a), height_split_of_two(b, c)]),
        a,
    );

    assert_eq!(layout.rotate_focused(Rotation::Forward, 1), Ok(()));

    assert_eq!(
        layout.tree(),
        &width_split(vec![height_split_of_two(b, c), LayoutTree::Leaf(a)])
    );
}

#[test]
fn rotate_inside_a_perpendicular_split_rotates_only_that_split() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(
        width_split(vec![LayoutTree::Leaf(a), height_split_of_two(b, c)]),
        b,
    );

    assert_eq!(layout.rotate_focused(Rotation::Forward, 1), Ok(()));

    assert_eq!(
        layout.tree(),
        &width_split(vec![LayoutTree::Leaf(a), height_split_of_two(c, b)])
    );
}

#[test]
fn rotate_two_levels_below_a_perpendicular_split_rotates_only_the_innermost_split() {
    let [a, b, c, d] = hume_core::fresh_ids();
    let mut layout = width_split_around_a_height_split_holding_a_width_split(a, b, c, d);
    layout.focus(c);

    assert_eq!(layout.rotate_focused(Rotation::Forward, 1), Ok(()));

    assert_eq!(
        layout.tree(),
        width_split_around_a_height_split_holding_a_width_split(a, b, d, c).tree()
    );
}

#[test]
fn rotate_refuses_a_sole_pane() {
    let [a] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::new(UnattachedPane::for_test(a));
    assert_eq!(layout.rotate_focused(Rotation::Forward, 1), Err(SinglePane));
    assert_eq!(layout.tree(), &LayoutTree::Leaf(a));
}

#[test]
fn growing_a_pane_takes_cells_from_the_next_one_only() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(width_split_of_three(a, b, c), b);
    assert_eq!(widths(&layout, &[a, b, c]), vec![32, 33, 33]);

    resize(&mut layout, Direction::Horizontal, Resize::Grow, 5);

    assert_eq!(widths(&layout, &[a, b, c]), vec![32, 38, 28]);
}

#[test]
fn resizing_the_last_pane_moves_the_seam_before_it() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(width_split_of_three(a, b, c), c);

    resize(&mut layout, Direction::Horizontal, Resize::Grow, 3);
    assert_eq!(widths(&layout, &[a, b, c]), vec![32, 30, 36]);

    resize(&mut layout, Direction::Horizontal, Resize::Shrink, 7);
    assert_eq!(widths(&layout, &[a, b, c]), vec![32, 37, 29]);
}

#[test]
fn a_resize_stops_at_the_neighbor_s_minimum_size() {
    let [a, b] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(
        width_split(vec![LayoutTree::Leaf(a), LayoutTree::Leaf(b)]),
        a,
    );

    resize(&mut layout, Direction::Horizontal, Resize::Grow, 1000);
    assert_eq!(widths(&layout, &[a, b]), vec![89, 10]);

    assert_eq!(
        layout.resize_focused(AREA, true, Direction::Horizontal, Resize::Grow, 1),
        Err(ResizeRefused::AtLimit)
    );
    resize(&mut layout, Direction::Horizontal, Resize::Shrink, 1000);
    assert_eq!(widths(&layout, &[a, b]), vec![10, 89]);
}

#[test]
fn a_perpendicular_member_shrinks_no_further_than_its_widest_minimum() {
    let [a, b, c, d] = hume_core::fresh_ids();
    let mut layout = width_split_around_a_height_split_holding_a_width_split(a, b, c, d);
    layout.focus(a);

    resize(&mut layout, Direction::Horizontal, Resize::Grow, 1000);

    // The height split's bottom member needs two 10-column panes and a seam.
    assert_eq!(widths(&layout, &[a, b, c, d]), vec![78, 21, 10, 10]);
}

/// `c` keeps 15 of the inner width split's 49 usable columns. At a height
/// split width `w` it gets `⌊(w − 1) · 15 / 49⌋` columns, which stays at
/// least 10 down to `w = 34`, leaving `a` 99 − 34 columns.
#[test]
fn a_resize_stops_where_a_pane_of_a_resized_nested_split_reaches_its_minimum() {
    let [a, b, c, d] = hume_core::fresh_ids();
    let mut layout = width_split_around_a_height_split_holding_a_width_split(a, b, c, d);
    layout.focus(c);
    resize(&mut layout, Direction::Horizontal, Resize::Shrink, 9);
    assert_eq!(widths(&layout, &[c, d]), vec![15, 34], "setup");
    layout.focus(a);

    resize(&mut layout, Direction::Horizontal, Resize::Grow, 1000);

    assert_eq!(widths(&layout, &[a, c, d]), vec![65, 10, 23]);
}

#[test]
fn dragging_a_divider_stops_where_a_pane_of_a_resized_nested_split_reaches_its_minimum() {
    let [a, b, c, d] = hume_core::fresh_ids();
    let mut layout = width_split_around_a_height_split_holding_a_width_split(a, b, c, d);
    layout.focus(c);
    resize(&mut layout, Direction::Horizontal, Resize::Shrink, 9);
    let mut seams = Vec::new();
    layout.tree().collect_seams_into(AREA, &mut seams);
    let outer = seams
        .iter()
        .find(|seam| seam.rect == rect(49, 0, 1, 50))
        .expect("setup: the divider beside a");

    assert_eq!(
        layout.move_seam(outer.handle, AREA, true, Position::new(90, 7)),
        Ok(())
    );

    assert_eq!(widths(&layout, &[a, c, d]), vec![65, 10, 23]);
    seams.clear();
    layout.tree().collect_seams_into(AREA, &mut seams);
    assert!(seams.iter().any(|seam| seam.rect == rect(65, 0, 1, 50)));
}

/// Every split, resize and divider drag on a few layouts, equal and placed,
/// nested on either axis: afterwards each pane is at least 10 columns by 3
/// rows, or no smaller than it was.
#[test]
fn no_split_resize_or_drag_shrinks_a_pane_below_its_minimum() {
    let ids: [PaneId; 12] = hume_core::fresh_ids();
    let [a, b, c, d, new, five @ ..] = ids;
    let layouts: [&dyn Fn() -> FocusedLayout; 4] = [
        &|| FocusedLayout::from_parts(width_split_of_three(a, b, c), a),
        &|| {
            let mut layout = width_split_around_a_height_split_holding_a_width_split(a, b, c, d);
            layout.focus(c);
            resize(&mut layout, Direction::Horizontal, Resize::Shrink, 9);
            layout
        },
        &|| {
            let mut layout = FocusedLayout::from_parts(
                height_split(vec![
                    LayoutTree::Leaf(a),
                    width_split(vec![LayoutTree::Leaf(b), height_split_of_two(c, d)]),
                ]),
                c,
            );
            resize(&mut layout, Direction::Vertical, Resize::Shrink, 7);
            layout
        },
        &|| {
            let mut layout = width_split_around_a_height_split_holding_five(five);
            resize(&mut layout, Direction::Horizontal, Resize::Shrink, 10);
            layout
        },
    ];
    let keeps_minimums = |before: &[(PaneId, Rect)], after: &[(PaneId, Rect)], what: &str| {
        for &(pid, now) in after {
            let was = before.iter().find(|&&(p, _)| p == pid).map(|&(_, r)| r);
            let width_ok = now.width >= 10 || was.is_some_and(|was| now.width >= was.width);
            let height_ok = now.height >= 3 || was.is_some_and(|was| now.height >= was.height);
            assert!(
                width_ok && height_ok,
                "{what}: {pid:?} was {was:?}, is {now:?}"
            );
        }
    };
    let axes = [Direction::Horizontal, Direction::Vertical];
    for make in layouts {
        let panes = make().tree().leaves();
        for &pane in &panes {
            for axis in axes {
                for how in [Resize::Grow, Resize::Shrink] {
                    let mut layout = make();
                    let before = rects(layout.tree(), AREA);
                    layout.focus(pane);
                    let _ = layout.resize_focused(AREA, true, axis, how, 1000);
                    let what = format!("resize {pane:?} {axis:?} {how:?}");
                    keeps_minimums(&before, &rects(layout.tree(), AREA), &what);
                }
                let mut layout = make();
                let before = rects(layout.tree(), AREA);
                layout.focus(pane);
                if layout.split_fits(AREA, true, axis, None) {
                    layout.splice_beside_focused(UnattachedPane::for_test(new), axis, None);
                }
                let what = format!("split {pane:?} {axis:?}");
                keeps_minimums(&before, &rects(layout.tree(), AREA), &what);
            }
        }
        let mut seams = Vec::new();
        make().tree().collect_seams_into(AREA, &mut seams);
        for seam in seams {
            for to in [Position::new(0, 0), Position::new(99, 49)] {
                let mut layout = make();
                let before = rects(layout.tree(), AREA);
                layout
                    .move_seam(seam.handle, AREA, true, to)
                    .expect("a seam of the layout");
                let what = format!("drag {:?} to {to:?}", seam.rect);
                keeps_minimums(&before, &rects(layout.tree(), AREA), &what);
            }
        }
    }
}

#[test]
fn a_split_whose_share_lands_on_a_seam_is_refused() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(width_split_of_three(a, b, c), b);
    resize(&mut layout, Direction::Horizontal, Resize::Grow, 5);
    let share = SplitShare::from_bits(std::num::NonZeroU32::MIN);

    assert!(!layout.split_fits(AREA, true, Direction::Horizontal, Some(share)));
}

#[test]
fn a_resize_moves_a_seam_of_the_innermost_split_holding_the_pane() {
    let [a, b, c, d] = hume_core::fresh_ids();
    let mut layout = width_split_around_a_height_split_holding_a_width_split(a, b, c, d);
    let wa = widths(&layout, &[a])[0];
    layout.focus(c);

    resize(&mut layout, Direction::Horizontal, Resize::Grow, 4);

    assert_eq!(widths(&layout, &[a])[0], wa);
    assert_eq!(widths(&layout, &[c, d]), vec![28, 21]);
}

/// `a | (b over five panes side by side)`, focused on `a`.
fn width_split_around_a_height_split_holding_five(ids: [PaneId; 7]) -> FocusedLayout {
    let [a, b, row @ ..] = ids;
    FocusedLayout::from_parts(
        width_split(vec![
            LayoutTree::Leaf(a),
            height_split(vec![
                LayoutTree::Leaf(b),
                width_split(row.map(LayoutTree::Leaf).to_vec()),
            ]),
        ]),
        a,
    )
}

#[test]
fn growing_a_pane_never_shrinks_a_pane_inside_it() {
    let ids: [PaneId; 7] = hume_core::fresh_ids();
    let [a, inside @ ..] = ids;
    let mut layout = width_split_around_a_height_split_holding_five(ids);
    resize(&mut layout, Direction::Horizontal, Resize::Shrink, 1);
    let before = widths(&layout, &inside);

    let resized = layout.resize_focused(AREA, true, Direction::Horizontal, Resize::Shrink, 1);

    assert_eq!(resized, Ok(()));
    assert_eq!(widths(&layout, &[a]), vec![47]);
    let after = widths(&layout, &inside);
    assert!(
        before.iter().zip(&after).all(|(was, now)| now >= was),
        "{before:?} became {after:?}"
    );
}

/// A 120-column screen: `a` gets 59 of the 119 columns left by the seam,
/// and the row of five gets the other 60 less its four seams.
const WIDE: Rect = Rect {
    x: 0,
    y: 0,
    width: 120,
    height: 30,
};

fn widths_in(area: Rect, layout: &FocusedLayout, panes: &[PaneId]) -> Vec<u16> {
    panes
        .iter()
        .map(|&p| rect_of(layout.tree(), p, area).width)
        .collect()
}

#[test]
fn narrowing_a_pane_widens_one_pane_of_the_row_beside_it_per_column() {
    let ids: [PaneId; 7] = hume_core::fresh_ids();
    let [a, _, row @ ..] = ids;
    let mut layout = width_split_around_a_height_split_holding_five(ids);
    assert_eq!(widths_in(WIDE, &layout, &[a]), vec![59]);
    assert_eq!(widths_in(WIDE, &layout, &row), vec![11, 11, 11, 11, 12]);

    for step in 1..=12 {
        let before = widths_in(WIDE, &layout, &row);
        let resized = layout.resize_focused(WIDE, true, Direction::Horizontal, Resize::Shrink, 1);

        assert_eq!(resized, Ok(()), "step {step}");
        assert_eq!(
            widths_in(WIDE, &layout, &[a]),
            vec![59 - step],
            "step {step}"
        );
        let after = widths_in(WIDE, &layout, &row);
        let gains: Vec<i32> = before
            .iter()
            .zip(&after)
            .map(|(&was, &now)| i32::from(now) - i32::from(was))
            .collect();
        assert_eq!(
            gains.iter().filter(|&&gain| gain == 1).count(),
            1,
            "step {step}: {gains:?}"
        );
        assert!(
            gains.iter().all(|&gain| gain >= 0),
            "step {step}: {gains:?}"
        );
    }
    assert_eq!(widths_in(WIDE, &layout, &row), vec![13, 13, 14, 14, 14]);
}

#[test]
fn widening_a_pane_stops_when_every_pane_of_the_row_beside_it_is_at_its_minimum() {
    let ids: [PaneId; 7] = hume_core::fresh_ids();
    let [a, _, row @ ..] = ids;
    let mut layout = width_split_around_a_height_split_holding_five(ids);

    let resized = layout.resize_focused(WIDE, true, Direction::Horizontal, Resize::Grow, 40);

    assert_eq!(resized, Ok(()));
    // Five 10-column panes and their four seams leave `a` 119 - 54 columns.
    assert_eq!(widths_in(WIDE, &layout, &row), vec![10; 5]);
    assert_eq!(widths_in(WIDE, &layout, &[a]), vec![65]);
    assert_eq!(
        layout.resize_focused(WIDE, true, Direction::Horizontal, Resize::Grow, 1),
        Err(ResizeRefused::AtLimit)
    );
}

#[test]
fn a_resize_with_no_split_on_its_axis_is_refused() {
    let [a, b] = hume_core::fresh_ids();
    let mut sole = FocusedLayout::new(UnattachedPane::for_test(a));
    assert_eq!(
        sole.resize_focused(AREA, true, Direction::Horizontal, Resize::Grow, 1),
        Err(ResizeRefused::NoNeighbor)
    );
    let mut stacked = FocusedLayout::from_parts(height_split_of_two(a, b), a);
    assert_eq!(
        stacked.resize_focused(AREA, true, Direction::Horizontal, Resize::Grow, 1),
        Err(ResizeRefused::NoNeighbor)
    );
}

#[test]
fn equalize_spreads_every_seam_evenly_again() {
    let [a, b, c, d] = hume_core::fresh_ids();
    let mut layout = width_split_around_a_height_split_holding_a_width_split(a, b, c, d);
    let equal = layout.tree().clone();
    resize(&mut layout, Direction::Vertical, Resize::Grow, 5);
    layout.focus(a);
    resize(&mut layout, Direction::Horizontal, Resize::Grow, 5);

    layout.equalize();

    assert_eq!(layout.tree(), &equal);
}

#[test]
fn moving_a_seam_lands_it_on_the_cell_and_moves_no_other() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(width_split_of_three(a, b, c), a);
    let mut seams = Vec::new();
    layout.tree().collect_seams_into(AREA, &mut seams);
    assert_eq!(seams.len(), 2);

    assert_eq!(
        layout.move_seam(seams[1].handle, AREA, true, Position::new(80, 7)),
        Ok(())
    );

    assert_eq!(rect_of(layout.tree(), c, AREA).x, 81);
    assert_eq!(widths(&layout, &[a, b, c]), vec![32, 47, 19]);
}

#[test]
fn a_seam_handle_does_not_resolve_between_panes_that_are_not_adjacent() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::from_parts(width_split_of_three(a, b, c), b);
    let mut seams = Vec::new();
    layout.tree().collect_seams_into(AREA, &mut seams);
    layout
        .rotate_focused(Rotation::Backward, 1)
        .expect("three panes");

    assert_eq!(
        layout.move_seam(seams[0].handle, AREA, true, Position::new(40, 7)),
        Err(StaleSeam)
    );
}

#[test]
fn focused_seam_segment_full_overlap_horizontal_adjacency() {
    // Two panes side by side with a 1-display_col seam between them at x=49.
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
    // the whole seam.
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
    // only the bottom-right corner, bounded by seams on both sides,
    // survives.
    let pane = rect(0, 0, 10, 10);
    let corners = focused_pane_corners(pane);
    assert_eq!(corners[0], None, "top-left off-origin on both axes");
    assert_eq!(corners[1], None, "top-right off-origin on y");
    assert_eq!(corners[2], None, "bottom-left off-origin on x");
    assert_eq!(corners[3], Some((10, 10)), "bottom-right");
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
    // T. The height split's 100 rows leave 99 to the panes, landing the
    // horizontal seam at row 49; the 100-wide width split below it lands
    // its vertical seam at column 49.
    let [a, b, c] = hume_core::fresh_ids();
    let tree = height_split(vec![
        LayoutTree::Leaf(a),
        width_split(vec![LayoutTree::Leaf(b), LayoutTree::Leaf(c)]),
    ]);
    let mut seams = Vec::new();
    tree.collect_seams_into(rect(0, 0, 100, 100), &mut seams);

    let mut arms = FxHashMap::default();
    collect_seam_arms(&seams, &mut arms);

    // The B|C seam starts one row below the A|BC seam, so it contributes
    // a southward arm to the cell where it meets the horizontal line.
    assert_eq!(arms.get(&(49, 49)), Some(&ARM_S));
}

#[test]
fn collect_seam_arms_cross_junction() {
    // (A|D) over (B|C), both width splits equal so their vertical seams
    // land in the same column (49). The horizontal seam between them is
    // sandwiched by both, producing a full cross.
    let [a, b, c, d] = hume_core::fresh_ids();
    let tree = height_split(vec![
        width_split(vec![LayoutTree::Leaf(a), LayoutTree::Leaf(d)]),
        width_split(vec![LayoutTree::Leaf(b), LayoutTree::Leaf(c)]),
    ]);
    let mut seams = Vec::new();
    tree.collect_seams_into(rect(0, 0, 100, 100), &mut seams);

    let mut arms = FxHashMap::default();
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
    let [a, b, c] = hume_core::fresh_ids();
    let t_tree = height_split(vec![
        LayoutTree::Leaf(a),
        width_split(vec![LayoutTree::Leaf(b), LayoutTree::Leaf(c)]),
    ]);
    let mut seams = Vec::new();
    t_tree.collect_seams_into(rect(0, 0, 100, 100), &mut seams);
    let mut arms = FxHashMap::default();
    collect_seam_arms(&seams, &mut arms);
    // The A|BC seam is horizontal (Direction::Vertical), base E|W.
    let base = ARM_E | ARM_W;
    let mask = base | arms.get(&(49, 49)).copied().unwrap_or(0);
    assert_eq!(junction_glyph(mask), "┬");
}

/// `contains_leaf` must recurse through splits, so a leaf nested two levels
/// down is still found.
#[test]
fn contains_leaf_finds_a_leaf_at_any_depth() {
    let [a, b, c, missing] = hume_core::fresh_ids();
    let tree = width_split(vec![LayoutTree::Leaf(a), height_split_of_two(b, c)]);
    assert!(tree.contains_leaf(a));
    assert!(tree.contains_leaf(c));
    assert!(!tree.contains_leaf(missing));
}

/// `into_detached` must recurse through splits and yield a token for every
/// nested leaf, not just the top level.
#[test]
fn into_detached_yields_one_token_per_leaf() {
    let [a, b, c] = hume_core::fresh_ids();
    let tree = width_split(vec![LayoutTree::Leaf(a), height_split_of_two(b, c)]);
    let ids: Vec<PaneId> = tree
        .into_detached()
        .into_iter()
        .map(|d| d.pane_id())
        .collect();
    assert_eq!(ids.len(), 3);
    for expected in [a, b, c] {
        assert!(ids.contains(&expected));
    }
}

#[test]
fn leaves_collects_every_pane_in_a_split_tree() {
    let [a, b, c] = hume_core::fresh_ids();
    let mut layout = FocusedLayout::new(UnattachedPane::for_test(a));
    layout.splice_beside_focused(UnattachedPane::for_test(b), Direction::Horizontal, None);
    layout.splice_beside_focused(UnattachedPane::for_test(c), Direction::Vertical, None);
    let tree = layout.tree();

    // Order-independent: `leaves` promises a depth-first walk, not a
    // specific one, and `PaneId` has no `Ord` to sort by.
    let out: std::collections::HashSet<_> = tree.leaves().into_iter().collect();
    assert_eq!(out, std::collections::HashSet::from([a, b, c]));
}

#[test]
fn leaves_of_a_sole_leaf_is_just_itself() {
    let [a] = hume_core::fresh_ids();
    let tree = LayoutTree::Leaf(a);

    assert_eq!(tree.leaves(), vec![a]);
}

// ── Bottom band partition ──────────────────────────────────────────────

struct NoopChrome;

impl crate::providers::ChromeProvider for NoopChrome {
    fn render(&self, _area: Rect, _theme: &Theme, _canvas: &mut crate::render::Canvas) {}
}

/// A one-pane view whose chrome can take rows.
fn chromed_view() -> EngineView {
    let (mut view, key) = EngineView::new(Theme::default(), hume_grid::Rect::default());
    let bid = view.buffers.insert(());
    let pane = view.insert_pane(crate::pane::Pane::new(bid));
    let _placeholder = view.replace_layout(&key, FocusedLayout::new(pane));
    view.set_chrome(Chrome {
        tabbar: Box::new(NoopChrome),
        bottom_bands: std::array::from_fn(|_| {
            Box::new(NoopChrome) as Box<dyn crate::providers::ChromeProvider>
        }),
    });
    view
}

/// The screen layout of a 40-column terminal `height` rows tall, with
/// `tabbar` rows at the top and `bands` above the statusline.
#[allow(clippy::disallowed_methods)]
fn screen(height: u16, tabbar: u16, bands: [u16; BOTTOM_BANDS]) -> ScreenLayout {
    let mut view = chromed_view();
    view.resize(rect(0, 0, 40, height));
    view.screen_layout(ChromeRows { tabbar, bands }, true)
}

#[test]
fn pane_area_reserves_drawer_height_above_statusline() {
    let screen = screen(20, 0, [3, 0]);

    // 20 rows total - 1 (statusline) - 3 (drawer) = 16 rows for panes,
    // starting at the top (no tab bar row).
    assert_eq!(screen.pane_area().y, 0);
    assert_eq!(screen.pane_area().height, 16);
}

#[test]
fn pane_area_folds_tabbar_and_drawer_together() {
    let screen = screen(20, 1, [3, 0]);

    // 20 - 1 (tab bar) - 1 (statusline) - 3 (drawer) = 15, offset by the
    // tab bar's 1 row.
    assert_eq!(screen.pane_area().y, 1);
    assert_eq!(screen.pane_area().height, 15);
    assert_eq!(screen.tabbar_area().height, 1);
}

#[test]
fn pane_area_degenerate_when_terminal_too_small_for_chrome_plus_drawer() {
    // The tab bar, the statusline and the drawer take one row each, all 3
    // rows of the terminal, so no row is left for the panes.
    let screen = screen(3, 1, [1, 0]);

    assert_eq!(screen.pane_area().height, 0);
}

#[test]
fn pane_area_sums_multiple_bottom_bands() {
    // Two bands stacked (a drawer and a docked popup) reserve the sum of
    // their heights.
    let screen = screen(20, 0, [3, 2]);

    // 20 - 1 (statusline) - 3 - 2 = 14.
    assert_eq!(screen.pane_area().height, 14);
}

#[test]
fn area_above_statusline_spans_panes_and_bands_below_the_tab_bar() {
    let screen = screen(20, 1, [3, 2]);

    // Rows 1..19: everything but the tab bar (row 0) and the statusline
    // (row 19).
    assert_eq!(screen.area_above_statusline(), rect(0, 1, 40, 18));
}

#[test]
fn statusline_area_is_the_last_row_and_empty_on_a_rowless_terminal() {
    assert_eq!(screen(20, 1, [3, 2]).statusline_area(), rect(0, 19, 40, 1));
    assert_eq!(screen(0, 0, [0, 0]).statusline_area().height, 0);
}

#[test]
#[allow(clippy::disallowed_methods)]
fn a_lone_pane_is_sized_to_the_pane_area() {
    let mut view = chromed_view();
    view.resize(rect(0, 0, 40, 20));
    let screen = view.screen_layout(
        ChromeRows {
            tabbar: 1,
            bands: [3, 0],
        },
        true,
    );

    let size = screen
        .pane_size(view.focused_layout(), view.focused_pane())
        .expect("the focused pane is in the layout");

    // 20 - 1 (tab bar) - 1 (statusline) - 3 (drawer) = 15 rows.
    assert_eq!((size.width(), size.height()), (40, 15));
}

#[test]
#[should_panic(expected = "chrome rows without chrome painters")]
#[allow(clippy::disallowed_methods)]
fn chrome_rows_without_painters_panic() {
    let (view, _key) = EngineView::new(Theme::default(), hume_grid::Rect::default());
    view.screen_layout(
        ChromeRows {
            tabbar: 1,
            bands: [0, 0],
        },
        true,
    );
}
