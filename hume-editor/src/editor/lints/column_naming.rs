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
fn segments_of(ident: &str) -> Vec<&str> {
    let mut segs = Vec::new();
    for part in ident.split('_') {
        if part.is_empty() {
            continue;
        }
        let bytes = part.as_bytes();
        let mut start = 0;
        for i in 1..bytes.len() {
            // A camelCase/PascalCase word boundary: an uppercase letter
            // immediately following a lowercase letter or digit.
            let boundary = bytes[i].is_ascii_uppercase()
                && (bytes[i - 1].is_ascii_lowercase() || bytes[i - 1].is_ascii_digit());
            if boundary {
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
        collect_source_rs(&workspace_root.join(c).join("src"), &mut paths);
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

        // Previous source line, kept so an opt-out marker on the line
        // *above* a violation suppresses it (cargo fmt can hoist a
        // trailing comment onto its own line above the code it annotates).
        let mut prev_line: &str = "";
        for (lineno, line) in src.lines().enumerate() {
            let prev_for_exempt = prev_line;
            prev_line = line;

            if line.contains(OPT_OUT_MARKER) || prev_for_exempt.contains(OPT_OUT_MARKER) {
                continue;
            }

            let code = strip_line_comment(line);
            for ident in identifiers_outside_strings(code) {
                if WHITELIST.contains(&ident) {
                    continue;
                }
                let segs = segments_of(ident);
                for (i, seg) in segs.iter().enumerate() {
                    if !is_column_segment(seg) {
                        continue;
                    }
                    let tagged = i > 0
                        && ALLOWED_PREFIXES.contains(&segs[i - 1].to_ascii_lowercase().as_str());
                    if !tagged {
                        violations.push(Violation {
                            file: file.clone(),
                            lineno: lineno + 1,
                            ident: ident.to_string(),
                        });
                        break; // one report per identifier is enough
                    }
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
