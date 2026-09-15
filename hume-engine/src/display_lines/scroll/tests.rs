//! Tests for the viewport scroll verbs (`heal`/`scroll_by`/`reveal`/`align`/
//! `reveal_horizontal`) and [`carry`].
//!
//! Ported from `hume-editor`'s former `editor/scroll/tests.rs`, which tested
//! the same behavior when it lived in `hume-editor` as free functions over a
//! `top_line`/`top_slot` pair. `local_content_pos`/`local_place` mirror
//! `hume-editor`'s `cursor::content_pos`/`place` just closely enough to
//! verify a verb's reported row agrees with a fresh forward walk — kept here
//! rather than pulling hume-editor into an hume-engine test.

use std::cell::Cell;
use std::rc::Rc;

use ropey::Rope;

use super::*;
use crate::display_lines::line_store::{FormatKey, PaneLineStore};
use crate::pane::{Viewport, WhitespaceConfig, WrapMode};
use crate::providers::{
    Decoration, DecorationKinds, DecorationSource, ProviderSet, VirtualLine, VirtualLineAnchor,
};
use hume_rope::column::DisplayLineCol;
use hume_rope::line::ContentLine;
use hume_rope::offset::CharOffset;

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

fn rope(text: &str) -> Rope {
    Rope::from_str(text)
}

fn viewport(top: usize, height: u16, width: u16) -> Viewport {
    let mut v = Viewport::new(width, height);
    v.seed_top_for_test(DisplayLinePos::new(ContentLine::new(top), 0));
    v
}

fn map<'a>(
    rope: &'a Rope,
    wrap: WrapMode,
    providers: &'a ProviderSet,
    content_width: u16,
    store: &'a mut PaneLineStore,
) -> DisplayLineMap<'a> {
    DisplayLineMap::new(
        rope,
        providers,
        content_width,
        FormatKey {
            buffer_tag: [0; 3],
            wrap_mode: wrap,
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
        },
        store,
    )
}

/// Mirror of `hume-editor`'s `cursor::content_pos` — see this module's doc.
fn local_content_pos(
    v: &Viewport,
    dlm: &mut DisplayLineMap<'_>,
    cursor_char: CharOffset,
) -> Option<(u16, u16)> {
    if v.height == 0 {
        return None;
    }
    let (cursor_pos, cursor_display_col) = dlm.locate(cursor_char);
    if cursor_display_col < v.horizontal_offset {
        return None;
    }
    let top = dlm.clamp(v.top());
    let row = dlm.distance(top, cursor_pos, v.height as usize - 1)?;
    Some(local_place(v, cursor_display_col, row))
}

/// Mirror of `hume-editor`'s `cursor::place` — see this module's doc.
fn local_place(v: &Viewport, cursor_display_col: DisplayLineCol, row: usize) -> (u16, u16) {
    let x = cursor_display_col.cells_since_saturating(v.horizontal_offset);
    (x as u16, row as u16)
}

// ---------------------------------------------------------------------------
// Test doubles
// ---------------------------------------------------------------------------

/// What each of a [`VirtualLineBlock`]'s display lines says.
enum BlockText {
    Same(&'static str),
    Ordinal,
}

/// A VIRTUAL_LINE source emitting `count` display lines at one anchor, and
/// nothing for any other line.
struct VirtualLineBlock {
    anchor: VirtualLineAnchor,
    count: usize,
    text: BlockText,
}

impl VirtualLineBlock {
    fn uniform(anchor: VirtualLineAnchor, count: usize, text: &'static str) -> Self {
        Self {
            anchor,
            count,
            text: BlockText::Same(text),
        }
    }

    fn numbered(anchor: VirtualLineAnchor, count: usize) -> Self {
        Self {
            anchor,
            count,
            text: BlockText::Ordinal,
        }
    }

    fn line(&self) -> ContentLine {
        match self.anchor {
            VirtualLineAnchor::Before(n) | VirtualLineAnchor::After(n) => n,
        }
    }
}

impl DecorationSource for VirtualLineBlock {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }

    fn decorations_for_line(&self, line_idx: ContentLine, out: &mut Vec<Decoration>) {
        if line_idx != self.line() {
            return;
        }
        for i in 0..self.count {
            out.push(Decoration::VirtualLine(VirtualLine {
                anchor: self.anchor,
                provider_id: 0,
                text: match self.text {
                    BlockText::Same(t) => t.to_string(),
                    BlockText::Ordinal => (i + 1).to_string(),
                },
                segments: Vec::new(),
                base_scope: None,
            }));
        }
    }
}

/// Counts how many times `line` is formatted, and decorates nothing.
struct FormatProbe {
    line: ContentLine,
    formats: Rc<Cell<usize>>,
}

impl FormatProbe {
    fn new(line: usize, formats: Rc<Cell<usize>>) -> Self {
        Self {
            line: ContentLine::new(line),
            formats,
        }
    }
}

impl DecorationSource for FormatProbe {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::INLINE
    }

    fn decorations_for_line(&self, line_idx: ContentLine, _out: &mut Vec<Decoration>) {
        if line_idx == self.line {
            self.formats.set(self.formats.get() + 1);
        }
    }
}

fn no_providers() -> ProviderSet {
    ProviderSet::new()
}

fn providers_with_before_line(line: usize) -> ProviderSet {
    let mut p = ProviderSet::new();
    p.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        VirtualLineAnchor::Before(ContentLine::new(line)),
        1,
        "V",
    )));
    p
}

// ── reveal (no-wrap) ──────────────────────────────────────────────────────

#[test]
fn no_wrap_cursor_visible_no_scroll_needed() {
    let r = rope("a\nb\nc\nd\ne\n");
    let mut v = viewport(0, 10, 80);
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let geo = v.geometry(3).unwrap();
    v.reveal(&mut dlm, geo, DisplayLinePos::new(ContentLine::new(2), 0));
    assert_eq!(v.top().line, ContentLine::new(0));
}

#[test]
fn no_wrap_cursor_below_viewport_scrolls_down() {
    let r = rope("a\nb\nc\nd\ne\nf\ng\nh\n");
    let mut v = viewport(0, 5, 80);
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let geo = v.geometry(3).unwrap();
    v.reveal(&mut dlm, geo, DisplayLinePos::new(ContentLine::new(7), 0));
    let cursor_line = 7usize;
    assert!(cursor_line >= v.top().line.index());
    assert!(cursor_line < v.top().line.index() + v.height as usize);
}

#[test]
fn no_wrap_cursor_above_viewport_scrolls_up() {
    let r = rope("a\nb\nc\nd\ne\nf\ng\nh\n");
    let mut v = viewport(5, 5, 80);
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let geo = v.geometry(3).unwrap();
    v.reveal(&mut dlm, geo, DisplayLinePos::new(ContentLine::new(1), 0));
    let cursor_line = 1usize;
    assert!(cursor_line >= v.top().line.index());
    assert!(cursor_line < v.top().line.index() + v.height as usize);
}

/// A `scrolloff` at or above half the viewport height (`:set scrolloff=999`'s
/// "always center" idiom, at an even height) used to leave the "no scroll
/// needed" window empty: the two correction arms disagreed about where the
/// cursor should land and rescrolled every single frame. Calling `reveal`
/// again with the cursor unmoved must be a no-op — it wasn't, before capping
/// the margin at `(height - 1) / 2`.
#[test]
fn no_wrap_huge_scrolloff_at_even_height_settles_after_one_scroll() {
    let text: String = (0..50).map(|i| format!("line{i}\n")).collect();
    let r = rope(&text);
    let mut v = viewport(0, 24, 80);
    let providers = no_providers();
    let cursor_pos = DisplayLinePos::new(ContentLine::new(20), 0);
    let geo = v.geometry(999).unwrap();

    let mut s = PaneLineStore::new();
    v.reveal(
        &mut map(&r, WrapMode::None, &providers, 80, &mut s),
        geo,
        cursor_pos,
    );
    let top_after_first = v.top();

    let mut s = PaneLineStore::new();
    v.reveal(
        &mut map(&r, WrapMode::None, &providers, 80, &mut s),
        geo,
        cursor_pos,
    );

    assert_eq!(
        v.top(),
        top_after_first,
        "reveal must be a fixed point once the cursor is already visible"
    );
}

// ── cursor sub-row ───────────────────────────────────────────────────────

#[test]
fn cursor_sub_display_line_no_wrap() {
    let r = rope("hello world\n");
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let sub = dlm.locate(co(5)).0.slot;
    assert_eq!(sub, 0);
}

#[test]
fn cursor_sub_display_line_wrapped() {
    let r = rope("abcdefgh\n");
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::Soft { width: 4 }, &providers, 80, &mut s);
    assert_eq!(dlm.locate(co(0)).0.slot, 0);
    assert_eq!(dlm.locate(co(4)).0.slot, 1);
}

// ── reveal (wrap) top/bottom margin enforcement ──────────────────────────

#[test]
fn wrap_cursor_within_top_margin_scrolls_up() {
    let r = rope(&"ab\n".repeat(10));
    let mut v = Viewport::new(3, 8);
    v.seed_top_for_test(DisplayLinePos::new(ContentLine::new(3), 0));
    let cursor_char =
        co(hume_rope::lines::line_start_char(&r, hume_rope::line::RopeyLine::new(3)).index());
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::Soft { width: 3 }, &providers, 3, &mut s);
    let cursor_pos = dlm.locate_display_line(cursor_char);
    let geo = v.geometry(2).unwrap();
    v.reveal(&mut dlm, geo, cursor_pos);
    assert_eq!(v.top(), DisplayLinePos::new(ContentLine::new(1), 0));
}

#[test]
fn wrap_cursor_within_bottom_margin_scrolls_down() {
    let r = rope(&"ab\n".repeat(10));
    let mut v = Viewport::new(3, 8);
    v.seed_top_for_test(DisplayLinePos::new(ContentLine::new(0), 0));
    let cursor_char =
        co(hume_rope::lines::line_start_char(&r, hume_rope::line::RopeyLine::new(7)).index());
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::Soft { width: 3 }, &providers, 3, &mut s);
    let cursor_pos = dlm.locate_display_line(cursor_char);
    let geo = v.geometry(2).unwrap();
    v.reveal(&mut dlm, geo, cursor_pos);
    assert_eq!(v.top(), DisplayLinePos::new(ContentLine::new(2), 0));
}

// ── align (z z / z k / z j) with scrolloff ────────────────────────────────

#[test]
fn view_top_then_scrolloff_trims_cursor_inward() {
    let r = rope(&"a\n".repeat(50));
    let mut v = viewport(0, 24, 80);
    let cursor_char =
        co(hume_rope::lines::line_start_char(&r, hume_rope::line::RopeyLine::new(25)).index());
    let providers = no_providers();

    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let cursor_pos = dlm.locate_display_line(cursor_char);
    let geo = v.geometry(3).unwrap();
    v.align(&mut dlm, geo, cursor_pos, 0);
    assert_eq!(
        v.top().line,
        ContentLine::new(22),
        "scrolloff trims top inward by margin (3)"
    );
}

#[test]
fn view_bottom_then_scrolloff_trims_cursor_inward() {
    let r = rope(&"a\n".repeat(50));
    let mut v = viewport(0, 24, 80);
    let cursor_char =
        co(hume_rope::lines::line_start_char(&r, hume_rope::line::RopeyLine::new(25)).index());
    let providers = no_providers();

    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let cursor_pos = dlm.locate_display_line(cursor_char);
    let geo = v.geometry(3).unwrap();
    v.align(&mut dlm, geo, cursor_pos, 23);
    assert_eq!(
        v.top().line,
        ContentLine::new(5),
        "scrolloff trims top up by margin (3)"
    );
}

// ── Virtual-line-aware scrolling (synthetic provider) ────────────────────

#[test]
fn reveal_accounts_for_a_stolen_virtual_display_line() {
    let r = rope("a\nb\nc\nd\n");
    let mut v = viewport(0, 2, 80);
    let wrap = WrapMode::Soft { width: 80 };
    let providers = providers_with_before_line(2);
    let cursor_char =
        co(hume_rope::lines::line_start_char(&r, hume_rope::line::RopeyLine::new(3)).index());

    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, wrap, &providers, 80, &mut s);
    let cursor_pos = dlm.locate_display_line(cursor_char);
    let geo = v.geometry(0).unwrap();
    v.reveal(&mut dlm, geo, cursor_pos);

    let mut s = PaneLineStore::new();
    let pos = local_content_pos(&v, &mut map(&r, wrap, &providers, 80, &mut s), cursor_char);
    let (_, row) = pos.expect("cursor must be visible after reveal");
    assert!(
        (row as usize) < v.height as usize,
        "cursor row {row} must be inside the {}-row viewport",
        v.height
    );
}

#[test]
fn reveal_accounts_for_a_stolen_virtual_display_line_no_wrap() {
    let r = rope("a\nb\nc\nd\n");
    let mut v = viewport(0, 2, 80);
    let wrap = WrapMode::None;
    let providers = providers_with_before_line(2);
    let cursor_char =
        co(hume_rope::lines::line_start_char(&r, hume_rope::line::RopeyLine::new(3)).index());

    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, wrap, &providers, 80, &mut s);
    let cursor_pos = dlm.locate_display_line(cursor_char);
    let geo = v.geometry(0).unwrap();
    v.reveal(&mut dlm, geo, cursor_pos);

    let mut s = PaneLineStore::new();
    let pos = local_content_pos(&v, &mut map(&r, wrap, &providers, 80, &mut s), cursor_char);
    let (_, row) = pos.expect("cursor must be visible after reveal");
    assert!(
        (row as usize) < v.height as usize,
        "cursor row {row} must be inside the {}-row viewport, no-wrap too",
        v.height
    );
}

#[test]
fn scroll_backward_from_cursor_reaches_into_before_line_0() {
    let r = rope("a\nb\nc\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(VirtualLineBlock::numbered(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        3,
    )));
    let cursor_char =
        co(hume_rope::lines::line_start_char(&r, hume_rope::line::RopeyLine::new(2)).index());

    for wrap in [WrapMode::None, WrapMode::Soft { width: 80 }] {
        let mut v = viewport(2, 20, 80);
        let mut s = PaneLineStore::new();
        let mut dlm = map(&r, wrap, &providers, 80, &mut s);
        let cursor_pos = dlm.locate_display_line(cursor_char);
        let geo = v.geometry(20).unwrap();
        v.reveal(&mut dlm, geo, cursor_pos);
        assert_eq!(
            v.top(),
            DisplayLinePos::new(ContentLine::new(0), 0),
            "walk reaches the first display line of Before(0)'s block ({wrap:?})"
        );
    }
}

/// [`Viewport::heal`] must shrink an out-of-range offset (as `recall_scroll`
/// or an LSP jump could leave behind) down to the top line's actual current
/// block size, in either wrap mode.
#[test]
fn heal_shrinks_stale_offset() {
    let r = rope("a\nb\n");
    let providers = providers_with_before_line(0); // Before(0): 1 display line + content: 1 = total 2

    for wrap in [WrapMode::None, WrapMode::Soft { width: 80 }] {
        let mut v = viewport(0, 5, 80);
        v.seed_top_for_test(DisplayLinePos::new(ContentLine::new(0), 200)); // wildly stale
        let mut s = PaneLineStore::new();
        v.heal(&mut map(&r, wrap, &providers, 80, &mut s));
        assert_eq!(
            v.top().slot,
            1,
            "clamped to the block's last valid display line (total 2, so max offset 1) ({wrap:?})"
        );
    }
}

#[test]
fn heal_is_a_noop_when_already_valid() {
    let r = rope("a\nb\n");
    let providers = providers_with_before_line(0);
    let mut v = viewport(0, 5, 80);
    v.seed_top_for_test(DisplayLinePos::new(ContentLine::new(0), 1));
    let mut s = PaneLineStore::new();
    v.heal(&mut map(&r, WrapMode::None, &providers, 80, &mut s));
    assert_eq!(v.top().slot, 1);
}

// ── reveal_horizontal ─────────────────────────────────────────────────────

#[test]
fn horizontal_scroll_margin_uses_content_width_not_viewport_width() {
    let r = rope(&("a".repeat(100) + "\n"));
    let mut v = viewport(0, 10, 80);
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let cursor_char = co(70);

    let mut dlm = map(&r, WrapMode::None, &providers, 72, &mut s);
    let cursor_display_col = dlm.locate(cursor_char).1;
    v.reveal_horizontal(&mut dlm, cursor_display_col);

    assert_eq!(
        v.horizontal_offset,
        DisplayLineCol::new(4),
        "cursor_display_col(70) - (content_width(72) - margin(5) - 1) = 4"
    );
}

#[test]
fn horizontal_scroll_margin_no_scroll_when_within_content_width() {
    let r = rope(&("a".repeat(100) + "\n"));
    let mut v = viewport(0, 10, 80);
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let cursor_char = co(70);

    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let cursor_display_col = dlm.locate(cursor_char).1;
    v.reveal_horizontal(&mut dlm, cursor_display_col);

    assert_eq!(
        v.horizontal_offset,
        DisplayLineCol::new(0),
        "70 < content_width(80) - margin(5)"
    );
}

/// A cursor past column 65535 on a huge unwrapped line must scroll to its
/// true (unclamped) column, not a `u16`-truncated one.
#[test]
fn horizontal_scroll_reaches_past_former_u16_column_ceiling() {
    let r = rope(&("a".repeat(70_000) + "\n"));
    let mut v = viewport(0, 10, 80);
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let cursor_char = co(69_999);

    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let cursor_display_col = dlm.locate(cursor_char).1;
    v.reveal_horizontal(&mut dlm, cursor_display_col);

    assert_eq!(
        v.horizontal_offset,
        DisplayLineCol::new(69_925),
        "cursor_display_col(69_999) - (content_width(80) - margin(5) - 1) = 69_925"
    );
    assert!(
        v.horizontal_offset.get() > u16::MAX as u32,
        "offset must exceed the former u16 ceiling, not wrap/truncate into it"
    );
}

// ── One cursor resolution per frame ──────────────────────────────────────

#[test]
fn reported_screen_row_agrees_with_a_forward_walk() {
    let r = rope("a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(VirtualLineBlock::numbered(
        VirtualLineAnchor::Before(ContentLine::new(3)),
        2,
    )));

    for wrap in [WrapMode::None, WrapMode::Soft { width: 80 }] {
        for height in [1u16, 2, 5, 8] {
            for top in [0usize, 2, 5, 9] {
                for line in
                    (0..hume_rope::lines::content_line_count(&r).get()).map(ContentLine::new)
                {
                    let cursor_char =
                        co(hume_rope::lines::line_start_char(&r, line.into()).index());
                    let mut v = viewport(top, height, 80);

                    let mut s = PaneLineStore::new();
                    let mut dlm = map(&r, wrap, &providers, 80, &mut s);
                    v.heal(&mut dlm);
                    let cursor_pos = dlm.locate_display_line(cursor_char);
                    let geo = v.geometry(2).unwrap();
                    let reported = v.reveal(&mut dlm, geo, cursor_pos);

                    let mut s = PaneLineStore::new();
                    let walked = local_content_pos(
                        &v,
                        &mut map(&r, wrap, &providers, 80, &mut s),
                        cursor_char,
                    );
                    assert_eq!(
                        Some(reported as u16),
                        walked.map(|(_, row)| row),
                        "{wrap:?}, height {height}, top {top}, line {}",
                        line.index()
                    );
                }
            }
        }
    }
}

#[test]
fn a_frame_formats_the_cursors_line_once_in_no_wrap() {
    let r = rope(&("a".repeat(5_000) + "\n"));
    let formats = Rc::new(Cell::new(0));
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FormatProbe::new(0, Rc::clone(&formats))));
    let mut v = viewport(0, 10, 80);
    let cursor_char = co(4_000);

    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    v.heal(&mut dlm);
    let (cursor_pos, cursor_display_col) = dlm.locate(cursor_char);
    let geo = v.geometry(3).unwrap();
    let row = v.reveal(&mut dlm, geo, cursor_pos);
    v.reveal_horizontal(&mut dlm, cursor_display_col);
    let placed = local_place(&v, cursor_display_col, row);

    assert_eq!(
        formats.get(),
        1,
        "the scroll step must resolve the cursor with a single format"
    );

    let mut s = PaneLineStore::new();
    let walked = local_content_pos(
        &v,
        &mut map(&r, WrapMode::None, &providers, 80, &mut s),
        cursor_char,
    );
    assert_eq!(
        Some(placed),
        walked,
        "placement must match a full re-derivation"
    );
}

// ── `distance`'s tightened cap ────────────────────────────────────────────

#[test]
fn far_jump_lands_at_the_same_top_as_before_the_cap_change() {
    let text: String = (0..150).map(|i| format!("line{i}\n")).collect();
    let r = rope(&text);
    let providers = no_providers();
    let mut v = viewport(0, 10, 80);
    let height = 10usize;
    let margin = 2usize;
    let cursor_line = 100;

    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::Soft { width: 80 }, &providers, 80, &mut s);
    let geo = v.geometry(margin).unwrap();
    let row = v.reveal(
        &mut dlm,
        geo,
        DisplayLinePos::new(ContentLine::new(cursor_line), 0),
    );

    let target = height - margin - 1;
    assert_eq!(v.top().line, ContentLine::new(cursor_line - target));
    assert_eq!(row, target);
}

#[test]
fn far_jump_forward_walk_does_not_format_past_the_tightened_cap() {
    let text: String = (0..150).map(|i| format!("line{i}\n")).collect();
    let r = rope(&text);
    let formats = Rc::new(Cell::new(0));
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FormatProbe::new(9, Rc::clone(&formats))));
    let mut v = viewport(0, 10, 80);

    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::Soft { width: 80 }, &providers, 80, &mut s);
    let geo = v.geometry(2).unwrap();
    v.reveal(&mut dlm, geo, DisplayLinePos::new(ContentLine::new(100), 0));

    assert_eq!(
        formats.get(),
        0,
        "line 9 sits past the tightened cap (height - margin - 1 = 7) but \
         inside the old cap (height = 10) — the forward walk must not reach it"
    );
}

#[test]
fn distance_line_delta_short_circuit_never_formats_when_unreachable() {
    let text: String = (0..1200).map(|i| format!("line{i}\n")).collect();
    let r = rope(&text);
    let formats = Rc::new(Cell::new(0));
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FormatProbe::new(0, Rc::clone(&formats))));

    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::Soft { width: 80 }, &providers, 80, &mut s);
    let result = dlm.distance(
        DisplayLinePos::new(ContentLine::new(0), 0),
        DisplayLinePos::new(ContentLine::new(1000), 0),
        5,
    );

    assert_eq!(result, None);
    assert_eq!(
        formats.get(),
        0,
        "a target 1000 lines away with cap 5 is provably unreachable — the \
         walk must never format line 0 to discover that"
    );
}

// ── scroll_by (viewport-only scroll, no cursor) ──────────────────────────

#[test]
fn down_no_wrap_clamps_so_the_last_real_line_reaches_the_bottom_row() {
    let r = rope(&"a\n".repeat(10));
    let mut v = viewport(0, 5, 80);
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let geo = v.geometry(0).unwrap();

    for _ in 0..20 {
        let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
        v.scroll_by(&mut dlm, geo, 3);
    }
    assert_eq!(
        v.top(),
        DisplayLinePos::new(ContentLine::new(5), 0),
        "top must stop where the last real line (9) reaches the bottom row"
    );
}

#[test]
fn down_no_wrap_file_fits_no_movement() {
    let r = rope(&"a\n".repeat(3));
    let mut v = viewport(0, 10, 80);
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let geo = v.geometry(0).unwrap();
    v.scroll_by(&mut dlm, geo, 3);
    assert_eq!(
        v.top(),
        DisplayLinePos::new(ContentLine::new(0), 0),
        "viewport must not move when the file already fits"
    );
}

#[test]
fn down_no_wrap_advances_by_count() {
    let r = rope(&"a\n".repeat(20));
    let mut v = viewport(0, 5, 80);
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let geo = v.geometry(0).unwrap();
    v.scroll_by(&mut dlm, geo, 3);
    assert_eq!(
        v.top().line,
        ContentLine::new(3),
        "first scroll advances by count, well short of the bound here"
    );
}

#[test]
fn down_never_moves_the_top_backwards_from_past_max_scroll_top() {
    // `align` (z z / z k / z j) deliberately does not clamp at the bottom —
    // see its own doc — so it, and an LSP goto near EOF, can leave the top
    // past max_scroll_top. 10 content lines, height 5, margin 0: the bound
    // is line 5. Seed the top at line 8, well past it.
    let r = rope(&"a\n".repeat(10));
    let mut v = viewport(8, 5, 80);
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let geo = v.geometry(0).unwrap();
    v.scroll_by(&mut dlm, geo, 3);
    assert_eq!(
        v.top().line,
        ContentLine::new(8),
        "a downward scroll must never move the top backwards, even past the bound"
    );
}

#[test]
fn up_no_wrap_clamps_at_zero() {
    let r = rope(&"a\n".repeat(10));
    let mut v = viewport(1, 5, 80);
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let geo = v.geometry(0).unwrap();
    v.scroll_by(&mut dlm, geo, -3);
    assert_eq!(
        v.top().line,
        ContentLine::new(0),
        "stepping back must not underflow"
    );
}

#[test]
fn up_no_wrap_decrements_by_count() {
    let r = rope(&"a\n".repeat(20));
    let mut v = viewport(10, 5, 80);
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let geo = v.geometry(0).unwrap();
    v.scroll_by(&mut dlm, geo, -3);
    assert_eq!(v.top().line, ContentLine::new(10 - 3));
}

#[test]
fn up_at_top_is_no_op() {
    let r = rope(&"a\n".repeat(10));
    let mut v = viewport(0, 5, 80);
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let geo = v.geometry(0).unwrap();
    v.scroll_by(&mut dlm, geo, -3);
    assert_eq!(v.top(), DisplayLinePos::new(ContentLine::new(0), 0));
}

#[test]
fn down_wrap_file_fits_no_movement() {
    let r = rope(&"a\n".repeat(2));
    let mut v = viewport(0, 10, 80);
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::Soft { width: 80 }, &providers, 80, &mut s);
    let geo = v.geometry(0).unwrap();
    v.scroll_by(&mut dlm, geo, 3);
    assert_eq!(
        v.top(),
        DisplayLinePos::new(ContentLine::new(0), 0),
        "no scroll when file fits in viewport"
    );
}

/// A 3-row `After(last_line)` block, viewport height 2. One-row-at-a-time
/// notches must reach every row of the block up to the point where its last
/// row sits on the bottom screen row, and then stay there.
#[test]
fn down_reaches_the_after_last_line_block_until_it_fills_the_bottom_row() {
    let r = rope(&"a\n".repeat(2));
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(VirtualLineBlock::numbered(
        VirtualLineAnchor::After(ContentLine::new(1)),
        3,
    )));
    let mut s = PaneLineStore::new();

    for wrap in [WrapMode::None, WrapMode::Soft { width: 80 }] {
        let mut v = viewport(0, 2, 80);
        let geo = v.geometry(0).unwrap();
        let expected = [(0, 0), (1, 0), (1, 1), (1, 2)];
        for &(exp_line, exp_offset) in &expected {
            assert_eq!(
                v.top(),
                DisplayLinePos::new(ContentLine::new(exp_line), exp_offset),
                "{wrap:?}"
            );
            let mut dlm = map(&r, wrap, &providers, 80, &mut s);
            v.scroll_by(&mut dlm, geo, 1);
        }
        // One more notch past the point where the block's last row reaches
        // the bottom must stay clamped there, not advance further.
        let mut dlm = map(&r, wrap, &providers, 80, &mut s);
        v.scroll_by(&mut dlm, geo, 1);
        assert_eq!(
            v.top(),
            DisplayLinePos::new(ContentLine::new(1), 2),
            "further scrolling must stay clamped once the block's last row fills the bottom row ({wrap:?})"
        );
    }
}

/// An overshooting notch (larger than what remains of the last line's block)
/// must clamp to the bound in one jump, not reset to 0.
#[test]
fn down_overshoot_past_after_last_line_clamps_not_resets() {
    let r = rope(&"a\n".repeat(2));
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(VirtualLineBlock::numbered(
        VirtualLineAnchor::After(ContentLine::new(1)),
        3,
    )));
    let mut s = PaneLineStore::new();

    for wrap in [WrapMode::None, WrapMode::Soft { width: 80 }] {
        let mut v = viewport(0, 2, 80);
        v.seed_top_for_test(DisplayLinePos::new(ContentLine::new(1), 1)); // already partway into the After block
        let mut dlm = map(&r, wrap, &providers, 80, &mut s);
        let geo = v.geometry(0).unwrap();
        v.scroll_by(&mut dlm, geo, 10);
        assert_eq!(v.top().line, ContentLine::new(1), "{wrap:?}");
        assert_eq!(
            v.top().slot,
            2,
            "must clamp to the bound, not reset to 0 ({wrap:?})"
        );
    }
}

/// A nonzero margin reserves that many display lines below the document's
/// last one, instead of pinning it to the bottom row — the same bound
/// `Viewport::reveal` settles on once the cursor reaches the document's end,
/// so a scroll-to-EOF and the very next ordinary cursor motion land on the
/// same top.
#[test]
fn down_with_margin_stops_short_of_the_bottom_row() {
    let r = rope(&"a\n".repeat(10));
    let mut v = viewport(0, 5, 80);
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let geo = v.geometry(2).unwrap();

    for _ in 0..20 {
        let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
        v.scroll_by(&mut dlm, geo, 3);
    }
    assert_eq!(
        v.top().line,
        ContentLine::new(7),
        "line 9 (the last) must land 2 rows above the bottom (row 2 of 0..4), \
         not on the bottom row itself"
    );
}

// ── carry ─────────────────────────────────────────────────────────────────
//
// `top`/`geo` in most of these are a generously tall, zero-scrolloff
// viewport (`margin == 0`, `target == height - 1`) seeded at the document
// start — big enough that the band clamp never fires, so these still
// exercise exactly the plain delta-walk they did before `carry` gained the
// band clamp. The clamp itself gets its own tests below.

fn generous_band() -> (ViewGeometry, DisplayLinePos) {
    let v = viewport(0, 25, 80);
    (v.geometry(0).unwrap(), v.top())
}

#[test]
fn carry_moves_head_the_requested_display_lines_down() {
    let r = rope(&"a\n".repeat(10));
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let head = DisplayLinePos::new(ContentLine::new(2), 0);
    let (geo, top) = generous_band();
    let landed = carry(&mut dlm, geo, top, head, 3).expect("content lines the whole way");
    assert_eq!(landed, DisplayLinePos::new(ContentLine::new(5), 0));
}

#[test]
fn carry_returns_none_when_head_is_already_at_the_document_edge() {
    let r = rope(&"a\n".repeat(3));
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let head = DisplayLinePos::new(ContentLine::new(0), 0);
    let (geo, top) = generous_band();
    assert_eq!(
        carry(&mut dlm, geo, top, head, -1),
        None,
        "no display line precedes the document start"
    );
}

#[test]
fn carry_zero_rows_is_always_none() {
    let r = rope("a\n");
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let head = DisplayLinePos::new(ContentLine::new(0), 0);
    let (geo, top) = generous_band();
    assert_eq!(carry(&mut dlm, geo, top, head, 0), None);
}

/// Line 0 content, then a 3-row `Before(1)` block whose slots 0..=2 precede
/// line 1's own content at slot 3. Walking 2 display lines down from line 0
/// lands inside the block (slot 1, still virtual); `carry` must keep walking
/// past it to line 1's content (slot 3) rather than stranding the head at
/// the block's near edge (see `carry`'s own doc) — as long as doing so still
/// lands inside a generous band; the band-bounded version of this same setup
/// is `carry_overshoot_past_the_band_gives_up_instead_of_landing_outside_it`
/// below.
#[test]
fn carry_overshoots_a_virtual_block_that_swallows_the_whole_budget() {
    let r = rope("a\nb\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(VirtualLineBlock::numbered(
        VirtualLineAnchor::Before(ContentLine::new(1)),
        3,
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let head = DisplayLinePos::new(ContentLine::new(0), 0);
    let (geo, top) = generous_band();
    let landed =
        carry(&mut dlm, geo, top, head, 2).expect("line 1's content is reachable past the block");
    assert_eq!(landed, DisplayLinePos::new(ContentLine::new(1), 3));
}

// ── carry's band clamp ──────────────────────────────────────────────────────

/// A landing above `geo.margin` (too close to `top`, or before it) must be
/// pushed down to the band's near edge — the case that makes `Viewport::reveal`
/// provably idle afterward instead of firing a second, undocumented
/// correction next frame. `top` and `head` both start at the document's
/// first line (the common "freshly opened file" state), height 10 with
/// scrolloff 3 (`margin` 3, `target` 6): walking down 2 display lines lands
/// on row 2, short of `margin`, so the clamp must walk it 1 further, to row 3.
#[test]
fn carry_pushes_a_landing_above_margin_down_to_the_bands_near_edge() {
    let r = rope(&"a\n".repeat(10));
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let v = viewport(0, 10, 80);
    let geo = v.geometry(3).unwrap();
    let top = v.top();
    let head = top;
    let landed = carry(&mut dlm, geo, top, head, 2).expect("plenty of content lines below top");
    assert_eq!(
        landed,
        DisplayLinePos::new(ContentLine::new(3), 0),
        "clamped down to margin (3), not left at row 2"
    );
}

/// A landing past `geo.target` (a big enough `delta`, no virtual lines
/// involved) must be pulled back up to the band's far edge. Height 10,
/// scrolloff 3 (`target` 6): walking down 8 real content lines from top
/// lands on row 8, past `target`, so the clamp must pull it back to row 6.
#[test]
fn carry_pulls_a_landing_past_target_back_to_the_bands_far_edge() {
    let r = rope(&"a\n".repeat(20));
    let providers = no_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let v = viewport(0, 10, 80);
    let geo = v.geometry(3).unwrap();
    let top = v.top();
    let head = top;
    let landed = carry(&mut dlm, geo, top, head, 8).expect("plenty of content lines below top");
    assert_eq!(
        landed,
        DisplayLinePos::new(ContentLine::new(6), 0),
        "pulled back to target (6), not left at row 8"
    );
}

/// A virtual-line block bigger than the band has no legal landing spot at
/// all — `carry` must give up (leave the selection untouched) rather than
/// land past `target`, the band-bounded counterpart to
/// `carry_overshoots_a_virtual_block_that_swallows_the_whole_budget` above.
/// Height 6, scrolloff 0 (`margin` 0, `target` 5): an 8-row `Before(1)`
/// block swallows every display line through row 8, past `target`.
#[test]
fn carry_overshoot_past_the_band_gives_up_instead_of_landing_outside_it() {
    let r = rope("a\nb\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(VirtualLineBlock::numbered(
        VirtualLineAnchor::Before(ContentLine::new(1)),
        8,
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let v = viewport(0, 6, 80);
    let geo = v.geometry(0).unwrap();
    let top = v.top();
    let head = top;
    assert_eq!(
        carry(&mut dlm, geo, top, head, 1),
        None,
        "the block outlasts the band, so there is nowhere in it to land"
    );
}
