use super::*;
use pretty_assertions::assert_eq;

// ── parse_state ───────────────────────────────────────────────────────────

#[test]
fn parse_cursor_at_start() {
    // -[h]>ello\n — cursor on 'h' (offset 0), anchor == head == 0
    let (buf, sels) = parse_state("-[h]>ello\n");
    assert_eq!(buf.to_string(), "hello\n");
    assert_eq!(sels.len(), 1);
    let s = sels.primary();
    assert!(s.is_collapsed());
    assert_eq!(s.head(), 0);
    assert_eq!(s.anchor(), 0);
}

#[test]
fn parse_cursor_at_end() {
    // hello-[\n]> — cursor on '\n' (offset 5)
    let (buf, sels) = parse_state("hello-[\n]>");
    assert_eq!(buf.to_string(), "hello\n");
    assert_eq!(sels.primary().head(), 5);
}

#[test]
fn parse_cursor_in_middle() {
    // hel-[l]>o\n — cursor on second 'l' (offset 3)
    let (buf, sels) = parse_state("hel-[l]>o\n");
    assert_eq!(buf.to_string(), "hello\n");
    assert_eq!(sels.primary().head(), 3);
}

#[test]
fn parse_forward_selection() {
    // -[hell]>o world\n — anchor=0, head=3 (selects "hell")
    let (buf, sels) = parse_state("-[hell]>o world\n");
    assert_eq!(buf.to_string(), "hello world\n");
    let s = sels.primary();
    assert_eq!(s.anchor(), 0);
    assert_eq!(s.head(), 3);
}

#[test]
fn parse_backward_selection() {
    // <[hel]-lo\n — head=0, anchor=2 (cursor on 'h', selects "hel")
    let (buf, sels) = parse_state("<[hel]-lo\n");
    assert_eq!(buf.to_string(), "hello\n");
    let s = sels.primary();
    assert_eq!(s.anchor(), 2);
    assert_eq!(s.head(), 0);
}

#[test]
fn parse_selection_near_end_of_buffer() {
    // hi -[ther]>e\n — anchor=3, head=6 (selects "ther")
    let (buf, sels) = parse_state("hi -[ther]>e\n");
    assert_eq!(buf.to_string(), "hi there\n");
    let s = sels.primary();
    assert_eq!(s.anchor(), 3);
    assert_eq!(s.head(), 6);
}

#[test]
fn parse_two_cursors() {
    // -[f]>oo-[ ]>bar\n — cursors on 'f' (0) and ' ' (3)
    let (buf, sels) = parse_state("-[f]>oo-[ ]>bar\n");
    assert_eq!(buf.to_string(), "foo bar\n");
    assert_eq!(sels.len(), 2);
    assert_eq!(sels.iter_sorted().next().unwrap().head(), 0);
    assert_eq!(sels.iter_sorted().nth(1).unwrap().head(), 3);
}

#[test]
fn parse_two_forward_selections() {
    // -[ab]> -[de]>\n — (anchor=0,head=1) and (anchor=4,head=5)
    let (buf, sels) = parse_state("-[ab]> -[de]>\n");
    assert_eq!(buf.to_string(), "ab de\n");
    assert_eq!(sels.len(), 2);
    let mut it = sels.iter_sorted();
    let s0 = it.next().unwrap();
    let s1 = it.next().unwrap();
    assert_eq!((s0.anchor(), s0.head()), (0, 1));
    assert_eq!((s1.anchor(), s1.head()), (3, 4));
}

#[test]
fn parse_cursor_on_unicode_char() {
    // "é" is U+00E9, a single Unicode scalar value (1 char).
    let (buf, sels) = parse_state("caf-[é]>\n");
    assert_eq!(buf.to_string(), "café\n");
    assert_eq!(sels.primary().head(), 3);
}

#[test]
fn parse_cursor_on_only_newline() {
    // -[\n]> — cursor on '\n' in a buffer that contains only the trailing newline
    let (buf, sels) = parse_state("-[\n]>");
    assert_eq!(buf.to_string(), "\n");
    assert_eq!(sels.primary().head(), 0);
}

#[test]
fn parse_literal_dash_and_angle_in_buffer() {
    // Lone `-` and `<` (not followed by `[`) are plain buffer content.
    let (buf, sels) = parse_state("-[x]>a-b<c\n");
    assert_eq!(buf.to_string(), "xa-b<c\n");
    assert_eq!(sels.primary().head(), 0);
}

// ── serialize_state ───────────────────────────────────────────────────────

#[test]
fn serialize_cursor_at_start() {
    let buf = Text::from("hello");
    let sels = SelectionSet::single(Selection::collapsed(0));
    assert_eq!(serialize_state(&buf, &sels), "-[h]>ello\n");
}

#[test]
fn serialize_cursor_at_end() {
    // cursor at 5 = on the structural trailing \n.
    let buf = Text::from("hello");
    let sels = SelectionSet::single(Selection::collapsed(5));
    assert_eq!(serialize_state(&buf, &sels), "hello-[\n]>");
}

#[test]
fn serialize_forward_selection() {
    // anchor=0, head=3 — selects "hell" (positions 0..=3).
    let buf = Text::from("hello world");
    let sels = SelectionSet::single(Selection::new(0, 3));
    assert_eq!(serialize_state(&buf, &sels), "-[hell]>o world\n");
}

#[test]
fn serialize_backward_selection() {
    // anchor=3, head=0 — selects "hell" (positions 0..=3), cursor on 'h'.
    let buf = Text::from("hello");
    let sels = SelectionSet::single(Selection::new(3, 0));
    assert_eq!(serialize_state(&buf, &sels), "<[hell]-o\n");
}

#[test]
fn serialize_forward_selection_head_at_eof() {
    // head=5 is the trailing \n in "hello\n". Selects "hello\n".
    let buf = Text::from("hello");
    let sels = SelectionSet::single(Selection::new(0, 5));
    assert_eq!(serialize_state(&buf, &sels), "-[hello\n]>");
}

// ── Round-trip ────────────────────────────────────────────────────────────

fn round_trip(s: &str) -> String {
    let (buf, sels) = parse_state(s);
    serialize_state(&buf, &sels)
}

#[test]
fn roundtrip_cursor() {
    assert_eq!(round_trip("-[h]>ello\n"), "-[h]>ello\n");
    assert_eq!(round_trip("hello-[\n]>"), "hello-[\n]>");
    assert_eq!(round_trip("hel-[l]>o\n"), "hel-[l]>o\n");
}

#[test]
fn roundtrip_forward_selection() {
    assert_eq!(round_trip("-[hell]>o world\n"), "-[hell]>o world\n");
}

#[test]
fn roundtrip_backward_selection() {
    assert_eq!(round_trip("<[hel]-lo\n"), "<[hel]-lo\n");
}

#[test]
fn roundtrip_two_cursors() {
    assert_eq!(round_trip("-[f]>oo-[ ]>bar\n"), "-[f]>oo-[ ]>bar\n");
}

#[test]
fn roundtrip_newline_only_buffer() {
    assert_eq!(round_trip("-[\n]>"), "-[\n]>");
}
