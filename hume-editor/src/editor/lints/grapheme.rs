//! # Grapheme-cluster discipline
//!
//! All position advances in motion and selection code must go through
//! `next_grapheme_boundary` / `prev_grapheme_boundary` — never raw
//! `pos += 1` / `pos -= 1`, which skip over combining codepoints (e.g. `é` =
//! U+0065 + U+0301) instead of advancing a full grapheme cluster.
//!
//! `no_raw_char_stepping_in_motion_code` recursively scans `hume-ops/src/`,
//! `hume-editing/src/lines.rs`, `hume-editing/src/word.rs` +
//! `hume-rope/src/lines.rs` for the forbidden patterns.
//!
//! **Opt-out**: annotate a line with `// grapheme-safe: <reason>` (e.g.
//! ASCII-only delimiter scanning, grapheme-boundary-aligned bound conversion).

use super::{collect_source_rs, scan_forbidden};

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

    // Collect all non-test source files under hume-ops/src/ plus two
    // standalone files. Using directory traversal so future submodule splits
    // are covered automatically.
    let root = std::path::Path::new(&manifest);
    let workspace_root = root.parent().expect("workspace root");
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    collect_source_rs(&workspace_root.join("hume-ops/src"), &mut paths);
    // lines.rs and word.rs live in the editing crate — scan them from there.
    paths.push(workspace_root.join("hume-editing/src/lines.rs"));
    paths.push(workspace_root.join("hume-editing/src/word.rs"));
    // The line-boundary helpers that walk grapheme boundaries
    // (snap_to_grapheme_boundary, line_content_end, place_char_column) moved
    // to hume-rope — scan their new home too. hume-rope/src/grapheme.rs (the
    // boundary-detection implementation itself) and cursor.rs (CharCursor,
    // deliberately char-level) stay out of scope, same as before the move.
    paths.push(workspace_root.join("hume-rope/src/lines.rs"));

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

    // `// grapheme-safe: <reason>` opt-out: lines where raw +1/-1 is
    // intentional and safe (e.g. ASCII-only delimiter arithmetic, or
    // converting a grapheme-boundary-aligned exclusive end to inclusive).
    // The reason after the colon must explain *why* it is safe.
    let violations: Vec<String> =
        scan_forbidden(&paths, workspace_root, &forbidden, "// grapheme-safe:")
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
        "\nRaw char-level stepping detected in motion/selection code.\n\
         Use next_grapheme_boundary(buf, pos) or prev_grapheme_boundary(buf, pos) instead.\n\
         Violations:\n{}\n",
        violations.join("\n")
    );
}
