//! # Plugin manifest command-list drift
//!
//! A plugin's `manifest.scm` (`#:commands '(...)`) is the zero-argument
//! `(declare-plugin "core:foo")` activation list — hand-maintained, and
//! duplicated nowhere else the compiler checks. A command added to a
//! feature file without a matching manifest entry silently never triggers
//! lazy activation (the plugin loads, but `:that-command` looks unbound
//! until something else activates it); a stale manifest entry for a
//! deleted command is dead weight.
//!
//! `plugin_manifest_commands_match_defined_commands` scans every
//! `runtime/plugins/core/*/manifest.scm` present, and asserts its
//! `#:commands` list is the exact same set (both directions) as every
//! `(define-command! "name" ...)` found across that plugin's own `*.scm`
//! files. A plugin directory with no `manifest.scm` (no zero-arg
//! `declare-plugin` activation defined) is skipped, not a violation.

use super::quoted_strings;

/// The string literals inside `manifest.scm`'s `#:commands '(...)` list —
/// scoped to that one clause so `#:languages`/the plugin name's own
/// quoted strings elsewhere in the file are never mistaken for commands.
/// Empty (not a violation by itself) if the manifest declares no
/// `#:commands` at all. Comment lines are stripped first — every
/// `manifest.scm` opens with a `;`-comment header that mentions
/// `#:commands` in prose, which would otherwise be the *first* (wrong)
/// match.
fn manifest_declared_commands(src: &str) -> Vec<String> {
    let code_only: String = src
        .lines()
        .filter(|line| !line.trim_start().starts_with(';'))
        .collect::<Vec<_>>()
        .join("\n");
    let src = &code_only;
    let Some(after) = src.find("#:commands") else {
        return Vec::new();
    };
    let after = &src[after..];
    let Some(open) = after.find('(') else {
        return Vec::new();
    };
    let mut depth = 0i32;
    let mut end = None;
    for (i, c) in after[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(open + i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(end) = end else {
        return Vec::new();
    };
    quoted_strings(&after[open..end])
}

/// Every name in a `(define-command! "name" ...)` call, across every
/// `*.scm` file directly inside `dir` (plugins don't nest subdirectories).
fn defined_commands(dir: &std::path::Path) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return names;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("scm") {
            continue;
        }
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        // Comment lines stripped first, same reason as
        // `manifest_declared_commands` — a doc comment mentioning
        // `define-command!` in prose must never be mistaken for a call.
        let code_only: String = src
            .lines()
            .filter(|line| !line.trim_start().starts_with(';'))
            .collect::<Vec<_>>()
            .join("\n");
        for (idx, _) in code_only.match_indices("define-command!") {
            let after = &code_only[idx + "define-command!".len()..];
            if let Some(name) = quoted_strings(after).into_iter().next() {
                names.push(name);
            }
        }
    }
    names
}

/// Fail oracle: comment out one entry in `core:lsp/manifest.scm`'s
/// `#:commands` list (e.g. delete `"lsp-hover"`) — this test must fail
/// naming `lsp-hover` as manifest-missing.
#[test]
fn plugin_manifest_commands_match_defined_commands() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
    let root = std::path::Path::new(&manifest);
    let workspace_root = root.parent().expect("workspace root");
    let plugins_root = workspace_root.join("runtime/plugins/core");

    let mut violations: Vec<String> = Vec::new();

    let Ok(rd) = std::fs::read_dir(&plugins_root) else {
        panic!("cannot read {}", plugins_root.display());
    };
    let mut plugin_dirs: Vec<std::path::PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    plugin_dirs.sort();

    for dir in plugin_dirs {
        let manifest_path = dir.join("manifest.scm");
        if !manifest_path.exists() {
            continue; // no zero-arg declare-plugin activation — nothing to check
        }
        let plugin_name = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let manifest_src = std::fs::read_to_string(&manifest_path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest_path.display()));

        let declared: std::collections::BTreeSet<String> =
            manifest_declared_commands(&manifest_src)
                .into_iter()
                .collect();
        let defined: std::collections::BTreeSet<String> =
            defined_commands(&dir).into_iter().collect();

        for missing in declared.difference(&defined) {
            violations.push(format!(
                "  {plugin_name}: \"{missing}\" is in manifest.scm's #:commands \
                 but no define-command! defines it"
            ));
        }
        for extra in defined.difference(&declared) {
            violations.push(format!(
                "  {plugin_name}: \"{extra}\" is defined via define-command! but \
                 missing from manifest.scm's #:commands"
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "\nPlugin manifest.scm #:commands drifted from its own define-command! calls.\n\
         A missing manifest entry means that command never triggers lazy activation;\n\
         a stale one is dead weight. Keep both in sync.\n\
         Violations:\n{}\n",
        violations.join("\n")
    );
}
