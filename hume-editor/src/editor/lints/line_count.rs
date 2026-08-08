//! # Line-count derivation discipline
//!
//! Every "how many lines" / "which line is last" / "range of lines"
//! computation must go through `hume-rope`'s six functions
//! (`ropey_line_count`, `last_ropey_line`, `ropey_lines_range`,
//! `content_line_count`, `last_content_line`, `content_lines_range`) — or a
//! `hume_editing::text::Text` method that delegates to one of them — never a
//! raw `len_lines()` call or a manual `+ 1` / `- 1` re-derivation of one of
//! these functions' own result.
//!
//! `no_raw_line_count_derivations` recursively scans every workspace
//! crate's `src/`, excluding `hume-rope/src` (the implementation itself)
//! and this `lints/` directory (where the pattern literals below live), for
//! the forbidden patterns.
//!
//! **Opt-out**: annotate a line with `// line-count-safe: <reason>`.

use super::{collect_source_rs, strip_line_comment};

/// Scan every workspace crate's source for manual line-count derivations
/// that should instead call one of `hume-rope`'s six line-count functions.
///
/// This test reads the source files at compile time, skips test blocks and
/// comment lines, and fails if any active code contains a forbidden
/// derivation pattern.
#[test]
fn no_raw_line_count_derivations() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
    let root = std::path::Path::new(&manifest);
    let workspace_root = root.parent().expect("workspace root");

    // Every workspace crate's src/ except hume-rope, the implementation.
    const CRATES: &[&str] = &[
        "hume-editing/src",
        "hume-engine/src",
        "hume-editor/src",
        "hume-ops/src",
        "hume-treesitter/src",
        "hume-lsp/src",
        "hume-scripting/src",
        "hume-platform/src",
        "hume-test-fixtures/src",
    ];
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for c in CRATES {
        collect_source_rs(&workspace_root.join(c), &mut paths);
    }
    // This lints/ directory holds the pattern literals scanned for below —
    // excluded so this file never flags itself.
    let lints_dir = workspace_root.join("hume-editor/src/editor/lints");
    paths.retain(|p| !p.starts_with(&lints_dir));

    // Forbidden patterns — manual line-count derivations. Bare `- 1`/`+ 1`
    // arithmetic on a line count is not greppable in general, so this
    // enforces the greppable stems: the raw ropey call, and re-deriving one
    // six-function result from another instead of calling the right one
    // directly.
    let forbidden = [
        "len_lines(",
        "content_line_count() - 1",
        "content_line_count().saturating_sub",
        "ropey_line_count() - 1",
        "ropey_line_count().saturating_sub",
        "last_ropey_line() + 1",
        "last_content_line() + 1",
    ];

    let mut violations: Vec<String> = Vec::new();

    for path in &paths {
        let file = path
            .strip_prefix(workspace_root)
            .unwrap_or(path)
            .display()
            .to_string();
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

        // Track whether we are inside a `#[cfg(test)] mod tests { … }` block
        // so we don't flag test-only derivations against raw ropey state.
        let mut in_test_block = false;
        let mut brace_depth: i64 = 0;
        let mut test_entry_depth: i64 = 0;
        let mut saw_cfg_test = false;

        for (lineno, line) in src.lines().enumerate() {
            let trimmed = line.trim();

            if trimmed == "#[cfg(test)]" {
                saw_cfg_test = true;
            }
            if saw_cfg_test && trimmed.starts_with("mod tests") {
                in_test_block = true;
                test_entry_depth = brace_depth;
                saw_cfg_test = false;
            }

            let opens = line.chars().filter(|&c| c == '{').count() as i64;
            let closes = line.chars().filter(|&c| c == '}').count() as i64;
            brace_depth += opens - closes;
            if in_test_block && brace_depth <= test_entry_depth {
                in_test_block = false;
            }

            if in_test_block {
                continue;
            }

            if trimmed.starts_with("//") {
                continue;
            }

            if line.contains("// line-count-safe:") {
                continue;
            }

            let code = strip_line_comment(line);

            for pattern in &forbidden {
                if code.contains(pattern) {
                    violations.push(format!(
                        "  {file}:{} — `{pattern}` in: {trimmed}",
                        lineno + 1,
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "\nManual line-count derivation detected outside hume-rope.\n\
         Use hume_rope's ropey_line_count/last_ropey_line/ropey_lines_range/\n\
         content_line_count/last_content_line/content_lines_range (or a \n\
         Text method that delegates to one) instead.\n\
         Violations:\n{}\n",
        violations.join("\n")
    );
}
