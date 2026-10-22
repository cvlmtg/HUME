//! Architectural lints, enforced as `cargo test` integration tests under
//! `tests/`. Each scans a curated list of source files on disk for patterns
//! that violate a rule, rather than exercising any workspace crate's own
//! code. One file per lint; this module holds the shared string/source-
//! scanning helpers every lint builds on.
//!
//! A lint here never constructs an `Editor` or links against `hume-editor`
//! at all: it reads `.rs`/`.scm`/`.md` files as text and pattern-matches
//! them, so it doesn't belong to any one crate's test suite. It scans every
//! workspace member equally, `hume-editor` included.

#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]

/// The workspace root every lint resolves its scan paths against.
///
/// `arch-lints` sits one directory below the workspace root, the same depth
/// as every other member crate, so `CARGO_MANIFEST_DIR`'s parent is always
/// correct. One helper instead of each lint repeating the
/// `env::var("CARGO_MANIFEST_DIR")` + `.parent()` dance.
pub fn workspace_root() -> std::path::PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set: run via `cargo test`");
    std::path::Path::new(&manifest)
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

// ── Shared helpers ───────────────────────────────────────────────────────────

/// Every workspace member crate name, derived from the root `Cargo.toml`'s
/// `[workspace] members = [...]` line, the single source of truth for
/// "what crates exist." A hand-maintained crate list can silently drop out
/// of sync with the workspace (a renamed directory, a newly added crate);
/// reading it back out of `Cargo.toml` can't.
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

/// Collect all `.rs` files under `dir`, recursively, with no exclusions:
/// `collect_source_rs`'s sibling for a lint whose whole job is scanning
/// what that one skips (a `tests/` tree). Results are sorted
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
/// via `collect_source_rs`, then retain out this `lints/` directory's own
/// pattern literals and any path in `extra_excludes` (a lint excluding one
/// specific implementation file while still scanning the rest of that
/// file's crate). This is the shared setup every whole-workspace lint needs.
pub fn workspace_source_paths(
    workspace_root: &std::path::Path,
    extra_excludes: &[std::path::PathBuf],
) -> Vec<std::path::PathBuf> {
    let crates: Vec<String> = workspace_member_crates(workspace_root);
    assert!(
        !crates.is_empty(),
        "workspace_member_crates found no members: Cargo.toml parsing broke"
    );
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for c in &crates {
        let src_dir = workspace_root.join(c).join("src");
        // Fail loudly on a crate whose `src/` moved: `collect_source_rs`
        // returns silently on an unreadable directory, so a renamed crate
        // would otherwise pass every lint by having nothing to check.
        assert!(
            src_dir.is_dir(),
            "workspace member {c} has no src/ at {}: this lint would silently scan nothing",
            src_dir.display()
        );
        collect_source_rs(&src_dir, &mut paths);
    }
    // This crate holds the pattern literals scanned for above, excluded so
    // a lint never flags itself. `workspace_member_crates` already put
    // `arch-lints/src` in `paths`; `arch-lints/tests` (the lints themselves)
    // never enters `paths` in the first place, since `workspace_source_paths`
    // only walks each member's `src/`.
    let arch_lints_src = workspace_root.join("arch-lints/src");
    paths.retain(|p| !p.starts_with(&arch_lints_src) && !extra_excludes.contains(p));
    paths
}

/// Every `.rs` file in every workspace member, `src/` and `tests/` alike.
/// For lints about comments rather than code, where test files matter as
/// much as production ones.
pub fn workspace_all_rs_paths(workspace_root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    for c in workspace_member_crates(workspace_root) {
        let dir = workspace_root.join(&c);
        assert!(
            dir.is_dir(),
            "workspace member {c} not found at {}",
            dir.display()
        );
        collect_all_rs(&dir, &mut paths);
    }
    paths
}

/// The portion of `line` before any line comment (`//`), skipping `//`
/// that appears inside a string literal. A naive `line.find("//")` would
/// truncate a call like `log_path("cache//tempfile::tempdir()")` at the
/// string's embedded `//`, hiding the rest of the line (and any forbidden
/// pattern in it, `tempfile::tempdir()` among them) from every lint that
/// strips comments this way. Escaped quotes (`\"`) inside a string keep it
/// open; char literals (`'x'`, `'\x'`) are skipped so their quote marks
/// don't falsely open/close string tracking; a bare `'` that isn't a char
/// literal (a lifetime) is left alone. Raw strings (`r"..."`) are not
/// handled: none of the scanned patterns appear inside one today.
fn strip_line_comment(line: &str) -> &str {
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

/// One violation found by [`scan_lines`].
pub struct Violation {
    /// `path` relative to the caller's `display_root` (or the absolute path,
    /// if `path` doesn't start with `display_root`).
    pub file: String,
    /// 1-based line number.
    pub lineno: usize,
    /// The offending line, trimmed of leading/trailing whitespace.
    pub detail: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "  {}:{}: {}", self.file, self.lineno, self.detail)
    }
}

/// The lines of `src` a lint checks, as `(0-based line index, line)`:
/// everything outside `#[cfg(test)] mod tests { … }` blocks, except whole-
/// line comments and lines a `marker` opt-out exempts.
///
/// **Opt-out**: a comment containing `marker` (e.g. `"// test-global-safe:"`)
/// exempts the line it is on; on the line *above*, only when the marker
/// starts that line (after trimming). `cargo fmt` hoists a trailing comment
/// onto its own line, so the marker often ends up above the forbidden
/// pattern rather than beside it, but a marker merely *appearing* somewhere
/// on an unrelated previous line must not silently exempt code it was never
/// meant to.
fn eligible_lines<'a>(src: &'a str, marker: &str) -> Vec<(usize, &'a str)> {
    let mut eligible = Vec::new();
    let mut in_test_block = false;
    let mut brace_depth: i64 = 0;
    let mut test_entry_depth: i64 = 0;
    let mut saw_cfg_test = false;
    // The previous source line (blank or not), kept so an opt-out marker
    // alone on the line *above* a violation exempts it.
    let mut prev_line: &str = "";

    for (index, line) in src.lines().enumerate() {
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

        if in_test_block || trimmed.starts_with("//") || line.contains(marker) {
            continue;
        }
        // A *trailing* marker on the line above exempts only its own line;
        // only a marker occupying that whole line reaches down to this one.
        if prev_for_exempt.trim_start().starts_with(marker) {
            continue;
        }
        eligible.push((index, line));
    }
    eligible
}

fn violation(file: &str, index: usize, line: &str) -> Violation {
    Violation {
        file: file.to_string(),
        lineno: index + 1,
        detail: line.trim().to_string(),
    }
}

/// `path` as a lint reports it: relative to `display_root` when inside it.
fn display_path(path: &std::path::Path, display_root: &std::path::Path) -> String {
    path.strip_prefix(display_root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn read_source(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Shared skeleton for every line-by-line lint in this module: calls `find`
/// on each comment-stripped line `eligible_lines` keeps, pushing one
/// `Violation` per match it reports finding.
pub fn scan_lines(
    paths: &[std::path::PathBuf],
    display_root: &std::path::Path,
    marker: &str,
    mut find: impl FnMut(&str) -> usize,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    for path in paths {
        let file = display_path(path, display_root);
        let src = read_source(path);
        for (index, line) in eligible_lines(&src, marker) {
            for _ in 0..find(strip_line_comment(line)) {
                violations.push(violation(&file, index, line));
            }
        }
    }
    violations
}

/// Every line where `token` appears inside the declaration of a `struct`,
/// `enum`, `union` or `type` alias in `paths`, among the lines
/// `eligible_lines` keeps: [`type_body_hits`] applied per file.
pub fn scan_type_bodies(
    paths: &[std::path::PathBuf],
    display_root: &std::path::Path,
    marker: &str,
    token: &str,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    for path in paths {
        let file = display_path(path, display_root);
        let src = read_source(path);
        let hits = type_body_hits(&src, token);
        for (index, line) in eligible_lines(&src, marker) {
            if hits.contains(&index) {
                violations.push(violation(&file, index, line));
            }
        }
    }
    violations
}

/// The 0-based indexes of the lines of `src` where `token`, as a whole
/// word, appears inside a `struct`, `enum`, `union` or `type` declaration:
/// a field, a variant payload or an alias. A declaration starts at a line
/// whose code begins with one of those keywords (after any visibility) and
/// ends where its braces close, or at its `;` outside parentheses when it
/// has no braced body. Braces and parentheses are counted on code with
/// comments stripped and string and char literal contents blanked.
pub fn type_body_hits(src: &str, token: &str) -> Vec<usize> {
    let mut hits = Vec::new();
    let mut depth: i64 = 0;
    // The brace depth a declaration started at, whether its braced body
    // has opened, and its parenthesis depth.
    let mut decl: Option<(i64, bool, i64)> = None;

    for (index, line) in src.lines().enumerate() {
        let code = blank_literals(strip_line_comment(line));
        if decl.is_none() && starts_type_declaration(&code) {
            decl = Some((depth, false, 0));
        }
        let in_decl = decl.is_some();
        for c in code.chars() {
            match c {
                '{' => {
                    depth += 1;
                    if let Some((start, opened, _)) = &mut decl
                        && depth == *start + 1
                    {
                        *opened = true;
                    }
                }
                '}' => {
                    depth -= 1;
                    if let Some((start, true, _)) = decl
                        && depth == start
                    {
                        decl = None;
                    }
                }
                '(' => {
                    if let Some((_, _, parens)) = &mut decl {
                        *parens += 1;
                    }
                }
                ')' => {
                    if let Some((_, _, parens)) = &mut decl {
                        *parens -= 1;
                    }
                }
                ';' => {
                    if let Some((start, false, 0)) = decl
                        && depth == start
                    {
                        decl = None;
                    }
                }
                _ => {}
            }
        }
        if in_decl && contains_word(&code, token) {
            hits.push(index);
        }
    }
    hits
}

/// Whether `code` begins a `struct`, `enum`, `union` or `type`
/// declaration, after any `pub`/`pub(…)` visibility.
fn starts_type_declaration(code: &str) -> bool {
    let mut rest = code.trim_start();
    if let Some(after) = rest.strip_prefix("pub") {
        rest = match after.strip_prefix('(') {
            Some(scoped) => match scoped.find(')') {
                Some(close) => &scoped[close + 1..],
                None => return false,
            },
            None => after,
        };
        if !rest.starts_with(char::is_whitespace) {
            return false;
        }
        rest = rest.trim_start();
    }
    ["struct", "enum", "union", "type"].iter().any(|keyword| {
        rest.strip_prefix(keyword)
            .is_some_and(|after| after.starts_with(char::is_whitespace))
    })
}

/// Whether `token` occurs in `code` with no identifier character on
/// either side.
fn contains_word(code: &str, token: &str) -> bool {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    code.match_indices(token).any(|(at, _)| {
        !code[..at].chars().next_back().is_some_and(is_ident)
            && !code[at + token.len()..]
                .chars()
                .next()
                .is_some_and(is_ident)
    })
}

/// `code` with the contents of every string and char literal replaced by
/// spaces, so a `{` or `;` inside one is not counted. Same literal rules
/// as [`strip_line_comment`].
fn blank_literals(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.push('"');
                let mut escaped = false;
                for inner in chars.by_ref() {
                    if escaped {
                        escaped = false;
                    } else if inner == '\\' {
                        escaped = true;
                    } else if inner == '"' {
                        out.push('"');
                        break;
                    }
                    out.push(' ');
                }
            }
            '\'' => {
                let rest: String = chars.clone().take(3).collect();
                let literal_len = if rest.starts_with('\\') && rest.chars().nth(2) == Some('\'') {
                    3
                } else if rest.chars().nth(1) == Some('\'') {
                    2
                } else {
                    0
                };
                out.push('\'');
                for _ in 0..literal_len {
                    chars.next();
                    out.push(' ');
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Extracts every double-quoted string literal's contents from `s`,
/// verbatim (no escape processing: plugin command names never contain a
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
    // The 2nd and 4th cases put `//` inside a string literal.
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
