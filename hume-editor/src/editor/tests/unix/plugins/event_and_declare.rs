//! Lazy plugin loading: event activations, and load-plugin! /
//! declare-plugin! interaction (editor-level).

use super::*;
use crate::editor::registry::MappableCommand;
use hume_platform::dirs::Dirs;
use hume_scripting::PluginStatus;

// ── Lazy plugin loading — event activations ──────────────────────────

/// `#:events` plugin activates on first matching hook fire; its handler
/// runs in the same fire that caused activation.
///
/// Without `activate_lazy_event_plugins` at the top of `queue_event`, the
/// plugin would stay `Declared` and the cursor would never move.
#[test]
fn event_trigger_activates_on_first_fire() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:events '(on-buffer-save))"#,
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right" (focused-pane))))"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };

    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
            Some(PluginStatus::Declared)
        ),
        "plugin must be Declared before first fire"
    );
    assert!(
        !ed.scripting
            .as_ref()
            .unwrap()
            .activation_event_plugins("on-buffer-save")
            .is_empty(),
        "activation_events must be populated before first fire"
    );

    let before = state(&ed);
    let bid = ed.focused_buffer_id();
    ed.queue_buffer_save(bid);
    ed.settle();

    assert_ne!(
        state(&ed),
        before,
        "hook handler must run and move the cursor on first fire"
    );
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
            Some(PluginStatus::Loaded)
        ),
        "plugin must be Loaded after first fire"
    );
    assert!(
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_event_plugins("on-buffer-save")
            .is_empty(),
        "activation_events must be cleared after plugin loads"
    );
}

/// Second fire: handler still runs (plugin already `Loaded`); no re-activation.
///
/// Even if `activation_events` kept its entry after load, `activate_plugin`'s
/// `Loaded` guard would make the second fire harmless. This test pins the fast
/// path instead, with no spurious activation attempt.
#[test]
fn event_trigger_idempotent_on_second_fire() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:events '(on-buffer-save))"#,
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right" (focused-pane))))"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    let bid = ed.focused_buffer_id();

    ed.queue_buffer_save(bid); // first fire: activates
    ed.settle();
    assert!(
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_event_plugins("on-buffer-save")
            .is_empty(),
        "activation_events must be empty after first fire"
    );

    let after_first = state(&ed);
    ed.queue_buffer_save(bid); // second fire: handler runs, no re-activation
    ed.settle();

    assert_ne!(
        state(&ed),
        after_first,
        "handler must run again on second fire"
    );
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
            Some(PluginStatus::Loaded)
        ),
        "plugin must remain Loaded after second fire (not re-enter Declared)"
    );
}

/// 1:many: two plugins both declare `#:events '(on-buffer-save)`; a single
/// fire activates both.
#[test]
fn event_trigger_one_to_many_activates_all() {
    use hume_scripting::attribution::PluginId;

    let dir = tempfile::tempdir().unwrap();
    let dir_a = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::write(
        dir_a.join("plugin.scm"),
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right" (focused-pane))))"#,
    )
    .unwrap();
    let dir_b = dir.path().join("plugins").join("user").join("tp2");
    std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(
        dir_b.join("plugin.scm"),
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right" (focused-pane))))"#,
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(declare-plugin! \"user/tp\"  #:events '(on-buffer-save))\n\
         (declare-plugin! \"user/tp2\" #:events '(on-buffer-save))",
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b c d\n");
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

    let id_a = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    let id_b = PluginId::User {
        user: "user".to_string(),
        repo: "tp2".to_string(),
    };
    let bid = ed.focused_buffer_id();
    ed.queue_buffer_save(bid);
    ed.settle();

    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id_a.clone())),
            Some(PluginStatus::Loaded)
        ),
        "plugin A must be Loaded after fire"
    );
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id_b.clone())),
            Some(PluginStatus::Loaded)
        ),
        "plugin B must be Loaded after fire"
    );
    assert!(
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_event_plugins("on-buffer-save")
            .is_empty(),
        "activation_events must be fully cleared after both plugins load"
    );
}

/// Body error: plugin raises at load time → `Failed`, error reported, activation
/// entry cleared, with no retry on a second fire.
///
/// Without the `activation_events` drop in `drop_activations_for`'s failure
/// path, the same plugin would retry activation on every fire.
#[test]
fn event_plugin_failure_marks_failed_no_retry() {
    use hume_scripting::attribution::PluginId;

    use crate::editor::Severity;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:events '(on-buffer-save))"#,
        r#"(error "intentional plugin failure")"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    let bid = ed.focused_buffer_id();

    ed.queue_buffer_save(bid); // first fire: activates → body fails
    ed.settle();

    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
            Some(PluginStatus::Failed)
        ),
        "plugin must be Failed after body error"
    );
    assert!(
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_event_plugins("on-buffer-save")
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
    ed.queue_buffer_save(bid); // second fire: no retry
    ed.settle();

    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
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

// ── load-plugin! / declare-plugin! interaction (editor-level) ────────

/// `(declare-plugin! "name")` with no activation entries is a hard error: the plugin
/// could never activate at runtime.
#[test]
fn declare_plugin_no_triggers_is_hard_error() {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(
        plugin_dir.join("plugin.scm"),
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, "(declare-plugin! \"user/tp\")").unwrap();

    let mut host = ScriptingHost::new(&Dirs {
        data: Some(dir.path().to_path_buf()),
        ..Dirs::none()
    });
    let mut ed = editor_from("-[a]>b\n");
    let result = {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    };
    assert!(
        result.is_err(),
        "declare-plugin! with no activation entries must abort init with an error"
    );
}

/// Top-level `(load-plugin! "user/tp")` with the plugin absent on disk → silent
/// skip (PLUM-friendly bootstrap), no error, and the plugin is recorded in
/// `declared-plugins` but absent from `loaded-plugins`.
///
/// An error here would stop users from declaring third-party plugins before
/// running `:plum-install-plugins` on a fresh setup.
#[test]
fn load_plugin_absent_top_level_silently_skips() {
    let dir = tempfile::tempdir().unwrap();
    // No plugin directory created: plugin is absent on disk.
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, r#"(load-plugin! "user/tp")"#).unwrap();

    let mut host = ScriptingHost::new(&Dirs {
        data: Some(dir.path().to_path_buf()),
        ..Dirs::none()
    });
    let mut ed = editor_from("-[a]>b\n");
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("absent top-level load-plugin! must not error");

    // Plugin was not inserted into lazy_registry (absent on disk).
    assert!(
        !host.has_any_loaded_plugin(),
        "no plugin must be Loaded when absent on disk"
    );
}

/// A lazy plugin B can call another lazy plugin A's command via `(call! "a-cmd")`.
/// The inline lazy-miss retry in `%dispatch-command!` activates A on the fly and
/// runs the command, with no `(load-plugin!)` needed.
///
/// Without the lazy-miss retry in `%dispatch-command!`, `(call! "a-cmd")` would
/// fall through to `%call-native!`, which does not know `a-cmd`, so it would log
/// a warning and the cursor would stay put.
#[test]
fn plugin_calls_cross_plugin_cmd_auto_activates_dep() {
    use hume_scripting::attribution::PluginId;

    let dir = tempfile::tempdir().unwrap();
    // Plugin A: defines "a-cmd" (move-right wrapper).
    let dir_a = dir.path().join("plugins").join("user").join("tpa");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::write(
        dir_a.join("plugin.scm"),
        r#"(define-command! "a-cmd" "doc" (lambda () (call! "move-right" (focused-pane))))"#,
    )
    .unwrap();
    // Plugin B: command activation entry; b-cmd's body calls "a-cmd" inline
    // (no load-plugin!), reached only once a keypress dispatches b-cmd itself.
    let dir_b = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(
        dir_b.join("plugin.scm"),
        "(define-typed-command! \"b-cmd\" \"doc\" (lambda () (call! \"a-cmd\")))",
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(declare-plugin! \"user/tpa\" #:commands '(\"a-cmd\"))\n\
         (declare-plugin! \"user/tp\"  #:typed-commands '(\"b-cmd\"))",
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b\n");
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
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id_a.clone())),
            Some(PluginStatus::Declared)
        ),
        "dep A must be Declared after init"
    );
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id_b.clone())),
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
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id_b.clone())),
            Some(PluginStatus::Loaded)
        ),
        "plugin B must be Loaded after dispatch"
    );
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id_a.clone())),
            Some(PluginStatus::Loaded)
        ),
        "dep A must be Loaded after B calls (call! \"a-cmd\")"
    );
}

/// Nested inline activation reached from a plugin's own top-level body (not
/// a command a keypress later dispatches), with both plugins
/// multi-file: B's `plugin.scm` `require`s a sibling file, then top-level
/// `(call! "a-cmd")`s a second, also multi-file, plugin A, re-entering
/// `hm.eval-string` while B's own `require` chain is still on the Steel call
/// stack. This is the shape `core:git-diff`/`core:lsp`'s own `core:stdlib`
/// guard exists to protect (both `require` several sibling files before
/// their guard runs).
///
/// The underlying nested-activation machinery is already covered against
/// `NullHost` with single-file plugins by
/// `nested_activation_commit_survives_enclosing_plugin_failure`
/// (`hume-scripting/src/activation/tests.rs`). This pins the same contract
/// through the real editor host with multi-file plugins.
#[test]
fn nested_activation_multi_file_via_real_editor_host() {
    use hume_scripting::attribution::PluginId;

    let dir = tempfile::tempdir().unwrap();

    // Each plugin's top-level body checks its own helper.scm binding twice
    // (once right after `require`, once after the nested call!), so a stack
    // mis-scoping that corrupts either plugin's module bindings fails loudly
    // as an eval_init error, not silently.
    let dir_a = dir.path().join("plugins").join("user").join("tpa");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::write(
        dir_a.join("helper.scm"),
        "(provide a-helper-marker)\n(define a-helper-marker 9)\n",
    )
    .unwrap();
    std::fs::write(
        dir_a.join("plugin.scm"),
        "(require \"helper.scm\")\n\
         (unless (equal? a-helper-marker 9)\n\
         \x20 (error \"A's own helper.scm binding must be visible in A's body\"))\n\
         (define-command! \"a-cmd\" \"doc\" (lambda () (+ 1 0)))",
    )
    .unwrap();

    let dir_b = dir.path().join("plugins").join("user").join("tpb");
    std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(
        dir_b.join("helper.scm"),
        "(provide b-helper-marker)\n(define b-helper-marker 7)\n",
    )
    .unwrap();
    std::fs::write(
        dir_b.join("plugin.scm"),
        "(require \"helper.scm\")\n\
         (call! \"a-cmd\")\n\
         (unless (equal? b-helper-marker 7)\n\
         \x20 (error \"B's own helper.scm binding must survive nested activation of A\"))\n\
         (define-command! \"b-cmd\" \"doc\" (lambda () (+ 1 0)))",
    )
    .unwrap();

    // A is merely declared: its activation must come from B's nested call!,
    // not from its own top-level load. B loads eagerly, running its body
    // (and the nested call! into A) during this eval_init.
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(declare-plugin! \"user/tpa\" #:commands '(\"a-cmd\"))\n\
         (load-plugin! \"user/tpb\")",
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs {
        data: Some(dir.path().to_path_buf()),
        ..Dirs::none()
    });
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed: B's top-level call! must inline-activate multi-file A");
    ed.scripting = Some(host);

    let id_a = PluginId::User {
        user: "user".to_string(),
        repo: "tpa".to_string(),
    };
    let id_b = PluginId::User {
        user: "user".to_string(),
        repo: "tpb".to_string(),
    };
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id_a.clone())),
            Some(PluginStatus::Loaded)
        ),
        "A must be Loaded: its multi-file require completed under nested activation"
    );
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id_b.clone())),
            Some(PluginStatus::Loaded)
        ),
        "B must be Loaded: its own multi-file require completed despite nesting a call! mid-body"
    );
}

/// A plugin body's `(plugin-config)` read must resolve to its own `#:config`
/// even after nested-activating a dependency in between: `plugin-config`
/// resolves off the top of `plugin_stack`
/// (`hume-scripting/src/builtins/plugins.rs`), which `%finish-lazy-activation!`
/// pops back to the enclosing plugin once the nested activation completes.
///
/// If the stack were left unpopped (or popped twice) after A's nested
/// activation, B's `(plugin-config)` read below would see A's hash, and the
/// `hash-ref`/`equal?` check inside B's body would raise.
#[test]
fn plugin_config_scoped_correctly_after_nested_activation() {
    let dir = tempfile::tempdir().unwrap();

    let dir_a = dir.path().join("plugins").join("user").join("tpa");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::write(
        dir_a.join("plugin.scm"),
        "(define-command! \"a-cmd\" \"doc\" (lambda () (call! \"move-right\")))",
    )
    .unwrap();

    let dir_b = dir.path().join("plugins").join("user").join("tpb");
    std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(
        dir_b.join("plugin.scm"),
        "(call! \"a-cmd\")\n\
         (unless (equal? (hash-ref (plugin-config) \"y\") 2)\n\
         \x20 (error \"B's plugin-config must be B's own after A's nested activation\"))\n\
         (define-command! \"b-cmd\" \"doc\" (lambda () (call! \"move-right\")))",
    )
    .unwrap();

    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(declare-plugin! \"user/tpa\" #:commands '(\"a-cmd\"))\n\
         (declare-plugin! \"user/tpb\" #:commands '(\"b-cmd\"))\n\
         (load-plugin! \"user/tpa\" #:config (hash \"x\" 1))\n\
         (load-plugin! \"user/tpb\" #:config (hash \"y\" 2))",
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs {
        data: Some(dir.path().to_path_buf()),
        ..Dirs::none()
    });
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed: B must see its own #:config after nested-activating A");
}

/// A lazy plugin activated via the in-Steel `call!` path whose body tries to
/// shadow a native command must fail cleanly, and the native command survives.
///
/// The in-Steel path matters: `SteelCtx::new_command` carries an empty
/// `builtin_cmd_names` set, so the Steel-side shadow guard is inert and the
/// conflict is only caught by `host.register_command`.  The failed define must
/// not leave `command_table`/`cmd_owners` entries that make the plugin-failure
/// rollback unregister the native command.
///
/// Two changes would each make `move-left` vanish from the registry: inserting
/// into `command_table`/`cmd_owners` before `host.register_command` in
/// `define_command`, or having `CommandRegistry::unregister` remove
/// unconditionally.
#[test]
fn native_command_survives_failed_shadowing_plugin() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        // Eager command whose body triggers the lazy plugin via call!:
        // the in-Steel activation path (empty builtin_cmd_names).
        r#"(declare-plugin! "user/tp" #:commands '("bar"))
           (define-typed-command! "trigger" "doc" (lambda () (call! "bar")))"#,
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
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
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
/// the binding must never be applied. A `Failed` plugin leaves no dangling
/// keybinding pointing at a command that (if it was also rolled back, or was
/// never valid) can no longer be dispatched.
///
/// `bind-key!` only queues an `Effect::BindKey`, and two independent layers
/// drop it: `pop_effect_marks(false)` discards the failed body's uncommitted
/// entries, and `take_eval_effects` hands the dispatcher's `Err` arm only the
/// *committed* ones. `Q` never reaches the keymap.
///
/// The bind would reach `apply_script_effects` only if both layers failed at
/// once: `pop_effect_marks`'s failure branch keeping every entry and
/// `take_eval_effects` salvaging uncommitted ones on `Err`. Either layer alone
/// is enough, so a bind is never applied unless the body that queued it
/// committed.
#[test]
fn plugin_keybinding_rolled_back_on_failed_activation() {
    use crate::editor::keymap::BindMode;
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:commands '("bar"))
           (define-typed-command! "trigger" "doc" (lambda () (call! "bar")))"#,
        r#"(bind-key! 'normal "Q" "some-cmd") (error "boom")"#,
    );

    type_cmd(&mut ed, ":trigger");

    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
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
/// errors: the hook must not survive, since a `Failed` plugin's hooks must stop
/// firing.
///
/// Without `hooks.remove_owned_by` in `finish_lazy_activation`,
/// `has_hook_handlers` would still report `true` after the failure.
#[test]
fn plugin_hook_rolled_back_on_failed_activation() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:commands '("bar"))
           (define-typed-command! "trigger" "doc" (lambda () (call! "bar")))"#,
        r#"(register-hook! 'on-buffer-save (lambda (bid) 0)) (error "boom")"#,
    );

    type_cmd(&mut ed, ":trigger");

    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
            Some(PluginStatus::Failed)
        ),
        "plugin must be Failed after intentional error"
    );
    assert!(
        !ed.scripting
            .as_ref()
            .unwrap()
            .has_hook_handlers("on-buffer-save", None),
        "the failed plugin's register-hook! must not survive rollback"
    );
}

/// Same rollback for a method-filtered handler: a failed plugin's
/// `register-lsp-notification-hook!` stops claiming its method, so that
/// method is traced as unhandled again.
#[test]
fn plugin_lsp_notification_hook_rolled_back_on_failed_activation() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:commands '("bar"))
           (define-typed-command! "trigger" "doc" (lambda () (call! "bar")))"#,
        r#"(register-lsp-notification-hook! "m/a" (lambda (server method params) 0))
           (error "boom")"#,
    );

    type_cmd(&mut ed, ":trigger");

    assert!(
        !ed.scripting
            .as_ref()
            .unwrap()
            .has_hook_handlers("on-lsp-notification", Some("m/a")),
        "the failed plugin's method-filtered handler must not survive rollback"
    );
}

/// `#:events '(on-lsp-notification)` plugin activates on the first
/// notification, same as any other lazy event trigger.
///
/// The plugin registers no hook until it activates, so the event must
/// reach `fire_one_event` regardless of what is registered at drain time.
#[test]
fn on_lsp_notification_event_trigger_activates_on_first_fire() {
    use hume_scripting::attribution::PluginId;

    let (mut lazy, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:events '(on-lsp-notification))"#,
        r#"(register-hook! 'on-lsp-notification (lambda (server method params)
             (when (equal? method "custom/event")
               (call! "move-right" (focused-pane)))))"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };

    let tmp = tempfile::tempdir().unwrap();
    let mut rig = crate::editor::tests::lsp_bridge::setup_with(tmp.path(), "-[a]>b\n", |_, _| {});
    rig.ed.scripting = lazy.scripting.take();
    let sid = rig.sid("rust-analyzer");

    let before = state(&rig.ed);
    rig.push(
        sid,
        hume_lsp::codec::Message::Notification {
            method: "custom/event".to_string(),
            params: serde_json::Value::Null,
        },
    );
    rig.ed.settle();
    let ed = rig.ed;

    assert_ne!(
        state(&ed),
        before,
        "on-lsp-notification hook must fire and move the cursor on first activation"
    );
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
            Some(PluginStatus::Loaded)
        ),
        "plugin must be Loaded after first fire"
    );
}

/// A lazy `#:events '(on-lsp-notification)` plugin that activates on a
/// method it registers no handler for leaves that notification traced as
/// unhandled.
#[test]
fn lazy_plugin_activated_by_unlisted_method_traces_unhandled() {
    use hume_scripting::attribution::PluginId;

    let (mut lazy, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:events '(on-lsp-notification))"#,
        r#"(register-lsp-notification-hook! "m/y"
             (lambda (server method params) (call! "move-right" (focused-pane))))"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };

    let tmp = tempfile::tempdir().unwrap();
    let mut rig = crate::editor::tests::lsp_bridge::setup_with(tmp.path(), "-[a]>b\n", |_, _| {});
    rig.ed.scripting = lazy.scripting.take();
    let sid = rig.sid("rust-analyzer");

    let before = state(&rig.ed);
    rig.push(
        sid,
        hume_lsp::codec::Message::Notification {
            method: "m/x".to_string(),
            params: serde_json::Value::Null,
        },
    );
    rig.ed.settle();
    let ed = rig.ed;

    assert_eq!(state(&ed), before, "the m/y handler must not run for m/x");
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
            Some(PluginStatus::Loaded)
        ),
        "plugin must be Loaded after the activating notification"
    );
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("unhandled notification m/x"),
        "m/x has no handler after activation; it must be traced: {log:?}"
    );
}
