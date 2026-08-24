//! # Line-count derivation discipline
//!
//! Every "how many lines" / "which line is last" / "range of lines"
//! computation must go through `hume-rope`'s six functions
//! (`ropey_line_count`, `last_ropey_line`, `ropey_lines_range`,
//! `content_line_count`, `last_content_line`, `content_lines_range`) — or a
//! `hume_editing::text::Text` method that delegates to one of them — never a
//! raw `len_lines()` call or a manual `+ 1` / `- 1` re-derivation of one of
//! these functions' own result. Writing a whole-buffer range out as
//! `0..<one of the counts>` is that same re-derivation — the two
//! `*_lines_range` functions already are that range. The rule also covers the
//! char offset of a line's own line-break: `line_end_exclusive(buf, line) - 1`
//! must be `hume_rope::lines::line_break_char(buf, line)` instead.
//!
//! `no_raw_line_count_derivations` recursively scans every workspace
//! crate's `src/` — derived from the root `Cargo.toml`'s `members` list, so
//! a renamed or newly added crate can't silently fall out of scope —
//! excluding `hume-rope/src` (the implementation itself) and this `lints/`
//! directory (where the pattern literals below live), for the forbidden
//! patterns.
//!
//! **Opt-out**: annotate a line with `// line-count-safe: <reason>`.

use super::{
    collect_all_rs, scan_forbidden, strip_line_comment, workspace_member_crates,
    workspace_source_paths,
};

/// Recursively collect every `.rs` file under `dir` that [`collect_source_rs`]
/// excludes: anything under a directory named `tests`, or a file named
/// `tests.rs`. Used to widen this lint's `len_lines(` and `0..<count>` checks
/// into test code (see below) — that excluded set is exactly where the wrong
/// ropey/content-domain phantom-line convention tends to get hand-encoded.
fn collect_test_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let n = name.to_string_lossy();
        if path.is_dir() {
            if n == "tests" {
                collect_all_rs(&path, out);
            } else {
                collect_test_rs(&path, out);
            }
        } else if path.is_file() && n == "tests.rs" {
            out.push(path);
        }
    }
}

/// The line-count calls a zero-based range can be built over, paired with the
/// range function that already is that range.
const ZERO_BASED_RANGE_STEMS: [(&str, &str); 2] = [
    ("ropey_line_count(", "ropey_lines_range"),
    ("content_line_count(", "content_lines_range"),
];

/// The range function `code` re-derives by spelling `0..<a line count>`, if it
/// does. `ropey_lines_range`/`content_lines_range` *are* that range, so writing
/// it out is the same re-derivation the fixed patterns below forbid — but the
/// receiver varies per call site (`0..ropey_line_count(rope)`,
/// `0..text.ropey_line_count()`, `0..doc.text().ropey_line_count()`), so it
/// can't be one fixed substring.
///
/// Requires the `0..` to reach the call through nothing but a receiver chain,
/// so a range that merely shares a line with a count call (`v[0..2]` beside
/// one) isn't flagged.
fn zero_based_line_count_range(code: &str) -> Option<&'static str> {
    ZERO_BASED_RANGE_STEMS.iter().find_map(|(stem, range_fn)| {
        // Every occurrence, not just the first: one call on a line can be a
        // plain bound while a later one on the same line opens a range.
        code.match_indices(stem)
            .any(|(stem_at, _)| {
                let before = &code[..stem_at];
                before.rfind("0..").is_some_and(|zero_at| {
                    before[zero_at + "0..".len()..].chars().all(|c| {
                        // Receiver chain or module path — `text.`, `doc.text().`,
                        // `hume_rope::lines::`. A separator that can't appear in
                        // one (a space, `;`, `,`, `]`) means the `0..` belongs to
                        // some other range that merely shares the line.
                        c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '(' | ')')
                    })
                })
            })
            .then_some(*range_fn)
    })
}

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
    let paths = workspace_source_paths(workspace_root, &["hume-rope"], &[]);
    // Re-derived (not read back out of `paths`) for the test-tree scan
    // below, which walks `collect_test_rs` — a different traversal than
    // `workspace_source_paths`'s own `collect_source_rs` — over the same
    // crate set.
    let crates: Vec<String> = workspace_member_crates(workspace_root)
        .into_iter()
        .filter(|c| c != "hume-rope")
        .collect();
    let lints_dir = workspace_root.join("hume-editor/src/editor/lints");

    // Forbidden patterns — manual line-count derivations. Bare `- 1`/`+ 1`
    // arithmetic on a line count stored in a local variable first is not
    // greppable at all, so this enforces the greppable stems: the raw ropey
    // call, `line_to_char(line + 1)` (a manual `line_end_exclusive`), and
    // re-deriving one six-function result from another (both operand orders,
    // both `+ 1` and `.saturating_add(1)`) instead of calling the right one
    // directly.
    let forbidden = [
        "len_lines(",
        "line_to_char(line + 1)",
        "content_line_count() - 1",
        "content_line_count().saturating_sub",
        "ropey_line_count() - 1",
        "ropey_line_count().saturating_sub",
        "last_ropey_line() + 1",
        "last_ropey_line().saturating_add(1)",
        "1 + last_ropey_line()",
        "last_content_line() + 1",
        "last_content_line().saturating_add(1)",
        "1 + last_content_line()",
    ];

    let mut violations: Vec<String> =
        scan_forbidden(&paths, workspace_root, &forbidden, "// line-count-safe:")
            .into_iter()
            .map(|v| {
                format!(
                    "  {}:{} — `{}` in: {}",
                    v.file, v.lineno, v.pattern, v.trimmed
                )
            })
            .collect();

    // `line_end_exclusive(buf, line) - 1` — the char position of `line`'s own
    // line-break — is a re-derivation like the twelve above, but its argument
    // is a per-call-site variable, so it can't be one fixed substring. Scan
    // for the stem, then keep only hits that also spell a trailing `- 1` or
    // `.saturating_sub(1)` — `hume_rope::lines::line_break_char` is the single
    // implementation; a bare `line_end_exclusive` call with no subtraction is
    // legitimate (it wants the *next* line's start, not this line's break).
    violations.extend(
        scan_forbidden(
            &paths,
            workspace_root,
            &["line_end_exclusive("],
            "// line-count-safe:",
        )
        .into_iter()
        .filter(|v| {
            let code = strip_line_comment(&v.trimmed);
            code.contains("- 1") || code.contains(".saturating_sub(1)")
        })
        .map(|v| {
            format!(
                "  {}:{} — `line_end_exclusive(...) - 1` (use hume_rope::lines::line_break_char) in: {}",
                v.file, v.lineno, v.trimmed
            )
        }),
    );

    // `collect_source_rs` drops every `tests.rs` file and `tests/` directory
    // so the rest of this lint stays test-exempt (independent-oracle
    // assertions legitimately compare a wrapper's result against ropey's own
    // raw count). But tests are exactly where the wrong ropey/content-domain
    // phantom-line convention tends to get hand-encoded (a loop bound, a
    // fixture's expected row count), so the `len_lines(` stem alone is
    // checked there too.
    let mut test_paths: Vec<std::path::PathBuf> = Vec::new();
    for c in &crates {
        collect_test_rs(&workspace_root.join(c).join("src"), &mut test_paths);
    }
    test_paths.retain(|p| !p.starts_with(&lints_dir));
    violations.extend(
        scan_forbidden(
            &test_paths,
            workspace_root,
            &["len_lines("],
            "// line-count-safe:",
        )
        .into_iter()
        .map(|v| {
            format!(
                "  {}:{} — `{}` in: {}",
                v.file, v.lineno, v.pattern, v.trimmed
            )
        }),
    );

    // `0..<a line count>` re-derives `ropey_lines_range`/`content_lines_range`,
    // which already are that range. Same stem-plus-qualifier shape as the
    // `line_end_exclusive` pass above, and run over test files too for the
    // same reason `len_lines(` is: a hand-written loop bound is where the
    // phantom-line convention gets silently picked.
    let count_stems: Vec<&'static str> = ZERO_BASED_RANGE_STEMS.iter().map(|(s, _)| *s).collect();
    let ranged_paths: Vec<std::path::PathBuf> =
        paths.iter().chain(test_paths.iter()).cloned().collect();
    violations.extend(
        scan_forbidden(
            &ranged_paths,
            workspace_root,
            &count_stems,
            "// line-count-safe:",
        )
        .into_iter()
        .filter_map(|v| {
            let range_fn = zero_based_line_count_range(strip_line_comment(&v.trimmed))?;
            Some(format!(
                "  {}:{} — `0..{}()` (use hume_rope::lines::{range_fn}) in: {}",
                v.file,
                v.lineno,
                v.pattern.trim_end_matches('('),
                v.trimmed
            ))
        }),
    );

    assert!(
        violations.is_empty(),
        "\nManual line-count derivation detected outside hume-rope.\n\
         Use hume_rope's ropey_line_count/last_ropey_line/ropey_lines_range/\n\
         content_line_count/last_content_line/content_lines_range (or a \n\
         Text method that delegates to one) instead — including for a whole-\n\
         buffer range, where `0..<a count>` is the *_lines_range function.\n\
         Violations:\n{}\n",
        violations.join("\n")
    );
}

/// Pins the receiver shapes `zero_based_line_count_range` must reach through,
/// and the near-misses it must not claim. The scan above routes through this
/// same function, so the two can't drift apart.
#[test]
fn zero_based_line_count_ranges_are_recognised_through_a_receiver_chain() {
    // Fail oracle: drop the between-chars check (accept any `0..` earlier on
    // the line) and the two indexing negatives below start failing.
    // Each positive also pins *which* range function is named as the fix.
    for (code, range_fn) in [
        ("0..ropey_line_count(rope)", "ropey_lines_range"),
        (
            "for line in 0..text.ropey_line_count() {",
            "ropey_lines_range",
        ),
        (
            "let rows = 0..doc.text().ropey_line_count();",
            "ropey_lines_range",
        ),
        (
            "for line_idx in 0..hume_rope::lines::ropey_line_count(&rope) {",
            "ropey_lines_range",
        ),
        // A plain bound first, the range second — the second occurrence counts.
        (
            "f(a.ropey_line_count(), 0..b.ropey_line_count())",
            "ropey_lines_range",
        ),
        ("0..self.rope.content_line_count()", "content_lines_range"),
    ] {
        assert_eq!(
            zero_based_line_count_range(code),
            Some(range_fn),
            "missed a zero-based line-count range: {code}"
        );
    }

    for code in [
        // A range that isn't over the count — it just shares the line.
        "assert_eq!(v[0..2], text.content_line_count())",
        "assert_eq!(&rows[0..2], text.ropey_line_count())",
        // A count used as a bound, not as a range end.
        "let total = buf.ropey_line_count();",
        "Vec::with_capacity(text.ropey_line_count())",
        // The range functions themselves, and a range that isn't zero-based.
        "for line in text.content_lines_range() {",
        "top..text.ropey_line_count()",
    ] {
        assert!(
            zero_based_line_count_range(code).is_none(),
            "false positive on: {code}"
        );
    }
}
