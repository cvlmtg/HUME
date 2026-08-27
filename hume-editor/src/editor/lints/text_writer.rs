//! # One text writer for the frame
//!
//! Text reaches the frame through `hume_engine::render::Canvas`'s
//! `write_text_run`/`fill_rect_bg`, never through `hume_grid::Grid`'s own
//! `set_glyph`/`fill_span` — see [`Canvas::write_text_run`]'s own doc for
//! why: the `right_edge` bound a bare cell write has no equivalent of, the
//! placeholder substitution for a cluster the terminal must not be shown as
//! itself, and the per-pane dim, which the canvas applies at its single
//! write point and a direct grid write would skip.
//!
//! [`Canvas::write_text_run`]: hume_engine::render::Canvas::write_text_run
//!
//! The grid primitives are not wrong — they are what the canvas is built on,
//! and they enforce the head/continuation pairing every write depends on.
//! They are simply one layer too low to be called from drawing code, which
//! is the distinction this lint keeps.
//!
//! **Opt-out**: `// static-glyph-safe: <reason>` for a write of text that is
//! a compile-time constant — box-drawing borders, a scrollbar thumb, a
//! literal space — or for the canvas's own primitives, which are the
//! sanctioned callers. Constant glyph text cannot contain a control
//! character or a zero-width cluster, and it is sized by the same code that
//! positions it.
//!
//! Not scanned: `hume-grid`'s own `src/` (the implementation), test code
//! (`collect_source_rs` skips any `tests/` directory and any `tests.rs`),
//! and this `lints/` directory. A test painting into its own scratch grid
//! has no frame to corrupt.

use super::{scan_forbidden, workspace_source_paths};

/// Scan every workspace crate's source for a direct grid write that should
/// instead go through `hume_engine::render::Canvas::write_text_run`.
#[test]
fn text_is_written_through_one_writer() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
    let root = std::path::Path::new(&manifest);
    let workspace_root = root.parent().expect("workspace root");
    // `hume-grid` defines the primitives being restricted; it is the one
    // crate that must be free to call them.
    let paths = workspace_source_paths(workspace_root, &["hume-grid"], &[]);

    // Both the method call and the UFCS spelling are covered —
    // `Grid::set_glyph(&mut grid, …)` bypasses the canvas just as much as
    // `grid.set_glyph(…)` does.
    let forbidden = [
        ".set_glyph(",
        "Grid::set_glyph(",
        ".fill_span(",
        "Grid::fill_span(",
    ];

    let violations: Vec<String> =
        scan_forbidden(&paths, workspace_root, &forbidden, "// static-glyph-safe:")
            .into_iter()
            .map(|v| {
                format!(
                    "  {}:{} — `{}` in: {}",
                    v.file, v.lineno, v.pattern, v.trimmed
                )
            })
            .collect();

    assert!(
        violations.is_empty(),
        "\nDirect grid write detected outside `hume_engine::render::Canvas`.\n\
         A bare `set_glyph`/`fill_span` clips only at the grid's own edge, applies \
         no per-pane dim, and substitutes no placeholder for a cluster the terminal \
         must not be shown as itself — so a field measured with `hume_rope::width` \
         can end up drawn past its pane, lane, or border, or undimmed in an \
         unfocused pane.\n\
         Use `canvas.write_text_run(x, y, text, style, right_edge)` — it advances by \
         exactly `str_width`, takes the bound as an argument, dims at the single \
         write point, and resolves the invisible-placeholder style from the `&Theme` \
         given to `Canvas::new`.\n\
         A write of compile-time-constant glyph text, or the canvas's own \
         primitives, may be annotated `// static-glyph-safe: <reason>`.\n\
         Violations:\n{}\n",
        violations.join("\n")
    );
}
