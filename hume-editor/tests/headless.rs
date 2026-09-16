//! End-to-end proof that `run_keys` (headless `--keys` mode) loads config —
//! through the public API `hume::run_keys` a real `hume --keys …`
//! invocation goes through, unlike `editor/tests/unix/reload_config.rs`'s
//! `init_scripting()`-level coverage of the same wiring.

extern crate hume_editor as hume;

use hume::cli::ConfigSource;

#[test]
fn config_binding_takes_effect_in_headless_replay() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.txt");
    let output = dir.path().join("out.txt");
    std::fs::write(&input, "hello\n").unwrap();

    // "Z" is unbound by default and, unlike "Q"/"q"/"\"", isn't handled
    // ahead of the keymap trie walk either (see `handle_normal`'s own
    // macro-record/-replay and register-prefix intercepts, run inside the
    // `Base` layer's handler before it reaches the trie) — this init.scm is
    // the only thing that can make it do anything.
    let config = dir.path().join("init.scm");
    std::fs::write(&config, r#"(bind-key! 'normal "Z" "delete-char-forward")"#).unwrap();

    hume::run_keys(input, "Z", output.clone(), ConfigSource::File(config)).unwrap();

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

    let result = hume::run_keys(input, "Z", output, ConfigSource::Skip);

    assert!(
        result.is_err(),
        "a failed output write must still surface as an error, not be swallowed \
         by the LSP shutdown that now runs alongside it"
    );
}
