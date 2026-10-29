//! Keys bound in a named keymap attached to the focused buffer, which are
//! looked up before the global keymap in every mode.

use super::*;
use crate::editor::tests::before_save::warning_texts;
use hume_core::BindMode;
use pretty_assertions::assert_eq;

/// Bind `keys` to `command` in a keymap attached to the focused buffer.
fn bind_here(ed: &mut Editor, mode: BindMode, keys: &[KeyEvent], command: &'static str) {
    let bid = ed.focused_buffer_id();
    let local = &mut ed.state.config.local_keymaps;
    local.bind(
        "test".into(),
        mode,
        keys,
        command.into(),
        false,
        hume_core::ConfigOrigin::User,
    );
    local.attach(bid, "test").expect("bound above");
}

#[test]
fn a_local_key_applies_only_in_the_attached_buffer() {
    let mut ed = editor_from("a-[b]>c\n");
    bind_here(&mut ed, BindMode::Normal, &[key('l')], "move-left");
    ed.feed_key(key('l'));
    assert_eq!(state(&ed), "-[a]>bc\n");

    open_other_buffer(&mut ed, "xyz\n");
    ed.feed_key(key('l'));
    assert_eq!(state(&ed), "x-[y]>z\n");
}

#[test]
fn a_local_extend_override_runs_without_extending() {
    let mut ed = editor_from("a-[b]>c\n");
    bind_here(&mut ed, BindMode::Extend, &[key('l')], "move-left");
    ed.feed_key(key('e'));
    ed.feed_key(key('l'));
    assert_eq!(state(&ed), "-[a]>bc\n");
}

#[test]
fn a_local_ctrl_binding_runs_as_bound() {
    let mut ed = editor_from("a-[b]>c\n");
    bind_here(&mut ed, BindMode::Normal, &[key_ctrl('y')], "move-left");
    ed.feed_key(key_ctrl('y'));
    assert_eq!(state(&ed), "-[a]>bc\n");
}

#[test]
fn kitty_ctrl_extends_through_a_local_binding() {
    let mut ed = editor_from_kitty("a-[b]>c\n");
    bind_here(&mut ed, BindMode::Normal, &[key('l')], "move-left");
    ed.feed_key(key_ctrl('l'));

    let mut global = editor_from_kitty("a-[b]>c\n");
    global.feed_key(key_ctrl('h'));
    assert_eq!(state(&ed), state(&global));
}

#[test]
fn a_local_insert_key_runs_its_command() {
    let mut ed = editor_from("-[\n]>");
    bind_here(
        &mut ed,
        BindMode::Insert,
        &[key_ctrl('y')],
        "delete-word-backward",
    );
    ed.feed_key(key('i'));
    type_chars(&mut ed, "foo bar");
    ed.feed_key(key_ctrl('y'));
    assert_eq!(ed.doc().text().to_string(), "foo \n");
}

#[test]
fn a_local_insert_key_dismisses_completion() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    type_chars(&mut ed, "pri");
    open_completion_session(&mut ed, &["print"]);
    assert!(ed.state.input.buffer_completion().is_some(), "sanity");
    bind_here(
        &mut ed,
        BindMode::Insert,
        &[key_ctrl('y')],
        "delete-word-backward",
    );

    ed.feed_key(key_ctrl('y'));
    assert!(ed.state.input.buffer_completion().is_none());
    assert_eq!(ed.doc().text().to_string(), "\n");
}

#[test]
fn a_local_command_prompt_key_runs_on_the_command_line() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "put-w" "" (lambda (pane) (paste-text! pane "W")))"#,
    );
    bind_here(&mut ed, BindMode::Command, &[key_ctrl('y')], "put-w");
    ed.feed_key(key(':'));
    type_chars(&mut ed, "ab");
    ed.feed_key(key_ctrl('y'));
    assert_eq!(minibuf_input(&ed), "abW");
}

// ── A key sequence belongs to the buffer it started in ──────────────────────

#[test]
fn a_sequence_and_its_count_do_not_continue_in_another_buffer() {
    let mut ed = editor_from("-[a]>bc\n");
    ed.feed_key(key('3'));
    ed.feed_key(key('g'));
    open_other_buffer(&mut ed, "x\ny\nz\nw\n");
    ed.feed_key(key('j'));
    assert_eq!(state(&ed), "x\n-[y]>\nz\nw\n");
    assert!(ed.state.pending.keys().is_empty());
}

#[test]
fn a_sequence_continues_in_the_buffer_it_started_in() {
    let mut ed = editor_from("x\n-[y]>\n");
    ed.feed_key(key('g'));
    ed.feed_key(key('g'));
    assert_eq!(state(&ed), "-[x]>\ny\n");
}

// ── `#:keymap` from Steel ────────────────────────────────────────────────────

fn attach_here(ed: &mut Editor, name: &str) {
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .local_keymaps
        .attach(bid, name)
        .expect("the keymap was bound into");
}

#[test]
fn a_named_bind_leaves_the_global_keymap_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("a-[b]>c\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'normal "l" "move-left" #:keymap "test-keys")"#,
    );
    assert_eq!(
        ed.state
            .config
            .keymap
            .lookup_command(BindMode::Normal, &[key('l')]),
        Some(("move-right".to_owned(), false))
    );
    attach_here(&mut ed, "test-keys");
    ed.feed_key(key('l'));
    assert_eq!(state(&ed), "-[a]>bc\n");
}

#[test]
fn a_named_unbind_removes_the_binding_from_that_keymap() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("a-[b]>c\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'normal "l" "move-left" #:keymap "test-keys")
           (unbind-key! 'normal "l" #:keymap "test-keys")"#,
    );
    attach_here(&mut ed, "test-keys");
    ed.feed_key(key('l'));
    assert_eq!(state(&ed), "ab-[c]>\n");
}

#[test]
fn a_named_bind_to_an_unknown_command_is_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'normal "z" "no-such-command" #:keymap "test-keys")"#,
    );
    let warnings = warning_texts(&ed);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("unknown command 'no-such-command'")),
        "warnings: {warnings:?}"
    );
}

// ── attach-keymap! / detach-keymap! from Steel ──────────────────────────────

/// `source` evaluated as `init.scm` would be, plus a `test-keys` keymap
/// binding `l` to `move-left`.
fn run_with_test_keys(ed: &mut Editor, tmp: &std::path::Path, source: &str) {
    run(
        ed,
        tmp,
        &format!(
            r#"(bind-key! 'normal "l" "move-left" #:keymap "test-keys")
               {source}"#
        ),
    );
}

fn set_language(ed: &mut Editor, name: &str) {
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern(name);
    ed.set_buffer_language(bid, Some(lang));
    ed.settle();
}

/// The state after pressing `l` in the focused buffer.
fn after_l(ed: &mut Editor) -> String {
    ed.feed_key(key('l'));
    state(ed)
}

const LANGUAGE_RECIPE: &str = r#"
(register-hook! 'on-language-set
  (lambda (pane lang)
    (if (equal? lang "markdown")
        (attach-keymap! pane "test-keys")
        (detach-keymap! pane "test-keys"))))"#;

#[test]
fn the_language_recipe_attaches_for_markdown_and_detaches_on_change() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("a-[b]>c\n");
    run_with_test_keys(&mut ed, tmp.path(), LANGUAGE_RECIPE);

    set_language(&mut ed, "markdown");
    assert_eq!(after_l(&mut ed), "-[a]>bc\n");
    set_language(&mut ed, "rust");
    assert_eq!(after_l(&mut ed), "a-[b]>c\n");
    assert_eq!(warning_texts(&ed), Vec::<String>::new());
}

#[test]
fn a_hook_can_bind_and_attach_in_one_go() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("a-[b]>c\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(register-hook! 'on-language-set
             (lambda (pane lang)
               (bind-key! 'normal "l" "move-left" #:keymap "hook-keys")
               (attach-keymap! pane "hook-keys")))"#,
    );
    set_language(&mut ed, "markdown");
    assert_eq!(after_l(&mut ed), "-[a]>bc\n");
}

#[test]
fn attaching_a_keymap_nothing_is_bound_into_warns_and_changes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("a-[b]>c\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(register-hook! 'on-language-set
             (lambda (pane lang) (attach-keymap! pane "later-keys")))"#,
    );
    set_language(&mut ed, "markdown");
    assert_eq!(after_l(&mut ed), "ab-[c]>\n");
    assert_eq!(
        warning_texts(&ed),
        vec!["attach-keymap!: no keymap named 'later-keys'".to_owned()]
    );
}

#[test]
fn detaching_a_keymap_nothing_is_bound_into_warns() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("a-[b]>c\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(register-hook! 'on-language-set
             (lambda (pane lang) (detach-keymap! pane "no-keys")))"#,
    );
    set_language(&mut ed, "markdown");
    assert_eq!(
        warning_texts(&ed),
        vec!["detach-keymap!: no keymap named 'no-keys'".to_owned()]
    );
}

#[test]
fn detaching_a_keymap_that_is_not_attached_says_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("a-[b]>c\n");
    run_with_test_keys(&mut ed, tmp.path(), LANGUAGE_RECIPE);
    set_language(&mut ed, "rust");
    assert_eq!(after_l(&mut ed), "ab-[c]>\n");
    assert_eq!(warning_texts(&ed), Vec::<String>::new());
}

#[test]
fn a_command_can_bind_into_an_attached_keymap() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("a-[b]>c\n");
    run_with_test_keys(
        &mut ed,
        tmp.path(),
        &format!(
            r#"{LANGUAGE_RECIPE}
               (define-command! "rebind-l" ""
                 (lambda () (bind-key! 'normal "l" "move-line-end" #:keymap "test-keys")))"#
        ),
    );
    set_language(&mut ed, "markdown");
    ed.execute_keymap_command("rebind-l", Count::UNTYPED, false);
    ed.settle();
    ed.feed_key(key('l'));
    let mut global = editor_from("a-[b]>c\n");
    global.execute_keymap_command("move-line-end", Count::UNTYPED, false);
    assert_eq!(state(&ed), state(&global));
}

#[test]
fn a_failed_hook_does_not_attach() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("a-[b]>c\n");
    run_with_test_keys(
        &mut ed,
        tmp.path(),
        r#"(register-hook! 'on-language-set
             (lambda (pane lang) (attach-keymap! pane "test-keys") (error "boom")))"#,
    );
    set_language(&mut ed, "markdown");
    assert_eq!(after_l(&mut ed), "ab-[c]>\n");
}

#[test]
fn closing_a_buffer_forgets_its_keymaps() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("a-[b]>c\n");
    run_with_test_keys(&mut ed, tmp.path(), LANGUAGE_RECIPE);
    set_language(&mut ed, "markdown");
    let bid = ed.focused_buffer_id();
    assert_eq!(ed.state.config.local_keymaps.attached(bid), ["test-keys"]);

    ed.close_buffer(bid)
        .expect("the buffer has no unsaved changes and runs no pipe");
    assert!(ed.state.config.local_keymaps.attached(bid).is_empty());
}

#[test]
fn reloading_the_config_drops_every_keymap() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("a-[b]>c\n");
    run_with_test_keys(&mut ed, tmp.path(), LANGUAGE_RECIPE);
    set_language(&mut ed, "markdown");
    assert_eq!(
        after_l(&mut ed),
        "-[a]>bc\n",
        "sanity: attached before the reset"
    );

    ed.reset_config_state();
    assert_eq!(after_l(&mut ed), "a-[b]>c\n");
}
