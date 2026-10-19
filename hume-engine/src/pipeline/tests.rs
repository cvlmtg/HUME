use hume_grid::{Grid, Position, Rect, Rgb};
use std::collections::HashMap;

use crate::test_support::{fg, theme_with};

use super::*;

use super::layout::split_rect;
use crate::display_lines::DisplayLinePos;
use crate::pane::{WhitespaceConfig, WrapMode};
use crate::providers::{
    Decoration, DecorationKinds, DecorationSource, VirtualLine, VirtualLineAnchor,
};
use crate::types::{DisplayLineKind, ResolvedStyle};
use hume_rope::column::ByteCol;
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
        indent_guides: true,
        cursor_is_block: true,
        gutter: crate::providers::GutterInputs::default(),
    };
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
                anchor: VirtualLineAnchor::Before(ContentLine::new(0)),
                provider_id: 0,
                text: "H~".to_string(),
                // "H" (byte 0..1) carries the scope; "~" (byte 1..2) carries
                // none and must fall back to `ui.virtual_text`.
                segments: vec![(ByteCol::new(0), ByteCol::new(1), self.scope)],
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
    let mut bids: SlotMap<BufferId, ()> = SlotMap::with_key();
    let bid = bids.insert(());
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

/// Emits one `Before(0)` display line whose four graphemes each carry a distinct
/// scope, with `segments` built in the *reverse* of byte order. The engine
/// contract (`VirtualLine::segments` doc) promises `DisplayLineMap::block` sorts
/// this at intake, and `format_virtual_lines`'s cursor scan requires that:
/// without the sort, a monotonic cursor started against descending-start
/// input never backs up, so every offset but the last resolves to `None`.
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
                anchor: VirtualLineAnchor::Before(ContentLine::new(0)),
                provider_id: 0,
                text: "ABCD".to_string(),
                segments: vec![
                    (ByteCol::new(3), ByteCol::new(4), self.scopes[3]),
                    (ByteCol::new(2), ByteCol::new(3), self.scopes[2]),
                    (ByteCol::new(1), ByteCol::new(2), self.scopes[1]),
                    (ByteCol::new(0), ByteCol::new(1), self.scopes[0]),
                ],
                base_scope: None,
            }));
        }
    }
}

#[test]
fn virtual_display_line_resolves_scopes_from_unsorted_segments() {
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
    let mut bids: SlotMap<BufferId, ()> = SlotMap::with_key();
    let bid = bids.insert(());
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

/// Emits one virtual line anchored to a fixed line index, only when that
/// line is in the visible range. Smoke-tests the whole virtual-line path
/// (`ProviderSet::add_decoration_source` → `render_pane`).
struct FixedVirtualLineSource {
    anchor: VirtualLineAnchor,
    text: &'static str,
}

impl DecorationSource for FixedVirtualLineSource {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }
    fn decorations_for_line(&self, line_idx: ContentLine, out: &mut Vec<Decoration>) {
        let line = match self.anchor {
            VirtualLineAnchor::Before(n) | VirtualLineAnchor::After(n) => n,
        };
        if line_idx == line {
            out.push(Decoration::VirtualLine(VirtualLine {
                anchor: self.anchor,
                provider_id: 0,
                text: self.text.to_string(),
                segments: Vec::new(),
                base_scope: None,
            }));
        }
    }
}

/// Build a pane viewing a rope whose line 0 is "aaaabbbbcccc". Under
/// `WrapMode::Soft { width: 4 }` this wraps into exactly 3 display lines:
/// "aaaa", "bbbb", "cccc" (each grapheme is 1 column, so Soft splits at the
/// exact column with no backtracking). `top_slot` controls how many of
/// those wrap display lines are already scrolled past.
fn render_wrapped_pane_with_virtual_line(top_slot: u16, anchor: VirtualLineAnchor) -> Grid {
    render_wrapped_pane_with_virtual_text(top_slot, anchor, "V")
}

/// [`render_wrapped_pane_with_virtual_line`] with the virtual line's own text.
fn render_wrapped_pane_with_virtual_text(
    top_slot: u16,
    anchor: VirtualLineAnchor,
    text: &'static str,
) -> Grid {
    let rope = ropey::Rope::from_str("aaaabbbbcccc\nz\n");
    let mut bids: SlotMap<BufferId, ()> = SlotMap::with_key();
    let bid = bids.insert(());

    let mut pane = Pane::new(bid);
    pane.viewport
        .seed_top_for_test(DisplayLinePos::new(ContentLine::new(0), top_slot as usize));
    pane.providers
        .add_decoration_source(Box::new(FixedVirtualLineSource { anchor, text }));

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
        render_wrapped_pane_with_virtual_line(1, VirtualLineAnchor::Before(ContentLine::new(0)));
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
        render_wrapped_pane_with_virtual_line(2, VirtualLineAnchor::Before(ContentLine::new(0)));
    assert_eq!(
        cell_symbol(&offset2, 0, 0),
        "b",
        "offset 2 skips V and wrap display line 0"
    );

    let offset3 =
        render_wrapped_pane_with_virtual_line(3, VirtualLineAnchor::Before(ContentLine::new(0)));
    assert_eq!(
        cell_symbol(&offset3, 0, 0),
        "c",
        "offset 3 reaches wrap display line 2"
    );

    let offset4 =
        render_wrapped_pane_with_virtual_line(4, VirtualLineAnchor::Before(ContentLine::new(0)));
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
        render_wrapped_pane_with_virtual_line(5, VirtualLineAnchor::Before(ContentLine::new(0)));
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
        render_wrapped_pane_with_virtual_line(0, VirtualLineAnchor::Before(ContentLine::new(0)));
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
        VirtualLineAnchor::After(ContentLine::new(0)),
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
    let buf =
        render_wrapped_pane_with_virtual_line(1, VirtualLineAnchor::After(ContentLine::new(0)));
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

/// Emits `self.0` distinct `Before(0)` virtual display lines, texted "1".."9":
/// a virtual block taller than a small viewport, to prove every display
/// line in it is individually reachable by scrolling, not just the display
/// lines nearest the edge.
struct MultiBeforeLine(usize);

impl DecorationSource for MultiBeforeLine {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }
    fn decorations_for_line(&self, line_idx: ContentLine, out: &mut Vec<Decoration>) {
        if line_idx == ContentLine::new(0) {
            for i in 0..self.0 {
                out.push(Decoration::VirtualLine(VirtualLine {
                    anchor: VirtualLineAnchor::Before(ContentLine::new(0)),
                    provider_id: 0,
                    text: (i + 1).to_string(),
                    segments: Vec::new(),
                    base_scope: None,
                }));
            }
        }
    }
}

/// Line 0 is "x" (one unwrapped display line) with `n` `Before(0)` virtual
/// display lines stacked above it, viewed through a viewport `height` rows
/// tall.
fn render_pane_with_n_before_lines(top_slot: u16, n: usize, height: u16) -> Grid {
    let rope = ropey::Rope::from_str("x\ny\n");
    let mut bids: SlotMap<BufferId, ()> = SlotMap::with_key();
    let bid = bids.insert(());

    let mut pane = Pane::new(bid);
    pane.viewport
        .seed_top_for_test(DisplayLinePos::new(ContentLine::new(0), top_slot as usize));
    pane.providers
        .add_decoration_source(Box::new(MultiBeforeLine(n)));

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
    anchor: VirtualLineAnchor,
}

impl DecorationSource for SpoofingVirtualLineSource {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }
    fn decorations_for_line(&self, line_idx: ContentLine, out: &mut Vec<Decoration>) {
        let line = match self.anchor {
            VirtualLineAnchor::Before(n) | VirtualLineAnchor::After(n) => n,
        };
        if line_idx == line {
            out.push(Decoration::VirtualLine(VirtualLine {
                anchor: self.anchor,
                provider_id: 9999, // spoofed, must be overwritten by the pipeline
                text: "V".to_string(),
                segments: Vec::new(),
                base_scope: None,
            }));
        }
    }
}

/// Renders the `DisplayLineKind::Virtual` provider_id as gutter text, so the test
/// can observe what actually reached `compose` after collection.
struct ProviderIdReportingGutter;

impl crate::providers::GutterColumn for ProviderIdReportingGutter {
    fn width(&self, _: RopeyLine, _: &crate::providers::GutterInputs) -> u8 {
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
    let mut bids: SlotMap<BufferId, ()> = SlotMap::with_key();
    let bid = bids.insert(());
    let mut pane = Pane::new(bid);
    pane.providers
        .add_gutter_column(Box::new(ProviderIdReportingGutter));
    let real_id = pane
        .providers
        .add_decoration_source(Box::new(SpoofingVirtualLineSource {
            anchor: VirtualLineAnchor::Before(ContentLine::new(0)),
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
    let mut bids: SlotMap<BufferId, ()> = SlotMap::with_key();
    let bid = bids.insert(());

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
    let mut bids: SlotMap<BufferId, ()> = SlotMap::with_key();
    let bid = bids.insert(());

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
        fn width(&self, _: RopeyLine, _: &crate::providers::GutterInputs) -> u8 {
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
    let mut bids: SlotMap<BufferId, ()> = SlotMap::with_key();
    let bid = bids.insert(());

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

#[test]
fn find_rect_matches_collect_rects_into_for_each_pane() {
    // A 4th id from the same slotmap as id_a/id_b/id_c (never inserted into
    // the tree below). Per `pane_ids`'s own doc, a *fresh* map's first key can
    // coincide with another fresh map's first key, so "unknown" must come
    // from this same call, not a separate one.
    let [id_a, id_b, id_c, unknown] = pane_ids();
    // A 3-pane tree: (a | (b / c)), a horizontal split whose right side is
    // itself split vertically.
    let tree = LayoutTree::Split {
        direction: Direction::Horizontal,
        ratio: 0.5,
        children: Box::new((
            LayoutTree::Leaf(id_a),
            LayoutTree::Split {
                direction: Direction::Vertical,
                ratio: 0.5,
                children: Box::new((LayoutTree::Leaf(id_b), LayoutTree::Leaf(id_c))),
            },
        )),
    };
    let area = rect(0, 0, 100, 50);
    let mut collected = Vec::new();
    tree.collect_rects_into(area, true, &mut collected);

    for &(pid, expected_rect) in &collected {
        assert_eq!(
            tree.find_rect(pid, area, true),
            Some(expected_rect),
            "find_rect must agree with collect_rects_into for {pid:?}"
        );
    }
    // A pid absent from the tree finds nothing.
    assert_eq!(tree.find_rect(unknown, area, true), None);
}

#[test]
fn find_containing_matches_collect_rects_into_by_position() {
    let [id_a, id_b, id_c] = pane_ids();
    let tree = LayoutTree::Split {
        direction: Direction::Horizontal,
        ratio: 0.5,
        children: Box::new((
            LayoutTree::Leaf(id_a),
            LayoutTree::Split {
                direction: Direction::Vertical,
                ratio: 0.5,
                children: Box::new((LayoutTree::Leaf(id_b), LayoutTree::Leaf(id_c))),
            },
        )),
    };
    let area = rect(0, 0, 100, 50);
    let mut collected = Vec::new();
    tree.collect_rects_into(area, true, &mut collected);

    // One interior point per leaf: same pane and rect either way.
    for &(pid, r) in &collected {
        let pos = Position::new(r.x, r.y);
        assert_eq!(
            tree.find_containing(pos, area, true),
            Some((pid, r)),
            "find_containing must agree with collect_rects_into at {pos:?}"
        );
    }
    // A point in the horizontal seam column (x=49, reserved by the a|bc
    // split) falls inside no leaf's rect.
    let seam_pos = Position::new(49, 0);
    assert!(collected.iter().all(|&(_, r)| !r.contains(seam_pos)));
    assert_eq!(tree.find_containing(seam_pos, area, true), None);
}

// ── FocusedLayout ────────────────────────────────────────────────────

fn split(direction: Direction, ratio: f32, a: LayoutTree, b: LayoutTree) -> LayoutTree {
    LayoutTree::Split {
        direction,
        ratio,
        children: Box::new((a, b)),
    }
}

/// `a | b | c` as the nested chain `a | (b | c)`, with equalized ratios.
fn row_of_three(a: PaneId, b: PaneId, c: PaneId) -> LayoutTree {
    split(
        Direction::Horizontal,
        1.0 / 3.0,
        LayoutTree::Leaf(a),
        split(
            Direction::Horizontal,
            0.5,
            LayoutTree::Leaf(b),
            LayoutTree::Leaf(c),
        ),
    )
}

/// `a | b | c | d` as the nested chain `a | (b | (c | d))`.
fn row_of_four(a: PaneId, b: PaneId, c: PaneId, d: PaneId) -> LayoutTree {
    split(
        Direction::Horizontal,
        0.25,
        LayoutTree::Leaf(a),
        row_of_three(b, c, d),
    )
}

/// `top` stacked above `bottom`.
fn column_of_two(top: PaneId, bottom: PaneId) -> LayoutTree {
    split(
        Direction::Vertical,
        0.5,
        LayoutTree::Leaf(top),
        LayoutTree::Leaf(bottom),
    )
}

#[test]
fn a_new_layout_is_its_pane_alone_and_focused() {
    let [a] = pane_ids();
    let layout = FocusedLayout::new(UnattachedPane::for_test(a));
    assert_eq!(layout.tree(), &LayoutTree::Leaf(a));
    assert_eq!(layout.focused(), a);
}

#[test]
fn focus_moves_to_a_leaf() {
    let [a, b] = pane_ids();
    let mut layout = FocusedLayout::from_parts(column_of_two(a, b), a);

    layout.focus(b);

    assert_eq!(layout.focused(), b);
}

#[test]
#[should_panic(expected = "a focus target is a leaf of the tree")]
fn focus_refuses_a_pane_outside_the_tree() {
    let [a, b, missing] = pane_ids();
    let mut layout = FocusedLayout::from_parts(column_of_two(a, b), a);

    layout.focus(missing);
}

#[test]
fn next_cycled_pane_follows_leaf_order_and_wraps() {
    let [a, b, c] = pane_ids();
    let mut layout = FocusedLayout::from_parts(row_of_three(a, b, c), a);
    assert_eq!(layout.pane_cycled_from_focused(Cycle::Next), Some(b));

    layout.focus(c);

    assert_eq!(layout.pane_cycled_from_focused(Cycle::Next), Some(a));
}

#[test]
fn prev_cycled_pane_follows_reverse_leaf_order_and_wraps() {
    let [a, b, c] = pane_ids();
    let mut layout = FocusedLayout::from_parts(row_of_three(a, b, c), c);
    assert_eq!(layout.pane_cycled_from_focused(Cycle::Prev), Some(b));

    layout.focus(a);

    assert_eq!(layout.pane_cycled_from_focused(Cycle::Prev), Some(c));
}

#[test]
fn cycled_pane_is_none_for_a_sole_pane() {
    let [a] = pane_ids();
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
    let [a, b, c] = pane_ids();
    let tree = split(
        Direction::Horizontal,
        0.5,
        LayoutTree::Leaf(a),
        split(
            Direction::Vertical,
            0.3,
            LayoutTree::Leaf(b),
            LayoutTree::Leaf(c),
        ),
    );
    let layout = FocusedLayout::from_parts(tree, a);

    assert_eq!(
        layout.focused_neighbor(rect(0, 0, 100, 50), true, Side::Right),
        Some(c)
    );
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
    let [a, b] = pane_ids();
    let mut layout = FocusedLayout::new(UnattachedPane::for_test(a));

    let new_pane = layout.split_focused(UnattachedPane::for_test(b), Direction::Vertical);

    assert_eq!(new_pane, b);
    assert_eq!(layout.tree(), &column_of_two(a, b));
    assert_eq!(layout.focused(), a);
}

#[test]
fn rotate_forward_moves_each_pane_one_place_along_its_row() {
    let [a, b, c] = pane_ids();
    let mut layout = FocusedLayout::from_parts(row_of_three(a, b, c), b);
    let area = rect(0, 0, 100, 50);
    let mut before = Vec::new();
    layout.tree().collect_rects_into(area, true, &mut before);

    assert_eq!(layout.rotate_focused(Rotation::Forward, 1), Ok(()));

    assert_eq!(layout.tree(), &row_of_three(c, a, b));
    assert_eq!(layout.focused(), b, "focus stays on the pane it was on");
    let mut after = Vec::new();
    layout.tree().collect_rects_into(area, true, &mut after);
    let member_rects = |v: &[(PaneId, Rect)]| v.iter().map(|&(_, r)| r).collect::<Vec<_>>();
    assert_eq!(member_rects(&after), member_rects(&before));
}

#[test]
fn rotate_backward_moves_each_pane_one_place_back_along_its_row() {
    let [a, b, c] = pane_ids();
    let mut layout = FocusedLayout::from_parts(row_of_three(a, b, c), b);
    assert_eq!(layout.rotate_focused(Rotation::Backward, 1), Ok(()));
    assert_eq!(layout.tree(), &row_of_three(b, c, a));
}

#[test]
fn rotate_by_a_count_moves_each_pane_that_many_places() {
    let [a, b, c, d] = pane_ids();
    let mut forward = FocusedLayout::from_parts(row_of_four(a, b, c, d), a);
    assert_eq!(forward.rotate_focused(Rotation::Forward, 2), Ok(()));
    assert_eq!(forward.tree(), &row_of_four(c, d, a, b));

    let mut backward = FocusedLayout::from_parts(row_of_four(a, b, c, d), a);
    assert_eq!(backward.rotate_focused(Rotation::Backward, 3), Ok(()));
    assert_eq!(backward.tree(), &row_of_four(d, a, b, c));
}

#[test]
fn rotate_by_the_member_count_leaves_the_row_unchanged() {
    let [a, b, c] = pane_ids();
    let mut layout = FocusedLayout::from_parts(row_of_three(a, b, c), a);
    assert_eq!(layout.rotate_focused(Rotation::Forward, 3), Ok(()));
    assert_eq!(layout.tree(), &row_of_three(a, b, c));
}

#[test]
fn rotate_treats_a_left_nested_chain_as_one_row() {
    let [a, b, c] = pane_ids();
    let left_nested = |x, y, z| {
        split(
            Direction::Horizontal,
            2.0 / 3.0,
            split(
                Direction::Horizontal,
                0.5,
                LayoutTree::Leaf(x),
                LayoutTree::Leaf(y),
            ),
            LayoutTree::Leaf(z),
        )
    };
    let mut layout = FocusedLayout::from_parts(left_nested(a, b, c), c);

    assert_eq!(layout.rotate_focused(Rotation::Forward, 1), Ok(()));

    assert_eq!(layout.tree(), &left_nested(c, a, b));
}

#[test]
fn rotate_moves_a_perpendicular_split_as_one_member() {
    let [a, b, c] = pane_ids();
    let mut layout = FocusedLayout::from_parts(
        split(
            Direction::Horizontal,
            0.5,
            LayoutTree::Leaf(a),
            column_of_two(b, c),
        ),
        a,
    );

    assert_eq!(layout.rotate_focused(Rotation::Forward, 1), Ok(()));

    assert_eq!(
        layout.tree(),
        &split(
            Direction::Horizontal,
            0.5,
            column_of_two(b, c),
            LayoutTree::Leaf(a)
        )
    );
}

#[test]
fn rotate_inside_a_perpendicular_split_rotates_only_that_column() {
    let [a, b, c] = pane_ids();
    let mut layout = FocusedLayout::from_parts(
        split(
            Direction::Horizontal,
            0.5,
            LayoutTree::Leaf(a),
            column_of_two(b, c),
        ),
        b,
    );

    assert_eq!(layout.rotate_focused(Rotation::Forward, 1), Ok(()));

    assert_eq!(
        layout.tree(),
        &split(
            Direction::Horizontal,
            0.5,
            LayoutTree::Leaf(a),
            column_of_two(c, b)
        )
    );
}

#[test]
fn rotate_two_levels_below_a_perpendicular_split_rotates_only_the_innermost_row() {
    let [a, b, c, d] = pane_ids();
    let nested = |x, y| {
        split(
            Direction::Horizontal,
            0.5,
            LayoutTree::Leaf(a),
            split(
                Direction::Vertical,
                0.5,
                LayoutTree::Leaf(b),
                split(
                    Direction::Horizontal,
                    0.5,
                    LayoutTree::Leaf(x),
                    LayoutTree::Leaf(y),
                ),
            ),
        )
    };
    let mut layout = FocusedLayout::from_parts(nested(c, d), c);

    assert_eq!(layout.rotate_focused(Rotation::Forward, 1), Ok(()));

    assert_eq!(layout.tree(), &nested(d, c));
}

#[test]
fn rotate_refuses_a_sole_pane() {
    let [a] = pane_ids();
    let mut layout = FocusedLayout::new(UnattachedPane::for_test(a));
    assert_eq!(layout.rotate_focused(Rotation::Forward, 1), Err(SinglePane));
    assert_eq!(layout.tree(), &LayoutTree::Leaf(a));
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
    assert_eq!(out.len(), 2, "one seam per split node: root + nested");
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

/// `PaneId::default()` is the slotmap null key: every default is equal,
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

    let mut arms = FxHashMap::default();
    collect_seam_arms(&seams, &mut arms);

    // The B|C seam starts one row below the A|BC seam, so it contributes
    // a southward arm to the cell where it meets the horizontal line.
    assert_eq!(arms.get(&(49, 49)), Some(&ARM_S));
}

#[test]
fn collect_seam_arms_cross_junction() {
    // (A|D) over (B|C), both rows split at the same ratio so their
    // vertical seams land in the same column (49). The horizontal seam
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
    let mut arms = FxHashMap::default();
    collect_seam_arms(&seams, &mut arms);
    // The A|BC seam is horizontal (Direction::Vertical), base E|W.
    let base = ARM_E | ARM_W;
    let mask = base | arms.get(&(49, 49)).copied().unwrap_or(0);
    assert_eq!(junction_glyph(mask), "┬");
}

#[test]
fn split_focused_on_a_nested_leaf() {
    let [a, b, c] = pane_ids();
    let mut layout = FocusedLayout::from_parts(
        split(
            Direction::Horizontal,
            0.5,
            LayoutTree::Leaf(a),
            LayoutTree::Leaf(b),
        ),
        b,
    );
    layout.split_focused(UnattachedPane::for_test(c), Direction::Vertical);
    let tree = layout.tree();
    assert_eq!(
        tree,
        &LayoutTree::Split {
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
fn split_focused_twice_gives_equal_thirds() {
    let [a, b, c] = pane_ids();
    let mut layout = FocusedLayout::new(UnattachedPane::for_test(a));
    layout.split_focused(UnattachedPane::for_test(b), Direction::Horizontal);
    layout.split_focused(UnattachedPane::for_test(c), Direction::Horizontal);
    let tree = layout.tree();

    // `a` is split again, so it (now paired with `c`) occupies 2 of the 3
    // slots on the horizontal axis (hence 2/3, not 1/3), with the nested
    // (a, c) pair itself split 0.5. The two ratios compose to an equal third
    // apiece, checked below against rendered widths.
    assert_eq!(
        tree,
        &LayoutTree::Split {
            direction: Direction::Horizontal,
            ratio: 2.0 / 3.0,
            children: Box::new((
                LayoutTree::Split {
                    direction: Direction::Horizontal,
                    ratio: 0.5,
                    children: Box::new((LayoutTree::Leaf(a), LayoutTree::Leaf(c))),
                },
                LayoutTree::Leaf(b),
            )),
        }
    );

    // Read the rects back, not the ratios again.
    let mut out = Vec::new();
    tree.collect_rects_into(rect(0, 0, 99, 10), false, &mut out);
    assert_eq!(out.len(), 3);
    let widths: Vec<u16> = out.iter().map(|(_, r)| r.width).collect();
    let min = *widths.iter().min().unwrap();
    let max = *widths.iter().max().unwrap();
    assert!(
        max - min <= 1,
        "widths {widths:?} not within 1 of each other"
    );
}

#[test]
fn split_focused_counts_perpendicular_subtree_as_one_share() {
    let [a, b, c, d] = pane_ids();
    let mut layout = FocusedLayout::from_parts(
        split(
            Direction::Horizontal,
            0.5,
            LayoutTree::Leaf(a),
            column_of_two(b, c),
        ),
        a,
    );
    layout.split_focused(UnattachedPane::for_test(d), Direction::Horizontal);
    let tree = layout.tree();
    let LayoutTree::Split { ratio, .. } = tree else {
        panic!("root must still be a split");
    };
    assert_eq!(*ratio, 2.0 / 3.0);

    let mut out = Vec::new();
    tree.collect_rects_into(rect(0, 0, 99, 10), false, &mut out);
    let widths: Vec<u16> = out.iter().map(|(_, r)| r.width).collect();
    let min = *widths.iter().min().unwrap();
    let max = *widths.iter().max().unwrap();
    assert!(
        max - min <= 1,
        "widths {widths:?} not within 1 of each other"
    );
}

#[test]
fn close_focused_collapses_the_parent_onto_the_sibling() {
    let [a, b] = pane_ids();
    let mut layout = FocusedLayout::from_parts(column_of_two(a, b), a);
    let detached = layout.close_focused().expect("two panes");
    assert_eq!(detached.pane_id(), a);
    assert_eq!(layout.tree(), &LayoutTree::Leaf(b));
    assert_eq!(layout.focused(), b);
}

#[test]
fn close_focused_moves_focus_to_the_promoted_subtree_leftmost_leaf() {
    let [a, b, c] = pane_ids();
    let mut layout = FocusedLayout::from_parts(
        split(
            Direction::Horizontal,
            0.5,
            LayoutTree::Leaf(a),
            column_of_two(b, c),
        ),
        a,
    );
    let detached = layout.close_focused().expect("three panes");
    assert_eq!(detached.pane_id(), a);
    assert_eq!(layout.tree(), &column_of_two(b, c));
    assert_eq!(layout.focused(), b);
}

#[test]
fn close_focused_refuses_a_sole_pane() {
    let [a] = pane_ids();
    let mut layout = FocusedLayout::new(UnattachedPane::for_test(a));
    assert_eq!(layout.close_focused().err(), Some(SinglePane));
    assert_eq!(layout.tree(), &LayoutTree::Leaf(a));
    assert_eq!(layout.focused(), a);
}

#[test]
fn close_focused_equalizes_survivors() {
    let [a, b, c] = pane_ids();
    let mut layout = FocusedLayout::new(UnattachedPane::for_test(a));
    layout.split_focused(UnattachedPane::for_test(b), Direction::Horizontal);
    let new_pane = layout.split_focused(UnattachedPane::for_test(c), Direction::Horizontal);
    layout.focus(new_pane);

    let detached = layout.close_focused().expect("three panes");
    assert_eq!(detached.pane_id(), c);
    assert_eq!(layout.focused(), a);
    assert_eq!(
        layout.tree(),
        &split(
            Direction::Horizontal,
            0.5,
            LayoutTree::Leaf(a),
            LayoutTree::Leaf(b)
        )
    );
}

/// `contains_leaf` must recurse through `Split` nodes, so a leaf nested two
/// levels down is still found.
#[test]
fn contains_leaf_finds_a_leaf_at_any_depth() {
    let [a, b, c, missing] = pane_ids();
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
    assert!(tree.contains_leaf(a));
    assert!(tree.contains_leaf(c));
    assert!(!tree.contains_leaf(missing));
}

/// `into_detached` must recurse through `Split` nodes and yield a token for
/// every nested leaf, not just the top level.
#[test]
fn into_detached_yields_one_token_per_leaf() {
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
    let [a, b, c] = pane_ids();
    let mut layout = FocusedLayout::new(UnattachedPane::for_test(a));
    layout.split_focused(UnattachedPane::for_test(b), Direction::Horizontal);
    layout.split_focused(UnattachedPane::for_test(c), Direction::Vertical);
    let tree = layout.tree();

    // Order-independent: `leaves` promises a depth-first walk, not a
    // specific one, and `PaneId` has no `Ord` to sort by.
    let out: std::collections::HashSet<_> = tree.leaves().into_iter().collect();
    assert_eq!(out, std::collections::HashSet::from([a, b, c]));
}

#[test]
fn leaves_of_a_sole_leaf_is_just_itself() {
    let [a] = pane_ids();
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

// ── FrameScratch ─────────────────────────────────────────────────────

#[test]
fn frame_scratch_clear_retains_capacity() {
    let mut s = FrameScratch::new();
    for _ in 0..100 {
        s.style.styles.push(hume_grid::ResolvedStyle::default());
    }
    let cap_before = s.style.styles.capacity();
    s.clear();
    assert_eq!(s.style.styles.len(), 0);
    assert!(s.style.styles.capacity() >= cap_before);
}
