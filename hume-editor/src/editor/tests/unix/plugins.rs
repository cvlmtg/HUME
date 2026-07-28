use super::*;
use crate::editor::dispatch::ArgSource;
use crate::editor::registry::MappableCommand;
use crate::editor::scripting_setup::make_init_host;
use hume_scripting::{PluginStatus, ScriptingHost, hooks::HookId};

// ── Phase 1 lazy plugin loading — editor-level tests ─────────────────────────

/// Helper: create a user plugin at `plugins/user/tp/plugin.scm`, write
/// `init.scm`, evaluate it, set up the lazy stubs, and wire the host into
/// `ed`.  Caller must keep `TempDir` alive.
fn setup_lazy_editor(init_body: &str, plugin_body: &str) -> (Editor, tempfile::TempDir) {
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
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed in setup_lazy_editor");

    ed.scripting = Some(host);
    (ed, dir)
}

/// After `eval_init`, a `Lazy` stub is present for the declared command name —
/// `declare-plugin` registers it directly via `CommandHost::register_lazy_command`
/// as the manifest is processed, with no separate post-init pass.
///
/// Flip: without the stub registration, `get_mappable("bar")` would be `None`.
#[test]
fn lazy_stub_present_after_init() {
    let (ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );
    assert!(
        matches!(
            ed.state.config.registry.get_mappable("bar"),
            Some(MappableCommand::Lazy { .. })
        ),
        "Lazy stub must be present after init; got: {:?}",
        ed.state
            .config
            .registry
            .get_mappable("bar")
            .map(|c| c.name())
    );
}

/// Dispatching a lazy command the first time activates the plugin, replaces the
/// stub with `SteelBacked`, and executes the real command (cursor moves).
///
/// Flip: if dispatch does nothing (stub stays Lazy), the cursor would not move
/// and the command would still be `Lazy`.
#[test]
fn first_dispatch_activates_plugin_and_runs() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (call! "move-right")))"#,
    );
    let before = state(&ed);

    // Dispatch "bar" through the command line.
    type_cmd(&mut ed, ":bar");

    // Cursor must have moved.
    assert_ne!(
        state(&ed),
        before,
        "dispatching lazy 'bar' must move the cursor"
    );
    // Stub must be replaced by a real SteelBacked command.
    assert!(
        matches!(
            ed.state.config.registry.get_mappable("bar"),
            Some(MappableCommand::SteelBacked { .. })
        ),
        "stub must be replaced by SteelBacked after first dispatch; got: {:?}",
        ed.state
            .config
            .registry
            .get_mappable("bar")
            .map(|c| c.name())
    );
}

/// Loop guard: if the plugin body never defines the declared command, the stub
/// is removed after dispatch and a Warning is reported.
///
/// Flip: without the loop guard, the stub would remain (infinite retry).
#[test]
fn loop_guard_removes_stub_when_body_never_defines_command() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("bar"))"#,
        // Plugin body exists but never defines "bar".
        r#"(define-command! "other-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    // Stub must be present before dispatch.
    assert!(
        matches!(
            ed.state.config.registry.get_mappable("bar"),
            Some(MappableCommand::Lazy { .. })
        ),
        "Lazy stub must be present before dispatch"
    );

    type_cmd(&mut ed, ":bar");

    // Stub must have been removed by the loop guard.
    assert!(
        ed.state.config.registry.get_mappable("bar").is_none(),
        "stub must be removed when body never defines the command; got: {:?}",
        ed.state
            .config
            .registry
            .get_mappable("bar")
            .map(|c| c.name())
    );
}

/// A lazy plugin body that queues a `register-lsp-server!` and then errors
/// before defining its activation command: the queued registration must not
/// survive the failed activation — it must not be picked up by
/// `apply_script_effects`'s own inline application, nor leak into some later
/// unrelated drain.
///
/// Flip: without `SteelCtx::pop_effect_marks` rolling back on failure,
/// `config_command_for_test("rust")` comes back `Some(..)`.
#[test]
fn failed_activation_does_not_leave_a_queued_lsp_registration() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("bar"))"#,
        r#"(register-lsp-server! "rust" #:command "rust-analyzer")
           (error "intentional mid-body error")"#,
    );

    type_cmd(&mut ed, ":bar");

    assert!(
        ed.lsp.config_command_for_test("rust").is_none(),
        "a failed activation must not leave its queued register-lsp-server! applied"
    );
}

/// A lazy plugin body that calls `%define-language!` (the builtin behind
/// `define-language!`) is applied in the very same activation call —
/// `apply_script_effects` drains `pending_language_regs` at runtime, not
/// only at the `eval_init` boundary.
///
/// Flip: without the runtime drain in `apply_script_effects`,
/// `languages.by_name("foo")` stays `None` after dispatch.
#[test]
fn lazy_plugin_defined_language_is_registered_on_activation() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("bar"))"#,
        r#"(%define-language! "foo" '() '() '())
           (define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );

    assert!(
        ed.state.config.languages.by_name("foo").is_none(),
        "precondition: 'foo' must not be registered before activation"
    );

    type_cmd(&mut ed, ":bar");

    assert!(
        ed.state.config.languages.by_name("foo").is_some(),
        "a define-language! from a lazily-activated plugin body must be applied \
         in the same activation call, not stranded until :reload-config"
    );
}

/// A lazily-activated `#:languages` plugin body that itself calls
/// `set-buffer-language!` on the very buffer whose language-set triggered
/// its own activation: the nested call (applied inline, before activation
/// returns) must win. The outer `set_buffer_language` call must detect the
/// buffer no longer holds the value it's about to fire `OnLanguageSet` for,
/// and bail out rather than enqueue a second, stale hook.
///
/// Flip: without the re-entrancy guard, `pending_hooks` holds two
/// `OnLanguageSet` entries (python, then a stale rust) instead of one.
#[test]
fn set_buffer_language_reentrant_activation_uses_final_value() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:languages '("rust"))"#,
        r#"(set-buffer-language! (car (buffers)) "python")"#,
    );
    let bid = ed.focused_buffer_id();

    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));

    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("python"),
        "the plugin's own set-buffer-language! call inside its activation body must win"
    );
    assert_eq!(
        ed.state.config.pending_hooks.len(),
        1,
        "exactly one OnLanguageSet hook must be queued, not a stale duplicate; got: {:?}",
        ed.state.config.pending_hooks
    );
    let (hook_id, args) = &ed.state.config.pending_hooks[0];
    assert_eq!(*hook_id, HookId::OnLanguageSet);
    assert!(
        matches!(&args[1], steel::rvals::SteelVal::StringV(s) if s.as_str() == "python"),
        "the queued OnLanguageSet hook must carry the final ('python') value, not the stale \
         ('rust') one that triggered activation; got: {:?}",
        args[1]
    );
}

/// Body-error path: if the plugin body raises an error, the state becomes
/// `Failed`, the stub is removed, and a Warning/Error is reported.
///
/// Flip: without error handling, the stub would survive and allow re-entry.
#[test]
fn body_error_removes_stub_and_marks_failed() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("bar"))"#,
        r#"(error "intentional plugin failure")"#,
    );
    assert!(
        matches!(
            ed.state.config.registry.get_mappable("bar"),
            Some(MappableCommand::Lazy { .. })
        ),
        "stub must be present before dispatch"
    );

    type_cmd(&mut ed, ":bar");

    // Stub removed.
    assert!(
        ed.state.config.registry.get_mappable("bar").is_none(),
        "stub must be removed after body error"
    );
    // Plugin state is Failed.
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Failed)
        ),
        "plugin must be Failed after body error"
    );
}

/// `:bar arg` on a lazy command: the arg is correctly passed to a 1-arity
/// command on first call (after activation).
///
/// Flip: if arg were silently dropped, the Steel command would receive false
/// (#f) instead of the string and the test string would not appear as output.
#[test]
fn lazy_cmd_arg_passed_on_first_call() {
    use hume_scripting::attribution::PluginId;

    // The plugin defines a 1-arity "bar" that does nothing visible — we just
    // verify that after activation the command is SteelBacked (i.e. arg was
    // accepted, no arity error), and the plugin is Loaded.
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda (x) (+ 1 0)))"#,
    );

    // Dispatch ":bar hello" — would fail at arity check if arg were dropped.
    type_cmd(&mut ed, ":bar hello");

    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Loaded)
        ),
        "plugin must be Loaded after first dispatch with arg"
    );
    assert!(
        matches!(
            ed.state.config.registry.get_mappable("bar"),
            Some(MappableCommand::SteelBacked { .. })
        ),
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
fn key_press_activates_lazy_plugin_via_keymap() {
    use crate::editor::keymap::BindMode;
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (call! "move-right")))"#,
    );
    // setup_lazy_editor passes a throwaway Keymap to eval_init; bind here so
    // the key lands in the editor's actual keymap.
    ed.state.config.keymap.bind_user_with_extend(
        BindMode::Normal,
        &[key('z')],
        "bar".into(),
        false,
    );
    let before = state(&ed);

    ed.handle_key(key('z'));

    assert_ne!(
        state(&ed),
        before,
        "pressing 'z' must activate 'bar' and move the cursor"
    );
    assert!(
        matches!(
            ed.state.config.registry.get_mappable("bar"),
            Some(MappableCommand::SteelBacked { .. })
        ),
        "stub must be replaced by SteelBacked after command-activated; got: {:?}",
        ed.state
            .config
            .registry
            .get_mappable("bar")
            .map(|c| c.name())
    );
}

/// Eager-plugin-command collision: an eager plugin defines "foo", then a lazy
/// plugin declares `#:commands '("foo")`.  The collision is caught at
/// `declare-plugin` time, directly against the editor's live registry: the
/// declaration fails with "no activation entries", the eager SteelBacked
/// command survives, and no `Lazy` stub is ever registered for "foo".
///
/// Flip: remove the eager-command check from `declare_plugin`'s filter loop
/// → declare-plugin succeeds, "foo" registers as a `Lazy` stub shadowing the
/// eager command, and the first assertion (eval_init returns Err) flips to Ok.
#[test]
fn lazy_stub_rejected_when_name_taken_by_eager_plugin() {
    let dir = {
        let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        tempfile::tempdir().unwrap()
    };
    // Eager plugin — loaded inline (no activation entries), defines "foo".
    let eager_dir = dir.path().join("plugins").join("user").join("eager");
    std::fs::create_dir_all(&eager_dir).unwrap();
    std::fs::write(
        eager_dir.join("plugin.scm"),
        r#"(define-command! "foo" "doc" (lambda () (+ 1 0)))"#,
    )
    .unwrap();
    // Lazy plugin — declares "foo" as its sole activation command, which
    // conflicts with the eager plugin.  The declare hard-errors at init time.
    let lazy_dir = dir.path().join("plugins").join("user").join("lz");
    std::fs::create_dir_all(&lazy_dir).unwrap();
    std::fs::write(lazy_dir.join("plugin.scm"), r#"(+ 1 0)"#).unwrap();

    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(load-plugin \"user/eager\")\n(declare-plugin \"user/lz\" #:commands '(\"foo\"))",
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    // Mirror real init_scripting order: eager command is in command_table
    // before declare-plugin runs, so the filter loop rejects "foo".
    let init_err = {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    };
    // declare-plugin now hard-errors when all entries are filtered (collision
    // caught at declaration, not at stub registration).
    let err =
        init_err.expect_err("eval_init must fail: declare-plugin rejects 'foo' at declare time");
    assert!(
        err.message.contains("no activation entries") || err.message.contains("conflicted"),
        "error must explain the cause; got: {err:?}"
    );

    ed.scripting = Some(host);

    // The eager command still registered correctly before the error.
    assert!(
        matches!(
            ed.state.config.registry.get_mappable("foo"),
            Some(MappableCommand::SteelBacked { .. })
        ),
        "eager 'foo' must survive as SteelBacked; got: {:?}",
        ed.state
            .config
            .registry
            .get_mappable("foo")
            .map(|c| c.name())
    );
}

/// Two plugins both declare `#:commands '("bar")` — the collision is caught
/// at `declare-plugin` time against the editor's live registry: the second
/// plugin's "bar" entry is dropped (logged as an Error, first-writer-wins),
/// and both plugins remain `Declared` (neither is stuck or hard-errored).
///
/// Ported from a `MockHost`-driven version in `hume-editor/tests/scripting.rs`
/// to a real `Editor` + `EditorHostImpl` — collision detection is
/// `CommandRegistry`'s decision, and testing it through a hand-rolled
/// `MockHost` copy of the same rules risked silently drifting from the real
/// behavior it was meant to prove.
///
/// Flip: remove the `register_lazy_command` collision check → "bar" would
/// register twice, the second `Lazy { plugin: pb, .. }` silently overwriting
/// the first in the registry, so pa's real ownership would be lost with no
/// error logged.
#[test]
fn lazy_stub_collision_lazy_vs_lazy_first_writer_wins() {
    let dir = {
        let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        tempfile::tempdir().unwrap()
    };
    let pa_dir = dir.path().join("plugins").join("user").join("pa");
    let pb_dir = dir.path().join("plugins").join("user").join("pb");
    std::fs::create_dir_all(&pa_dir).unwrap();
    std::fs::create_dir_all(&pb_dir).unwrap();
    std::fs::write(
        pa_dir.join("plugin.scm"),
        r#"(define-command! "tp-a" "doc" (lambda () (+ 1 0)))"#,
    )
    .unwrap();
    std::fs::write(
        pb_dir.join("plugin.scm"),
        r#"(define-command! "tp-b" "doc" (lambda () (+ 1 0)))"#,
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(declare-plugin \"user/pa\" #:commands '(\"bar\"))\n\
         (declare-plugin \"user/pb\" #:commands '(\"bar\" \"pb-only\"))",
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("lazy-vs-lazy collision must NOT abort init");

    // Error logged for pb's duplicate "bar" entry. `eval_init` queues log
    // messages on the host (`ctx.log`) rather than writing `ed.state.
    // message_log` directly — only `Editor::init_scripting`'s tail code
    // flushes that queue, which this test bypasses by calling `eval_init`
    // directly, so check the host's queue itself.
    assert!(
        host.peek_pending_messages().iter().any(|(sev, msg)| {
            matches!(sev, hume_scripting::LogLevel::Error)
                && msg.contains("bar")
                && msg.contains("already claimed")
        }),
        "expected an Error about 'bar' already claimed; got: {:?}",
        host.peek_pending_messages()
    );
    ed.scripting = Some(host);

    // First-writer (pa) owns the "bar" stub.
    use hume_scripting::attribution::PluginId;
    let pa_id = PluginId::User {
        user: "user".to_string(),
        repo: "pa".to_string(),
    };
    let pb_id = PluginId::User {
        user: "user".to_string(),
        repo: "pb".to_string(),
    };
    assert!(
        matches!(
            ed.state.config.registry.get_mappable("bar"),
            Some(MappableCommand::Lazy { plugin, .. }) if *plugin == pa_id
        ),
        "bar's Lazy stub must be owned by pa (first-writer-wins)"
    );
    // Both plugins are Declared — pb stays declared even though its "bar" entry was dropped.
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&pa_id),
            Some(PluginStatus::Declared)
        ),
        "pa must be Declared"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&pb_id),
            Some(PluginStatus::Declared)
        ),
        "pb must be Declared even with its 'bar' entry dropped"
    );
}

// ── Phase 2 lazy plugin loading — event activations ──────────────────────────

/// `#:events` plugin activates on first matching hook fire; its handler
/// runs in the same fire that caused activation.
///
/// Flip: without A3 (`activate_lazy_event_plugins` at the top of
/// `fire_hook_silent`), the plugin stays `Declared` and the cursor never moves.
#[test]
fn event_trigger_activates_on_first_fire() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:events '("on-buffer-save"))"#,
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right")))"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Declared)
        ),
        "plugin must be Declared before first fire"
    );
    assert!(
        !ed.scripting
            .as_ref()
            .unwrap()
            .activation_event_plugins(HookId::OnBufferSave)
            .is_empty(),
        "activation_events must be populated before first fire"
    );

    let before = state(&ed);
    let bid = ed.focused_buffer_id();
    ed.fire_hook_buffer_save(bid);
    ed.drain_hooks();

    assert_ne!(
        state(&ed),
        before,
        "hook handler must run and move the cursor on first fire"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Loaded)
        ),
        "plugin must be Loaded after first fire"
    );
    assert!(
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_event_plugins(HookId::OnBufferSave)
            .is_empty(),
        "activation_events must be cleared after plugin loads"
    );
}

/// Second fire: handler still runs (plugin already `Loaded`); no re-activation.
///
/// Flip: if `activation_events` were not cleared after load, `activate_plugin`'s
/// `Loaded` guard would still fire harmlessly — but the test documents that
/// the fast path is taken (no spurious activation attempt).
#[test]
fn event_trigger_idempotent_on_second_fire() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:events '("on-buffer-save"))"#,
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right")))"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    let bid = ed.focused_buffer_id();

    ed.fire_hook_buffer_save(bid); // first fire — activates
    ed.drain_hooks();
    assert!(
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_event_plugins(HookId::OnBufferSave)
            .is_empty(),
        "activation_events must be empty after first fire"
    );

    let after_first = state(&ed);
    ed.fire_hook_buffer_save(bid); // second fire — handler runs, no re-activation
    ed.drain_hooks();

    assert_ne!(
        state(&ed),
        after_first,
        "handler must run again on second fire"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Loaded)
        ),
        "plugin must remain Loaded after second fire (not re-enter Declared)"
    );
}

/// 1:many: two plugins both declare `#:events '("on-buffer-save")`; a single
/// fire activates both.
///
/// Flip: if only the first plugin in the activation Vec were activated, the second
/// would stay `Declared` with its handler never registering.
#[test]
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
    )
    .unwrap();
    let dir_b = dir.path().join("plugins").join("user").join("tp2");
    std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(
        dir_b.join("plugin.scm"),
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right")))"#,
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(declare-plugin \"user/tp\"  #:events '(\"on-buffer-save\"))\n\
         (declare-plugin \"user/tp2\" #:events '(\"on-buffer-save\"))",
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b c d\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");
    ed.scripting = Some(host);

    let id_a = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    let id_b = PluginId::User {
        user: "user".to_string(),
        repo: "tp2".to_string(),
    };
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
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_event_plugins(HookId::OnBufferSave)
            .is_empty(),
        "activation_events must be fully cleared after both plugins load"
    );
}

/// Body error: plugin raises at load time → `Failed`, error reported, activation
/// entry cleared — no retry on a second fire.
///
/// Flip: without `activation_events` drop in `drop_activations_for`'s failure path,
/// the same plugin would attempt activation on every fire.
#[test]
fn event_plugin_failure_marks_failed_no_retry() {
    use hume_scripting::attribution::PluginId;

    use crate::editor::Severity;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:events '("on-buffer-save"))"#,
        r#"(error "intentional plugin failure")"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    let bid = ed.focused_buffer_id();

    ed.fire_hook_buffer_save(bid); // first fire — activates → body fails
    ed.drain_hooks();

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Failed)
        ),
        "plugin must be Failed after body error"
    );
    assert!(
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_event_plugins(HookId::OnBufferSave)
            .is_empty(),
        "activation_events must be cleared even after failure"
    );
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error),
        "Severity::Error must be logged after body failure"
    );

    let msg_count = ed.state.message_log.entries().count();
    ed.fire_hook_buffer_save(bid); // second fire — no retry
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

/// `(declare-plugin "name")` with no activation entries is a hard error — the plugin
/// could never activate at runtime.
///
/// Flip: remove the zero-activation guard in declare_plugin and eval_init succeeds.
#[test]
fn declare_plugin_no_triggers_is_hard_error() {
    let (dir, init_path) = {
        let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("plugins").join("user").join("tp");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.scm"),
            r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
        )
        .unwrap();
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
        "declare-plugin with no activation entries must abort init with an error"
    );
}

/// Top-level `(load-plugin "user/tp")` with the plugin absent on disk → silent
/// skip (PLUM-friendly bootstrap), no error, and the plugin is recorded in
/// `declared-plugins` but absent from `loaded-plugins`.
///
/// Flip: if this errored, users could not declare third-party plugins before
/// running `:plum-install` on a fresh setup.
#[test]
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
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("absent top-level load-plugin must not error");

    // Plugin was not inserted into lazy_registry (absent on disk).
    assert!(
        !host.has_any_loaded_plugin(),
        "no plugin must be Loaded when absent on disk"
    );
}

/// A lazy plugin B can call another lazy plugin A's command via `(call! "a-cmd")`.
/// The inline lazy-miss retry in `%dispatch-command` activates A on the fly and
/// runs the command — no `(load-plugin)` needed.
///
/// Flip: remove the lazy-miss retry from `%dispatch-command` → `(call! "a-cmd")`
/// falls through to `%call-native!` → `a-cmd` is unknown → logs warning → no move.
#[test]
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
    )
    .unwrap();
    // Plugin B — command activation entry; body calls "a-cmd" inline (no load-plugin).
    let dir_b = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(
        dir_b.join("plugin.scm"),
        "(define-command! \"b-cmd\" \"doc\" (lambda () (call! \"a-cmd\")))",
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(declare-plugin \"user/tpa\" #:commands '(\"a-cmd\"))\n\
         (declare-plugin \"user/tp\"  #:commands '(\"b-cmd\"))",
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");
    ed.scripting = Some(host);

    let id_a = PluginId::User {
        user: "user".to_string(),
        repo: "tpa".to_string(),
    };
    let id_b = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };

    // After init: both Declared.
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id_a),
            Some(PluginStatus::Declared)
        ),
        "dep A must be Declared after init"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id_b),
            Some(PluginStatus::Declared)
        ),
        "plugin B must be Declared after init"
    );

    // Dispatch b-cmd → B activates → B body calls (call! "a-cmd") → lazy-miss
    // retry activates A → a-cmd runs → cursor moves.
    let before = state(&ed);
    type_cmd(&mut ed, ":b-cmd");

    assert_ne!(
        state(&ed),
        before,
        "b-cmd via (call! \"a-cmd\") must have moved the cursor"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id_b),
            Some(PluginStatus::Loaded)
        ),
        "plugin B must be Loaded after dispatch"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id_a),
            Some(PluginStatus::Loaded)
        ),
        "dep A must be Loaded after B calls (call! \"a-cmd\")"
    );
}

/// A lazy plugin activated via the in-Steel `call!` path whose body tries to
/// shadow a native command must fail cleanly — the native command survives.
///
/// The in-Steel path matters: `SteelCtx::new_command` carries an empty
/// `builtin_cmd_names` set, so the Steel-side shadow guard is inert and the
/// conflict is only caught by `host.register_command`.  The failed define must
/// not leave `command_table`/`cmd_owners` entries that make the plugin-failure
/// rollback unregister the native command.
///
/// Flip (either revert triggers this): insert into `command_table`/`cmd_owners`
/// before `host.register_command` in `define_command_inner`, or revert
/// `CommandRegistry::unregister` to an unconditional remove → `move-left`
/// disappears from the registry and the assertions fire.
#[test]
fn native_command_survives_failed_shadowing_plugin() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        // Eager command whose body triggers the lazy plugin via call! —
        // the in-Steel activation path (empty builtin_cmd_names).
        r#"(declare-plugin "user/tp" #:commands '("bar"))
           (define-command! "trigger" "doc" (lambda () (call! "bar")))"#,
        // Plugin body shadows a native command.
        r#"(define-command! "move-left" "doc" (lambda () (+ 1 0)))"#,
    );

    type_cmd(&mut ed, ":trigger");

    // Plugin must be Failed (its body errored on the shadow conflict).
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Failed)
        ),
        "plugin must be Failed after shadowing attempt"
    );
    // The native command must survive in the registry.
    assert!(
        ed.state
            .config
            .registry
            .get_mappable("move-left")
            .is_some_and(MappableCommand::is_native),
        "native move-left must survive the failed plugin rollback"
    );
    // No orphan Steel-side entries for the native name.
    assert!(
        !ed.scripting
            .as_ref()
            .unwrap()
            .command_table_for_test()
            .contains_key("move-left"),
        "command_table must not retain the rejected shadow entry"
    );
    assert!(
        !ed.scripting
            .as_ref()
            .unwrap()
            .cmd_owners_for_test()
            .contains_key("move-left"),
        "cmd_owners must not retain the rejected shadow entry"
    );
}

/// A lazy plugin activated via `call!` whose body binds a key and then errors:
/// the binding must never be applied — a `Failed` plugin leaves no dangling
/// keybinding pointing at a command that (if it was also rolled back, or was
/// never valid) can no longer be dispatched.
///
/// `bind-key!` only queues an `Effect::BindKey`, and two independent layers
/// drop it: `pop_effect_marks(false)` discards the failed body's uncommitted
/// entries, and `take_eval_effects` hands the dispatcher's `Err` arm only the
/// *committed* ones. `Q` never reaches the keymap.
///
/// Flip: both layers must be defeated together for this to fire — make
/// `pop_effect_marks`'s failure branch keep every entry AND `take_eval_effects`
/// salvage uncommitted ones on `Err`; then the bind reaches
/// `apply_script_effects` and `lookup_command` returns `Some(("some-cmd",
/// false))`. Flipping either alone is caught by the other, which is the point:
/// a bind is never applied unless the body that queued it committed.
#[test]
fn plugin_keybinding_rolled_back_on_failed_activation() {
    use crate::editor::keymap::BindMode;
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("bar"))
           (define-command! "trigger" "doc" (lambda () (call! "bar")))"#,
        r#"(bind-key! 'normal "Q" "some-cmd") (error "boom")"#,
    );

    type_cmd(&mut ed, ":trigger");

    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Failed)
        ),
        "plugin must be Failed after intentional error"
    );
    assert!(
        ed.state
            .config
            .keymap
            .lookup_command(BindMode::Normal, &[key('Q')])
            .is_none(),
        "the failed plugin's bind-key! must be unbound, not left dangling"
    );
}

/// A lazy plugin activated via `call!` whose body registers a hook and then
/// errors: the hook must not survive — a `Failed` plugin's hooks must stop
/// firing.
///
/// Flip: drop `hooks.remove_owned_by` from `finish_lazy_activation` →
/// `has_hook_handlers` still reports `true` after failure.
#[test]
fn plugin_hook_rolled_back_on_failed_activation() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("bar"))
           (define-command! "trigger" "doc" (lambda () (call! "bar")))"#,
        r#"(register-hook! 'on-buffer-save (lambda (bid) 0)) (error "boom")"#,
    );

    type_cmd(&mut ed, ":trigger");

    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Failed)
        ),
        "plugin must be Failed after intentional error"
    );
    assert!(
        !ed.scripting
            .as_ref()
            .unwrap()
            .has_hook_handlers(HookId::OnBufferSave),
        "the failed plugin's register-hook! must not survive rollback"
    );
}

// ── Phase 3b lazy plugin loading — language/filetype activations ──────────────

/// `#:languages` plugin activates on first matching language set; its
/// `on-language-set` handler runs in the same call that caused activation.
///
/// Flip: without `activate_lazy_language_plugins` in `set_buffer_language`,
/// the plugin stays `Declared` and the cursor never moves.
#[test]
fn language_trigger_activates_on_set() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:languages '("rust"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Declared)
        ),
        "plugin must be Declared before first language set"
    );
    assert!(
        !ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("rust")
            .is_empty(),
        "activation_languages must be populated before first set"
    );

    let before = state(&ed);
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    ed.drain_hooks();

    assert_ne!(
        state(&ed),
        before,
        "on-language-set handler must run and move cursor on first set"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Loaded)
        ),
        "plugin must be Loaded after first language set"
    );
    assert!(
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("rust")
            .is_empty(),
        "activation_languages must be cleared after plugin loads"
    );
}

/// Second set to the same language: handler still runs; no re-activation.
///
/// Flip: if `activation_languages` were not cleared on load, a second matching set
/// would attempt activation again — `activate_plugin`'s `Loaded` guard prevents
/// a crash, but the test documents the intended fast path.
#[test]
fn language_trigger_idempotent_on_round_trip() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:languages '("rust"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    let bid = ed.focused_buffer_id();

    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang)); // first set — activates
    ed.drain_hooks();
    assert!(
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("rust")
            .is_empty(),
        "activation_languages must be empty after first set"
    );

    let after_first = state(&ed);
    let lang = ed.state.config.languages.intern("toml");
    ed.set_buffer_language(bid, Some(lang)); // round-trip out
    ed.drain_hooks();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang)); // round-trip back — handler runs, no re-activation
    ed.drain_hooks();

    assert_ne!(
        state(&ed),
        after_first,
        "handler must run again on second rust set"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Loaded)
        ),
        "plugin must remain Loaded after round-trip (not re-enter Declared or fail)"
    );
    assert!(
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("rust")
            .is_empty(),
        "activation_languages must remain cleared after round-trip"
    );
}

/// 1:many: two plugins both declare `#:languages '("rust")`; a single language
/// set activates both.
///
/// Flip: if only the first plugin in the activation Vec were activated, the second
/// would stay `Declared` with its handler never registering.
#[test]
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
    )
    .unwrap();
    let dir_b = dir.path().join("plugins").join("user").join("tp2");
    std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(
        dir_b.join("plugin.scm"),
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(declare-plugin \"user/tp\"  #:languages '(\"rust\"))\n\
         (declare-plugin \"user/tp2\" #:languages '(\"rust\"))",
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b c d\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");
    ed.scripting = Some(host);

    let id_a = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    let id_b = PluginId::User {
        user: "user".to_string(),
        repo: "tp2".to_string(),
    };
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));

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
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("rust")
            .is_empty(),
        "activation_languages must be fully cleared after both plugins load"
    );
}

/// Language set for an unregistered language → plugin stays `Declared`.
///
/// Flip: if `activate_lazy_language_plugins` looked up the wrong map or iterated
/// unconditionally, the plugin would load on any language set.
#[test]
fn language_trigger_does_not_fire_on_unrelated_language() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:languages '("rust"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    let bid = ed.focused_buffer_id();

    let lang = ed.state.config.languages.intern("toml");
    ed.set_buffer_language(bid, Some(lang)); // unrelated language

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Declared)
        ),
        "plugin must stay Declared when an unrelated language is set"
    );
    assert!(
        !ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("rust")
            .is_empty(),
        "activation_languages[\"rust\"] must remain intact after an unrelated set"
    );
}

/// `#:languages '("*")` activates on ANY language set, not just an exact match —
/// the wildcard a manifest.scm uses because it can't enumerate every language a
/// user might want it for.
///
/// Flip: if `activation_language_plugins` only checked the exact key, the
/// wildcard entry would never fire and the plugin would stay `Declared` forever.
#[test]
fn language_wildcard_trigger_activates_on_any_language() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:languages '("*"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };

    assert!(
        !ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("toml")
            .is_empty(),
        "the \"*\" entry must be returned for a language it never named explicitly"
    );

    let before = state(&ed);
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("toml");
    ed.set_buffer_language(bid, Some(lang));
    ed.drain_hooks();

    assert_ne!(
        state(&ed),
        before,
        "on-language-set handler must run and move cursor on a wildcard-matched set"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Loaded)
        ),
        "plugin must be Loaded after a wildcard-matched language set"
    );
}

/// A wildcard entry and a specific-language entry coexist without cross-firing:
/// a set for a language only the wildcard plugin matches activates that plugin
/// alone, leaving the specific-language plugin `Declared`.
///
/// Flip: if the union in `activation_language_plugins` deduped incorrectly or
/// dropped the specific-language map, either the wrong plugin would activate or
/// both would.
#[test]
fn language_wildcard_and_specific_entry_coexist() {
    use hume_scripting::attribution::PluginId;

    let dir = {
        let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        tempfile::tempdir().unwrap()
    };
    let dir_rust = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&dir_rust).unwrap();
    std::fs::write(dir_rust.join("plugin.scm"), "(+ 1 0)").unwrap();
    let dir_any = dir.path().join("plugins").join("user").join("tp2");
    std::fs::create_dir_all(&dir_any).unwrap();
    std::fs::write(dir_any.join("plugin.scm"), "(+ 1 0)").unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(declare-plugin \"user/tp\"  #:languages '(\"rust\"))\n\
         (declare-plugin \"user/tp2\" #:languages '(\"*\"))",
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b c d\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");
    ed.scripting = Some(host);

    let id_rust = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    let id_any = PluginId::User {
        user: "user".to_string(),
        repo: "tp2".to_string(),
    };
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("toml");
    ed.set_buffer_language(bid, Some(lang));

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id_rust),
            Some(PluginStatus::Declared)
        ),
        "the \"rust\"-only plugin must stay Declared for an unrelated language"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id_any),
            Some(PluginStatus::Loaded)
        ),
        "the wildcard plugin must activate for any language, including \"toml\""
    );
}

// ── Phase 4 Polish — load-time activation reporting ──────────────────────────

/// Command activation: first dispatch of a lazy command logs a Trace entry naming
/// the activating command.
///
/// Flip: before dispatch, no such Trace exists — confirming the entry is
/// produced by the activation path, not during init.
#[test]
fn command_trigger_logs_trace_on_activation() {
    use crate::editor::Severity;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );

    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Trace && e.text.contains("by command")),
        "no activation Trace before dispatch; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );

    type_cmd(&mut ed, ":bar");

    assert!(
        ed.state.message_log.entries().any(|e| {
            e.severity == Severity::Trace && e.text.contains("bar") && e.text.contains("by command")
        }),
        "expected Trace entry naming command activation 'bar' after dispatch; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

// ── Phase 4 Polish — post-init keymap lint ────────────────────────────────────

/// Helper: write `init_scm` to a temporary config dir, set `XDG_CONFIG_HOME`
/// and `HUME_RUNTIME`, call `init_scripting` on a fresh Editor, restore env
/// vars before returning.  Caller must keep the returned `Vec<TempDir>` alive.
///
/// `runtime_dir`: `None` points `HUME_RUNTIME` at a fresh empty tempdir (for
/// synthetic-fixture tests with no shipped plugin sources) and includes it in
/// the returned `Vec`; `Some(path)` points at `path` instead (typically the
/// repo's real `runtime/` tree, for tests exercising a real shipped
/// `manifest.scm`/plugin end to end) and does not add a tempdir for it, since
/// the caller owns that path's lifetime.
fn setup_editor_with_init_scripting(
    init_scm: &str,
    runtime_dir: Option<&std::path::Path>,
) -> (Editor, Vec<tempfile::TempDir>) {
    let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    let config_tmp = tempfile::tempdir().unwrap();
    let data_tmp = tempfile::tempdir().unwrap();
    let runtime_tmp = runtime_dir.is_none().then(|| tempfile::tempdir().unwrap());
    let runtime_path = runtime_dir.unwrap_or_else(|| runtime_tmp.as_ref().unwrap().path());

    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(hume_config.join("init.scm"), init_scm).unwrap();

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", config_tmp.path());
        std::env::set_var("HUME_RUNTIME", runtime_path);
        std::env::set_var("XDG_DATA_HOME", data_tmp.path());
    }

    let mut ed = editor_from("-[a]>b\n");
    ed.init_scripting(&mut Default::default());

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HUME_RUNTIME");
        std::env::remove_var("XDG_DATA_HOME");
    }

    let mut dirs = vec![config_tmp, data_tmp];
    dirs.extend(runtime_tmp);
    (ed, dirs)
}

/// Keymap lint warns when a bind-key! targets a name not in the command registry.
///
/// Flip: binding to a known command ("move-down") must produce no warning, so
/// the warning here is definitely about the unknown name, not an always-fire.
#[test]
fn keymap_lint_warns_on_unknown_command() {
    use crate::editor::Severity;

    let (ed, _dirs) =
        setup_editor_with_init_scripting(r#"(bind-key! 'normal "Q" "bogus-unknown-cmd")"#, None);

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| { e.severity == Severity::Warning && e.text.contains("bogus-unknown-cmd") }),
        "expected Warning about unknown command 'bogus-unknown-cmd'; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// Native default keymaps must never bind a key to a command that isn't a Rust
/// built-in — `lsp-completion-trigger` (Ctrl+Space) lives entirely in
/// `core:lsp`'s `plugin.scm` now, not in `keymap/defaults.rs`, so an editor
/// that never loads or declares `core:lsp` must start up with no keymap-lint
/// warning naming it.
///
/// Flip: re-adding `t.bind_leaf(key!(Ctrl + ' '), cmd!("lsp-completion-trigger"))`
/// to `default_insert_keymap` in `keymap/defaults.rs` makes this fail.
#[test]
fn no_keymap_lint_warning_for_lsp_completion_trigger_without_core_lsp() {
    use crate::editor::Severity;

    let (ed, _dirs) = setup_editor_with_init_scripting("", None);

    assert!(
        !ed.state.message_log.entries().any(|e| {
            e.severity == Severity::Warning && e.text.contains("lsp-completion-trigger")
        }),
        "no warning should name 'lsp-completion-trigger' when core:lsp is never loaded/declared; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// `(load-plugin …)` called from a plugin body during *runtime* activation
/// (command activation) is rejected — registration verbs are top-level-only.
/// The parent plugin is marked `Failed` and an `Error` is logged.
///
/// Flip: remove `ensure_top_level` from `load_plugin` and the call succeeds
/// at runtime instead of failing fast.
#[test]
fn load_plugin_in_runtime_plugin_body_fails_fast() {
    use crate::editor::Severity;
    use hume_scripting::attribution::PluginId;

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
        )
        .unwrap();
        std::fs::write(dep_dir.join("plugin.scm"), r#"(+ 1 0)"#).unwrap();
        let init = dir.path().join("init.scm");
        std::fs::write(&init, r#"(declare-plugin "user/tp" #:commands '("bar"))"#).unwrap();
        dir
    };

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    let init_path = dir.path().join("init.scm");
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":bar");

    let tp_id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&tp_id),
            Some(PluginStatus::Failed)
        ),
        "plugin must be Failed when body calls (load-plugin) at runtime"
    );
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error),
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
fn define_command_collision_with_builtin_keeps_builtin() {
    use crate::editor::Severity;

    let (ed, _dirs) = setup_editor_with_init_scripting(
        r#"(define-command! "move-right" "redefine" (lambda () (+ 1 0)))"#,
        None,
    );

    // The built-in "move-right" must survive — not replaced by SteelBacked.
    assert!(
        !matches!(
            ed.state.config.registry.get_mappable("move-right"),
            Some(MappableCommand::SteelBacked { .. })
        ),
        "built-in move-right must not be replaced by Steel; got: {:?}",
        ed.state
            .config
            .registry
            .get_mappable("move-right")
            .map(|c| c.name())
    );
    // A Severity::Error must have been logged about the conflict.
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| { e.severity == Severity::Error && e.text.contains("move-right") }),
        "collision must produce an Error; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// A lazy plugin whose body contains a top-level `(call! "move-right")` must
/// have that command executed when the plugin is activated at runtime (command
/// activation).  `activate_plugin_inline` runs with `session = EvalSession::Runtime`
/// so `%call-native!` dispatches synchronously via `run_command_sync`.
///
/// Flip: change `new_activation` to `new_init` in `activate_plugin_inline`
/// → `session = EvalSession::Init` → `%call-native!` warns and skips → cursor stays.
#[test]
fn lazy_plugin_call_bang_at_body_top_level_is_drained_on_runtime_activation() {
    // Plugin defines "trigger-me" (the command stub key) + calls move-right at
    // load time.  When "trigger-me" is dispatched, the plugin activates and the
    // body-level (call! "move-right") should execute.
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("trigger-me"))"#,
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

// ── G2: post-init language-activation lint ───────────────────────────────────────

/// Helper: create a `user/tp` plugin file, write init.scm, run `init_scripting`.
///
/// Parallels `setup_editor_with_init_scripting` but also puts a plugin on disk so
/// `#:languages` activation entries for `"user/tp"` are actually recorded.  (Absent-path
/// plugins early-return in `declare_plugin` and skip activation registration.)
fn setup_lang_lint_editor(init_body: &str) -> (Editor, Vec<tempfile::TempDir>) {
    let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    let config_tmp = tempfile::tempdir().unwrap();
    let runtime_tmp = tempfile::tempdir().unwrap();
    let data_tmp = tempfile::tempdir().unwrap();

    // Trivial plugin body — the lint checks activation entry names, not plugin behaviour.
    let plugin_dir = data_tmp
        .path()
        .join("hume")
        .join("plugins")
        .join("user")
        .join("tp");
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
    ed.init_scripting(&mut Default::default());

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HUME_RUNTIME");
        std::env::remove_var("XDG_DATA_HOME");
    }

    (ed, vec![config_tmp, runtime_tmp, data_tmp])
}

/// Language-activation lint warns when `#:languages` names a language that no
/// `define-language!` has registered.
///
/// Flip: remove the post-init language-activation lint → no Warning produced →
/// assertion fires.
#[test]
fn language_activation_lint_warns_on_unknown_language() {
    use crate::editor::Severity;

    let (ed, _dirs) = setup_lang_lint_editor(r#"(declare-plugin "user/tp" #:languages '("rsut"))"#);

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| { e.severity == Severity::Warning && e.text.contains("rsut") }),
        "expected Warning about unknown language 'rsut'; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// Language-activation lint is silent when the declared language was registered via
/// `define-language!` earlier in the same init.scm.
///
/// Flip: running the lint before the second language flush (instead of after)
/// would incorrectly warn here because the flush has not yet applied the
/// `define-language!` call to `state.config.languages`.
#[test]
fn language_trigger_lint_silent_for_known_language() {
    use crate::editor::Severity;

    // %define-language! (the Rust primitive) works without prelude.scm
    // (the macro wrapper in languages.scm is absent in the test environment).
    let (ed, _dirs) = setup_lang_lint_editor(
        r#"(%define-language! "foo" '() '() '())
           (declare-plugin "user/tp" #:languages '("foo"))"#,
    );

    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| { e.severity == Severity::Warning && e.text.contains("foo") }),
        "must not warn about known language 'foo'; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// Forward-reference order-independence: `declare-plugin #:languages '("foo")`
/// appearing BEFORE `define-language! "foo"` in the same init.scm must not warn.
///
/// A declare-time check would see "foo" absent from the live registry and falsely
/// reject it.  The post-init placement (after every eval's effects are applied)
/// makes the check order-independent.
///
/// Flip: move the lint before `init.scm`'s `apply_script_effects` call →
/// "foo" is not yet in `state.config.languages` → lint emits a spurious Warning →
/// assertion fires.
#[test]
fn language_trigger_lint_silent_for_forward_defined_language() {
    use crate::editor::Severity;

    // declare-plugin BEFORE define-language! — the forward-reference case.
    let (ed, _dirs) = setup_lang_lint_editor(
        r#"(declare-plugin "user/tp" #:languages '("foo"))
           (%define-language! "foo" '() '() '())"#,
    );

    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| { e.severity == Severity::Warning && e.text.contains("foo") }),
        "forward-defined language must not warn; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// Language-activation lint never warns about `"*"` — it's the any-language
/// wildcard, not a language identity to look up in the registry.
///
/// Flip: drop the `lang != "*"` guard from the lint → "*" is looked up in
/// `state.config.languages`, is never found, and a spurious Warning fires on every
/// startup for any manifest.scm using the wildcard.
#[test]
fn language_activation_lint_silent_for_wildcard() {
    use crate::editor::Severity;

    let (ed, _dirs) = setup_lang_lint_editor(r#"(declare-plugin "user/tp" #:languages '("*"))"#);

    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| { e.severity == Severity::Warning && e.text.contains('*') }),
        "must not warn about the \"*\" wildcard; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// A real core plugin with no `manifest.scm` of its own (`core:vim-keybind`) still
/// hard-errors on a zero-trigger `(declare-plugin "core:vim-keybind")` against the
/// repo's actual `runtime/` tree — the manifest opt-in doesn't silently make
/// every plugin support the zero-trigger form.
///
/// Flip: if manifest resolution fell back to some default instead of hard
/// erroring on a missing file, this would incorrectly log no error at all.
#[test]
fn core_vim_keybind_has_no_manifest_scm_zero_trigger_declare_errors() {
    use crate::editor::Severity;

    let runtime_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("hume-editor/ must have a parent (the repo root)")
        .join("runtime");
    assert!(
        !runtime_dir
            .join("plugins")
            .join("core")
            .join("vim-keybind")
            .join("manifest.scm")
            .exists(),
        "sanity: core:vim-keybind must NOT ship a manifest.scm for this negative check to be meaningful"
    );

    let (ed, _dirs) = setup_editor_with_init_scripting(
        r#"(declare-plugin "core:vim-keybind")"#,
        Some(&runtime_dir),
    );

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| { e.severity == Severity::Error && e.text.contains("manifest.scm") }),
        "must log an Error naming the missing manifest.scm; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

// ── End-to-end: real manifest.scm ─────────────────────────────────────────

/// The real `core:lsp` plugin's own shipped `manifest.scm` (not a synthetic
/// fixture) resolves and evaluates via a zero-trigger `(declare-plugin
/// "core:lsp")`, through the full production `init_scripting` path against the
/// repo's actual `runtime/` tree.
///
/// Flip: a syntax error, a wrong plugin name, or a stale command list in the
/// real `runtime/plugins/core/lsp/manifest.scm` would fail this test while
/// every synthetic-fixture test elsewhere in this file still passes.
#[test]
fn core_lsp_real_manifest_scm_resolves_via_zero_trigger_declare() {
    use crate::editor::Severity;
    use hume_scripting::attribution::PluginId;

    let runtime_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("hume-editor/ must have a parent (the repo root)")
        .join("runtime");
    assert!(
        runtime_dir
            .join("plugins")
            .join("core")
            .join("lsp")
            .join("manifest.scm")
            .exists(),
        "sanity: the real manifest.scm must exist at the expected repo path"
    );

    let (ed, _dirs) = setup_editor_with_init_scripting(
        r#"(load-plugin "core:stdlib")
           (register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))
           (declare-plugin "core:lsp")"#,
        Some(&runtime_dir),
    );

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        errors.is_empty(),
        "init.scm with core:lsp's real manifest.scm must not log errors; got: {errors:?}"
    );

    let id = PluginId::Core("lsp".to_string());
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Declared)
        ),
        "core:lsp must be Declared (not yet activated) once the zero-trigger \
         declare resolves its manifest.scm"
    );
    assert!(
        matches!(
            ed.state.config.registry.get_mappable("lsp-install"),
            Some(MappableCommand::Lazy { .. })
        ),
        "manifest.scm's #:commands entries must be registered as Lazy stubs, \
         including \"lsp-install\""
    );
    assert!(
        !ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("some-made-up-language")
            .is_empty(),
        "manifest.scm's #:languages '(\"*\") must match any language, including an unregistered one"
    );
}

/// The real `core:stdlib` plugin's own shipped `manifest.scm` resolves and evaluates via a
/// zero-trigger `(declare-plugin "core:stdlib")`, through the full production
/// `init_scripting` path against the repo's actual `runtime/` tree.
///
/// Flip: a syntax error, a wrong plugin name, or a stale command list in the real
/// `runtime/plugins/core/stdlib/manifest.scm` would fail this test while every
/// synthetic-fixture test elsewhere in this file still passes.
#[test]
fn core_stdlib_real_manifest_scm_resolves_via_zero_trigger_declare() {
    use crate::editor::Severity;
    use hume_scripting::attribution::PluginId;

    let runtime_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("hume-editor/ must have a parent (the repo root)")
        .join("runtime");
    assert!(
        runtime_dir
            .join("plugins")
            .join("core")
            .join("stdlib")
            .join("manifest.scm")
            .exists(),
        "sanity: the real manifest.scm must exist at the expected repo path"
    );

    let (ed, _dirs) =
        setup_editor_with_init_scripting(r#"(declare-plugin "core:stdlib")"#, Some(&runtime_dir));

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        errors.is_empty(),
        "init.scm with core:stdlib's real manifest.scm must not log errors; got: {errors:?}"
    );

    let id = PluginId::Core("stdlib".to_string());
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Declared)
        ),
        "core:stdlib must be Declared (not yet activated) once the zero-trigger \
         declare resolves its manifest.scm"
    );
    assert!(
        matches!(
            ed.state
                .config
                .registry
                .get_mappable("stdlib/all-single-char?"),
            Some(MappableCommand::Lazy { .. })
        ),
        "manifest.scm's #:commands entries must be registered as Lazy stubs, \
         including \"stdlib/all-single-char?\""
    );
}

/// The real `core:plum` plugin's own shipped `manifest.scm` resolves and evaluates via a
/// zero-trigger `(declare-plugin "core:plum")`, through the full production `init_scripting`
/// path against the repo's actual `runtime/` tree.
///
/// Flip: a syntax error, a wrong plugin name, or a stale command/language list in the real
/// `runtime/plugins/core/plum/manifest.scm` would fail this test while every synthetic-fixture
/// test elsewhere in this file still passes.
#[test]
fn core_plum_real_manifest_scm_resolves_via_zero_trigger_declare() {
    use crate::editor::Severity;
    use hume_scripting::attribution::PluginId;

    let runtime_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("hume-editor/ must have a parent (the repo root)")
        .join("runtime");
    assert!(
        runtime_dir
            .join("plugins")
            .join("core")
            .join("plum")
            .join("manifest.scm")
            .exists(),
        "sanity: the real manifest.scm must exist at the expected repo path"
    );

    let (ed, _dirs) =
        setup_editor_with_init_scripting(r#"(declare-plugin "core:plum")"#, Some(&runtime_dir));

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        errors.is_empty(),
        "init.scm with core:plum's real manifest.scm must not log errors; got: {errors:?}"
    );

    let id = PluginId::Core("plum".to_string());
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Declared)
        ),
        "core:plum must be Declared (not yet activated) once the zero-trigger \
         declare resolves its manifest.scm"
    );
    assert!(
        matches!(
            ed.state.config.registry.get_mappable("plum-list"),
            Some(MappableCommand::Lazy { .. })
        ),
        "manifest.scm's #:commands entries must be registered as Lazy stubs, \
         including \"plum-list\""
    );
    assert!(
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("some-made-up-language")
            .is_empty(),
        "manifest.scm declares no #:languages — startup grammar registration is \
         core's job, so core:plum has no reason to activate on a language set"
    );
}

/// Keymap lint is silent when every bound key targets a registered command.
///
/// Flip: the test above binds an *unknown* name and asserts a Warning is
/// produced — this test confirms the warning path does not fire for valid names.
#[test]
fn keymap_lint_silent_for_known_command() {
    use crate::editor::Severity;
    use crate::editor::keymap::BindMode;

    let (ed, _dirs) =
        setup_editor_with_init_scripting(r#"(bind-key! 'normal "Q" "move-down")"#, None);

    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| { e.severity == Severity::Warning && e.text.contains("move-down") }),
        "must not warn about known command 'move-down'; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
    // The lint reads all three tries at once, so it can't tell Normal from
    // Insert — this is what pins `Effect::BindKey`'s mode all the way through
    // `to_editor_bind_mode` into the right trie.
    assert!(
        ed.state
            .config
            .keymap
            .lookup_command(BindMode::Normal, &[key('Q')])
            .is_some(),
        "init.scm's bind-key! must land in the Normal trie specifically"
    );
}

// ── core:stdlib — real shipped plugin ─────────────────────────────────────────

/// Stage the real shipped `core:stdlib` plugin into an isolated `HUME_RUNTIME`
/// and eagerly load it via a real `init.scm`, returning everything the caller
/// needs kept alive plus the host for further inspection or `eval_source`.
fn setup_stdlib_editor() -> (Editor, ScriptingHost, HumeRuntimeGuard, tempfile::TempDir) {
    let guard = HumeRuntimeGuard::new();
    write_core_plugin(&guard, "stdlib", STDLIB_PLUGIN);

    let init_dir = tempfile::tempdir().unwrap();
    let init_path = init_dir.path().join("init.scm");
    std::fs::write(&init_path, r#"(load-plugin "core:stdlib")"#).unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed loading core:stdlib");

    (ed, host, guard, init_dir)
}

/// The shipped `core:stdlib` plugin must load eagerly and reach `Loaded`.
#[test]
fn core_stdlib_plugin_loads_eagerly() {
    use hume_scripting::attribution::PluginId;

    let (_ed, host, _guard, _init_dir) = setup_stdlib_editor();

    let id = PluginId::parse("core:stdlib").expect("valid plugin name");
    assert_eq!(
        host.plugin_status(&id),
        Some(PluginStatus::Loaded),
        "core:stdlib must be Loaded after eager load-plugin"
    );
}

/// The `core:stdlib` selection-query commands must compute the expected
/// results on literal selection-list arguments via `call!`, and pass `#f`
/// straight through untouched — the same cross-plugin surface
/// `core:vim-keybind` uses for its conditional `C` binding.
///
/// Each assertion is a hand-written literal-tuple oracle, independent of the
/// implementation: if any command computes the wrong result, its `unless`
/// fires `(error ...)`, which propagates as an `Err` — caught by the assert
/// below, failing the test with the offending assertion name.
///
/// `stdlib`'s per-triple accessors (`selection-anchor`/`-head`/`-primary?`,
/// `primary-selection`) are module-private composition helpers with no
/// `(provide)` — they are intentionally not directly testable from outside
/// the plugin's module. The three commands below exercise every accessor
/// transitively (primary-flag selection on both ends, `#f` passthrough).
#[test]
fn core_stdlib_selection_commands() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();

    let assertions = r#"
(unless (equal? (call! "stdlib/all-single-char?" #f) #f) (error "all-single-char? #f passthrough"))
(unless (equal? (call! "stdlib/single-selection?" #f) #f) (error "single-selection? #f passthrough"))
(unless (equal? (call! "stdlib/cursor-char-index" #f) #f) (error "cursor-char-index #f passthrough"))

(unless (equal? (call! "stdlib/single-selection?" (list (list 0 1 #t))) #t)
  (error "single-selection? true"))
(unless (equal? (call! "stdlib/single-selection?" (list (list 0 1 #t) (list 2 3 #f))) #f)
  (error "single-selection? false"))

(unless (equal? (call! "stdlib/all-single-char?" (list (list 2 2 #t) (list 5 5 #f))) #t)
  (error "all-single-char? true"))
(unless (equal? (call! "stdlib/all-single-char?" (list (list 2 3 #t))) #f)
  (error "all-single-char? false"))
(unless (equal? (call! "stdlib/all-single-char?" (list (list 2 2 #f) (list 5 6 #t))) #f)
  (error "all-single-char? false when only a later selection is wide"))

(unless (equal? (call! "stdlib/cursor-char-index" (list (list 0 0 #f) (list 7 4 #t))) 4)
  (error "cursor-char-index picks the primary's head"))
"#;

    let result = {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_source(assertions, &mut ih)
    };
    assert!(
        result.is_ok(),
        "stdlib selection command assertions must all pass: {result:?}"
    );
}

// ── Retrospective issue 2: one registry, one dispatcher — new coverage ───────

/// A lazy command's first dispatch leaves identical bookkeeping whether
/// triggered via keypress-style dispatch or the `:` command line — both are
/// an "outer" `Editor::dispatch` call for the same command name, so both
/// stamp `last_command`/jump/paste bookkeeping identically.
///
/// Not compared against a `call!`-from-another-command path: `call!`'s
/// bookkeeping is deliberately outer-name-wins (see `dispatch.rs`'s
/// `run_steel_command` — "Outer-name-wins: stamp the outer command so `.`
/// replays it, not any inner command the body dispatched via `call!`") — a
/// command reached via an outer wrapper stamps the WRAPPER's name, not the
/// inner command's, so a 3-way keypress/`:`/`call!` identity claim would be
/// asserting behavior the system deliberately does not have.
///
/// Fail oracle: if lazy activation's AFTER-stage bookkeeping (jump/paste/
/// last_command) diverged between the two entry points — e.g. one skipped
/// the repeatable-action stamp — one of the two snapshots would differ.
#[test]
fn lazy_command_first_dispatch_parity_keypress_vs_minibuf() {
    let (mut ed_key, _dir_key) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("bar"))"#,
        r#"(define-command! "bar" "" (lambda () (call! "delete")) #:repeatable #t)"#,
    );
    let before_key = snapshot_bookkeeping(&ed_key);
    ed_key.execute_keymap_command("bar".into(), Some(1), false, ArgSource::Keymap);
    let snap_key = snapshot_bookkeeping(&ed_key);

    let (mut ed_mb, _dir_mb) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("bar"))"#,
        r#"(define-command! "bar" "" (lambda () (call! "delete")) #:repeatable #t)"#,
    );
    let before_mb = snapshot_bookkeeping(&ed_mb);
    type_cmd(&mut ed_mb, ":bar");
    let snap_mb = snapshot_bookkeeping(&ed_mb);

    assert_eq!(
        before_key, before_mb,
        "pre-condition: both fresh editors must start identical"
    );
    assert_eq!(
        snap_key, snap_mb,
        "lazy command's first dispatch must leave identical bookkeeping via \
         keypress vs `:` line"
    );
    // Both paths must have actually activated the plugin and run the command.
    assert_eq!(ed_key.doc().text().to_string(), "b\n");
    assert_eq!(ed_mb.doc().text().to_string(), "b\n");
}

/// `:cmd arg` on a lazy command's very first dispatch must forward `arg` to
/// the lambda — pins the ordering `Editor::run_steel_command` now depends on:
/// activation (which replaces the `Lazy` stub with `SteelBacked`) must
/// complete before `ArgSource::Minibuf` marshalling reads the resolved arity.
///
/// Fail oracle: if activation ran after arg marshalling instead of before,
/// the stub's `Lazy` arity (not yet resolved) would be used instead of the
/// real lambda's arity, and the forwarded arg would never reach `call!`.
#[test]
fn lazy_command_first_call_minibuf_arg_forwarded() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("echo-arg"))"#,
        r#"(define-command! "echo-arg" "" (lambda (x) (when (string? x) (call! x))))"#,
    );
    let before = state(&ed);

    // The typed arg "move-right" is a native command name — echo-arg forwards
    // it straight to `call!`, so a cursor move is observable proof the arg
    // arrived, not just that some command ran.
    type_cmd(&mut ed, ":echo-arg move-right");

    assert_ne!(
        state(&ed),
        before,
        "first :echo-arg dispatch must forward the typed arg (\"move-right\") \
         to the lambda"
    );
    assert!(
        matches!(
            ed.state.config.registry.get_mappable("echo-arg"),
            Some(MappableCommand::SteelBacked { .. })
        ),
        "echo-arg stub must have activated on first dispatch"
    );
}

/// A failed activation removes EVERY remaining `Lazy` stub of that plugin —
/// not just the one that triggered the activation. Extends
/// `body_error_removes_stub_and_marks_failed` (single-command case) to a
/// plugin declaring two commands, only one of which is dispatched.
///
/// Before `CommandHost::unregister_lazy_stubs_of` was called from
/// `finish_lazy_activation`, a sibling stub survived as a dangling `Lazy`
/// entry pointing at a now-`Failed` plugin until it was itself dispatched
/// (and only then cleaned up by the per-dispatch loop guard) — a behavior
/// improvement this test pins.
///
/// Fail oracle: revert to only unregistering the dispatched stub (the old
/// per-dispatch loop guard alone) → `stub-b` survives as `Lazy` after
/// `stub-a`'s activation fails.
#[test]
fn failed_activation_removes_all_of_the_plugins_stubs_not_just_the_dispatched_one() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("stub-a" "stub-b"))"#,
        r#"(error "intentional plugin failure")"#,
    );
    assert!(
        matches!(
            ed.state.config.registry.get_mappable("stub-a"),
            Some(MappableCommand::Lazy { .. })
        ),
        "stub-a must be present before dispatch"
    );
    assert!(
        matches!(
            ed.state.config.registry.get_mappable("stub-b"),
            Some(MappableCommand::Lazy { .. })
        ),
        "stub-b must be present before dispatch"
    );

    type_cmd(&mut ed, ":stub-a");

    assert!(
        ed.state.config.registry.get_mappable("stub-a").is_none(),
        "the dispatched stub must be gone after failed activation"
    );
    assert!(
        ed.state.config.registry.get_mappable("stub-b").is_none(),
        "a sibling stub of the same failed plugin must ALSO be gone \
         immediately — not left dangling until it is itself dispatched"
    );
}

/// `:plugin-status` reports a `Declared` plugin's pending `cmd:` activation
/// entries sourced from the editor's live `Lazy` stubs — the plumbing
/// `lazy_status_string`/`format_status` now require since this crate no
/// longer tracks pending command activations itself.
///
/// Fail oracle: if `typed_plugin_status` stopped passing `registry.lazy_stubs()`
/// through, `:plugin-status` would show no `cmd:` entries for any `Declared`
/// plugin regardless of what it actually declared.
#[test]
fn plugin_status_shows_pending_command_from_live_registry_stubs() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:commands '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );

    type_cmd(&mut ed, ":plugin-status");

    assert_eq!(
        ed.doc().display_name(),
        "[plugin-status]",
        ":plugin-status must open the [plugin-status] read-only view"
    );
    let out = ed.doc().text().to_string();
    assert!(
        out.contains("user/tp") && out.contains("cmd:bar"),
        ":plugin-status must show the pending cmd:bar entry for user/tp; got: {out:?}"
    );
}
