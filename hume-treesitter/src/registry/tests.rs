use std::path::Path;

use super::{
    ConfigOrigin, DetectPaths, FileExtension, GrammarSource, LanguageField, LanguageFields,
    LanguageId, LanguageRegistry, OptionalQuery, QueryFiles, ShebangInterpreter, WriteOutcome,
    detect_language,
};
use hume_editing::comment::{BlockCommentTokens, LineCommentTokens};
use hume_engine::theme::ScopeRegistry;
use hume_platform::path::AbsolutePath;
use ropey::Rope;
use test_fixtures::{
    grammar_parser_path, grammar_query_path, require_grammars, require_helix_query,
};

/// Write `src` to `<tmp>/<filename>` and return its path (kept alive via the
/// returned `TempDir`).
fn write_temp_scm(filename: &str, src: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(filename);
    std::fs::write(&path, src).unwrap();
    (dir, path)
}

/// The language `reg` detects for a file opened at `as_opened` that resolves
/// to `resolved`, both root-anchored literals, with `first_line` as its first
/// line.
fn detect_linked(
    as_opened: &str,
    resolved: &str,
    first_line: Option<&str>,
    reg: &LanguageRegistry,
) -> Option<LanguageId> {
    let as_opened = AbsolutePath::rooted(as_opened);
    let resolved = AbsolutePath::rooted(resolved);
    let paths = DetectPaths {
        as_opened: &as_opened,
        resolved: &resolved,
    };
    detect_language(
        Some(paths),
        &Rope::from_str(first_line.unwrap_or_default()),
        reg,
    )
}

/// The language `reg` detects for a file opened at `path`, a root-anchored
/// literal that is also its resolved path, with `first_line` as its first line.
fn detect_at(path: &str, first_line: Option<&str>, reg: &LanguageRegistry) -> Option<LanguageId> {
    detect_linked(path, path, first_line, reg)
}

/// The rust fixture grammar with `queries`.
fn rust_source(queries: QueryFiles) -> GrammarSource {
    GrammarSource {
        path: grammar_parser_path("rust"),
        symbol: "tree_sitter_rust".to_owned(),
        queries,
    }
}

#[test]
fn attach_grammar_with_valid_injections_populates_bundle() {
    require_grammars(&["rust"]);
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
            &rust_source(
                QueryFiles::new(hl_path.clone()).with(OptionalQuery::Injections, inj_path.clone()),
            ),
            ConfigOrigin::User,
            &mut scope_reg,
        )
        .expect("attach with valid injections must succeed")
        .expect("no higher origin holds the grammar");

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
fn attach_grammar_without_injections_file_leaves_injections_none() {
    require_grammars(&["rust"]);
    let hl_path = grammar_query_path("rust");

    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let bundle = reg
        .attach_grammar(
            "rust",
            &rust_source(QueryFiles::new(hl_path.clone())),
            ConfigOrigin::User,
            &mut scope_reg,
        )
        .expect("attach without injections must succeed")
        .expect("no higher origin holds the grammar");

    assert!(
        bundle.injections.is_none(),
        "no injections file given → injections must stay None"
    );
}

/// A broken injections.scm hard-fails the whole attach (same as a broken
/// highlights.scm) rather than degrading to a warning. Both files come
/// from the same trusted pinned source, so there is no separate soft-fail
/// path.
#[test]
fn attach_grammar_with_broken_injections_fails_whole_attach() {
    require_grammars(&["rust"]);
    let hl_path = grammar_query_path("rust");
    let (_dir, inj_path) = write_temp_scm(
        "injections.scm",
        "(this is not valid tree-sitter query syntax",
    );

    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let scopes_before = scope_reg.len();
    let result = reg.attach_grammar(
        "rust",
        &rust_source(
            QueryFiles::new(hl_path.clone()).with(OptionalQuery::Injections, inj_path.clone()),
        ),
        ConfigOrigin::User,
        &mut scope_reg,
    );
    let Err(err) = result else {
        panic!("broken injections.scm must fail the attach");
    };
    assert!(
        matches!(
            err,
            super::RegisterError::OptionalQueryBuild {
                kind: OptionalQuery::Injections,
                ..
            }
        ),
        "broken injections.scm must surface as OptionalQueryBuild for Injections, got: {err:?}"
    );
    assert!(
        reg.defined_id("rust").is_none(),
        "a failed attach must not leave a partial identity behind"
    );
    assert_eq!(
        scope_reg.len(),
        scopes_before,
        "a failed attach must not intern highlight scopes"
    );
}

#[test]
fn attach_grammar_with_valid_textobjects_populates_bundle() {
    require_grammars(&["rust"]);
    let hl_path = grammar_query_path("rust");
    let to_path = require_helix_query("rust", OptionalQuery::TextObjects.file_name());

    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let bundle = reg
        .attach_grammar(
            "rust",
            &rust_source(
                QueryFiles::new(hl_path.clone()).with(OptionalQuery::TextObjects, to_path.clone()),
            ),
            ConfigOrigin::User,
            &mut scope_reg,
        )
        .expect("attach with valid textobjects must succeed")
        .expect("no higher origin holds the grammar");

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
fn attach_grammar_without_textobjects_file_leaves_textobjects_none() {
    require_grammars(&["rust"]);
    let hl_path = grammar_query_path("rust");

    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let bundle = reg
        .attach_grammar(
            "rust",
            &rust_source(QueryFiles::new(hl_path.clone())),
            ConfigOrigin::User,
            &mut scope_reg,
        )
        .expect("attach without textobjects must succeed")
        .expect("no higher origin holds the grammar");

    assert!(
        bundle.textobjects.is_none(),
        "no textobjects file given → textobjects must stay None"
    );
}

/// A broken textobjects.scm hard-fails the whole attach, same policy as a
/// broken injections.scm (see `attach_grammar_with_broken_injections_fails_whole_attach`).
#[test]
fn attach_grammar_with_broken_textobjects_fails_whole_attach() {
    require_grammars(&["rust"]);
    let hl_path = grammar_query_path("rust");
    let (_dir, to_path) = write_temp_scm(
        "textobjects.scm",
        "(this is not valid tree-sitter query syntax",
    );

    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let result = reg.attach_grammar(
        "rust",
        &rust_source(
            QueryFiles::new(hl_path.clone()).with(OptionalQuery::TextObjects, to_path.clone()),
        ),
        ConfigOrigin::User,
        &mut scope_reg,
    );
    let Err(err) = result else {
        panic!("broken textobjects.scm must fail the attach");
    };
    assert!(
        matches!(
            err,
            super::RegisterError::OptionalQueryBuild {
                kind: OptionalQuery::TextObjects,
                ..
            }
        ),
        "broken textobjects.scm must surface as OptionalQueryBuild for TextObjects, got: {err:?}"
    );
    assert!(
        reg.defined_id("rust").is_none(),
        "a failed attach must not leave a partial identity behind"
    );
}

#[test]
fn query_files_get_returns_what_with_stored_per_kind() {
    let files = QueryFiles::new("hl.scm".into());
    assert_eq!(files.highlights(), Path::new("hl.scm"));
    for kind in OptionalQuery::ALL {
        assert_eq!(files.get(kind), None, "{kind:?} is unset on a fresh set");
    }

    for kind in OptionalQuery::ALL {
        let path = format!("{}-path", kind.key());
        let files = QueryFiles::new("hl.scm".into()).with(kind, path.clone().into());
        assert_eq!(files.get(kind), Some(Path::new(&path)));
        for other in OptionalQuery::ALL.into_iter().filter(|&k| k != kind) {
            assert_eq!(files.get(other), None, "{other:?} stays unset");
        }
    }
}

#[test]
fn attach_grammar_with_missing_injections_file_fails_with_read_error() {
    require_grammars(&["rust"]);
    let hl_path = grammar_query_path("rust");
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("injections.scm");

    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let result = reg.attach_grammar(
        "rust",
        &rust_source(QueryFiles::new(hl_path).with(OptionalQuery::Injections, missing)),
        ConfigOrigin::User,
        &mut scope_reg,
    );
    let Err(err) = result else {
        panic!("a missing injections.scm must fail the attach");
    };
    assert!(
        matches!(
            err,
            super::RegisterError::OptionalQueryRead {
                kind: OptionalQuery::Injections,
                ..
            }
        ),
        "got: {err:?}"
    );
}

#[test]
fn language_registry_by_ext_lookup_empty() {
    let reg = LanguageRegistry::new();
    assert!(reg.by_file_name("f.rs").is_none());
}

#[test]
fn language_registry_remove_is_idempotent() {
    let mut reg = LanguageRegistry::new();
    assert!(!reg.remove("rust"));
}

#[test]
fn register_identity_then_defined_id_returns_its_id() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("toml", &["toml"], &[], &[]).unwrap();
    let id = reg
        .defined_id("toml")
        .expect("identity should be registered");
    assert_eq!(reg.name_of(id), "toml");
    assert!(
        reg.by_file_name("f.toml").is_some(),
        "extension lookup must work after identity reg"
    );
    // Unknown ext should not match.
    assert!(reg.by_file_name("f.yaml").is_none());
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn roots_of_returns_the_configured_markers() {
    let mut reg = LanguageRegistry::new();
    let id = reg.register_identity("go", &["go"], &[], &[]).unwrap();
    assert_eq!(
        set_roots(&mut reg, id, &["go.work", "go.mod"], ConfigOrigin::User),
        WriteOutcome::Applied
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
    let id = reg
        .register_identity_no_rebuild("go", &[], &[], &[], ConfigOrigin::Bundled)
        .0;
    assert_eq!(
        set_roots(&mut reg, id, &["go.mod"], ConfigOrigin::User),
        WriteOutcome::Applied
    );
    let (_, outcome) = reg
        .register_identity_by("go", &["go"], &[], &[], ConfigOrigin::User)
        .expect("the test detection data is valid");
    assert_eq!(outcome, WriteOutcome::Applied);
    assert_eq!(reg.roots_of(id), ["go.mod"]);
}

fn tsx() -> (LanguageRegistry, LanguageId) {
    let mut reg = LanguageRegistry::new();
    let id = reg
        .register_identity_no_rebuild("tsx", &[], &[], &[], ConfigOrigin::Bundled)
        .0;
    (reg, id)
}

fn set_roots(
    reg: &mut LanguageRegistry,
    id: LanguageId,
    roots: &[&str],
    by: ConfigOrigin,
) -> WriteOutcome {
    let fields = LanguageFields {
        roots: Some(strings(roots)),
        ..LanguageFields::default()
    };
    configure_one(reg, id, fields, by)
}

fn set_id(
    reg: &mut LanguageRegistry,
    id: LanguageId,
    lsp_language_id: &str,
    by: ConfigOrigin,
) -> WriteOutcome {
    let fields = LanguageFields {
        lsp_language_id: Some(lsp_language_id.to_string()),
        ..LanguageFields::default()
    };
    configure_one(reg, id, fields, by)
}

/// The outcome of a write that gives one field.
fn configure_one(
    reg: &mut LanguageRegistry,
    id: LanguageId,
    fields: LanguageFields,
    by: ConfigOrigin,
) -> WriteOutcome {
    match reg.configure_fields(id, fields, by).as_slice() {
        [(_, outcome)] => *outcome,
        outcomes => panic!("one field given, outcomes {outcomes:?}"),
    }
}

#[test]
fn configure_fields_reports_each_given_field_in_order() {
    let (mut reg, id) = tsx();
    let all = || LanguageFields {
        roots: Some(strings(&["package.json"])),
        lsp_language_id: Some("typescriptreact".to_string()),
        comment_tokens: Some(LineCommentTokens::from_strs(&["//"])),
        block_comment_tokens: Some(BlockCommentTokens::from_pairs(&[("/*", "*/")])),
    };
    let tagged = |outcome| LanguageField::ALL.map(|field| (field, outcome)).to_vec();

    assert_eq!(
        reg.configure_fields(id, all(), ConfigOrigin::User),
        tagged(WriteOutcome::Applied)
    );
    assert_eq!(
        reg.configure_fields(id, all(), ConfigOrigin::User),
        tagged(WriteOutcome::Ignored)
    );
    assert_eq!(
        reg.configure_fields(id, LanguageFields::default(), ConfigOrigin::User),
        []
    );
}

fn fill_tsx_bundled(reg: &mut LanguageRegistry, id: LanguageId) {
    reg.fill_bundled(
        id,
        LanguageFields {
            roots: Some(strings(&["package.json"])),
            lsp_language_id: Some("typescriptreact".to_string()),
            ..LanguageFields::default()
        },
    );
}

#[test]
fn bundled_values_fill_a_language_with_none_configured() {
    let (mut reg, id) = tsx();
    assert!(!reg.bundled_loaded(id));

    fill_tsx_bundled(&mut reg, id);

    assert!(reg.bundled_loaded(id));
    assert_eq!(reg.roots_of(id), ["package.json"]);
    assert_eq!(reg.lsp_language_id_of(id), "typescriptreact");
}

#[test]
fn bundled_values_never_replace_configured_ones() {
    let (mut reg, id) = tsx();
    let user = ConfigOrigin::User;
    assert_eq!(
        set_roots(&mut reg, id, &["deno.json"], user),
        WriteOutcome::Applied
    );
    assert_eq!(
        set_id(&mut reg, id, "tsx-custom", user),
        WriteOutcome::Applied
    );

    fill_tsx_bundled(&mut reg, id);

    assert_eq!(reg.roots_of(id), ["deno.json"]);
    assert_eq!(reg.lsp_language_id_of(id), "tsx-custom");
}

#[test]
fn bundled_values_fill_only_the_fields_left_unset() {
    let (mut reg, id) = tsx();
    assert_eq!(
        set_roots(&mut reg, id, &["deno.json"], ConfigOrigin::User),
        WriteOutcome::Applied
    );

    fill_tsx_bundled(&mut reg, id);

    assert_eq!(reg.roots_of(id), ["deno.json"]);
    assert_eq!(reg.lsp_language_id_of(id), "typescriptreact");
}

#[test]
fn a_configured_value_replaces_a_bundled_one() {
    let (mut reg, id) = tsx();
    let user = ConfigOrigin::User;
    fill_tsx_bundled(&mut reg, id);

    assert_eq!(
        set_roots(&mut reg, id, &["deno.json"], user),
        WriteOutcome::Applied
    );
    assert_eq!(
        set_id(&mut reg, id, "tsx-custom", user),
        WriteOutcome::Applied
    );

    assert_eq!(reg.roots_of(id), ["deno.json"]);
    assert_eq!(reg.lsp_language_id_of(id), "tsx-custom");
}

#[test]
fn a_second_configured_value_is_ignored() {
    let (mut reg, id) = tsx();
    let user = ConfigOrigin::User;
    assert_eq!(
        set_roots(&mut reg, id, &["deno.json"], user),
        WriteOutcome::Applied
    );
    assert_eq!(set_id(&mut reg, id, "first", user), WriteOutcome::Applied);

    assert_eq!(
        set_roots(&mut reg, id, &["package.json"], user),
        WriteOutcome::Ignored
    );
    assert_eq!(set_id(&mut reg, id, "second", user), WriteOutcome::Ignored);

    assert_eq!(reg.roots_of(id), ["deno.json"]);
    assert_eq!(reg.lsp_language_id_of(id), "first");
}

#[test]
fn a_second_plugin_value_is_ignored() {
    let (mut reg, id) = tsx();
    let plugin = ConfigOrigin::Plugin;
    assert_eq!(
        set_roots(&mut reg, id, &["deno.json"], plugin),
        WriteOutcome::Applied
    );

    assert_eq!(
        set_roots(&mut reg, id, &["package.json"], plugin),
        WriteOutcome::Ignored
    );

    assert_eq!(reg.roots_of(id), ["deno.json"]);
}

#[test]
fn a_user_value_replaces_a_plugin_value() {
    let (mut reg, id) = tsx();
    let (user, plugin) = (ConfigOrigin::User, ConfigOrigin::Plugin);
    assert_eq!(
        set_roots(&mut reg, id, &["deno.json"], plugin),
        WriteOutcome::Applied
    );
    assert_eq!(
        set_id(&mut reg, id, "plugin", plugin),
        WriteOutcome::Applied
    );

    assert_eq!(
        set_roots(&mut reg, id, &["package.json"], user),
        WriteOutcome::Applied
    );
    assert_eq!(set_id(&mut reg, id, "user", user), WriteOutcome::Applied);

    assert_eq!(reg.roots_of(id), ["package.json"]);
    assert_eq!(reg.lsp_language_id_of(id), "user");
}

#[test]
fn a_plugin_value_after_a_user_value_is_shadowed() {
    let (mut reg, id) = tsx();
    let (user, plugin) = (ConfigOrigin::User, ConfigOrigin::Plugin);
    assert_eq!(
        set_roots(&mut reg, id, &["package.json"], user),
        WriteOutcome::Applied
    );
    assert_eq!(set_id(&mut reg, id, "user", user), WriteOutcome::Applied);

    assert_eq!(
        set_roots(&mut reg, id, &["deno.json"], plugin),
        WriteOutcome::Shadowed
    );
    assert_eq!(
        set_id(&mut reg, id, "plugin", plugin),
        WriteOutcome::Shadowed
    );

    assert_eq!(reg.roots_of(id), ["package.json"]);
    assert_eq!(reg.lsp_language_id_of(id), "user");
}

#[test]
fn an_empty_bundled_fill_still_marks_the_language_loaded() {
    let mut reg = LanguageRegistry::new();
    let id = reg
        .register_identity_no_rebuild("plain", &[], &[], &[], ConfigOrigin::Bundled)
        .0;

    reg.fill_bundled(id, LanguageFields::default());

    assert!(reg.bundled_loaded(id));
    assert!(reg.roots_of(id).is_empty());
    assert_eq!(reg.lsp_language_id_of(id), "plain");
}

fn set_line(
    reg: &mut LanguageRegistry,
    id: LanguageId,
    tokens: &[&str],
    by: ConfigOrigin,
) -> WriteOutcome {
    let fields = LanguageFields {
        comment_tokens: Some(LineCommentTokens::from_strs(tokens)),
        ..LanguageFields::default()
    };
    configure_one(reg, id, fields, by)
}

fn set_block(
    reg: &mut LanguageRegistry,
    id: LanguageId,
    pairs: &[(&str, &str)],
    by: ConfigOrigin,
) -> WriteOutcome {
    let fields = LanguageFields {
        block_comment_tokens: Some(BlockCommentTokens::from_pairs(pairs)),
        ..LanguageFields::default()
    };
    configure_one(reg, id, fields, by)
}

fn fill_comment_tokens(reg: &mut LanguageRegistry, id: LanguageId) {
    reg.fill_bundled(
        id,
        LanguageFields {
            comment_tokens: Some(LineCommentTokens::from_strs(&["//"])),
            block_comment_tokens: Some(BlockCommentTokens::from_pairs(&[("/*", "*/")])),
            ..LanguageFields::default()
        },
    );
}

#[test]
fn a_language_without_comment_tokens_has_none() {
    let (reg, id) = tsx();
    assert_eq!(reg.comment_tokens_of(id), None);
}

#[test]
fn bundled_comment_tokens_fill_both_kinds() {
    let (mut reg, id) = tsx();

    fill_comment_tokens(&mut reg, id);

    let tokens = reg.comment_tokens_of(id).expect("comment tokens");
    assert_eq!(tokens.line(), Some(&LineCommentTokens::from_strs(&["//"])));
    assert_eq!(
        tokens.block(),
        Some(&BlockCommentTokens::from_pairs(&[("/*", "*/")]))
    );
}

#[test]
fn configured_line_tokens_keep_the_bundled_block_tokens() {
    let (mut reg, id) = tsx();
    assert_eq!(
        set_line(&mut reg, id, &["#"], ConfigOrigin::User),
        WriteOutcome::Applied
    );

    fill_comment_tokens(&mut reg, id);

    let tokens = reg.comment_tokens_of(id).expect("comment tokens");
    assert_eq!(tokens.line(), Some(&LineCommentTokens::from_strs(&["#"])));
    assert_eq!(
        tokens.block(),
        Some(&BlockCommentTokens::from_pairs(&[("/*", "*/")]))
    );
}

#[test]
fn configured_comment_tokens_replace_bundled_ones() {
    let (mut reg, id) = tsx();
    let user = ConfigOrigin::User;
    fill_comment_tokens(&mut reg, id);

    assert_eq!(set_line(&mut reg, id, &["#"], user), WriteOutcome::Applied);
    assert_eq!(
        set_block(&mut reg, id, &[("<!--", "-->")], user),
        WriteOutcome::Applied
    );

    let tokens = reg.comment_tokens_of(id).expect("comment tokens");
    assert_eq!(tokens.line(), Some(&LineCommentTokens::from_strs(&["#"])));
    assert_eq!(
        tokens.block(),
        Some(&BlockCommentTokens::from_pairs(&[("<!--", "-->")]))
    );
}

#[test]
fn comment_tokens_follow_the_configurer_tiers() {
    let (mut reg, id) = tsx();
    let (user, plugin) = (ConfigOrigin::User, ConfigOrigin::Plugin);
    assert_eq!(
        set_block(&mut reg, id, &[("/*", "*/")], user),
        WriteOutcome::Applied
    );
    assert_eq!(
        set_block(&mut reg, id, &[("(*", "*)")], user),
        WriteOutcome::Ignored
    );
    assert_eq!(
        set_block(&mut reg, id, &[("{-", "-}")], plugin),
        WriteOutcome::Shadowed
    );
    assert_eq!(
        set_line(&mut reg, id, &["--"], plugin),
        WriteOutcome::Applied
    );
    assert_eq!(
        set_line(&mut reg, id, &[";"], plugin),
        WriteOutcome::Ignored
    );

    let tokens = reg.comment_tokens_of(id).expect("comment tokens");
    assert_eq!(tokens.line(), Some(&LineCommentTokens::from_strs(&["--"])));
    assert_eq!(
        tokens.block(),
        Some(&BlockCommentTokens::from_pairs(&[("/*", "*/")]))
    );
}

#[test]
fn an_attached_bundle_names_its_language() {
    require_grammars(&["rust"]);
    let hl_path = grammar_query_path("rust");
    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    reg.intern("markdown");

    let bundle = reg
        .attach_grammar(
            "rust",
            &rust_source(QueryFiles::new(hl_path.clone())),
            ConfigOrigin::User,
            &mut scope_reg,
        )
        .expect("attach rust")
        .expect("no higher origin holds the grammar");

    assert_eq!(Some(bundle.language), reg.id_of("rust"));
}

#[test]
fn register_identity_with_globs_lookup() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("makefile", &[], &["Makefile", "GNUmakefile"], &[])
        .unwrap();
    let matches = reg.compiled_globs().matches(Path::new("/x/Makefile"));
    assert!(!matches.is_empty(), "Makefile should match registered glob");
    let name = reg.glob_lang_id(matches[0]).map(|id| reg.name_of(id));
    assert_eq!(name, Some("makefile"));
    // A path the globs don't cover yields no match.
    assert!(
        reg.compiled_globs()
            .matches(Path::new("/x/Cargo.toml"))
            .is_empty()
    );
}

#[test]
fn remove_clears_glob_and_shebang_entries() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("python", &["py"], &["*.py"], &["python"])
        .unwrap();
    assert!(reg.by_file_name("f.py").is_some());
    assert!(reg.by_interpreter("python").is_some());
    assert!(
        !reg.compiled_globs()
            .matches(Path::new("/x/foo.py"))
            .is_empty()
    );

    reg.remove("python");

    assert!(reg.by_file_name("f.py").is_none());
    assert!(reg.by_interpreter("python").is_none());
    assert!(
        reg.compiled_globs()
            .matches(Path::new("/x/foo.py"))
            .is_empty()
    );
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
        reg.by_file_name("f.h"),
        reg.id_of("cpp"),
        "cpp must win the shared .h extension (last-registered-wins)"
    );

    // Re-register c, dropping .h from its own extension list.
    let (_, outcome) = reg
        .register_identity_by("c", &["c"], &[], &[], ConfigOrigin::User)
        .expect("the test detection data is valid");
    assert_eq!(outcome, WriteOutcome::Applied);

    assert_eq!(
        reg.by_file_name("f.h"),
        reg.id_of("cpp"),
        "re-registering c must not clobber cpp's still-current .h mapping"
    );
    assert_eq!(reg.by_file_name("f.c"), reg.id_of("c"));
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
    let hl_path = grammar_query_path("rust");

    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    reg.attach_grammar(
        "rust",
        &rust_source(QueryFiles::new(hl_path.clone())),
        ConfigOrigin::User,
        &mut scope_reg,
    )
    .expect("attach must succeed")
    .expect("no higher origin holds the grammar");
    assert!(reg.has_grammar("rust"), "sanity: grammar attached");

    // Re-register identity, as `define-language!` would from init.scm to add
    // an extra extension.
    let (_, outcome) = reg
        .register_identity_by("rust", &["rs", "mylang"], &[], &[], ConfigOrigin::User)
        .expect("the test detection data is valid");
    assert_eq!(outcome, WriteOutcome::Applied);

    assert!(
        reg.has_grammar("rust"),
        "identity re-registration must keep the attached grammar"
    );
    let id = reg.id_of("rust").expect("rust must still be interned");
    assert!(
        reg.loaded_grammar(id).is_some(),
        "loaded_grammar(id) must still resolve after re-registration"
    );
    assert!(
        matches!(
            reg.grammar_snapshot().get("rust"),
            Some(super::SnapshotGrammar::Loaded(_))
        ),
        "grammar_snapshot must still carry rust after re-registration"
    );

    // The identity replacement itself must still have taken effect.
    let detected = detect_at("/x/foo.mylang", None, &reg);
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
        set_id(&mut reg, id, "typescriptreact", ConfigOrigin::User),
        WriteOutcome::Applied
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
        set_id(&mut reg, id, "typescriptreact", ConfigOrigin::User),
        WriteOutcome::Applied
    );

    let (_, outcome) = reg
        .register_identity_by("tsx", &["tsx", "mytsx"], &[], &[], ConfigOrigin::User)
        .expect("the test detection data is valid");
    assert_eq!(outcome, WriteOutcome::Applied);

    assert_eq!(reg.lsp_language_id_of(id), "typescriptreact");
}

#[test]
fn detect_language_by_extension() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("rust", &["rs"], &[], &[]).unwrap();
    let id = detect_at("/x/foo.rs", None, &reg);
    assert_eq!(id, reg.id_of("rust"));
    // A different extension must not detect.
    let no_match = detect_at("/x/foo.py", None, &reg);
    assert!(no_match.is_none());
}

#[test]
fn detect_language_by_glob() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("makefile", &[], &["Makefile", "GNUmakefile"], &[])
        .unwrap();
    let id = detect_at("/project/Makefile", None, &reg);
    assert_eq!(id, reg.id_of("makefile"));
    let no_match = detect_at("/project/other", None, &reg);
    assert!(no_match.is_none());
}

#[test]
fn a_glob_with_a_directory_matches_the_end_of_the_path() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("yaml", &["yaml"], &[], &[]).unwrap();
    reg.register_identity("gha", &[], &[".github/workflows/*.yaml"], &[])
        .unwrap();
    assert_eq!(
        detect_at("/repo/.github/workflows/ci.yaml", None, &reg),
        reg.id_of("gha")
    );
    assert_eq!(detect_at("/repo/ci.yaml", None, &reg), reg.id_of("yaml"));
}

#[test]
fn a_glob_matches_whole_path_components_only() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("git-config", &[], &[".git/config"], &[])
        .unwrap();
    assert_eq!(
        detect_at("/repo/.git/config", None, &reg),
        reg.id_of("git-config")
    );
    assert_eq!(detect_at("/repo/x.git/config", None, &reg), None);
}

/// A file opened through a symlink is detected by the path it was opened
/// as, then by the path the link resolves to.
#[test]
fn a_symlinked_file_is_detected_by_either_path() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("git-config", &[], &[".config/git/config"], &[])
        .unwrap();
    reg.register_identity("toml", &["toml"], &[], &[]).unwrap();
    assert_eq!(
        detect_linked(
            "/h/.config/git/config",
            "/h/dotfiles/git/config",
            None,
            &reg
        ),
        reg.id_of("git-config")
    );
    assert_eq!(
        detect_linked("/h/current", "/h/release.toml", None, &reg),
        reg.id_of("toml")
    );
}

#[test]
fn a_dotted_extension_matches_before_the_last_one() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("map", &["map"], &[], &[]).unwrap();
    reg.register_identity("json", &["json", "js.map"], &[], &[])
        .unwrap();
    assert_eq!(detect_at("/x/app.js.map", None, &reg), reg.id_of("json"));
    assert_eq!(detect_at("/x/app.css.map", None, &reg), reg.id_of("map"));
}

#[test]
fn a_dotfiles_leading_dot_does_not_start_an_extension() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("bash", &["bashrc"], &[], &[])
        .unwrap();
    assert_eq!(detect_at("/home/u/.bashrc", None, &reg), None);
    assert_eq!(detect_at("/home/u/x.bashrc", None, &reg), reg.id_of("bash"));
}

#[test]
fn detect_language_glob_beats_extension() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("typescript", &["ts"], &[], &[])
        .unwrap();
    reg.register_identity("tsconfig", &[], &["tsconfig.json", "*.config.json"], &[])
        .unwrap();
    reg.register_identity("json", &["json"], &[], &[]).unwrap();
    let id = detect_at("/x/tsconfig.json", None, &reg);
    assert_eq!(id, reg.id_of("tsconfig"));
    // A plain .json that no glob covers still detects as json.
    let plain = detect_at("/x/other.json", None, &reg);
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
        detect_at("/x/config.json", None, &reg),
        reg.id_of("strict-json"),
    );

    let mut reg2 = LanguageRegistry::new();
    reg2.register_identity("strict-json", &[], &["*.json"], &[])
        .unwrap();
    reg2.register_identity("generic-json", &[], &["*.json"], &[])
        .unwrap();
    assert_eq!(
        detect_at("/x/config.json", None, &reg2),
        reg2.id_of("generic-json"),
    );
}

#[test]
fn detect_language_by_shebang() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("python", &["py"], &[], &["python"])
        .unwrap();
    let id = detect_at("/x/script", Some("#!/usr/bin/env python3"), &reg);
    assert_eq!(id, reg.id_of("python"));
    // Some other shebang must not match.
    let no_match = detect_at("/x/script", Some("#!/bin/bash"), &reg);
    assert!(no_match.is_none());
}

#[test]
fn detect_language_shebang_direct_path() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("bash", &["sh"], &[], &["bash"])
        .unwrap();
    // Extension wins over shebang.
    let id = detect_at("/x/run.sh", Some("#!/bin/bash"), &reg);
    assert_eq!(id, reg.id_of("bash"));
    // Without extension, shebang is used.
    let id2 = detect_at("/x/run", Some("#!/bin/bash"), &reg);
    assert_eq!(id2, reg.id_of("bash"));
}

/// Only the first line is read for a shebang, and a long interpreter path
/// stays whole.
#[test]
fn the_shebang_is_read_from_the_whole_first_line() {
    let reg = interpreters();
    let shebang = "#!/nix/store/0123456789abcdfghijklmnpqrsvwxyz-python3-3.12.4/bin/python3";
    let paths = DetectPaths {
        as_opened: &AbsolutePath::rooted("/x/run"),
        resolved: &AbsolutePath::rooted("/x/run"),
    };
    let text = Rope::from_str(&format!("{shebang}\n#!/usr/bin/lua\n"));
    assert_eq!(
        detect_language(Some(paths), &text, &reg),
        reg.id_of("python")
    );
}

/// `python` and `lua` registered, as the bundled data names them.
fn interpreters() -> LanguageRegistry {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("python", &[], &[], &["python"])
        .unwrap();
    reg.register_identity("lua", &[], &[], &["lua"]).unwrap();
    reg
}

#[test]
fn a_version_after_the_interpreter_name_is_ignored() {
    let reg = interpreters();
    for line in [
        "#!/usr/bin/python3.12",
        "#!/usr/bin/env python3",
        "#! /usr/bin/python3",
    ] {
        assert_eq!(
            detect_at("/x/run", Some(line), &reg),
            reg.id_of("python"),
            "{line}"
        );
    }
    assert_eq!(
        detect_at("/x/run", Some("#!/usr/bin/lua5.4"), &reg),
        reg.id_of("lua")
    );
}

#[test]
fn env_options_and_assignments_before_the_command_are_skipped() {
    let reg = interpreters();
    for line in [
        "#!/usr/bin/env -S python3 -u",
        "#!/usr/bin/env PYTHONDONTWRITEBYTECODE=1 python3",
        "#!/usr/bin/env -u PYTHONPATH python3",
        "#!/usr/bin/env --chdir /tmp python3",
        "#!/usr/bin/env -i -- python3",
        "#!/usr/bin/env /usr/local/bin/python3",
        "#!/usr/bin/env -L root python3",
        "#!/usr/bin/env -U root/daemon python3",
    ] {
        assert_eq!(
            detect_at("/x/run", Some(line), &reg),
            reg.id_of("python"),
            "{line}"
        );
    }
}

#[test]
fn an_interpreter_named_with_its_version_beats_the_unversioned_name() {
    let mut reg = interpreters();
    reg.register_identity("python2", &[], &[], &["python2"])
        .unwrap();
    assert_eq!(
        detect_at("/x/run", Some("#!/usr/bin/python2"), &reg),
        reg.id_of("python2")
    );
    assert_eq!(
        detect_at("/x/run", Some("#!/usr/bin/python3"), &reg),
        reg.id_of("python")
    );
}

#[test]
fn a_minor_version_falls_back_to_a_major_versioned_name_first() {
    let mut reg = interpreters();
    reg.register_identity("python3", &[], &[], &["python3"])
        .unwrap();
    reg.register_identity("perl5", &[], &[], &["perl5"])
        .unwrap();
    for line in ["#!/usr/bin/python3.12", "#!/usr/bin/env python3.12"] {
        assert_eq!(
            detect_at("/x/run", Some(line), &reg),
            reg.id_of("python3"),
            "{line}"
        );
    }
    assert_eq!(
        detect_at("/x/run", Some("#!/usr/bin/perl5.36.0"), &reg),
        reg.id_of("perl5")
    );
    assert_eq!(
        detect_at("/x/run", Some("#!/usr/bin/lua5.4"), &reg),
        reg.id_of("lua")
    );
}

#[test]
fn a_backslash_separated_interpreter_path_detects_by_its_last_name() {
    let reg = interpreters();
    assert_eq!(
        detect_at("/x/run", Some(r"#!C:\Python312\python"), &reg),
        reg.id_of("python")
    );
}

#[test]
fn a_bare_version_is_no_interpreter_name() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("python", &[], &[], &["python"])
        .unwrap();
    assert_eq!(detect_at("/x/run", Some("#!/usr/bin/3.12"), &reg), None);
}

#[test]
fn a_shebang_interpreter_is_one_name_with_no_space_or_separator() {
    for invalid in ["", "v run", "bin/python", r"bin\python", " python"] {
        assert!(
            ShebangInterpreter::new(invalid.to_owned()).is_err(),
            "{invalid:?}"
        );
    }
    for valid in ["python", "python3.12", "octave-cli", "R"] {
        assert_eq!(
            ShebangInterpreter::new(valid.to_owned()).map(|i| i.as_str().to_owned()),
            Ok(valid.to_owned())
        );
    }
}

#[test]
fn detect_language_no_match() {
    let reg = LanguageRegistry::new();
    assert!(detect_at("/x/foo.xyz", None, &reg).is_none());
    assert!(detect_language(None, &Rope::new(), &reg).is_none());
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
    assert_eq!(detect_at("/x/foo.c", None, &reg), reg.id_of("c"),);
    assert_eq!(detect_at("/x/foo.C", None, &reg), reg.id_of("cpp"),);
    // Sanity: unrelated extension is still None.
    assert!(detect_at("/x/foo.rs", None, &reg).is_none());
}

#[test]
fn register_grammar_opens_nothing() {
    let mut reg = LanguageRegistry::new();

    let id = reg
        .register_grammar(
            "missing",
            GrammarSource::missing("missing"),
            ConfigOrigin::User,
        )
        .expect("an empty slot takes a grammar");

    assert!(reg.has_grammar("missing"));
    assert!(reg.grammar_available(id));
    assert!(reg.loaded_grammar(id).is_none());
    assert_eq!(reg.defined_id("missing"), Some(id));
    assert_eq!(reg.take_bundled_pending(), vec![id]);
}

#[test]
fn load_grammar_compiles_once() {
    require_grammars(&["rust"]);
    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let id = reg
        .register_grammar(
            "rust",
            rust_source(QueryFiles::new(grammar_query_path("rust"))),
            ConfigOrigin::User,
        )
        .expect("an empty slot takes a grammar");

    let first = reg.load_grammar(id, &mut scope_reg).expect("rust loads");
    let second = reg
        .load_grammar(id, &mut scope_reg)
        .expect("a loaded grammar loads again");

    assert!(std::sync::Arc::ptr_eq(&first, &second));
    assert!(std::sync::Arc::ptr_eq(
        &first,
        reg.loaded_grammar(id).expect("rust is loaded")
    ));
}

#[test]
fn a_failed_load_is_queued_once_then_is_unavailable() {
    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let id = reg
        .register_grammar(
            "missing",
            GrammarSource::missing("missing"),
            ConfigOrigin::User,
        )
        .expect("an empty slot takes a grammar");

    assert!(reg.load_grammar(id, &mut scope_reg).is_none());
    assert!(reg.load_grammar(id, &mut scope_reg).is_none());

    let failed = reg.take_failed_loads();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].0, id);
    assert!(matches!(failed[0].1, super::RegisterError::GrammarLoad(_)));
    assert!(reg.take_failed_loads().is_empty(), "taken once");
    assert!(!reg.has_grammar("missing"));
    assert!(!reg.grammar_available(id));
}

#[test]
fn re_registering_a_failed_grammar_makes_it_available_again() {
    require_grammars(&["rust"]);
    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let id = reg
        .register_grammar("rust", GrammarSource::missing("rust"), ConfigOrigin::User)
        .expect("an empty slot takes a grammar");
    assert!(reg.load_grammar(id, &mut scope_reg).is_none());

    reg.register_grammar(
        "rust",
        rust_source(QueryFiles::new(grammar_query_path("rust"))),
        ConfigOrigin::User,
    );

    assert!(reg.has_grammar("rust"));
    let bundle = reg.load_grammar(id, &mut scope_reg).expect("rust loads");
    assert_eq!(bundle.language, id);
}

#[test]
fn a_failed_attach_keeps_the_prior_bundle() {
    require_grammars(&["rust"]);
    let mut reg = LanguageRegistry::new();
    let mut scope_reg = ScopeRegistry::new();
    let prior = reg
        .attach_grammar(
            "rust",
            &rust_source(QueryFiles::new(grammar_query_path("rust"))),
            ConfigOrigin::User,
            &mut scope_reg,
        )
        .expect("rust attaches")
        .expect("no higher origin holds the grammar");

    let result = reg.attach_grammar(
        "rust",
        &GrammarSource::missing("rust"),
        ConfigOrigin::User,
        &mut scope_reg,
    );

    assert!(result.is_err());
    let id = reg.id_of("rust").expect("rust is interned");
    assert!(std::sync::Arc::ptr_eq(
        &prior,
        reg.loaded_grammar(id).expect("the prior bundle stays")
    ));
}

#[test]
fn config_gen_is_unique_across_registries() {
    require_grammars(&["rust"]);
    let attach = || {
        LanguageRegistry::new()
            .attach_grammar(
                "rust",
                &rust_source(QueryFiles::new(grammar_query_path("rust"))),
                ConfigOrigin::User,
                &mut ScopeRegistry::new(),
            )
            .expect("rust attaches")
            .expect("no higher origin holds the grammar")
    };

    let first = attach();
    let second = attach();

    assert_ne!(first.config_gen, second.config_gen);
}

#[cfg(unix)]
mod unix;

// ── Origin ranking ───────────────────────────────────────────────────────

// ── Grammar ranking ──────────────────────────────────────────────────────

/// The rust fixture grammar with its highlights query.
fn rust_grammar() -> GrammarSource {
    rust_source(QueryFiles::new(grammar_query_path("rust")))
}

/// Whether `name`'s held grammar is the rust fixture: it loads, where a
/// [`GrammarSource::missing`] one fails.
fn holds_rust(reg: &mut LanguageRegistry, name: &str) -> bool {
    let id = reg.id_of(name).expect("the name is interned");
    reg.load_grammar(id, &mut ScopeRegistry::new()).is_some()
}

#[test]
fn a_plugins_grammar_does_not_replace_the_users() {
    require_grammars(&["rust"]);
    let mut reg = LanguageRegistry::new();
    reg.register_grammar("rust", rust_grammar(), ConfigOrigin::User);

    let shadowed =
        reg.register_grammar("rust", GrammarSource::missing("rust"), ConfigOrigin::Plugin);

    assert_eq!(shadowed, None);
    assert!(holds_rust(&mut reg, "rust"));
}

#[test]
fn a_users_grammar_replaces_a_plugins_and_a_plugins_replaces_a_bundled_one() {
    require_grammars(&["rust"]);
    let mut reg = LanguageRegistry::new();
    reg.register_grammar("rust", rust_grammar(), ConfigOrigin::Plugin);
    let replaced = reg.register_grammar("rust", GrammarSource::missing("rust"), ConfigOrigin::User);
    assert_eq!(replaced, reg.id_of("rust"));
    assert!(!holds_rust(&mut reg, "rust"));

    reg.register_grammar("go", rust_grammar(), ConfigOrigin::Bundled);
    reg.register_grammar("go", GrammarSource::missing("go"), ConfigOrigin::Plugin);
    assert!(!holds_rust(&mut reg, "go"));
}

#[test]
fn a_second_grammar_from_one_origin_replaces_the_first() {
    require_grammars(&["rust"]);
    let mut reg = LanguageRegistry::new();
    reg.register_grammar("rust", rust_grammar(), ConfigOrigin::User);

    let replaced = reg.register_grammar("rust", GrammarSource::missing("rust"), ConfigOrigin::User);

    assert_eq!(replaced, reg.id_of("rust"));
    assert!(!holds_rust(&mut reg, "rust"));
}

#[test]
fn a_loaded_grammar_keeps_the_origin_that_registered_it() {
    require_grammars(&["rust"]);
    let mut reg = LanguageRegistry::new();
    reg.register_grammar("rust", rust_grammar(), ConfigOrigin::User);
    assert!(holds_rust(&mut reg, "rust"));

    let shadowed =
        reg.register_grammar("rust", GrammarSource::missing("rust"), ConfigOrigin::Plugin);

    assert_eq!(shadowed, None);
    let id = reg.id_of("rust").expect("rust is interned");
    assert!(reg.loaded_grammar(id).is_some(), "the loaded bundle stays");
}

#[test]
fn a_shadowed_attach_compiles_nothing() {
    require_grammars(&["rust"]);
    let mut reg = LanguageRegistry::new();
    reg.register_grammar("rust", rust_grammar(), ConfigOrigin::User);

    let attached = reg
        .attach_grammar(
            "rust",
            &GrammarSource::missing("rust"),
            ConfigOrigin::Plugin,
            &mut ScopeRegistry::new(),
        )
        .expect("a shadowed attach opens no library, so it cannot fail");

    assert!(attached.is_none());
    assert!(holds_rust(&mut reg, "rust"));
}

// ── Identity ranking ─────────────────────────────────────────────────────

/// `name`'s detection data, as `define-language!` from `by` would set it,
/// with one extension.
fn define(
    reg: &mut LanguageRegistry,
    name: &str,
    extension: &str,
    by: ConfigOrigin,
) -> WriteOutcome {
    let extension = FileExtension::new(extension.to_owned()).unwrap();
    reg.register_identity_no_rebuild(name, &[extension], &[], &[], by)
        .1
}

#[test]
fn a_user_definition_replaces_the_bundled_one_and_a_plugins_does_not_replace_it() {
    let mut reg = LanguageRegistry::new();
    assert_eq!(
        define(&mut reg, "tsx", "tsx", ConfigOrigin::Bundled),
        WriteOutcome::Applied
    );
    assert_eq!(
        define(&mut reg, "tsx", "utsx", ConfigOrigin::User),
        WriteOutcome::Applied
    );
    assert_eq!(
        define(&mut reg, "tsx", "ptsx", ConfigOrigin::Plugin),
        WriteOutcome::Shadowed
    );
    let tsx = reg.id_of("tsx");
    assert_eq!(reg.by_file_name("a.utsx"), tsx);
    assert_eq!(reg.by_file_name("a.tsx"), None);
    assert_eq!(reg.by_file_name("a.ptsx"), None);
}

#[test]
fn a_second_definition_from_one_tier_is_ignored() {
    let mut reg = LanguageRegistry::new();
    assert_eq!(
        define(&mut reg, "tsx", "first", ConfigOrigin::Plugin),
        WriteOutcome::Applied
    );
    assert_eq!(
        define(&mut reg, "tsx", "second", ConfigOrigin::Plugin),
        WriteOutcome::Ignored
    );
    assert_eq!(reg.by_file_name("a.first"), reg.id_of("tsx"));
    assert_eq!(reg.by_file_name("a.second"), None);
}

/// A grammar defines the language it names with no detection data, which
/// a later definition from any tier then gives it.
#[test]
fn a_grammar_defines_its_language_and_leaves_its_detection_to_a_definition() {
    let mut reg = LanguageRegistry::new();
    reg.register_grammar(
        "zlang",
        rust_source(QueryFiles::new("unused.scm".into())),
        ConfigOrigin::User,
    );
    let zlang = reg.defined_id("zlang");
    assert!(zlang.is_some(), "a grammar defines its language");
    assert!(reg.iter_names().any(|name| name == "zlang"));

    assert_eq!(
        define(&mut reg, "zlang", "zl", ConfigOrigin::Bundled),
        WriteOutcome::Applied
    );
    assert_eq!(reg.by_file_name("a.zl"), zlang);
}

/// Removing a language its grammar alone defined leaves it undefined, with
/// no grammar.
#[test]
fn remove_takes_away_a_language_only_its_grammar_defined() {
    let mut reg = LanguageRegistry::new();
    reg.register_grammar(
        "zlang",
        rust_source(QueryFiles::new("unused.scm".into())),
        ConfigOrigin::User,
    );

    reg.remove("zlang");

    assert_eq!(reg.defined_id("zlang"), None);
    assert!(!reg.has_grammar("zlang"));
}
