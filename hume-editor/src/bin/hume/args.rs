use clap::Parser;
use std::path::{Path, PathBuf};

/// HUME: a modal text editor.
#[derive(Parser)]
#[command(name = "hume", version = hume_platform::version::DISPLAY)]
pub(super) struct Cli {
    /// Headless key-runner: replay a golf-notation key STREAM.
    ///
    /// Requires --output and exactly one positional input file.
    /// Used by the golf harness (`tools/golf/golf.sh`).
    #[arg(long, value_name = "STREAM", requires = "output")]
    keys: Option<String>,

    /// Output file for headless mode.
    #[arg(long, value_name = "PATH", requires = "keys")]
    output: Option<PathBuf>,

    /// Load configuration from FILE instead of the default `init.scm`.
    ///
    /// Themes and the data directory still resolve from the standard
    /// directories.
    #[arg(long, value_name = "FILE", conflicts_with = "no_config")]
    config: Option<PathBuf>,

    /// Skip `init.scm`: no user config, no plugins. Bundled language
    /// identities, grammars, and prelude macros still load.
    #[arg(long, conflicts_with = "config")]
    no_config: bool,

    /// Files to open (normal mode) or the single input file (headless mode).
    #[arg(value_name = "FILE")]
    files: Vec<PathBuf>,
}

pub(super) enum Mode {
    Headless {
        input: hume_editor::cli::FileArg,
        keys: String,
        output: PathBuf,
    },
    Normal {
        files: Vec<hume_editor::cli::FileArg>,
    },
}

pub(super) struct Invocation {
    pub(super) mode: Mode,
    pub(super) config: hume_editor::cli::ConfigSource,
}

// Classify validated args into a run mode plus a config source. The two are
// orthogonal (any ConfigSource is valid with either Mode). clap guarantees
// `output` is present whenever `keys` is, and vice-versa, via `requires`,
// and that `config` and `no_config` never appear together, via
// `conflicts_with`. The
// constraints clap can't express (exactly one input file in headless mode,
// `config` naming a real file) are checked here.
pub(super) fn resolve(cli: Cli, cwd: &Path) -> Result<Invocation, String> {
    // A missing default `init.scm` is normal and silently skipped (see
    // `Editor::init_scripting`), but a path the user named explicitly is an
    // assertion: a typo here should fail loudly before the terminal even
    // enters raw mode (or, in headless mode, before any key is replayed),
    // not silently boot unconfigured. `File::open` (not `fs::metadata`, a
    // bare `stat`) proves the path is both present and readable in one
    // syscall. Runs for both modes.
    let config = if cli.no_config {
        hume_editor::cli::ConfigSource::Skip
    } else if let Some(path) = cli.config {
        // The process cwd follows `:cd`, so a relative path is pinned to the
        // startup cwd here. Otherwise `:reload-config` would re-resolve it
        // against wherever `:cd` last left the process, miss the file, and
        // reset to compiled-in defaults instead of erroring (see
        // `Editor::config_path`).
        let pinned = hume_platform::path::absolute_unresolved(&path, cwd);
        let file = std::fs::File::open(&pinned)
            .map_err(|e| format!("--config: {}: {e}", path.display()))?;
        let is_file = file
            .metadata()
            .map_err(|e| format!("--config: {}: {e}", path.display()))?
            .is_file();
        if !is_file {
            return Err(format!("--config: not a file: {}", path.display()));
        }
        hume_editor::cli::ConfigSource::File(pinned)
    } else {
        hume_editor::cli::ConfigSource::Default
    };

    let mode = match cli.keys {
        Some(keys) => {
            // Safe: clap's `requires` ensures output is set when keys is set.
            let output = cli
                .output
                .expect("clap ensures --output when --keys is set");
            let Ok([input]) = <[_; 1]>::try_from(cli.files) else {
                return Err("--keys mode requires exactly one input file".into());
            };
            Mode::Headless {
                input: hume_editor::cli::parse_file_arg(&input, cwd)?,
                keys,
                // Written after the replay, so a `:cd` in `keys` must not move it.
                output: hume_platform::path::absolute_unresolved(&output, cwd),
            }
        }
        None => Mode::Normal {
            files: cli
                .files
                .iter()
                .map(|p| hume_editor::cli::parse_file_arg(p, cwd))
                .collect::<Result<_, _>>()?,
        },
    };
    Ok(Invocation { mode, config })
}

#[cfg(test)]
mod tests;
