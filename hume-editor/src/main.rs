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
    },
}

// Classify validated args into a run mode. clap guarantees `output` is present
// whenever `keys` is, and vice-versa, via `requires`. The only constraint clap
// can't express — exactly one input file in headless mode — is checked here.
fn resolve(cli: Cli) -> Result<Mode, String> {
    match cli.keys {
        Some(keys) => {
            // Safe: clap's `requires` ensures output is set when keys is set.
            let output = cli.output.expect("clap ensures --output when --keys is set");
            match cli.files.as_slice() {
                [input] => Ok(Mode::Headless {
                    input: input.clone(),
                    keys,
                    output,
                }),
                _ => Err("--keys mode requires exactly one input file".into()),
            }
        }
        None => Ok(Mode::Normal { files: cli.files }),
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
        Mode::Headless { input, keys, output } => hume_editor::run_keys(input, &keys, output),
        Mode::Normal { files } => hume_editor::run(files),
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
        assert_eq!(cli.files, vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")]);
    }

    #[test]
    fn parse_no_args() {
        let cli = Cli::try_parse_from(["hume"]).expect("bare invocation should parse");
        assert_eq!(cli.keys, None);
        assert!(cli.files.is_empty());
    }

    // ── resolve layer: mode classification (pure logic, no clap) ─────────────

    fn make_headless(files: Vec<PathBuf>) -> Cli {
        Cli {
            keys: Some("dw".into()),
            output: Some(PathBuf::from("out.txt")),
            files,
        }
    }

    fn make_normal(files: Vec<PathBuf>) -> Cli {
        Cli { keys: None, output: None, files }
    }

    #[test]
    fn resolve_headless_exactly_one_file_succeeds() {
        let cli = make_headless(vec![PathBuf::from("in.txt")]);
        let mode = resolve(cli).expect("one input file should succeed");
        let Mode::Headless { input, keys, output } = mode else {
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
        let mode = resolve(make_normal(files.clone())).expect("normal mode should succeed");
        let Mode::Normal { files: got } = mode else {
            panic!("expected Mode::Normal");
        };
        assert_eq!(got, files);
    }

    #[test]
    fn resolve_normal_no_files() {
        let mode = resolve(make_normal(vec![])).expect("no-file launch should succeed");
        let Mode::Normal { files } = mode else {
            panic!("expected Mode::Normal");
        };
        assert!(files.is_empty());
    }
}
