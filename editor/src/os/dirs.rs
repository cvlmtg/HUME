//! Platform-aware base directory resolution for HUME.
//!
//! Follows XDG Base Directory conventions on Unix and macOS, and uses
//! `%APPDATA%\hume\` on Windows native. All three roots are documented
//! in STEEL.md §"Three root directories".
//!
//! All resolvers return `Option<PathBuf>`: `None` means the platform-specific
//! env vars are unset (no silent fallback to `.config/hume` or `.local/share/hume`).
//! Callers decide how to handle the missing directory — PLUM disables itself,
//! `init.scm` loading is skipped, etc. Fail-fast over silent-wrong.

use std::{env, path::PathBuf};

/// Returns the configuration directory for HUME, if it can be resolved.
///
/// - Unix / macOS: `$XDG_CONFIG_HOME/hume/` → `$HOME/.config/hume/`
/// - Windows: `%APPDATA%\hume\config\`
///
/// Returns `None` only if both the relevant env vars are unset (`HOME` on
/// Unix, `APPDATA` on Windows). Callers should report and skip scripting
/// init rather than fall back to a relative path.
pub(crate) fn config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env::var("APPDATA").ok().map(|base| PathBuf::from(base).join("hume").join("config"))
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
            Some(PathBuf::from(xdg).join("hume"))
        } else if let Ok(home) = env::var("HOME") {
            Some(PathBuf::from(home).join(".config").join("hume"))
        } else {
            None
        }
    }
}

/// Returns the data directory for HUME, if it can be resolved.
///
/// - Unix / macOS: `$XDG_DATA_HOME/hume/` → `$HOME/.local/share/hume/`
/// - Windows: `%APPDATA%\hume\data\`
///
/// Returns `None` only if the relevant env vars are unset. Callers should
/// disable features that need on-disk storage (PLUM install, user plugins).
pub(crate) fn data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env::var("APPDATA").ok().map(|base| PathBuf::from(base).join("hume").join("data"))
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = env::var("XDG_DATA_HOME") {
            Some(PathBuf::from(xdg).join("hume"))
        } else if let Ok(home) = env::var("HOME") {
            Some(PathBuf::from(home).join(".local").join("share").join("hume"))
        } else {
            None
        }
    }
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
    if let Ok(rt) = env::var("HUME_RUNTIME") {
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
    use std::env;

    #[test]
    fn config_dir_respects_xdg_config_home() {
        // Temporarily set XDG_CONFIG_HOME.
        // Test-only env mutation; tests run sequentially in this module.
        let tmp = tempfile::tempdir().unwrap();
        let prev = env::var("XDG_CONFIG_HOME").ok();
        unsafe { env::set_var("XDG_CONFIG_HOME", tmp.path()); }

        let result = config_dir();
        assert_eq!(result, Some(tmp.path().join("hume")));

        unsafe {
            match prev {
                Some(v) => env::set_var("XDG_CONFIG_HOME", v),
                None => env::remove_var("XDG_CONFIG_HOME"),
            }
        }
    }

    #[test]
    fn data_dir_respects_xdg_data_home() {
        let tmp = tempfile::tempdir().unwrap();
        let prev = env::var("XDG_DATA_HOME").ok();
        unsafe { env::set_var("XDG_DATA_HOME", tmp.path()); }

        let result = data_dir();
        assert_eq!(result, Some(tmp.path().join("hume")));

        unsafe {
            match prev {
                Some(v) => env::set_var("XDG_DATA_HOME", v),
                None => env::remove_var("XDG_DATA_HOME"),
            }
        }
    }

    #[test]
    fn runtime_dir_respects_hume_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        unsafe { env::set_var("HUME_RUNTIME", tmp.path()); }

        let result = runtime_dir();
        assert_eq!(result, Some(tmp.path().to_path_buf()));

        unsafe { env::remove_var("HUME_RUNTIME"); }
    }
}
