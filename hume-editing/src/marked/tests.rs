use pretty_assertions::assert_eq;

use super::*;

fn round_trip(input: &str) {
    assert_eq!(render(parse(input).view()), input);
}

#[test]
fn notation_round_trips() {
    round_trip("-[h]>ello\n");
    round_trip("-[hell]>o\n");
    round_trip("<[hell]-o\n");
    round_trip("-[he]>llo -[wor]>ld\n");
    round_trip("ab-[\n]>");
    round_trip("-[a]>-[b]>\n");
    round_trip("x-[e\u{301}]>y-[\u{1f1ee}\u{1f1f9}]>\n");
    round_trip("x<[e\u{301}y]-\n");
}

#[test]
fn markers_are_literal_text_when_they_open_nothing() {
    let state = parse("a-b<c]d-[x]>\n");
    assert_eq!(state.text().to_string(), "a-b<c]dx\n");
}

#[test]
fn the_first_selection_is_primary() {
    let state = parse("-[a]>b-[c]>\n");
    assert_eq!(state.view().primary().index(), 0);
}

#[test]
#[should_panic(expected = "splits a grapheme cluster")]
fn a_marker_inside_a_cluster_panics() {
    parse("-[e]>\u{301}\n");
}

#[test]
#[should_panic(expected = "no selection")]
fn a_notation_without_a_selection_panics() {
    parse("hello\n");
}

#[test]
#[should_panic(expected = "structural")]
fn a_notation_without_the_final_break_panics() {
    parse("-[h]>ello");
}

#[test]
#[should_panic(expected = "unterminated")]
fn an_unterminated_selection_panics() {
    parse("-[hello\n");
}

#[test]
#[should_panic(expected = "empty selection")]
fn an_empty_selection_panics() {
    parse("a-[]>b\n");
}

fn ends(input: &str) -> (usize, usize) {
    let state = parse(input);
    let sel = state.view().primary().selection();
    (sel.anchor().offset().index(), sel.head().offset().index())
}

#[test]
fn anchor_and_head_are_the_starts_of_the_clusters_they_cover() {
    assert_eq!(ends("-[h]>ello\n"), (0, 0));
    assert_eq!(ends("hello-[\n]>"), (5, 5));
    assert_eq!(ends("-[hell]>o\n"), (0, 3));
    assert_eq!(ends("<[hel]-lo\n"), (2, 0));
    assert_eq!(ends("a-[e\u{301}]>b\n"), (1, 1));
    assert_eq!(ends("-[ae\u{301}]>b\n"), (0, 1));
    assert_eq!(ends("<[ae\u{301}]-b\n"), (1, 0));
}

#[test]
fn notation_round_trips_over_every_corpus_cluster() {
    for sample in test_fixtures::unicode::ALL {
        round_trip(&format!("\n-[{sample}]>b\n"));
        round_trip(&format!("-[\n{sample}]>b\n"));
        round_trip(&format!("<[\n{sample}]-b\n"));
    }
}

#[test]
#[should_panic(expected = "splits a grapheme cluster")]
fn an_open_marker_inside_a_cluster_panics() {
    parse("ae-[\u{301}]>b\n");
}
