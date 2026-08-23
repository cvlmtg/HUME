//! Editor-integration filesystem builtins for HUME's Steel scripting engine.
//!
//! Full-trust plugin model (see `user-manual/docs/plugins.md`'s "Filesystem
//! and processes"): general filesystem access goes through Steel's own
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
//! | `path->display` | `string → string`               | UNC-strip + `~`-collapse; no filesystem access |
//!
//! `path-separator` is *not* registered here — steel-core's `steel/meta`
//! module already provides it as a bare global (`Engine::new()` baseline),
//! and HUME registering its own would silently shadow it.

use std::path::{Path, PathBuf};

use steel::rvals::{IntoSteelVal, SteelVal};

use crate::SteelCtx;

use super::SteelResult;
use super::errors::generic_err;

// ── data-dir / runtime-dir ───────────────────────────────────────────────────

/// Shared body for `(data-dir)`/`(runtime-dir)`: return `dir`'s display-form
/// path as a string, or `#f` if `dir` is `None`.
fn dir_builtin(dir: Option<&Path>) -> SteelResult {
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
pub(crate) fn data_dir(ctx: &mut SteelCtx) -> SteelResult {
    dir_builtin(ctx.dirs.data_dir_display.as_deref())
}

/// `(runtime-dir)` — returns the HUME runtime directory as a string, or `#f`
/// if no runtime directory was found.
///
/// The returned path is the display form (no `\\?\` extended-length prefix on
/// Windows).
pub(crate) fn runtime_dir(ctx: &mut SteelCtx) -> SteelResult {
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
pub(crate) fn path_join(args: &[SteelVal]) -> SteelResult {
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

// ── path->display ─────────────────────────────────────────────────────────────

/// `(path->display path)` — run `path` through HUME's display-form pipeline
/// (Windows `\\?\` stripping, `~`-collapse) so a path a plugin only has as a
/// string (e.g. `lsp/location-display`'s `path->display` call on the
/// already-decoded path `lsp-locations->display-parts` hands it) renders the
/// same way as paths that came from `buffer-display-path`. No filesystem
/// access; expects an absolute path — `~`-collapse is a no-op on relative
/// input.
pub(crate) fn path_to_display(args: &[SteelVal]) -> SteelResult {
    if args.len() != 1 {
        steel::stop!(ArityMismatch => "path->display expects exactly 1 arg, got {}", args.len());
    }
    let SteelVal::StringV(s) = &args[0] else {
        steel::stop!(TypeMismatch => "path->display: arg must be a string, got {:?}", args[0]);
    };
    hume_platform::path::display_form(Path::new(s.as_str()))
        .into_steelval()
        .map_err(generic_err)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
