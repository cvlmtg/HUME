use super::*;
use hume_scripting::ScriptingHost;

/// When a Lazy command stub is dispatched with extend=true (e.g. Ctrl+key), the
/// injection must forward extend=true to the resolved SteelBacked lambda body.
///
/// The lambda here distinguishes extend by branching: extend=true → move-right,
/// extend=false → move-down. Dispatching with extend=true must move the cursor
/// right, not down.
///
/// Fail oracle: if the Lazy path did not forward extend correctly (e.g. always
/// injected extend=false), the cursor would move down instead of right.
#[test]
fn lazy_command_first_dispatch_forwards_extend() {
    use crate::editor::scripting_setup::make_init_host;

    let dir = safe_tempdir();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(
        plugin_dir.join("plugin.scm"),
        r#"(define-command! "tp-branch" ""
             (lambda (count extend)
               (if extend
                   (call! "move-right")
                   (call! "move-down"))))"#,
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        r#"(declare-plugin "user/tp" #:commands '("tp-branch"))"#,
    )
    .unwrap();

    // Buffer with 2 lines so move-down doesn't go to the structural newline.
    let mut ed = editor_from("-[a]>b\ncd\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");
    ed.scripting = Some(host);

    // Dispatch with extend=true on the first (Lazy) call.
    ed.execute_keymap_command("tp-branch".into(), Some(1), true);

    // move-right advances by 1 char on line 1; move-down would land on line 2.
    // (The inner (call! "move-right") dispatches without extend, so the
    // selection moves rather than grows — extend=true only picks the branch.)
    assert_eq!(
        state(&ed),
        "a-[b]>\ncd\n",
        "extend=true must forward to lambda → move-right, not move-down"
    );
}
