use super::*;

/// A `#:repeatable` command inside a lazy plugin must be recorded in
/// `last_repeatable_action` on its FIRST dispatch, after the Lazy→SteelBacked
/// activation happens mid-dispatch.
///
/// The re-query in `editor/mod.rs` `dispatch()` reads `meta().repeatable` on the
/// now-SteelBacked entry (not the pre-dispatch Lazy stub, which is never repeatable).
///
/// Fail oracle: if the re-query used the pre-dispatch `Lazy` variant,
/// `last_repeatable_action` would be `None` on first dispatch — `.` is then
/// a no-op and "bar" survives.
///
/// Not on Windows: Scheme `require` strings embed OS paths with forward slashes.
#[test]
fn lazy_repeatable_round_trip() {
    use crate::editor::scripting_setup::make_init_host;
    use hume_scripting::ScriptingHost;

    let dir = safe_tempdir();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(
        plugin_dir.join("plugin.scm"),
        r#"(define-command! "tp-del" "" (lambda () (call! "delete")) #:repeatable #t)"#,
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        r#"(declare-plugin "user/tp" #:commands '("tp-del"))"#,
    )
    .unwrap();

    let mut ed = editor_from("-[foo]> bar\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");
    ed.scripting = Some(host);

    // First dispatch: Lazy miss → plugin activates → SteelBacked runs.
    ed.execute_keymap_command("tp-del".into(), Some(1), false);
    assert_eq!(ed.doc().text().to_string(), " bar\n");

    // The re-query must see the now-activated SteelBacked repeatable entry.
    assert_eq!(
        ed.state
            .last_repeatable_action
            .as_ref()
            .map(|a| a.command.as_ref()),
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
