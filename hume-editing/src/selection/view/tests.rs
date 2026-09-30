use hume_rope::offset::{CharOffset, ExclusiveRange};
use pretty_assertions::assert_eq;

use crate::marked::parse;

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

fn covered(input: &str) -> ExclusiveRange<CharOffset> {
    parse(input).view().primary().covered().chars()
}

// ── Extent ────────────────────────────────────────────────────────────────

#[test]
fn covered_runs_to_the_end_of_the_last_cluster() {
    assert_eq!(covered("a-[bc]>d\n"), ExclusiveRange::new(co(1), co(3)));
    assert_eq!(
        covered("a-[e\u{301}]>d\n"),
        ExclusiveRange::new(co(1), co(3))
    );
    assert_eq!(
        covered("a<[be\u{301}]-d\n"),
        ExclusiveRange::new(co(1), co(4))
    );
}

#[test]
fn the_slice_is_the_covered_text() {
    let state = parse("x-{a\u{308}\u{301}}>y-[\u{1f468}\u{200d}\u{1f469}]>z\n");
    let slices: Vec<String> = state.view().iter().map(|v| v.slice().to_string()).collect();
    assert_eq!(
        slices,
        vec!["a\u{308}\u{301}", "\u{1f468}\u{200d}\u{1f469}"]
    );
}

#[test]
fn ends_on_break_reads_the_last_covered_cluster() {
    let state = parse("-{ab\n}>c-[d]>\n");
    let ends: Vec<bool> = state.view().iter().map(|v| v.ends_on_break()).collect();
    assert_eq!(ends, vec![true, false]);
    assert!(parse("ab-[\n]>").view().primary().ends_on_break());
}

#[test]
fn content_drops_the_line_break_the_selection_ends_on() {
    let view_of = |input| {
        let state = parse(input);
        let primary = state.view().primary();
        (
            primary.content().map(|r| r.chars()),
            primary
                .content()
                .map(|r| state.text().slice(r.chars()))
                .map_or_else(String::new, |slice| slice.to_string()),
        )
    };
    assert_eq!(
        view_of("-[ab\n]>c\n"),
        (Some(ExclusiveRange::new(co(0), co(2))), "ab".to_owned())
    );
    assert_eq!(
        view_of("-[ab]>c\n"),
        (Some(ExclusiveRange::new(co(0), co(2))), "ab".to_owned())
    );
    assert_eq!(view_of("ab\n-[\n]>c\n"), (None, String::new()));
    assert_eq!(
        view_of("-[a\nb\n]>"),
        (Some(ExclusiveRange::new(co(0), co(3))), "a\nb".to_owned())
    );
}

#[test]
fn append_point_is_after_the_selection_but_before_its_line_break() {
    let at = |input| parse(input).view().primary().append_point().offset();
    assert_eq!(at("-[ae\u{301}]>b\n"), co(3));
    assert_eq!(at("-[ab\n]>c\n"), co(2));
    assert_eq!(at("ab-[\n]>"), co(2));
    assert_eq!(at("-[ab]>\n"), co(2));
}

// ── Lines ─────────────────────────────────────────────────────────────────

#[test]
fn lines_are_those_of_the_first_and_last_clusters() {
    let state = parse("ab\nc-[d\ne]>f\n");
    let lines = state.view().primary().lines();
    assert_eq!((lines.start.index(), lines.end.index()), (1, 2));
    assert_eq!(state.view().primary().head_line().index(), 2);
}

#[test]
fn starts_line_uses_the_line_of_the_first_cluster() {
    let starts = |input| parse(input).view().primary().starts_line();
    assert!(starts("-[h]>ello\n"));
    assert!(!starts("he-[l]>lo\n"));
    assert!(starts("hi\n-[b]>ye\n"));
    assert!(!starts("hi\nb-[y]>e\n"));
    assert!(!starts("hi-[\n]>"));
}

#[test]
fn is_linewise_needs_a_line_start_and_a_final_break() {
    let linewise = |input| parse(input).view().primary().is_linewise();
    assert!(linewise("-[hello\n]>world\n"));
    assert!(linewise("-[hello\nworld\n]>"));
    assert!(linewise("hello\n-[world\n]>"));
    assert!(!linewise("h-[ello\n]>world\n"));
    assert!(!linewise("-[hel]>lo\n"));
    assert!(linewise("a\n-[\n]>b\n"));
}

#[test]
fn a_cursor_on_an_empty_line_is_ambiguously_linewise() {
    let classify = |input| parse(input).view().primary().linewise_classification();
    assert_eq!(classify("a\n-[\n]>b\n"), None);
    assert_eq!(classify("-[hello\n]>\n"), Some(true));
    assert_eq!(classify("he-[l]>lo\n"), Some(false));
}

// ── Clusters and union ────────────────────────────────────────────────────

#[test]
fn clusters_yields_each_covered_cluster() {
    let state = parse("x-[ae\u{301}b]>y\n");
    let firsts: Vec<char> = state
        .view()
        .primary()
        .clusters()
        .map(|c| c.first())
        .collect();
    assert_eq!(firsts, vec!['a', 'e', 'b']);
}

#[test]
fn union_widens_to_cover_a_range_and_never_shrinks() {
    let state = parse("-[abc]>de\u{301}f\n");
    let primary = state.view().primary();
    let found = state
        .text()
        .covering(ExclusiveRange::new(co(3), co(5)))
        .expect("non-empty");
    let widened = primary.union(found, crate::selection::Facing::Forward);
    let widened = state.clone().replace_primary(widened);
    assert_eq!(crate::marked::render(widened.view()), "-[abcde\u{301}]>f\n");

    let nested = state
        .text()
        .covering(ExclusiveRange::new(co(1), co(2)))
        .expect("non-empty");
    let same = primary.union(nested, crate::selection::Facing::Forward);
    assert_eq!(same, primary.selection());
}

// ── Line spans ────────────────────────────────────────────────────────────

/// `(line, content text, covers the break)` for each line `input`'s primary
/// selection touches.
fn line_spans(input: &str) -> Vec<(usize, String, bool)> {
    let state = parse(input);
    state
        .view()
        .primary()
        .line_spans()
        .map(|span| {
            let content = span
                .content
                .map(|range| state.text().slice(range.chars()).to_string())
                .unwrap_or_default();
            (span.line.index(), content, span.has_break)
        })
        .collect()
}

#[test]
fn line_spans_split_a_selection_by_line() {
    assert_eq!(
        line_spans("a-[bc\nde\nf]>g\n"),
        vec![
            (0, "bc".to_string(), true),
            (1, "de".to_string(), true),
            (2, "f".to_string(), false),
        ]
    );
}

#[test]
fn line_spans_of_a_cursor_on_an_empty_line_cover_only_its_break() {
    assert_eq!(line_spans("a\n-[\n]>b\n"), vec![(1, String::new(), true)]);
}

#[test]
fn line_spans_of_a_selection_ending_on_a_break_stop_on_that_line() {
    assert_eq!(
        line_spans("-[ab\ncd\n]>"),
        vec![(0, "ab".to_string(), true), (1, "cd".to_string(), true)]
    );
}
