//! # Absent-as-`#f` decode locality
//!
//! Steel represents an optional/absent value as `#f` — `SteelVal::BoolV(false)`.
//! Decoding that convention (`#f` -> `None`, anything else -> `Some(...)`) is
//! `hume-scripting/src/builtins/args.rs`'s `optional_*` family
//! (`optional_string_arg`, `optional_path_arg`, `optional_usize_arg`,
//! `optional_json_arg`, `optional_bid_arg`, `optional_symbol_arg`,
//! `optional_pair_fields`) — one vocabulary, so every builtin's
//! `#f`-means-absent behavior and error wording agree.
//!
//! `absent_marker_is_decoded_only_in_args_rs` recursively scans every
//! workspace crate's `src/` — derived from the root `Cargo.toml`'s `members`
//! list, so a renamed or newly added crate can't silently fall out of scope —
//! excluding `args.rs` itself, for a line that *reads* `SteelVal::BoolV(false)`
//! — a match arm (`=>` follows it on the line) or a `matches!`/`if let` test —
//! rather than *constructs* one as a return value (`Ok(SteelVal::BoolV(false))`,
//! `None => SteelVal::BoolV(false)`, both clean: the marker appears before any
//! `=>` on the line, not after).
//!
//! **What is not scanned**: test code (`collect_source_rs` skips any `tests/`
//! directory and any `tests.rs`) and this `lints/` directory. `args.rs` is
//! excluded too — it's the one file allowed to read the marker.
//!
//! **Opt-out**: `// absent-decode-safe: <reason>` — the one known-legitimate
//! case is `builtins/ui.rs`'s picker-item payload check, which reads `#f` as
//! a *reserved dismiss sentinel to reject*, not an absent optional to unwrap.

use super::{scan_lines, workspace_source_paths};

const OPT_OUT_MARKER: &str = "// absent-decode-safe:";
const MARKER_PATTERN: &str = "BoolV(false)";

/// Whether `code` (one comment-stripped source line) reads the absent-marker
/// pattern rather than merely constructing one as a return value. A `=>`
/// after the marker means it's the pattern side of a match arm; `matches!`/
/// `if let` are the two other shapes a read can take. The same test the scan
/// below applies, factored out so it can be exercised directly on synthetic
/// lines no file contains.
fn reads_absent_marker(code: &str) -> bool {
    if code.contains("matches!") || code.contains("if let") {
        return code.contains(MARKER_PATTERN);
    }
    code.find(MARKER_PATTERN)
        .is_some_and(|idx| code[idx..].contains("=>"))
}

#[test]
fn reads_absent_marker_distinguishes_read_from_construct() {
    for read in [
        "SteelVal::BoolV(false) => Ok(None),",
        "SteelVal::BoolV(false) => None,",
        "if matches!(payload, SteelVal::BoolV(false)) {",
        "if let SteelVal::BoolV(false) = val {",
    ] {
        assert!(reads_absent_marker(read), "`{read}` reads the marker");
    }
    for construct in [
        "None => Ok(SteelVal::BoolV(false)),",
        "None => SteelVal::BoolV(false),",
        "return Ok(SteelVal::BoolV(false));",
        "Ok(SteelVal::BoolV(false))",
    ] {
        assert!(
            !reads_absent_marker(construct),
            "`{construct}` only constructs the marker"
        );
    }
}

/// Fail oracle: hand-roll a `SteelVal::BoolV(false) => None`-shaped decode in
/// any non-test, non-`args.rs` file (e.g. reintroduce the old
/// `diagnostics_for_buffer` decode in `decorations.rs`) and this test names
/// the file and line.
#[test]
fn absent_marker_is_decoded_only_in_args_rs() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
    let root = std::path::Path::new(&manifest);
    let workspace_root = root.parent().expect("workspace root");

    let args_rs = workspace_root.join("hume-scripting/src/builtins/args.rs");
    let paths = workspace_source_paths(workspace_root, &[], &[args_rs]);

    let violations = scan_lines(&paths, workspace_root, OPT_OUT_MARKER, |code| {
        if reads_absent_marker(code) {
            vec![MARKER_PATTERN.to_string()]
        } else {
            Vec::new()
        }
    });

    assert!(
        violations.is_empty(),
        "\n`SteelVal::BoolV(false)` (Steel's #f-means-absent marker) decoded \
         outside args.rs's optional_* family.\n\
         Route the decode through args.rs's matching optional_*_arg helper \
         (add one if none fits) instead of hand-rolling the `#f` check.\n\
         Opt out a specific line with `// absent-decode-safe: <reason>` only \
         when `#f` is a reserved sentinel to reject, not an absent optional \
         (see builtins/ui.rs's picker-item payload check).\n\
         Violations:\n{}\n",
        violations
            .iter()
            .map(|v| format!("  {}:{} — {}", v.file, v.lineno, v.trimmed))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
