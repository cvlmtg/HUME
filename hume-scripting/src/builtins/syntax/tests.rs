use super::super::args::cons_pair;
use super::*;
use crate::{ConfigOrigin, PendingLanguageReg, test_support::SteelCtxTestHarness};
use hume_editing::comment::{BlockCommentTokens, LineCommentTokens};
use hume_treesitter::registry::LanguageFields;
use steel::rvals::IntoSteelVal as _;

fn str_val(s: &str) -> SteelVal {
    SteelVal::StringV(s.into())
}

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

fn sym_hash(pairs: Vec<(&'static str, SteelVal)>) -> SteelVal {
    super::super::args::symbol_hash(pairs)
}

fn str_list(items: &[&str]) -> SteelVal {
    items
        .iter()
        .map(|s| str_val(s))
        .collect::<Vec<_>>()
        .into_steelval()
        .unwrap()
}

fn configure(ctx: &mut SteelCtx, name: &str, fields: Vec<(&'static str, SteelVal)>) -> SteelResult {
    configure_language(ctx, str_val(name), sym_hash(fields))
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
                ("roots", str_list(&["go.work", "go.mod"])),
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
                ("comment-tokens", str_list(&["//", "///"])),
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
            comment_tokens: Some(
                LineCommentTokens::new(vec!["//".to_string(), "///".to_string()]).unwrap()
            ),
            block_comment_tokens: Some(
                BlockCommentTokens::new(vec![
                    ("/*".to_string(), "*/".to_string()),
                    ("/**".to_string(), "*/".to_string()),
                ])
                .unwrap()
            ),
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
        ("comment-tokens", str_list(&[""]), "empty"),
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
    let err = configure(&mut h.ctx_init(), "go", vec![("grammar", str_list(&["x"]))])
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

/// A string (not `#f`) in the 5th position must reach
/// `PendingLanguageReg::Grammar.injections_path` as `Some`. If
/// `optional_path_arg` ignored the string branch, it would be `None`, just
/// like the `#f` case in the tests above.
#[test]
fn register_grammar_with_injections_path_populates_pending_reg() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx_init();
        let result = register_grammar(
            &mut ctx,
            str_val("markdown"),
            str_val("/tmp/markdown.so"),
            str_val("tree_sitter_markdown"),
            str_val("/tmp/highlights.scm"),
            str_val("/tmp/injections.scm"),
            steel::rvals::SteelVal::BoolV(false),
        );
        assert!(result.is_ok());
    }
    match lang_regs(&h)[0] {
        PendingLanguageReg::Grammar(reg) => assert_eq!(
            reg.injections_path.as_deref(),
            Some(std::path::Path::new("/tmp/injections.scm")),
        ),
        other => panic!("expected a Grammar entry, got: {other:?}"),
    }
}

/// A string (not `#f`) in the 6th position must reach
/// `PendingLanguageReg::Grammar.textobjects_path` as `Some`, which it
/// could not do if `optional_path_arg` dropped the string branch.
#[test]
fn register_grammar_with_textobjects_path_populates_pending_reg() {
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
            str_val("/tmp/textobjects.scm"),
        );
        assert!(result.is_ok());
    }
    match lang_regs(&h)[0] {
        PendingLanguageReg::Grammar(reg) => assert_eq!(
            reg.textobjects_path.as_deref(),
            Some(std::path::Path::new("/tmp/textobjects.scm")),
        ),
        other => panic!("expected a Grammar entry, got: {other:?}"),
    }
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

/// Relative grammar paths resolve lexically against the editor cwd, the same
/// way `open-buffer!` and `#:cwd` resolve theirs.
#[test]
fn register_grammar_resolves_relative_paths_against_the_editor_cwd() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx_init();
        let result = register_grammar(
            &mut ctx,
            str_val("rust"),
            str_val("g/../rust.so"),
            str_val("tree_sitter_rust"),
            str_val("g/../highlights.scm"),
            str_val("g/../injections.scm"),
            str_val("g/../textobjects.scm"),
        );
        assert!(result.is_ok());
    }
    match lang_regs(&h)[0] {
        PendingLanguageReg::Grammar(reg) => {
            assert_eq!(reg.grammar_path, std::path::Path::new("rust.so"));
            assert_eq!(reg.highlights_path, std::path::Path::new("highlights.scm"));
            assert_eq!(
                reg.injections_path.as_deref(),
                Some(std::path::Path::new("injections.scm"))
            );
            assert_eq!(
                reg.textobjects_path.as_deref(),
                Some(std::path::Path::new("textobjects.scm"))
            );
        }
        other => panic!("expected a Grammar entry, got: {other:?}"),
    }
}
