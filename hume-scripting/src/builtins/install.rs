//! LSP server install pipeline builtins: platform identification, sha256
//! hashing, archive unpacking, and a cross-process install lock.
//!
//! No sandbox checks — full-trust plugin model (see
//! `user-manual/docs/plugins.md`'s "Filesystem and processes").
//!
//! sha256 hashing and archive unpacking shell out to per-platform system
//! tools (`hume_platform::process`) rather than pulling in hashing/archive
//! crates: `shasum`/`sha256sum`/`certutil` for hashing, `gzip` for `.gz`,
//! `unzip`/`tar` for `.zip` — the OS/toolchain already ships all of these,
//! so it costs no new install step in the common case, at the price of a
//! hard runtime dependency on them being present.
//!
//! | Steel name                     | Signature              | Notes                              |
//! |---------------------------------|------------------------|-------------------------------------|
//! | `hume-target`                   | `() → string \| #f`    | install-target id, or `#f`         |
//! | `sha256-file`                   | `string → string`      | sha256 digest as lowercase hex     |
//! | `unpack-gz`                     | `string string → void` | `gzip -dc`, chmod 0755 on Unix     |
//! | `unpack-zip`                    | `string string string → void` | `unzip`/`tar`, bin-path chmod'd |
//! | `acquire-install-lock!`         | `() → void`            | O_EXCL over `<data>/servers/.install-lock`; stale (>1h) → replace |
//! | `release-install-lock!`        | `() → void`            | idempotent — a missing lock is not an error |
//! | `%run-inline-output!`           | `string list string|#f → int` | process-group-isolated spawn for `#:inline-output` commands; see `run_inline_output` doc |

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use steel::rvals::{IntoSteelVal, SteelVal};

use crate::SteelCtx;
use crate::log::LogLevel;

use super::SteelResult;
use super::args::{list_to_strings, optional_path_arg};
use super::errors::generic_err;

const INSTALL_LOCK_FILE_NAME: &str = ".install-lock";

/// A lock file older than this is treated as abandoned by a crashed or
/// killed prior process — replaced with a warning, rather than left to
/// block every future install/uninstall forever.
const STALE_INSTALL_LOCK_AGE: Duration = Duration::from_secs(60 * 60);

/// `(hume-target)` — the install-target identifier for the current platform
/// (`"darwin-arm64"`, `"darwin-x64"`, `"linux-x64"`, `"windows-x64"`), or
/// `#f` on any other platform/architecture. `#f`, not an error, so
/// `:lsp-servers` can render "unsupported platform" rather than aborting.
pub(crate) fn hume_target(args: &[SteelVal]) -> SteelResult {
    if !args.is_empty() {
        steel::stop!(ArityMismatch => "hume-target expects 0 args, got {}", args.len());
    }
    Ok(match hume_platform::target::hume_target() {
        Some(t) => SteelVal::StringV(t.into()),
        None => SteelVal::BoolV(false),
    })
}

/// `(sha256-file path)` — the sha256 digest of `path` as lowercase hex.
///
/// No sandbox check and no compare/delete logic — full-trust plugin model
/// (see `user-manual/docs/plugins.md`'s "Filesystem and processes"). Compare-and-delete-
/// on-mismatch lives in Scheme (`lsp/verify-sha256!` in `servers.scm`) —
/// this is a thin wrapper over the platform tool selection (`shasum`/
/// `sha256sum`/`certutil`) that a Scheme rewrite would only make worse.
pub(crate) fn sha256_file(ctx: &mut SteelCtx, path: String) -> SteelResult {
    ctx.log(LogLevel::Trace, format!("sha256-file: hashing {path}"));
    let digest = hume_platform::process::sha256_file(Path::new(&path))
        .map_err(|e| generic_err(format!("sha256-file: cannot hash '{path}': {e}")))?;
    digest.into_steelval().map_err(generic_err)
}

/// `(unpack-gz src dest)` — decode the single-file gzip archive at `src`
/// into `dest` (shells out to `gzip -dc`; on Unix, `dest` is chmod'd `0o755`
/// after success — Mason `.gz` assets are bare server executables).
///
/// On error, any partial `dest` is removed before raising.
pub(crate) fn unpack_gz(ctx: &mut SteelCtx, src: String, dest: String) -> SteelResult {
    let src_path = PathBuf::from(&src);
    let dest_path = PathBuf::from(&dest);

    ctx.log(LogLevel::Trace, format!("unpack-gz: {src} → {dest}"));

    if let Err(e) = hume_platform::process::unpack_gz(&src_path, &dest_path) {
        let _ = std::fs::remove_file(&dest_path);
        steel::stop!(Generic => "unpack-gz: {}", e);
    }
    Ok(SteelVal::Void)
}

/// `(unpack-zip src dest-dir bin-path)` — extract the zip archive at `src`
/// into `dest-dir` (`unzip -o` on Unix, `tar -xf` on Windows), then verify
/// `bin-path` (relative to `dest-dir`) exists and — on Unix — chmod it
/// `0o755`. Unlike `.gz` (always a bare executable, chmod'd unconditionally),
/// zip entries carry the archive's own stored permissions and CI-built
/// release zips routinely strip the exec bit.
///
/// Zip-slip and symlink-entry protection is delegated to the system tool
/// (modern Info-ZIP strips `../` entries; bsdtar refuses them by default) —
/// the residual risk is bounded by the sha256 pin: this runs only after
/// `lsp/verify-sha256!` has confirmed the archive matches the maintainer-
/// vetted, hash-locked asset recorded in `lsp-sources.scm`, so an attacker
/// would need to compromise the pinned upstream release itself, not just
/// something interposed at install time. `dest-dir` is created if
/// absent (`tar -C` requires an existing directory). On error, `dest-dir` is
/// left as-is — a dir-without-receipt is already the interrupted-install
/// signal the installer relies on, so cleaning up here would duplicate that
/// mechanism.
pub(crate) fn unpack_zip(
    ctx: &mut SteelCtx,
    src: String,
    dest_dir: String,
    bin_path: String,
) -> SteelResult {
    let src_path = PathBuf::from(&src);
    let dest_path = PathBuf::from(&dest_dir);
    std::fs::create_dir_all(&dest_path).map_err(|e| {
        generic_err(format!(
            "unpack-zip: cannot create dest dir '{dest_dir}': {e}"
        ))
    })?;

    ctx.log(
        LogLevel::Trace,
        format!("unpack-zip: {src} → {dest_dir} (bin: {bin_path})"),
    );

    // `unzip`/`tar` inherit stdio (see `hume_platform::process::unpack_zip`'s
    // doc), so this is a real terminal write — open the bracket first.
    if let Some(output) = ctx.host.output() {
        output
            .ensure_inline_output_screen()
            .map_err(|e| generic_err(format!("unpack-zip: {e}")))?;
    }
    hume_platform::process::unpack_zip(&src_path, &dest_path, Path::new(&bin_path))
        .map_err(|e| generic_err(format!("unpack-zip: {e}")))?;
    Ok(SteelVal::Void)
}

/// Create `path` with O_EXCL semantics: errors with `AlreadyExists` if it's
/// already there. Content doesn't matter, only existence + mtime.
fn create_lock_file(path: &Path) -> std::io::Result<()> {
    OpenOptions::new().write(true).create_new(true).open(path)?;
    Ok(())
}

/// `(acquire-install-lock!)` — create `<data>/servers/.install-lock` with
/// O_EXCL semantics, guarding `:lsp-install`/`:lsp-uninstall` against a
/// second HUME process running one of them concurrently. A lock already
/// present and *provably* older than an hour is treated as abandoned (the
/// process that held it crashed or was killed without releasing) and
/// replaced, with a warning. Everything else — younger than an hour, or an
/// age that can't be positively determined at all (unreadable metadata, or
/// a future mtime from clock skew / a networked or synced filesystem) —
/// is treated as live: deleting a lock we can't prove abandoned risks two
/// installs racing on the same server directory.
///
/// # Errors
/// A live (or indeterminate-age) lock already exists, or the lock file
/// can't be created/replaced.
pub(crate) fn acquire_install_lock(ctx: &mut SteelCtx) -> SteelResult {
    let lock_path = ctx.dirs.servers_dir()?.join(INSTALL_LOCK_FILE_NAME);
    let Err(create_err) = create_lock_file(&lock_path) else {
        return Ok(SteelVal::Void);
    };
    if create_err.kind() != std::io::ErrorKind::AlreadyExists {
        steel::stop!(Generic => "acquire-install-lock!: {}", create_err);
    }
    // `duration_since` errors (rather than defaulting to "unknown") on a
    // future mtime — clock skew or a networked/synced filesystem — which
    // is exactly the case that must NOT be treated as stale.
    let is_stale = std::fs::metadata(&lock_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age > STALE_INSTALL_LOCK_AGE);
    if !is_stale {
        steel::stop!(Generic =>
            "acquire-install-lock!: another install/uninstall is already in progress");
    }
    ctx.log(
        LogLevel::Warning,
        "acquire-install-lock!: stale lock (older than 1h) — replacing".to_string(),
    );
    std::fs::remove_file(&lock_path).map_err(|e| {
        generic_err(format!(
            "acquire-install-lock!: cannot remove stale lock: {e}"
        ))
    })?;
    create_lock_file(&lock_path).map_err(|e| {
        generic_err(format!(
            "acquire-install-lock!: cannot create lock after removing stale one: {e}"
        ))
    })?;
    Ok(SteelVal::Void)
}

/// `(%run-inline-output! cmd args cwd)` — spawn `cmd` with `args` (a list of
/// strings), inherited stdio, in its own process group; blocks until exit and
/// returns the exit code as an int. `cwd` is a string or `#f`.
///
/// The process-group isolation is the entire reason this is a Rust builtin
/// rather than Steel's own `spawn-process`: `#:inline-output` commands run
/// with terminal raw mode off (see `run_inline_output`'s doc comment in
/// `hume-platform::process`), so an unisolated child would be killed by the
/// same Ctrl+C that's meant to interrupt only it. No sandbox checks — plugins
/// are trusted code (see `user-manual/docs/plugins.md`'s "Filesystem and processes").
///
/// # Errors
/// The binary can't be spawned (e.g. not found on `PATH`).
pub(crate) fn run_inline_output(
    ctx: &mut SteelCtx,
    cmd: String,
    args_val: SteelVal,
    cwd_val: SteelVal,
) -> SteelResult {
    let args = list_to_strings(args_val, "%run-inline-output! args")?;
    let cwd = optional_path_arg(cwd_val, "%run-inline-output! cwd")?;

    // The child inherits stdio, so this is a real terminal write — open the
    // bracket before spawning it.
    if let Some(output) = ctx.host.output() {
        output
            .ensure_inline_output_screen()
            .map_err(|e| generic_err(format!("run-inline-output!: {e}")))?;
    }

    let status = hume_platform::process::run_inline_output(&cmd, &args, cwd.as_deref())
        .map_err(|e| generic_err(format!("run-inline-output!: cannot run '{cmd}': {e}")))?;

    // `-1` for a signal-killed child (no exit code) — matches the sentinel a
    // real exit code can never produce, since process exit codes are u8-wide.
    Ok(SteelVal::IntV(status.code().unwrap_or(-1) as isize))
}

/// `(release-install-lock!)` — remove `<data>/servers/.install-lock`.
/// Idempotent: a missing lock (already released, or never acquired) is not
/// an error.
pub(crate) fn release_install_lock(ctx: &mut SteelCtx) -> SteelResult {
    let lock_path = ctx.dirs.servers_dir()?.join(INSTALL_LOCK_FILE_NAME);
    match std::fs::remove_file(&lock_path) {
        Ok(()) => Ok(SteelVal::Void),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SteelVal::Void),
        Err(e) => Err(generic_err(format!("release-install-lock!: {e}"))),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
