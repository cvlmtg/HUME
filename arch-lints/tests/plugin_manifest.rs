//! # Plugin manifest command-list drift
//!
//! A plugin's `manifest.scm` (`#:commands '(...)` / `#:typed-commands '(...)`)
//! is the zero-argument `(declare-plugin! "core:foo")` activation list,
//! hand-maintained, and duplicated nowhere else the compiler checks. The two
//! clauses feed mutually exclusive lookups (`get_mappable` vs `get_typed`), so
//! a command added to a feature file without a matching manifest entry
//! silently never triggers lazy activation (the plugin loads, but the command
//! looks unbound until something else activates it); a stale manifest entry
//! for a deleted command is dead weight; and a name listed under the *wrong*
//! clause registers a stub the matching lookup never returns: the plugin
//! never activates for it at all, with nothing at build or run time to say
//! why.
//!
//! `plugin_manifest_commands_match_defined_commands` scans every
//! `runtime/plugins/core/*/manifest.scm` present, and for both
//! `#:commands`/`define-command!` and `#:typed-commands`/`define-typed-command!`
//! asserts the manifest clause, unioned over every `declare-plugin!` in the
//! manifest, is the exact same set (both directions) as the matching
//! `define-*!` calls found across that plugin's own `*.scm` files. A
//! declaration with `#:entry` additionally owns only the commands defined in
//! that entry file or the files it `require`s, directly or transitively: a
//! stub claimed by one entry and defined by another would activate the wrong
//! file, and so would a module that two entries both reach. A
//! name declared in one clause but defined by the other kind's `define-*!` is
//! reported once, as a wrong-clause violation, rather than as an unpaired
//! "missing" and "stale" entry on each side. A plugin directory with no
//! `manifest.scm` (no zero-arg `declare-plugin!` activation defined) is
//! skipped, not a violation.

use arch_lints::{quoted_strings, workspace_root};

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

/// The string literals inside `manifest.scm`'s `clause '(...)` list, scoped
/// to that one clause so `#:languages`/the plugin name's own quoted strings
/// elsewhere in the file are never mistaken for commands. `"#:typed-commands"`
/// does not contain `"#:commands"` as a substring, so searching for either
/// literal independently can't cross-match the other's clause. Empty (not a
/// violation by itself) if the manifest declares no such clause at all.
/// Comment lines are stripped first: every `manifest.scm` opens with a
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

/// `src` without its `;`-comment lines: a doc comment mentioning a
/// `define-*!` or `require` call in prose must never be mistaken for one.
fn code_only(src: &str) -> String {
    src.lines()
        .filter(|line| !line.trim_start().starts_with(';'))
        .collect::<Vec<_>>()
        .join("\n")
}

fn read_scm(dir: &std::path::Path, file: &str) -> String {
    let path = dir.join(file);
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    code_only(&src)
}

/// Every `*.scm` file name directly inside `dir` (plugins don't nest
/// subdirectories).
fn scm_files(dir: &std::path::Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<String> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("scm"))
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    files.sort();
    files
}

/// Every name in a `(definer "name" ...)` call, across `files` inside `dir`.
/// The match is anchored on the opening paren (`(define-typed-command!`, not
/// the bare word) so a call like `(#%register-global "define-typed-command!")`,
/// which mentions the definer only as a quoted string argument, is never
/// mistaken for a defining call.
fn defined_commands(dir: &std::path::Path, files: &[String], definer: &str) -> Vec<String> {
    let mut names = Vec::new();
    let needle = format!("({definer}");
    for file in files {
        let src = read_scm(dir, file);
        for (idx, _) in src.match_indices(&needle) {
            let after = &src[idx + needle.len()..];
            if let Some(name) = quoted_strings(after).into_iter().next() {
                names.push(name);
            }
        }
    }
    names
}

/// The file names in `(require "x.scm")` forms of `src`.
fn required_names(src: &str) -> Vec<String> {
    src.match_indices("(require \"")
        .filter_map(|(idx, needle)| {
            quoted_strings(&src[idx + needle.len() - 1..])
                .into_iter()
                .next()
        })
        .collect()
}

/// `entry` plus every file it `require`s, directly or transitively.
fn entry_closure(dir: &std::path::Path, entry: &str) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut pending = vec![entry.to_string()];
    while let Some(file) = pending.pop() {
        if seen.contains(&file) || !dir.join(&file).exists() {
            continue;
        }
        pending.extend(required_names(&read_scm(dir, &file)));
        seen.push(file);
    }
    seen
}

/// The text of every balanced `(declare-plugin! ...)` form in comment-free
/// `src`.
fn declare_forms(src: &str) -> Vec<String> {
    let mut forms = Vec::new();
    for (start, _) in src.match_indices("(declare-plugin!") {
        let mut depth = 0i32;
        for (i, c) in src[start..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        forms.push(src[start..start + i + 1].to_string());
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    forms
}

/// The file a `declare-plugin!` form's `#:entry` names, `plugin.scm` when it
/// has none.
fn form_entry(form: &str) -> String {
    form.find("#:entry")
        .and_then(|at| quoted_strings(&form[at..]).into_iter().next())
        .unwrap_or_else(|| "plugin.scm".to_string())
}

/// The entries other than `entry` whose closure also defines `name`. Each item
/// pairs an entry file with every command name its closure defines. A module
/// shared by two closures runs under whichever entry loads first, so a command
/// defined there is claimed by the wrong entry half the time.
fn other_entries_defining<'a>(
    entries: &'a [(String, std::collections::BTreeSet<String>)],
    entry: &str,
    name: &str,
) -> Vec<&'a str> {
    entries
        .iter()
        .filter(|(other, defined)| other != entry && defined.contains(name))
        .map(|(other, _)| other.as_str())
        .collect()
}

/// A command defined by a plugin but absent from its manifest is reported
/// as missing. A typed command listed under `#:commands` (or the reverse)
/// is reported once, as being in the wrong clause.
#[test]
fn plugin_manifest_commands_match_defined_commands() {
    let workspace_root = workspace_root();
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
            continue; // no zero-arg declare-plugin! activation, nothing to check
        }
        let plugin_name = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let manifest_src = code_only(
            &std::fs::read_to_string(&manifest_path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest_path.display())),
        );
        let forms = declare_forms(&manifest_src);
        let files = scm_files(&dir);

        // Both clauses' declared/defined sets are gathered up front so the
        // loop below can check each kind against the *other* kind's sets to
        // detect a name in the wrong clause, not just missing/stale.
        let declared: Vec<std::collections::BTreeSet<String>> = KINDS
            .iter()
            .map(|k| {
                forms
                    .iter()
                    .flat_map(|form| manifest_clause_names(form, k.clause))
                    .collect()
            })
            .collect();
        let defined: Vec<std::collections::BTreeSet<String>> = KINDS
            .iter()
            .map(|k| {
                defined_commands(&dir, &files, k.definer)
                    .into_iter()
                    .collect()
            })
            .collect();

        for (i, kind) in KINDS.iter().enumerate() {
            let per_entry: Vec<(String, std::collections::BTreeSet<String>)> = forms
                .iter()
                .map(|form| {
                    let entry = form_entry(form);
                    let closure = entry_closure(&dir, &entry);
                    let in_entry = defined_commands(&dir, &closure, kind.definer)
                        .into_iter()
                        .collect();
                    (entry, in_entry)
                })
                .collect();
            for (form, (entry, in_entry)) in forms.iter().zip(&per_entry) {
                for name in manifest_clause_names(form, kind.clause) {
                    if defined[i].contains(&name) && !in_entry.contains(&name) {
                        violations.push(format!(
                            "  {plugin_name}: \"{name}\" is declared for entry {entry} \
                             but {} defines it outside that file and what it requires",
                            kind.definer
                        ));
                    }
                    for other in other_entries_defining(&per_entry, entry, &name) {
                        violations.push(format!(
                            "  {plugin_name}: \"{name}\" is declared for entry {entry} \
                             but {other}'s requires also reach a {} for it",
                            kind.definer
                        ));
                    }
                }
            }
        }

        for (i, kind) in KINDS.iter().enumerate() {
            let other = 1 - i;

            for missing in declared[i].difference(&defined[i]) {
                if defined[other].contains(missing) {
                    violations.push(format!(
                        "  {plugin_name}: \"{missing}\" is listed in manifest.scm's \
                         {} but is defined by {}: it belongs in {}",
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
                // wrong-clause violation. Skip so one mistake yields one line.
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

#[test]
fn declare_forms_splits_a_manifest_and_reads_each_entry() {
    let src = "(declare-plugin! \"p\" #:languages '(\"*\"))\n\
               (declare-plugin! \"p\" #:entry \"x.scm\" #:typed-commands '(\"c\"))";
    let forms = declare_forms(src);
    assert_eq!(forms.len(), 2);
    assert_eq!(form_entry(&forms[0]), "plugin.scm");
    assert_eq!(form_entry(&forms[1]), "x.scm");
    assert_eq!(
        manifest_clause_names(&forms[1], "#:typed-commands"),
        vec!["c"]
    );
    assert!(manifest_clause_names(&forms[0], "#:typed-commands").is_empty());
}

#[test]
fn required_names_reads_every_require() {
    let src = "(require \"a.scm\")\n(require \"b.scm\")\n(define x 1)";
    assert_eq!(required_names(src), vec!["a.scm", "b.scm"]);
}

#[test]
fn other_entries_defining_finds_a_name_shared_through_another_closure() {
    let defs = |names: &[&str]| -> std::collections::BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    };
    let entries = vec![
        ("plugin.scm".to_string(), defs(&["a", "c"])),
        ("x.scm".to_string(), defs(&["c", "d"])),
    ];
    assert_eq!(
        other_entries_defining(&entries, "x.scm", "c"),
        vec!["plugin.scm"]
    );
    assert!(other_entries_defining(&entries, "x.scm", "d").is_empty());
    assert!(other_entries_defining(&entries, "plugin.scm", "a").is_empty());
}
