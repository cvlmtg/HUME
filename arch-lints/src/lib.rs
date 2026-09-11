//! Architectural lints, enforced as `cargo test` integration tests under
//! `tests/` — each scans a curated list of source files on disk for patterns
//! that violate a rule, rather than exercising any workspace crate's own
//! code. One file per lint; this module holds the shared string/source-
//! scanning helpers every lint builds on.
//!
//! A lint here never constructs an `Editor` or links against `hume-editor`
//! at all — it reads `.rs`/`.scm`/`.md` files as text and pattern-matches
//! them, so it doesn't belong to any one crate's test suite. It scans every
//! workspace member equally, `hume-editor` included.

/// The workspace root every lint resolves its scan paths against.
///
/// `arch-lints` sits one directory below the workspace root, the same depth
/// as every other member crate, so `CARGO_MANIFEST_DIR`'s parent is always
/// correct — one helper instead of each lint repeating the
/// `env::var("CARGO_MANIFEST_DIR")` + `.parent()` dance.
pub fn workspace_root() -> std::path::PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
    std::path::Path::new(&manifest)
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

// ── Shared helpers ───────────────────────────────────────────────────────────

/// Every workspace member crate name, derived from the root `Cargo.toml`'s
/// `[workspace] members = [...]` line — the single source of truth for
/// "what crates exist." A hand-maintained crate list can silently drop out
/// of sync with the workspace (a renamed directory, a newly added crate);
/// reading it back out of `Cargo.toml` can't. Shared by every lint that
/// scans the whole workspace rather than a curated file list.
pub(crate) fn workspace_member_crates(workspace_root: &std::path::Path) -> Vec<String> {
    let manifest = std::fs::read_to_string(workspace_root.join("Cargo.toml"))
        .expect("cannot read workspace Cargo.toml");
    let members_line = manifest
        .lines()
        .find(|l| l.trim_start().starts_with("members"))
        .expect("no `members = [...]` line in workspace Cargo.toml");
    quoted_strings(members_line)
}

/// Collect all `.rs` files under `dir`, recursively, excluding any
/// directory named `tests` and any file named `tests.rs`.  Results are
/// sorted for deterministic test output.
pub(crate) fn collect_source_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let n = name.to_string_lossy();
        if path.is_dir() && n != "tests" {
            collect_source_rs(&path, out);
        } else if path.is_file() && n.ends_with(".rs") && n != "tests.rs" {
            out.push(path);
        }
    }
}

/// Collect all `.rs` files under `dir`, recursively, with no exclusions —
/// [`collect_source_rs`]'s sibling for a lint whose whole job is scanning
/// what that one deliberately skips (a `tests/` tree). Results are sorted
/// for deterministic test output, same as `collect_source_rs`.
pub fn collect_all_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
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
            collect_all_rs(&path, out);
        } else if path.is_file() && n.ends_with(".rs") {
            out.push(path);
        }
    }
}

/// Every source file every whole-workspace lint in this module scans:
/// enumerate crates from the root `Cargo.toml`, assert each has a `src/` (a
/// silently-empty scan would let a renamed crate escape unnoticed), collect
/// via [`collect_source_rs`], then retain out this `lints/` directory's own
/// pattern literals and any path in `extra_excludes` (a lint excluding one
/// specific implementation file while still scanning the rest of that
/// file's crate — `absent_decode`'s sole caller excludes its own
/// `hume-scripting/src/builtins/args.rs`, the file that defines the pattern
/// it scans for) — the shared setup every whole-workspace lint needs.
pub fn workspace_source_paths(
    workspace_root: &std::path::Path,
    extra_excludes: &[std::path::PathBuf],
) -> Vec<std::path::PathBuf> {
    let crates: Vec<String> = workspace_member_crates(workspace_root);
    assert!(
        !crates.is_empty(),
        "workspace_member_crates found no members — Cargo.toml parsing broke"
    );
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for c in &crates {
        let src_dir = workspace_root.join(c).join("src");
        // Fail loudly on a crate whose `src/` moved: `collect_source_rs`
        // returns silently on an unreadable directory, so a renamed crate
        // would otherwise pass every lint by having nothing to check.
        assert!(
            src_dir.is_dir(),
            "workspace member {c} has no src/ at {} — this lint would silently scan nothing",
            src_dir.display()
        );
        collect_source_rs(&src_dir, &mut paths);
    }
    // This crate holds the pattern literals scanned for above — excluded so
    // a lint never flags itself. `workspace_member_crates` already put
    // `arch-lints/src` in `paths`; `arch-lints/tests` (the lints themselves)
    // never enters `paths` in the first place, since `workspace_source_paths`
    // only walks each member's `src/`.
    let arch_lints_src = workspace_root.join("arch-lints/src");
    paths.retain(|p| !p.starts_with(&arch_lints_src) && !extra_excludes.contains(p));
    paths
}

/// The portion of `line` before any line comment (`//`), skipping `//`
/// that appears inside a string literal — a naive `line.find("//")` would
/// truncate a call like `log_path("cache//tempfile::tempdir()")` at the
/// string's embedded `//`, hiding the rest of the line (and any forbidden
/// pattern in it, `tempfile::tempdir()` among them) from every lint that
/// strips comments this way. Escaped quotes (`\"`) inside a string keep it
/// open; char literals (`'x'`, `'\x'`) are skipped so their quote marks
/// don't falsely open/close string tracking; a bare `'` that isn't a char
/// literal (a lifetime) is left alone. Raw strings (`r"..."`) are not
/// handled — none of the scanned patterns appear inside one today.
pub fn strip_line_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
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
            b'/' if bytes.get(i + 1) == Some(&b'/') => return &line[..i],
            _ => i += 1,
        }
    }
    line
}

/// One violation found by [`scan_lines`]/[`scan_forbidden`].
pub struct Violation {
    /// `path` relative to the caller's `display_root` (or the absolute path,
    /// if `path` doesn't start with `display_root`).
    pub file: String,
    /// 1-based line number.
    pub lineno: usize,
    /// The offending line, trimmed of leading/trailing whitespace.
    pub trimmed: String,
}

/// Shared skeleton for every line-by-line lint in this module: walks
/// `paths` tracking `#[cfg(test)] mod tests { … }` extent (skipped
/// entirely) and a two-tier opt-out, then calls `find` on each surviving,
/// comment-stripped line, pushing one `Violation` per match it reports
/// finding. `scan_forbidden` (below) is the common case — a fixed
/// forbidden-substring list.
///
/// **Opt-out**: a comment containing `marker` (e.g. `"// test-global-safe:"`)
/// suppresses a hit on the violation line itself; on the line *above*, only
/// when the marker starts that line (after trimming) — `cargo fmt` hoists a
/// trailing comment onto its own line, so the marker often ends up above
/// the forbidden pattern rather than beside it, but a marker merely
/// *appearing* somewhere on an unrelated previous line must not silently
/// exempt code it was never meant to.
pub fn scan_lines(
    paths: &[std::path::PathBuf],
    display_root: &std::path::Path,
    marker: &str,
    mut find: impl FnMut(&str) -> usize,
) -> Vec<Violation> {
    let mut violations = Vec::new();

    for path in paths {
        let file = path
            .strip_prefix(display_root)
            .unwrap_or(path)
            .display()
            .to_string();
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

        // Track whether we are inside a `#[cfg(test)] mod tests { … }` block
        // so violations there don't get flagged.
        let mut in_test_block = false;
        let mut brace_depth: i64 = 0;
        let mut test_entry_depth: i64 = 0;
        let mut saw_cfg_test = false;
        // The previous source line (blank or not), kept so an opt-out
        // marker alone on the line *above* a violation suppresses it.
        let mut prev_line: &str = "";

        for (lineno, line) in src.lines().enumerate() {
            let trimmed = line.trim();
            let prev_for_exempt = prev_line;
            prev_line = line;

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
            // Same-line opt-out.
            if line.contains(marker) {
                continue;
            }
            // Preceding-line opt-out — a *trailing* marker up there exempts
            // only its own line, not this one; only a marker occupying that
            // whole line reaches down to the line below it.
            if prev_for_exempt.trim_start().starts_with(marker) {
                continue;
            }

            let code = strip_line_comment(line);
            for _ in 0..find(code) {
                violations.push(Violation {
                    file: file.clone(),
                    lineno: lineno + 1,
                    trimmed: trimmed.to_string(),
                });
            }
        }
    }

    violations
}

/// Scan `paths` for any of `forbidden` patterns in active (non-test,
/// non-comment) code — [`scan_lines`] specialized to a fixed
/// forbidden-substring list, the shape every lint in this module needs.
pub fn scan_forbidden(
    paths: &[std::path::PathBuf],
    display_root: &std::path::Path,
    forbidden: &[&'static str],
    marker: &str,
) -> Vec<Violation> {
    scan_lines(paths, display_root, marker, |code| {
        forbidden
            .iter()
            .filter(|&&pattern| code.contains(pattern))
            .count()
    })
}

/// Extracts every double-quoted string literal's contents from `s`,
/// verbatim (no escape processing — plugin command names never contain a
/// `"`, so a naive quote-delimited split is exact here).
pub fn quoted_strings(s: &str) -> Vec<String> {
    s.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

#[test]
fn strip_line_comment_cases() {
    // Fail oracle: revert strip_line_comment to a naive `line.find("//")`
    // and the string-literal cases below (2nd and 4th) must start failing.
    assert_eq!(strip_line_comment("foo(); // bar"), "foo(); ");
    assert_eq!(strip_line_comment("foo();"), "foo();");
    assert_eq!(strip_line_comment(r#"call("a//b")"#), r#"call("a//b")"#);
    assert_eq!(
        strip_line_comment(r#"call("a//b") // note"#),
        r#"call("a//b") "#
    );
    assert_eq!(
        strip_line_comment(r#"let q = '"'; // c"#),
        r#"let q = '"'; "#
    );
    assert_eq!(
        strip_line_comment(r#""a\"//b""#),
        r#""a\"//b""#,
        "escaped quote must not end the string early"
    );
}
