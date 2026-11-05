//! A plugin entry declared with `#:languages` acts only on buffers of those
//! languages: every other buffer looks closed to it.

use super::*;

/// Logs every buffer hook it gets as `tp <event> <language>`, plus `tp mode`
/// for the global `on-mode-change`.
const LOGGING_PLUGIN: &str = r#"
(define (tp-lang p)
  (let ([l (get-buffer-option p "language")])
    (if (and (string? l) (not (equal? l ""))) l "none")))
(define (tp-log . parts) (log! 'trace (apply string-append "tp " parts)))
(register-hook! 'on-text-changed (lambda (p changes) (tp-log "text " (tp-lang p))))
(register-hook! 'on-language-set
  (lambda (p l) (tp-log "set " (if (equal? l "") "none" l))))
(register-hook! 'on-buffer-open (lambda (p) (tp-log "open " (tp-lang p))))
(register-hook! 'on-buffer-close (lambda (p) (tp-log "close")))
(register-hook! 'on-buffer-enter (lambda (p) (tp-log "enter " (tp-lang p))))
(register-hook! 'on-mode-change (lambda (from to) (tp-log "mode")))
"#;

fn declared(languages: &str) -> String {
    format!(r#"(declare-plugin! "user/tp" #:languages '({languages}))"#)
}

/// The lines logged since the log held `since` of them.
fn logged_since(ed: &Editor, since: usize) -> Vec<String> {
    tp_log(ed).split_off(since)
}

/// A focused buffer whose open the plugins have been told of.
fn opened_buffer(ed: &mut Editor) -> BufferId {
    let bid = open_other_buffer(ed, "a\n");
    ed.settle();
    bid
}

fn set_language(ed: &mut Editor, bid: BufferId, language: Option<&str>) {
    let id = language.map(|name| ed.state.config.languages.intern(name));
    ed.set_buffer_language(bid, id).expect_open();
    ed.settle();
}

/// Types into the focused buffer, settling once: the mode round trip nets
/// out, so only the text change is announced.
fn type_x(ed: &mut Editor) {
    type_chars(ed, "ix");
    ed.feed_key(key_esc());
    ed.settle();
}

/// Enters Insert mode and leaves it, settling after each, so both mode
/// changes are announced.
fn insert_round_trip(ed: &mut Editor) {
    ed.feed_key(key('i'));
    ed.settle();
    ed.feed_key(key_esc());
    ed.settle();
}

fn switch_to(ed: &mut Editor, bid: BufferId) {
    ed.enter_buffer(FocusedPane::current(&ed.view), bid);
}

/// `user/tp`, running `plugin_body`, declared for `languages` and
/// activated by setting a focused, opened buffer to `first`. Returns that
/// buffer.
fn activated(
    languages: &str,
    first: &str,
    plugin_body: &str,
) -> (Editor, tempfile::TempDir, BufferId) {
    let (mut ed, dir) = setup_lazy_editor(&declared(languages), plugin_body);
    let bid = opened_buffer(&mut ed);
    set_language(&mut ed, bid, Some(first));
    (ed, dir, bid)
}

/// [`activated`] with [`LOGGING_PLUGIN`], also returning how many lines
/// the plugin had logged.
fn activated_on(languages: &str, first: &str) -> (Editor, tempfile::TempDir, BufferId, usize) {
    let (ed, dir, bid) = activated(languages, first, LOGGING_PLUGIN);
    let logged = tp_log(&ed).len();
    (ed, dir, bid, logged)
}

// ── Buffer hooks ──────────────────────────────────────────────────────

#[test]
fn a_scoped_plugin_gets_no_text_change_for_a_buffer_of_another_language() {
    let (mut ed, _dir, bid, start) = activated_on(r#""rust""#, "rust");
    type_x(&mut ed);
    set_language(&mut ed, bid, Some("markdown"));
    type_x(&mut ed);
    assert_eq!(logged_since(&ed, start), ["text rust", "close"]);
}

#[test]
fn a_scoped_plugin_gets_no_hook_for_a_buffer_without_a_language() {
    let (mut ed, _dir, bid, start) = activated_on(r#""rust""#, "rust");
    set_language(&mut ed, bid, None);
    type_x(&mut ed);
    assert_eq!(logged_since(&ed, start), ["close"]);
}

#[test]
fn a_pane_event_follows_the_scope_of_the_buffer_the_pane_shows() {
    let (mut ed, _dir, rust, start) = activated_on(r#""rust""#, "rust");
    opened_buffer(&mut ed);
    switch_to(&mut ed, rust);
    ed.settle();
    assert_eq!(logged_since(&ed, start), ["enter rust"]);
}

#[test]
fn the_close_of_a_buffer_outside_the_scope_does_not_reach_the_plugin() {
    let (mut ed, _dir, rust, start) = activated_on(r#""rust""#, "rust");
    let other = opened_buffer(&mut ed);
    switch_to(&mut ed, rust);
    ed.close_buffer(other)
        .expect("the buffer has no unsaved changes");
    ed.settle();
    assert_eq!(logged_since(&ed, start), ["enter rust"]);
}

#[test]
fn global_events_reach_a_scoped_plugin() {
    let (mut ed, _dir, bid, start) = activated_on(r#""rust""#, "rust");
    set_language(&mut ed, bid, Some("markdown"));
    insert_round_trip(&mut ed);
    assert_eq!(logged_since(&ed, start), ["close", "mode", "mode"]);
}

#[test]
fn a_wildcard_plugin_sees_every_buffer_including_one_without_a_language() {
    let (mut ed, _dir, bid, start) = activated_on(r#""*""#, "rust");
    set_language(&mut ed, bid, None);
    type_x(&mut ed);
    assert_eq!(logged_since(&ed, start), ["set none", "text none"]);
}

#[test]
fn init_scm_hooks_see_every_buffer() {
    let (mut ed, _dir) = setup_lazy_editor(
        &format!(
            r#"{}
               (register-hook! 'on-text-changed (lambda (p c) (log! 'trace "tp init text")))"#,
            declared(r#""rust""#)
        ),
        LOGGING_PLUGIN,
    );
    let bid = opened_buffer(&mut ed);
    set_language(&mut ed, bid, Some("rust"));
    set_language(&mut ed, bid, Some("markdown"));
    let start = tp_log(&ed).len();
    type_x(&mut ed);
    assert_eq!(logged_since(&ed, start), ["init text"]);
}

#[test]
fn an_eager_plugin_sees_every_buffer() {
    let (mut ed, _dir) = setup_lazy_editor(r#"(load-plugin! "user/tp")"#, LOGGING_PLUGIN);
    let bid = opened_buffer(&mut ed);
    set_language(&mut ed, bid, Some("markdown"));
    let start = tp_log(&ed).len();
    type_x(&mut ed);
    assert_eq!(logged_since(&ed, start), ["text markdown"]);
}

#[test]
fn a_user_declaration_replaces_the_manifest_scope() {
    let (mut ed, _dir) = setup_plugin_editor_on(
        "-[a]>b\n",
        Some(r#"(declare-plugin! "user/tp" #:languages '("rust"))"#),
        &format!("{}\n(load-plugin! \"user/tp\")", declared(r#""markdown""#)),
        LOGGING_PLUGIN,
    );
    let bid = opened_buffer(&mut ed);
    set_language(&mut ed, bid, Some("markdown"));
    type_x(&mut ed);
    set_language(&mut ed, bid, Some("rust"));
    type_x(&mut ed);
    assert_eq!(
        tp_log(&ed),
        ["set markdown", "open markdown", "text markdown", "close"]
    );
}

/// A hook a plugin body registers through another owner's command acts on
/// the buffers that owner's code does: here `init.scm`'s, every buffer.
#[test]
fn a_hook_registered_by_another_owners_command_follows_that_owners_scope() {
    let (mut ed, _dir) = setup_lazy_editor(
        &format!(
            r#"(define-command! "user-hook" "Register a hook."
                 (lambda () (register-hook! 'on-text-changed
                   (lambda (p c) (log! 'trace "tp user text")))))
               {}"#,
            declared(r#""rust""#)
        ),
        &format!("{LOGGING_PLUGIN}\n(call! \"user-hook\")"),
    );
    let bid = opened_buffer(&mut ed);
    set_language(&mut ed, bid, Some("rust"));
    set_language(&mut ed, bid, Some("markdown"));
    let start = tp_log(&ed).len();
    type_x(&mut ed);
    assert_eq!(logged_since(&ed, start), ["user text"]);
}

// ── A language change ─────────────────────────────────────────────────

#[test]
fn an_entry_activated_by_the_change_gets_language_set_then_open() {
    let (ed, _dir, _bid, _start) = activated_on(r#""rust""#, "rust");
    assert_eq!(tp_log(&ed), ["set rust", "open rust"]);
}

#[test]
fn an_entry_activated_by_the_change_gets_the_close_of_the_buffer_it_opened() {
    let (mut ed, _dir, bid, start) = activated_on(r#""rust""#, "rust");
    ed.close_buffer(bid)
        .expect("the buffer has no unsaved changes");
    ed.settle();
    assert_eq!(
        logged_since(&ed, start)
            .iter()
            .filter(|line| *line == "close")
            .count(),
        1
    );
}

#[test]
fn a_buffer_leaving_the_scope_gets_close_and_no_language_set() {
    let (mut ed, _dir, bid, start) = activated_on(r#""rust""#, "rust");
    set_language(&mut ed, bid, Some("markdown"));
    assert_eq!(logged_since(&ed, start), ["close"]);
}

#[test]
fn a_buffer_entering_the_scope_gets_language_set_then_open() {
    let (mut ed, _dir, bid, _start) = activated_on(r#""rust""#, "rust");
    set_language(&mut ed, bid, Some("markdown"));
    let start = tp_log(&ed).len();
    set_language(&mut ed, bid, Some("rust"));
    assert_eq!(logged_since(&ed, start), ["set rust", "open rust"]);
}

#[test]
fn a_buffer_staying_in_scope_gets_only_language_set() {
    let (mut ed, _dir, bid, start) = activated_on(r#""rust" "toml""#, "rust");
    set_language(&mut ed, bid, Some("toml"));
    assert_eq!(logged_since(&ed, start), ["set toml"]);
}

#[test]
fn a_newly_opened_file_in_scope_gets_language_set_then_one_open() {
    let (mut ed, dir) = setup_lazy_editor(&declared(r#""rust""#), LOGGING_PLUGIN);
    ed.state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .expect("a test identity registers");
    let first = opened_buffer(&mut ed);
    set_language(&mut ed, first, Some("rust"));
    let start = tp_log(&ed).len();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    open_path(&mut ed, &path);
    ed.settle();
    assert_eq!(logged_since(&ed, start), ["set rust", "open rust"]);
}

#[test]
fn unscoped_plugins_get_no_extra_open_or_close() {
    let (mut ed, _dir, bid, start) = activated_on(r#""*""#, "rust");
    set_language(&mut ed, bid, Some("markdown"));
    set_language(&mut ed, bid, Some("rust"));
    assert_eq!(logged_since(&ed, start), ["set markdown", "set rust"]);
}

#[test]
fn a_buffer_closed_after_leaving_the_scope_closes_once() {
    let (mut ed, _dir, rust, _start) = activated_on(r#""rust""#, "rust");
    let other = opened_buffer(&mut ed);
    set_language(&mut ed, other, Some("rust"));
    set_language(&mut ed, other, Some("markdown"));
    let start = tp_log(&ed).len();
    switch_to(&mut ed, rust);
    ed.close_buffer(other)
        .expect("the buffer has no unsaved changes");
    ed.settle();
    assert_eq!(logged_since(&ed, start), ["enter rust"]);
}

#[test]
fn a_language_change_then_a_close_before_dispatch_closes_once() {
    let (mut ed, _dir, rust, _start) = activated_on(r#""rust""#, "rust");
    let other = opened_buffer(&mut ed);
    set_language(&mut ed, other, Some("rust"));
    let start = tp_log(&ed).len();
    let markdown = ed.state.config.languages.intern("markdown");
    ed.set_buffer_language(other, Some(markdown)).expect_open();
    switch_to(&mut ed, rust);
    ed.close_buffer(other)
        .expect("the buffer has no unsaved changes");
    ed.settle();
    assert_eq!(logged_since(&ed, start), ["close", "enter rust"]);
}

/// The close a buffer leaving the scope delivers can close the buffer: no
/// plugin then hears its language set.
#[test]
fn a_buffer_closed_by_the_leaving_plugin_gets_no_language_set() {
    let (mut ed, _dir) = setup_lazy_editor(
        &format!(
            r#"(register-hook! 'on-language-set
                 (lambda (p l) (log! 'trace (string-append "tp user set " l))))
               {}"#,
            declared(r#""rust""#)
        ),
        r#"(register-hook! 'on-buffer-close
             (lambda (p) (log! 'trace "tp close") (close-buffer! p)))"#,
    );
    let bid = opened_buffer(&mut ed);
    set_language(&mut ed, bid, Some("rust"));
    let start = tp_log(&ed).len();
    set_language(&mut ed, bid, Some("markdown"));
    assert!(ed.state.buffers.try_get(bid).is_none());
    assert_eq!(logged_since(&ed, start), ["close"]);
}

/// The language set a buffer entering the scope delivers can close the
/// buffer: the plugin then gets no open for it.
#[test]
fn a_buffer_closed_by_its_language_set_gets_no_open() {
    let (mut ed, _dir) = setup_lazy_editor(
        &declared(r#""markdown""#),
        r#"(register-hook! 'on-language-set
             (lambda (p l) (log! 'trace "tp set") (close-buffer! p)))
           (register-hook! 'on-buffer-open (lambda (p) (log! 'trace "tp open")))"#,
    );
    let bid = opened_buffer(&mut ed);
    set_language(&mut ed, bid, Some("rust"));
    set_language(&mut ed, bid, Some("markdown"));
    assert!(ed.state.buffers.try_get(bid).is_none());
    assert_eq!(tp_log(&ed), ["set"]);
}

// ── The save that waits for its handlers ──────────────────────────────

/// A `(pane done)` handler that never calls `done`.
const HOLDING_PLUGIN: &str = r#"
(register-hook! 'on-before-buffer-save (lambda (pane done) (log! 'trace "tp held")))
"#;

/// A buffer on a file holding `old`, edited, with `language`, in an editor
/// where `user/tp` is declared for rust and active.
fn edited_file_buffer(language: &str) -> (Editor, tempfile::TempDir, std::path::PathBuf) {
    let (mut ed, dir, _first) = activated(r#""rust""#, "rust", HOLDING_PLUGIN);
    let (bid, path) = edited_notes(&mut ed, dir.path());
    set_language(&mut ed, bid, Some(language));
    (ed, dir, path)
}

#[test]
fn a_scoped_held_handler_does_not_hold_a_buffer_of_another_language() {
    let (mut ed, _dir, path) = edited_file_buffer("markdown");
    type_cmd(&mut ed, ":w");
    ed.settle();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "xold\n");
    assert!(tp_log(&ed).is_empty(), "{:?}", tp_log(&ed));
}

#[test]
fn a_scoped_held_handler_holds_a_buffer_in_its_languages() {
    let (mut ed, _dir, path) = edited_file_buffer("rust");
    type_cmd(&mut ed, ":w");
    ed.settle();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\n");
    assert_eq!(tp_log(&ed), ["held"]);
}

// ── `(buffers)` ───────────────────────────────────────────────────────

/// Logs how many buffers `(buffers)` lists, from a hook on rust buffers'
/// text changes and from a typed command, and how many panes `(panes)`
/// lists, from another typed command.
const COUNTING_PLUGIN: &str = r#"
(define (tp-count where) (log! 'trace (string-append "tp " where " " (number->string (length (buffers))))))
(register-hook! 'on-text-changed (lambda (p c) (tp-count "hook")))
(define-typed-command! "tp-count" "Count buffers." (lambda () (tp-count "command")))
(define-typed-command! "tp-panes" "Count panes."
  (lambda () (log! 'trace (string-append "tp panes " (number->string (length (panes)))))))
"#;

/// Three buffers: the startup one with no language, a markdown one, and a
/// focused rust one that activated `user/tp`. Returns the markdown one.
fn three_buffers(init: &str) -> (Editor, tempfile::TempDir, BufferId) {
    let (mut ed, dir) = setup_lazy_editor(
        &format!("{}\n{init}", declared(r#""rust""#)),
        COUNTING_PLUGIN,
    );
    let markdown = opened_buffer(&mut ed);
    set_language(&mut ed, markdown, Some("markdown"));
    let rust = opened_buffer(&mut ed);
    set_language(&mut ed, rust, Some("rust"));
    (ed, dir, markdown)
}

#[test]
fn buffers_leaves_out_a_buffer_outside_the_callers_languages() {
    let (mut ed, _dir, _markdown) = three_buffers("");
    ed.execute_typed("tp-count", None).expect(":tp-count runs");
    ed.settle();
    assert_eq!(tp_log(&ed), ["command 1"]);
}

#[test]
fn buffers_in_a_scoped_hook_handler_follows_the_handlers_plugin() {
    let (mut ed, _dir, _markdown) = three_buffers("");
    type_x(&mut ed);
    assert_eq!(tp_log(&ed), ["hook 1"]);
}

#[test]
fn buffers_lists_every_buffer_for_init_scm_code() {
    let (mut ed, _dir, _markdown) = three_buffers(
        r#"(define-typed-command! "user-count" "Count buffers."
             (lambda () (log! 'trace (string-append "tp user " (number->string (length (buffers)))))))"#,
    );
    ed.execute_typed("user-count", None)
        .expect(":user-count runs");
    ed.settle();
    assert_eq!(tp_log(&ed), ["user 3"]);
}

#[test]
fn panes_leaves_out_a_pane_on_a_buffer_outside_the_callers_languages() {
    let (mut ed, _dir, markdown) = three_buffers("");
    vsplit_focused(&mut ed);
    switch_to(&mut ed, markdown);
    ed.settle();
    ed.execute_typed("tp-panes", None).expect(":tp-panes runs");
    ed.settle();
    assert_eq!(tp_log(&ed), ["panes 1"]);
}

/// The body runs inside the change that activates it, before the plugin
/// is told the buffer's new language: `(buffers)` reads the language the
/// buffer has.
#[test]
fn buffers_in_the_body_lists_the_buffer_whose_change_activated_it() {
    let (mut ed, _dir) = setup_lazy_editor(
        &declared(r#""rust""#),
        &format!("{COUNTING_PLUGIN}\n(tp-count \"body\")"),
    );
    let bid = opened_buffer(&mut ed);
    set_language(&mut ed, bid, Some("rust"));
    assert_eq!(tp_log(&ed).first().map(String::as_str), Some("body 1"));
}

// ── `(focused-pane)` ──────────────────────────────────────────────────

/// Logs, from a hook about no buffer, whether `(focused-pane #:in-scope #t)`
/// and `(focused-pane)` name a pane.
const FOCUS_PLUGIN: &str = r#"
(define (tp-yes? p) (if p "pane" "none"))
(register-hook! 'on-mode-change
  (lambda (from to)
    (log! 'trace (string-append "tp " (tp-yes? (focused-pane #:in-scope #t))
                                " " (tp-yes? (focused-pane))))))
"#;

#[test]
fn focused_pane_in_scope_is_false_for_a_buffer_outside_the_callers_languages() {
    let (mut ed, _dir, bid) = activated(r#""rust""#, "rust", FOCUS_PLUGIN);
    insert_round_trip(&mut ed);
    set_language(&mut ed, bid, Some("markdown"));
    insert_round_trip(&mut ed);
    assert_eq!(
        tp_log(&ed),
        ["pane pane", "pane pane", "none pane", "none pane"]
    );
}

// ── Buffer completion sources ─────────────────────────────────────────

/// A `'buffer` source that logs each call and asks to be called again on
/// every keystroke, with `.` as its trigger in rust and markdown buffers.
const SOURCE_PLUGIN: &str = r#"
(register-completion-source! "tp-words"
  (lambda (id pane prefix)
    (log! 'trace "tp called")
    (completion-emit! id (list "alpha") #:incomplete #t))
  #:target 'buffer)
(set-completion-triggers! "rust" "tp-words" (list #\.))
(set-completion-triggers! "markdown" "tp-words" (list #\.))
"#;

/// A focused buffer of `language` in an editor where `user/tp`, declared
/// for rust, was activated by another buffer, then Insert mode.
fn inserting_in(language: &str) -> (Editor, tempfile::TempDir, BufferId) {
    let (mut ed, dir, _first) = activated(r#""rust""#, "rust", SOURCE_PLUGIN);
    let bid = opened_buffer(&mut ed);
    set_language(&mut ed, bid, Some(language));
    ed.feed_key(key('i'));
    ed.settle();
    (ed, dir, bid)
}

#[test]
fn ctrl_space_does_not_call_a_scoped_source_in_a_buffer_of_another_language() {
    let (mut ed, _dir, _bid) = inserting_in("markdown");
    ed.feed_key(key_ctrl(' '));
    ed.settle();
    assert!(tp_log(&ed).is_empty(), "{:?}", tp_log(&ed));
    assert_eq!(status(&ed), "no completion sources for this buffer");
}

#[test]
fn a_trigger_char_does_not_call_a_scoped_source_outside_its_languages() {
    let (mut ed, _dir, _bid) = inserting_in("markdown");
    ed.state.status_msg.clear();
    ed.feed_key(key('.'));
    ed.settle();
    assert!(tp_log(&ed).is_empty(), "{:?}", tp_log(&ed));
    assert_eq!(status(&ed), "");
}

#[test]
fn a_scoped_source_is_called_in_its_languages() {
    let (mut ed, _dir, _bid) = inserting_in("rust");
    ed.feed_key(key_ctrl(' '));
    ed.settle();
    assert_eq!(tp_log(&ed), ["called"]);
}

#[test]
fn a_scoped_source_is_not_called_again_once_the_buffer_leaves_its_languages() {
    let (mut ed, _dir, bid) = inserting_in("rust");
    ed.feed_key(key_ctrl(' '));
    ed.settle();
    set_language(&mut ed, bid, Some("markdown"));
    ed.feed_key(key('a'));
    ed.settle();
    assert_eq!(tp_log(&ed), ["called"]);
}
