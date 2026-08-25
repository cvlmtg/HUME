use clap::Parser;
use std::path::PathBuf;
use std::process;

/// HUME — a modal text editor.
#[derive(Parser)]
#[command(name = "hume", version = hume_editor::VERSION)]
struct Cli {
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
    /// directories. Not valid in headless --keys mode.
    #[arg(long, value_name = "FILE", conflicts_with = "keys")]
    config: Option<PathBuf>,

    /// Files to open (normal mode) or the single input file (headless mode).
    #[arg(value_name = "FILE")]
    files: Vec<PathBuf>,
}

enum Mode {
    Headless {
        input: PathBuf,
        keys: String,
        output: PathBuf,
    },
    Normal {
        files: Vec<PathBuf>,
        config: Option<PathBuf>,
    },
}

// Classify validated args into a run mode. clap guarantees `output` is present
// whenever `keys` is, and vice-versa, via `requires`, and that `config` never
// appears alongside `keys`, via `conflicts_with`. The constraints clap can't
// express — exactly one input file in headless mode, `config` naming a real
// file — are checked here.
fn resolve(cli: Cli) -> Result<Mode, String> {
    match cli.keys {
        Some(keys) => {
            // Safe: clap's `requires` ensures output is set when keys is set.
            let output = cli
                .output
                .expect("clap ensures --output when --keys is set");
            match cli.files.as_slice() {
                [input] => Ok(Mode::Headless {
                    input: input.clone(),
                    keys,
                    output,
                }),
                _ => Err("--keys mode requires exactly one input file".into()),
            }
        }
        None => {
            // A missing default `init.scm` is normal and silently skipped
            // (see `Editor::init_scripting`), but a path the user named
            // explicitly is an assertion — a typo here should fail loudly
            // before the terminal even enters raw mode, not silently boot
            // unconfigured. `File::open` (not `fs::metadata`, a bare `stat`)
            // proves the path is both present and readable in one syscall.
            let config = match cli.config {
                Some(path) => {
                    let file = std::fs::File::open(&path)
                        .map_err(|e| format!("--config: {}: {e}", path.display()))?;
                    let is_file = file
                        .metadata()
                        .map_err(|e| format!("--config: {}: {e}", path.display()))?
                        .is_file();
                    if !is_file {
                        return Err(format!("--config: not a file: {}", path.display()));
                    }
                    // HUME moves its own process cwd at runtime (`:cd`), so a
                    // relative path must be pinned to the startup cwd here —
                    // otherwise `:reload-config` would re-resolve it against
                    // wherever `:cd` last left the process, miss the file,
                    // and silently reset to compiled-in defaults instead of
                    // erroring (see `Editor::config_path`).
                    let cwd = std::env::current_dir()
                        .map_err(|e| format!("--config: resolving current directory: {e}"))?;
                    Some(hume_platform::path::absolute_unresolved(&path, &cwd))
                }
                None => None,
            };
            Ok(Mode::Normal {
                files: cli.files,
                config,
            })
        }
    }
}

fn main() {
    let mode = match resolve(Cli::parse()) {
        Ok(mode) => mode,
        Err(msg) => {
            eprintln!("hume: {msg}");
            process::exit(1);
        }
    };
    let result = match mode {
        Mode::Headless {
            input,
            keys,
            output,
        } => hume_editor::run_keys(input, &keys, output),
        Mode::Normal { files, config } => hume_editor::run(files, config),
    };
    if let Err(e) = result {
        eprintln!("hume: {e}");
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── clap layer: parse from argv strings ──────────────────────────────────

    #[test]
    fn parse_headless_happy_path() {
        let cli = Cli::try_parse_from(["hume", "--keys", "dwx", "--output", "o.txt", "in.txt"])
            .expect("valid headless invocation should parse");
        assert_eq!(cli.keys.as_deref(), Some("dwx"));
        assert_eq!(cli.output.as_deref(), Some(std::path::Path::new("o.txt")));
        assert_eq!(cli.files, vec![PathBuf::from("in.txt")]);
    }

    #[test]
    fn parse_keys_without_output_is_rejected() {
        let err = Cli::try_parse_from(["hume", "--keys", "dwx", "in.txt"]);
        assert!(err.is_err(), "clap must reject --keys without --output");
    }

    #[test]
    fn parse_output_without_keys_is_rejected() {
        let err = Cli::try_parse_from(["hume", "--output", "o.txt", "in.txt"]);
        assert!(err.is_err(), "clap must reject --output without --keys");
    }

    #[test]
    fn parse_normal_multi_file() {
        let cli = Cli::try_parse_from(["hume", "a.rs", "b.rs"])
            .expect("normal multi-file invocation should parse");
        assert_eq!(cli.keys, None);
        assert_eq!(cli.output, None);
        assert_eq!(
            cli.files,
            vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")]
        );
    }

    #[test]
    fn parse_no_args() {
        let cli = Cli::try_parse_from(["hume"]).expect("bare invocation should parse");
        assert_eq!(cli.keys, None);
        assert!(cli.files.is_empty());
    }

    #[test]
    fn parse_config_flag() {
        let cli = Cli::try_parse_from(["hume", "--config", "alt.scm", "in.txt"])
            .expect("--config should parse");
        assert_eq!(cli.config.as_deref(), Some(std::path::Path::new("alt.scm")));
    }

    #[test]
    fn parse_config_with_keys_is_rejected() {
        let err = Cli::try_parse_from([
            "hume", "--keys", "dwx", "--output", "o.txt", "--config", "alt.scm", "in.txt",
        ]);
        assert!(err.is_err(), "clap must reject --config with --keys");
    }

    // ── resolve layer: mode classification (no clap; touches the filesystem
    //    only to validate/pin a `--config` path) ───────────────────────────

    fn make_headless(files: Vec<PathBuf>) -> Cli {
        Cli {
            keys: Some("dw".into()),
            output: Some(PathBuf::from("out.txt")),
            config: None,
            files,
        }
    }

    fn make_normal(files: Vec<PathBuf>, config: Option<PathBuf>) -> Cli {
        Cli {
            keys: None,
            output: None,
            config,
            files,
        }
    }

    #[test]
    fn resolve_headless_exactly_one_file_succeeds() {
        let cli = make_headless(vec![PathBuf::from("in.txt")]);
        let mode = resolve(cli).expect("one input file should succeed");
        let Mode::Headless {
            input,
            keys,
            output,
        } = mode
        else {
            panic!("expected Mode::Headless");
        };
        assert_eq!(input, PathBuf::from("in.txt"));
        assert_eq!(keys, "dw");
        assert_eq!(output, PathBuf::from("out.txt"));
    }

    #[test]
    fn resolve_headless_zero_files_errors() {
        let err = resolve(make_headless(vec![]));
        assert!(err.is_err(), "zero inputs must be rejected");
    }

    #[test]
    fn resolve_headless_two_files_errors() {
        let err = resolve(make_headless(vec![
            PathBuf::from("a.txt"),
            PathBuf::from("b.txt"),
        ]));
        assert!(err.is_err(), "two inputs must be rejected");
    }

    #[test]
    fn resolve_normal_carries_all_files() {
        let files = vec![PathBuf::from("x.rs"), PathBuf::from("y.rs")];
        let mode = resolve(make_normal(files.clone(), None)).expect("normal mode should succeed");
        let Mode::Normal { files: got, .. } = mode else {
            panic!("expected Mode::Normal");
        };
        assert_eq!(got, files);
    }

    #[test]
    fn resolve_normal_no_files() {
        let mode = resolve(make_normal(vec![], None)).expect("no-file launch should succeed");
        let Mode::Normal { files, .. } = mode else {
            panic!("expected Mode::Normal");
        };
        assert!(files.is_empty());
    }

    #[test]
    fn resolve_config_pointing_at_real_file_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("alt.scm");
        std::fs::write(&path, "").unwrap();
        let mode = resolve(make_normal(vec![], Some(path.clone()))).expect("real file should pass");
        let Mode::Normal { config, .. } = mode else {
            panic!("expected Mode::Normal");
        };
        // Already absolute with no `.`/`..` components, so pinning to the
        // startup cwd (see `resolve_config_relative_path_is_pinned_to_startup_cwd`)
        // is a no-op here.
        assert_eq!(config, Some(path));
    }

    #[test]
    fn resolve_config_missing_file_errors() {
        let err = resolve(make_normal(
            vec![],
            Some(PathBuf::from("/no/such/file/alt.scm")),
        ));
        assert!(err.is_err(), "a nonexistent --config path must be rejected");
    }

    #[test]
    fn resolve_config_pointing_at_directory_errors() {
        let dir = tempfile::tempdir().unwrap();
        let err = resolve(make_normal(vec![], Some(dir.path().to_path_buf())));
        assert!(err.is_err(), "a directory --config path must be rejected");
    }

    #[test]
    fn resolve_no_config_flag_succeeds() {
        let mode = resolve(make_normal(vec![], None)).expect("no --config should pass");
        let Mode::Normal { config, .. } = mode else {
            panic!("expected Mode::Normal");
        };
        assert_eq!(config, None);
    }

    // A relative `--config` path must be pinned to the startup cwd, not left
    // relative — HUME moves its own process cwd at runtime (`:cd`), so a
    // relative path re-resolved against a later cwd at `:reload-config` time
    // would silently miss the file it originally pointed at (see
    // `Editor::config_path`'s doc and the `--config` flag's doc comment).
    #[cfg(unix)]
    #[test]
    fn resolve_config_relative_path_is_pinned_to_startup_cwd() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alt.scm"), "").unwrap();
        // `getcwd()` resolves symlinks in the path (e.g. macOS's
        // `/var` → `/private/var`), so the expected value must go through
        // the same resolution `resolve` will apply via `current_dir()` —
        // comparing against the raw, un-resolved `dir.path()` would spuriously
        // fail there.
        let canonical_dir = dir.path().canonicalize().unwrap();
        let saved_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = resolve(make_normal(vec![], Some(PathBuf::from("alt.scm"))));
        std::env::set_current_dir(&saved_cwd).unwrap();

        let Mode::Normal { config, .. } = result.expect("relative real file should pass") else {
            panic!("expected Mode::Normal");
        };
        assert_eq!(
            config,
            Some(canonical_dir.join("alt.scm")),
            "a relative --config path must be resolved against the cwd at \
             startup, not left relative for a later re-resolution to miss"
        );
    }

    // `File::open` (not a bare `fs::metadata` stat) is what makes an
    // unreadable path a startup error too, not just a missing one.
    #[cfg(unix)]
    #[test]
    fn resolve_config_unreadable_file_errors() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("alt.scm");
        std::fs::write(&path, "").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

        let result = resolve(make_normal(vec![], Some(path.clone())));

        // Restore permissions before any assertion can panic and leak an
        // unreadable file for the tempdir's own `Drop` cleanup to trip over.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert!(
            result.is_err(),
            "an unreadable --config path must be rejected"
        );
    }
}
