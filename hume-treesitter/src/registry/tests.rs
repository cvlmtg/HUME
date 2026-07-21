use std::path::Path;

use super::{LanguageRegistry, detect_language};
use hume_engine::theme::ScopeRegistry;
use hume_test_fixtures::{grammar_parser_path, grammar_query_path, skip_unless_grammars};

/// Write `src` to a temp file and return its path (kept alive via the
/// returned `TempDir`).
fn write_temp_scm(src: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("injections.scm");
    std::fs::write(&path, src).unwrap();
    (dir, path)
}

#[test]
fn attach_grammar_with_valid_injections_populates_bundle() {
    if skip_unless_grammars(&["rust"]) {
        return;
    }
    let parser_path = grammar_parser_path("rust");
    let hl_path = grammar_query_path("rust");
    let (_dir, inj_path) =
        write_temp_scm(r#"((_) @injection.content (#set! injection.language "markdown"))"#);

    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let bundle = reg
        .attach_grammar(
            "rust",
            &parser_path,
            "tree_sitter_rust",
            &hl_path,
            Some(&inj_path),
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
    if skip_unless_grammars(&["rust"]) {
        return;
    }
    let parser_path = grammar_parser_path("rust");
    let hl_path = grammar_query_path("rust");

    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let bundle = reg
        .attach_grammar(
            "rust",
            &parser_path,
            "tree_sitter_rust",
            &hl_path,
            None,
            &mut scope_reg,
        )
        .expect("attach without injections must succeed");

    assert!(
        bundle.injections.is_none(),
        "no injections_path given → injections must stay None"
    );
}

/// A broken injections.scm hard-fails the whole attach — same as a broken
/// highlights.scm — rather than degrading to a warning. Both files come
/// from the same trusted pinned source, so there is no separate soft-fail
/// path.
#[test]
fn attach_grammar_with_broken_injections_fails_whole_attach() {
    if skip_unless_grammars(&["rust"]) {
        return;
    }
    let parser_path = grammar_parser_path("rust");
    let hl_path = grammar_query_path("rust");
    let (_dir, inj_path) = write_temp_scm("(this is not valid tree-sitter query syntax");

    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let result = reg.attach_grammar(
        "rust",
        &parser_path,
        "tree_sitter_rust",
        &hl_path,
        Some(&inj_path),
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
    // Flip: unknown ext should not match.
    assert!(reg.by_extension("yaml").is_none());
}

#[test]
fn register_identity_with_invalid_glob_errors() {
    let mut reg = LanguageRegistry::new();
    let err = reg
        .register_identity("broken", &[], &["["], &[])
        .expect_err("unterminated character class must fail glob compilation");
    assert!(
        matches!(err, super::RegisterError::GlobBuild(_)),
        "expected GlobBuild error, got: {err:?}"
    );
}

#[test]
fn register_identity_with_globs_lookup() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("makefile", &[], &["Makefile", "GNUmakefile"], &[])
        .unwrap();
    let matches = reg.compiled_globs().matches(Path::new("Makefile"));
    assert!(!matches.is_empty(), "Makefile should match registered glob");
    let name = reg.glob_lang_id(matches[0]).map(|id| reg.name_of(id));
    assert_eq!(name, Some("makefile"));
    // Flip: non-matching path must produce empty match.
    assert!(
        reg.compiled_globs()
            .matches(Path::new("Cargo.toml"))
            .is_empty()
    );
}

#[test]
fn remove_clears_glob_and_shebang_entries() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("python", &["py"], &["*.py"], &["python"])
        .unwrap();
    assert!(reg.by_extension("py").is_some());
    assert!(reg.by_shebang("python").is_some());
    assert!(!reg.compiled_globs().matches(Path::new("foo.py")).is_empty());

    reg.remove("python");

    assert!(reg.by_extension("py").is_none());
    assert!(reg.by_shebang("python").is_none());
    // Flip expectation: after remove, matches must be empty.
    assert!(reg.compiled_globs().matches(Path::new("foo.py")).is_empty());
}

/// Regression: `deindex` must only remove an index entry it still owns.
/// `c` and `cpp` both claim `.h` (last-registered wins, so `cpp` takes
/// it); re-registering `c` without `.h` must not evict `cpp`'s mapping —
/// `c` never owned it at the time of re-registration.
///
/// Flip: an unconditional `by_ext.remove(ext)` in `deindex` (no ownership
/// check) makes this test fail — `cpp` loses `.h` even though it's the
/// current owner.
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

#[test]
fn detect_language_by_extension() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("rust", &["rs"], &[], &[]).unwrap();
    let id = detect_language(Some(Path::new("foo.rs")), None, &reg);
    assert_eq!(id, reg.id_of("rust"));
    // Flip: wrong extension must not detect.
    let no_match = detect_language(Some(Path::new("foo.py")), None, &reg);
    assert!(no_match.is_none());
}

#[test]
fn detect_language_by_glob() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("makefile", &[], &["Makefile", "GNUmakefile"], &[])
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
    reg.register_identity("tsconfig", &[], &["tsconfig.json", "*.config.json"], &[])
        .unwrap();
    reg.register_identity("json", &["json"], &[], &[]).unwrap();
    let id = detect_language(Some(Path::new("tsconfig.json")), None, &reg);
    assert_eq!(id, reg.id_of("tsconfig"));
    // Flip: without the glob match, a plain .json should detect as json.
    let plain = detect_language(Some(Path::new("other.json")), None, &reg);
    assert_eq!(plain, reg.id_of("json"));
}

#[test]
fn detect_language_glob_tiebreak_last_registered_wins() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("generic-json", &[], &["*.json"], &[])
        .unwrap();
    reg.register_identity("strict-json", &[], &["*.json"], &[])
        .unwrap();
    assert_eq!(
        detect_language(Some(Path::new("config.json")), None, &reg),
        reg.id_of("strict-json"),
    );

    let mut reg2 = LanguageRegistry::new();
    reg2.register_identity("strict-json", &[], &["*.json"], &[])
        .unwrap();
    reg2.register_identity("generic-json", &[], &["*.json"], &[])
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
    // Flip: wrong shebang must not match.
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
/// distinct languages — `foo.c` detects as `c`, `foo.C` detects as `cpp`.
///
/// Flip: if extensions were folded to lowercase both would map to the
/// later-registered language (cpp wins, .c misdetects as cpp).
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
