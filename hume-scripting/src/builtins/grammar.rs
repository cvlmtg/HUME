//! Grammar compilation builtin for HUME's Steel scripting engine.
//!
//! | Steel name             | Signature           | Notes                          |
//! |------------------------|---------------------|--------------------------------|
//! | `compile-grammar!`     | `string string → void` | `tree-sitter build -o out src` |
//!
//! No sandbox checks — full-trust plugin model (see
//! `user-manual/docs/plugins.md`'s "Filesystem and processes"). This stays a Rust builtin only for the
//! Windows compiler-selection dance (`hume_platform::process::tree_sitter_build`
//! probes `cl`/`clang`/`gcc`/`zig` and writes `--target`-stripping wrapper
//! scripts) — a Scheme rewrite would only make that logic worse.
//! `grammar-output-path` (path construction, no filesystem access) moved to
//! Scheme (`runtime/plugins/core/plum/grammars.scm`) since it isn't
//! platform-conditional in the same way.

use std::path::PathBuf;

use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::log::LogLevel;

use super::SteelResult;
use super::errors::generic_err;

/// Suffix appended to a grammar-compile failure message when no C compiler
/// was found on `PATH` — empty on non-Windows, where a compiler is either
/// preinstalled or the platform-native error is already clear enough.
fn windows_compiler_hint() -> String {
    #[cfg(windows)]
    if hume_platform::process::no_windows_compiler_found() {
        return " — no C compiler found; install one of: MSVC Build Tools, clang, gcc, or zig, \
                 and ensure it is on PATH"
            .to_string();
    }
    String::new()
}

/// `(compile-grammar! src out)` — compile the tree-sitter grammar source at
/// `src` to `out` via `tree-sitter build -o <out> <src>`.
///
/// - **Init mode**: logs a Warning on failure and returns `#<void>` — a
///   missing `tree-sitter` binary should not abort the editor on startup.
/// - **Command mode**: raises a Steel error on failure so the user sees it.
pub(crate) fn compile_grammar(ctx: &mut SteelCtx, src: String, out: String) -> SteelResult {
    let src_path = PathBuf::from(&src);
    let out_path = PathBuf::from(&out);

    ctx.log(
        LogLevel::Trace,
        format!("compile-grammar!: `tree-sitter build -o {out} {src}`"),
    );

    // `tree-sitter build` inherits stdio, so this is a real terminal write —
    // open the bracket first. Init-time compiles run pre-terminal (no screen
    // to enter, and no `#:inline-output` frame is ever armed there anyway).
    if ctx.session == crate::context::EvalSession::Runtime
        && let Some(output) = ctx.host.output()
    {
        output
            .ensure_inline_output_screen()
            .map_err(|e| generic_err(format!("compile-grammar!: {e}")))?;
    }

    let result = hume_platform::process::tree_sitter_build(&src_path, &out_path);
    let msg = match result {
        Ok(status) if status.success() => return Ok(SteelVal::Void),
        Ok(status) => format!(
            "compile-grammar!: `tree-sitter build` failed ({})",
            hume_platform::process::exit_code_str(status)
        ),
        Err(e) => format!("compile-grammar!: cannot run tree-sitter: {e}"),
    };
    // A failure with no compiler at all on PATH is a common, fixable cause on
    // Windows (no MSVC Build Tools) — point the user at it instead of leaving
    // them with a bare tree-sitter exit code.
    let msg = format!("{msg}{}", windows_compiler_hint());

    if ctx.session == crate::context::EvalSession::Init {
        ctx.log(LogLevel::Warning, msg);
        Ok(SteelVal::Void)
    } else {
        Err(generic_err(msg))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
