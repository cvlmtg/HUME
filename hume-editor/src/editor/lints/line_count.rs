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
//! crate's `src/` — derived from the root `Cargo.toml`'s `members` list, so
//! a renamed or newly added crate can't silently fall out of scope —
//! excluding `hume-rope/src` (the implementation itself) and this `lints/`
//! directory (where the pattern literals below live), for the forbidden
//! patterns.
//!
//! **Opt-out**: annotate a line with `// line-count-safe: <reason>`.

use super::{collect_source_rs, quoted_strings, scan_forbidden};

/// Every workspace member crate name, derived from the root `Cargo.toml`'s
/// `[workspace] members = [...]` line — the single source of truth for
/// "what crates exist." A hand-maintained crate list can silently drop out
/// of sync with the workspace (a renamed directory, a newly added crate);
/// reading it back out of `Cargo.toml` can't.
fn workspace_member_crates(workspace_root: &std::path::Path) -> Vec<String> {
    let manifest = std::fs::read_to_string(workspace_root.join("Cargo.toml"))
        .expect("cannot read workspace Cargo.toml");
    let members_line = manifest
        .lines()
        .find(|l| l.trim_start().starts_with("members"))
        .expect("no `members = [...]` line in workspace Cargo.toml");
    quoted_strings(members_line)
}

/// Recursively collect every `.rs` file under `dir` that [`collect_source_rs`]
/// excludes: anything under a directory named `tests`, or a file named
/// `tests.rs`. Used to widen this lint's `len_lines(` check into test code
/// (see below) — that excluded set is exactly where the wrong
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

/// Recursively collect every `.rs` file under `dir`, no exclusions.
fn collect_all_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_all_rs(&path, out);
        } else if path.is_file() && path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
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
    let crates: Vec<String> = workspace_member_crates(workspace_root)
        .into_iter()
        .filter(|c| c != "hume-rope")
        .collect();
    assert!(
        !crates.is_empty(),
        "workspace_member_crates found no members — Cargo.toml parsing broke"
    );
    // Fail loudly on a missing src/ dir rather than let collect_source_rs's
    // silent-on-read_dir-failure behavior scan zero files for it — a crate
    // with no src/ would otherwise pass this lint by having nothing to check.
    for c in &crates {
        let src_dir = workspace_root.join(c).join("src");
        assert!(
            src_dir.is_dir(),
            "workspace member {c:?} has no src/ dir at {src_dir:?}"
        );
    }
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for c in &crates {
        collect_source_rs(&workspace_root.join(c).join("src"), &mut paths);
    }
    // This lints/ directory holds the pattern literals scanned for below —
    // excluded so this file never flags itself.
    let lints_dir = workspace_root.join("hume-editor/src/editor/lints");
    paths.retain(|p| !p.starts_with(&lints_dir));

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
