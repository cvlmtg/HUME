//! `block()`/`slot()`/`clamp()`/`last_line()` tests.

use super::*;

/// A LINE_BG-kind source that counts every `decorations_for_line` call —
/// used to prove the layout stage (`block`/`ensure_formatted`) never queries a
/// kind it has no use for, even when the same line is both counted and
/// formatted.
struct CountingLineBg(Rc<Cell<usize>>);

impl DecorationSource for CountingLineBg {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::LINE_BG
    }
    fn decorations_for_line(&self, _line_idx: ContentLine, _out: &mut Vec<Decoration>) {
        self.0.set(self.0.get() + 1);
    }
}

// ---------------------------------------------------------------------------
// block()
// ---------------------------------------------------------------------------

#[test]
fn block_without_providers_is_content_only() {
    let rope = Rope::from_str("hello\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.block(ContentLine::new(0)),
        BlockBreakdown {
            before: 0,
            content: 1,
            after: 0,
        }
    );
}

#[test]
fn block_counts_one_content_display_line_per_wrap_display_line() {
    // "abcdefgh" is 8 columns wrapped at 4 → "abcd" / "efgh" = 2 display lines, and
    // "efgh" exactly fills its display line, so the trailing '\n's own sentinel wraps
    // onto a third display line rather than landing past the pane's edge.
    let rope = Rope::from_str("abcdefgh\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(dlm.block(ContentLine::new(0)).content, 3);
}

#[test]
fn block_counts_before_and_after_virtual_lines() {
    // Line 5 of "a\nb\nc\nd\ne\nf\n" gets 2 Before display lines and 1 After display line from
    // two separate providers, on top of its own single unwrapped content display line.
    let rope = Rope::from_str("a\nb\nc\nd\ne\nf\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::Before(ContentLine::new(5)),
        2,
    )));
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::After(ContentLine::new(5)),
        1,
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    let block = dlm.block(ContentLine::new(5));
    assert_eq!(
        block,
        BlockBreakdown {
            before: 2,
            content: 1,
            after: 1,
        }
    );
    assert_eq!(block.total(), 4);
}

#[test]
fn block_ignores_virtual_lines_anchored_to_other_lines() {
    let rope = Rope::from_str("a\nb\nc\nd\ne\nf\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::Before(ContentLine::new(2)),
        3,
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(dlm.block(ContentLine::new(5)).before, 0);
    assert_eq!(dlm.block(ContentLine::new(5)).after, 0);
}

#[test]
fn layout_stage_never_queries_a_paint_only_kind() {
    // block() drives both the VIRTUAL_LINE query and, under wrapping,
    // ensure_formatted()'s INLINE query — a LINE_BG-kind source (paint-only)
    // must be invisible to both.
    let calls = Rc::new(Cell::new(0));
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(CountingLineBg(Rc::clone(&calls))));
    let rope = Rope::from_str("abcdef\n");
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    dlm.block(ContentLine::new(0));
    assert_eq!(
        calls.get(),
        0,
        "block() must never query a LINE_BG-only source"
    );
}

#[test]
fn block_counts_inline_inserts_toward_wrapping() {
    // The bug this fixes: an inlay hint participates in wrapping, so a line
    // that fits without it can need two display lines with it, and display line counting that
    // ignores inserts disagrees with what the renderer emits.
    //
    // "abcdef" is 6 columns; wrapped at 8 that is one display line. A 4-column insert
    // at the head of the line makes 10 columns, which is two.
    let rope = Rope::from_str("abcdef\n");
    let wrap = WrapMode::Soft { width: 8 };

    let bare = ProviderSet::new();
    let mut s = PaneLineStore::new();
    assert_eq!(
        map(&rope, wrap, &bare, &mut s)
            .block(ContentLine::new(0))
            .content,
        1,
        "6 columns fit on one display line at width 8"
    );

    let (providers, _calls) = with_counting_insert(ContentLine::new(0), 0, "hint");
    let mut s = PaneLineStore::new();
    assert_eq!(
        map(&rope, wrap, &providers, &mut s)
            .block(ContentLine::new(0))
            .content,
        2,
        "4 columns of inlay hint push the line's 6 columns past width 8"
    );
}

#[test]
fn no_wrap_block_counts_without_running_the_formatter() {
    // `WrapMode::None` is always one content display line, so counting must not format
    // — that is what keeps a display line query O(1) instead of O(line length) on a
    // minified line megabytes wide. Querying decorations is what formatting
    // does first, so a zero call count is the observable proxy.
    let rope = Rope::from_str("abcdef\n");
    let (providers, calls) = with_counting_insert(ContentLine::new(0), 0, "hint");
    let mut s = PaneLineStore::new();

    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);
    assert_eq!(dlm.block(ContentLine::new(0)).content, 1);
    assert_eq!(calls.get(), 0, "no-wrap counting must not format");

    // The same query while wrapping has to format, because the display line count
    // genuinely depends on the content.
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 8 }, &providers, &mut s);
    dlm.block(ContentLine::new(0));
    assert!(
        calls.get() > 0,
        "wrapping must format to count display lines"
    );
}

// ---------------------------------------------------------------------------
// slot() / clamp() / last_line()
// ---------------------------------------------------------------------------

/// Line 0 wrapped into 2 content display lines, with 2 Before display lines and 1 After display line:
/// a 5-display-line block whose every display line slot is hand-known.
fn mixed_block_providers() -> ProviderSet {
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        2,
    )));
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::After(ContentLine::new(0)),
        1,
    )));
    providers
}

#[test]
fn slot_classifies_every_display_line_of_a_mixed_block() {
    // "abcdefgh\n" at width 4 supplies 3 content display lines, not 2: "efgh" exactly
    // fills the wrap width, so the trailing '\n's own sentinel wraps onto a
    // display line of its own (see `content_display_line_char_bounds_scopes_to_one_wrap_display_line`).
    let rope = Rope::from_str("abcdefgh\n");
    let providers = mixed_block_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(dlm.block(ContentLine::new(0)).total(), 6);
    let slots: Vec<BlockSlot> = (0..6)
        .map(|slot_idx| dlm.slot(DisplayLinePos::new(ContentLine::new(0), slot_idx)))
        .collect();
    assert_eq!(
        slots,
        vec![
            BlockSlot::Before(0),
            BlockSlot::Before(1),
            BlockSlot::Content(0),
            BlockSlot::Content(1),
            BlockSlot::Content(2),
            BlockSlot::After(0),
        ]
    );
}

#[test]
fn clamp_pulls_line_and_slot_into_the_document() {
    let rope = Rope::from_str("abcdefgh\n");
    let providers = mixed_block_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(
        dlm.clamp(DisplayLinePos::new(ContentLine::new(0), 99)),
        DisplayLinePos::new(ContentLine::new(0), 5),
        "display line clamps to the block's last display line"
    );
    assert_eq!(
        dlm.clamp(DisplayLinePos::new(ContentLine::new(99), 99)),
        DisplayLinePos::new(ContentLine::new(0), 5),
        "line clamps to the last real line, then display line to its block"
    );
    assert_eq!(
        dlm.clamp(DisplayLinePos::new(ContentLine::new(0), 2)),
        DisplayLinePos::new(ContentLine::new(0), 2),
        "an address already inside the document is untouched"
    );
}

#[test]
fn last_line_excludes_the_phantom_trailing_line() {
    // Every buffer ends with a structural '\n', so ropey's `len_lines()`
    // reports one extra empty line past the content; `last_line` must not
    // count it as a real line.
    let rope = Rope::from_str("a\nb\nc\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(dlm.last_line(), ContentLine::new(2));
}

#[test]
fn clamp_reaches_the_documents_very_last_display_line() {
    // "a\nb\nc\n" has 3 real lines; line 2 carries 1 After display line, so the
    // document's last display line is that virtual display line at (2, 1). `clamp` is the
    // documented way to reach it (DisplayLineMap has no dedicated accessor).
    let rope = Rope::from_str("a\nb\nc\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::After(ContentLine::new(2)),
        1,
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.clamp(DisplayLinePos::new(
            ContentLine::new(usize::MAX),
            usize::MAX
        )),
        DisplayLinePos::new(ContentLine::new(2), 1)
    );
}
