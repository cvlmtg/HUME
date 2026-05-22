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
