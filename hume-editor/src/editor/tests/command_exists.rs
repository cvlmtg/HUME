// End-to-end coverage for `(command-exists? name)`.

use super::*;

/// Defines the fixtures, then a `:check` command that logs `(command-exists? name)`
/// for each of `names`, and returns the logged values in order.
fn exists(ed: &mut Editor, tmp: &std::path::Path, setup: &str, names: &[&str]) -> Vec<bool> {
    let calls: String = names
        .iter()
        .map(|n| format!(r#"(command-exists? "{n}")"#))
        .collect::<Vec<_>>()
        .join(" ");
    let source = format!(
        r#"{setup}
           (define-typed-command! "check" "" (lambda (bid) (log! 'info (to-string (list {calls})))))"#
    );
    let mut host = hume_scripting::ScriptingHost::new();
    host.set_data_dir(tmp.to_path_buf());
    install_source(ed, host, &source, tmp);
    type_cmd(ed, ":check");
    let msg = ed.state.status_msg.clone().unwrap();
    msg.trim_matches(|c| c == '(' || c == ')')
        .split_whitespace()
        .map(|w| w == "#true")
        .collect()
}

#[test]
fn native_and_steel_commands_exist() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    let got = exists(
        &mut ed,
        tmp.path(),
        r#"(define-command! "steel-cmd" "doc" (lambda () (+ 1 0)))"#,
        &["move-right", "steel-cmd"],
    );
    assert_eq!(got, [true, true]);
}

#[test]
fn lazy_plugin_command_exists_without_activating_the_plugin() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    let plugin_dir = tmp.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(
        plugin_dir.join("plugin.scm"),
        r#"(define-command! "lazy-cmd" "doc" (lambda () (+ 1 0)))"#,
    )
    .unwrap();
    let got = exists(
        &mut ed,
        tmp.path(),
        r#"(declare-plugin! "user/tp" #:commands '("lazy-cmd"))"#,
        &["lazy-cmd"],
    );
    assert_eq!(got, [true]);
    assert!(
        ed.state
            .config
            .registry
            .lazy_mappable_owner("lazy-cmd")
            .is_some(),
        "probing must leave the lazy stub in place"
    );
}

#[test]
fn builtin_procedures_exist() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    let got = exists(
        &mut ed,
        tmp.path(),
        "",
        &["hume-version", "hume-version>=?"],
    );
    assert_eq!(got, [true, true]);
}

#[test]
fn macros_and_unknown_names_do_not_exist() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    let got = exists(&mut ed, tmp.path(), "", &["call!", "no-such-thing-xyz"]);
    assert_eq!(got, [false, false]);
}

#[test]
fn typed_only_command_does_not_exist() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    let got = exists(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "typed-only" "" (lambda (bid) (+ 1 0)))"#,
        &["typed-only"],
    );
    assert_eq!(got, [false]);
}
