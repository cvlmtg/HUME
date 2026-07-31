//! # Grapheme-cluster discipline
//!
//! All position advances in motion and selection code must go through
//! `next_grapheme_boundary` / `prev_grapheme_boundary` — never raw
//! `pos += 1` / `pos -= 1`, which skip over combining codepoints (e.g. `é` =
//! U+0065 + U+0301) instead of advancing a full grapheme cluster.
//!
//! `no_raw_char_stepping_in_motion_code` recursively scans `src/ops/`,
//! `hume-editing/src/lines.rs` + `hume-editing/src/word.rs` for the
//! forbidden patterns.
//!
//! **Opt-out**: annotate a line with `// grapheme-safe: <reason>` (e.g.
//! ASCII-only delimiter scanning, grapheme-boundary-aligned bound conversion).

use super::{collect_source_rs, strip_line_comment};

/// Scan motion-related source files for raw char-level stepping.
///
/// The grapheme cluster invariant (CLAUDE.md) requires that all position
/// advances in motion and selection code go through `next_grapheme_boundary`
/// or `prev_grapheme_boundary` — never raw `pos += 1` / `pos -= 1`.
///
/// The bug that prompted this test: word motions used `pos += 1`, causing
/// combining codepoints (e.g. U+0301, which classify_char sees as Punctuation)
/// to be treated as false word boundaries inside a grapheme cluster.
///
/// This test reads the source files at compile time, skips test blocks and
/// comment lines, and fails if any active code contains a forbidden stepping
/// pattern on a char-position variable.
#[test]
fn no_raw_char_stepping_in_motion_code() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");

    // Collect all non-test source files under src/ops/ plus two standalone files.
    // Using directory traversal so future submodule splits are covered automatically.
    let root = std::path::Path::new(&manifest);
    let workspace_root = root.parent().expect("workspace root");
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    collect_source_rs(&root.join("src/ops"), &mut paths);
    // lines.rs and word.rs live in the editing crate — scan them from there.
    paths.push(workspace_root.join("hume-editing/src/lines.rs"));
    paths.push(workspace_root.join("hume-editing/src/word.rs"));

    // Forbidden patterns — raw +1/-1 steps on char-position variables.
    // Stepping by 1 skips over combining codepoints (e.g. é = U+0065 + U+0301)
    // instead of advancing by a full grapheme cluster.
    //
    // Assignment forms: caught directly.
    // char_at() forms: explicitly forbidden by CLAUDE.md — char_at(pos + 1) and
    //   char_at(pos - 1) were the original motivating footguns.
    let forbidden = [
        // ── Assignment forms ───────────────────────────────────────────────
        "pos += 1",
        "pos -= 1",
        "start += 1",
        "start -= 1",
        "end += 1",
        "end -= 1",
        "head += 1",
        "head -= 1",
        "anchor += 1",
        "anchor -= 1",
        // ── char_at() expression forms ─────────────────────────────────────
        "char_at(pos + 1)",
        "char_at(pos - 1)",
        "char_at(head + 1)",
        "char_at(head - 1)",
        "char_at(anchor + 1)",
        "char_at(anchor - 1)",
    ];

    let mut violations: Vec<String> = Vec::new();

    for path in &paths {
        let file = path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string();
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

        // Track whether we are inside a `#[cfg(test)] mod tests { … }` block
        // so we don't flag historical references in test comments.
        let mut in_test_block = false;
        let mut brace_depth: i64 = 0;
        let mut test_entry_depth: i64 = 0;
        let mut saw_cfg_test = false;

        for (lineno, line) in src.lines().enumerate() {
            let trimmed = line.trim();

            // Detect `#[cfg(test)]` on its own line.
            if trimmed == "#[cfg(test)]" {
                saw_cfg_test = true;
            }
            // The very next `mod tests` after that attribute opens the block.
            if saw_cfg_test && trimmed.starts_with("mod tests") {
                in_test_block = true;
                test_entry_depth = brace_depth;
                saw_cfg_test = false;
            }

            // Track brace depth so we know when the test block closes.
            let opens = line.chars().filter(|&c| c == '{').count() as i64;
            let closes = line.chars().filter(|&c| c == '}').count() as i64;
            brace_depth += opens - closes;
            if in_test_block && brace_depth <= test_entry_depth {
                in_test_block = false;
            }

            // Skip everything inside the test module.
            if in_test_block {
                continue;
            }

            // Skip pure comment lines.
            if trimmed.starts_with("//") {
                continue;
            }

            // `// grapheme-safe: <reason>` opt-out: lines where raw +1/-1 is
            // intentional and safe (e.g. ASCII-only delimiter arithmetic, or
            // converting a grapheme-boundary-aligned exclusive end to inclusive).
            // The reason after the colon must explain *why* it is safe.
            if line.contains("// grapheme-safe:") {
                continue;
            }

            // Strip any remaining inline comment before pattern-matching.
            // This prevents explanatory comments like `// was: pos += 1` from
            // triggering false positives.
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
        "\nRaw char-level stepping detected in motion/selection code.\n\
         Use next_grapheme_boundary(buf, pos) or prev_grapheme_boundary(buf, pos) instead.\n\
         Violations:\n{}\n",
        violations.join("\n")
    );
}
