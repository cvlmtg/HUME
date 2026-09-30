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
    round_trip("-{he}>llo -[wor]>ld\n");
    round_trip("-[he]>llo <{wor}-ld\n");
    round_trip("ab-[\n]>");
    round_trip("-[a]>-{b}>\n");
    round_trip("x-{e\u{301}}>y-[\u{1f1ee}\u{1f1f9}]>\n");
    round_trip("x<[e\u{301}y]-\n");
}

#[test]
fn markers_are_literal_text_when_they_open_nothing() {
    let state = parse("a-b<c]d-[x]>\n");
    assert_eq!(state.text().to_string(), "a-b<c]dx\n");
}

#[test]
fn brace_markers_are_literal_text_when_they_open_nothing() {
    let state = parse("f()}>x}-y-[z]>\n");
    assert_eq!(state.text().to_string(), "f()}>x}-yz\n");
}

#[test]
fn the_braced_selection_is_primary() {
    assert_eq!(parse("-{a}>b-[c]>\n").view().primary().index(), 0);
    assert_eq!(parse("-[a]>b<{c}-\n").view().primary().index(), 1);
}

#[test]
fn a_primary_after_the_first_renders_braced() {
    let state = parse("-{a}>b-[c]>\n").cycle_primary(1);
    assert_eq!(render(state.view()), "-[a]>b-{c}>\n");
}

#[test]
#[should_panic(expected = "no primary")]
fn several_selections_without_a_primary_panic() {
    parse("-[a]>b-[c]>\n");
}

#[test]
#[should_panic(expected = "two primaries")]
fn several_braced_selections_panic() {
    parse("-{a}>b-{c}>\n");
}

#[test]
#[should_panic(expected = "single selection")]
fn a_braced_single_selection_panics() {
    parse("-{a}>b\n");
}

#[test]
#[should_panic(expected = "closed by")]
fn a_brace_closed_by_a_bracket_panics() {
    parse("-{a]>b-[c]>\n");
}

#[test]
#[should_panic(expected = "closed by")]
fn a_bracket_closed_by_a_brace_panics() {
    parse("-[a}>b-{c}>\n");
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

#[test]
#[should_panic(expected = "notation is LF-only")]
fn parse_refuses_crlf_notation() {
    parse("a\r\n-[b]>\n");
}

#[test]
#[should_panic(expected = "a selection opened inside another")]
fn parse_refuses_a_selection_opened_inside_another() {
    parse("-[a-[b]>\n");
}

/// `text` with one selection from `anchor` to `head`, char offsets of
/// cluster starts, in notation.
fn render_one(text: &str, anchor: usize, head: usize) -> String {
    let text = BufferText::from(text);
    let sel = Selection::new(start_at(&text, anchor), start_at(&text, head));
    render(EditState::from_text(text, vec![sel], 0).view())
}

#[test]
fn render_marks_a_backward_selection_ending_on_the_structural_break() {
    assert_eq!(render_one("ab\n", 2, 1), "a<[b\n]-");
}

#[test]
fn render_marks_a_cursor_on_a_zwj_sequence_around_the_whole_cluster() {
    let family = zwj_family();
    assert_eq!(
        render_one(&format!("x{family}\n"), 1, 1),
        format!("x-[{family}]>\n")
    );
}

/// A ZWJ family emoji: five chars, one cluster.
fn zwj_family() -> &'static str {
    "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}"
}

#[test]
fn an_empty_text_with_a_cursor_on_its_break_round_trips() {
    round_trip("-[\n]>");
}

#[test]
#[should_panic(expected = "char 1 is not a cluster start")]
fn start_at_refuses_a_char_inside_a_cluster() {
    start_at(&BufferText::from("e\u{301}\n"), 1);
}

#[test]
#[should_panic(expected = "char 2 is not a cluster start")]
fn start_at_refuses_the_text_end() {
    start_at(&BufferText::from("a\n"), 2);
}
