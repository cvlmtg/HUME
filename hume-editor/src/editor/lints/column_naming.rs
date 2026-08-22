//! Column-naming discipline: "column" means five different things in this
//! codebase (a terminal-cell coordinate, a display column, a char index, a
//! grapheme index, a byte offset) — see CLAUDE.md's "Column naming"
//! invariant. A bare `col`/`column` identifier segment says nothing about
//! which one it is; this lint requires every occurrence to carry its kind.
//!
//! `no_untagged_column_identifiers` recursively scans every workspace
//! crate's `src/` — derived from the root `Cargo.toml`'s `members` list, so
//! a renamed or newly added crate can't silently fall out of scope — for a
//! `col`/`cols`/`column`/`columns` identifier *segment* (a snake_case or
//! CamelCase word component) that isn't immediately preceded by one of the
//! four sanctioned kind prefixes: `display`, `char`, `grapheme`, `byte`.
//!
//! **What is not scanned**, all shared with this module's sibling lints:
//! test code (`collect_source_rs` skips any `tests/` directory and any
//! `tests.rs`), this `lints/` directory itself (it holds the patterns), and —
//! within a scanned file — comment text and the contents of string literals.
//! The last two are why a Steel-facing name that only ever appears as a
//! string (`"char-col"`, `"grapheme-col"`) is beyond this lint's reach: those
//! are enforced by the Steel-side tests that consume them, not here.
//! LSP wire positions use `character` (the protocol's own term) instead of
//! a `col` variant entirely; terminal-cell coordinates use the `x`/`y`
//! family instead — neither is scanned for by this lint, since neither
//! contains the word "column" in the first place.
//!
//! **Opt-outs**:
//! - The gutter-*widget* sense of "column" (`GutterColumn`, `SignColumn`,
//!   `LineNumberColumn`, and their close kin) names a vertical lane in the
//!   gutter, not a coordinate — a different concept entirely, whitelisted
//!   by exact identifier below.
//! - `// column-name-safe: <reason>` — same convention as this module's
//!   sibling lints (see `scan_forbidden`'s doc) — exempts a line whose
//!   violation is an upstream name this project doesn't control, e.g.
//!   termina's `MouseEvent::column` or tree-sitter's `Point::column`
//!   (each uses its own, non-display-column convention).

use super::{collect_source_rs, strip_line_comment, workspace_member_crates};

const OPT_OUT_MARKER: &str = "// column-name-safe:";

/// Exact identifiers naming a gutter *widget*, not a coordinate — the full
/// word "Column" in a CamelCase type name is reserved for this sense
/// project-wide (see CLAUDE.md's "Column naming" invariant).
const WHITELIST: &[&str] = &[
    "GutterColumn",
    "SignColumn",
    "LineNumberColumn",
    "SignColumnConfig",
    "SignColumnMode",
    "gutter_columns",
    "add_gutter_column",
    "sign_column",
    "signcolumn",
    "sync_sign_column_width",
];

/// The four sanctioned kind prefixes a `col`/`column` segment must follow.
const ALLOWED_PREFIXES: &[&str] = &["display", "char", "grapheme", "byte"];

/// Whether `seg`, case-insensitively, is one of the four column-shaped words
/// that require a kind prefix.
fn is_column_segment(seg: &str) -> bool {
    matches!(
        seg.to_ascii_lowercase().as_str(),
        "col" | "cols" | "column" | "columns"
    )
}

/// Split an identifier into its snake_case / CamelCase word components.
/// `"GutterColumn"` -> `["Gutter", "Column"]`; `"display_col"` ->
/// `["display", "col"]`; `"DisplayColTarget"` -> `["Display", "Col", "Target"]`.
///
/// Two boundary kinds beyond the plain lower→upper step, both of which hide a
/// column segment from [`is_column_segment`] if missed: an acronym running
/// into a capitalised word (`LSPCol` -> `["LSP", "Col"]`, `HTMLColumn` ->
/// `["HTML", "Column"]`), and a trailing digit run (`col2` -> `["col", "2"]`,
/// so the `col` is still seen as its own segment).
fn segments_of(ident: &str) -> Vec<&str> {
    let mut segs = Vec::new();
    for part in ident.split('_') {
        if part.is_empty() {
            continue;
        }
        let bytes = part.as_bytes();
        let mut start = 0;
        for i in 1..bytes.len() {
            let prev = bytes[i - 1];
            let cur = bytes[i];
            // camelCase/PascalCase: an uppercase letter after a lowercase
            // letter or digit.
            let camel =
                cur.is_ascii_uppercase() && (prev.is_ascii_lowercase() || prev.is_ascii_digit());
            // Acronym tail: the last capital of a run, when a lowercase
            // letter follows — `LSPCol`'s `C` starts a new word, `LS`'s `S`
            // does not.
            let acronym_tail = cur.is_ascii_uppercase()
                && prev.is_ascii_uppercase()
                && bytes.get(i + 1).is_some_and(u8::is_ascii_lowercase);
            // A digit run after letters is its own segment, so a trailing
            // index doesn't fuse onto the word before it.
            let digit_run = cur.is_ascii_digit() && prev.is_ascii_alphabetic();
            if camel || acronym_tail || digit_run {
                segs.push(&part[start..i]);
                start = i;
            }
        }
        segs.push(&part[start..]);
    }
    segs
}

/// Extract every identifier-shaped token from `line`, skipping the contents
/// of double-quoted string literals — a `"col 0"` assert message is prose,
/// not an identifier — the same way `strip_line_comment` skips them when
/// hunting for a real `//`. Char literals (`'x'`, `'\x'`) are skipped too so
/// their quotes don't falsely open/close string tracking.
fn identifiers_outside_strings(line: &str) -> Vec<&str> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' => {
                in_string = true;
                i += 1;
            }
            b'\'' if bytes.get(i + 1) == Some(&b'\\') && bytes.get(i + 3) == Some(&b'\'') => {
                i += 4; // escaped char literal: '\x'
            }
            b'\'' if bytes.get(i + 2) == Some(&b'\'') => {
                i += 3; // char literal: 'x'
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                out.push(&line[start..i]);
            }
            _ => i += 1,
        }
    }
    out
}

/// One untagged-column-identifier hit.
struct Violation {
    file: String,
    lineno: usize,
    ident: String,
}

/// Whether `ident` would be reported — the same test the scan below applies,
/// factored out so it can be exercised directly on names no file contains.
fn is_untagged(ident: &str) -> bool {
    if WHITELIST.contains(&ident) {
        return false;
    }
    let segs = segments_of(ident);
    segs.iter().enumerate().any(|(i, seg)| {
        is_column_segment(seg)
            && !(i > 0 && ALLOWED_PREFIXES.contains(&segs[i - 1].to_ascii_lowercase().as_str()))
    })
}

#[test]
fn untagged_column_identifiers_are_recognised_across_naming_shapes() {
    // Tagged — the four sanctioned prefixes, in both cases.
    for ok in [
        "display_col",
        "char_col",
        "grapheme_col",
        "byte_col",
        "DisplayColTarget",
        "sticky_display_col",
        "char_cols",
        "start_byte_col",
    ] {
        assert!(!is_untagged(ok), "`{ok}` is correctly tagged");
    }

    // Untagged, including the shapes a plain lower→upper split misses: an
    // acronym prefix and a trailing digit.
    for bad in [
        "col",
        "cols",
        "column",
        "columns",
        "my_col",
        "screen_col",
        "colIdx",
        "SelCol",
        "MAX_COL",
        "LSPCol",
        "HTMLColumn",
        "col2",
        "cols2",
        "column1",
        "col_0",
    ] {
        assert!(is_untagged(bad), "`{bad}` must be reported as untagged");
    }

    // Names that merely contain the letters are not segments.
    for unrelated in [
        "collect",
        "colour",
        "protocol",
        "columnar_store",
        "Collapse",
    ] {
        assert!(
            !is_untagged(unrelated),
            "`{unrelated}` is not a column name"
        );
    }

    // The gutter-widget sense stays exempt.
    for widget in ["GutterColumn", "SignColumnConfig", "sign_column"] {
        assert!(!is_untagged(widget), "`{widget}` is a gutter widget");
    }
}

#[test]
fn no_untagged_column_identifiers() {
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
        // Fail loudly on a crate whose `src/` moved: `collect_source_rs`
        // returns silently on an unreadable directory, so a renamed crate
        // would otherwise pass this lint by having nothing to check.
        assert!(
            src_dir.is_dir(),
            "workspace member {c} has no src/ at {} — this lint would silently scan nothing",
            src_dir.display()
        );
        collect_source_rs(&src_dir, &mut paths);
    }
    // This lints/ directory holds the pattern literals scanned for above —
    // excluded so this file never flags itself.
    let lints_dir = workspace_root.join("hume-editor/src/editor/lints");
    paths.retain(|p| !p.starts_with(&lints_dir));

    let mut violations = Vec::new();
    for path in &paths {
        let file = path
            .strip_prefix(workspace_root)
            .unwrap_or(path)
            .display()
            .to_string();
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

        // Previous source line, kept so an opt-out marker cargo fmt hoisted
        // onto its own line still exempts the code beneath it.
        let mut prev_line: &str = "";
        for (lineno, line) in src.lines().enumerate() {
            let prev_for_exempt = prev_line;
            prev_line = line;

            // A *trailing* marker exempts only the line it sits on. Only a
            // marker occupying its whole line reaches down to the next one —
            // otherwise annotating one upstream name would silently exempt
            // whatever happened to follow it.
            let hoisted_above = prev_for_exempt.trim_start().starts_with(OPT_OUT_MARKER);
            if line.contains(OPT_OUT_MARKER) || hoisted_above {
                continue;
            }

            let code = strip_line_comment(line);
            for ident in identifiers_outside_strings(code) {
                if is_untagged(ident) {
                    violations.push(Violation {
                        file: file.clone(),
                        lineno: lineno + 1,
                        ident: ident.to_string(),
                    });
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "\nUntagged `col`/`column` identifier(s) found — every occurrence must be \
         prefixed with its kind (display_col, char_col, grapheme_col, byte_col) or \
         use `character` for LSP wire positions / the `x`-family for terminal cells. \
         See CLAUDE.md's \"Column naming\" invariant.\n\
         Opt out a specific line with `// column-name-safe: <reason>` for an upstream \
         name this project doesn't control (or add it to this lint's WHITELIST if it's \
         the gutter-widget sense of \"column\").\n\
         Violations:\n{}\n",
        violations
            .iter()
            .map(|v| format!("  {}:{} — `{}`", v.file, v.lineno, v.ident))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
