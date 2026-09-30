use pretty_assertions::assert_eq;
use ropey::Rope;
use unicode_segmentation::UnicodeSegmentation;

use super::*;
use crate::grapheme::{
    cluster_end, clusters_before, first_cluster, last_cluster, next_cluster, prev_cluster,
    snap_to_cluster, text_end,
};
use crate::test_support::rope;

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

/// Cluster boundaries of `text` as char offsets, 0 and the text end
/// included, from `unicode-segmentation` over the whole text as one `&str`.
fn boundaries(text: &Rope) -> Vec<usize> {
    let mut out = vec![0];
    let mut chars = 0;
    for g in text.to_string().graphemes(true) {
        chars += g.chars().count();
        out.push(chars);
    }
    out
}

fn floor(b: &[usize], pos: usize) -> usize {
    *b.iter()
        .rev()
        .find(|&&x| x <= pos)
        .expect("0 is a boundary")
}

fn ceil(b: &[usize], pos: usize) -> usize {
    *b.iter()
        .find(|&&x| x >= pos)
        .expect("the text end is a boundary")
}

/// Texts built from every corpus sample, placed where segmentation differs:
/// mid-line, doubled, at a line start, and followed by a lone mark.
fn corpus() -> Vec<Rope> {
    let mut out = Vec::new();
    for sample in test_fixtures::unicode::ALL {
        for text in [
            format!("a{sample}b"),
            format!("{sample}{sample}"),
            format!("\n{sample}\n"),
            format!("{sample}\u{301}x"),
        ] {
            out.push(rope(&text));
        }
    }
    out
}

/// Every cluster start of `text`, walked with the typed steppers.
fn typed_starts(text: &Rope) -> Vec<usize> {
    let slice = text.slice(..);
    std::iter::successors(first_cluster(slice), |&s| next_cluster(slice, s))
        .map(|s| s.offset().index())
        .collect()
}

// ── Steppers ──────────────────────────────────────────────────────────────

#[test]
fn typed_steppers_visit_every_segmentation_boundary() {
    for text in corpus() {
        let b = boundaries(&text);
        let len = text.len_chars();
        assert_eq!(typed_starts(&text), b[..b.len() - 1], "{text:?}");
        assert_eq!(text_end(text.slice(..)).offset(), co(len));
        for &start in &b[..b.len() - 1] {
            let s = snap_to_cluster(text.slice(..), co(start))
                .expect("non-empty text")
                .start();
            assert_eq!(
                cluster_end(text.slice(..), s).offset(),
                co(ceil(&b, start + 1)),
                "{text:?} at {start}"
            );
        }
    }
}

#[test]
fn prev_cluster_steps_back_one_boundary() {
    for text in corpus() {
        let b = boundaries(&text);
        let slice = text.slice(..);
        let mut bound = text_end(slice);
        let mut seen = vec![bound.offset().index()];
        while let Some(prev) = prev_cluster(slice, bound) {
            bound = prev.into();
            seen.push(prev.offset().index());
        }
        seen.reverse();
        assert_eq!(seen, b, "{text:?}");
    }
}

#[test]
fn first_and_last_cluster_bracket_the_text() {
    let text = rope("ab");
    assert_eq!(
        first_cluster(text.slice(..)).map(ClusterStart::offset),
        Some(co(0))
    );
    assert_eq!(
        last_cluster(text.slice(..)).map(ClusterStart::offset),
        Some(co(2))
    );
    let empty = Rope::new();
    assert_eq!(first_cluster(empty.slice(..)), None);
    assert_eq!(last_cluster(empty.slice(..)), None);
}

#[test]
fn clusters_before_walks_the_forward_clusters_in_reverse() {
    for text in corpus() {
        let slice = text.slice(..);
        let forward: Vec<_> = typed_starts(&text);
        let backward: Vec<usize> = clusters_before(slice, text_end(slice))
            .map(|c| c.start().offset().index())
            .collect();
        let mut reversed = forward.clone();
        reversed.reverse();
        assert_eq!(backward, reversed, "{text:?}");
    }
}

#[test]
fn clusters_before_stops_at_the_bound_it_was_given() {
    let text = rope("abc");
    let slice = text.slice(..);
    let c = snap_to_cluster(slice, co(2)).expect("non-empty").start();
    let got: Vec<char> = clusters_before(slice, c.into())
        .map(|c| c.first())
        .collect();
    assert_eq!(got, vec!['b', 'a']);
}

// ── snap_to_cluster ───────────────────────────────────────────────────────

#[test]
fn snap_to_cluster_finds_the_cluster_holding_each_char() {
    for text in corpus() {
        let b = boundaries(&text);
        for pos in 0..text.len_chars() {
            let cluster = snap_to_cluster(text.slice(..), co(pos)).expect("non-empty text");
            assert_eq!(
                cluster.start().offset(),
                co(floor(&b, pos)),
                "{text:?} at {pos}"
            );
            assert_eq!(
                cluster.end().offset(),
                co(ceil(&b, pos + 1)),
                "{text:?} at {pos}"
            );
        }
    }
}

#[test]
fn snap_to_cluster_past_the_end_is_the_last_cluster() {
    let text = rope("ae\u{301}");
    let len = text.len_chars();
    for pos in [len, len + 1, len + 100] {
        let cluster = snap_to_cluster(text.slice(..), co(pos)).expect("non-empty text");
        assert_eq!(cluster.start().offset(), co(len - 1), "at {pos}");
        assert_eq!(cluster.first(), '\n');
    }
}

#[test]
fn snap_to_cluster_on_an_empty_slice_is_none() {
    let empty = Rope::new();
    assert!(snap_to_cluster(empty.slice(..), co(0)).is_none());
}

// ── ClusterRange ──────────────────────────────────────────────────────────

#[test]
fn covering_widens_both_ends_to_whole_clusters() {
    for text in corpus() {
        let b = boundaries(&text);
        let len = text.len_chars();
        for start in 0..len {
            for end in start + 1..=len {
                let range =
                    ClusterRange::covering(text.slice(..), ExclusiveRange::new(co(start), co(end)))
                        .expect("non-empty input");
                let (lo, hi) = (floor(&b, start), ceil(&b, end));
                assert_eq!(
                    range.chars(),
                    ExclusiveRange::new(co(lo), co(hi)),
                    "{text:?} {start}..{end}"
                );
                assert_eq!(range.start().offset(), co(lo));
                assert_eq!(range.end().offset(), co(hi));
                assert_eq!(range.last().offset(), co(floor(&b, hi - 1)));
            }
        }
    }
}

#[test]
fn covering_clamps_past_the_end_and_rejects_empty_input() {
    let text = rope("ab");
    let slice = text.slice(..);
    let range =
        ClusterRange::covering(slice, ExclusiveRange::new(co(1), co(99))).expect("non-empty");
    assert_eq!(range.chars(), ExclusiveRange::new(co(1), co(3)));
    assert_eq!(
        ClusterRange::covering(slice, ExclusiveRange::new(co(1), co(1))),
        None
    );
    assert_eq!(
        ClusterRange::covering(slice, ExclusiveRange::new(co(5), co(9))),
        None
    );
}

#[test]
fn within_narrows_both_ends_to_whole_clusters() {
    for text in corpus() {
        let b = boundaries(&text);
        let len = text.len_chars();
        for start in 0..len {
            for end in start + 1..=len {
                let got =
                    ClusterRange::within(text.slice(..), ExclusiveRange::new(co(start), co(end)));
                let (lo, hi) = (ceil(&b, start), floor(&b, end));
                let expected = (lo < hi).then(|| ExclusiveRange::new(co(lo), co(hi)));
                assert_eq!(
                    got.map(ClusterRange::chars),
                    expected,
                    "{text:?} {start}..{end}"
                );
            }
        }
    }
}

#[test]
fn through_and_between_agree_on_the_same_clusters() {
    let text = rope("xe\u{301}y");
    let slice = text.slice(..);
    let x = snap_to_cluster(slice, co(0)).expect("x").start();
    let e = snap_to_cluster(slice, co(1)).expect("e").start();
    let y = snap_to_cluster(slice, co(3)).expect("y").start();
    let through = ClusterRange::through(slice, x, e).expect("x <= e");
    let between = ClusterRange::between(slice, x, y.into()).expect("x < y");
    assert_eq!(through, between);
    assert_eq!(through.last(), e);
    assert_eq!(through.chars(), ExclusiveRange::new(co(0), co(3)));
    assert_eq!(ClusterRange::through(slice, e, x), None);
    assert_eq!(ClusterRange::between(slice, y, x.into()), None);
    assert_eq!(ClusterRange::between(slice, x, x.into()), None);
}

#[test]
fn contains_and_hull() {
    let text = rope("abcd");
    let slice = text.slice(..);
    let at = |n| snap_to_cluster(slice, co(n)).expect("in text").start();
    let ab = ClusterRange::through(slice, at(0), at(1)).expect("ordered");
    let cd = ClusterRange::through(slice, at(2), at(3)).expect("ordered");
    assert!(ab.contains(at(1)));
    assert!(!ab.contains(at(2)));
    let hull = ab.hull(cd);
    assert_eq!(
        hull,
        ClusterRange::through(slice, at(0), at(3)).expect("ordered")
    );
    assert_eq!(cd.hull(ab), hull);
    assert_eq!(ExclusiveRange::from(hull), hull.chars());
}

#[test]
fn a_single_cluster_range_has_one_start_and_last() {
    let text = rope("e\u{301}");
    let cluster = snap_to_cluster(text.slice(..), co(1)).expect("non-empty");
    let range = cluster.range();
    assert_eq!(range.start(), range.last());
    assert_eq!(range.chars(), ExclusiveRange::new(co(0), co(2)));
}

#[test]
fn a_start_widens_to_the_same_bound() {
    let text = rope("ab");
    let b = snap_to_cluster(text.slice(..), co(1)).expect("b").start();
    assert_eq!(ClusterBound::from(b).offset(), b.offset());
    assert_eq!(CharOffset::from(b), co(1));
    assert_eq!(ClusterBound::TEXT_START.offset(), co(0));
}
