use super::super::args::{cons_pair, string_list, symbol_hash};
use super::*;
use crate::test_support::{SteelCtxTestHarness, str_val};
use crate::{ConfigOrigin, PendingLanguageReg};
use hume_editing::comment::{BlockCommentTokens, LineCommentTokens};
use hume_treesitter::registry::{LanguageFields, OptionalQuery};
use steel::rvals::IntoSteelVal as _;

fn empty_list() -> SteelVal {
    Vec::<SteelVal>::new().into_steelval().unwrap()
}

/// `Effect::LanguageReg` entries queued so far, in emission order.
fn lang_regs(h: &SteelCtxTestHarness) -> Vec<&PendingLanguageReg> {
    h.effects
        .iter()
        .filter_map(|e| match &e.effect {
            Effect::LanguageReg(reg) => Some(reg),
            _ => None,
        })
        .collect()
}

// ── %define-language! ────────────────────────────────────────────────────

/// `%define-language!` is blocked in plain command mode, gated at
/// registration time (`config` kind in `builtins!`'s table), not in the
/// body, so this tests the gate primitive directly rather than calling
/// `define_language` (its body has no guard to hit).
///
/// An `open` table entry would let language identity be defined at runtime,
/// corrupting the language registry.
#[test]
fn define_language_blocked_in_command_mode() {
    let mut h = SteelCtxTestHarness::new();
    let result = super::super::errors::require_config(&h.ctx(), "%define-language!");
    assert!(
        result.is_err(),
        "%define-language! must error in command mode"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("init"),
        "error must mention 'init'; got: {msg}"
    );
}

/// `%define-language!` rejects a non-string/symbol `name`.
#[test]
fn define_language_non_string_name_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = define_language(
        &mut ctx,
        SteelVal::IntV(42), // wrong type
        empty_list(),
        empty_list(),
        empty_list(),
    );
    assert!(
        result.is_err(),
        "%define-language! must reject non-string name"
    );
}

/// `%define-language!` in init mode queues a `PendingLanguageReg::Identity`.
#[test]
fn define_language_queues_pending_reg() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx_init();
        let result = define_language(
            &mut ctx,
            str_val("Rust"),
            empty_list(),
            empty_list(),
            empty_list(),
        );
        assert!(
            result.is_ok(),
            "%define-language! must succeed in init mode"
        );
    }
    assert_eq!(lang_regs(&h).len(), 1);
    assert!(
        matches!(lang_regs(&h)[0], PendingLanguageReg::Identity { name, .. } if name == "Rust"),
        "pending reg must be an Identity entry with name 'Rust'"
    );
}

/// The error `%define-language!` raises for `extensions` and `globs`.
fn define_language_error(extensions: &[&str], globs: &[&str]) -> String {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let strings = |items: &[&str]| string_list(items.iter().map(|s| s.to_string()));
    define_language(
        &mut ctx,
        str_val("x"),
        strings(extensions),
        strings(globs),
        empty_list(),
    )
    .unwrap_err()
    .to_string()
}

#[test]
fn define_language_raises_for_an_extension_that_cannot_name_a_file() {
    for ext in [".rs", "rs.", "", "a/b", "a..b"] {
        let err = define_language_error(&[ext], &[]);
        assert!(
            err.contains(&format!("invalid extension {ext:?}")),
            "{ext:?}: {err}"
        );
    }
}

#[test]
fn define_language_raises_for_an_invalid_glob() {
    let err = define_language_error(&[], &["[a"]);
    assert!(err.contains("invalid glob \"[a\""), "{err}");
}

/// `%define-language!` accepts a symbol as the language name.
#[test]
fn define_language_accepts_symbol_name() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx_init();
        let result = define_language(
            &mut ctx,
            SteelVal::SymbolV("python".into()),
            empty_list(),
            empty_list(),
            empty_list(),
        );
        assert!(result.is_ok());
    }
    assert!(matches!(
        lang_regs(&h)[0],
        PendingLanguageReg::Identity { name, .. } if name == "python"
    ));
}

fn configure(ctx: &mut SteelCtx, name: &str, fields: Vec<(&'static str, SteelVal)>) -> SteelResult {
    configure_language(ctx, str_val(name), symbol_hash(fields))
}

fn configured_fields(h: &SteelCtxTestHarness) -> Vec<&LanguageFields> {
    lang_regs(h)
        .into_iter()
        .map(|reg| match reg {
            PendingLanguageReg::Configure { fields, .. } => fields,
            other => panic!("expected a Configure registration, got {other:?}"),
        })
        .collect()
}

/// `%configure-language!` queues a `PendingLanguageReg::Configure` with each
/// field decoded; a missing key or `#f` becomes `None`.
#[test]
fn configure_language_decodes_each_field() {
    let mut h = SteelCtxTestHarness::new();
    let block = vec![
        cons_pair(str_val("/*"), str_val("*/")).unwrap(),
        cons_pair(str_val("/**"), str_val("*/")).unwrap(),
    ]
    .into_steelval()
    .unwrap();
    {
        let mut ctx = h.ctx_init();
        configure(
            &mut ctx,
            "go",
            vec![
                (
                    "roots",
                    string_list(["go.work", "go.mod"].map(String::from)),
                ),
                ("language-id", SteelVal::BoolV(false)),
            ],
        )
        .expect("roots without a language-id must decode");
        configure(
            &mut ctx,
            "tsx",
            vec![("language-id", str_val("typescriptreact"))],
        )
        .expect("a language-id without roots must decode");
        configure(
            &mut ctx,
            "rust",
            vec![
                (
                    "comment-tokens",
                    string_list(["//", "///"].map(String::from)),
                ),
                ("block-comment-tokens", block),
            ],
        )
        .expect("comment tokens must decode");
    }
    let fields = configured_fields(&h);
    assert_eq!(
        *fields[0],
        LanguageFields {
            roots: Some(vec!["go.work".to_string(), "go.mod".to_string()]),
            ..LanguageFields::default()
        }
    );
    assert_eq!(
        *fields[1],
        LanguageFields {
            lsp_language_id: Some("typescriptreact".to_string()),
            ..LanguageFields::default()
        }
    );
    assert_eq!(
        *fields[2],
        LanguageFields {
            comment_tokens: Some(LineCommentTokens::from_strs(&["//", "///"])),
            block_comment_tokens: Some(BlockCommentTokens::from_pairs(&[
                ("/*", "*/"),
                ("/**", "*/")
            ])),
            ..LanguageFields::default()
        }
    );
}

/// `%configure-language!` records who called it: init.scm's top level is the
/// user, any plugin body (eager load or lazy activation) is a plugin.
#[test]
fn configure_language_records_its_origin() {
    use crate::attribution::{EntryId, PluginId};

    let mut h = SteelCtxTestHarness::new();
    configure(&mut h.ctx_init(), "go", vec![("language-id", str_val("a"))])
        .expect("init top level must decode");
    h.plugin_stack
        .push(EntryId::main(PluginId::parse("core:test-plugin").unwrap()));
    configure(&mut h.ctx_init(), "go", vec![("language-id", str_val("b"))])
        .expect("eager plugin load must decode");
    configure(
        &mut h.ctx_activation(),
        "go",
        vec![("language-id", str_val("c"))],
    )
    .expect("lazy plugin activation must decode");

    let origins: Vec<ConfigOrigin> = lang_regs(&h)
        .into_iter()
        .map(|reg| match reg {
            PendingLanguageReg::Configure { origin, .. } => *origin,
            other => panic!("expected a Configure registration, got {other:?}"),
        })
        .collect();
    assert_eq!(
        origins,
        [
            ConfigOrigin::User,
            ConfigOrigin::Plugin,
            ConfigOrigin::Plugin
        ]
    );
}

/// Each malformed field is rejected with an error naming it.
#[test]
fn configure_language_rejects_a_malformed_field() {
    let not_a_pair = vec![str_val("/*")].into_steelval().unwrap();
    let blank_edge = vec![cons_pair(str_val("/* "), str_val("*/")).unwrap()]
        .into_steelval()
        .unwrap();
    for (key, value, problem) in [
        ("roots", str_val("go.mod"), "roots"),
        (
            "comment-tokens",
            string_list([""].map(String::from)),
            "empty",
        ),
        ("comment-tokens", empty_list(), "at least one"),
        ("block-comment-tokens", not_a_pair, "block-comment-tokens"),
        ("block-comment-tokens", blank_edge, "blank"),
    ] {
        let mut h = SteelCtxTestHarness::new();
        let err = configure(&mut h.ctx_init(), "go", vec![(key, value)])
            .expect_err("a malformed field must be rejected")
            .to_string();
        assert!(err.contains(key) && err.contains(problem), "{key}: {err}");
        assert!(lang_regs(&h).is_empty(), "{key}: nothing is queued");
    }
}

/// A key `configure-language!` does not know is an error, not ignored.
#[test]
fn configure_language_rejects_an_unknown_key() {
    let mut h = SteelCtxTestHarness::new();
    let err = configure(
        &mut h.ctx_init(),
        "go",
        vec![("grammar", string_list(["x"].map(String::from)))],
    )
    .expect_err("an unknown key must be rejected")
    .to_string();
    assert!(err.contains("grammar"), "{err}");
}

/// `%configure-language!` is gated to config evaluation, like
/// `%define-language!`.
#[test]
fn configure_language_blocked_in_command_mode() {
    let mut h = SteelCtxTestHarness::new();
    let result = super::super::errors::require_config(&h.ctx(), "%configure-language!");
    assert!(
        result.is_err(),
        "%configure-language! must error in command mode"
    );
}

// ── register-grammar! ────────────────────────────────────────────────────

/// In init mode, `register-grammar!` queues a `PendingLanguageReg::Grammar`
/// instead of calling the host immediately.
#[test]
fn register_grammar_init_mode_queues_pending_reg() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx_init();
        let result = register_grammar(
            &mut ctx,
            str_val("rust"),
            str_val("/tmp/rust.so"),
            str_val("tree_sitter_rust"),
            str_val("/tmp/highlights.scm"),
            steel::rvals::SteelVal::BoolV(false),
        );
        assert!(result.is_ok(), "register-grammar! in init must succeed");
    }
    assert_eq!(lang_regs(&h).len(), 1);
    assert!(
        matches!(lang_regs(&h)[0], PendingLanguageReg::Grammar(reg) if reg.name == "rust"),
        "pending reg must be a Grammar entry with name 'rust'"
    );
}

/// In command mode, `register-grammar!` calls the host immediately.
/// NullHost returns Err, proving the call reached the host.
#[test]
fn register_grammar_command_mode_calls_host() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx();
        let result = register_grammar(
            &mut ctx,
            str_val("rust"),
            str_val("/tmp/rust.so"),
            str_val("tree_sitter_rust"),
            str_val("/tmp/highlights.scm"),
            steel::rvals::SteelVal::BoolV(false),
        );
        // NullHost.attach_grammar returns Err.
        assert!(
            result.is_err(),
            "NullHost must return Err from attach_grammar"
        );
    }
    // Nothing queued: command mode calls the host directly.
    assert!(
        lang_regs(&h).is_empty(),
        "command mode must not queue pending regs"
    );
}

/// A `'textobjects` entry reaches `GrammarReg.queries` and leaves the other
/// kinds absent.
#[test]
fn register_grammar_hash_with_textobjects_only_populates_that_kind() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx_init();
        let result = register_grammar(
            &mut ctx,
            str_val("rust"),
            str_val("/tmp/rust.so"),
            str_val("tree_sitter_rust"),
            str_val("/tmp/highlights.scm"),
            symbol_hash([("textobjects", str_val("/tmp/textobjects.scm"))]),
        );
        assert!(result.is_ok());
    }
    match lang_regs(&h)[0] {
        PendingLanguageReg::Grammar(reg) => {
            assert_eq!(
                reg.source.queries.get(OptionalQuery::TextObjects),
                Some(std::path::Path::new("/tmp/textobjects.scm")),
            );
            assert_eq!(reg.source.queries.get(OptionalQuery::Injections), None);
        }
        other => panic!("expected a Grammar entry, got: {other:?}"),
    }
}

/// `#f` for the optional-queries argument yields no optional file, and a
/// key bound to `#f` is the same as an absent key.
#[test]
fn register_grammar_false_optional_queries_yield_none() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx_init();
        for optional in [
            SteelVal::BoolV(false),
            symbol_hash([("injections", SteelVal::BoolV(false))]),
        ] {
            register_grammar(
                &mut ctx,
                str_val("rust"),
                str_val("/tmp/rust.so"),
                str_val("tree_sitter_rust"),
                str_val("/tmp/highlights.scm"),
                optional,
            )
            .expect("register-grammar! must accept it");
        }
    }
    for reg in lang_regs(&h) {
        match reg {
            PendingLanguageReg::Grammar(reg) => {
                for kind in OptionalQuery::ALL {
                    assert_eq!(reg.source.queries.get(kind), None, "{kind:?}");
                }
            }
            other => panic!("expected a Grammar entry, got: {other:?}"),
        }
    }
}

/// A key outside `OptionalQuery::ALL` is reported, naming the key.
#[test]
fn register_grammar_unknown_optional_query_key_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let err = register_grammar(
        &mut ctx,
        str_val("rust"),
        str_val("/tmp/rust.so"),
        str_val("tree_sitter_rust"),
        str_val("/tmp/highlights.scm"),
        symbol_hash([("textobject", str_val("/tmp/t.scm"))]),
    )
    .expect_err("a misspelled key must be rejected");
    let msg = err.to_string();
    assert!(msg.contains("unknown key 'textobject"), "{msg}");
}

/// A value that is neither a path string nor `#f` is a type error.
#[test]
fn register_grammar_non_path_optional_query_value_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = register_grammar(
        &mut ctx,
        str_val("rust"),
        str_val("/tmp/rust.so"),
        str_val("tree_sitter_rust"),
        str_val("/tmp/highlights.scm"),
        symbol_hash([("injections", SteelVal::IntV(1))]),
    );
    let msg = result
        .expect_err("a non-path value must be rejected")
        .to_string();
    assert!(msg.contains("injections"), "{msg}");
}

/// `grammar-optional-queries` lists every kind as `(key . file-name)`, in
/// `OptionalQuery::ALL` order.
#[test]
fn grammar_optional_queries_lists_every_kind_with_its_file() {
    let listed = grammar_optional_queries().expect("builtin must succeed");
    let SteelVal::ListV(items) = listed else {
        panic!("expected a list, got: {listed:?}");
    };
    let listed: Vec<String> = items.iter().map(SteelVal::to_string).collect();
    assert_eq!(
        listed,
        [
            "(injections . \"injections.scm\")",
            "(textobjects . \"textobjects.scm\")",
            "(indents . \"indents.scm\")",
        ]
    );
}

// ── language-has-grammar? ─────────────────────────────────────────────────

/// `language-has-grammar?` returns `#f` when the host reports no grammar
/// (NullHost always returns `false`).
#[test]
fn language_has_grammar_returns_false_for_unknown() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = language_has_grammar(&mut ctx, str_val("rust"));
    assert!(matches!(result, Ok(SteelVal::BoolV(false))));
}

/// `language-has-grammar?` rejects a non-string/symbol argument.
#[test]
fn language_has_grammar_wrong_type_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = language_has_grammar(&mut ctx, SteelVal::IntV(0));
    assert!(result.is_err());
}

/// Relative grammar paths resolve against the editor cwd, the same way
/// `open-buffer!` and `#:cwd` resolve theirs.
#[test]
fn register_grammar_resolves_relative_paths_against_the_editor_cwd() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx_init();
        let result = register_grammar(
            &mut ctx,
            str_val("rust"),
            str_val("g/rust.so"),
            str_val("tree_sitter_rust"),
            str_val("g/highlights.scm"),
            symbol_hash([
                ("injections", str_val("g/injections.scm")),
                ("textobjects", str_val("g/textobjects.scm")),
            ]),
        );
        assert!(result.is_ok());
    }
    let cwd = std::env::current_dir().unwrap();
    match lang_regs(&h)[0] {
        PendingLanguageReg::Grammar(reg) => {
            assert_eq!(reg.source.path, cwd.join("g").join("rust.so"));
            assert_eq!(
                reg.source.queries.highlights(),
                cwd.join("g").join("highlights.scm")
            );
            assert_eq!(
                reg.source.queries.get(OptionalQuery::Injections),
                Some(cwd.join("g").join("injections.scm").as_path())
            );
            assert_eq!(
                reg.source.queries.get(OptionalQuery::TextObjects),
                Some(cwd.join("g").join("textobjects.scm").as_path())
            );
        }
        other => panic!("expected a Grammar entry, got: {other:?}"),
    }
}
