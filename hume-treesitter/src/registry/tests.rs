use std::path::Path;

use globset::Glob;

use super::{ConfigureOutcome, ConfiguredBy, LanguageRegistry, QueryPaths, detect_language};
use hume_engine::theme::ScopeRegistry;
use test_fixtures::{
    grammar_parser_path, grammar_query_path, helix_textobjects_path, require_fixture_file,
    require_grammars,
};

/// Write `src` to `<tmp>/<filename>` and return its path (kept alive via the
/// returned `TempDir`).
fn write_temp_scm(filename: &str, src: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(filename);
    std::fs::write(&path, src).unwrap();
    (dir, path)
}

/// Parse each pattern into a `Glob`, panicking on a malformed test fixture:
/// `register_identity` takes pre-parsed globs, so invalid syntax is now a
/// caller bug, not a runtime error to test for.
fn globs(pats: &[&str]) -> Vec<Glob> {
    pats.iter()
        .map(|p| Glob::new(p).expect("test glob must compile"))
        .collect()
}

#[test]
fn attach_grammar_with_valid_injections_populates_bundle() {
    require_grammars(&["rust"]);
    let parser_path = grammar_parser_path("rust");
    let hl_path = grammar_query_path("rust");
    let (_dir, inj_path) = write_temp_scm(
        "injections.scm",
        r#"((_) @injection.content (#set! injection.language "markdown"))"#,
    );

    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let bundle = reg
        .attach_grammar(
            "rust",
            &parser_path,
            "tree_sitter_rust",
            QueryPaths {
                highlights: &hl_path,
                injections: Some(&inj_path),
                textobjects: None,
            },
            &mut scope_reg,
        )
        .expect("attach with valid injections must succeed");

    let injections = bundle
        .injections
        .as_ref()
        .expect("injections query must be populated");
    assert!(
        injections.content_capture.is_some(),
        "injection.content capture must be found"
    );
    assert_eq!(injections.patterns.len(), 1);
    assert_eq!(
        injections.patterns[0].language.as_deref(),
        Some("markdown"),
        "static #set! injection.language must be captured"
    );
}

#[test]
fn attach_grammar_without_injections_path_leaves_injections_none() {
    require_grammars(&["rust"]);
    let parser_path = grammar_parser_path("rust");
    let hl_path = grammar_query_path("rust");

    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let bundle = reg
        .attach_grammar(
            "rust",
            &parser_path,
            "tree_sitter_rust",
            QueryPaths::highlights_only(&hl_path),
            &mut scope_reg,
        )
        .expect("attach without injections must succeed");

    assert!(
        bundle.injections.is_none(),
        "no injections_path given → injections must stay None"
    );
}

/// A broken injections.scm hard-fails the whole attach (same as a broken
/// highlights.scm) rather than degrading to a warning. Both files come
/// from the same trusted pinned source, so there is no separate soft-fail
/// path.
#[test]
fn attach_grammar_with_broken_injections_fails_whole_attach() {
    require_grammars(&["rust"]);
    let parser_path = grammar_parser_path("rust");
    let hl_path = grammar_query_path("rust");
    let (_dir, inj_path) = write_temp_scm(
        "injections.scm",
        "(this is not valid tree-sitter query syntax",
    );

    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let result = reg.attach_grammar(
        "rust",
        &parser_path,
        "tree_sitter_rust",
        QueryPaths {
            highlights: &hl_path,
            injections: Some(&inj_path),
            textobjects: None,
        },
        &mut scope_reg,
    );
    let Err(err) = result else {
        panic!("broken injections.scm must fail the attach");
    };
    assert!(
        matches!(err, super::RegisterError::InjectionsQueryBuild(_)),
        "broken injections.scm must surface as InjectionsQueryBuild, got: {err:?}"
    );
    assert!(
        reg.by_name("rust").is_none(),
        "a failed attach must not leave a partial identity behind"
    );
}

#[test]
fn attach_grammar_with_valid_textobjects_populates_bundle() {
    require_grammars(&["rust"]);
    let parser_path = grammar_parser_path("rust");
    let hl_path = grammar_query_path("rust");
    let to_path = helix_textobjects_path("rust")
        .expect("rust helix-textobjects.scm fixture (run scripts/fetch-test-grammars.sh)");
    require_fixture_file(&to_path, "rust helix-textobjects.scm");

    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let bundle = reg
        .attach_grammar(
            "rust",
            &parser_path,
            "tree_sitter_rust",
            QueryPaths {
                highlights: &hl_path,
                injections: None,
                textobjects: Some(&to_path),
            },
            &mut scope_reg,
        )
        .expect("attach with valid textobjects must succeed");

    let textobjects = bundle
        .textobjects
        .as_ref()
        .expect("textobjects query must be populated");
    assert!(
        textobjects.defines(
            crate::textobjects::ObjectKind::Function,
            crate::textobjects::ObjectSpan::Around
        ),
        "function.around must be defined by the real rust textobjects query"
    );
}

#[test]
fn attach_grammar_without_textobjects_path_leaves_textobjects_none() {
    require_grammars(&["rust"]);
    let parser_path = grammar_parser_path("rust");
    let hl_path = grammar_query_path("rust");

    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let bundle = reg
        .attach_grammar(
            "rust",
            &parser_path,
            "tree_sitter_rust",
            QueryPaths::highlights_only(&hl_path),
            &mut scope_reg,
        )
        .expect("attach without textobjects must succeed");

    assert!(
        bundle.textobjects.is_none(),
        "no textobjects_path given → textobjects must stay None"
    );
}

/// A broken textobjects.scm hard-fails the whole attach, same policy as a
/// broken injections.scm (see `attach_grammar_with_broken_injections_fails_whole_attach`).
#[test]
fn attach_grammar_with_broken_textobjects_fails_whole_attach() {
    require_grammars(&["rust"]);
    let parser_path = grammar_parser_path("rust");
    let hl_path = grammar_query_path("rust");
    let (_dir, to_path) = write_temp_scm(
        "textobjects.scm",
        "(this is not valid tree-sitter query syntax",
    );

    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let result = reg.attach_grammar(
        "rust",
        &parser_path,
        "tree_sitter_rust",
        QueryPaths {
            highlights: &hl_path,
            injections: None,
            textobjects: Some(&to_path),
        },
        &mut scope_reg,
    );
    let Err(err) = result else {
        panic!("broken textobjects.scm must fail the attach");
    };
    assert!(
        matches!(err, super::RegisterError::TextObjectsQueryBuild(_)),
        "broken textobjects.scm must surface as TextObjectsQueryBuild, got: {err:?}"
    );
    assert!(
        reg.by_name("rust").is_none(),
        "a failed attach must not leave a partial identity behind"
    );
}

#[test]
fn language_registry_by_ext_lookup_empty() {
    let reg = LanguageRegistry::new();
    assert!(reg.by_extension("rs").is_none());
}

#[test]
fn language_registry_remove_is_idempotent() {
    let mut reg = LanguageRegistry::new();
    assert!(reg.remove("rust").is_none());
}

#[test]
fn register_identity_then_by_name_returns_entry() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("toml", &["toml"], &[], &[]).unwrap();
    assert!(
        reg.by_name("toml").is_some(),
        "identity should be registered"
    );
    let id = reg.id_of("toml").expect("toml must be interned");
    assert_eq!(reg.name_of(id), "toml");
    assert!(
        reg.by_extension("toml").is_some(),
        "extension lookup must work after identity reg"
    );
    // Unknown ext should not match.
    assert!(reg.by_extension("yaml").is_none());
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn roots_of_returns_the_configured_markers() {
    let mut reg = LanguageRegistry::new();
    let id = reg.register_identity_no_rebuild("go", &["go"], &[], &[]);
    assert_eq!(
        reg.configure_roots(id, strings(&["go.work", "go.mod"]), ConfiguredBy::User),
        ConfigureOutcome::Applied
    );
    assert_eq!(reg.roots_of(id), ["go.work", "go.mod"]);
}

#[test]
fn roots_of_an_interned_language_without_identity_is_empty() {
    let mut reg = LanguageRegistry::new();
    let id = reg.intern("bare");
    assert!(reg.roots_of(id).is_empty());
}

#[test]
fn reregistering_an_identity_keeps_its_configured_roots() {
    let mut reg = LanguageRegistry::new();
    let id = reg.register_identity_no_rebuild("go", &[], &[], &[]);
    assert_eq!(
        reg.configure_roots(id, strings(&["go.mod"]), ConfiguredBy::User),
        ConfigureOutcome::Applied
    );
    reg.register_identity_no_rebuild("go", &["go"], &[], &[]);
    assert_eq!(reg.roots_of(id), ["go.mod"]);
}

#[test]
fn bundled_values_fill_a_language_with_none_configured() {
    let mut reg = LanguageRegistry::new();
    let id = reg.register_identity_no_rebuild("tsx", &[], &[], &[]);
    assert!(!reg.bundled_loaded(id));

    reg.fill_bundled(
        id,
        strings(&["package.json"]),
        Some("typescriptreact".to_string()),
    );

    assert!(reg.bundled_loaded(id));
    assert_eq!(reg.roots_of(id), ["package.json"]);
    assert_eq!(reg.lsp_language_id_of(id), "typescriptreact");
}

#[test]
fn bundled_values_never_replace_configured_ones() {
    let mut reg = LanguageRegistry::new();
    let id = reg.register_identity_no_rebuild("tsx", &[], &[], &[]);
    assert_eq!(
        reg.configure_roots(id, strings(&["deno.json"]), ConfiguredBy::User),
        ConfigureOutcome::Applied
    );
    assert_eq!(
        reg.configure_lsp_language_id(id, "tsx-custom".to_string(), ConfiguredBy::User),
        ConfigureOutcome::Applied
    );

    reg.fill_bundled(
        id,
        strings(&["package.json"]),
        Some("typescriptreact".to_string()),
    );

    assert_eq!(reg.roots_of(id), ["deno.json"]);
    assert_eq!(reg.lsp_language_id_of(id), "tsx-custom");
}

#[test]
fn bundled_values_fill_only_the_fields_left_unset() {
    let mut reg = LanguageRegistry::new();
    let id = reg.register_identity_no_rebuild("tsx", &[], &[], &[]);
    assert_eq!(
        reg.configure_roots(id, strings(&["deno.json"]), ConfiguredBy::User),
        ConfigureOutcome::Applied
    );

    reg.fill_bundled(
        id,
        strings(&["package.json"]),
        Some("typescriptreact".to_string()),
    );

    assert_eq!(reg.roots_of(id), ["deno.json"]);
    assert_eq!(reg.lsp_language_id_of(id), "typescriptreact");
}

#[test]
fn a_configured_value_replaces_a_bundled_one() {
    let mut reg = LanguageRegistry::new();
    let id = reg.register_identity_no_rebuild("tsx", &[], &[], &[]);
    reg.fill_bundled(
        id,
        strings(&["package.json"]),
        Some("typescriptreact".to_string()),
    );

    assert_eq!(
        reg.configure_roots(id, strings(&["deno.json"]), ConfiguredBy::User),
        ConfigureOutcome::Applied
    );
    assert_eq!(
        reg.configure_lsp_language_id(id, "tsx-custom".to_string(), ConfiguredBy::User),
        ConfigureOutcome::Applied
    );

    assert_eq!(reg.roots_of(id), ["deno.json"]);
    assert_eq!(reg.lsp_language_id_of(id), "tsx-custom");
}

#[test]
fn a_second_configured_value_is_ignored() {
    let mut reg = LanguageRegistry::new();
    let id = reg.register_identity_no_rebuild("tsx", &[], &[], &[]);
    assert_eq!(
        reg.configure_roots(id, strings(&["deno.json"]), ConfiguredBy::User),
        ConfigureOutcome::Applied
    );
    assert_eq!(
        reg.configure_lsp_language_id(id, "first".to_string(), ConfiguredBy::User),
        ConfigureOutcome::Applied
    );

    assert_eq!(
        reg.configure_roots(id, strings(&["package.json"]), ConfiguredBy::User),
        ConfigureOutcome::Ignored
    );
    assert_eq!(
        reg.configure_lsp_language_id(id, "second".to_string(), ConfiguredBy::User),
        ConfigureOutcome::Ignored
    );

    assert_eq!(reg.roots_of(id), ["deno.json"]);
    assert_eq!(reg.lsp_language_id_of(id), "first");
}

#[test]
fn a_second_plugin_value_is_ignored() {
    let mut reg = LanguageRegistry::new();
    let id = reg.register_identity_no_rebuild("tsx", &[], &[], &[]);
    assert_eq!(
        reg.configure_roots(id, strings(&["deno.json"]), ConfiguredBy::Plugin),
        ConfigureOutcome::Applied
    );

    assert_eq!(
        reg.configure_roots(id, strings(&["package.json"]), ConfiguredBy::Plugin),
        ConfigureOutcome::Ignored
    );

    assert_eq!(reg.roots_of(id), ["deno.json"]);
}

#[test]
fn a_user_value_replaces_a_plugin_value() {
    let mut reg = LanguageRegistry::new();
    let id = reg.register_identity_no_rebuild("tsx", &[], &[], &[]);
    assert_eq!(
        reg.configure_roots(id, strings(&["deno.json"]), ConfiguredBy::Plugin),
        ConfigureOutcome::Applied
    );
    assert_eq!(
        reg.configure_lsp_language_id(id, "plugin".to_string(), ConfiguredBy::Plugin),
        ConfigureOutcome::Applied
    );

    assert_eq!(
        reg.configure_roots(id, strings(&["package.json"]), ConfiguredBy::User),
        ConfigureOutcome::Applied
    );
    assert_eq!(
        reg.configure_lsp_language_id(id, "user".to_string(), ConfiguredBy::User),
        ConfigureOutcome::Applied
    );

    assert_eq!(reg.roots_of(id), ["package.json"]);
    assert_eq!(reg.lsp_language_id_of(id), "user");
}

#[test]
fn a_plugin_value_after_a_user_value_is_shadowed() {
    let mut reg = LanguageRegistry::new();
    let id = reg.register_identity_no_rebuild("tsx", &[], &[], &[]);
    assert_eq!(
        reg.configure_roots(id, strings(&["package.json"]), ConfiguredBy::User),
        ConfigureOutcome::Applied
    );
    assert_eq!(
        reg.configure_lsp_language_id(id, "user".to_string(), ConfiguredBy::User),
        ConfigureOutcome::Applied
    );

    assert_eq!(
        reg.configure_roots(id, strings(&["deno.json"]), ConfiguredBy::Plugin),
        ConfigureOutcome::Shadowed
    );
    assert_eq!(
        reg.configure_lsp_language_id(id, "plugin".to_string(), ConfiguredBy::Plugin),
        ConfigureOutcome::Shadowed
    );

    assert_eq!(reg.roots_of(id), ["package.json"]);
    assert_eq!(reg.lsp_language_id_of(id), "user");
}

#[test]
fn an_empty_bundled_fill_still_marks_the_language_loaded() {
    let mut reg = LanguageRegistry::new();
    let id = reg.register_identity_no_rebuild("plain", &[], &[], &[]);

    reg.fill_bundled(id, Vec::new(), None);

    assert!(reg.bundled_loaded(id));
    assert!(reg.roots_of(id).is_empty());
    assert_eq!(reg.lsp_language_id_of(id), "plain");
}

#[test]
fn register_identity_with_globs_lookup() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("makefile", &[], &globs(&["Makefile", "GNUmakefile"]), &[])
        .unwrap();
    let matches = reg.compiled_globs().matches(Path::new("Makefile"));
    assert!(!matches.is_empty(), "Makefile should match registered glob");
    let name = reg.glob_lang_id(matches[0]).map(|id| reg.name_of(id));
    assert_eq!(name, Some("makefile"));
    // A path the globs don't cover yields no match.
    assert!(
        reg.compiled_globs()
            .matches(Path::new("Cargo.toml"))
            .is_empty()
    );
}

#[test]
fn remove_clears_glob_and_shebang_entries() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("python", &["py"], &globs(&["*.py"]), &["python"])
        .unwrap();
    assert!(reg.by_extension("py").is_some());
    assert!(reg.by_shebang("python").is_some());
    assert!(!reg.compiled_globs().matches(Path::new("foo.py")).is_empty());

    reg.remove("python");

    assert!(reg.by_extension("py").is_none());
    assert!(reg.by_shebang("python").is_none());
    assert!(reg.compiled_globs().matches(Path::new("foo.py")).is_empty());
}

/// `deindex` must only remove an index entry it still owns.
/// `c` and `cpp` both claim `.h` (last-registered wins, so `cpp` takes
/// it); re-registering `c` without `.h` must not evict `cpp`'s mapping:
/// `c` never owned it at the time of re-registration.
#[test]
fn deindex_does_not_clobber_another_languages_shared_extension() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("c", &["c", "h"], &[], &[]).unwrap();
    reg.register_identity("cpp", &["cpp", "h"], &[], &[])
        .unwrap();
    assert_eq!(
        reg.by_extension("h"),
        reg.id_of("cpp"),
        "cpp must win the shared .h extension (last-registered-wins)"
    );

    // Re-register c, dropping .h from its own extension list.
    reg.register_identity("c", &["c"], &[], &[]).unwrap();

    assert_eq!(
        reg.by_extension("h"),
        reg.id_of("cpp"),
        "re-registering c must not clobber cpp's still-current .h mapping"
    );
    assert_eq!(reg.by_extension("c"), reg.id_of("c"));
}

/// A `define-language!` override in `init.scm` (e.g. adding an extension to
/// an already-grammared language) must not undo the grammar `grammars.scm`
/// attached at startup. Identity and grammar are independent facts.
///
/// If `register_identity_no_rebuild` cleared `grammars[id]`, both
/// `has_grammar` and `grammar_snapshot` would come back empty here.
#[test]
fn attached_grammar_survives_identity_re_registration() {
    require_grammars(&["rust"]);
    let parser_path = grammar_parser_path("rust");
    let hl_path = grammar_query_path("rust");

    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    reg.attach_grammar(
        "rust",
        &parser_path,
        "tree_sitter_rust",
        QueryPaths::highlights_only(&hl_path),
        &mut scope_reg,
    )
    .expect("attach must succeed");
    assert!(reg.has_grammar("rust"), "sanity: grammar attached");

    // Re-register identity, as `define-language!` would from init.scm to add
    // an extra extension.
    reg.register_identity("rust", &["rs", "mylang"], &[], &[])
        .expect("re-registering identity must succeed");

    assert!(
        reg.has_grammar("rust"),
        "identity re-registration must keep the attached grammar"
    );
    let id = reg.id_of("rust").expect("rust must still be interned");
    assert!(
        reg.grammar(id).is_some(),
        "grammar(id) must still resolve after re-registration"
    );
    assert!(
        reg.grammar_snapshot().contains_key("rust"),
        "grammar_snapshot must still carry rust after re-registration"
    );

    // The identity replacement itself must still have taken effect.
    let detected = detect_language(Some(Path::new("foo.mylang")), None, &reg);
    assert_eq!(detected, reg.id_of("rust"));
}

/// With no `lsp_language_id` override, the wire `languageId` falls back to
/// the language's own name.
#[test]
fn lsp_language_id_of_falls_back_to_name_when_unset() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("rust", &["rs"], &[], &[]).unwrap();
    let id = reg.id_of("rust").unwrap();
    assert_eq!(reg.lsp_language_id_of(id), "rust");
}

/// A configured `languageId` is returned verbatim instead of the name: the
/// case the override exists for (`tsx` -> `typescriptreact`).
#[test]
fn lsp_language_id_of_returns_the_configured_override() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("tsx", &["tsx"], &[], &[]).unwrap();
    let id = reg.id_of("tsx").unwrap();
    assert_eq!(
        reg.configure_lsp_language_id(id, "typescriptreact".to_string(), ConfiguredBy::User),
        ConfigureOutcome::Applied
    );
    assert_eq!(reg.lsp_language_id_of(id), "typescriptreact");
}

/// The `languageId` is per-language configuration, not identity:
/// `define-language!` replacing the detection data leaves it in place.
#[test]
fn identity_re_registration_keeps_the_configured_lsp_language_id() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("tsx", &["tsx"], &[], &[]).unwrap();
    let id = reg.id_of("tsx").unwrap();
    assert_eq!(
        reg.configure_lsp_language_id(id, "typescriptreact".to_string(), ConfiguredBy::User),
        ConfigureOutcome::Applied
    );

    reg.register_identity("tsx", &["tsx", "mytsx"], &[], &[])
        .unwrap();

    assert_eq!(reg.lsp_language_id_of(id), "typescriptreact");
}

#[test]
fn detect_language_by_extension() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("rust", &["rs"], &[], &[]).unwrap();
    let id = detect_language(Some(Path::new("foo.rs")), None, &reg);
    assert_eq!(id, reg.id_of("rust"));
    // A different extension must not detect.
    let no_match = detect_language(Some(Path::new("foo.py")), None, &reg);
    assert!(no_match.is_none());
}

#[test]
fn detect_language_by_glob() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("makefile", &[], &globs(&["Makefile", "GNUmakefile"]), &[])
        .unwrap();
    let id = detect_language(Some(Path::new("/project/Makefile")), None, &reg);
    assert_eq!(id, reg.id_of("makefile"));
    let no_match = detect_language(Some(Path::new("/project/other")), None, &reg);
    assert!(no_match.is_none());
}

#[test]
fn detect_language_glob_beats_extension() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("typescript", &["ts"], &[], &[])
        .unwrap();
    reg.register_identity(
        "tsconfig",
        &[],
        &globs(&["tsconfig.json", "*.config.json"]),
        &[],
    )
    .unwrap();
    reg.register_identity("json", &["json"], &[], &[]).unwrap();
    let id = detect_language(Some(Path::new("tsconfig.json")), None, &reg);
    assert_eq!(id, reg.id_of("tsconfig"));
    // A plain .json that no glob covers still detects as json.
    let plain = detect_language(Some(Path::new("other.json")), None, &reg);
    assert_eq!(plain, reg.id_of("json"));
}

#[test]
fn detect_language_glob_tiebreak_last_registered_wins() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("generic-json", &[], &globs(&["*.json"]), &[])
        .unwrap();
    reg.register_identity("strict-json", &[], &globs(&["*.json"]), &[])
        .unwrap();
    assert_eq!(
        detect_language(Some(Path::new("config.json")), None, &reg),
        reg.id_of("strict-json"),
    );

    let mut reg2 = LanguageRegistry::new();
    reg2.register_identity("strict-json", &[], &globs(&["*.json"]), &[])
        .unwrap();
    reg2.register_identity("generic-json", &[], &globs(&["*.json"]), &[])
        .unwrap();
    assert_eq!(
        detect_language(Some(Path::new("config.json")), None, &reg2),
        reg2.id_of("generic-json"),
    );
}

#[test]
fn detect_language_by_shebang() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("python", &["py"], &[], &["python3", "python"])
        .unwrap();
    let id = detect_language(
        Some(Path::new("script")),
        Some("#!/usr/bin/env python3"),
        &reg,
    );
    assert_eq!(id, reg.id_of("python"));
    // Some other shebang must not match.
    let no_match = detect_language(Some(Path::new("script")), Some("#!/bin/bash"), &reg);
    assert!(no_match.is_none());
}

#[test]
fn detect_language_shebang_direct_path() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("bash", &["sh"], &[], &["bash"])
        .unwrap();
    // Extension wins over shebang.
    let id = detect_language(Some(Path::new("run.sh")), Some("#!/bin/bash"), &reg);
    assert_eq!(id, reg.id_of("bash"));
    // Without extension, shebang is used.
    let id2 = detect_language(Some(Path::new("run")), Some("#!/bin/bash"), &reg);
    assert_eq!(id2, reg.id_of("bash"));
}

#[test]
fn detect_language_no_match() {
    let reg = LanguageRegistry::new();
    assert!(detect_language(Some(Path::new("foo.xyz")), None, &reg).is_none());
    assert!(detect_language(None, None, &reg).is_none());
}

/// Extensions are matched case-sensitively, so `"c"` and `"C"` map to
/// distinct languages: `foo.c` detects as `c`, `foo.C` detects as `cpp`.
///
/// Folding extensions to lowercase would send both files to cpp, the
/// later-registered language.
#[test]
fn extension_matching_is_case_sensitive() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("c", &["c"], &[], &[]).unwrap();
    reg.register_identity("cpp", &["C"], &[], &[]).unwrap();
    assert_eq!(
        detect_language(Some(Path::new("foo.c")), None, &reg),
        reg.id_of("c"),
    );
    assert_eq!(
        detect_language(Some(Path::new("foo.C")), None, &reg),
        reg.id_of("cpp"),
    );
    // Sanity: unrelated extension is still None.
    assert!(detect_language(Some(Path::new("foo.rs")), None, &reg).is_none());
}
