use pretty_assertions::assert_eq;
use ropey::{Rope, RopeSlice};

use super::*;
use crate::test_support::{chunk_straddling_text, segmentation_boundaries};

/// The non-empty chunks of `slice` with their byte and char origins.
fn chunk_origins(slice: RopeSlice<'_>) -> Vec<(&str, usize, usize)> {
    let (mut byte, mut char) = (0, 0);
    let mut out = Vec::new();
    for chunk in slice.chunks() {
        if !chunk.is_empty() {
            out.push((chunk, byte, char));
        }
        byte += chunk.len();
        char += chunk.chars().count();
    }
    out
}

fn position<'a>(cur: &ChunkCursor<'a>) -> (&'a str, usize, usize) {
    (cur.chunk, cur.chunk_byte_start, cur.chunk_char_start)
}

/// The straddling rope whole, and a sub-slice of it starting and ending
/// inside chunks.
fn slices(rope: &Rope) -> [RopeSlice<'_>; 2] {
    let quarter = rope.len_chars() / 4;
    [rope.slice(..), rope.slice(quarter + 1..3 * quarter + 1)]
}

#[test]
fn advance_and_retreat_visit_every_chunk_with_its_origins() {
    let (rope, _) = chunk_straddling_text();
    for slice in slices(&rope) {
        let expected = chunk_origins(slice);
        assert!(expected.len() > 1, "the slice must span several chunks");

        let mut cur = ChunkCursor::at_byte(slice, 0);
        let mut forward = vec![position(&cur)];
        while cur.advance() {
            forward.push(position(&cur));
        }
        assert_eq!(forward, expected);
        assert_eq!(
            position(&cur),
            ("", slice.len_bytes(), slice.len_chars()),
            "advancing past the last chunk reaches the text end"
        );

        let mut backward = Vec::new();
        while cur.retreat() {
            backward.push(position(&cur));
        }
        backward.reverse();
        assert_eq!(backward, expected);
    }
}

#[test]
fn a_seek_lands_on_the_chunk_ropey_reports_for_that_offset() {
    let (rope, _) = chunk_straddling_text();
    for slice in slices(&rope) {
        for byte in (0..slice.len_bytes()).step_by(97) {
            let cur = ChunkCursor::at_byte(slice, byte);
            let (chunk, byte_start, char_start, _) = slice.chunk_at_byte(byte);
            assert_eq!(
                position(&cur),
                (chunk, byte_start, char_start),
                "byte {byte}"
            );
        }
        for char_idx in (0..slice.len_chars()).step_by(89) {
            let (cur, byte) = ChunkCursor::at_char(slice, char_idx);
            let (chunk, byte_start, char_start, _) = slice.chunk_at_char(char_idx);
            assert_eq!(
                position(&cur),
                (chunk, byte_start, char_start),
                "char {char_idx}"
            );
            assert_eq!(byte, slice.char_to_byte(char_idx), "char {char_idx}");
            assert_eq!(cur.byte_to_char(byte), char_idx, "char {char_idx}");
        }
    }
}

#[test]
fn a_seek_to_the_text_end_is_past_the_last_chunk() {
    let (rope, _) = chunk_straddling_text();
    for slice in slices(&rope) {
        let end = ("", slice.len_bytes(), slice.len_chars());
        let mut cur = ChunkCursor::at_byte(slice, slice.len_bytes());
        assert_eq!(position(&cur), end);
        let (at_char, byte) = ChunkCursor::at_char(slice, slice.len_chars());
        assert_eq!((position(&at_char), byte), (end, slice.len_bytes()));

        assert!(!cur.advance());
        assert_eq!(position(&cur), end);
        assert!(cur.retreat());
        let last = *chunk_origins(slice).last().expect("non-empty");
        assert_eq!(position(&cur), last);
    }
}

#[test]
fn an_empty_slice_has_only_the_text_end() {
    let rope = Rope::from_str("abc");
    let slice = rope.slice(1..1);
    let mut cur = ChunkCursor::at_byte(slice, 0);
    assert_eq!(position(&cur), ("", 0, 0));
    assert!(!cur.advance());
    assert!(!cur.retreat());
    assert_eq!(cur.char_after(0), None);
    assert_eq!(cur.char_before(0), None);
    assert_eq!(position(&cur), ("", 0, 0));
}

#[test]
fn peeks_across_a_chunk_edge_read_the_neighbouring_char_and_stay_put() {
    let (rope, _) = chunk_straddling_text();
    for slice in slices(&rope) {
        let text = slice.to_string();
        for (_, byte_start, _) in chunk_origins(slice) {
            for byte in [byte_start, slice.len_bytes()] {
                let mut cur = ChunkCursor::at_byte(slice, byte);
                let before = position(&cur);
                assert_eq!(
                    cur.char_after(byte),
                    text[byte..].chars().next(),
                    "at {byte}"
                );
                assert_eq!(
                    cur.char_before(byte),
                    text[..byte].chars().next_back(),
                    "at {byte}"
                );
                assert_eq!(position(&cur), before, "at {byte}");
            }
        }
    }
}

/// Every probe of `slice` (a char offset), with the boundaries
/// segmentation of the slice's own text puts before and after it.
fn assert_boundaries_match_segmentation(slice: RopeSlice<'_>, probes: &[usize], label: &str) {
    let b = segmentation_boundaries(&slice.to_string());
    let len = slice.len_chars();
    for &pos in probes {
        if pos < len {
            let (mut cur, byte) = ChunkCursor::at_char(slice, pos);
            let next = cur.next_boundary(byte);
            let expected = b.iter().find(|&&x| x > pos).copied().unwrap_or(len);
            assert_eq!(cur.byte_to_char(next), expected, "next at {pos} in {label}");
        }
        if pos > 0 {
            let (mut cur, byte) = ChunkCursor::at_char(slice, pos);
            let prev = cur.prev_boundary(byte);
            let expected = b.iter().rev().find(|&&x| x < pos).copied().unwrap_or(0);
            assert_eq!(cur.byte_to_char(prev), expected, "prev at {pos} in {label}");
        }
    }
}

#[test]
fn boundaries_across_chunk_edges_match_segmentation() {
    let (rope, probes) = chunk_straddling_text();
    assert_boundaries_match_segmentation(rope.slice(..), &probes, "the whole rope");

    let (from, to) = (rope.len_chars() / 4 + 1, 3 * rope.len_chars() / 4 + 1);
    let local: Vec<usize> = probes
        .iter()
        .filter(|&&p| from <= p && p <= to)
        .map(|&p| p - from)
        .collect();
    assert_boundaries_match_segmentation(rope.slice(from..to), &local, "a sub-slice");
}

#[test]
fn boundaries_keep_crlf_whole_at_chunk_edges() {
    for unit in ["\r\n", "x\r\n"] {
        let rope = Rope::from_str(&unit.repeat(2000));
        let mut probes = Vec::new();
        let mut byte = 0;
        for chunk in rope.chunks() {
            byte += chunk.len();
            let at = rope.byte_to_char(byte);
            probes.extend(at.saturating_sub(3)..=(at + 3).min(rope.len_chars()));
        }
        assert_boundaries_match_segmentation(rope.slice(..), &probes, unit);
    }
}
