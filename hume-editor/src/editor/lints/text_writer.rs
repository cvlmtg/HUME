//! # One text writer for the frame
//!
//! Text is written to the terminal buffer through `hume_engine::render::Canvas`'s
//! `write_text_run`/`fill_rect_bg` methods, never ratatui's
//! `Buffer::set_string`/`set_stringn` — see [`Canvas::write_text_run`]'s own
//! doc for why: the measurement/draw model the two disagree on, the
//! `right_edge` bound `set_string` has no equivalent of, and the placeholder
//! substitution for a cluster the terminal must not be shown as itself.
//!
//! [`Canvas::write_text_run`]: hume_engine::render::Canvas::write_text_run
//!
//! `ratatui::widgets`/`ratatui::text` are banned outright, not just their
//! individual string-write methods: `Paragraph`, `Block`, `List`, `Tabs`,
//! and `Line`/`Span` each measure and draw text through ratatui's own rule,
//! the same gap as `set_string` — nothing in this workspace has a
//! legitimate reason to import from either module.
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

use super::{scan_forbidden, workspace_source_paths};

/// Scan every workspace crate's source for a ratatui string write that should
/// instead go through `hume_engine::render::Canvas::write_text_run`.
#[test]
fn text_is_written_through_one_writer() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
    let root = std::path::Path::new(&manifest);
    let workspace_root = root.parent().expect("workspace root");
    let paths = workspace_source_paths(workspace_root, &[], &[]);

    // `set_symbol`/`set_char` write one cell straight through the `Cell` API,
    // bypassing both writers — caught here too, since the reason to reach for
    // them is the same. Both a method call and UFCS spelling are covered
    // (`Buffer::set_string(&mut buf, …)` bypasses just as much as
    // `buf.set_string(…)` does). `ratatui::widgets`/`ratatui::text` (the
    // module a `use` statement would name first) are banned outright: those
    // widgets (`Paragraph`, `Block`, `List`, `Tabs`, `Line`/`Span`) measure
    // and draw text with their own rule, and nothing in this workspace has
    // a legitimate reason to import one — see this module's doc.
    let forbidden = [
        ".set_string(",
        "Buffer::set_string(",
        ".set_stringn(",
        "Buffer::set_stringn(",
        ".set_symbol(",
        "Cell::set_symbol(",
        ".set_char(",
        "Cell::set_char(",
        "ratatui::widgets",
        "ratatui::text",
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
