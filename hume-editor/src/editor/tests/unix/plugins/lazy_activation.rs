//! Lazy plugin loading: editor-level tests.

use super::*;
use crate::editor::event::EditorEvent;
use crate::editor::registry::{MappableCommand, TypedBody};
use hume_platform::dirs::Dirs;
use hume_scripting::PluginStatus;

/// After `eval_init`, a `Lazy` stub is present for the declared command name:
/// `declare-plugin!` registers it directly via `CommandHost::register_lazy_commands`
/// as the manifest is processed, with no separate post-init pass.
#[test]
fn lazy_stub_present_after_init() {
    let (ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:commands '("bar"))"#,
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
#[test]
fn first_dispatch_activates_plugin_and_runs() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:typed-commands '("bar"))"#,
        r#"(define-typed-command! "bar" "doc" (lambda () (call! "move-right" (focused-pane))))"#,
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
            Some(TypedBody::Steel(_))
        ),
        "stub must be replaced by a Steel typed command after first dispatch"
    );
}

/// `register_lazy_commands` hardcodes `completer: None` on a typed stub;
/// `#:complete` only reaches the registry once the plugin body actually
/// runs `define-typed-command!`. Tab on the command's *first* use must
/// still resolve it: `trigger_minibuf_completion`'s `Editor`-level callers
/// activate the owning plugin before resolving, same as dispatch already
/// does on Enter.
#[test]
fn a_lazily_activated_typed_commands_declared_completer_works_on_first_use() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:typed-commands '("myfetch"))"#,
        r#"(define-typed-command! "myfetch" "doc" (lambda (bid arg) (log! 'info arg)) #:complete "path")"#,
    );
    let files_dir = tempfile::tempdir().unwrap();
    std::fs::write(files_dir.path().join("hello.txt"), b"").unwrap();

    ed.handle_key(key(':'));
    for ch in format!("myfetch {}/hel", files_dir.path().display()).chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_tab());

    assert_eq!(
        ed.state
            .minibuf()
            .map(|mb| mb.input.clone())
            .unwrap_or_default(),
        format!("myfetch {}/hello.txt", files_dir.path().display()),
        "the lazy stub's real #:complete completer must resolve on the \
         command's very first use, not only after it has already run once"
    );
}

/// Tab after a differently cased name of a lazy typed command activates its
/// plugin too, so the declared completer works on first use.
#[test]
fn a_lazy_typed_commands_completer_works_under_a_differently_cased_name() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:typed-commands '("myfetch"))"#,
        r#"(define-typed-command! "myfetch" "doc" (lambda (bid arg) (log! 'info arg)) #:complete "path")"#,
    );
    let files_dir = tempfile::tempdir().unwrap();
    std::fs::write(files_dir.path().join("hello.txt"), b"").unwrap();

    ed.handle_key(key(':'));
    for ch in format!("MYFETCH {}/hel", files_dir.path().display()).chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_tab());

    assert_eq!(
        ed.state
            .minibuf()
            .map(|mb| mb.input.clone())
            .unwrap_or_default(),
        format!("MYFETCH {}/hello.txt", files_dir.path().display()),
        "`MYFETCH`'s Tab must activate `myfetch`'s plugin and use its completer"
    );
}

/// `call!` resolves a name inside the VM, where only mappable commands are
/// reachable, so a typed-only lazy stub must not be activatable through
/// `call!`: activating a plugin for a lookup that will miss again right after
/// is a permanent side effect for a call that could never succeed.
#[test]
fn call_bang_does_not_activate_a_typed_only_lazy_stub() {
    use hume_scripting::attribution::PluginId;

    let (ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:typed-commands '("bar"))
           (with-handler (lambda (err) (log! 'info (to-string err))) (call! "bar"))"#,
        r#"(define-typed-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );

    let tp_id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(tp_id.clone())),
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

/// A lazy typed stub reached through a differently cased name activates its
/// plugin and runs the command the plugin defined under the declared name.
#[test]
fn first_dispatch_under_a_differently_cased_name_activates_and_runs() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:typed-commands '("bar"))"#,
        r#"(define-typed-command! "bar" "doc" (lambda () (call! "move-right" (focused-pane))))"#,
    );
    let before = state(&ed);

    type_cmd(&mut ed, ":BAR");

    assert_ne!(state(&ed), before, "`:BAR` must run `bar`'s body");
    assert!(
        matches!(
            ed.state.config.registry.get_typed("bar").map(|tc| &tc.body),
            Some(TypedBody::Steel(_))
        ),
        "the stub must be replaced by the plugin's Steel typed command"
    );
}

/// A drawer render proc that `call!`s a declared plugin's command is refused
/// the activation: the render fails, and the plugin stays `Declared` and
/// still activates through a normal dispatch.
#[test]
fn a_drawer_render_cannot_activate_a_plugin_and_leaves_it_declared() {
    use hume_scripting::attribution::{EntryId, PluginId};

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:commands '("fmt-row"))
           (define-typed-command! "go" "" (lambda (pane)
             (show-drawer-list! pane (list 1 2 3) (lambda (i tok) (begin))
               #:render (lambda (start keys) (map (lambda (k) (call! "fmt-row")) keys)))))"#,
        r#"(define-command! "fmt-row" "" (lambda () "row"))"#,
    );
    let tp = EntryId::main(PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    });

    type_cmd(&mut ed, ":go");
    frame(&mut ed, 40, 12);

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&tp),
            Some(PluginStatus::Declared)
        ),
        "a refused activation must leave the plugin Declared"
    );
    assert!(
        ed.state.message_log.entries().any(|e| e.text.contains("drawer render error")
            && e.text.contains("plugin activation")),
        "the render must fail on the refused activation; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| e.text.clone())
            .collect::<Vec<_>>()
    );

    ed.execute_keymap_command("fmt-row", Count::typed(1), false);

    assert!(
        matches!(
            ed.state.config.registry.get_mappable("fmt-row"),
            Some(MappableCommand::SteelBacked(_))
        ),
        "a normal dispatch must still activate the plugin"
    );
}

/// A `#:typed-commands` stub names its plugin before activation. The real
/// registry tells the two stub kinds apart, unlike `MockHost`, so the test
/// checks `command-plugin` on a typed stub against it.
#[test]
fn command_plugin_names_a_typed_stubs_plugin_before_activation() {
    let (ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:typed-commands '("bar"))
           (log! 'warning (command-plugin "bar"))"#,
        r#"(define-typed-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );

    let messages = ed.scripting.as_ref().unwrap().peek_pending_messages();
    assert!(
        messages.iter().any(|(_, msg)| msg == "user/tp"),
        "command-plugin must name the typed stub's plugin; logged: {messages:?}"
    );
}

/// A declared command the plugin body never defines is gone once the first
/// dispatch activates the plugin, so a later dispatch does not retry it.
#[test]
fn activation_drops_a_stub_the_body_never_defined() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:typed-commands '("bar"))"#,
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

    assert!(
        ed.state.config.registry.get_typed("bar").is_none(),
        "activation must drop the stub the body never defined"
    );
}

/// `:` on a declared command the plugin body never defines warns, as `:` on
/// any other unknown name does.
#[test]
fn a_declared_command_the_body_never_defined_warns() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:typed-commands '("bar"))"#,
        r#"(define-command! "other-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let e = ed
        .execute_typed("bar", None)
        .expect_err("`:bar` is unknown");

    assert_eq!(e.severity(), hume_core::Severity::Warning);
}

/// A lazy plugin body that logs and then raises: its log line is reported
/// before its activation error, during the same dispatch.
#[test]
fn a_failing_activations_log_lands_before_its_error() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:typed-commands '("bar"))"#,
        r#"(log! 'warning "step 1") (error "boom")"#,
    );

    ed.execute_typed("bar", None)
        .expect_err("activation raises before defining `:bar`");

    let texts: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .map(|e| e.text.as_str())
        .collect();
    let step = texts.iter().position(|t| t.contains("step 1"));
    let boom = texts.iter().position(|t| t.contains("boom"));
    assert!(
        matches!((step, boom), (Some(s), Some(b)) if s < b),
        "the log line must be reported before the error; log: {texts:?}"
    );
}

/// A lazy command dispatched with no scripting host fails the way an
/// activated Steel command does.
#[test]
fn a_lazy_command_with_no_scripting_host_reports_scripting_unavailable() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:typed-commands '("bar"))"#,
        r#"(define-typed-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );
    ed.scripting = None;

    let e = ed
        .execute_typed("bar", None)
        .expect_err("no host runs `:bar`");

    assert_eq!(
        (e.message(), e.severity()),
        (
            "bar: scripting is not available",
            hume_core::Severity::Error
        )
    );
}

/// A lazy plugin body that queues a `register-lsp-server!` and then errors
/// before defining its activation command: the queued registration must not
/// survive the failed activation: it must not be picked up by
/// `apply_script_effects`'s own inline application, nor leak into some later
/// unrelated drain.
///
/// The rollback in `SteelCtx::pop_effect_marks` is what keeps
/// `registered_command_for_test("rust-analyzer")` at `None`.
#[test]
fn failed_activation_does_not_leave_a_queued_lsp_registration() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:typed-commands '("bar"))"#,
        r#"(register-lsp-server! "rust-analyzer" #:command "rust-analyzer")
           (error "intentional mid-body error")"#,
    );

    type_cmd(&mut ed, ":bar");

    assert!(
        ed.state
            .lsp
            .registered_command_for_test("rust-analyzer")
            .is_none(),
        "a failed activation must not leave its queued register-lsp-server! applied"
    );
}

/// A lazy plugin body that calls `%define-language!` (the builtin behind
/// `define-language!`) is applied in the very same activation call:
/// `apply_script_effects` drains `pending_language_regs` at runtime, not
/// only at the `eval_init` boundary.
#[test]
fn lazy_plugin_defined_language_is_registered_on_activation() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:typed-commands '("bar"))"#,
        r#"(%define-language! "foo" '() '() '())
           (define-typed-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );

    assert!(
        ed.state.config.languages.defined_id("foo").is_none(),
        "precondition: 'foo' must not be registered before activation"
    );

    type_cmd(&mut ed, ":bar");

    assert!(
        ed.state.config.languages.defined_id("foo").is_some(),
        "a define-language! from a lazily-activated plugin body must be applied \
         in the same activation call, not stranded until :reload-config"
    );
}

/// A lazily-activated `#:languages` plugin body that itself calls
/// `set-buffer-option!`'s `"language"` on the very buffer whose language-set triggered
/// its own activation: the nested call (applied inline, before activation
/// returns) must win. The outer `set_buffer_language` call must detect the
/// buffer no longer holds the value it's about to fire `OnLanguageSet` for,
/// and bail out rather than enqueue a second, stale hook.
#[test]
fn set_buffer_language_reentrant_activation_uses_final_value() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:languages '("rust"))"#,
        r#"(set-buffer-option! (car (buffers)) "language" "python")"#,
    );
    let bid = ed.focused_buffer_id();

    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));

    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("python"),
        "the plugin's own language set-buffer-option! call inside its activation body must win"
    );
    let queued_events: Vec<&EditorEvent> = ed
        .state
        .config
        .pending_work
        .iter()
        .filter_map(|w| match w {
            crate::editor::event::PendingWork::Event(e) => Some(e),
            crate::editor::event::PendingWork::Call { .. } => None,
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
#[test]
fn body_error_removes_stub_and_marks_failed() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:typed-commands '("bar"))"#,
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
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
            Some(PluginStatus::Failed)
        ),
        "plugin must be Failed after body error"
    );
}

/// `:bar arg` on a lazy command: the arg is correctly passed to a
/// `(bid arg)`-arity command on first call (after activation).
#[test]
fn lazy_cmd_arg_passed_on_first_call() {
    use hume_scripting::attribution::PluginId;

    // The plugin defines a `(bid x)`-arity "bar" that does nothing visible;
    // we just verify that after activation the command is SteelBacked (i.e.
    // arg was accepted, no arity error), and the plugin is Loaded.
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:typed-commands '("bar"))"#,
        r#"(define-typed-command! "bar" "doc" (lambda (bid x) (+ 1 0)))"#,
    );

    type_cmd(&mut ed, ":bar hello");

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
            Some(PluginStatus::Loaded)
        ),
        "plugin must be Loaded after first dispatch with arg"
    );
    assert!(
        matches!(
            ed.state.config.registry.get_typed("bar").map(|tc| &tc.body),
            Some(TypedBody::Steel(_))
        ),
        "stub must be replaced by a Steel typed command after first dispatch with arg"
    );
}

/// A key bound to a lazy command name activates the plugin on first press,
/// exercising the `execute_keymap_command` Lazy arm, the path the
/// implementation claims keys use "for free".
#[test]
fn key_press_activates_lazy_plugin_via_keymap() {
    use hume_core::BindMode;
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:commands '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (call! "move-right" (focused-pane))))"#,
    );
    // setup_lazy_editor passes a throwaway Keymap to eval_init; bind here so
    // the key lands in the editor's actual keymap.
    ed.state.config.keymap.bind(
        BindMode::Normal,
        &[key('z')],
        "bar".into(),
        false,
        hume_core::ConfigOrigin::User,
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
/// plugin declares `#:commands '("foo")`. The declaration raises with
/// `define-command!`'s message, the eager SteelBacked command survives, and
/// no `Lazy` stub is ever registered for "foo".
#[test]
fn lazy_stub_rejected_when_name_taken_by_eager_plugin() {
    let dir = tempfile::tempdir().unwrap();
    // Eager plugin: loaded inline (no activation entries), defines "foo".
    let eager_dir = dir.path().join("plugins").join("user").join("eager");
    std::fs::create_dir_all(&eager_dir).unwrap();
    std::fs::write(
        eager_dir.join("plugin.scm"),
        r#"(define-command! "foo" "doc" (lambda () (+ 1 0)))"#,
    )
    .unwrap();
    // Lazy plugin: declares "foo" as its sole activation command, which
    // conflicts with the eager plugin.  The declare hard-errors at init time.
    let lazy_dir = dir.path().join("plugins").join("user").join("lz");
    std::fs::create_dir_all(&lazy_dir).unwrap();
    std::fs::write(lazy_dir.join("plugin.scm"), r#"(+ 1 0)"#).unwrap();

    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(load-plugin! \"user/eager\")\n(declare-plugin! \"user/lz\" #:commands '(\"foo\"))",
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs {
        data: Some(dir.path().to_path_buf()),
        ..Dirs::none()
    });
    // Mirror real init_scripting order: eager command is defined before
    // declare-plugin! runs, so the filter loop rejects "foo".
    let init_err = {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih)
    };
    let err =
        init_err.expect_err("eval_init must fail: declare-plugin! rejects 'foo' at declare time");
    assert!(
        err.message.contains("command 'foo' is already defined by"),
        "error must name the collision; got: {err:?}"
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

/// Two plugins both declare `#:commands '("bar")`: the second declaration
/// raises, so the first plugin keeps "bar" and the second gets no entry and
/// no stub, "pb-only" included.
///
/// Runs against a real `Editor` + `EditorHostImpl`, not a hand-rolled
/// `MockHost`: collision detection is `CommandRegistry`'s decision, and a
/// `MockHost` copy of the same rules could drift from the real behavior it's
/// meant to prove.
#[test]
fn lazy_stub_collision_lazy_vs_lazy_raises_for_the_second() {
    let dir = tempfile::tempdir().unwrap();
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
        "(declare-plugin! \"user/pa\" #:commands '(\"bar\"))\n\
         (declare-plugin! \"user/pb\" #:commands '(\"bar\" \"pb-only\"))",
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs {
        data: Some(dir.path().to_path_buf()),
        ..Dirs::none()
    });
    let err = {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih)
    }
    .expect_err("the second claim of 'bar' raises");
    assert!(
        err.message.contains(
            "command 'bar' is already claimed as an activation command by lazy plugin 'user/pa'"
        ),
        "got: {err:?}"
    );
    ed.scripting = Some(host);

    use hume_scripting::attribution::{EntryId, PluginId};
    let pa = EntryId::main(PluginId::User {
        user: "user".to_string(),
        repo: "pa".to_string(),
    });
    let pb = EntryId::main(PluginId::User {
        user: "user".to_string(),
        repo: "pb".to_string(),
    });
    assert!(
        matches!(
            ed.state.config.registry.get_mappable("bar"),
            Some(MappableCommand::Lazy { plugin, .. }) if *plugin == pa
        ),
        "bar's Lazy stub must stay owned by pa"
    );
    assert!(ed.state.config.registry.get_mappable("pb-only").is_none());
    let scripting = ed.scripting.as_ref().unwrap();
    assert!(matches!(
        scripting.plugin_status(&pa),
        Some(PluginStatus::Declared)
    ));
    assert!(scripting.plugin_status(&pb).is_none());
}

/// A plugin body defines its activation command as the kind it declared.
#[test]
fn an_activation_cannot_define_its_command_as_the_other_kind() {
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:typed-commands '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () #t))"#,
    );

    ed.execute_typed("bar", None)
        .expect_err("the activation raises");
    let errors = error_log(&ed);
    assert!(
        errors.iter().any(|e| e
            .contains("command 'bar' is declared by lazy plugin 'user/tp' in #:typed-commands")),
        "got: {errors:?}"
    );
}
