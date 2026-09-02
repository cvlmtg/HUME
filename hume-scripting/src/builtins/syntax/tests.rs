use super::*;
use crate::{PendingLanguageReg, test_support::SteelCtxTestHarness};
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

/// `%define-language!` is blocked in plain command mode — gated at
/// registration time (`config` kind in `builtins!`'s table), not in the
/// body, so this tests the gate primitive directly rather than calling
/// `define_language` (its body has no guard to hit).
///
/// Fail oracle: change `%define-language!`'s table entry from `config` to
/// `open` → language identity could be defined at runtime, corrupting the
/// language registry.
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
        SteelVal::BoolV(false),
    );
    assert!(
        result.is_err(),
        "%define-language! must reject non-string name"
    );
}

/// `%define-language!` in init mode queues a `PendingLanguageReg::Identity`.
///
/// Fail oracle: make the call a no-op → the effect log stays empty →
/// last assert fires.
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
            SteelVal::BoolV(false),
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
            SteelVal::BoolV(false),
        );
        assert!(result.is_ok());
    }
    assert!(matches!(
        lang_regs(&h)[0],
        PendingLanguageReg::Identity { name, .. } if name == "python"
    ));
}

/// `%define-language!`'s 5th arg decodes into `PendingLanguageReg::Identity`'s
/// `lsp_language_id` — `#f` becomes `None`, a string becomes `Some`.
///
/// Fail oracle: dropping the 5th arg or ignoring it would leave
/// `lsp_language_id` `None` even when a string was passed — the second
/// assertion below would fail.
#[test]
fn define_language_decodes_lsp_language_id_arg() {
    let mut h = SteelCtxTestHarness::new();
    {
        let mut ctx = h.ctx_init();
        define_language(
            &mut ctx,
            str_val("tsx"),
            empty_list(),
            empty_list(),
            empty_list(),
            str_val("typescriptreact"),
        )
        .expect("%define-language! must succeed with a string lsp-language-id");
    }
    assert!(matches!(
        lang_regs(&h)[0],
        PendingLanguageReg::Identity { lsp_language_id, .. }
            if lsp_language_id.as_deref() == Some("typescriptreact")
    ));
}

// ── register-grammar! ────────────────────────────────────────────────────

/// In init mode, `register-grammar!` queues a `PendingLanguageReg::Grammar`
/// instead of calling the host immediately.
///
/// Fail oracle: call host immediately in init mode → the effect log
/// stays empty → last assert fires.
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
        matches!(lang_regs(&h)[0], PendingLanguageReg::Grammar { name, .. } if name == "rust"),
        "pending reg must be a Grammar entry with name 'rust'"
    );
}

/// In command mode, `register-grammar!` calls the host immediately.
/// NullHost returns Err, proving the call reached the host.
///
/// Fail oracle: always queue in init path even for command mode → host is never
/// called → the error would be absent and the effect log would be non-empty.
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
    // Nothing queued — command mode calls the host directly.
    assert!(
        lang_regs(&h).is_empty(),
        "command mode must not queue pending regs"
    );
}

/// A string (not `#f`) in the 5th position must reach
/// `PendingLanguageReg::Grammar.injections_path` as `Some`.
///
/// Flip: if `optional_path_arg` ignored the string branch, this would be
/// `None` — same as the `#f` case in the tests above.
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
        PendingLanguageReg::Grammar {
            injections_path, ..
        } => assert_eq!(
            injections_path.as_deref(),
            Some(std::path::Path::new("/tmp/injections.scm")),
        ),
        other => panic!("expected a Grammar entry, got: {other:?}"),
    }
}

/// A string (not `#f`) in the 6th position must reach
/// `PendingLanguageReg::Grammar.textobjects_path` as `Some`.
///
/// Flip: if `optional_path_arg` ignored the string branch, this would be
/// `None` — same as the `#f` case in the tests above.
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
        PendingLanguageReg::Grammar {
            textobjects_path, ..
        } => assert_eq!(
            textobjects_path.as_deref(),
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
