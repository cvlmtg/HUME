use pretty_assertions::assert_eq;

use super::*;

fn bytes(start: usize, end: usize) -> ExclusiveRange<ByteCol> {
    ExclusiveRange::new(ByteCol::new(start), ByteCol::new(end))
}

fn spans<T: Clone>(
    text: &str,
    spans: Vec<(usize, usize, T)>,
) -> Result<Vec<(ExclusiveRange<ByteCol>, T)>, SpanError> {
    SpannedText::new(text.to_string(), spans).map(|spanned| spanned.spans().to_vec())
}

fn error(index: usize, kind: SpanErrorKind) -> SpanError {
    SpanError { index, kind }
}

#[test]
fn new_sorts_by_start() {
    // ASCII text, so char and byte offsets coincide and only the sort is under test.
    let out = spans("abcdef", vec![(4, 6, "b"), (0, 2, "a")]).unwrap();
    assert_eq!(out, vec![(bytes(0, 2), "a"), (bytes(4, 6), "b")]);
}

#[test]
fn new_keeps_the_text() {
    let spanned = SpannedText::new("abc".to_string(), vec![(0, 1, ())]).unwrap();
    assert_eq!(spanned.text(), "abc");
}

#[test]
fn plain_has_no_spans() {
    let spanned = SpannedText::<()>::plain("abc".to_string());
    assert_eq!(spanned.text(), "abc");
    assert!(spanned.spans().is_empty());
}

#[test]
fn map_payload_keeps_ranges_and_order() {
    let spanned = SpannedText::new("abcdef".to_string(), vec![(3, 6, 2), (0, 2, 1)])
        .unwrap()
        .map_payload(|n| n * 10);
    assert_eq!(spanned.spans(), &[(bytes(0, 2), 10), (bytes(3, 6), 20)]);
    assert_eq!(spanned.text(), "abcdef");
}

fn run_list<'a>(
    text: &'a str,
    spans: &'a [(ExclusiveRange<ByteCol>, char)],
) -> Vec<(&'a str, Option<char>)> {
    runs(text, spans)
        .map(|(run, p)| (run, p.copied()))
        .collect()
}

#[test]
fn runs_split_gaps_spans_and_tail() {
    let spans = [(bytes(2, 4), 'a'), (bytes(5, 7), 'b')];
    assert_eq!(
        run_list("xxAAyBBzz", &spans),
        vec![
            ("xx", None),
            ("AA", Some('a')),
            ("y", None),
            ("BB", Some('b')),
            ("zz", None),
        ]
    );
}

#[test]
fn runs_skip_empty_gaps_and_tail() {
    let spans = [(bytes(0, 2), 'a'), (bytes(2, 4), 'b')];
    assert_eq!(
        run_list("AABB", &spans),
        vec![("AA", Some('a')), ("BB", Some('b'))]
    );
}

#[test]
fn runs_of_text_without_spans_is_the_whole_text() {
    assert_eq!(run_list("abc", &[]), vec![("abc", None)]);
}

#[test]
fn runs_of_empty_text_is_empty() {
    assert_eq!(run_list("", &[]), vec![]);
}

#[test]
fn new_rejects_start_ge_end() {
    assert_eq!(
        spans("abcdef", vec![(2, 2, ())]),
        Err(error(0, SpanErrorKind::Empty { start: 2, end: 2 }))
    );
}

#[test]
fn new_rejects_end_past_char_length() {
    assert_eq!(
        spans("ab", vec![(0, 5, ())]),
        Err(error(0, SpanErrorKind::PastEnd { end: 5, len: 2 }))
    );
}

#[test]
fn new_rejects_overlap() {
    assert_eq!(
        spans("abcdef", vec![(0, 3, ()), (2, 5, ())]),
        Err(error(
            1,
            SpanErrorKind::Overlap {
                start: 2,
                end: 5,
                other_start: 0,
                other_end: 3
            }
        ))
    );
}

#[test]
fn an_error_names_the_span_by_its_place_in_the_callers_list() {
    // Sorted, the overlapping span comes second, but the caller passed it first.
    assert_eq!(
        spans("abcdef", vec![(2, 5, ()), (4, 6, ()), (0, 3, ())]),
        Err(error(
            0,
            SpanErrorKind::Overlap {
                start: 2,
                end: 5,
                other_start: 0,
                other_end: 3
            }
        ))
    );
    assert_eq!(
        spans("ab", vec![(1, 9, ()), (0, 1, ())]),
        Err(error(0, SpanErrorKind::PastEnd { end: 9, len: 2 }))
    );
}

#[test]
fn new_widens_a_span_splitting_a_grapheme_cluster() {
    // "e" + U+0301 is chars 0..2 and bytes 0..3, one cluster; "x" is byte 3.
    let text = "e\u{301}x";
    assert_eq!(
        spans(text, vec![(0, 1, ())]).unwrap(),
        vec![(bytes(0, 3), ())]
    );
    assert_eq!(
        spans(text, vec![(1, 3, ())]).unwrap(),
        vec![(bytes(0, 4), ())]
    );
}

#[test]
fn new_widens_across_a_zwj_sequence() {
    // Man (4 bytes) + ZWJ (3 bytes) + woman (4 bytes): three chars, one cluster of 11 bytes.
    let text = "\u{1F468}\u{200D}\u{1F469}";
    assert_eq!(
        spans(text, vec![(1, 2, ())]).unwrap(),
        vec![(bytes(0, 11), ())]
    );
}

#[test]
fn new_rejects_two_spans_splitting_one_cluster() {
    assert_eq!(
        spans("e\u{301}", vec![(0, 1, ()), (1, 2, ())]),
        Err(error(
            1,
            SpanErrorKind::Overlap {
                start: 1,
                end: 2,
                other_start: 0,
                other_end: 1
            }
        ))
    );
}

#[test]
fn new_converts_multibyte_char_offsets() {
    // "é" is 2 bytes and "→" is 3, each its own cluster: chars 0..2 are bytes 0..5.
    assert_eq!(
        spans("é→x", vec![(0, 2, ())]).unwrap(),
        vec![(bytes(0, 5), ())]
    );
}

#[test]
fn span_error_kind_display_names_no_noun() {
    assert_eq!(
        SpanErrorKind::Empty { start: 2, end: 1 }.to_string(),
        "(2, 1) must have start < end"
    );
    assert_eq!(
        SpanErrorKind::PastEnd { end: 9, len: 3 }.to_string(),
        "end 9 is past the text's char length 3"
    );
    assert_eq!(
        SpanErrorKind::Overlap {
            start: 1,
            end: 3,
            other_start: 0,
            other_end: 2
        }
        .to_string(),
        "(1, 3) overlaps (0, 2): they share a character"
    );
}
