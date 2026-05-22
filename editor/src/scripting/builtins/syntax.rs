//! Language-identity Steel builtin: `%define-language!`.

use steel::rerrs::{ErrorKind, SteelErr};
use steel::rvals::SteelVal;

use crate::scripting::{PendingLanguageReg, SteelCtx};

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
    let extensions = parse_string_list(exts_val, "%define-language! extensions")?;
    let globs = parse_string_list(globs_val, "%define-language! globs")?;
    let shebangs = parse_string_list(shebangs_val, "%define-language! shebangs")?;
    ctx.pending_language_regs.push(PendingLanguageReg::Identity { name, extensions, globs, shebangs });
    Ok(SteelVal::Void)
}

/// Parse a Steel list of strings into a `Vec<String>`.
fn parse_string_list(val: SteelVal, ctx_name: &str) -> Result<Vec<String>, SteelErr> {
    match val {
        SteelVal::ListV(list) => {
            let mut out = Vec::with_capacity(list.len());
            for item in list {
                match item {
                    SteelVal::StringV(s) => out.push(s.to_string()),
                    _ => {
                        return Err(SteelErr::new(
                            ErrorKind::TypeMismatch,
                            format!("{ctx_name}: list must contain only strings"),
                        ))
                    }
                }
            }
            Ok(out)
        }
        SteelVal::VectorV(v) => {
            let mut out = Vec::with_capacity(v.len());
            for item in v.iter() {
                match item {
                    SteelVal::StringV(s) => out.push(s.to_string()),
                    _ => {
                        return Err(SteelErr::new(
                            ErrorKind::TypeMismatch,
                            format!("{ctx_name}: list must contain only strings"),
                        ))
                    }
                }
            }
            Ok(out)
        }
        _ => Err(SteelErr::new(
            ErrorKind::TypeMismatch,
            format!("{ctx_name}: expected a list"),
        )),
    }
}
