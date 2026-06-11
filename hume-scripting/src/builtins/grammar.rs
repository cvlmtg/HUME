//! Grammar compilation builtins for HUME's Steel scripting engine.
//!
//! | Steel name             | Signature           | Notes                          |
//! |------------------------|---------------------|--------------------------------|
//! | `grammar-output-path`  | `string → string`      | `<data>/grammars/<name>.<ext>` |
//! | `compile-grammar!`     | `string string → void` | `tree-sitter build -o out src` |

use std::path::PathBuf;

use steel::rerrs::SteelErr;
use steel::rvals::{IntoSteelVal, SteelVal};

use crate::log::LogLevel;
use crate::SteelCtx;

/// Platform-specific shared library extension for tree-sitter grammars.
fn platform_grammar_ext() -> &'static str {
    #[cfg(target_os = "macos")]
    { "dylib" }
    #[cfg(target_os = "windows")]
    { "dll" }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { "so" }
}

/// `(grammar-output-path name)` — return the output path for a compiled grammar:
/// `<data>/grammars/<name>.<platform-ext>`.
///
/// `name` must be a single normal path component (no `..`, no separators).
/// Safe in both init and command mode — does not touch the filesystem.
pub(crate) fn grammar_output_path(
    _ctx: &mut SteelCtx,
    name: String,
) -> Result<SteelVal, SteelErr> {
    let filename = format!("{}.{}", name, platform_grammar_ext());
    let path = super::sandbox::with_data_grammars_or_subpath(&filename, |p| p.to_path_buf())?;
    path.to_string_lossy()
        .as_ref()
        .into_steelval()
        .map_err(|e| {
            SteelErr::new(steel::rerrs::ErrorKind::ConversionError, e.to_string())
        })
}

/// `(compile-grammar! src out)` — compile the tree-sitter grammar source at
/// `src` to `out` via `tree-sitter build -o <out> <src>`.
///
/// Both paths must resolve inside `<data>/grammars/`.
///
/// - **Init mode**: logs a Warning on failure and returns `#<void>` — a
///   missing `tree-sitter` binary should not abort the editor on startup.
/// - **Command mode**: raises a Steel error on failure so the user sees it.
pub(crate) fn compile_grammar(
    ctx: &mut SteelCtx,
    src: String,
    out: String,
) -> Result<SteelVal, SteelErr> {
    let src_path = PathBuf::from(&src);
    let out_path = PathBuf::from(&out);

    if super::sandbox::has_dotdot(&src_path) {
        steel::stop!(Generic =>
            "compile-grammar!: src must not contain '..' components: {}", src);
    }
    // validate_new_path returns the canonical dest — use it for the subprocess
    // so the same resolved path is used for both the check and the spawn.
    let canonical_out =
        super::shell::validate_new_path(&out_path, "compile-grammar!", super::shell::SandboxKind::Grammars)?;

    // Sandbox-check src (must exist → full canonicalize).
    let canonical_src = hume_platform::fs::canonicalize(&src_path).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("compile-grammar!: cannot resolve src '{src}': {e}"),
        )
    })?;
    super::sandbox::with_data_grammars(|sandbox| {
        if !canonical_src.starts_with(sandbox) {
            Err(SteelErr::new(
                steel::rerrs::ErrorKind::Generic,
                format!("compile-grammar!: src '{src}' is outside the grammars sandbox"),
            ))
        } else {
            Ok(())
        }
    })??;

    ctx.log(
        LogLevel::Trace,
        format!("compile-grammar!: `tree-sitter build -o {out} {src}`"),
    );

    let result = hume_platform::process::tree_sitter_build(&canonical_src, &canonical_out);
    let msg = match result {
        Ok(status) if status.success() => return Ok(SteelVal::Void),
        Ok(status) => format!(
            "compile-grammar!: `tree-sitter build` failed ({})",
            hume_platform::process::exit_code_str(status)
        ),
        Err(e) => format!("compile-grammar!: cannot run tree-sitter: {e}"),
    };

    if ctx.is_init {
        ctx.log(LogLevel::Warning, msg);
        Ok(SteelVal::Void)
    } else {
        Err(SteelErr::new(steel::rerrs::ErrorKind::Generic, msg))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::SteelCtxTestHarness;
    use std::fs;
    use tempfile::TempDir;

    fn setup(tmp: &TempDir) {
        let data_dir = tmp.path().join("hume");
        fs::create_dir_all(data_dir.join("plugins")).unwrap();
        fs::create_dir_all(data_dir.join("grammars/sources")).unwrap();
        super::super::sandbox::init_dirs(Some(data_dir), None);
    }

    // ── grammar-output-path ───────────────────────────────────────────────────

    /// Flip: if grammar-output-path returned a path outside grammars/, the prefix
    /// check in compile-grammar! would catch it and error — but path construction
    /// must be correct so compile-grammar! can use it directly.
    #[test]
    #[cfg(not(windows))]
    fn grammar_output_path_returns_grammars_subpath() {
        use std::env;
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("hume");
        fs::create_dir_all(data_dir.join("grammars/sources")).unwrap();
        unsafe { env::set_var("XDG_DATA_HOME", tmp.path()) };
        super::super::sandbox::init_dirs(Some(data_dir), None);

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = grammar_output_path(&mut ctx, "json".to_string()).unwrap();
        let s = match result {
            SteelVal::StringV(s) => s.to_string(),
            other => panic!("expected string, got {other:?}"),
        };
        let ext = platform_grammar_ext();
        assert!(
            s.ends_with(&format!("json.{ext}")),
            "output path must end with json.{ext}, got: {s}"
        );
        assert!(
            s.contains("grammars"),
            "output path must be inside grammars dir, got: {s}"
        );
    }

    // ── compile-grammar! sandbox checks ──────────────────────────────────────

    /// Flip: if sandbox checks were removed, a path traversal to outside grammars/
    /// would not be caught — this test exercises the src-side check.
    #[test]
    fn compile_grammar_rejects_dotdot_in_src() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        let data_dir = tmp.path().join("hume");
        let evil_src = format!("{}/grammars/sources/json/../../..", data_dir.display());
        let valid_out = format!("{}/grammars/json.dylib", data_dir.display());

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = compile_grammar(&mut ctx, evil_src, valid_out).unwrap_err();
        assert!(
            err.to_string().contains(".."),
            "expected .. rejection, got: {err}"
        );
    }

    #[test]
    fn compile_grammar_rejects_src_outside_grammars_sandbox() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        // src points to plugins/ — wrong sandbox
        let data_dir = tmp.path().join("hume");
        let bad_src = format!("{}/plugins/repo", data_dir.display());
        fs::create_dir_all(&bad_src).unwrap();
        let valid_out = format!("{}/grammars/json.dylib", data_dir.display());

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = compile_grammar(&mut ctx, bad_src, valid_out).unwrap_err();
        assert!(
            err.to_string().contains("grammars sandbox") || err.to_string().contains("outside"),
            "expected sandbox error, got: {err}"
        );
    }

    #[test]
    fn compile_grammar_rejects_out_outside_grammars_sandbox() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        // src is valid; out points outside grammars/
        let data_dir = tmp.path().join("hume");
        let valid_src = format!("{}/grammars/sources/json", data_dir.display());
        fs::create_dir_all(&valid_src).unwrap();
        let bad_out = format!("{}/evil.dylib", tmp.path().display());

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = compile_grammar(&mut ctx, valid_src, bad_out).unwrap_err();
        assert!(
            err.to_string().contains("grammars sandbox")
                || err.to_string().contains("outside")
                || err.to_string().contains("cannot resolve"),
            "expected sandbox error, got: {err}"
        );
    }

}
