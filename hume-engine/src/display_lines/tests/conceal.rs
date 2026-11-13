//! Revealed lines: which conceals a line formats with, and when the store
//! formats a line again.

use hume_rope::offset::InclusiveRange;

use super::*;
use crate::folds::{Fold, HiddenLines};
use crate::line_set::LineSet;
use crate::reveal::RevealedLines;
use crate::test_doubles::ConcealSource;

fn span(start: usize, end: usize) -> InclusiveRange<ContentLine> {
    InclusiveRange::new(ContentLine::new(start), ContentLine::new(end))
}

fn lines(ranges: &[(usize, usize)]) -> RevealedLines {
    RevealedLines::Lines(LineSet::from_ranges(
        ranges.iter().map(|&(start, end)| span(start, end)),
    ))
}

fn revealing(conceal: Conceal, start: usize, end: usize) -> Conceal {
    Conceal {
        reveal: Some(span(start, end)),
        ..conceal
    }
}

/// A map over `rope` that shows `revealed` raw and hides what `hidden`
/// folds away.
fn map_with<'a>(
    rope: &'a Rope,
    providers: &'a ProviderSet,
    revealed: RevealedLines,
    hidden: HiddenLines,
    store: &'a mut PaneLineStore,
) -> DisplayLineMap<'a> {
    DisplayLineMap::new(
        rope,
        providers,
        DisplayLineInputs {
            format: FormatKey {
                buffer_tag: [0; 3],
                wrap_mode: WrapMode::None,
                tab_width: 4,
                whitespace: ws(),
            },
            hidden,
            content_width: crate::pipeline::ContentWidth::for_test(80),
            revealed,
        },
        store,
    )
}

/// The text each of `rope`'s lines shows under `revealed`.
fn shown(rope: &Rope, providers: &ProviderSet, revealed: RevealedLines) -> Vec<String> {
    let mut store = PaneLineStore::new();
    let mut dlm = map_with(
        rope,
        providers,
        revealed,
        HiddenLines::default(),
        &mut store,
    );
    (0..content_line_count(rope).get())
        .map(|n| {
            display_line_text(&dlm.render_display_line(DisplayLinePos::new(ContentLine::new(n), 0)))
        })
        .collect()
}

/// A counting `ConcealSource` hiding the first two bytes of line 0.
fn counted_line_0() -> (ProviderSet, Rc<Cell<usize>>) {
    let calls = Rc::new(Cell::new(0));
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(
        ConcealSource::new(vec![(ContentLine::new(0), vec![hidden(0, 2, None)])])
            .counting(Rc::clone(&calls)),
    ));
    (providers, calls)
}

/// Formats line 0 through a fresh map over `store`.
fn format_line_0(
    rope: &Rope,
    providers: &ProviderSet,
    revealed: RevealedLines,
    hidden: HiddenLines,
    store: &mut PaneLineStore,
) {
    map_with(rope, providers, revealed, hidden, store)
        .render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));
}

#[test]
fn conceal_on_a_revealed_line_is_dropped() {
    let rope = Rope::from_str("**a**\n**b**\n");
    let providers = concealed(vec![
        (0, vec![hidden(0, 2, None), hidden(3, 5, None)]),
        (1, vec![hidden(0, 2, None), hidden(3, 5, None)]),
    ]);

    assert_eq!(shown(&rope, &providers, lines(&[(0, 0)])), ["**a**", "b"]);
}

#[test]
fn conceal_with_a_reveal_span_is_dropped_when_its_own_line_is_revealed() {
    let rope = Rope::from_str("**a**\nb\nc\n");
    let providers = concealed(vec![(0, vec![revealing(hidden(0, 2, None), 1, 2)])]);

    assert_eq!(
        shown(&rope, &providers, lines(&[(0, 0)]))[0],
        "**a**",
        "its own line revealed, outside its span"
    );
    assert_eq!(
        shown(&rope, &providers, lines(&[(4, 4)]))[0],
        "a**",
        "no line of its own or of its span revealed"
    );
}

#[test]
fn cursor_on_table_row_3_reveals_row_1() {
    let rope = Rope::from_str("|a|\n|b|\n|c|\n");
    let row = || vec![revealing(hidden(0, 1, None), 0, 2)];
    let providers = concealed(vec![(0, row()), (1, row()), (2, row())]);

    assert_eq!(
        shown(&rope, &providers, lines(&[(2, 2)])),
        ["|a|", "|b|", "|c|"]
    );
    assert_eq!(
        shown(&rope, &providers, lines(&[(3, 3)])),
        ["a|", "b|", "c|"],
        "a revealed line past the span leaves the rows concealed"
    );
}

#[test]
fn inserted_conceal_with_a_reveal_span_drops_for_a_span_line_and_for_its_own_line() {
    let rope = Rope::from_str("a\nb\n");
    let providers = concealed(vec![(0, vec![revealing(inserted(0, "•"), 1, 1)])]);

    assert_eq!(shown(&rope, &providers, lines(&[]))[0], "•a");
    assert_eq!(
        shown(&rope, &providers, lines(&[(1, 1)]))[0],
        "a",
        "a line of its span revealed"
    );
    assert_eq!(
        shown(&rope, &providers, lines(&[(0, 0)]))[0],
        "a",
        "its own line revealed"
    );
}

#[test]
fn all_shows_every_line_raw() {
    let rope = Rope::from_str("**a**\n# b\n");
    let providers = concealed(vec![
        (0, vec![hidden(0, 2, None), hidden(3, 5, None)]),
        (1, vec![hidden(0, 2, Some("◆ ")), inserted(3, "¶")]),
    ]);

    assert_eq!(
        shown(&rope, &providers, RevealedLines::All),
        ["**a**", "# b"]
    );
}

#[test]
fn changed_revealed_set_rewinds_the_store() {
    let rope = Rope::from_str("**a\nb\n");
    let (providers, calls) = counted_line_0();
    let mut store = PaneLineStore::new();

    format_line_0(
        &rope,
        &providers,
        lines(&[(0, 0)]),
        HiddenLines::default(),
        &mut store,
    );
    format_line_0(
        &rope,
        &providers,
        lines(&[(1, 1)]),
        HiddenLines::default(),
        &mut store,
    );

    assert_eq!(calls.get(), 2);
}

#[test]
fn reveal_all_then_drawn_map_reformat() {
    let rope = Rope::from_str("**a\nb\n");
    let (providers, calls) = counted_line_0();
    let mut store = PaneLineStore::new();

    format_line_0(
        &rope,
        &providers,
        RevealedLines::All,
        HiddenLines::default(),
        &mut store,
    );
    format_line_0(
        &rope,
        &providers,
        lines(&[(1, 1)]),
        HiddenLines::default(),
        &mut store,
    );

    assert_eq!(calls.get(), 2);
}

#[test]
fn fold_toggle_does_not_reformat() {
    let rope = Rope::from_str("**a\nb\nc\n");
    let (providers, calls) = counted_line_0();
    let mut store = PaneLineStore::new();
    let fold = Fold::new(ContentLine::new(1), ContentLine::new(2), false).expect("hides line 2");

    format_line_0(
        &rope,
        &providers,
        lines(&[(1, 1)]),
        HiddenLines::default(),
        &mut store,
    );
    format_line_0(
        &rope,
        &providers,
        lines(&[(1, 1)]),
        HiddenLines::from_folds([fold]),
        &mut store,
    );

    assert_eq!(calls.get(), 1);
}

#[test]
fn one_conceal_query_per_formatted_line_per_scope() {
    let rope = Rope::from_str("**a\nb\n");
    let (providers, calls) = counted_line_0();
    let mut store = PaneLineStore::new();
    let mut dlm = map_with(
        &rope,
        &providers,
        lines(&[(1, 1)]),
        HiddenLines::default(),
        &mut store,
    );

    dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));
    dlm.locate(at(&rope, 2));
    dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));

    assert_eq!(calls.get(), 1);
}
