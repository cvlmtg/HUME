//! End-to-end proof that `run_keys` (headless `--keys` mode) loads config,
//! through the public API `hume::run_keys` a real `hume --keys …`
//! invocation goes through, unlike `editor/tests/unix/reload_config.rs`'s
//! `init_scripting()`-level coverage of the same wiring.

extern crate hume_editor as hume;

use hume::cli::{ConfigSource, FileArg};

#[test]
fn config_binding_takes_effect_in_headless_replay() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.txt");
    let output = dir.path().join("out.txt");
    std::fs::write(&input, "hello\n").unwrap();

    // "Z" is unbound by default and, unlike "Q"/"q"/"\"", isn't handled
    // ahead of the keymap trie walk either (see `handle_normal`'s own
    // macro-record/-replay and register-prefix intercepts, run inside the
    // `Base` layer's handler before it reaches the trie), so this init.scm is
    // the only thing that can make it do anything.
    let config = dir.path().join("init.scm");
    std::fs::write(&config, r#"(bind-key! 'normal "Z" "delete-char-forward")"#).unwrap();

    hume::run_keys(
        FileArg {
            path: input,
            pos: None,
        },
        "Z",
        output.clone(),
        ConfigSource::File(config),
        std::env::current_dir().unwrap(),
    )
    .unwrap();

    assert_eq!(
        std::fs::read_to_string(&output).unwrap(),
        "ello\n",
        "the --config override's bind-key! must take effect in headless replay"
    );
}

#[test]
fn write_failure_still_returns_err() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.txt");
    std::fs::write(&input, "hello\n").unwrap();
    // A directory that doesn't exist: `std::fs::write` fails with `NotFound`
    // rather than creating it.
    let output = dir.path().join("missing-dir").join("out.txt");

    let result = hume::run_keys(
        FileArg {
            path: input,
            pos: None,
        },
        "Z",
        output,
        ConfigSource::Skip,
        std::env::current_dir().unwrap(),
    );

    assert!(
        result.is_err(),
        "a failed output write must still surface as an error, not be swallowed \
         by the LSP shutdown that now runs alongside it"
    );
}

#[test]
fn a_crash_dump_next_to_the_file_is_neither_offered_nor_touched() {
    let run = |with_dump: bool| {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.txt");
        let output = dir.path().join("out.txt");
        std::fs::write(&input, "hello\nworld\n").unwrap();
        let dump = dir.path().join("in.txt.dump");
        if with_dump {
            std::fs::write(&dump, "crashed\n").unwrap();
        }
        hume::run_keys(
            FileArg {
                path: input,
                pos: None,
            },
            "xd",
            output.clone(),
            ConfigSource::Skip,
            std::env::current_dir().unwrap(),
        )
        .unwrap();
        let dump_after = std::fs::read_to_string(&dump).ok();
        (std::fs::read_to_string(&output).unwrap(), dump_after)
    };

    let (baseline, _) = run(false);
    let (with_dump, dump_after) = run(true);

    assert_eq!(with_dump, baseline);
    assert_eq!(dump_after.as_deref(), Some("crashed\n"));
}

struct HeadlessRun {
    exit_code: Option<i32>,
    stderr: String,
    /// `None` when the output file was never written.
    output: Option<String>,
}

/// Runs the `hume` binary in `--keys` mode on `"hello\nworld\n"` with
/// `config` as its `--config`. `position` is appended to the input path as
/// typed on the command line (`""`, or a suffix such as `":2"`). The config,
/// data, runtime and Steel home directories are pinned so nothing on the
/// developer's machine is loaded or logged.
fn run_headless(config: &str, keys: &str, position: &str) -> HeadlessRun {
    let dir = tempfile::tempdir().expect("create temp dir");
    let input = dir.path().join("in.txt");
    let output = dir.path().join("out.txt");
    let config_path = dir.path().join("test.scm");
    std::fs::write(&input, "hello\nworld\n").expect("write input file");
    std::fs::write(&config_path, config).expect("write config file");
    let home = dir.path().join("home");
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime");

    let result = std::process::Command::new(env!("CARGO_BIN_EXE_hume"))
        .arg("--keys")
        .arg(keys)
        .arg("--output")
        .arg(&output)
        .arg("--config")
        .arg(&config_path)
        .arg(format!("{}{position}", input.display()))
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("APPDATA", home.join("appdata"))
        .env("LOCALAPPDATA", home.join("localappdata"))
        .env("HUME_RUNTIME", runtime)
        .env("STEEL_HOME", home.join("steel"))
        .output()
        .expect("run the hume binary");

    HeadlessRun {
        exit_code: result.status.code(),
        stderr: String::from_utf8(result.stderr).expect("stderr is UTF-8"),
        output: std::fs::read_to_string(&output).ok(),
    }
}

#[test]
fn a_config_error_fails_the_run_before_any_key_is_replayed() {
    let run = run_headless("(offset->line 1 2)\n", "xd", "");

    assert_eq!(run.exit_code, Some(1));
    // A Steel error report spans several lines after its first.
    let report = run
        .stderr
        .split_once("hume: error: test.scm: ")
        .map(|(_, report)| report)
        .unwrap_or_else(|| panic!("no init error on stderr:\n{}", run.stderr));
    assert!(report.contains("offset->line"), "{report}");
    assert_eq!(run.output, None);
}

#[test]
fn an_error_during_the_replay_fails_the_run_and_still_writes_the_output() {
    let config = r#"
(define-command! "raise-in-test" "Raise an error." (lambda (pane) (error "raised by the test")))
(bind-key! 'normal "Z" "raise-in-test")
"#;
    let run = run_headless(config, "xdZ", "");

    assert_eq!(run.exit_code, Some(1));
    let report = run
        .stderr
        .split_once("hume: error: ")
        .map(|(_, report)| report)
        .unwrap_or_else(|| panic!("no replay error on stderr:\n{}", run.stderr));
    assert!(report.contains("raised by the test"), "{report}");
    assert_eq!(run.output.as_deref(), Some("world\n"));
}

#[test]
fn a_warning_is_reported_on_stderr_without_failing_the_run() {
    let run = run_headless(r#"(bind-key! 'normal "Z" "no-such-command")"#, "xd", "");

    assert_eq!(run.exit_code, Some(0), "stderr:\n{}", run.stderr);
    let problems: Vec<&str> = run.stderr.lines().collect();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].starts_with("hume: warning: key bound to unknown command 'no-such-command'"),
        "{problems:?}"
    );
    assert_eq!(run.output.as_deref(), Some("world\n"));
}

#[test]
fn a_line_suffix_on_the_input_places_the_cursor_before_the_first_key() {
    let run = run_headless("", "xd", ":2");

    assert_eq!(run.exit_code, Some(0), "{}", run.stderr);
    assert_eq!(run.output.as_deref(), Some("hello\n"));
}
