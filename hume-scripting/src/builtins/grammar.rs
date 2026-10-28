//! Grammar compilation builtin for HUME's Steel scripting engine.
//!
//! | Steel name             | Signature           | Notes                          |
//! |------------------------|---------------------|--------------------------------|
//! | `compile-grammar!`     | `string string → void` | `hume-grammar-build -o out src` |
//!
//! No sandbox checks: full-trust plugin model (see
//! `user-manual/docs/writing-plugins.md`'s "Filesystem and processes"). This is a Rust
//! builtin because the compiler is found beside the HUME executable, which
//! only the scripting host knows. The compiler's output is captured, never
//! written to the terminal, so a compile runs anywhere a script does.
//! `grammar-output-path` (path
//! construction, no filesystem access) lives in Scheme
//! (`runtime/scheme/grammars.scm`).

use std::path::Path;

use hume_platform::dirs::GrammarBuild;
use hume_platform::path::display_form;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use hume_core::Severity;

use super::SteelResult;
use super::errors::generic_err;

/// `(compile-grammar! src out)`: compile the tree-sitter grammar source at
/// `src` to `out` with the grammar compiler shipped beside HUME.
///
/// - **Init mode**: logs a Warning on failure and returns `#<void>`; a
///   grammar that fails to build should not abort the editor on startup.
/// - **Command mode**: raises a Steel error on failure so the user sees it.
pub(crate) fn compile_grammar(ctx: &mut SteelCtx, src: String, out: String) -> SteelResult {
    let Err(msg) = build(ctx, &src, &out) else {
        return Ok(SteelVal::Void);
    };
    if ctx.session == crate::context::EvalSession::Init {
        ctx.log(Severity::Warning, format!("compile-grammar!: {msg}"));
        Ok(SteelVal::Void)
    } else {
        Err(generic_err(msg))
    }
}

/// Run the grammar compiler, or say why it did not build `src`.
fn build(ctx: &mut SteelCtx, src: &str, out: &str) -> Result<(), String> {
    let tool = match &ctx.dirs.grammar_build {
        GrammarBuild::Found(tool) => tool.clone(),
        GrammarBuild::ExeUnknown => {
            return Err("hume-grammar-build was not found: \
                 the location of the hume executable is unknown"
                .to_string());
        }
        GrammarBuild::Missing { searched } => {
            let paths: Vec<String> = searched.iter().map(|p| display_form(p)).collect();
            return Err(format!(
                "hume-grammar-build was not found; looked for {}",
                paths.join(", ")
            ));
        }
    };
    let program = display_form(&tool);

    ctx.log(
        Severity::Trace,
        format!("compile-grammar!: `{program} -o {out} {src}`"),
    );

    let cwd = ctx.host.buffers().cwd();
    let search_path = ctx.host.buffers().search_path();
    let launch = hume_platform::process::Launch {
        cwd: &cwd,
        search_path: search_path.as_deref(),
    };
    let built = ctx.while_waiting(|| {
        hume_platform::process::grammar_build(&tool, Path::new(src), Path::new(out), launch)
    });
    match built {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(hume_platform::process::failure_message(
            &format!("`{program}`"),
            Some(output.status),
            &hume_platform::process::decode_lossy(output.stderr),
        )),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests;
