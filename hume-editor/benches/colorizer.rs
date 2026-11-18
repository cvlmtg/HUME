//! What one `core:colorizer` render costs on the main thread: the
//! `on-viewport-change` hook its viewport debounce fires, alone and with the
//! frame drawn after it, on prose with no literal, on one hex literal and one
//! out-of-gamut `oklch()` per line, on one minified CSS line of several
//! lengths, on a minified line of the default render budget on every line,
//! and on prose lines of several lengths, at a small and a full-screen pane.
//!
//! The `screen_<n>` rows set the default of `'max-render-chars`: each prints
//! the chars its render walks, and the default is that count for the longest
//! lines whose 200x60 row fits a 16 ms frame on the efficiency cores.
//!
//! The plugin skips a render of a text and view it has painted, so the
//! `render` rows delete a char and undo it before each render, outside the
//! timed span: each timed render paints a text it has not seen.
//!
//! A steady render of one viewport sends no new color after its first, so
//! `scroll_render_then_frame` pages down before each render: every timed
//! iteration paints a screen of colors the session has never styled, and so
//! pays for `set-scope-styles!` and the theme re-bake the next frame does.
//! When no page is left below, the editor is rebuilt outside the timed span,
//! so each row is a mean over passes through the file's 5,000 lines.

use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use hume_editor::testing::{ColorizerSample, PluginRenderBench, colorizer_bench};

const MINIFIED_CHARS: [usize; 6] = [500, 1_000, 2_000, 4_000, 8_000, 16_000];

const SCREEN_CHARS: [usize; 5] = [80, 100, 120, 160, 200];

const SIZES: [(u16, u16); 2] = [(80, 25), (200, 60)];

fn samples() -> Vec<(String, ColorizerSample)> {
    let mut samples = vec![
        ("prose".to_string(), ColorizerSample::Prose),
        ("hex".to_string(), ColorizerSample::Hex),
        ("oklch".to_string(), ColorizerSample::Oklch),
    ];
    samples.extend(MINIFIED_CHARS.map(|chars| {
        (
            format!("minified_{chars}"),
            ColorizerSample::Minified(chars),
        )
    }));
    samples.push(("long_lines".to_string(), ColorizerSample::LongLines));
    samples.extend(
        SCREEN_CHARS.map(|chars| (format!("screen_{chars}"), ColorizerSample::Screen(chars))),
    );
    samples
}

/// The time `iters` runs of `timed` take, each after a
/// [`PluginRenderBench::touch`] outside the timed span, which the render
/// would otherwise skip as already painted.
fn touched(
    bench: &mut PluginRenderBench,
    iters: u64,
    timed: fn(&mut PluginRenderBench),
) -> Duration {
    let mut total = Duration::ZERO;
    for _ in 0..iters {
        bench.touch();
        let start = Instant::now();
        timed(bench);
        total += start.elapsed();
    }
    total
}

fn bench_colorizer(c: &mut Criterion) {
    let root = tempfile::tempdir().expect("a temp dir");
    let root = root.path();
    let mut group = c.benchmark_group("colorizer");

    for (sample_name, sample) in samples() {
        for (width, height) in SIZES {
            let id = format!("{sample_name}/{width}x{height}");

            let mut bench = colorizer_bench(root, sample, width, height);
            if let ColorizerSample::Screen(_) = sample {
                eprintln!("walked_chars/{id}: {}", bench.walked_chars());
            }
            group.bench_function(format!("render/{id}"), |b| {
                b.iter_custom(|iters| touched(&mut bench, iters, PluginRenderBench::render));
            });

            let mut bench = colorizer_bench(root, sample, width, height);
            group.bench_function(format!("render_then_frame/{id}"), |b| {
                b.iter_custom(|iters| {
                    touched(&mut bench, iters, PluginRenderBench::render_then_frame)
                });
            });
        }
    }

    for (sample_name, sample) in [
        ("hex", ColorizerSample::Hex),
        ("oklch", ColorizerSample::Oklch),
    ] {
        for (width, height) in SIZES {
            let mut bench = colorizer_bench(root, sample, width, height);
            group.bench_function(
                format!("scroll_render_then_frame/{sample_name}/{width}x{height}"),
                |b| {
                    b.iter_custom(|iters| {
                        let mut total = Duration::ZERO;
                        for _ in 0..iters {
                            if bench.screens_below() == 0 {
                                bench = colorizer_bench(root, sample, width, height);
                            }
                            let scopes = bench.scope_count();
                            let start = Instant::now();
                            bench.scroll_render_then_frame();
                            total += start.elapsed();
                            assert!(
                                bench.scope_count() > scopes,
                                "the scrolled render styled no new color"
                            );
                        }
                        total
                    });
                },
            );
        }
    }

    group.finish();
}

criterion_group!(benches, bench_colorizer);
criterion_main!(benches);
