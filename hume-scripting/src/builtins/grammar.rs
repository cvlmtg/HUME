//! Grammar compilation builtin for HUME's Steel scripting engine.
//!
//! | Steel name             | Signature           | Notes                          |
//! |------------------------|---------------------|--------------------------------|
//! | `compile-grammar!`     | `string string → void` | `hume-grammar-build -o out src` |
//!
//! No sandbox checks: full-trust plugin model (see
//! `user-manual/docs/plugins.md`'s "Filesystem and processes"). This is a Rust
//! builtin because the compiler is found beside the HUME executable, which
//! only the scripting host knows, and because its inherited output needs the
//! inline-output screen opened first. `grammar-output-path` (path
//! construction, no filesystem access) lives in Scheme
//! (`runtime/scheme/grammars.scm`).

use std::path::Path;

use hume_platform::dirs::GrammarBuild;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::log::LogLevel;

use super::SteelResult;
use super::errors::generic_err;

/// `(compile-grammar! src out)`: compile the tree-sitter grammar source at
/// `src` to `out` with the grammar compiler shipped beside HUME.
///
/// - **Init mode**: logs a Warning on failure and returns `#<void>`; a
///   grammar that fails to build should not abort the editor on startup.
/// - **Command mode**: raises a Steel error on failure so the user sees it.
pub(crate) fn compile_grammar(ctx: &mut SteelCtx, src: String, out: String) -> SteelResult {
    let msg = match build(ctx, &src, &out) {
        Ok(()) => return Ok(SteelVal::Void),
        Err(msg) => msg,
    };
    if ctx.session == crate::context::EvalSession::Init {
        ctx.log(LogLevel::Warning, msg);
        Ok(SteelVal::Void)
    } else {
        Err(generic_err(msg))
    }
}

/// Run the grammar compiler, or say why it did not build `src`.
fn build(ctx: &mut SteelCtx, src: &str, out: &str) -> Result<(), String> {
    let tool = match &ctx.dirs.grammar_build {
        GrammarBuild::Found(tool) => tool.clone(),
        GrammarBuild::Missing { searched } if searched.is_empty() => {
            return Err("compile-grammar!: hume-grammar-build was not found: \
                 the location of the hume executable is unknown"
                .to_string());
        }
        GrammarBuild::Missing { searched } => {
            let paths: Vec<String> = searched.iter().map(|p| p.display().to_string()).collect();
            return Err(format!(
                "compile-grammar!: hume-grammar-build was not found; looked for {}",
                paths.join(", ")
            ));
        }
    };

    ctx.log(
        LogLevel::Trace,
        format!("compile-grammar!: `{} -o {out} {src}`", tool.display()),
    );

    // The compiler inherits stdio, so this is a real terminal write, so open
    // the bracket first. Init-time compiles run pre-terminal (no screen to
    // enter, and no `#:inline-output` frame is ever armed there anyway).
    if ctx.session == crate::context::EvalSession::Runtime
        && let Some(output) = ctx.host.output()
    {
        output
            .ensure_inline_output_screen()
            .map_err(|e| format!("compile-grammar!: {e}"))?;
    }

    let cwd = ctx.host.buffers().cwd();
    let program = tool.display().to_string();
    match hume_platform::process::grammar_build(&tool, Path::new(src), Path::new(out), &cwd) {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(hume_platform::process::failure_message(
            &format!("compile-grammar!: `{program}`"),
            Some(status),
            "",
        )),
        Err(e) => Err(format!("compile-grammar!: {e}")),
    }
}

#[cfg(test)]
mod tests;
