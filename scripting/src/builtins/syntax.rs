//! Language-identity and grammar Steel builtins.

use std::path::PathBuf;

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::{PendingLanguageReg, SteelCtx};

use super::list_to_strings;

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

/// `(%define-language! name extensions globs shebangs)` — init-only.
///
/// All three list args must be lists of strings. Pushes a `PendingLanguageReg::Identity`
/// onto `ctx.pending_language_regs`; `Editor::flush_pending_language_regs` applies
/// them after each `eval_init` boundary.
pub(crate) fn define_language(
    ctx: &mut SteelCtx,
    name: SteelVal,
    exts_val: SteelVal,
    globs_val: SteelVal,
    shebangs_val: SteelVal,
) -> SteelResult {
    if !ctx.is_init {
        steel::stop!(Generic => "%define-language!: only callable during init (use define-language! in init.scm or a plugin)");
    }
    let name = match &name {
        SteelVal::StringV(s) => s.to_string(),
        SteelVal::SymbolV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch => "%define-language!: name must be a string or symbol"),
    };
    let extensions = list_to_strings(exts_val, "%define-language! extensions")?;
    let globs = list_to_strings(globs_val, "%define-language! globs")?;
    let shebangs = list_to_strings(shebangs_val, "%define-language! shebangs")?;
    ctx.pending_language_regs.push(PendingLanguageReg::Identity { name, extensions, globs, shebangs });
    Ok(SteelVal::Void)
}

/// `(register-grammar! name grammar-path symbol highlights-path)` — init or command.
///
/// - **Init mode**: queues a `PendingLanguageReg::Grammar`; applied after
///   `eval_init` completes via `apply_pending_language_regs`.
/// - **Command mode**: attaches immediately via the editor host, rebakes the
///   theme, and queues a buffer sweep.
pub(crate) fn register_grammar(
    ctx: &mut SteelCtx,
    name: SteelVal,
    grammar_path: SteelVal,
    symbol: SteelVal,
    highlights_path: SteelVal,
) -> SteelResult {
    let name = string_arg(name, "register-grammar! name")?;
    let grammar_path = path_arg(grammar_path, "register-grammar! grammar-path")?;
    let symbol = string_arg(symbol, "register-grammar! symbol")?;
    let highlights_path = path_arg(highlights_path, "register-grammar! highlights-path")?;

    if ctx.is_init {
        ctx.pending_language_regs.push(PendingLanguageReg::Grammar {
            name,
            grammar_path,
            symbol,
            highlights_path,
        });
        return Ok(SteelVal::Void);
    }

    // Command mode — attach immediately via host and trigger a buffer sweep.
    ctx.host
        .attach_grammar(&name, &grammar_path, &symbol, &highlights_path)
        .map_err(|e| {
            steel::rerrs::SteelErr::new(
                steel::rerrs::ErrorKind::Generic,
                format!("register-grammar! '{name}': {e}"),
            )
        })?;
    ctx.pending_grammar_sweeps.push(name);
    Ok(SteelVal::Void)
}

/// `(language-has-grammar? name)` — returns `#t` if `name` has an attached grammar.
pub(crate) fn language_has_grammar(ctx: &mut SteelCtx, name: SteelVal) -> SteelResult {
    let name = string_arg(name, "language-has-grammar?")?;
    Ok(SteelVal::BoolV(ctx.host.has_grammar(&name)))
}
