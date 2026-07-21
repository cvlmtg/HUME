//! Language-identity and grammar Steel builtins.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::{Effect, PendingLanguageReg, SteelCtx};

use super::args::{list_to_strings, optional_path_arg, path_arg, string_arg};
use super::errors::generic_err;

type SteelResult = Result<SteelVal, SteelErr>;

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
    let name = match &name {
        SteelVal::StringV(s) => s.to_string(),
        SteelVal::SymbolV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch => "%define-language!: name must be a string or symbol"),
    };
    let extensions = list_to_strings(exts_val, "%define-language! extensions")?;
    let globs = list_to_strings(globs_val, "%define-language! globs")?;
    let shebangs = list_to_strings(shebangs_val, "%define-language! shebangs")?;
    ctx.push_effect(Effect::LanguageReg(PendingLanguageReg::Identity {
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

    if ctx.session == crate::context::EvalSession::Init {
        ctx.push_effect(Effect::LanguageReg(PendingLanguageReg::Grammar {
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
        .map_err(generic_err)?;
    ctx.push_effect(Effect::GrammarSweep(name));
    Ok(SteelVal::Void)
}

/// `(language-has-grammar? name)` — returns `#t` if `name` has an attached grammar.
pub(crate) fn language_has_grammar(ctx: &mut SteelCtx, name: SteelVal) -> SteelResult {
    let name = string_arg(name, "language-has-grammar?")?;
    Ok(SteelVal::BoolV(ctx.host.language().has_grammar(&name)))
}

#[cfg(test)]
mod tests;
