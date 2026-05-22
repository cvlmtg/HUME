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
    // Hold HUME_RUNTIME_MUTEX while creating the temp dir so that a concurrent
    // HumeRuntimeGuard test cannot redirect TMPDIR and nest our dir inside its
    // managed cleanup tree (which it deletes when the guard drops).
    let dir = {
        let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        tempfile::tempdir().unwrap()
    };
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
    let dir = {
        let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        tempfile::tempdir().unwrap()
    };
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

// ── Phase 2 lazy plugin loading — event triggers ──────────────────────────────

/// `#:on-event` plugin activates on first matching hook fire; its handler
/// runs in the same fire that triggered activation.
///
/// Flip: without A3 (`activate_lazy_event_plugins` at the top of
/// `fire_hook_silent`), the plugin stays `Declared` and the cursor never moves.
#[test]
#[cfg(not(windows))]
fn event_trigger_activates_on_first_fire() {
    use crate::scripting::attribution::PluginId;
    use crate::scripting::lazy::PluginState;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(load-plugin "user/tp" #:on-event '("on-buffer-save"))"#,
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right")))"#,
    );
    let id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id),
            Some(PluginState::Declared { .. })
        ),
        "plugin must be Declared before first fire"
    );
    assert!(
        !ed.scripting.as_ref().unwrap().lazy_registry.event_triggers.is_empty(),
        "event_triggers must be populated before first fire"
    );

    let before = state(&ed);
    let bid = ed.focused_buffer_id();
    ed.fire_hook_buffer_save(bid);

    assert_ne!(state(&ed), before, "hook handler must run and move the cursor on first fire");
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id),
            Some(PluginState::Loaded)
        ),
        "plugin must be Loaded after first fire"
    );
    assert!(
        ed.scripting.as_ref().unwrap().lazy_registry.event_triggers.is_empty(),
        "event_triggers must be cleared after plugin loads"
    );
}

/// Second fire: handler still runs (plugin already `Loaded`); no re-activation.
///
/// Flip: if `event_triggers` were not cleared after load, `activate_plugin`'s
/// `Loaded` guard would still fire harmlessly — but the test documents that
/// the fast path is taken (no spurious activation attempt).
#[test]
#[cfg(not(windows))]
fn event_trigger_idempotent_on_second_fire() {
    use crate::scripting::attribution::PluginId;
    use crate::scripting::lazy::PluginState;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(load-plugin "user/tp" #:on-event '("on-buffer-save"))"#,
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right")))"#,
    );
    let id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    let bid = ed.focused_buffer_id();

    ed.fire_hook_buffer_save(bid);  // first fire — activates
    assert!(
        ed.scripting.as_ref().unwrap().lazy_registry.event_triggers.is_empty(),
        "event_triggers must be empty after first fire"
    );

    let after_first = state(&ed);
    ed.fire_hook_buffer_save(bid);  // second fire — handler runs, no re-activation

    assert_ne!(state(&ed), after_first, "handler must run again on second fire");
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id),
            Some(PluginState::Loaded)
        ),
        "plugin must remain Loaded after second fire (not re-enter Declared)"
    );
}

/// 1:many: two plugins both declare `#:on-event '("on-buffer-save")`; a single
/// fire activates both.
///
/// Flip: if only the first plugin in the trigger Vec were activated, the second
/// would stay `Declared` with its handler never registering.
#[test]
#[cfg(not(windows))]
fn event_trigger_one_to_many_activates_all() {
    use crate::scripting::attribution::PluginId;
    use crate::scripting::lazy::PluginState;

    let dir = {
        let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        tempfile::tempdir().unwrap()
    };
    let dir_a = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::write(
        dir_a.join("plugin.scm"),
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right")))"#,
    ).unwrap();
    let dir_b = dir.path().join("plugins").join("user").join("tp2");
    std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(
        dir_b.join("plugin.scm"),
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right")))"#,
    ).unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(load-plugin \"user/tp\"  #:on-event '(\"on-buffer-save\"))\n\
         (load-plugin \"user/tp2\" #:on-event '(\"on-buffer-save\"))",
    ).unwrap();

    let mut ed = editor_from("-[a]>b c d\n");
    let mut host = ScriptingHost::new();
    host.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    host.eval_init(&init_path, &mut s, &mut km, Default::default())
        .expect("eval_init must succeed");
    ed.scripting = Some(host);

    let id_a = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    let id_b = PluginId::User { user: "user".to_string(), repo: "tp2".to_string() };
    let bid = ed.focused_buffer_id();
    ed.fire_hook_buffer_save(bid);

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id_a),
            Some(PluginState::Loaded)
        ),
        "plugin A must be Loaded after fire"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id_b),
            Some(PluginState::Loaded)
        ),
        "plugin B must be Loaded after fire"
    );
    assert!(
        ed.scripting.as_ref().unwrap().lazy_registry.event_triggers.is_empty(),
        "event_triggers must be fully cleared after both plugins load"
    );
}

/// Body error: plugin raises at load time → `Failed`, error reported, trigger
/// cleared — no retry on a second fire.
///
/// Flip: without `event_triggers` drop in `activate_plugin`'s failure branch,
/// the same plugin would attempt activation on every fire.
#[test]
#[cfg(not(windows))]
fn event_plugin_failure_marks_failed_no_retry() {
    use crate::scripting::attribution::PluginId;
    use crate::scripting::lazy::PluginState;
    use crate::editor::Severity;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(load-plugin "user/tp" #:on-event '("on-buffer-save"))"#,
        r#"(error "intentional plugin failure")"#,
    );
    let id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    let bid = ed.focused_buffer_id();

    ed.fire_hook_buffer_save(bid);  // first fire — activates → body fails

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id),
            Some(PluginState::Failed)
        ),
        "plugin must be Failed after body error"
    );
    assert!(
        ed.scripting.as_ref().unwrap().lazy_registry.event_triggers.is_empty(),
        "event_triggers must be cleared even after failure"
    );
    assert!(
        ed.message_log.entries().any(|e| e.severity == Severity::Error),
        "Severity::Error must be logged after body failure"
    );

    let msg_count = ed.message_log.entries().count();
    ed.fire_hook_buffer_save(bid);  // second fire — no retry

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id),
            Some(PluginState::Failed)
        ),
        "plugin must remain Failed after second fire (no retry)"
    );
    assert_eq!(
        ed.message_log.entries().count(),
        msg_count,
        "no new error must be logged on second fire (no retry)"
    );
}

// ── Phase 2 — require-plugin (editor-level) ───────────────────────────────────

/// `(require-plugin "name")` in `init.scm` force-activates a bare `#:lazy #t`
/// plugin at init time.
///
/// Flip: without `require-plugin` pushing to `pending_plugin_loads`, the plugin
/// would remain `Declared` after init.
#[test]
#[cfg(not(windows))]
fn require_plugin_loads_bare_lazy() {
    use crate::scripting::attribution::PluginId;
    use crate::scripting::lazy::PluginState;

    let (dir, init_path) = {
        let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("plugins").join("user").join("tp");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.scm"),
            r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
        ).unwrap();
        let init_path = dir.path().join("init.scm");
        std::fs::write(
            &init_path,
            "(load-plugin \"user/tp\" #:lazy #t)\n(require-plugin \"user/tp\")",
        ).unwrap();
        (dir, init_path)
    };

    let mut host = ScriptingHost::new();
    host.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    host.eval_init(&init_path, &mut s, &mut km, Default::default())
        .expect("eval_init must succeed");

    let id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    assert!(
        matches!(host.lazy_registry.plugins.get(&id), Some(PluginState::Loaded)),
        "bare-lazy plugin must be Loaded after (require-plugin) in init.scm; got: {:?}",
        host.lazy_registry.plugins.get(&id)
    );
}

/// `(require-plugin "unknown")` — no prior `load-plugin` — raises a Steel error.
///
/// Flip: if the unknown-name check were removed, a typo'd name would silently
/// queue a no-op activation.
#[test]
#[cfg(not(windows))]
fn require_plugin_unknown_errors() {
    let (dir, init_path) = {
        let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let init_path = dir.path().join("init.scm");
        std::fs::write(&init_path, r#"(require-plugin "user/tp")"#).unwrap();
        (dir, init_path)
    };

    let mut host = ScriptingHost::new();
    host.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let Err(msg) = host.eval_init(&init_path, &mut s, &mut km, Default::default()) else {
        panic!("require-plugin on undeclared plugin must return Err");
    };
    assert!(msg.contains("load-plugin first"), "error must mention load-plugin; got: {msg}");
}

/// `require-plugin` in a lazy plugin body (B) loads a dependency (A)
/// transitively at B's activation time — A is NOT promoted to eager at init.
///
/// Flip: if `activate_plugin` did not drain `pending_plugin_loads` from the
/// body's ctx (B3), A would remain `Declared` after B activates.
/// If A were incorrectly activated at init, it would be `Loaded` before dispatch.
#[test]
#[cfg(not(windows))]
fn require_plugin_transitive_is_lazy() {
    use crate::scripting::attribution::PluginId;
    use crate::scripting::lazy::PluginState;

    let dir = {
        let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        tempfile::tempdir().unwrap()
    };
    // Plugin A — bare-lazy dep.
    let dir_a = dir.path().join("plugins").join("user").join("tpa");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::write(
        dir_a.join("plugin.scm"),
        r#"(define-command! "a-cmd" "doc" (lambda () (+ 1 0)))"#,
    ).unwrap();
    // Plugin B — on-command trigger; body requires A.
    let dir_b = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(
        dir_b.join("plugin.scm"),
        "(require-plugin \"user/tpa\")\n\
         (define-command! \"b-cmd\" \"doc\" (lambda () (call! \"move-right\")))",
    ).unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(load-plugin \"user/tpa\" #:lazy #t)\n\
         (load-plugin \"user/tp\"  #:on-command '(\"b-cmd\"))",
    ).unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    host.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    host.eval_init(&init_path, &mut s, &mut km, Default::default())
        .expect("eval_init must succeed");
    let triggers = host.lazy_registry.command_triggers.clone();
    ed.register_lazy_command_stubs(&triggers);
    ed.scripting = Some(host);

    let id_a = PluginId::User { user: "user".to_string(), repo: "tpa".to_string() };
    let id_b = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };

    // After init: both Declared — A was not eagerly promoted.
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id_a),
            Some(PluginState::Declared { .. })
        ),
        "dep A must be Declared after init (not promoted to eager)"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id_b),
            Some(PluginState::Declared { .. })
        ),
        "plugin B must be Declared after init"
    );

    // Dispatch B's command → B activates → B body requires A → A activates.
    let before = state(&ed);
    type_cmd(&mut ed, ":b-cmd");

    assert_ne!(state(&ed), before, "b-cmd must have moved the cursor after activation");
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id_b),
            Some(PluginState::Loaded)
        ),
        "plugin B must be Loaded after dispatch"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id_a),
            Some(PluginState::Loaded)
        ),
        "dep A must be Loaded transitively after B activates"
    );
}

// ── Phase 3b lazy plugin loading — language/filetype triggers ─────────────────

/// `#:on-language` plugin activates on first matching language set; its
/// `on-language-set` handler runs in the same call that triggered activation.
///
/// Flip: without `activate_lazy_language_plugins` in `set_buffer_language`,
/// the plugin stays `Declared` and the cursor never moves.
#[test]
#[cfg(not(windows))]
fn language_trigger_activates_on_set() {
    use crate::scripting::attribution::PluginId;
    use crate::scripting::lazy::PluginState;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(load-plugin "user/tp" #:on-language '("rust"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    );
    let id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id),
            Some(PluginState::Declared { .. })
        ),
        "plugin must be Declared before first language set"
    );
    assert!(
        ed.scripting.as_ref().unwrap().lazy_registry.language_triggers.contains_key("rust"),
        "language_triggers must be populated before first set"
    );

    let before = state(&ed);
    let bid = ed.focused_buffer_id();
    ed.set_buffer_language(bid, Some("rust".into()));

    assert_ne!(state(&ed), before, "on-language-set handler must run and move cursor on first set");
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id),
            Some(PluginState::Loaded)
        ),
        "plugin must be Loaded after first language set"
    );
    assert!(
        !ed.scripting.as_ref().unwrap().lazy_registry.language_triggers.contains_key("rust"),
        "language_triggers must be cleared after plugin loads"
    );
}

/// Second set to the same language: handler still runs; no re-activation.
///
/// Flip: if `language_triggers` were not cleared on load, a second matching set
/// would attempt activation again — `activate_plugin`'s `Loaded` guard prevents
/// a crash, but the test documents the intended fast path.
#[test]
#[cfg(not(windows))]
fn language_trigger_idempotent_on_round_trip() {
    use crate::scripting::attribution::PluginId;
    use crate::scripting::lazy::PluginState;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(load-plugin "user/tp" #:on-language '("rust"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    );
    let id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    let bid = ed.focused_buffer_id();

    ed.set_buffer_language(bid, Some("rust".into()));  // first set — activates
    assert!(
        !ed.scripting.as_ref().unwrap().lazy_registry.language_triggers.contains_key("rust"),
        "language_triggers must be empty after first set"
    );

    let after_first = state(&ed);
    ed.set_buffer_language(bid, Some("toml".into()));  // round-trip out
    ed.set_buffer_language(bid, Some("rust".into()));  // round-trip back — handler runs, no re-activation

    assert_ne!(state(&ed), after_first, "handler must run again on second rust set");
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id),
            Some(PluginState::Loaded)
        ),
        "plugin must remain Loaded after round-trip (not re-enter Declared or fail)"
    );
    assert!(
        !ed.scripting.as_ref().unwrap().lazy_registry.language_triggers.contains_key("rust"),
        "language_triggers must remain cleared after round-trip"
    );
}

/// 1:many: two plugins both declare `#:on-language '("rust")`; a single language
/// set activates both.
///
/// Flip: if only the first plugin in the trigger Vec were activated, the second
/// would stay `Declared` with its handler never registering.
#[test]
#[cfg(not(windows))]
fn language_trigger_one_to_many_activates_all() {
    use crate::scripting::attribution::PluginId;
    use crate::scripting::lazy::PluginState;

    let dir = {
        let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        tempfile::tempdir().unwrap()
    };
    let dir_a = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::write(
        dir_a.join("plugin.scm"),
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    ).unwrap();
    let dir_b = dir.path().join("plugins").join("user").join("tp2");
    std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(
        dir_b.join("plugin.scm"),
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    ).unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(load-plugin \"user/tp\"  #:on-language '(\"rust\"))\n\
         (load-plugin \"user/tp2\" #:on-language '(\"rust\"))",
    ).unwrap();

    let mut ed = editor_from("-[a]>b c d\n");
    let mut host = ScriptingHost::new();
    host.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    host.eval_init(&init_path, &mut s, &mut km, Default::default())
        .expect("eval_init must succeed");
    ed.scripting = Some(host);

    let id_a = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    let id_b = PluginId::User { user: "user".to_string(), repo: "tp2".to_string() };
    let bid = ed.focused_buffer_id();
    ed.set_buffer_language(bid, Some("rust".into()));

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id_a),
            Some(PluginState::Loaded)
        ),
        "plugin A must be Loaded after language set"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id_b),
            Some(PluginState::Loaded)
        ),
        "plugin B must be Loaded after language set"
    );
    assert!(
        !ed.scripting.as_ref().unwrap().lazy_registry.language_triggers.contains_key("rust"),
        "language_triggers must be fully cleared after both plugins load"
    );
}

/// Language set for an unregistered language → plugin stays `Declared`.
///
/// Flip: if `activate_lazy_language_plugins` looked up the wrong map or iterated
/// unconditionally, the plugin would load on any language set.
#[test]
#[cfg(not(windows))]
fn language_trigger_does_not_fire_on_unrelated_language() {
    use crate::scripting::attribution::PluginId;
    use crate::scripting::lazy::PluginState;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(load-plugin "user/tp" #:on-language '("rust"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    );
    let id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    let bid = ed.focused_buffer_id();

    ed.set_buffer_language(bid, Some("toml".into()));  // unrelated language

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().lazy_registry.plugins.get(&id),
            Some(PluginState::Declared { .. })
        ),
        "plugin must stay Declared when an unrelated language is set"
    );
    assert!(
        ed.scripting.as_ref().unwrap().lazy_registry.language_triggers.contains_key("rust"),
        "language_triggers[\"rust\"] must remain intact after an unrelated set"
    );
}

// ── Phase 4 Polish — load-time activation reporting ──────────────────────────

/// Command trigger: first dispatch of a lazy command logs a Trace entry naming
/// the triggering command.
///
/// Flip: before dispatch, no such Trace exists — confirming the entry is
/// produced by the activation path, not during init.
#[test]
#[cfg(not(windows))]
fn command_trigger_logs_trace_on_activation() {
    use crate::editor::Severity;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(load-plugin "user/tp" #:on-command '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );

    assert!(
        !ed.message_log
            .entries()
            .any(|e| e.severity == Severity::Trace && e.text.contains("command trigger")),
        "no activation Trace before dispatch; messages: {:?}",
        ed.message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );

    type_cmd(&mut ed, ":bar");

    assert!(
        ed.message_log.entries().any(|e| {
            e.severity == Severity::Trace
                && e.text.contains("bar")
                && e.text.contains("command trigger")
        }),
        "expected Trace entry naming command trigger 'bar' after dispatch; messages: {:?}",
        ed.message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

// ── Phase 4 Polish — post-init keymap lint ────────────────────────────────────

/// Helper: write `init_scm` to a temporary config dir, set `XDG_CONFIG_HOME`
/// and `HUME_RUNTIME`, call `init_scripting` on a fresh Editor, restore env
/// vars before returning.  Caller must keep the returned `Vec<TempDir>` alive.
#[cfg(not(windows))]
fn setup_editor_with_init_scripting(init_scm: &str) -> (Editor, Vec<tempfile::TempDir>) {
    let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    let config_tmp = tempfile::tempdir().unwrap();
    let runtime_tmp = tempfile::tempdir().unwrap();
    let data_tmp = tempfile::tempdir().unwrap();

    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(hume_config.join("init.scm"), init_scm).unwrap();

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", config_tmp.path());
        std::env::set_var("HUME_RUNTIME", runtime_tmp.path());
        std::env::set_var("XDG_DATA_HOME", data_tmp.path());
    }

    let mut ed = editor_from("-[a]>b\n");
    ed.init_scripting();

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HUME_RUNTIME");
        std::env::remove_var("XDG_DATA_HOME");
    }

    (ed, vec![config_tmp, runtime_tmp, data_tmp])
}

/// Keymap lint warns when a bind-key! targets a name not in the command registry.
///
/// Flip: binding to a known command ("move-down") must produce no warning, so
/// the warning here is definitely about the unknown name, not an always-fire.
#[test]
#[cfg(not(windows))]
fn keymap_lint_warns_on_unknown_command() {
    use crate::editor::Severity;

    let (ed, _dirs) = setup_editor_with_init_scripting(
        r#"(bind-key! "normal" "Q" "bogus-unknown-cmd")"#,
    );

    assert!(
        ed.message_log.entries().any(|e| {
            e.severity == Severity::Warning && e.text.contains("bogus-unknown-cmd")
        }),
        "expected Warning about unknown command 'bogus-unknown-cmd'; messages: {:?}",
        ed.message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// Keymap lint is silent when every bound key targets a registered command.
///
/// Flip: the test above binds an *unknown* name and asserts a Warning is
/// produced — this test confirms the warning path does not fire for valid names.
#[test]
#[cfg(not(windows))]
fn keymap_lint_silent_for_known_command() {
    use crate::editor::Severity;

    let (ed, _dirs) = setup_editor_with_init_scripting(
        r#"(bind-key! "normal" "Q" "move-down")"#,
    );

    assert!(
        !ed.message_log.entries().any(|e| {
            e.severity == Severity::Warning && e.text.contains("move-down")
        }),
        "must not warn about known command 'move-down'; messages: {:?}",
        ed.message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}
