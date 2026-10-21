//! The prelude's `insert-register` command and its `Ctrl-r` bindings in
//! Insert mode and the command mode prompt.

use std::path::{MAIN_SEPARATOR, Path};

use super::editor_cwd::editor_in;
use super::scripting_grammar::prelude_source;
use super::*;
use pretty_assertions::assert_eq;

/// Evaluates the repo's prelude, then `extra`, and installs the host.
fn with_prelude(ed: &mut Editor, tmp: &Path, extra: &str) {
    run(ed, tmp, &format!("{}\n{extra}", prelude_source()));
}

/// An editor on `rel` (holding `x\n`) inside a fresh working directory, with
/// the prelude loaded.
fn editor_on_file(rel: &Path) -> (Editor, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join(rel);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "x\n").unwrap();
    let mut ed = editor_in(tmp.path());
    type_cmd(&mut ed, &format!(":e {}", rel.display()));
    with_prelude(&mut ed, tmp.path(), "");
    (ed, tmp)
}

/// `Ctrl-r` then `name`, letting the read's callback run.
fn ctrl_r(ed: &mut Editor, name: KeyEvent) {
    ed.feed_key(key_ctrl('r'));
    ed.feed_key(name);
    ed.settle();
}

// ── Paths ───────────────────────────────────────────────────────────────────

#[test]
fn f_inserts_the_file_path_relative_to_the_working_directory() {
    let rel = Path::new("sub").join("f.rs");
    let (mut ed, _tmp) = editor_on_file(&rel);
    ed.feed_key(key('i'));
    ctrl_r(&mut ed, key('f'));
    assert_eq!(text(&ed), format!("{}x\n", rel.display()));
}

#[test]
fn p_inserts_the_directory_with_a_trailing_separator() {
    let (mut ed, _tmp) = editor_on_file(&Path::new("sub").join("f.rs"));
    ed.feed_key(key('i'));
    ctrl_r(&mut ed, key('p'));
    assert_eq!(text(&ed), format!("sub{MAIN_SEPARATOR}x\n"));
}

#[test]
fn p_for_a_file_in_the_working_directory_inserts_dot() {
    let (mut ed, _tmp) = editor_on_file(Path::new("f.rs"));
    ed.feed_key(key('i'));
    ctrl_r(&mut ed, key('p'));
    assert_eq!(text(&ed), format!(".{MAIN_SEPARATOR}x\n"));
}

#[test]
fn p_on_the_command_line_inserts_the_directory() {
    let (mut ed, _tmp) = editor_on_file(&Path::new("sub").join("f.rs"));
    ed.feed_key(key(':'));
    type_chars(&mut ed, "e ");
    ctrl_r(&mut ed, key('p'));
    assert_eq!(minibuf_input(&ed), format!("e sub{MAIN_SEPARATOR}"));
}

#[test]
fn f_in_a_scratch_buffer_inserts_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    with_prelude(&mut ed, tmp.path(), "");
    ed.feed_key(key('i'));
    ctrl_r(&mut ed, key('f'));
    assert_eq!(text(&ed), "hello\n");
    assert!(!reading_char(&ed));
}

// ── Registers ───────────────────────────────────────────────────────────────

/// Register `0` holds `ab` then `cd`, one value per yanked selection, and
/// a single cursor sits on `a`.
fn editor_with_two_value_register() -> (Editor, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[ab]> -{cd}>\n");
    with_prelude(&mut ed, tmp.path(), "");
    ed.feed_key(key('"'));
    ed.feed_key(key('0'));
    ed.feed_key(key('y'));
    select_marked(&mut ed, "-[a]>b cd\n");
    (ed, tmp)
}

#[test]
fn a_register_name_at_one_cursor_inserts_its_values_joined() {
    let (mut ed, _tmp) = editor_with_two_value_register();
    ed.feed_key(key('i'));
    ctrl_r(&mut ed, key('0'));
    assert_eq!(text(&ed), "abcdab cd\n");
}

#[test]
fn a_register_name_gives_each_of_as_many_cursors_its_own_value() {
    let (mut ed, _tmp) = editor_with_two_value_register();
    select_marked(&mut ed, "-[a]>b -{c}>d\n");
    ed.feed_key(key('i'));
    ctrl_r(&mut ed, key('0'));
    assert_eq!(text(&ed), "abab cdcd\n");
}

#[test]
fn a_register_name_on_the_command_line_inserts_its_values_joined() {
    let (mut ed, _tmp) = editor_with_two_value_register();
    ed.feed_key(key(':'));
    ctrl_r(&mut ed, key('0'));
    assert_eq!(minibuf_input(&ed), "abcd");
}

#[test]
fn a_register_inserts_the_text_paste_before_gives() {
    for marked in ["-[a]>b cd\n", "-[a]>b -{c}>d\n"] {
        let (mut typed, _tmp) = editor_with_two_value_register();
        select_marked(&mut typed, marked);
        typed.feed_key(key('i'));
        ctrl_r(&mut typed, key('0'));

        let (mut pasted, _tmp) = editor_with_two_value_register();
        select_marked(&mut pasted, marked);
        for k in ['"', '0', 'P'] {
            pasted.feed_key(key(k));
        }

        assert_eq!(text(&typed), text(&pasted), "selections: {marked}");
    }
}

#[test]
fn a_key_that_names_no_register_reports_it_and_inserts_nothing() {
    let (mut ed, _tmp) = editor_with_two_value_register();
    ed.feed_key(key('i'));
    ctrl_r(&mut ed, key('x'));
    assert_eq!(text(&ed), "ab cd\n");
    assert!(!reading_char(&ed));
    let msg = status(&ed);
    assert!(
        msg.contains("insert-register: invalid register 'x'"),
        "got: {msg}"
    );
    assert!(!msg.contains("read-register"), "got: {msg}");
}

#[test]
fn escape_after_ctrl_r_inserts_nothing_and_stays_in_insert() {
    let (mut ed, _tmp) = editor_with_two_value_register();
    ed.feed_key(key('i'));
    ctrl_r(&mut ed, key_esc());
    assert_eq!(text(&ed), "ab cd\n");
    assert_eq!(ed.state.mode(), Mode::Insert);
}

#[test]
fn dot_repeats_the_inserted_register_without_reading_a_key() {
    let (mut ed, _tmp) = editor_with_two_value_register();
    ed.feed_key(key('i'));
    ctrl_r(&mut ed, key('0'));
    ed.feed_key(key_esc());
    select_marked(&mut ed, "abcdab -[c]>d\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert!(!reading_char(&ed));
    assert_eq!(text(&ed), "abcdab abcdcd\n");
}

// ── Bindings ────────────────────────────────────────────────────────────────

#[test]
fn an_init_binding_of_ctrl_r_wins_over_the_prelude() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    with_prelude(
        &mut ed,
        tmp.path(),
        r#"(define-command! "mine" "" (lambda (pane) (paste-text! pane "M")))
           (bind-key! 'insert "ctrl-r" "mine")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('r'));
    assert_eq!(text(&ed), "Mhello\n");
}
