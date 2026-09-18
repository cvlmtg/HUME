//! Lazy plugin loading — editor-level tests.

use super::*;
use crate::editor::event::EditorEvent;
use crate::editor::registry::{MappableCommand, TypedBody};
use hume_scripting::PluginStatus;

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
        r#"(declare-plugin "user/tp" #:typed-commands '("bar"))"#,
        r#"(define-typed-command! "bar" "doc" (lambda () (call! "move-right")))"#,
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
    // Stub must be replaced by a real Steel typed command.
    assert!(
        matches!(
            ed.state.config.registry.get_typed("bar").map(|tc| &tc.body),
            Some(TypedBody::Steel { .. })
        ),
        "stub must be replaced by a Steel typed command after first dispatch"
    );
}

/// `call!` can never reach a typed command (`typed_command_table` is a
/// separate table from `command_table` — see its own doc), so a typed-only
/// lazy stub must not be activatable through `call!`: activating a plugin
/// for a lookup that will miss again right after is a permanent side effect
/// for a call that could never succeed.
///
/// Flip: without a mappable-only `%lazy-command-owner`, `(call! "bar")` would
/// see the typed `Lazy` stub as an activatable owner, load the plugin, miss
/// again in `command_table`, and leave the plugin `Loaded` anyway.
#[test]
fn call_bang_does_not_activate_a_typed_only_lazy_stub() {
    use hume_scripting::attribution::PluginId;

    let (ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:typed-commands '("bar"))
           (call! "bar")"#,
        r#"(define-typed-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );

    let tp_id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&tp_id),
            Some(PluginStatus::Declared)
        ),
        "call! on a typed-only name must not activate the plugin"
    );
    assert!(
        matches!(
            ed.state.config.registry.get_typed("bar").map(|tc| &tc.body),
            Some(TypedBody::Lazy(_))
        ),
        "bar's typed stub must remain unresolved"
    );
}

/// Loop guard: if the plugin body never defines the declared command, the stub
/// is removed after dispatch and a Warning is reported.
///
/// Flip: without the loop guard, the stub would remain (infinite retry).
#[test]
fn loop_guard_removes_stub_when_body_never_defines_command() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:typed-commands '("bar"))"#,
        // Plugin body exists but never defines "bar".
        r#"(define-command! "other-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    // Stub must be present before dispatch.
    assert!(
        matches!(
            ed.state.config.registry.get_typed("bar").map(|tc| &tc.body),
            Some(TypedBody::Lazy(_))
        ),
        "Lazy stub must be present before dispatch"
    );

    type_cmd(&mut ed, ":bar");

    // Stub must have been removed by the loop guard.
    assert!(
        ed.state.config.registry.get_typed("bar").is_none(),
        "stub must be removed when body never defines the command"
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
        r#"(declare-plugin "user/tp" #:typed-commands '("bar"))"#,
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
        r#"(declare-plugin "user/tp" #:typed-commands '("bar"))"#,
        r#"(%define-language! "foo" '() '() '() #f)
           (define-typed-command! "bar" "doc" (lambda () (+ 1 0)))"#,
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
/// Flip: without the re-entrancy guard, `pending_work` holds two
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
    let queued_events: Vec<&EditorEvent> = ed
        .state
        .config
        .pending_work
        .iter()
        .filter_map(|w| match w {
            crate::editor::event::PendingWork::Event(e) => Some(e),
            crate::editor::event::PendingWork::Call(..) => None,
        })
        .collect();
    assert_eq!(
        queued_events.len(),
        1,
        "exactly one OnLanguageSet hook must be queued, not a stale duplicate; got: {queued_events:?}"
    );
    assert!(
        matches!(
            queued_events[0],
            EditorEvent::OnLanguageSet { language, .. } if language.as_deref() == Some("python")
        ),
        "the queued OnLanguageSet hook must carry the final ('python') value, not the stale \
         ('rust') one that triggered activation; got: {:?}",
        queued_events[0]
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
        r#"(declare-plugin "user/tp" #:typed-commands '("bar"))"#,
        r#"(error "intentional plugin failure")"#,
    );
    assert!(
        matches!(
            ed.state.config.registry.get_typed("bar").map(|tc| &tc.body),
            Some(TypedBody::Lazy(_))
        ),
        "stub must be present before dispatch"
    );

    type_cmd(&mut ed, ":bar");

    // Stub removed.
    assert!(
        ed.state.config.registry.get_typed("bar").is_none(),
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
        r#"(declare-plugin "user/tp" #:typed-commands '("bar"))"#,
        r#"(define-typed-command! "bar" "doc" (lambda (x) (+ 1 0)))"#,
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
            ed.state.config.registry.get_typed("bar").map(|tc| &tc.body),
            Some(TypedBody::Steel { .. })
        ),
        "stub must be replaced by a Steel typed command after first dispatch with arg"
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
    let dir = safe_tempdir();
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
        let mut ih = init_host!(ed);
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
/// Runs against a real `Editor` + `EditorHostImpl`, not a hand-rolled
/// `MockHost` — collision detection is `CommandRegistry`'s decision, and a
/// `MockHost` copy of the same rules would risk silently drifting from the
/// real behavior it's meant to prove.
///
/// Flip: remove the `register_lazy_command` collision check → "bar" would
/// register twice, the second `Lazy { plugin: pb, .. }` silently overwriting
/// the first in the registry, so pa's real ownership would be lost with no
/// error logged.
#[test]
fn lazy_stub_collision_lazy_vs_lazy_first_writer_wins() {
    let dir = safe_tempdir();
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
        let mut ih = init_host!(ed);
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
