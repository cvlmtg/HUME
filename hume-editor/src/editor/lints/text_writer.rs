//! # One text writer for the frame
//!
//! Text is written to the terminal buffer through `hume_engine::render::Canvas`
//! — its `write_text_run`/`fill_rect_bg` methods outside `hume-engine`, or the
//! `pub(crate)` free functions of the same names inside it — never with
//! ratatui's `Buffer::set_string`/`set_stringn`.
//!
//! The two measure differently. `set_string` discards a grapheme holding a
//! control character or measuring zero columns, and `cell_width` reports any
//! single-byte string as 1 without consulting `unicode-width` and adds a cell
//! per halfwidth dakuten. HUME sizes every field with `hume_rope::width`
//! instead, so a caller that measured with `str_width` and drew with
//! `set_string` could reserve columns nothing was drawn in, or draw wider
//! than it reserved — and the two conventions drift independently, since one
//! of them lives in a dependency. `write_text_run` walks by `grapheme_width`,
//! which makes its advance exactly `str_width`: measurement and drawing are
//! one model and cannot disagree.
//!
//! It also takes a `right_edge`, which `set_string` has no equivalent of — it
//! clips at the terminal buffer and nothing narrower. That gap was live: menu
//! rows arrive untruncated and the box is clamped to the pane, so a long
//! completion label was written over its own right border and past it.
//!
//! A cluster the terminal must not be shown as itself (a control character,
//! or one measuring zero columns) draws as its `<200b>`-style codepoint
//! placeholder, styled `ui.virtual.invisible` — `Canvas` resolves that once
//! from the `&Theme` passed to `Canvas::new`, so no caller threads a style
//! for it.
//!
//! **Opt-out**: `// static-glyph-safe: <reason>` for a write of text that is
//! a compile-time constant — box-drawing borders, a scrollbar thumb, a
//! literal space. Those cannot contain a control character, a zero-width
//! cluster, or anything ratatui measures differently, and they are sized by
//! the same code that positions them.
//!
//! Not scanned: test code (`collect_source_rs` skips any `tests/` directory
//! and any `tests.rs`) and this `lints/` directory. A test painting into its
//! own scratch buffer has no frame to corrupt.

use super::{collect_source_rs, scan_forbidden, workspace_member_crates};

/// Scan every workspace crate's source for a ratatui string write that should
/// instead go through `hume_engine::render::Canvas::write_text_run`.
#[test]
fn text_is_written_through_one_writer() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
    let root = std::path::Path::new(&manifest);
    let workspace_root = root.parent().expect("workspace root");

    let crates = workspace_member_crates(workspace_root);
    assert!(
        !crates.is_empty(),
        "workspace_member_crates found no members — Cargo.toml parsing broke"
    );
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for c in &crates {
        let src_dir = workspace_root.join(c).join("src");
        assert!(
            src_dir.is_dir(),
            "workspace member {c:?} has no src/ dir at {src_dir:?}"
        );
        collect_source_rs(&src_dir, &mut paths);
    }
    // This lints/ directory holds the pattern literals scanned for below —
    // excluded so this file never flags itself.
    let lints_dir = workspace_root.join("hume-editor/src/editor/lints");
    paths.retain(|p| !p.starts_with(&lints_dir));

    // `set_symbol`/`set_char` write one cell straight through the `Cell` API,
    // bypassing both writers — caught here too, since the reason to reach for
    // them is the same.
    let forbidden = [
        ".set_string(",
        ".set_stringn(",
        ".set_symbol(",
        ".set_char(",
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
        "\nRatatui text write detected outside `hume_engine::render::Canvas`.\n\
         `set_string` measures by its own rule (it drops control and zero-width \
         graphemes, and widens a halfwidth dakuten) and clips only at the terminal \
         buffer, so a field measured with `hume_rope::width` can end up drawn at a \
         different width or past its pane, lane, or border.\n\
         Use `canvas.write_text_run(x, y, text, style, right_edge)` — it advances by \
         exactly `str_width`, takes the bound as an argument, and resolves the \
         invisible-placeholder style from the `&Theme` given to `Canvas::new`.\n\
         A write of compile-time-constant glyph text may be annotated \
         `// static-glyph-safe: <reason>`.\n\
         Violations:\n{}\n",
        violations.join("\n")
    );
}
