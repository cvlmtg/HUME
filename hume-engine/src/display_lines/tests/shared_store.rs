//! Shared-line-store tests (cross-map, cross-frame reuse).

use super::*;

/// A call-counting source emitting one `Before(0)` display line, so the entry whose
/// re-query the counter is watching has a non-trivial block shape.
fn with_counting_anchor() -> (ProviderSet, Rc<Cell<usize>>) {
    let calls = Rc::new(Cell::new(0));
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(
        VirtualLineBlock::uniform(VirtualLineAnchor::Before(ContentLine::new(0)), 1, "V")
            .counting(Rc::clone(&calls)),
    ));
    (providers, calls)
}

// ---------------------------------------------------------------------------
// Shared line store
// ---------------------------------------------------------------------------

/// The frame's two passes over one pane — the editor's scroll step and the
/// render pass — each build their own `DisplayLineMap`, but over the same store. Under
/// a wrapping mode each formats the lines it walks, so sharing the store must
/// collapse that to one format per line: `CountingInsert` counts INLINE
/// queries, which happen once per format and nowhere else.
#[test]
fn two_passes_over_one_pane_format_each_line_once() {
    let r = Rope::from_str("alpha\nbravo\ncharlie\ndelta\necho\n");
    let (providers, calls) = with_counting_insert(ContentLine::new(0), 0, "x");
    let mut store = PaneLineStore::new();
    let lines = 0..5;

    // Pass 1 (stands in for the scroll step).
    let mut dlm = map(&r, WrapMode::Soft { width: 80 }, &providers, &mut store);
    for line in lines.clone() {
        dlm.block(ContentLine::new(line));
    }
    drop(dlm);
    let after_first = calls.get();
    assert_eq!(after_first, 5, "the first pass formats each line once");

    // Pass 2 (stands in for the render pass): a cold `DisplayLineMap`, same store.
    let mut dlm = map(&r, WrapMode::Soft { width: 80 }, &providers, &mut store);
    for line in lines {
        dlm.block(ContentLine::new(line));
    }

    assert_eq!(
        calls.get(),
        after_first,
        "the second pass must reuse every line the first formatted"
    );
}

/// A line read back from the store must produce the display line list the formatter
/// would have. The line carries a tab and an inline insert so both the
/// `line_texts` slices (`Grapheme::byte_range`) and the `virtual_texts` arena
/// (`CellContent`'s `(start, len)`) are exercised — those offsets are
/// line-local, which is what makes an entry readable on its own.
#[test]
fn a_stored_format_reproduces_the_display_lines_it_replaced() {
    let r = Rope::from_str("ab\tcdefghij\n");
    let (providers, _) = with_counting_insert(ContentLine::new(0), 4, "HINT");
    let wrap = WrapMode::Soft { width: 6 };

    let display_lines_of =
        |dlm: &mut DisplayLineMap<'_>| -> Vec<(crate::types::DisplayLineKind, String)> {
            let total = dlm.block(ContentLine::new(0)).total();
            (0..total)
                .map(|slot_idx| {
                    let rendered =
                        dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), slot_idx));
                    (rendered.display_line.kind, display_line_text(&rendered))
                })
                .collect()
        };

    // A store that has never seen the line: the formatter runs.
    let mut fresh = PaneLineStore::new();
    let expected = display_lines_of(&mut map(&r, wrap, &providers, &mut fresh));

    // A store one pass already filled: the second reads it back.
    let mut shared = PaneLineStore::new();
    let mut warm = map(&r, wrap, &providers, &mut shared);
    warm.block(ContentLine::new(0));
    drop(warm);
    let actual = display_lines_of(&mut map(&r, wrap, &providers, &mut shared));

    assert!(expected.len() > 1, "the fixture must actually wrap");
    assert_eq!(actual, expected);
}

/// A horizontally clipped format is not interchangeable with an unclipped
/// one — it *drops* the graphemes left of its window rather than truncating,
/// so it cannot stand in for a scan that kept them. `LineFormat::covers`
/// checks the recorded window against the querying map's own, so the two
/// never meet; this is what stops the frame's two passes sharing a *format*
/// in `WrapMode::None`, where only the render pass clips (they still share
/// the entry's block shape — see `an_h_window_change_keeps_the_block_shape`).
#[test]
fn an_h_window_map_does_not_read_an_unclipped_format() {
    let r = Rope::from_str("alpha bravo charlie delta\n");
    let (providers, calls) = with_counting_insert(ContentLine::new(0), 0, "x");
    let mut store = PaneLineStore::new();

    let mut unclipped = map(&r, WrapMode::None, &providers, &mut store);
    unclipped.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));
    drop(unclipped);
    let after_first = calls.get();
    assert_eq!(after_first, 1, "no-wrap formats on render, not on block");

    let mut clipped =
        map(&r, WrapMode::None, &providers, &mut store).with_h_window(Some(dc(6)..dc(20)));
    clipped.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));

    assert_eq!(
        calls.get(),
        after_first + 1,
        "a windowed map must format for itself, not read the unclipped entry"
    );
}

/// Block shape (virtual display lines, `before`/`after`) does not depend on the
/// horizontal window a `WrapMode::None` render clips to — only the formatted
/// display lines do. An `h_window` change must not force `block_entry` to re-query
/// providers for a line the store already has.
#[test]
fn an_h_window_change_keeps_the_block_shape() {
    let r = Rope::from_str("alpha\n");
    let (providers, calls) = with_counting_anchor();
    let mut store = PaneLineStore::new();

    map(&r, WrapMode::None, &providers, &mut store).block(ContentLine::new(0));
    let after_first = calls.get();

    map(&r, WrapMode::None, &providers, &mut store)
        .with_h_window(Some(dc(0)..dc(5)))
        .block(ContentLine::new(0));

    assert_eq!(
        calls.get(),
        after_first,
        "an h_window change must reuse the entry's block shape, not rebuild it"
    );
}

/// The scope key covers every formatting input, so a buffer that changed
/// under the same pane invalidates what was stored for it — those entries
/// describe a format the new inputs would not produce.
#[test]
fn a_changed_key_drops_the_scope() {
    let r = Rope::from_str("alpha\nbravo\n");
    let (providers, calls) = with_counting_insert(ContentLine::new(0), 0, "x");
    let mut store = PaneLineStore::new();
    let wrap = WrapMode::Soft { width: 80 };

    DisplayLineMap::new(
        &r,
        &providers,
        80,
        FormatKey {
            buffer_tag: [1; 3],
            wrap_mode: wrap,
            tab_width: 4,
            whitespace: ws(),
        },
        &mut store,
    )
    .block(ContentLine::new(0));
    let after_first = calls.get();

    // Same store, same line, different buffer tag.
    DisplayLineMap::new(
        &r,
        &providers,
        80,
        FormatKey {
            buffer_tag: [2; 3],
            wrap_mode: wrap,
            tab_width: 4,
            whitespace: ws(),
        },
        &mut store,
    )
    .block(ContentLine::new(0));

    assert_eq!(
        calls.get(),
        after_first + 1,
        "entries built under the old key must not be served under the new one"
    );
}

/// Entries must not outlive the frame that produced them: the per-pane
/// inline-insert mirrors are rebuilt each frame filtered to that frame's
/// viewport, without bumping anything the key can see. `rewind` is what
/// `EngineView::begin_frame` calls on every pane's store to enforce that.
#[test]
fn rewind_drops_the_previous_frames_entries() {
    let r = Rope::from_str("alpha\nbravo\n");
    let (providers, calls) = with_counting_insert(ContentLine::new(0), 0, "x");
    let mut store = PaneLineStore::new();
    let wrap = WrapMode::Soft { width: 80 };

    map(&r, wrap, &providers, &mut store).block(ContentLine::new(0));
    let after_first = calls.get();

    store.rewind();

    map(&r, wrap, &providers, &mut store).block(ContentLine::new(0));

    assert_eq!(
        calls.get(),
        after_first + 1,
        "a new frame must re-format rather than read last frame's entry"
    );
}

/// A line whose *block shape* is all anyone asked for must not pay for format
/// buffers it never fills. Under `WrapMode::None` — the default — `content_display_lines`
/// answers 1 without running the formatter, so a walk that only asks for shape
/// (a half-page motion stepping display line by display line, once per selection) touches many
/// lines and formats none of them. Their entries have to cost bookkeeping
/// rather than a buffer apiece.
#[test]
fn a_shape_only_entry_allocates_no_format_buffers() {
    let r = Rope::from_str("hello world\nsecond line\n");
    let providers = ProviderSet::new();
    let mut store = PaneLineStore::new();

    let breakdown = DisplayLineMap::new(
        &r,
        &providers,
        80,
        FormatKey {
            buffer_tag: [1; 3],
            wrap_mode: WrapMode::None,
            tab_width: 4,
            whitespace: ws(),
        },
        &mut store,
    )
    .block(ContentLine::new(0));
    assert_eq!(
        breakdown.content, 1,
        "sanity: no-wrap block shape must resolve without formatting"
    );

    let format = &store.entry(store.find(ContentLine::new(0)).unwrap()).format;
    assert_eq!(
        format.graphemes.capacity(),
        0,
        "a walked-but-unformatted entry must hold no grapheme buffer"
    );
    assert_eq!(
        format.line_texts.capacity(),
        0,
        "a walked-but-unformatted entry must hold no line-text buffer"
    );
}

/// A line pathologically wider than any ordinary source line (a minified-JS
/// file's single line, megabytes wide) must not pin its whole grapheme/text
/// capacity once the frame that formatted it has passed — that would reverse
/// the free list's own memory bound.
///
/// The frame boundary is the only moment that can be relied on to give it
/// back. Reusing the slot cannot: a slot is rebound only by a later frame
/// that walks at least that many lines, and a pane showing one wide line and
/// then a handful of short ones never walks far enough to reach it again.
#[test]
fn rewind_shrinks_an_oversized_entry() {
    let (before, after) = grapheme_capacity_across_rewind(50_000);
    assert!(
        before > 50_000,
        "sanity: formatting the huge line must have grown its capacity"
    );
    assert!(
        after < before,
        "the frame boundary must hand back a pathologically grown entry \
         rather than wait for a later frame to rebind its slot"
    );
}

/// Below the shrink ceiling, an entry keeps its capacity across the frame
/// boundary — the free list's whole point, which a naive "always shrink on
/// rewind" implementation would defeat for every ordinary line.
#[test]
fn rewind_does_not_shrink_an_ordinary_entry() {
    let (before, after) = grapheme_capacity_across_rewind(2_000);
    assert_eq!(
        after, before,
        "an ordinary-sized line's capacity must survive the frame boundary"
    );
}

/// Format a `width`-grapheme line into a store's single entry, then rewind
/// the store, reporting that entry's grapheme capacity on each side of the
/// rewind.
///
/// Reads the entry back by slot rather than by line: `rewind` clears the line
/// index, and a slot nothing later rebinds is exactly the case this pins.
fn grapheme_capacity_across_rewind(width: usize) -> (usize, usize) {
    let r = Rope::from_str(&format!("{}\n", "x".repeat(width)));
    let providers = ProviderSet::new();
    let mut store = PaneLineStore::new();

    DisplayLineMap::new(
        &r,
        &providers,
        80,
        FormatKey {
            buffer_tag: [1; 3],
            wrap_mode: WrapMode::None,
            tab_width: 4,
            whitespace: ws(),
        },
        &mut store,
    )
    .render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));
    let before = store.entry(0).format.graphemes.capacity();

    store.rewind();

    (before, store.entry(0).format.graphemes.capacity())
}
