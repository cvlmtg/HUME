use clap::Parser;
use std::path::{Path, PathBuf};
use std::process;

/// HUME: a modal text editor.
#[derive(Parser)]
#[command(name = "hume", version = hume_platform::version::DISPLAY)]
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

enum Mode {
    Headless {
        input: PathBuf,
        keys: String,
        output: PathBuf,
    },
    Normal {
        files: Vec<hume_editor::cli::FileArg>,
    },
}

struct Invocation {
    mode: Mode,
    config: hume_editor::cli::ConfigSource,
}

// Classify validated args into a run mode plus a config source. The two are
// orthogonal (any ConfigSource is valid with either Mode). clap guarantees
// `output` is present whenever `keys` is, and vice-versa, via `requires`,
// and that `config` and `no_config` never appear together, via
// `conflicts_with`. The
// constraints clap can't express (exactly one input file in headless mode,
// `config` naming a real file) are checked here.
fn resolve(cli: Cli, cwd: &Path) -> Result<Invocation, String> {
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
            match cli.files.as_slice() {
                [input] => Mode::Headless {
                    input: input.clone(),
                    keys,
                    // Written after the replay, so a `:cd` in `keys` must not move it.
                    output: hume_platform::path::absolute_unresolved(&output, cwd),
                },
                _ => return Err("--keys mode requires exactly one input file".into()),
            }
        }
        None => {
            let files = cli
                .files
                .iter()
                .map(|p| hume_editor::cli::parse_file_arg(p, cwd))
                .collect::<Result<Vec<_>, _>>()?;
            Mode::Normal { files }
        }
    };
    Ok(Invocation { mode, config })
}

fn main() {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(e) => {
            eprintln!("hume: resolving current directory: {e}");
            process::exit(1);
        }
    };
    let Invocation { mode, config } = match resolve(Cli::parse(), &cwd) {
        Ok(inv) => inv,
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
        } => hume_editor::run_keys(input, &keys, output, config),
        Mode::Normal { files } => hume_editor::run(files, config),
    };
    if let Err(e) = result {
        eprintln!("hume: {e}");
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hume_editor::cli::{ConfigSource, FileArg};
    use hume_rope::column::GraphemeCol;

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

    // `--config` and `--keys` are orthogonal (see `Invocation`).
    #[test]
    fn parse_config_with_keys_is_accepted() {
        let cli = Cli::try_parse_from([
            "hume", "--keys", "dwx", "--output", "o.txt", "--config", "alt.scm", "in.txt",
        ])
        .expect("--config must now be valid alongside --keys");
        assert_eq!(cli.config.as_deref(), Some(std::path::Path::new("alt.scm")));
        assert_eq!(cli.keys.as_deref(), Some("dwx"));
    }

    #[test]
    fn parse_no_config_flag() {
        let cli = Cli::try_parse_from(["hume", "--no-config", "in.txt"])
            .expect("--no-config should parse");
        assert!(cli.no_config);
    }

    #[test]
    fn parse_no_config_with_config_is_rejected() {
        let err = Cli::try_parse_from(["hume", "--no-config", "--config", "alt.scm", "in.txt"]);
        assert!(err.is_err(), "clap must reject --no-config with --config");
    }

    #[test]
    fn parse_no_config_with_keys_is_accepted() {
        let cli = Cli::try_parse_from([
            "hume",
            "--keys",
            "dwx",
            "--output",
            "o.txt",
            "--no-config",
            "in.txt",
        ])
        .expect("--no-config must be valid alongside --keys");
        assert!(cli.no_config);
    }

    // ── resolve layer: mode + config-source classification (no clap; touches
    //    the filesystem only to validate/pin a `--config` path) ─────────────

    /// `resolve` against the cwd the test binary started in.
    fn resolve_here(cli: Cli) -> Result<Invocation, String> {
        resolve(cli, &std::env::current_dir().unwrap())
    }

    fn make_headless(files: Vec<PathBuf>) -> Cli {
        Cli {
            keys: Some("dw".into()),
            output: Some(PathBuf::from("out.txt")),
            config: None,
            no_config: false,
            files,
        }
    }

    fn make_normal(files: Vec<PathBuf>, config: Option<PathBuf>) -> Cli {
        Cli {
            keys: None,
            output: None,
            config,
            no_config: false,
            files,
        }
    }

    #[test]
    fn resolve_output_relative_path_is_pinned_to_the_startup_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let cli = make_headless(vec![PathBuf::from("in.txt")]);

        let inv = resolve(cli, dir.path()).expect("headless invocation should resolve");

        let Mode::Headless { output, .. } = inv.mode else {
            panic!("expected headless mode");
        };
        assert_eq!(
            output,
            dir.path().join("out.txt"),
            "a relative --output must be resolved against the cwd at startup, \
             so a later `:cd` in the keys cannot move it"
        );
    }

    #[test]
    fn resolve_headless_exactly_one_file_succeeds() {
        let cwd = std::env::current_dir().unwrap();
        let cli = make_headless(vec![PathBuf::from("in.txt")]);
        let inv = resolve(cli, &cwd).expect("one input file should succeed");
        let Mode::Headless {
            input,
            keys,
            output,
        } = inv.mode
        else {
            panic!("expected Mode::Headless");
        };
        assert_eq!(input, PathBuf::from("in.txt"));
        assert_eq!(keys, "dw");
        assert_eq!(output, cwd.join("out.txt"));
        assert_eq!(
            inv.config,
            ConfigSource::Default,
            "headless mode now loads config by default, same as interactive mode"
        );
    }

    #[test]
    fn resolve_headless_zero_files_errors() {
        let err = resolve_here(make_headless(vec![]));
        assert!(err.is_err(), "zero inputs must be rejected");
    }

    #[test]
    fn resolve_headless_two_files_errors() {
        let err = resolve_here(make_headless(vec![
            PathBuf::from("a.txt"),
            PathBuf::from("b.txt"),
        ]));
        assert!(err.is_err(), "two inputs must be rejected");
    }

    #[test]
    fn resolve_normal_carries_all_files() {
        let files = vec![PathBuf::from("x.rs"), PathBuf::from("y.rs")];
        let inv = resolve_here(make_normal(files, None)).expect("normal mode should succeed");
        let Mode::Normal { files: got } = inv.mode else {
            panic!("expected Mode::Normal");
        };
        assert_eq!(
            got,
            vec![
                FileArg {
                    path: PathBuf::from("x.rs"),
                    pos: None
                },
                FileArg {
                    path: PathBuf::from("y.rs"),
                    pos: None
                },
            ]
        );
    }

    // `resolve` runs every positional file through `cli::parse_file_arg`;
    // this pins that wiring at the `resolve` layer, complementing
    // `cli::tests`' coverage of the parser itself.
    #[test]
    fn resolve_normal_splits_a_line_column_suffix() {
        let files = vec![PathBuf::from("does-not-exist.rs:12:24")];
        let inv = resolve_here(make_normal(files, None)).expect("normal mode should succeed");
        let Mode::Normal { files: got } = inv.mode else {
            panic!("expected Mode::Normal");
        };
        assert_eq!(
            got,
            vec![FileArg {
                path: PathBuf::from("does-not-exist.rs"),
                pos: Some(hume_editor::cli::PathPosition {
                    line: hume_rope::line::ContentLine::from_number(12).unwrap(),
                    grapheme_col: GraphemeCol::from_number(24).unwrap()
                })
            }]
        );
    }

    // A `--keys`-mode input path is the golf harness's literal single
    // argument, never a `path:line:col` diagnostic pasted onto the command
    // line. `resolve` must leave it untouched by `cli::parse_file_arg`.
    #[test]
    fn resolve_headless_input_path_is_never_split() {
        let cli = make_headless(vec![PathBuf::from("weird:12")]);
        let inv = resolve_here(cli).expect("one input file should succeed");
        let Mode::Headless { input, .. } = inv.mode else {
            panic!("expected Mode::Headless");
        };
        assert_eq!(input, PathBuf::from("weird:12"));
    }

    #[test]
    fn resolve_normal_no_files() {
        let inv = resolve_here(make_normal(vec![], None)).expect("no-file launch should succeed");
        let Mode::Normal { files } = inv.mode else {
            panic!("expected Mode::Normal");
        };
        assert!(files.is_empty());
    }

    #[test]
    fn resolve_config_pointing_at_real_file_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("alt.scm");
        std::fs::write(&path, "").unwrap();
        let inv =
            resolve_here(make_normal(vec![], Some(path.clone()))).expect("real file should pass");
        // Already absolute with no `.`/`..` components, so pinning to the
        // startup cwd (see `resolve_config_relative_path_is_pinned_to_startup_cwd`)
        // is a no-op here.
        assert_eq!(inv.config, ConfigSource::File(path));
    }

    #[test]
    fn resolve_config_missing_file_errors() {
        let err = resolve_here(make_normal(
            vec![],
            Some(PathBuf::from("/no/such/file/alt.scm")),
        ));
        assert!(err.is_err(), "a nonexistent --config path must be rejected");
    }

    #[test]
    fn resolve_config_pointing_at_directory_errors() {
        let dir = tempfile::tempdir().unwrap();
        let err = resolve_here(make_normal(vec![], Some(dir.path().to_path_buf())));
        assert!(err.is_err(), "a directory --config path must be rejected");
    }

    #[test]
    fn resolve_default_config_source_when_no_flags_given() {
        let inv = resolve_here(make_normal(vec![], None)).expect("no --config should pass");
        assert_eq!(inv.config, ConfigSource::Default);
    }

    #[test]
    fn resolve_no_config_flag_skips_config() {
        let cli = Cli {
            no_config: true,
            ..make_normal(vec![], None)
        };
        let inv = resolve_here(cli).expect("--no-config should pass");
        assert_eq!(inv.config, ConfigSource::Skip);
    }

    // The `--config` validation/pinning path is shared code, exercised in
    // full above under normal mode. This pins that headless mode reaches
    // the same code, not a bypassed copy.
    #[test]
    fn resolve_headless_config_flag_validates_and_pins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("alt.scm");
        std::fs::write(&path, "").unwrap();
        let cli = Cli {
            config: Some(path.clone()),
            ..make_headless(vec![PathBuf::from("in.txt")])
        };
        let inv = resolve_here(cli).expect("real --config file should pass in headless mode");
        assert!(matches!(inv.mode, Mode::Headless { .. }));
        assert_eq!(inv.config, ConfigSource::File(path));
    }

    // A relative `--config` path must be pinned to the startup cwd, not left
    // relative: the process cwd follows `:cd`, so a relative path
    // re-resolved against a later cwd at `:reload-config` time would
    // miss the file it named at startup (see `Editor::config_path`'s doc and
    // the `--config` flag's doc comment).
    #[test]
    fn resolve_config_relative_path_is_pinned_to_startup_cwd() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alt.scm"), "").unwrap();

        let inv = resolve(
            make_normal(vec![], Some(PathBuf::from("alt.scm"))),
            dir.path(),
        )
        .expect("relative real file should pass");

        assert_eq!(
            inv.config,
            ConfigSource::File(dir.path().join("alt.scm")),
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

        let result = resolve_here(make_normal(vec![], Some(path.clone())));

        // Restore permissions before any assertion can panic and leak an
        // unreadable file for the tempdir's own `Drop` cleanup to trip over.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert!(
            result.is_err(),
            "an unreadable --config path must be rejected"
        );
    }
}
