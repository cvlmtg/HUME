//! Render-accessor tests (`render_display_line`).

use super::*;

/// A VIRTUAL_LINE-kind source that never emits anything — registered only to
/// consume a `ProviderId` so the next provider's real id is not 0.
struct NoVirtualLines;

impl DecorationSource for NoVirtualLines {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }
    fn decorations_for_line(&self, _line_idx: ContentLine, _out: &mut Vec<Decoration>) {}
}

// ---------------------------------------------------------------------------
// Render accessors
// ---------------------------------------------------------------------------

#[test]
fn render_display_line_yields_a_content_lines_wrap_display_lines() {
    let rope = Rope::from_str("abcdefgh\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    let dl0 = dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));
    assert_eq!(
        dl0.display_line.kind,
        crate::types::DisplayLineKind::LineStart {
            line_idx: RopeyLine::new(0)
        }
    );
    assert_eq!(display_line_text(&dl0), "abcd");

    let dl1 = dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 1));
    assert_eq!(
        dl1.display_line.kind,
        crate::types::DisplayLineKind::Wrap {
            line_idx: RopeyLine::new(0),
            wrap_index: 1,
        }
    );
    assert_eq!(display_line_text(&dl1), "efgh");
}

#[test]
fn render_display_line_segments_a_virtual_lines_text() {
    let rope = Rope::from_str("hi\n");
    let mut providers = ProviderSet::new();
    // Consume id 0 so the emitting provider's real id is 1 — it self-reports
    // 0, which must be overwritten.
    providers.add_decoration_source(Box::new(NoVirtualLines));
    providers.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        1,
        "deleted line",
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    let virtual_line = dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));
    assert_eq!(
        virtual_line.display_line.kind,
        crate::types::DisplayLineKind::Virtual {
            provider_id: 1,
            anchor_line: RopeyLine::new(0),
        },
        "the registry's id replaces the provider's self-reported one"
    );
    assert_eq!(display_line_text(&virtual_line), "deleted line");
}

#[test]
fn render_display_line_expands_a_tab_in_a_virtual_lines_text() {
    // A virtual display line must be tab-aware exactly like a real buffer line — this
    // is what lets `set-virtual-lines!` accept a literal `\t` in `'text`
    // instead of requiring the caller to expand it by hand (previously the
    // git-diff plugin's job, and the source of its column-counting bug).
    let rope = Rope::from_str("hi\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(NoVirtualLines));
    providers.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        1,
        "\tx",
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s); // tab_width == 4, see `map`

    let virtual_line = dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));
    let cells = &virtual_line.graphemes[virtual_line.display_line.graphemes.clone()];
    assert_eq!(cells.len(), 2, "one cell for the tab, one for 'x'");
    assert_eq!(cells[0].display_col, dc(0));
    assert_eq!(
        cells[0].width, 4,
        "tab at display_col 0, tab_width 4 -> full stop"
    );
    assert!(
        matches!(cells[0].content, CellContent::TabFill),
        "a tab renders as TabFill, matching a real buffer line's tab with its indicator off"
    );
    assert_eq!(
        cells[1].display_col,
        dc(4),
        "'x' lands right after the tab stop"
    );
}

#[test]
fn render_display_line_wide_cjk_before_tab_in_a_virtual_lines_text_shifts_the_stop() {
    // A wide CJK grapheme before a tab must shift the tab's stop by its full
    // 2-column width, matching a real buffer line — the exact case
    // `git-diff/render.scm` used to get wrong when it counted one Steel char
    // (not one display column) per preceding character.
    let rope = Rope::from_str("hi\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(NoVirtualLines));
    providers.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        1,
        "\u{6F22}\tx",
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s); // tab_width == 4, see `map`

    let virtual_line = dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));
    let cells = &virtual_line.graphemes[virtual_line.display_line.graphemes.clone()];
    // 漢(w2) + its WidthContinuation, then the tab (tab_advance(2, 4) == 2,
    // so it also occupies 2 columns and gets its own WidthContinuation —
    // same as any width-2 cell, tab or not), then 'x'.
    assert_eq!(cells.len(), 5);
    assert_eq!(cells[0].display_col, dc(0));
    assert_eq!(cells[0].width, 2);
    assert!(matches!(cells[1].content, CellContent::WidthContinuation));
    assert_eq!(
        cells[2].display_col,
        dc(2),
        "tab starts right after the wide char"
    );
    assert_eq!(cells[2].width, 2, "tab_advance(2, 4) == 2");
    assert!(matches!(cells[3].content, CellContent::WidthContinuation));
    assert_eq!(cells[4].display_col, dc(4), "'x' lands at column 4, not 3");
}

#[test]
fn h_window_clips_an_unwrapped_display_lines_graphemes_without_changing_its_display_line_count() {
    // The render path's bound on arbitrarily long unwrapped lines: only the
    // window's columns are emitted, but the line is still one display line.
    let rope = Rope::from_str("abcdefghij\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s).with_h_window(Some(dc(2)..dc(5)));

    assert_eq!(
        dlm.block(ContentLine::new(0)).content,
        1,
        "clipping never adds or drops display lines"
    );
    assert_eq!(
        display_line_text(&dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0))),
        "cde",
        "columns 2..5 of the line, and nothing else"
    );
}

#[test]
fn render_display_line_formats_a_line_once_however_many_display_lines_are_drawn() {
    // Both wrap display lines of one line come from a single format pass.
    let rope = Rope::from_str("abcdef\n");
    let (providers, calls) = with_counting_insert(ContentLine::new(0), 0, "hint");
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 8 }, &providers, &mut s);

    assert_eq!(dlm.block(ContentLine::new(0)).content, 2);
    let after_count = calls.get();
    dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));
    dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 1));
    assert_eq!(
        calls.get(),
        after_count,
        "rendering display lines of an already-counted line must not re-format it"
    );
}

#[test]
fn render_display_line_does_not_reformat_a_line_because_of_its_virtual_lines() {
    // A Before display line is laid out and rendered before its line's content display lines —
    // it must not disturb the already-formatted content display line/grapheme/arena
    // state that follows it in the same block.
    let rope = Rope::from_str("abcdef\n");
    let (mut providers, calls) = with_counting_insert(ContentLine::new(0), 0, "hint");
    providers.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        1,
        "V",
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 8 }, &providers, &mut s);

    assert_eq!(dlm.block(ContentLine::new(0)).content, 2);
    let after_count = calls.get();
    dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0)); // the Before display line
    dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 1)); // content display line 0
    dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 2)); // content display line 1
    assert_eq!(
        calls.get(),
        after_count,
        "rendering a line's Before display line must not force its content display lines to re-format"
    );
}

#[test]
fn render_display_line_yields_correct_content_display_lines_after_a_virtual_display_line() {
    // A virtual display line's layout must not disturb the content display lines that follow
    // it in the same block: they must still come back correct.
    let rope = Rope::from_str("abcdefgh\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        1,
        "V",
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(
        display_line_text(&dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0))),
        "V"
    );
    assert_eq!(
        display_line_text(&dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 1))),
        "abcd"
    );
    assert_eq!(
        display_line_text(&dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 2))),
        "efgh"
    );
}
