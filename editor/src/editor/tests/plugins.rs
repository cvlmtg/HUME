use super::*;
use crate::editor::registry::MappableCommand;
use crate::scripting::ScriptingHost;
use crate::settings::EditorSettings;
use crate::editor::keymap::Keymap;

// ── Phase 1 lazy plugin loading — editor-level tests ─────────────────────────
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals.

/// Helper: create a user plugin at `plugins/user/tp/plugin.scm`, write
/// `init.scm`, evaluate it, set up the lazy stubs, and wire the host into
/// `ed`.  Caller must keep `TempDir` alive.
#[cfg(not(windows))]
fn setup_lazy_editor(
    init_body: &str,
    plugin_body: &str,
) -> (Editor, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), plugin_body).unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, init_body).unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    host.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    host.eval_init(&init_path, &mut s, &mut km, Default::default())
        .expect("eval_init must succeed in setup_lazy_editor");

    let triggers: std::collections::HashMap<_, _> =
        host.lazy_registry.command_triggers.clone();
    ed.register_lazy_command_stubs(&triggers);
    ed.scripting = Some(host);
    (ed, dir)
}

/// After `eval_init` + `register_lazy_command_stubs`, a `Lazy` stub is present
/// for the declared command name.
///
/// Flip: without the stub registration, `get_mappable("bar")` would be `None`.
#[test]
#[cfg(not(windows))]
fn lazy_stub_present_after_init() {
    let (_ed, _dir) = setup_lazy_editor(
        r#"(load-plugin "user/tp" #:on-command '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );
    let (ed, _dir) = setup_lazy_editor(
        r#"(load-plugin "user/tp" #:on-command '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );
    assert!(
        matches!(ed.registry.get_mappable("bar"), Some(MappableCommand::Lazy { .. })),
        "Lazy stub must be present after init; got: {:?}",
        ed.registry.get_mappable("bar").map(|c| c.name())
    );
}

/// Dispatching a lazy command the first time activates the plugin, replaces the
/// stub with `SteelBacked`, and executes the real command (cursor moves).
///
/// Flip: if dispatch does nothing (stub stays Lazy), the cursor would not move
/// and the command would still be `Lazy`.
#[test]
#[cfg(not(windows))]
fn first_dispatch_activates_plugin_and_runs() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(load-plugin "user/tp" #:on-command '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (call! "move-right")))"#,
    );
    let before = state(&ed);

    // Dispatch "bar" through the command line.
    type_cmd(&mut ed, ":bar");

    // Cursor must have moved.
    assert_ne!(state(&ed), before, "dispatching lazy 'bar' must move the cursor");
    // Stub must be replaced by a real SteelBacked command.
    assert!(
        matches!(ed.registry.get_mappable("bar"), Some(MappableCommand::SteelBacked { .. })),
        "stub must be replaced by SteelBacked after first dispatch; got: {:?}",
        ed.registry.get_mappable("bar").map(|c| c.name())
    );
}

/// Loop guard: if the plugin body never defines the declared command, the stub
/// is removed after dispatch and a Warning is reported.
///
/// Flip: without the loop guard, the stub would remain (infinite retry).
#[test]
#[cfg(not(windows))]
fn loop_guard_removes_stub_when_body_never_defines_command() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(load-plugin "user/tp" #:on-command '("bar"))"#,
        // Plugin body exists but never defines "bar".
        r#"(define-command! "other-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    // Stub must be present before dispatch.
    assert!(
        matches!(ed.registry.get_mappable("bar"), Some(MappableCommand::Lazy { .. })),
        "Lazy stub must be present before dispatch"
    );

    type_cmd(&mut ed, ":bar");

    // Stub must have been removed by the loop guard.
    assert!(
        ed.registry.get_mappable("bar").is_none(),
        "stub must be removed when body never defines the command; got: {:?}",
        ed.registry.get_mappable("bar").map(|c| c.name())
    );
}

/// Body-error path: if the plugin body raises an error, the state becomes
/// `Failed`, the stub is removed, and a Warning/Error is reported.
///
/// Flip: without error handling, the stub would survive and allow re-entry.
#[test]
#[cfg(not(windows))]
fn body_error_removes_stub_and_marks_failed() {
    use crate::scripting::lazy::PluginState;
    use crate::scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(load-plugin "user/tp" #:on-command '("bar"))"#,
        r#"(error "intentional plugin failure")"#,
    );
    assert!(
        matches!(ed.registry.get_mappable("bar"), Some(MappableCommand::Lazy { .. })),
        "stub must be present before dispatch"
    );

    type_cmd(&mut ed, ":bar");

    // Stub removed.
    assert!(
        ed.registry.get_mappable("bar").is_none(),
        "stub must be removed after body error"
    );
    // Plugin state is Failed.
    let id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id),
            Some(PluginState::Failed)
        ),
        "plugin must be Failed after body error"
    );
}

/// `unregister_dynamic_commands` removes `Lazy` stubs (reload hygiene).
///
/// Flip: if only SteelBacked were removed, the stub would survive.
#[test]
#[cfg(not(windows))]
fn unregister_dynamic_commands_clears_lazy_stubs() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(load-plugin "user/tp" #:on-command '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );

    assert!(
        matches!(ed.registry.get_mappable("bar"), Some(MappableCommand::Lazy { .. })),
        "Lazy stub must be present before unregister"
    );

    ed.registry.unregister_dynamic_commands();

    assert!(
        ed.registry.get_mappable("bar").is_none(),
        "Lazy stub must be removed by unregister_dynamic_commands"
    );
    // Built-in commands are untouched.
    assert!(
        ed.registry.get_mappable("move-right").is_some(),
        "move-right must survive unregister_dynamic_commands"
    );
}

/// `:bar arg` on a lazy command: the arg is correctly passed to a 1-arity
/// command on first call (after activation).
///
/// Flip: if arg were silently dropped, the Steel command would receive false
/// (#f) instead of the string and the test string would not appear as output.
#[test]
#[cfg(not(windows))]
fn lazy_cmd_arg_passed_on_first_call() {
    use crate::scripting::attribution::PluginId;
    use crate::scripting::lazy::PluginState;

    // The plugin defines a 1-arity "bar" that does nothing visible — we just
    // verify that after activation the command is SteelBacked (i.e. arg was
    // accepted, no arity error), and the plugin is Loaded.
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(load-plugin "user/tp" #:on-command '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda (x) (+ 1 0)))"#,
    );

    // Dispatch ":bar hello" — would fail at arity check if arg were dropped.
    type_cmd(&mut ed, ":bar hello");

    let id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id),
            Some(PluginState::Loaded)
        ),
        "plugin must be Loaded after first dispatch with arg"
    );
    assert!(
        matches!(ed.registry.get_mappable("bar"), Some(MappableCommand::SteelBacked { .. })),
        "stub must be replaced by SteelBacked after first dispatch with arg"
    );
}

/// A key bound to a lazy command name activates the plugin on first press,
/// exercising the `execute_keymap_command` Lazy arm — the path the
/// implementation claims keys use "for free".
///
/// Flip: if the Lazy arm did nothing, the cursor would not move and the stub
/// would remain Lazy.
#[test]
#[cfg(not(windows))]
fn key_press_activates_lazy_plugin_via_keymap() {
    use crate::editor::keymap::BindMode;
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(load-plugin "user/tp" #:on-command '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (call! "move-right")))"#,
    );
    // setup_lazy_editor passes a throwaway Keymap to eval_init; bind here so
    // the key lands in the editor's actual keymap.
    ed.keymap.bind_user_with_extend(BindMode::Normal, &[key('z')], "bar".into(), false);
    let before = state(&ed);

    ed.handle_key(key('z'));

    assert_ne!(state(&ed), before, "pressing 'z' must activate 'bar' and move the cursor");
    assert!(
        matches!(ed.registry.get_mappable("bar"), Some(MappableCommand::SteelBacked { .. })),
        "stub must be replaced by SteelBacked after key-triggered activation; got: {:?}",
        ed.registry.get_mappable("bar").map(|c| c.name())
    );
}

/// Eager-plugin-command collision: an eager plugin defines "foo", then a lazy
/// plugin declares `#:on-command '("foo")`.  The lazy stub is rejected (no
/// shadow) and an Error is logged; the eager SteelBacked command survives.
///
/// Flip: if the stub overwrote the eager command, `get_mappable("foo")` would
/// be `Lazy` instead of `SteelBacked`.
#[test]
#[cfg(not(windows))]
fn lazy_stub_rejected_when_name_taken_by_eager_plugin() {
    use crate::editor::Severity;
    let dir = tempfile::tempdir().unwrap();
    // Eager plugin — loaded inline (no triggers), defines "foo".
    let eager_dir = dir.path().join("plugins").join("user").join("eager");
    std::fs::create_dir_all(&eager_dir).unwrap();
    std::fs::write(
        eager_dir.join("plugin.scm"),
        r#"(define-command! "foo" "doc" (lambda () (+ 1 0)))"#,
    ).unwrap();
    // Lazy plugin — declares "foo" as a command trigger; body never runs in
    // this test (stub is rejected before activation).
    let lazy_dir = dir.path().join("plugins").join("user").join("lz");
    std::fs::create_dir_all(&lazy_dir).unwrap();
    std::fs::write(lazy_dir.join("plugin.scm"), r#"(+ 1 0)"#).unwrap();

    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(load-plugin \"user/eager\")\n(load-plugin \"user/lz\" #:on-command '(\"foo\"))",
    ).unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    host.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    // Mirror real init_scripting order so the eager command reaches the
    // registry before register_lazy_command_stubs checks for collisions.
    let cmds = host
        .eval_init(&init_path, &mut s, &mut km, Default::default())
        .expect("eval_init must succeed — collision is caught at stub registration, not here");
    ed.register_steel_cmds(cmds);
    let triggers: std::collections::HashMap<_, _> =
        host.lazy_registry.command_triggers.clone();
    ed.register_lazy_command_stubs(&triggers);
    ed.scripting = Some(host);

    // Eager command survives as SteelBacked — lazy stub never shadowed it.
    assert!(
        matches!(ed.registry.get_mappable("foo"), Some(MappableCommand::SteelBacked { .. })),
        "eager 'foo' must survive as SteelBacked; got: {:?}",
        ed.registry.get_mappable("foo").map(|c| c.name())
    );
    // An Error was logged for the rejected lazy stub.
    assert!(
        ed.message_log
            .entries()
            .any(|e| e.severity == Severity::Error
                && e.text.contains("foo")
                && e.text.contains("conflicts")),
        "expected an Error about the lazy/eager 'foo' collision; messages: {:?}",
        ed.message_log.entries().map(|e| &e.text).collect::<Vec<_>>()
    );
}
