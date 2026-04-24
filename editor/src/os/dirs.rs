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

use std::{env, path::PathBuf};

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
pub(crate) fn config_dir() -> Option<PathBuf> {
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
pub(crate) fn data_dir() -> Option<PathBuf> {
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
pub(crate) fn home_dir() -> Option<PathBuf> {
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
/// Search order (STEEL.md §"Runtime directory discovery"):
/// 1. `HUME_RUNTIME` environment variable (development escape hatch).
/// 2. `../share/hume/` relative to the binary on Unix / macOS.
/// 3. Same directory as the binary on Windows.
/// 4. `./runtime` relative to cwd (dev fallback when running with `cargo run`).
///
/// Returns `None` if no candidate path exists on disk.
pub(crate) fn runtime_dir() -> Option<PathBuf> {
    runtime_dir_with(env_var)
}

fn runtime_dir_with(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(rt) = env("HUME_RUNTIME") {
        return Some(PathBuf::from(rt));
    }

    let exe = env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    #[cfg(windows)]
    {
        Some(exe_dir.to_path_buf())
    }
    #[cfg(not(windows))]
    {
        // Installed layout: .../bin/hume → .../share/hume/
        let share = exe_dir.parent()?.join("share").join("hume");
        if share.exists() {
            return Some(share);
        }
        // Dev layout: cargo run produces target/…/hume; runtime/ sits at workspace root.
        let cwd_rt = env::current_dir().ok()?.join("runtime");
        if cwd_rt.exists() { Some(cwd_rt) } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(windows))]
    fn config_dir_respects_xdg_config_home() {
        let tmp = tempfile::tempdir().unwrap();
        let xdg = tmp.path().to_string_lossy().into_owned();
        let result = config_dir_with(|k| match k {
            "XDG_CONFIG_HOME" => Some(xdg.clone()),
            _ => None,
        });
        assert_eq!(result, Some(tmp.path().join("hume")));
    }

    #[test]
    #[cfg(not(windows))]
    fn data_dir_respects_xdg_data_home() {
        let tmp = tempfile::tempdir().unwrap();
        let xdg = tmp.path().to_string_lossy().into_owned();
        let result = data_dir_with(|k| match k {
            "XDG_DATA_HOME" => Some(xdg.clone()),
            _ => None,
        });
        assert_eq!(result, Some(tmp.path().join("hume")));
    }

    #[test]
    fn runtime_dir_respects_hume_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = tmp.path().to_string_lossy().into_owned();
        let result = runtime_dir_with(|k| match k {
            "HUME_RUNTIME" => Some(rt.clone()),
            _ => None,
        });
        assert_eq!(result, Some(tmp.path().to_path_buf()));
    }

    #[test]
    #[cfg(not(windows))]
    fn home_dir_uses_home_env() {
        let result = home_dir_with(|k| match k {
            "HOME" => Some("/home/alice".to_owned()),
            _ => None,
        });
        assert_eq!(result, Some(PathBuf::from("/home/alice")));
    }

    #[test]
    #[cfg(not(windows))]
    fn home_dir_none_when_home_unset() {
        let result = home_dir_with(|_| None);
        assert_eq!(result, None);
    }

    #[test]
    #[cfg(windows)]
    fn home_dir_uses_userprofile() {
        let result = home_dir_with(|k| match k {
            "USERPROFILE" => Some(r"C:\Users\Alice".to_owned()),
            _ => None,
        });
        assert_eq!(result, Some(PathBuf::from(r"C:\Users\Alice")));
    }

    #[test]
    #[cfg(windows)]
    fn home_dir_falls_back_to_homedrive_homepath() {
        let result = home_dir_with(|k| match k {
            "HOMEDRIVE" => Some("C:".to_owned()),
            "HOMEPATH" => Some(r"\Users\Alice".to_owned()),
            _ => None,
        });
        assert_eq!(result, Some(PathBuf::from(r"C:\Users\Alice")));
    }

    #[test]
    #[cfg(windows)]
    fn home_dir_none_when_all_unset() {
        let result = home_dir_with(|_| None);
        assert_eq!(result, None);
    }
}
