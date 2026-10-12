//! Language-identity and grammar Steel builtins.

use steel::rvals::SteelVal;

use crate::{Effect, GrammarReg, PendingLanguageReg, SteelCtx};

use super::SteelResult;
use super::args::{
    list_to_strings, optional_list_to_strings, optional_path_arg, optional_string_arg, path_arg,
    string_arg,
};
use super::errors::generic_err;

/// A language name argument: a string or a symbol.
fn language_name(val: &SteelVal, builtin: &str) -> Result<String, steel::rerrs::SteelErr> {
    match val {
        SteelVal::StringV(s) => Ok(s.to_string()),
        SteelVal::SymbolV(s) => Ok(s.to_string()),
        _ => steel::stop!(TypeMismatch => "{}: name must be a string or symbol", builtin),
    }
}

/// `(%define-language! name extensions globs shebangs)`: init-only.
///
/// `extensions`, `globs` and `shebangs` must be lists of strings. Pushes an
/// `Effect::LanguageReg(PendingLanguageReg::Identity)`;
/// `Editor::apply_pending_language_regs` applies it as part of
/// `Editor::apply_script_effects`.
pub(crate) fn define_language(
    ctx: &mut SteelCtx,
    name: SteelVal,
    exts_val: SteelVal,
    globs_val: SteelVal,
    shebangs_val: SteelVal,
) -> SteelResult {
    let name = language_name(&name, "%define-language!")?;
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

/// `(%configure-language! name roots lsp-language-id)`: init-only, like
/// `%define-language!`. `roots` is a list of strings or `#f`,
/// `lsp-language-id` a string or `#f`; `#f` leaves that field as it is.
/// Pushes an `Effect::LanguageReg(PendingLanguageReg::Configure)`, applied in
/// queue order after any `%define-language!` queued before it.
pub(crate) fn configure_language(
    ctx: &mut SteelCtx,
    name: SteelVal,
    roots_val: SteelVal,
    lsp_language_id_val: SteelVal,
) -> SteelResult {
    let name = language_name(&name, "%configure-language!")?;
    let roots = optional_list_to_strings(roots_val, "%configure-language! roots")?;
    let lsp_language_id =
        optional_string_arg(lsp_language_id_val, "%configure-language! language-id")?;
    ctx.push_effect(Effect::LanguageReg(PendingLanguageReg::Configure {
        name,
        roots,
        lsp_language_id,
    }));
    Ok(SteelVal::Void)
}

/// `(%register-grammar! name grammar-path symbol highlights-path injections-path
/// textobjects-path)`: init or command. `injections-path` and
/// `textobjects-path` are each a string or `#f`; the Scheme-side
/// `register-grammar!` wrapper (`prelude.scm`) supplies `#f` for either
/// keyword the caller omits.
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
    textobjects_path: SteelVal,
) -> SteelResult {
    let cwd = ctx.host.buffers().cwd();
    let under_cwd = |p: std::path::PathBuf| hume_platform::path::absolute_unresolved(&p, &cwd);
    let reg = GrammarReg {
        name: string_arg(name, "register-grammar! name")?,
        grammar_path: under_cwd(path_arg(grammar_path, "register-grammar! grammar-path")?),
        symbol: string_arg(symbol, "register-grammar! symbol")?,
        highlights_path: under_cwd(path_arg(
            highlights_path,
            "register-grammar! highlights-path",
        )?),
        injections_path: optional_path_arg(injections_path, "register-grammar! injections-path")?
            .map(under_cwd),
        textobjects_path: optional_path_arg(
            textobjects_path,
            "register-grammar! textobjects-path",
        )?
        .map(under_cwd),
    };

    if ctx.session == crate::context::EvalSession::Init {
        ctx.push_effect(Effect::LanguageReg(PendingLanguageReg::Grammar(reg)));
        return Ok(SteelVal::Void);
    }

    // Command mode: attach immediately via host and trigger a buffer sweep.
    // The host (`EditorHostImpl::attach_grammar`) owns the `register-grammar! '<name>':`
    // prefix; just lift its String error into a SteelErr without re-prefixing.
    ctx.host
        .language()
        .attach_grammar(&reg)
        .map_err(generic_err)?;
    ctx.push_effect(Effect::GrammarSweep(reg.name));
    Ok(SteelVal::Void)
}

/// `(language-has-grammar? name)` returns `#t` if `name` has an attached grammar.
pub(crate) fn language_has_grammar(ctx: &mut SteelCtx, name: SteelVal) -> SteelResult {
    let name = string_arg(name, "language-has-grammar?")?;
    Ok(SteelVal::BoolV(ctx.host.language().has_grammar(&name)))
}

#[cfg(test)]
mod tests;
