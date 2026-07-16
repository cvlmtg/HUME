//! Language-identity and grammar Steel builtins.

use std::path::PathBuf;

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::{Effect, PendingLanguageReg, SteelCtx};

use super::{list_to_strings, require_config_ctx};

type SteelResult = Result<SteelVal, SteelErr>;

fn string_arg(val: SteelVal, ctx_name: &str) -> Result<String, SteelErr> {
    match val {
        SteelVal::StringV(s) => Ok(s.to_string()),
        SteelVal::SymbolV(s) => Ok(s.to_string()),
        _ => steel::stop!(TypeMismatch => "{}: expected a string", ctx_name),
    }
}

fn path_arg(val: SteelVal, ctx_name: &str) -> Result<PathBuf, SteelErr> {
    match val {
        SteelVal::StringV(s) => Ok(PathBuf::from(s.as_str())),
        _ => steel::stop!(TypeMismatch => "{}: expected a string path", ctx_name),
    }
}

/// A path argument that may be `#f` (absent).
fn optional_path_arg(val: SteelVal, ctx_name: &str) -> Result<Option<PathBuf>, SteelErr> {
    match val {
        SteelVal::BoolV(false) => Ok(None),
        SteelVal::StringV(s) => Ok(Some(PathBuf::from(s.as_str()))),
        _ => steel::stop!(TypeMismatch => "{}: expected a string path or #f", ctx_name),
    }
}

/// `(%define-language! name extensions globs shebangs)` — init-only.
///
/// All three list args must be lists of strings. Pushes an
/// `Effect::LanguageReg(PendingLanguageReg::Identity)`; `Editor::apply_pending_language_regs`
/// applies it as part of `Editor::apply_script_effects`.
pub(crate) fn define_language(
    ctx: &mut SteelCtx,
    name: SteelVal,
    exts_val: SteelVal,
    globs_val: SteelVal,
    shebangs_val: SteelVal,
) -> SteelResult {
    require_config_ctx!(ctx, "%define-language!");
    let name = match &name {
        SteelVal::StringV(s) => s.to_string(),
        SteelVal::SymbolV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch => "%define-language!: name must be a string or symbol"),
    };
    let extensions = list_to_strings(exts_val, "%define-language! extensions")?;
    let globs = list_to_strings(globs_val, "%define-language! globs")?;
    let shebangs = list_to_strings(shebangs_val, "%define-language! shebangs")?;
    ctx.effects
        .push(Effect::LanguageReg(PendingLanguageReg::Identity {
            name,
            extensions,
            globs,
            shebangs,
        }));
    Ok(SteelVal::Void)
}

/// `(%register-grammar! name grammar-path symbol highlights-path injections-path)`
/// — init or command. `injections-path` is a string or `#f`. The Scheme-side
/// `register-grammar!` macro (`prelude.scm`) supplies `#f` when the caller
/// omits it.
///
/// - **Init mode**: pushes an `Effect::LanguageReg(PendingLanguageReg::Grammar)`.
/// - **Command mode**: attaches immediately via the editor host and pushes
///   an `Effect::GrammarSweep` to sweep already-open buffers afterward.
pub(crate) fn register_grammar(
    ctx: &mut SteelCtx,
    name: SteelVal,
    grammar_path: SteelVal,
    symbol: SteelVal,
    highlights_path: SteelVal,
    injections_path: SteelVal,
) -> SteelResult {
    let name = string_arg(name, "register-grammar! name")?;
    let grammar_path = path_arg(grammar_path, "register-grammar! grammar-path")?;
    let symbol = string_arg(symbol, "register-grammar! symbol")?;
    let highlights_path = path_arg(highlights_path, "register-grammar! highlights-path")?;
    let injections_path = optional_path_arg(injections_path, "register-grammar! injections-path")?;

    if ctx.is_init {
        ctx.effects
            .push(Effect::LanguageReg(PendingLanguageReg::Grammar {
                name,
                grammar_path,
                symbol,
                highlights_path,
                injections_path,
            }));
        return Ok(SteelVal::Void);
    }

    // Command mode — attach immediately via host and trigger a buffer sweep.
    // The host (`EditorHostImpl::attach_grammar`) owns the `register-grammar! '<name>':`
    // prefix; just lift its String error into a SteelErr without re-prefixing.
    ctx.host
        .language()
        .attach_grammar(
            &name,
            &grammar_path,
            &symbol,
            &highlights_path,
            injections_path.as_deref(),
        )
        .map_err(|e| steel::rerrs::SteelErr::new(steel::rerrs::ErrorKind::Generic, e))?;
    ctx.effects.push(Effect::GrammarSweep(name));
    Ok(SteelVal::Void)
}

/// `(language-has-grammar? name)` — returns `#t` if `name` has an attached grammar.
pub(crate) fn language_has_grammar(ctx: &mut SteelCtx, name: SteelVal) -> SteelResult {
    let name = string_arg(name, "language-has-grammar?")?;
    Ok(SteelVal::BoolV(ctx.host.language().has_grammar(&name)))
}

#[cfg(test)]
mod tests {
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
            .filter_map(|e| match e {
                Effect::LanguageReg(reg) => Some(reg),
                _ => None,
            })
            .collect()
    }

    // ── %define-language! ────────────────────────────────────────────────────

    /// `%define-language!` is blocked in plain command mode.
    ///
    /// Fail oracle: remove the guard → language identity can be defined at runtime,
    /// corrupting the language registry.
    #[test]
    fn define_language_blocked_in_command_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = define_language(
            &mut ctx,
            str_val("Rust"),
            empty_list(),
            empty_list(),
            empty_list(),
        );
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
}
