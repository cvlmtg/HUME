//! Platform-aware base directory resolution for HUME.
//!
//! Follows XDG Base Directory conventions on Unix and macOS, and uses
//! `%APPDATA%\hume\` on Windows native. All three roots are documented
//! in STEEL.md §"Three root directories".

use std::{env, path::PathBuf};

/// Returns the configuration directory for HUME.
///
/// - Unix / macOS: `$XDG_CONFIG_HOME/hume/` → `~/.config/hume/`
/// - Windows: `%APPDATA%\hume\config\`
pub(crate) fn config_dir() -> PathBuf {
    #[cfg(windows)]
    {
        let base = env::var("APPDATA").unwrap_or_else(|_| ".".into());
        PathBuf::from(base).join("hume").join("config")
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
            PathBuf::from(xdg).join("hume")
        } else if let Ok(home) = env::var("HOME") {
            PathBuf::from(home).join(".config").join("hume")
        } else {
            // Last-resort fallback; should not happen in practice.
            PathBuf::from(".config/hume")
        }
    }
}

/// Returns the data directory for HUME.
///
/// - Unix / macOS: `$XDG_DATA_HOME/hume/` → `~/.local/share/hume/`
/// - Windows: `%APPDATA%\hume\data\`
pub(crate) fn data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        let base = env::var("APPDATA").unwrap_or_else(|_| ".".into());
        PathBuf::from(base).join("hume").join("data")
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = env::var("XDG_DATA_HOME") {
            PathBuf::from(xdg).join("hume")
        } else if let Ok(home) = env::var("HOME") {
            PathBuf::from(home).join(".local").join("share").join("hume")
        } else {
            PathBuf::from(".local/share/hume")
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
        // SAFETY: test-only env mutation; tests run sequentially in this module.
        let tmp = tempfile::tempdir().unwrap();
        let prev = env::var("XDG_CONFIG_HOME").ok();
        unsafe { env::set_var("XDG_CONFIG_HOME", tmp.path()); }

        let result = config_dir();
        assert_eq!(result, tmp.path().join("hume"));

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
        assert_eq!(result, tmp.path().join("hume"));

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
