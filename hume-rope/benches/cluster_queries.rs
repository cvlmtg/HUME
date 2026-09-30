use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use hume_rope::cluster::{ClusterBound, ClusterRange, ClusterStart};
use hume_rope::column::BufferLineCol;
use hume_rope::grapheme::{
    char_pos_at_display_col, cluster_end, clusters_before, display_col_in_line, graphemes_at,
    prev_cluster, snap_to_cluster,
};
use hume_rope::line::ContentLine;
use hume_rope::offset::{CharOffset, ExclusiveRange};
use ropey::Rope;
use test_fixtures::unicode::{ALL, CJK, COMBINING, ZWJ_FAMILY};

const MIB: usize = 1 << 20;
const OFFSETS: usize = 64;
const RANGES: usize = 200;
const WALK: usize = 2000;

/// A text with the structural trailing `\n` every buffer has.
fn rope(s: &str) -> Rope {
    let mut r = Rope::from_str(s);
    if !s.ends_with('\n') {
        r.insert_char(r.len_chars(), '\n');
    }
    r
}

/// The start of the cluster holding each of `offsets`.
fn cluster_starts(slice: ropey::RopeSlice<'_>, offsets: &[CharOffset]) -> Vec<ClusterStart> {
    offsets
        .iter()
        .filter_map(|&at| snap_to_cluster(slice, at).map(|cluster| cluster.start()))
        .collect()
}

/// About 1 MiB of code-like ASCII lines, each ending in a corpus sample.
fn mixed() -> Rope {
    let mut s = String::new();
    for sample in ALL.iter().cycle() {
        if s.len() >= MIB {
            break;
        }
        s.push_str("    let value = compute(left, right, 42); // ");
        s.push_str(sample);
        s.push('\n');
    }
    rope(&s)
}

fn ascii() -> Rope {
    let line = "    let value = compute(left, right, 42); // plain comment\n";
    rope(&line.repeat(MIB / line.len()))
}

/// One line of about 4000 clusters, mixing narrow, wide and multi-char ones.
fn long_line() -> Rope {
    rope(&format!("ab{COMBINING}{CJK}{ZWJ_FAMILY} ").repeat(4000 / 6))
}

fn stride_offsets(text: &Rope) -> Vec<CharOffset> {
    (0..OFFSETS)
        .map(|i| CharOffset::new(i * text.len_chars() / OFFSETS))
        .collect()
}

/// For each stride offset, the second char of the first multi-char cluster
/// at or after it.
fn mid_cluster_offsets(text: &Rope) -> Vec<CharOffset> {
    let slice = text.slice(..);
    stride_offsets(text)
        .into_iter()
        .map(|at| {
            let from = snap_to_cluster(slice, at).expect("non-empty").start();
            let cluster = graphemes_at(slice, from.into())
                .find(|c| c.end().offset().chars_since(c.start().offset()) > 1)
                .expect("the mixed text repeats multi-char clusters");
            cluster.start().offset().shift(1)
        })
        .collect()
}

/// Small char ranges scattered over `text`, the shape of a search's matches.
fn scattered_ranges(text: &Rope) -> Vec<ExclusiveRange<CharOffset>> {
    (0..RANGES)
        .map(|i| {
            let start = i * (text.len_chars() - 32) / RANGES;
            let len = 1 + (i * 7) % 30;
            ExclusiveRange::new(CharOffset::new(start), CharOffset::new(start + len))
        })
        .collect()
}

fn mid_bound(text: &Rope) -> ClusterBound {
    let slice = text.slice(..);
    snap_to_cluster(slice, CharOffset::new(text.len_chars() / 2))
        .expect("non-empty")
        .start()
        .into()
}

fn point_queries(c: &mut Criterion) {
    let ascii = ascii();
    let mixed = mixed();
    let ascii_offsets = stride_offsets(&ascii);
    let mixed_offsets = stride_offsets(&mixed);
    let mid_offsets = mid_cluster_offsets(&mixed);

    c.bench_function("snap/ascii", |b| {
        let slice = ascii.slice(..);
        b.iter(|| {
            for &at in &ascii_offsets {
                black_box(snap_to_cluster(slice, black_box(at)));
            }
        })
    });
    c.bench_function("snap/unicode_mid", |b| {
        let slice = mixed.slice(..);
        b.iter(|| {
            for &at in &mid_offsets {
                black_box(snap_to_cluster(slice, black_box(at)));
            }
        })
    });
    c.bench_function("boundary/next", |b| {
        let slice = mixed.slice(..);
        let starts = cluster_starts(slice, &mixed_offsets);
        b.iter(|| {
            for &at in &starts {
                black_box(cluster_end(slice, black_box(at)));
            }
        })
    });
    c.bench_function("boundary/prev", |b| {
        let slice = mixed.slice(..);
        let starts = cluster_starts(slice, &mixed_offsets);
        b.iter(|| {
            for &at in &starts {
                black_box(prev_cluster(slice, black_box(at.into())));
            }
        })
    });
}

fn range_queries(c: &mut Criterion) {
    let mixed = mixed();
    let ranges = scattered_ranges(&mixed);
    let byte_ranges: Vec<_> = ranges
        .iter()
        .map(|r| mixed.char_to_byte(r.start.index())..mixed.char_to_byte(r.end.index()))
        .collect();

    c.bench_function("range/covering", |b| {
        let slice = mixed.slice(..);
        b.iter(|| {
            for &range in &ranges {
                black_box(ClusterRange::covering(slice, black_box(range)));
            }
        })
    });
    c.bench_function("range/covering_bytes", |b| {
        let slice = mixed.slice(..);
        b.iter(|| {
            for range in &byte_ranges {
                black_box(ClusterRange::covering_bytes(
                    slice,
                    black_box(range.clone()),
                ));
            }
        })
    });
    c.bench_function("range/within", |b| {
        let slice = mixed.slice(..);
        b.iter(|| {
            for &range in &ranges {
                black_box(ClusterRange::within(slice, black_box(range)));
            }
        })
    });
}

fn walks(c: &mut Criterion) {
    let mixed = mixed();
    let from = mid_bound(&mixed);

    c.bench_function("walk/graphemes_forward", |b| {
        let slice = mixed.slice(..);
        b.iter(|| black_box(graphemes_at(slice, black_box(from)).take(WALK).count()))
    });
    c.bench_function("walk/clusters_before", |b| {
        let slice = mixed.slice(..);
        b.iter(|| black_box(clusters_before(slice, black_box(from)).take(WALK).count()))
    });
}

fn line_columns(c: &mut Criterion) {
    let line = long_line();
    let slice = line.slice(..);
    let first = ContentLine::new(0);
    let last_char = CharOffset::new(line.len_chars() - 1);
    let width = display_col_in_line(slice, first, last_char, 4);
    let target = BufferLineCol::new(width.get() - 1);

    c.bench_function("line/display_col", |b| {
        b.iter(|| black_box(display_col_in_line(slice, first, black_box(last_char), 4)))
    });
    c.bench_function("line/char_pos_at_col", |b| {
        b.iter(|| black_box(char_pos_at_display_col(slice, first, black_box(target), 4)))
    });
}

criterion_group!(benches, point_queries, range_queries, walks, line_columns);
criterion_main!(benches);
