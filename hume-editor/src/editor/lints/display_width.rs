//! # Display-width computation discipline
//!
//! Every "how many terminal columns does this text occupy" computation must
//! go through `hume_rope::width` (`tab_advance`, `grapheme_width`,
//! `str_width`) — never a direct `unicode-width` call. `hume-engine`'s
//! renderer, `hume-ops`'s editing-ops tab math, and `hume-editor`'s UI
//! chrome all measure display width through that one module rather than
//! each carrying its own convention, which could silently disagree with
//! the others at a tab stop or an unrenderable cluster.
//!
//! `no_raw_display_width` recursively scans every workspace crate's `src/` —
//! derived from the root `Cargo.toml`'s `members` list, so a renamed or
//! newly added crate can't silently fall out of scope — excluding
//! `hume-rope/src/width.rs` (the implementation itself), for direct
//! `unicode-width` use.
//!
//! **What is not scanned**: test code (`collect_source_rs` skips any
//! `tests/` directory and any `tests.rs`) and this `lints/` directory. Tests
//! are deliberately out of scope here — several assert against
//! `unicode-width` directly, as an oracle independent of the code under
//! test. Note also that the forbidden list is `unicode-width`'s own symbols:
//! a *hand-rolled* width computation (re-deriving `tw - col % tw`, or
//! counting `chars()`) violates the same invariant without tripping this
//! lint.
//!
//! **Opt-out**: annotate a line with `// display-width-safe: <reason>` — the
//! known-legitimate cases are `GutterColumn::width`, whose `.width()` is a
//! gutter *cell count*, not a display-width measurement, and
//! `hume_rope::width::Cluster::width()`, whose `.width()` *is* a
//! display-width measurement but reads a decision `classify` already made
//! rather than computing a second one.

use super::{scan_forbidden, workspace_source_paths};

/// Scan every workspace crate's source for direct `unicode-width` use that
/// should instead go through `hume_rope::width`.
#[test]
fn no_raw_display_width() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
    let root = std::path::Path::new(&manifest);
    let workspace_root = root.parent().expect("workspace root");

    // The implementation itself.
    let width_rs = workspace_root.join("hume-rope/src/width.rs");
    let paths = workspace_source_paths(workspace_root, &[], &[width_rs]);

    let forbidden = [
        "unicode_width",
        "UnicodeWidthStr",
        "UnicodeWidthChar",
        "width_cjk",
        ".width(",
    ];

    let violations: Vec<String> =
        scan_forbidden(&paths, workspace_root, &forbidden, "// display-width-safe:")
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
        "\nDirect unicode-width use detected outside hume_rope::width.\n\
         Use hume_rope::width::tab_advance/grapheme_width/str_width instead \
         (or annotate a genuine non-display-width `.width()` call, e.g. a \
         gutter cell count, with `// display-width-safe: <reason>`).\n\
         Violations:\n{}\n",
        violations.join("\n")
    );
}
