//! Platform-aware base directory resolution for HUME.
//!
//! Follows XDG Base Directory conventions on Unix and macOS. On Windows:
//! - Config lives under `%APPDATA%\hume\` (Roaming: syncs across domain machines,
//!   appropriate for user-edited config files).
//! - Data lives under `%LOCALAPPDATA%\hume\` (Local: machine-specific, appropriate
//!   for plugin binaries and caches that must not roam between machines).
//!
//! All resolvers return `Option<PathBuf>`: `None` means the platform-specific
//! env vars are unset (no silent fallback to `.config/hume` or `.local/share/hume`).
//! Callers decide how to handle the missing directory: PLUM disables itself,
//! `init.scm` loading is skipped, etc. Fail-fast over silent-wrong.

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};

fn env_var(key: &str) -> Option<String> {
    env::var(key).ok()
}

/// The base directories a session works from, and the `PATH` it runs programs
/// from, resolved once at startup.
///
/// Consumers read them from here rather than the process environment, so a
/// test builds a `Dirs` with explicit paths and never mutates global env.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dirs {
    /// `$XDG_CONFIG_HOME/hume/`, else `$HOME/.config/hume/` (Unix and macOS);
    /// `%APPDATA%\hume\` (Windows). `None` when those variables are unset.
    pub config: Option<PathBuf>,
    /// `$XDG_DATA_HOME/hume/`, else `$HOME/.local/share/hume/` (Unix and
    /// macOS); `%LOCALAPPDATA%\hume\`, falling back to `%APPDATA%\hume\`
    /// (Windows). `None` when those variables are unset, which disables
    /// features that need on-disk storage (PLUM install, user plugins,
    /// third-party theme installs).
    pub data: Option<PathBuf>,
    /// Where core plugins, themes and docs live; see `runtime_dir_with` for
    /// the search order. `None` when no candidate exists.
    pub runtime: Option<PathBuf>,
    /// Where `:tutor` writes its editable copy of the lesson.
    pub tmp: Option<PathBuf>,
    /// The grammar compiler `hume-grammar-build`; see [`GrammarBuild::beside`]
    /// for where it is looked for.
    pub grammar_build: GrammarBuild,
    /// `PATH` as the session started with it: every spawned child gets it, and
    /// a program named without a directory is looked up on it. `None` when
    /// `PATH` is unset, which a child then is too.
    pub search_path: Option<OsString>,
}

/// Where the grammar compiler was found, or every path looked at for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrammarBuild {
    Found(PathBuf),
    Missing {
        searched: Vec<PathBuf>,
    },
    /// The executable's own path is unknown, and every candidate is
    /// relative to it.
    ExeUnknown,
}

/// Why the resolved directories cannot be used.
#[derive(Debug, PartialEq, Eq)]
pub enum DirsError {
    /// The runtime dir is the data dir, so the bundled `plugins/core/` and
    /// `themes/` would read as user-installed ones.
    RuntimeIsDataDir(PathBuf),
    /// A directory could not be made absolute against the startup working
    /// directory (see [`crate::path::absolute()`]).
    Unresolvable {
        dir: PathBuf,
        kind: std::io::ErrorKind,
    },
}

impl std::fmt::Display for DirsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RuntimeIsDataDir(dir) => write!(
                f,
                "the runtime directory {} is also the data directory; \
                 set HUME_RUNTIME to a separate copy of runtime/",
                dir.display()
            ),
            Self::Unresolvable { dir, kind } => write!(f, "{}: {kind}", dir.display()),
        }
    }
}

impl std::error::Error for DirsError {}

impl Dirs {
    /// Resolve every directory from the process environment and the
    /// executable. `cwd` is the startup working directory that relative values
    /// are pinned to.
    pub fn from_env(cwd: &std::path::Path) -> Result<Self, DirsError> {
        Self::resolve(
            env_var,
            env::current_exe().ok(),
            Some(cwd.to_path_buf()),
            env::temp_dir(),
            env::var_os("PATH"),
        )
    }

    /// The pure form of [`Self::from_env`]. A relative directory (a relative
    /// `XDG_DATA_HOME`, `HUME_RUNTIME=runtime`, a relative `TMPDIR`) is made
    /// absolute against `cwd` here, once, so it keeps naming the same place after the
    /// working directory moves. Without a `cwd` there is nothing to join onto
    /// and values stay as given.
    ///
    /// A runtime dir that is the data dir, spelled alike or through a
    /// symlink, is an error.
    fn resolve(
        env: impl Fn(&str) -> Option<String>,
        exe: Option<PathBuf>,
        cwd: Option<PathBuf>,
        tmp: PathBuf,
        search_path: Option<OsString>,
    ) -> Result<Self, DirsError> {
        let pin = |dir: PathBuf| match &cwd {
            Some(cwd) => crate::path::absolute(&dir, cwd).map_err(|e| DirsError::Unresolvable {
                dir,
                kind: e.kind(),
            }),
            None => Ok(dir),
        };
        let dirs = Self {
            config: config_dir_with(&env).map(pin).transpose()?,
            data: data_dir_with(&env).map(pin).transpose()?,
            grammar_build: exe
                .as_deref()
                .map_or(GrammarBuild::ExeUnknown, GrammarBuild::beside),
            runtime: runtime_dir_with(&env, exe, cwd.clone())
                .map(pin)
                .transpose()?,
            tmp: Some(pin(tmp)?),
            search_path,
        };
        if let (Some(runtime), Some(data)) = (&dirs.runtime, &dirs.data)
            && (runtime == data
                || matches!(
                    (runtime.canonicalize(), data.canonicalize()),
                    (Ok(r), Ok(d)) if r == d
                ))
        {
            return Err(DirsError::RuntimeIsDataDir(runtime.clone()));
        }
        Ok(dirs)
    }

    /// No directory resolved and no `PATH`: scripting runs without a runtime,
    /// data, config or temp directory.
    pub const fn none() -> Self {
        Self {
            config: None,
            data: None,
            runtime: None,
            tmp: None,
            grammar_build: GrammarBuild::ExeUnknown,
            search_path: None,
        }
    }
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
/// 2. `../share/hume/runtime/` relative to the binary, Unix / macOS installed (FHS)
///    layout. Not `../share/hume/` itself: with the binary in `~/.local/bin` that
///    is the data dir, whose `plugins/` and `themes/` would alias the runtime's.
/// 3. `runtime/` relative to the binary: portable/archive layout (nightly zips/tarballs
///    ship `hume(.exe)` next to a `runtime/` directory; this covers Windows, where there
///    is no `share/` layout, and any unpacked archive run in place on Unix).
/// 4. `./runtime` relative to cwd: dev fallback when running with `cargo run` from the
///    workspace root.
///
/// Every candidate except (1) is existence-checked; returns `None` if none exist.
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
        if let Some(share_runtime) = exe_dir
            .parent()
            .map(|p| p.join("share").join("hume").join("runtime"))
            && share_runtime.exists()
        {
            return Some(share_runtime);
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

impl GrammarBuild {
    /// Where the grammar compiler is, relative to the executable `exe`:
    ///
    /// 1. `../libexec/hume/` relative to the binary: Unix / macOS installed
    ///    layout, which keeps a helper only HUME runs off `PATH`.
    /// 2. The binary's own directory: the Windows archive layout, and a cargo
    ///    build, which puts every workspace binary in one `target/<profile>/`.
    ///
    /// The first candidate that exists wins.
    pub fn beside(exe: &Path) -> Self {
        let Some(exe_dir) = exe.parent() else {
            return Self::ExeUnknown;
        };
        let file = format!("hume-grammar-build{}", env::consts::EXE_SUFFIX);

        let libexec = exe_dir
            .parent()
            .filter(|_| cfg!(not(windows)))
            .map(|prefix| prefix.join("libexec").join("hume").join(&file));
        let searched: Vec<_> = libexec.into_iter().chain([exe_dir.join(file)]).collect();

        match searched.iter().find(|tool| tool.exists()) {
            Some(tool) => Self::Found(tool.clone()),
            None => Self::Missing { searched },
        }
    }
}

#[cfg(test)]
mod tests;
