//! End-to-end proof that `run_keys` (headless `--keys` mode) actually loads
//! config now — through the public API `hume::run_keys` a real
//! `hume --keys …` invocation goes through, unlike
//! `editor/tests/unix/reload_config.rs`'s `init_scripting()`-level coverage
//! of the same wiring.

extern crate hume_editor as hume;

use hume::cli::ConfigSource;

#[test]
fn config_binding_takes_effect_in_headless_replay() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.txt");
    let output = dir.path().join("out.txt");
    std::fs::write(&input, "hello\n").unwrap();

    // "Z" is unbound by default and, unlike "Q"/"q"/"\"", isn't intercepted
    // ahead of the keymap trie either (see `handle_normal`'s macro-record/
    // -replay and register-prefix intercepts) — this init.scm is the only
    // thing that can make it do anything.
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
fn no_config_leaves_config_bindings_inert_in_headless_replay() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.txt");
    let output = dir.path().join("out.txt");
    std::fs::write(&input, "hello\n").unwrap();

    hume::run_keys(input, "Z", output.clone(), ConfigSource::Skip).unwrap();

    assert_eq!(
        std::fs::read_to_string(&output).unwrap(),
        "hello\n",
        "--no-config must leave an unbound key inert — no init.scm ever ran to bind it"
    );
}
