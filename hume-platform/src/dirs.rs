//! Platform-aware base directory resolution for HUME.
//!
//! Follows XDG Base Directory conventions on Unix and macOS. On Windows:
//! - Config lives under `%APPDATA%\hume\` (Roaming — syncs across domain machines,
//!   appropriate for user-edited config files).
//! - Data lives under `%LOCALAPPDATA%\hume\` (Local — machine-specific, appropriate
//!   for plugin binaries and caches that must not roam between machines).
//!
//! All resolvers return `Option<PathBuf>`: `None` means the platform-specific
//! env vars are unset (no silent fallback to `.config/hume` or `.local/share/hume`).
//! Callers decide how to handle the missing directory — PLUM disables itself,
//! `init.scm` loading is skipped, etc. Fail-fast over silent-wrong.

use std::{
    env,
    path::{Path, PathBuf},
};

fn env_var(key: &str) -> Option<String> {
    env::var(key).ok()
}

/// Returns the configuration directory for HUME, if it can be resolved.
///
/// - Unix / macOS: `$XDG_CONFIG_HOME/hume/` → `$HOME/.config/hume/`
/// - Windows: `%APPDATA%\hume\`
///
/// Returns `None` only if both the relevant env vars are unset (`HOME` on
/// Unix, `APPDATA` on Windows). Callers should report and skip scripting
/// init rather than fall back to a relative path.
pub fn config_dir() -> Option<PathBuf> {
    config_dir_with(env_var)
}

#[cfg(windows)]
fn config_dir_with(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    env("APPDATA").map(|base| PathBuf::from(base).join("hume"))
}

#[cfg(not(windows))]
fn config_dir_with(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(xdg) = env("XDG_CONFIG_HOME") {
        Some(PathBuf::from(xdg).join("hume"))
    } else {
        env("HOME").map(|h| PathBuf::from(h).join(".config").join("hume"))
    }
}

/// Returns the data directory for HUME, if it can be resolved.
///
/// - Unix / macOS: `$XDG_DATA_HOME/hume/` → `$HOME/.local/share/hume/`
/// - Windows: `%LOCALAPPDATA%\hume\` (falls back to `%APPDATA%\hume\` if `LOCALAPPDATA` is unset)
///
/// Returns `None` only if the relevant env vars are unset. Callers should
/// disable features that need on-disk storage (PLUM install, user plugins).
pub fn data_dir() -> Option<PathBuf> {
    data_dir_with(env_var)
}

#[cfg(windows)]
fn data_dir_with(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    // Prefer LOCALAPPDATA (machine-local) for plugin binaries and caches;
    // roaming them via APPDATA across domain machines risks arch mismatches
    // and stale paths. Fall back to APPDATA in environments where LOCALAPPDATA
    // is not populated (stripped CI images, some service accounts).
    env("LOCALAPPDATA")
        .or_else(|| env("APPDATA"))
        .map(|base| PathBuf::from(base).join("hume"))
}

#[cfg(not(windows))]
fn data_dir_with(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(xdg) = env("XDG_DATA_HOME") {
        Some(PathBuf::from(xdg).join("hume"))
    } else {
        env("HOME").map(|h| PathBuf::from(h).join(".local").join("share").join("hume"))
    }
}

/// Returns the current user's home directory, if it can be resolved.
///
/// - Unix / macOS: `$HOME`
/// - Windows: `%USERPROFILE%`, falling back to `%HOMEDRIVE%%HOMEPATH%`
///
/// Returns `None` if the relevant env vars are unset.  Callers that need a
/// home directory for path expansion should treat `None` as "leave literal"
/// rather than silently falling back to a relative path.
pub fn home_dir() -> Option<PathBuf> {
    home_dir_with(env_var)
}

#[cfg(not(windows))]
fn home_dir_with(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    env("HOME").map(PathBuf::from)
}

#[cfg(windows)]
fn home_dir_with(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    // USERPROFILE is set in every modern Windows session and is the canonical
    // home directory.  HOMEDRIVE+HOMEPATH is the legacy fallback used by older
    // tools and stripped service-account environments.
    env("USERPROFILE")
        .or_else(|| match (env("HOMEDRIVE"), env("HOMEPATH")) {
            (Some(d), Some(p)) => Some(format!("{d}{p}")),
            _ => None,
        })
        .map(PathBuf::from)
}

/// Returns the runtime directory for HUME, if one can be found.
///
/// Search order:
/// 1. `HUME_RUNTIME` environment variable (dev escape hatch; not existence-checked).
/// 2. `../share/hume/` relative to the binary, Unix / macOS installed (FHS) layout.
/// 3. `runtime/` relative to the binary — portable/archive layout (nightly zips/tarballs
///    ship `hume(.exe)` next to a `runtime/` directory; this covers Windows, where there
///    is no `share/` layout, and any unpacked archive run in place on Unix).
/// 4. `./runtime` relative to cwd — dev fallback when running with `cargo run` from the
///    workspace root.
///
/// Every candidate except (1) is existence-checked; returns `None` if none exist.
pub fn runtime_dir() -> Option<PathBuf> {
    runtime_dir_with(env_var, env::current_exe().ok(), env::current_dir().ok())
}

fn runtime_dir_with(
    env: impl Fn(&str) -> Option<String>,
    exe: Option<PathBuf>,
    cwd: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(rt) = env("HUME_RUNTIME") {
        return Some(PathBuf::from(rt));
    }

    if let Some(exe_dir) = exe.as_deref().and_then(Path::parent) {
        #[cfg(not(windows))]
        if let Some(share) = exe_dir.parent().map(|p| p.join("share").join("hume"))
            && share.exists()
        {
            return Some(share);
        }
        let exe_runtime = exe_dir.join("runtime");
        if exe_runtime.exists() {
            return Some(exe_runtime);
        }
    }

    if let Some(cwd_runtime) = cwd.map(|c| c.join("runtime"))
        && cwd_runtime.exists()
    {
        return Some(cwd_runtime);
    }

    None
}

#[cfg(test)]
mod tests;
