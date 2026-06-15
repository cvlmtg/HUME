use super::*;
use crate::editor::registry::MappableCommand;
use crate::editor::scripting_setup::make_init_host;
use hume_scripting::{PluginStatus, ScriptingHost, hooks::HookId};

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
    host.set_data_dir(dir.path().to_path_buf());
    { let mut ih = make_init_host(&mut ed.state, &mut ed.view); host.eval_init(&init_path, 10_000, &mut ih, Default::default()) }
        .expect("eval_init must succeed in setup_lazy_editor");

    let triggers: std::collections::HashMap<_, _> =
        host.command_triggers();
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
        r#"(declare-plugin "user/tp" #:on-command '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );
    assert!(
        matches!(ed.state.registry.get_mappable("bar"), Some(MappableCommand::Lazy { .. })),
        "Lazy stub must be present after init; got: {:?}",
        ed.state.registry.get_mappable("bar").map(|c| c.name())
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
        r#"(declare-plugin "user/tp" #:on-command '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (call! "move-right")))"#,
    );
    let before = state(&ed);

    // Dispatch "bar" through the command line.
    type_cmd(&mut ed, ":bar");

    // Cursor must have moved.
    assert_ne!(state(&ed), before, "dispatching lazy 'bar' must move the cursor");
    // Stub must be replaced by a real SteelBacked command.
    assert!(
        matches!(ed.state.registry.get_mappable("bar"), Some(MappableCommand::SteelBacked { .. })),
        "stub must be replaced by SteelBacked after first dispatch; got: {:?}",
        ed.state.registry.get_mappable("bar").map(|c| c.name())
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
        r#"(declare-plugin "user/tp" #:on-command '("bar"))"#,
        // Plugin body exists but never defines "bar".
        r#"(define-command! "other-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    // Stub must be present before dispatch.
    assert!(
        matches!(ed.state.registry.get_mappable("bar"), Some(MappableCommand::Lazy { .. })),
        "Lazy stub must be present before dispatch"
    );

    type_cmd(&mut ed, ":bar");

    // Stub must have been removed by the loop guard.
    assert!(
        ed.state.registry.get_mappable("bar").is_none(),
        "stub must be removed when body never defines the command; got: {:?}",
        ed.state.registry.get_mappable("bar").map(|c| c.name())
    );
}

/// Body-error path: if the plugin body raises an error, the state becomes
/// `Failed`, the stub is removed, and a Warning/Error is reported.
///
/// Flip: without error handling, the stub would survive and allow re-entry.
#[test]
#[cfg(not(windows))]
fn body_error_removes_stub_and_marks_failed() {
    
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:on-command '("bar"))"#,
        r#"(error "intentional plugin failure")"#,
    );
    assert!(
        matches!(ed.state.registry.get_mappable("bar"), Some(MappableCommand::Lazy { .. })),
        "stub must be present before dispatch"
    );

    type_cmd(&mut ed, ":bar");

    // Stub removed.
    assert!(
        ed.state.registry.get_mappable("bar").is_none(),
        "stub must be removed after body error"
    );
    // Plugin state is Failed.
    let id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Failed)
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
        r#"(declare-plugin "user/tp" #:on-command '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );

    assert!(
        matches!(ed.state.registry.get_mappable("bar"), Some(MappableCommand::Lazy { .. })),
        "Lazy stub must be present before unregister"
    );

    ed.state.registry.unregister_dynamic_commands();

    assert!(
        ed.state.registry.get_mappable("bar").is_none(),
        "Lazy stub must be removed by unregister_dynamic_commands"
    );
    // Built-in commands are untouched.
    assert!(
        ed.state.registry.get_mappable("move-right").is_some(),
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
    use hume_scripting::attribution::PluginId;
    

    // The plugin defines a 1-arity "bar" that does nothing visible — we just
    // verify that after activation the command is SteelBacked (i.e. arg was
    // accepted, no arity error), and the plugin is Loaded.
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:on-command '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda (x) (+ 1 0)))"#,
    );

    // Dispatch ":bar hello" — would fail at arity check if arg were dropped.
    type_cmd(&mut ed, ":bar hello");

    let id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Loaded)
        ),
        "plugin must be Loaded after first dispatch with arg"
    );
    assert!(
        matches!(ed.state.registry.get_mappable("bar"), Some(MappableCommand::SteelBacked { .. })),
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
        r#"(declare-plugin "user/tp" #:on-command '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (call! "move-right")))"#,
    );
    // setup_lazy_editor passes a throwaway Keymap to eval_init; bind here so
    // the key lands in the editor's actual keymap.
    ed.state.keymap.bind_user_with_extend(BindMode::Normal, &[key('z')], "bar".into(), false);
    let before = state(&ed);

    ed.handle_key(key('z'));

    assert_ne!(state(&ed), before, "pressing 'z' must activate 'bar' and move the cursor");
    assert!(
        matches!(ed.state.registry.get_mappable("bar"), Some(MappableCommand::SteelBacked { .. })),
        "stub must be replaced by SteelBacked after key-triggered activation; got: {:?}",
        ed.state.registry.get_mappable("bar").map(|c| c.name())
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
        "(load-plugin \"user/eager\")\n(declare-plugin \"user/lz\" #:on-command '(\"foo\"))",
    ).unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    // Mirror real init_scripting order so the eager command reaches the
    // registry before register_lazy_command_stubs checks for collisions.
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed — collision is caught at stub registration, not here");
    let triggers: std::collections::HashMap<_, _> =
        host.command_triggers();
    ed.register_lazy_command_stubs(&triggers);
    ed.scripting = Some(host);

    // Eager command survives as SteelBacked — lazy stub never shadowed it.
    assert!(
        matches!(ed.state.registry.get_mappable("foo"), Some(MappableCommand::SteelBacked { .. })),
        "eager 'foo' must survive as SteelBacked; got: {:?}",
        ed.state.registry.get_mappable("foo").map(|c| c.name())
    );
    // An Error was logged for the rejected lazy stub.
    assert!(
        ed.state.message_log
            .entries()
            .any(|e| e.severity == Severity::Error
                && e.text.contains("foo")
                && e.text.contains("conflicts")),
        "expected an Error about the lazy/eager 'foo' collision; messages: {:?}",
        ed.state.message_log.entries().map(|e| &e.text).collect::<Vec<_>>()
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
    use hume_scripting::attribution::PluginId;
    

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:on-event '("on-buffer-save"))"#,
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right")))"#,
    );
    let id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Declared)
        ),
        "plugin must be Declared before first fire"
    );
    assert!(
        !ed.scripting.as_ref().unwrap().event_trigger_plugins(HookId::OnBufferSave).is_empty(),
        "event_triggers must be populated before first fire"
    );

    let before = state(&ed);
    let bid = ed.focused_buffer_id();
    ed.fire_hook_buffer_save(bid);
    ed.drain_hooks();

    assert_ne!(state(&ed), before, "hook handler must run and move the cursor on first fire");
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Loaded)
        ),
        "plugin must be Loaded after first fire"
    );
    assert!(
        ed.scripting.as_ref().unwrap().event_trigger_plugins(HookId::OnBufferSave).is_empty(),
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
    use hume_scripting::attribution::PluginId;
    

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:on-event '("on-buffer-save"))"#,
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right")))"#,
    );
    let id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    let bid = ed.focused_buffer_id();

    ed.fire_hook_buffer_save(bid);  // first fire — activates
    ed.drain_hooks();
    assert!(
        ed.scripting.as_ref().unwrap().event_trigger_plugins(HookId::OnBufferSave).is_empty(),
        "event_triggers must be empty after first fire"
    );

    let after_first = state(&ed);
    ed.fire_hook_buffer_save(bid);  // second fire — handler runs, no re-activation
    ed.drain_hooks();

    assert_ne!(state(&ed), after_first, "handler must run again on second fire");
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Loaded)
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
    use hume_scripting::attribution::PluginId;
    

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
        "(declare-plugin \"user/tp\"  #:on-event '(\"on-buffer-save\"))\n\
         (declare-plugin \"user/tp2\" #:on-event '(\"on-buffer-save\"))",
    ).unwrap();

    let mut ed = editor_from("-[a]>b c d\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());    { let mut ih = make_init_host(&mut ed.state, &mut ed.view); host.eval_init(&init_path, 10_000, &mut ih, Default::default()) }
        .expect("eval_init must succeed");
    ed.scripting = Some(host);

    let id_a = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    let id_b = PluginId::User { user: "user".to_string(), repo: "tp2".to_string() };
    let bid = ed.focused_buffer_id();
    ed.fire_hook_buffer_save(bid);
    ed.drain_hooks();

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id_a),
            Some(PluginStatus::Loaded)
        ),
        "plugin A must be Loaded after fire"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id_b),
            Some(PluginStatus::Loaded)
        ),
        "plugin B must be Loaded after fire"
    );
    assert!(
        ed.scripting.as_ref().unwrap().event_trigger_plugins(HookId::OnBufferSave).is_empty(),
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
    use hume_scripting::attribution::PluginId;
    
    use crate::editor::Severity;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:on-event '("on-buffer-save"))"#,
        r#"(error "intentional plugin failure")"#,
    );
    let id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    let bid = ed.focused_buffer_id();

    ed.fire_hook_buffer_save(bid);  // first fire — activates → body fails
    ed.drain_hooks();

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Failed)
        ),
        "plugin must be Failed after body error"
    );
    assert!(
        ed.scripting.as_ref().unwrap().event_trigger_plugins(HookId::OnBufferSave).is_empty(),
        "event_triggers must be cleared even after failure"
    );
    assert!(
        ed.state.message_log.entries().any(|e| e.severity == Severity::Error),
        "Severity::Error must be logged after body failure"
    );

    let msg_count = ed.state.message_log.entries().count();
    ed.fire_hook_buffer_save(bid);  // second fire — no retry
    ed.drain_hooks();

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Failed)
        ),
        "plugin must remain Failed after second fire (no retry)"
    );
    assert_eq!(
        ed.state.message_log.entries().count(),
        msg_count,
        "no new error must be logged on second fire (no retry)"
    );
}

// ── Phase 2 — load-plugin / declare-plugin interaction (editor-level) ────────

/// `(declare-plugin "name")` with no triggers is a hard error — the plugin
/// could never activate at runtime.
///
/// Flip: remove the zero-trigger guard in declare_plugin and eval_init succeeds.
#[test]
#[cfg(not(windows))]
fn declare_plugin_no_triggers_is_hard_error() {
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
        std::fs::write(&init_path, "(declare-plugin \"user/tp\")").unwrap();
        (dir, init_path)
    };

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    let mut ed = editor_from("-[a]>b\n");
    let result = {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    };
    assert!(
        result.is_err(),
        "declare-plugin with no triggers must abort init with an error"
    );
}

/// Top-level `(load-plugin "user/tp")` with the plugin absent on disk → silent
/// skip (PLUM-friendly bootstrap), no error, and the plugin is recorded in
/// `declared-plugins` but absent from `loaded-plugins`.
///
/// Flip: if this errored, users could not declare third-party plugins before
/// running `:plum-install` on a fresh setup.
#[test]
#[cfg(not(windows))]
fn load_plugin_absent_top_level_silently_skips() {
    

    let (dir, init_path) = {
        let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // No plugin directory created — plugin is absent on disk.
        let init_path = dir.path().join("init.scm");
        std::fs::write(&init_path, r#"(load-plugin "user/tp")"#).unwrap();
        (dir, init_path)
    };

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    let mut ed = editor_from("-[a]>b\n");
    { let mut ih = make_init_host(&mut ed.state, &mut ed.view); host.eval_init(&init_path, 10_000, &mut ih, Default::default()) }
        .expect("absent top-level load-plugin must not error");

    // Plugin was not inserted into lazy_registry (absent on disk).
    assert!(!host.has_any_loaded_plugin(), "no plugin must be Loaded when absent on disk");
}

/// A lazy plugin B can call another lazy plugin A's command via `(call! "a-cmd")`.
/// The inline lazy-miss retry in `%dispatch-command` activates A on the fly and
/// runs the command — no `(load-plugin)` needed.
///
/// Flip: remove the lazy-miss retry from `%dispatch-command` → `(call! "a-cmd")`
/// falls through to `%call-native!` → `a-cmd` is unknown → logs warning → no move.
#[test]
#[cfg(not(windows))]
fn plugin_calls_cross_plugin_cmd_auto_activates_dep() {
    use hume_scripting::attribution::PluginId;

    let dir = {
        let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        tempfile::tempdir().unwrap()
    };
    // Plugin A — defines "a-cmd" (move-right wrapper).
    let dir_a = dir.path().join("plugins").join("user").join("tpa");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::write(
        dir_a.join("plugin.scm"),
        r#"(define-command! "a-cmd" "doc" (lambda () (call! "move-right")))"#,
    ).unwrap();
    // Plugin B — on-command trigger; body calls "a-cmd" inline (no load-plugin).
    let dir_b = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(
        dir_b.join("plugin.scm"),
        "(define-command! \"b-cmd\" \"doc\" (lambda () (call! \"a-cmd\")))",
    ).unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(declare-plugin \"user/tpa\" #:on-command '(\"a-cmd\"))\n\
         (declare-plugin \"user/tp\"  #:on-command '(\"b-cmd\"))",
    ).unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    { let mut ih = make_init_host(&mut ed.state, &mut ed.view); host.eval_init(&init_path, 10_000, &mut ih, Default::default()) }
        .expect("eval_init must succeed");
    let triggers = host.command_triggers();
    ed.register_lazy_command_stubs(&triggers);
    ed.scripting = Some(host);

    let id_a = PluginId::User { user: "user".to_string(), repo: "tpa".to_string() };
    let id_b = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };

    // After init: both Declared.
    assert!(
        matches!(ed.scripting.as_ref().unwrap().plugin_status(&id_a), Some(PluginStatus::Declared)),
        "dep A must be Declared after init"
    );
    assert!(
        matches!(ed.scripting.as_ref().unwrap().plugin_status(&id_b), Some(PluginStatus::Declared)),
        "plugin B must be Declared after init"
    );

    // Dispatch b-cmd → B activates → B body calls (call! "a-cmd") → lazy-miss
    // retry activates A → a-cmd runs → cursor moves.
    let before = state(&ed);
    type_cmd(&mut ed, ":b-cmd");

    assert_ne!(state(&ed), before, "b-cmd via (call! \"a-cmd\") must have moved the cursor");
    assert!(
        matches!(ed.scripting.as_ref().unwrap().plugin_status(&id_b), Some(PluginStatus::Loaded)),
        "plugin B must be Loaded after dispatch"
    );
    assert!(
        matches!(ed.scripting.as_ref().unwrap().plugin_status(&id_a), Some(PluginStatus::Loaded)),
        "dep A must be Loaded after B calls (call! \"a-cmd\")"
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
    use hume_scripting::attribution::PluginId;
    

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:on-language '("rust"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    );
    let id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Declared)
        ),
        "plugin must be Declared before first language set"
    );
    assert!(
        !ed.scripting.as_ref().unwrap().language_trigger_plugins("rust").is_empty(),
        "language_triggers must be populated before first set"
    );

    let before = state(&ed);
    let bid = ed.focused_buffer_id();
    ed.set_buffer_language(bid, Some("rust".into()));
    ed.drain_hooks();

    assert_ne!(state(&ed), before, "on-language-set handler must run and move cursor on first set");
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Loaded)
        ),
        "plugin must be Loaded after first language set"
    );
    assert!(
        ed.scripting.as_ref().unwrap().language_trigger_plugins("rust").is_empty(),
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
    use hume_scripting::attribution::PluginId;
    

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:on-language '("rust"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    );
    let id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    let bid = ed.focused_buffer_id();

    ed.set_buffer_language(bid, Some("rust".into()));  // first set — activates
    ed.drain_hooks();
    assert!(
        ed.scripting.as_ref().unwrap().language_trigger_plugins("rust").is_empty(),
        "language_triggers must be empty after first set"
    );

    let after_first = state(&ed);
    ed.set_buffer_language(bid, Some("toml".into()));  // round-trip out
    ed.drain_hooks();
    ed.set_buffer_language(bid, Some("rust".into()));  // round-trip back — handler runs, no re-activation
    ed.drain_hooks();

    assert_ne!(state(&ed), after_first, "handler must run again on second rust set");
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Loaded)
        ),
        "plugin must remain Loaded after round-trip (not re-enter Declared or fail)"
    );
    assert!(
        ed.scripting.as_ref().unwrap().language_trigger_plugins("rust").is_empty(),
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
    use hume_scripting::attribution::PluginId;
    

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
        "(declare-plugin \"user/tp\"  #:on-language '(\"rust\"))\n\
         (declare-plugin \"user/tp2\" #:on-language '(\"rust\"))",
    ).unwrap();

    let mut ed = editor_from("-[a]>b c d\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());    { let mut ih = make_init_host(&mut ed.state, &mut ed.view); host.eval_init(&init_path, 10_000, &mut ih, Default::default()) }
        .expect("eval_init must succeed");
    ed.scripting = Some(host);

    let id_a = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    let id_b = PluginId::User { user: "user".to_string(), repo: "tp2".to_string() };
    let bid = ed.focused_buffer_id();
    ed.set_buffer_language(bid, Some("rust".into()));

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id_a),
            Some(PluginStatus::Loaded)
        ),
        "plugin A must be Loaded after language set"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id_b),
            Some(PluginStatus::Loaded)
        ),
        "plugin B must be Loaded after language set"
    );
    assert!(
        ed.scripting.as_ref().unwrap().language_trigger_plugins("rust").is_empty(),
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
    use hume_scripting::attribution::PluginId;
    

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:on-language '("rust"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    );
    let id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    let bid = ed.focused_buffer_id();

    ed.set_buffer_language(bid, Some("toml".into()));  // unrelated language

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Declared)
        ),
        "plugin must stay Declared when an unrelated language is set"
    );
    assert!(
        !ed.scripting.as_ref().unwrap().language_trigger_plugins("rust").is_empty(),
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
        r#"(declare-plugin "user/tp" #:on-command '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );

    assert!(
        !ed.state.message_log
            .entries()
            .any(|e| e.severity == Severity::Trace && e.text.contains("command trigger")),
        "no activation Trace before dispatch; messages: {:?}",
        ed.state.message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );

    type_cmd(&mut ed, ":bar");

    assert!(
        ed.state.message_log.entries().any(|e| {
            e.severity == Severity::Trace
                && e.text.contains("bar")
                && e.text.contains("command trigger")
        }),
        "expected Trace entry naming command trigger 'bar' after dispatch; messages: {:?}",
        ed.state.message_log
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
        ed.state.message_log.entries().any(|e| {
            e.severity == Severity::Warning && e.text.contains("bogus-unknown-cmd")
        }),
        "expected Warning about unknown command 'bogus-unknown-cmd'; messages: {:?}",
        ed.state.message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// `(load-plugin …)` called from a plugin body during *runtime* activation
/// (command trigger) is rejected — registration verbs are top-level-only.
/// The parent plugin is marked `Failed` and an `Error` is logged.
///
/// Flip: remove `ensure_top_level` from `load_plugin` and the call succeeds
/// at runtime instead of failing fast.
#[test]
#[cfg(not(windows))]
fn load_plugin_in_runtime_plugin_body_fails_fast() {
    use hume_scripting::attribution::PluginId;
    use crate::editor::Severity;

    let dir = {
        let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let tp_dir = dir.path().join("plugins").join("user").join("tp");
        let dep_dir = dir.path().join("plugins").join("user").join("dep");
        std::fs::create_dir_all(&tp_dir).unwrap();
        std::fs::create_dir_all(&dep_dir).unwrap();
        std::fs::write(
            tp_dir.join("plugin.scm"),
            // Plugin body calls (load-plugin) at runtime — hard error expected.
            r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))
               (load-plugin "user/dep")"#,
        ).unwrap();
        std::fs::write(dep_dir.join("plugin.scm"), r#"(+ 1 0)"#).unwrap();
        let init = dir.path().join("init.scm");
        std::fs::write(&init, r#"(declare-plugin "user/tp" #:on-command '("bar"))"#).unwrap();
        dir
    };

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    let init_path = dir.path().join("init.scm");
    { let mut ih = make_init_host(&mut ed.state, &mut ed.view); host.eval_init(&init_path, 10_000, &mut ih, Default::default()) }
        .expect("eval_init must succeed");
    let triggers: std::collections::HashMap<_, _> = host.command_triggers();
    ed.register_lazy_command_stubs(&triggers);
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":bar");

    let tp_id = PluginId::User { user: "user".to_string(), repo: "tp".to_string() };
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&tp_id),
            Some(PluginStatus::Failed)
        ),
        "plugin must be Failed when body calls (load-plugin) at runtime"
    );
    assert!(
        ed.state.message_log.entries().any(|e| e.severity == Severity::Error),
        "error must be logged when (load-plugin) is called from a runtime plugin body"
    );
}

/// `define-command!` with a built-in name is rejected first-wins.
///
/// Defining "move-right" in init.scm must leave the built-in intact and log
/// Severity::Error.  Without this check the Steel definition would silently
/// replace the built-in.
///
/// Flip: if the collision check were removed, move-right would become
/// SteelBacked and the Error assertion would fail.
#[test]
#[cfg(not(windows))]
fn define_command_collision_with_builtin_keeps_builtin() {
    use crate::editor::Severity;

    let (ed, _dirs) = setup_editor_with_init_scripting(
        r#"(define-command! "move-right" "redefine" (lambda () (+ 1 0)))"#,
    );

    // The built-in "move-right" must survive — not replaced by SteelBacked.
    assert!(
        !matches!(ed.state.registry.get_mappable("move-right"), Some(MappableCommand::SteelBacked { .. })),
        "built-in move-right must not be replaced by Steel; got: {:?}",
        ed.state.registry.get_mappable("move-right").map(|c| c.name())
    );
    // A Severity::Error must have been logged about the conflict.
    assert!(
        ed.state.message_log.entries().any(|e| {
            e.severity == Severity::Error && e.text.contains("move-right")
        }),
        "collision must produce an Error; messages: {:?}",
        ed.state.message_log.entries().map(|e| format!("{:?}: {}", e.severity, e.text)).collect::<Vec<_>>()
    );
}

/// A lazy plugin whose body contains a top-level `(call! "move-right")` must
/// have that command executed when the plugin is activated at runtime (command
/// trigger).  `activate_plugin_inline` runs with `is_init = false` so
/// `%call-native!` dispatches synchronously via `run_command_sync`.
///
/// Flip: change `new_activation` to `new_init` in `activate_plugin_inline`
/// → `is_init = true` → `%call-native!` warns and skips → cursor stays.
#[test]
#[cfg(not(windows))]
fn lazy_plugin_call_bang_at_body_top_level_is_drained_on_runtime_activation() {
    // Plugin defines "trigger-me" (the command stub key) + calls move-right at
    // load time.  When "trigger-me" is dispatched, the plugin activates and the
    // body-level (call! "move-right") should execute.
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:on-command '("trigger-me"))"#,
        r#"(define-command! "trigger-me" "doc" (lambda () (+ 1 0)))
           (call! "move-right")"#,
    );
    let before = state(&ed);

    type_cmd(&mut ed, ":trigger-me");

    assert_ne!(
        state(&ed),
        before,
        "body-level (call! \"move-right\") must execute when the plugin activates at runtime"
    );
}

// ── G2: post-init language-trigger lint ───────────────────────────────────────

/// Helper: create a `user/tp` plugin file, write init.scm, run `init_scripting`.
///
/// Parallels `setup_editor_with_init_scripting` but also puts a plugin on disk so
/// `#:on-language` triggers for `"user/tp"` are actually recorded.  (Absent-path
/// plugins early-return in `declare_plugin` and skip trigger registration.)
#[cfg(not(windows))]
fn setup_lang_lint_editor(init_body: &str) -> (Editor, Vec<tempfile::TempDir>) {
    let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    let config_tmp = tempfile::tempdir().unwrap();
    let runtime_tmp = tempfile::tempdir().unwrap();
    let data_tmp = tempfile::tempdir().unwrap();

    // Trivial plugin body — the lint checks trigger names, not plugin behaviour.
    let plugin_dir = data_tmp.path().join("hume").join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), "(+ 1 0)").unwrap();

    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(hume_config.join("init.scm"), init_body).unwrap();

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

/// Language-trigger lint warns when `#:on-language` names a language that no
/// `define-language!` has registered.
///
/// Flip: remove the post-init language-trigger lint → no Warning produced →
/// assertion fires.
#[test]
#[cfg(not(windows))]
fn language_trigger_lint_warns_on_unknown_language() {
    use crate::editor::Severity;

    let (ed, _dirs) = setup_lang_lint_editor(
        r#"(declare-plugin "user/tp" #:on-language '("rsut"))"#,
    );

    assert!(
        ed.state.message_log.entries().any(|e| {
            e.severity == Severity::Warning && e.text.contains("rsut")
        }),
        "expected Warning about unknown language 'rsut'; messages: {:?}",
        ed.state.message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// Language-trigger lint is silent when the declared language was registered via
/// `define-language!` earlier in the same init.scm.
///
/// Flip: running the lint before the second language flush (instead of after)
/// would incorrectly warn here because the flush has not yet applied the
/// `define-language!` call to `state.languages`.
#[test]
#[cfg(not(windows))]
fn language_trigger_lint_silent_for_known_language() {
    use crate::editor::Severity;

    // %define-language! (the Rust primitive) works without prelude.scm
    // (the macro wrapper in languages.scm is absent in the test environment).
    let (ed, _dirs) = setup_lang_lint_editor(
        r#"(%define-language! "foo" '() '() '())
           (declare-plugin "user/tp" #:on-language '("foo"))"#,
    );

    assert!(
        !ed.state.message_log.entries().any(|e| {
            e.severity == Severity::Warning && e.text.contains("foo")
        }),
        "must not warn about known language 'foo'; messages: {:?}",
        ed.state.message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// Forward-reference order-independence: `declare-plugin #:on-language '("foo")`
/// appearing BEFORE `define-language! "foo"` in the same init.scm must not warn.
///
/// A declare-time check would see "foo" absent from the live registry and falsely
/// reject it.  The post-init placement (after the second flush) makes the check
/// order-independent.
///
/// Flip: move the lint before the second `flush_pending_language_regs` call →
/// "foo" is not yet in `state.languages` → lint emits a spurious Warning →
/// assertion fires.
#[test]
#[cfg(not(windows))]
fn language_trigger_lint_silent_for_forward_defined_language() {
    use crate::editor::Severity;

    // declare-plugin BEFORE define-language! — the forward-reference case.
    let (ed, _dirs) = setup_lang_lint_editor(
        r#"(declare-plugin "user/tp" #:on-language '("foo"))
           (%define-language! "foo" '() '() '())"#,
    );

    assert!(
        !ed.state.message_log.entries().any(|e| {
            e.severity == Severity::Warning && e.text.contains("foo")
        }),
        "forward-defined language must not warn; messages: {:?}",
        ed.state.message_log
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
        !ed.state.message_log.entries().any(|e| {
            e.severity == Severity::Warning && e.text.contains("move-down")
        }),
        "must not warn about known command 'move-down'; messages: {:?}",
        ed.state.message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}
