use super::*;
use hume_platform::dirs::Dirs;

/// A `#:repeatable` command inside a lazy plugin must be the recorded change
/// on its FIRST dispatch. `Editor::dispatch` activates the plugin before
/// reading `meta().repeatable`, so it reads the activated SteelBacked entry,
/// not the Lazy stub, which is never repeatable. Read from the stub, the
/// first dispatch would record nothing, `.` would do nothing, and "bar"
/// would survive.
///
/// Not on Windows: Scheme `require` strings embed OS paths with forward slashes.
#[test]
fn lazy_repeatable_round_trip() {
    use hume_scripting::ScriptingHost;

    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(
        plugin_dir.join("plugin.scm"),
        r#"(define-command! "tp-del" "" (lambda () (call! "delete" (focused-pane))) #:repeatable #t)"#,
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        r#"(declare-plugin! "user/tp" #:commands '("tp-del"))"#,
    )
    .unwrap();

    let mut ed = editor_from("-[foo]> bar\n");
    let mut host = ScriptingHost::new(&Dirs {
        data: Some(dir.path().to_path_buf()),
        ..Dirs::none()
    });
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");
    ed.scripting = Some(host);

    // First dispatch: Lazy miss → plugin activates → SteelBacked runs.
    ed.execute_keymap_command("tp-del".into(), Some(1), false);
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
    use hume_scripting::ScriptingHost;

    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(
        plugin_dir.join("plugin.scm"),
        r#"(define-command! "tp-plain" "" (lambda () (call! "delete" (focused-pane))))"#,
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        r#"(declare-plugin! "user/tp" #:commands '("tp-plain"))"#,
    )
    .unwrap();

    let mut ed = editor_from("-[foo]> bar\n");
    let mut host = ScriptingHost::new(&Dirs {
        data: Some(dir.path().to_path_buf()),
        ..Dirs::none()
    });
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");
    ed.scripting = Some(host);

    ed.execute_keymap_command("tp-plain".into(), Some(1), false);
    assert_eq!(ed.doc().text().to_string(), " bar\n");
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("delete")
    );
}
