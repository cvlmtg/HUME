//! # Plugin manifest command-list drift
//!
//! A plugin's `manifest.scm` (`#:commands '(...)` / `#:typed-commands '(...)`)
//! is the zero-argument `(declare-plugin "core:foo")` activation list —
//! hand-maintained, and duplicated nowhere else the compiler checks. The two
//! clauses feed mutually exclusive lookups (`get_mappable` vs `get_typed`), so
//! a command added to a feature file without a matching manifest entry
//! silently never triggers lazy activation (the plugin loads, but the command
//! looks unbound until something else activates it); a stale manifest entry
//! for a deleted command is dead weight; and a name listed under the *wrong*
//! clause registers a stub the matching lookup never returns — the plugin
//! never activates for it at all, with nothing at build or run time to say
//! why.
//!
//! `plugin_manifest_commands_match_defined_commands` scans every
//! `runtime/plugins/core/*/manifest.scm` present, and for both
//! `#:commands`/`define-command!` and `#:typed-commands`/`define-typed-command!`
//! asserts the manifest clause is the exact same set (both directions) as the
//! matching `define-*!` calls found across that plugin's own `*.scm` files. A
//! name declared in one clause but defined by the other kind's `define-*!` is
//! reported once, as a wrong-clause violation, rather than as an unpaired
//! "missing" and "stale" entry on each side. A plugin directory with no
//! `manifest.scm` (no zero-arg `declare-plugin` activation defined) is
//! skipped, not a violation.

use super::quoted_strings;

/// One manifest clause paired with the `define-*!` call that must back it.
/// Bundling them in one constant (rather than two independently-indexed
/// lists) means a call site can't accidentally check `#:typed-commands`
/// against `define-command!`.
struct CommandKind {
    clause: &'static str,
    definer: &'static str,
}

const KINDS: [CommandKind; 2] = [
    CommandKind {
        clause: "#:commands",
        definer: "define-command!",
    },
    CommandKind {
        clause: "#:typed-commands",
        definer: "define-typed-command!",
    },
];

/// The string literals inside `manifest.scm`'s `clause '(...)` list — scoped
/// to that one clause so `#:languages`/the plugin name's own quoted strings
/// elsewhere in the file are never mistaken for commands. `"#:typed-commands"`
/// does not contain `"#:commands"` as a substring, so searching for either
/// literal independently can't cross-match the other's clause. Empty (not a
/// violation by itself) if the manifest declares no such clause at all.
/// Comment lines are stripped first — every `manifest.scm` opens with a
/// `;`-comment header that mentions both clauses in prose, which would
/// otherwise be the *first* (wrong) match.
fn manifest_clause_names(src: &str, clause: &str) -> Vec<String> {
    let code_only: String = src
        .lines()
        .filter(|line| !line.trim_start().starts_with(';'))
        .collect::<Vec<_>>()
        .join("\n");
    let src = &code_only;
    let Some(after) = src.find(clause) else {
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

/// Every name in a `(definer "name" ...)` call, across every `*.scm` file
/// directly inside `dir` (plugins don't nest subdirectories). The match is
/// anchored on the opening paren (`(define-typed-command!`, not the bare
/// word) so a call like `(#%register-global "define-typed-command!")` —
/// which mentions the definer only as a quoted string argument — is never
/// mistaken for a defining call.
fn defined_commands(dir: &std::path::Path, definer: &str) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return names;
    };
    let needle = format!("({definer}");
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("scm") {
            continue;
        }
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        // Comment lines stripped first, same reason as
        // `manifest_clause_names` — a doc comment mentioning a `define-*!`
        // call in prose must never be mistaken for one.
        let code_only: String = src
            .lines()
            .filter(|line| !line.trim_start().starts_with(';'))
            .collect::<Vec<_>>()
            .join("\n");
        for (idx, _) in code_only.match_indices(&needle) {
            let after = &code_only[idx + needle.len()..];
            if let Some(name) = quoted_strings(after).into_iter().next() {
                names.push(name);
            }
        }
    }
    names
}

/// Fail oracle: comment out one entry in `core:lsp/manifest.scm`'s
/// `#:commands` list (e.g. delete `"lsp-hover"`) — this test must fail
/// naming `lsp-hover` as manifest-missing. Move `"lsp-status"` from
/// `#:typed-commands` to `#:commands` in the same manifest — this test must
/// fail naming `lsp-status` as belonging in `#:typed-commands`, as a single
/// wrong-clause violation (not two unpaired missing/stale lines).
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

        // Both clauses' declared/defined sets are gathered up front so the
        // loop below can check each kind against the *other* kind's sets to
        // detect a name in the wrong clause, not just missing/stale.
        let declared: Vec<std::collections::BTreeSet<String>> = KINDS
            .iter()
            .map(|k| {
                manifest_clause_names(&manifest_src, k.clause)
                    .into_iter()
                    .collect()
            })
            .collect();
        let defined: Vec<std::collections::BTreeSet<String>> = KINDS
            .iter()
            .map(|k| defined_commands(&dir, k.definer).into_iter().collect())
            .collect();

        for (i, kind) in KINDS.iter().enumerate() {
            let other = 1 - i;

            for missing in declared[i].difference(&defined[i]) {
                if defined[other].contains(missing) {
                    violations.push(format!(
                        "  {plugin_name}: \"{missing}\" is listed in manifest.scm's \
                         {} but is defined by {} — it belongs in {}",
                        kind.clause, KINDS[other].definer, KINDS[other].clause
                    ));
                } else {
                    violations.push(format!(
                        "  {plugin_name}: \"{missing}\" is in manifest.scm's {} \
                         but no {} defines it",
                        kind.clause, kind.definer
                    ));
                }
            }
            for extra in defined[i].difference(&declared[i]) {
                // Already reported (from the other kind's pass) as a
                // wrong-clause violation — skip so one mistake yields one line.
                if declared[other].contains(extra) {
                    continue;
                }
                violations.push(format!(
                    "  {plugin_name}: \"{extra}\" is defined via {} but missing \
                     from manifest.scm's {}",
                    kind.definer, kind.clause
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "\nPlugin manifest.scm #:commands/#:typed-commands drifted from its own \
         define-command!/define-typed-command! calls.\n\
         A missing manifest entry means that command never triggers lazy activation;\n\
         a stale one is dead weight; a name in the wrong clause registers a stub the\n\
         matching lookup (get_mappable vs get_typed) never returns, so the command\n\
         never activates. Keep manifest and definitions in sync.\n\
         Violations:\n{}\n",
        violations.join("\n")
    );
}
