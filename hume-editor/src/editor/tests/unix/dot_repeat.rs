use super::*;

/// A `#:repeatable` command inside a lazy plugin must be the recorded change
/// on its FIRST dispatch. `Editor::resolve_mappable` activates the plugin
/// before `meta().repeatable` is read, so it reads the activated SteelBacked entry,
/// not the Lazy stub, which is never repeatable. Read from the stub, the
/// first dispatch would record nothing, `.` would do nothing, and "bar"
/// would survive.
///
/// Not on Windows: Scheme `require` strings embed OS paths with forward slashes.
#[test]
fn lazy_repeatable_round_trip() {
    let (mut ed, _dir) = setup_lazy_editor_on(
        "-[foo]> bar\n",
        r#"(declare-plugin! "user/tp" #:commands '("tp-del"))"#,
        r#"(define-command! "tp-del" "" (lambda () (call! "delete" (focused-pane))) #:repeatable #t)"#,
    );

    // First dispatch: Lazy miss → plugin activates → SteelBacked runs.
    ed.execute_keymap_command("tp-del", Count::typed(1), false);
    assert_eq!(ed.doc().text().to_string(), " bar\n");

    // Dispatch must have read the activated SteelBacked repeatable entry.
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("tp-del"),
        "lazy-activated repeatable command must be recorded on first dispatch"
    );

    // `.` must replay via the activated SteelBacked entry.
    ed.feed_key(key('w'));
    ed.feed_key(key('.'));
    assert!(
        !ed.doc().text().to_string().contains("bar"),
        "dot-repeat must replay the lazy-activated Steel command"
    );
}

/// A non-repeatable command inside a lazy plugin is transparent to `.` on its
/// first dispatch, the same as on every later one: the repeatable native
/// command it calls is the recorded change.
#[test]
fn lazy_non_repeatable_wrapper_records_its_inner_change_on_first_dispatch() {
    let (mut ed, _dir) = setup_lazy_editor_on(
        "-[foo]> bar\n",
        r#"(declare-plugin! "user/tp" #:commands '("tp-plain"))"#,
        r#"(define-command! "tp-plain" "" (lambda () (call! "delete" (focused-pane))))"#,
    );

    ed.execute_keymap_command("tp-plain", Count::typed(1), false);
    assert_eq!(ed.doc().text().to_string(), " bar\n");
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("delete")
    );
}

/// A `#:repeatable` command in a lazy plugin, bound to an Insert key, is
/// recorded as repeatable on the press that activates the plugin: its pick
/// joins the change rather than ending it.
#[test]
fn lazy_repeatable_insert_bindings_pick_is_recorded_on_first_press() {
    use crate::editor::replay::Input;

    let (mut ed, _dir) = setup_lazy_editor_on(
        "-[h]>ello\n",
        r#"(declare-plugin! "user/tp" #:commands '("tp-pick"))
           (bind-key! 'insert "ctrl-y" "tp-pick")"#,
        r#"(define-command! "tp-pick" ""
             (lambda (pane)
               (picker! pane (list (cons "a" "A")) (lambda (p) (+ 1 0))))
             #:repeatable #t)"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    assert!(ed.state.input.picker().is_some(), "sanity: picker open");
    ed.feed_key(key_enter());
    ed.settle();
    ed.feed_key(key_esc());

    let transcript = &ed.state.last_action().expect("`i` recorded").transcript;
    assert!(
        matches!(transcript.first(), Some(Input::Binding { name, answers, .. })
            if &**name == "tp-pick" && answers.len() == 1),
        "got {transcript:?}"
    );
}
