//! # Unguarded unqualified subprocess spawns
//!
//! `Global::Env`'s doc (`editor/tests/mod.rs`) states the reader obligation
//! this lint enforces: a test that spawns a subprocess by unqualified name
//! (`Command::new("git")`, `Command::new("sh")`, …) reads process `PATH` at
//! the spawn instant exactly as much as an explicit `std::env::var` call
//! would, so it must hold a `Global::Env` claim for the spawn's duration —
//! not just a test that mutates an env var directly.
//!
//! [`unguarded_unqualified_spawn`] scans every `#[test] fn` body in
//! `editor/tests/` (the opposite direction from this file's sibling lints in
//! [`super::test_globals`], which scan every *non*-test file for a raw
//! mutator) for a spawn appearing before a claim in the same function body.
//!
//! **Neither side is always literal text in the test's own body.**
//! `git_diff_plugin.rs`'s tests call `git_init()`/`commit_file()`, which call
//! a shared `git()` helper (`unix/mod.rs`) that does the unqualified
//! `Command::new("git")` itself — two hops from the test, with no
//! `Command::new` text anywhere in the test's own body. Symmetrically, those
//! same tests call `setup()`, which claims `Global::Env` via
//! `RealRuntimeGuard::new()` and returns the guard for the caller to bind and
//! hold — again with no `RealRuntimeGuard`/`TEST_GLOBALS` text in the test
//! body itself. A scan that only recognized literal `Command::new`/claim text
//! would either flag every one of these tests as unguarded (wrong — they are
//! guarded, just through a helper) or, worse, silently pass a genuinely
//! unguarded one that happened to call some other helper by coincidence.
//!
//! [`collect_helper_fns`] gathers every plain (non-`#[test]`) *free*
//! function's body anywhere in the tree — deliberately skipping `impl`
//! blocks entirely, not just their `#[test]` methods. Every actual
//! spawn/claim helper this lint cares about (`git`, `git_init`,
//! `commit_file`, `setup`, …) is a bare free function; an `impl` method
//! keeps only its bare name once extracted, and `fn new(...)` is defined
//! identically-named inside dozens of unrelated structs across this tree —
//! recording `RealRuntimeGuard::new`'s claiming body under the bare name
//! "new" would make [`calls_fn`] treat a call to *any* type's `::new()` as
//! calling the one that happens to claim, which is exactly the false-positive
//! flood this lint's first working draft produced.
//!
//! [`spawning_helper_names`] and [`claiming_helper_names`] each seed a set —
//! bodies directly containing an unqualified `Command::new("literal")`, and
//! bodies directly containing an [`AUTO_CLAIM_MARKERS`] entry, respectively —
//! then [`propagate_transitively`] grows each to a fixed point: a helper that
//! *calls* one already in the set joins it too, however many hops away
//! (`commit_file` → `git` → `Command::new` is two). [`scan`] then treats a
//! call to any name in the spawning set the same as a direct `Command::new`,
//! and a call to any name in the claiming set the same as a direct claim.
//! Brace-depth tracking throughout (function boundaries, `#[test] fn`
//! extent) goes through [`brace_delta`], not a raw `{`/`}` character count —
//! this tree's fixture strings routinely embed an unbalanced-looking brace
//! (`format!("...#:config {cfg})")`), and counting those as real nesting
//! drifted every function boundary after the first such string, corrupting
//! the propagation far more broadly than the `impl`-block issue above.
//!
//! **What this still can't see**: a spawn buried inside *production* code
//! reached via `spawn-async!`/`EditorHostImpl::spawn_async` (`async_job.rs`'s
//! `"sh"`, for instance) has no `Command::new` text anywhere in the test
//! tree at all — covered by the `AUTO_CLAIM_MARKERS` convention today,
//! verified by hand rather than by this lint. And the propagation trusts
//! that a claiming helper's caller actually binds and holds the guard it
//! returns (every current one does, `let (_, guard) = helper()` — a helper
//! that claimed and immediately dropped the guard before returning would
//! read as "claiming" here without truly protecting its caller).
//!
//! **Opt-out**: `// test-global-safe: <reason>` on the violation line (or the
//! line above, so `cargo fmt` doesn't hoist a trailing comment past it) —
//! same convention and marker as [`super::test_globals`]'s two lints.

use super::strip_line_comment;

/// A construct that, once seen in a `#[test] fn` body, is trusted to already
/// hold (or have just claimed) `Global::Env` for the rest of that body — see
/// this module's doc for why the list is a fixed set of textual patterns
/// rather than a call-graph walk. Order doesn't matter; every entry is
/// checked independently.
const AUTO_CLAIM_MARKERS: &[&str] = &[
    "TEST_GLOBALS.claim(Global::Env)",
    "RealRuntimeGuard::new(",
    "HumeRuntimeGuard::new(",
    "RealThemeRuntimeGuard::new(",
    "NoConfigDirGuard::new(",
];

const OPT_OUT_MARKER: &str = "// test-global-safe:";

/// Collect every `.rs` file under `dir`, recursively — this lint's whole job
/// is scanning the test tree itself, so (unlike `collect_source_rs`, this
/// module's sibling lints' helper) there is no `tests`-directory exclusion.
/// Mirrors `test_globals.rs`'s own `collect_all_rs`, duplicated rather than
/// shared since that one is `fn`-private to its module.
fn collect_all_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
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

/// Net change in brace depth `line` contributes — `{`/`}` characters inside
/// a string or char literal don't count. Without this, a line like
/// `format!("...#:config {cfg})")` (a single, balanced `{`/`}` pair, but
/// *inside* the string) still nets to zero on this particular line, but a
/// string holding a lone `{` or `}` with no same-line match — common in this
/// tree's Scheme-source and JSON-shaped fixture strings — would silently
/// shift every function-boundary guess after it for the rest of the file,
/// which is what happened here before this existed: a helper's captured body
/// swallowed dozens of unrelated later functions, and the claim/spawn
/// propagation below exploded outward from that corruption.
fn brace_delta(line: &str) -> i64 {
    let mut delta = 0i64;
    let mut in_string = false;
    let mut escaped = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '\'' if chars.peek() == Some(&'\\') => {
                // Escaped char literal '\x' — consume the rest of it so its
                // backslash can't be mistaken for a string escape below.
                chars.next();
                chars.next();
                chars.next();
            }
            '\'' => {
                // Could be a char literal ('x') or a lifetime ('a); either
                // way there's no brace to miscount between here and the
                // next quote a char literal would close with, and a
                // lifetime has no closing quote at all — skip only the one
                // character so a lifetime's identifier is still scanned.
            }
            '{' => delta += 1,
            '}' => delta -= 1,
            _ => {}
        }
    }
    delta
}

/// If `line` spawns a subprocess by an unqualified program name — a
/// `Command::new("literal")` whose `literal` contains no `/` or `\` — return
/// that literal. A qualified path (`Command::new("/usr/bin/git")`) is not a
/// `PATH` read, so it's outside this lint's concern.
fn unqualified_command_spawn(line: &str) -> Option<&str> {
    let after = line.split("Command::new(").nth(1)?;
    let literal = after.trim_start().strip_prefix('"')?;
    let end = literal.find('"')?;
    let literal = &literal[..end];
    (!literal.contains('/') && !literal.contains('\\')).then_some(literal)
}

/// The identifier right after a `fn ` at the start of `trimmed`, if any —
/// `"fn git_init(dir: &Path) {"` -> `Some("git_init")`. No visibility
/// modifier handling: every helper this lint cares about (and every
/// `#[test] fn`) in this tree is a bare, module-private `fn`.
fn fn_name(trimmed: &str) -> Option<&str> {
    let after = trimmed.strip_prefix("fn ")?;
    let end = after.find('(')?;
    Some(after[..end].trim())
}

/// True when `code` calls `name` as a function — `name` immediately followed
/// by `(`, with no identifier character before it (so a call to `git_init(`
/// is never mistaken for one to `git(`, and a local variable merely named
/// after the helper, e.g. `let git = ...`, isn't itself a call).
fn calls_fn(code: &str, name: &str) -> bool {
    let pat_len = name.len();
    code.match_indices(name).any(|(i, _)| {
        let before_ok = i == 0
            || !code.as_bytes()[i - 1].is_ascii_alphanumeric() && code.as_bytes()[i - 1] != b'_';
        let after_ok = code.as_bytes().get(i + pat_len) == Some(&b'(');
        before_ok && after_ok
    })
}

/// Every top-level, non-`#[test]`, non-`impl`-method free function anywhere
/// in `paths`, as `(name, body)` — `body` is every line from the `fn` line
/// to its closing brace, joined back with `\n`, so a caller can ask "does
/// this helper's body mention X" the same way the per-line scans elsewhere
/// in this file do. `impl` blocks are skipped entirely — see this module's
/// doc for why a bare method name (`new`) is unsafe to key on here.
fn collect_helper_fns(paths: &[std::path::PathBuf]) -> Vec<(String, String)> {
    let mut fns = Vec::new();

    for path in paths {
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

        let mut saw_test_attr = false;
        let mut cur_fn: Option<(String, i64, Vec<&str>)> = None; // (name, entry_depth, body lines)
        let mut in_impl: Option<i64> = None; // entry_depth of the impl block we're inside, if any
        let mut brace_depth: i64 = 0;

        for line in src.lines() {
            let trimmed = line.trim();

            if trimmed == "#[test]" {
                saw_test_attr = true;
            } else if in_impl.is_none() && cur_fn.is_none() && trimmed.starts_with("impl ") {
                in_impl = Some(brace_depth);
                saw_test_attr = false;
            } else if let Some(name) = fn_name(trimmed) {
                if !saw_test_attr && cur_fn.is_none() && in_impl.is_none() {
                    cur_fn = Some((name.to_string(), brace_depth, Vec::new()));
                }
                saw_test_attr = false;
            } else if !trimmed.starts_with('#') && !trimmed.is_empty() {
                saw_test_attr = false;
            }

            if let Some((_, _, body)) = &mut cur_fn {
                body.push(line);
            }

            brace_depth += brace_delta(line);

            if let Some(entry_depth) = in_impl
                && brace_depth <= entry_depth
            {
                in_impl = None;
            }

            if let Some((name, entry_depth, body)) = cur_fn.take() {
                if brace_depth <= entry_depth {
                    fns.push((name, body.join("\n")));
                } else {
                    cur_fn = Some((name, entry_depth, body));
                }
            }
        }
    }

    fns
}

/// Grow `seeded` (a name → its own body already known to have some property)
/// to a fixed point: any other helper whose body *calls* a name already in
/// the set gains the property too, however many hops away — `commit_file`
/// calling `git`, which calls `Command::new`, is two. Shared by
/// [`spawning_helper_names`] and [`claiming_helper_names`], which differ
/// only in which predicate seeds the initial set.
fn propagate_transitively(
    helper_fns: &[(String, String)],
    mut has_property: std::collections::HashSet<String>,
) -> Vec<String> {
    loop {
        let mut added = false;
        for (name, body) in helper_fns {
            if has_property.contains(name) {
                continue;
            }
            if has_property.iter().any(|s| calls_fn(body, s)) {
                has_property.insert(name.clone());
                added = true;
            }
        }
        if !added {
            break;
        }
    }

    has_property.into_iter().collect()
}

/// Every helper name (from [`collect_helper_fns`]) that spawns an unqualified
/// subprocess — directly (its own body contains `Command::new("literal")`)
/// or transitively (its body calls another name already known to spawn one).
fn spawning_helper_names(helper_fns: &[(String, String)]) -> Vec<String> {
    let seed = helper_fns
        .iter()
        .filter(|(_, body)| {
            body.lines()
                .any(|l| unqualified_command_spawn(strip_line_comment(l)).is_some())
        })
        .map(|(name, _)| name.clone())
        .collect();
    propagate_transitively(helper_fns, seed)
}

/// Every helper name (from [`collect_helper_fns`]) that claims (or returns an
/// already-claimed guard for) `Global::Env` — directly (its own body
/// contains an [`AUTO_CLAIM_MARKERS`] entry, the shape `setup()`-style test
/// fixtures across this tree follow: claim, do setup, return `(Editor,
/// SomeGuard)` for the caller to bind and keep alive) or transitively (calls
/// another name already known to claim). The same transitive-indirection gap
/// [`spawning_helper_names`] closes on the spawn side, mirrored here: a test
/// calling `setup(...)` never mentions `RealRuntimeGuard::new` itself.
fn claiming_helper_names(helper_fns: &[(String, String)]) -> Vec<String> {
    let seed = helper_fns
        .iter()
        .filter(|(_, body)| AUTO_CLAIM_MARKERS.iter().any(|m| body.contains(m)))
        .map(|(name, _)| name.clone())
        .collect();
    propagate_transitively(helper_fns, seed)
}

/// One violation: a spawn (direct, or through a [`spawning_helper_names`]
/// helper) found before a claim (direct, or through a
/// [`claiming_helper_names`] helper) in its enclosing `#[test] fn`.
struct Violation {
    file: String,
    lineno: usize,
    what: String,
}

/// Final pass: scan every `#[test] fn` body (via brace-depth entry/exit, the
/// same technique [`super::scan_forbidden`] uses for a `mod tests { … }`
/// block) for a call to any name in `spawning_helpers`, or a direct
/// `Command::new`, appearing before an [`AUTO_CLAIM_MARKERS`] entry or a call
/// to any name in `claiming_helpers`, in the same body.
fn scan(
    paths: &[std::path::PathBuf],
    display_root: &std::path::Path,
    spawning_helpers: &[String],
    claiming_helpers: &[String],
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

        let mut in_test_fn = false;
        let mut fn_entry_depth: i64 = 0;
        let mut brace_depth: i64 = 0;
        let mut saw_test_attr = false;
        let mut claimed = false;
        let mut prev_line: &str = "";

        for (lineno, line) in src.lines().enumerate() {
            let trimmed = line.trim();
            let prev_for_exempt = prev_line;
            prev_line = line;

            if trimmed == "#[test]" {
                saw_test_attr = true;
            } else if saw_test_attr && trimmed.starts_with("fn ") {
                in_test_fn = true;
                fn_entry_depth = brace_depth;
                claimed = false;
                saw_test_attr = false;
            } else if !trimmed.starts_with('#') && !trimmed.is_empty() {
                // Any other real code line between `#[test]` and its `fn`
                // (there isn't one in this codebase's style, but a stray
                // attribute in between must not silently carry the flag
                // forward onto some later, unrelated `fn`) resets the flag.
                saw_test_attr = false;
            }

            brace_depth += brace_delta(line);
            if in_test_fn && brace_depth <= fn_entry_depth {
                in_test_fn = false;
            }

            if !in_test_fn {
                continue;
            }
            if trimmed.starts_with("//") {
                continue;
            }

            let code = strip_line_comment(line);
            if AUTO_CLAIM_MARKERS.iter().any(|m| code.contains(m))
                || claiming_helpers.iter().any(|h| calls_fn(code, h))
            {
                claimed = true;
            }
            if claimed {
                continue;
            }
            if line.contains(OPT_OUT_MARKER) || prev_for_exempt.contains(OPT_OUT_MARKER) {
                continue;
            }
            if let Some(program) = unqualified_command_spawn(code) {
                violations.push(Violation {
                    file: file.clone(),
                    lineno: lineno + 1,
                    what: format!("Command::new({program:?})"),
                });
                continue;
            }
            if let Some(helper) = spawning_helpers.iter().find(|h| calls_fn(code, h)) {
                violations.push(Violation {
                    file: file.clone(),
                    lineno: lineno + 1,
                    what: format!("{helper}(...) — spawns unqualified internally"),
                });
            }
        }
    }

    violations
}

/// Fail oracle: add `std::process::Command::new("some-tool").spawn()` to any
/// `#[test] fn` body in `editor/tests/` with no preceding auto-claim marker —
/// this test must fail naming that line. (Sabotage-verified against the
/// pre-fix shape of `git_diff_plugin.rs`, which called `git_init`/
/// `commit_file` — themselves calling `unix/mod.rs`'s unqualified
/// `Command::new("git")` — before `setup()`'s `RealRuntimeGuard::new()`.)
#[test]
fn unguarded_unqualified_spawn() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
    let root = std::path::Path::new(&manifest);

    let mut paths = Vec::new();
    collect_all_rs(&root.join("src/editor/tests"), &mut paths);

    let helper_fns = collect_helper_fns(&paths);
    let spawning_helpers = spawning_helper_names(&helper_fns);
    let claiming_helpers = claiming_helper_names(&helper_fns);
    let violations: Vec<String> = scan(&paths, root, &spawning_helpers, &claiming_helpers)
        .into_iter()
        .map(|v| format!("  {}:{} — {}", v.file, v.lineno, v.what))
        .collect();

    assert!(
        violations.is_empty(),
        "\nUnqualified subprocess spawn found with no `Global::Env` claim held yet in its\n\
         `#[test] fn` — either directly, or through a helper that itself spawns one\n\
         unqualified. The OS resolves an unqualified program name against process `PATH`\n\
         at the spawn instant — the same read a `std::env::var(\"PATH\")` call would need\n\
         to hold a claim for. Claim it directly (`TEST_GLOBALS.claim(Global::Env)`) or via\n\
         a guard whose constructor already does (`RealRuntimeGuard::new()`, …) before the\n\
         spawn, or add the guard to this lint's `AUTO_CLAIM_MARKERS` if it's a new one.\n\
         Violations:\n{}\n",
        violations.join("\n")
    );
}
