//! Language-identity Steel builtin: `%define-language!` / `(define-language!)`.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::scripting::{PendingLanguageReg, SteelCtx};

use super::list_to_strings;

type SteelResult = Result<SteelVal, SteelErr>;

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

