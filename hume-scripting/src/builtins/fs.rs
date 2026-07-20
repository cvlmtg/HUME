//! Editor-integration filesystem builtins for HUME's Steel scripting engine.
//!
//! Full-trust plugin model (see `docs/ROADMAP.md`'s plugin trust model
//! decision): general filesystem access goes through Steel's own
//! `steel/filesystem`/`steel/ports` stdlib, not a HUME builtin. What remains
//! here is editor-integration info Steel can't derive on its own, plus
//! `path-join`'s Windows UNC-prefix stripping (Steel's own
//! `canonicalize-path` leaks `\\?\`, which external tools reject).
//!
//! | Steel name      | Signature                      | Notes                                        |
//! |-----------------|--------------------------------|----------------------------------------------|
//! | `data-dir`      | `() → string \| #f`            | HUME data directory (XDG), or `#f` if unset  |
//! | `runtime-dir`   | `() → string \| #f`            | Runtime dir, or `#f` if absent               |
//! | `path-join`     | `string… → string`             | OS-native join; no sandbox, no filesystem access |

use std::path::{Path, PathBuf};

use steel::rerrs::SteelErr;
use steel::rvals::{IntoSteelVal, SteelVal};

use crate::SteelCtx;

use super::errors::generic_err;

// ── data-dir / runtime-dir ───────────────────────────────────────────────────

/// Shared body for `(data-dir)`/`(runtime-dir)`: return `dir`'s display-form
/// path as a string, or `#f` if `dir` is `None`.
fn dir_builtin(dir: Option<&Path>) -> Result<SteelVal, SteelErr> {
    match dir {
        Some(p) => p
            .to_string_lossy()
            .as_ref()
            .into_steelval()
            .map_err(generic_err),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(data-dir)` — returns the HUME data directory as a string, or `#f` if
/// HOME/APPDATA is unset.
///
/// The returned path is the display form (no `\\?\` extended-length prefix on
/// Windows) so Scheme plugins can safely join segments with `(path-join …)`
/// or, if necessary, plain string concatenation.
pub(crate) fn data_dir(ctx: &mut SteelCtx) -> Result<SteelVal, SteelErr> {
    dir_builtin(ctx.dirs.data_dir_display.as_deref())
}

/// `(runtime-dir)` — returns the HUME runtime directory as a string, or `#f`
/// if no runtime directory was found.
///
/// The returned path is the display form (no `\\?\` extended-length prefix on
/// Windows).
pub(crate) fn runtime_dir(ctx: &mut SteelCtx) -> Result<SteelVal, SteelErr> {
    dir_builtin(ctx.dirs.runtime_dir_display.as_deref())
}

// ── path-join ─────────────────────────────────────────────────────────────────

/// `(path-join seg1 seg2 …)` — join path segments using the OS-native
/// separator and return the result as a string.
///
/// Uses `PathBuf::push` semantics: if any segment is an absolute path it
/// replaces everything to the left (the same rule as `Path::join`).  This
/// lets plugins build paths portably without hard-coding `"/"` or `"\\"`.
///
/// No sandbox check — this is a pure string-construction helper that does not
/// access the filesystem.
pub(crate) fn path_join(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    if args.is_empty() {
        steel::stop!(ArityMismatch => "path-join expects at least 1 arg, got 0");
    }
    let mut result = PathBuf::new();
    for (i, arg) in args.iter().enumerate() {
        match arg {
            SteelVal::StringV(s) => result.push(s.as_str()),
            _ => steel::stop!(TypeMismatch =>
                "path-join: arg {} must be a string, got {:?}", i, arg),
        }
    }
    result
        .to_string_lossy()
        .as_ref()
        .into_steelval()
        .map_err(generic_err)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::dirs::ScriptDirs;
    use crate::test_support::SteelCtxTestHarness;
    use tempfile::TempDir;

    fn harness_with_data_dir(tmp: &TempDir) -> SteelCtxTestHarness {
        let data_dir = tmp.path().join("hume");
        std::fs::create_dir_all(&data_dir).unwrap();
        let mut h = SteelCtxTestHarness::new();
        h.dirs = ScriptDirs::new(Some(data_dir), None);
        h
    }

    // ── path-join ────────────────────────────────────────────────────────────

    #[test]
    fn path_join_two_segments() {
        let args = vec![
            SteelVal::StringV("foo".into()),
            SteelVal::StringV("bar".into()),
        ];
        let result = path_join(&args).unwrap();
        let s = match result {
            SteelVal::StringV(s) => s.to_string(),
            other => panic!("expected string, got {other:?}"),
        };
        // The joined path must contain both components separated by the OS separator.
        let expected = std::path::PathBuf::from("foo").join("bar");
        assert_eq!(s, expected.to_string_lossy().as_ref());
    }

    #[test]
    fn path_join_single_segment() {
        let args = vec![SteelVal::StringV("only".into())];
        let result = path_join(&args).unwrap();
        assert!(matches!(result, SteelVal::StringV(s) if s.as_str() == "only"));
    }

    #[test]
    fn path_join_no_args_errors() {
        assert!(path_join(&[]).is_err());
    }

    #[test]
    fn path_join_type_error() {
        let args = vec![SteelVal::IntV(42)];
        assert!(path_join(&args).is_err());
    }

    // ── data-dir display (no UNC prefix) ─────────────────────────────────────

    /// On all platforms `(data-dir)` must return a string that does not begin
    /// with the Windows extended-length prefix `\\?\`.  On Unix this prefix
    /// never appears, so the test is platform-neutral.
    #[test]
    fn data_dir_no_unc_prefix() {
        let tmp = TempDir::new().unwrap();
        let mut h = harness_with_data_dir(&tmp);
        let mut ctx = h.ctx();

        let result = data_dir(&mut ctx).unwrap();
        let s = match result {
            SteelVal::StringV(s) => s.to_string(),
            other => panic!("expected string, got {other:?}"),
        };
        assert!(
            !s.starts_with(r"\\?\"),
            "data-dir must not return an extended-length UNC path, got: {s}"
        );
    }

    /// End-to-end through the real registration table (`builtins::mod`'s
    /// `builtins!` table registers `data-dir` as a ctx-injected `open`
    /// builtin) — proves `(data-dir)` resolves to `ctx.dirs` through that path.
    /// `eval_source` only reports success/failure, not a return value, so
    /// the result is round-tripped through `log!` and read back from the
    /// message log.
    #[test]
    fn data_dir_resolves_through_real_registration() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("hume");
        std::fs::create_dir_all(&data_dir).unwrap();

        let mut host = crate::ScriptingHost::new();
        host.set_data_dir(data_dir.clone());
        let mut null_host = crate::null_host::NullHost;
        host.eval_source(r#"(log! 'info (data-dir))"#, &mut null_host)
            .expect("(data-dir) must evaluate through the real registration table");

        // macOS TempDir paths are under /var, which is itself a symlink to
        // /private/var — canonicalize the *expected* side so the comparison
        // isn't platform-dependent (see memory feedback_macos_tempfile_canonicalize).
        // On Windows canonicalize yields a `\\?\`-prefixed path, but
        // `(data-dir)` returns the display form — strip the prefix to match.
        let expected =
            hume_platform::path::strip_unc_prefix(std::fs::canonicalize(&data_dir).unwrap());
        let msgs = host.take_pending_messages();
        assert!(
            msgs.iter()
                .any(|(_, msg)| msg == &expected.to_string_lossy()),
            "expected a log message equal to {:?}, got: {:?}",
            expected,
            msgs
        );
    }
}
