use rustc_hash::FxHashSet;
use std::io::Write as _;

use tempfile::TempDir;

use crate::ScriptingHost;
use crate::attribution::PluginId;
use crate::lazy::PluginState;
use crate::null_host::NullHost;

/// Write a Steel source file into `dir` and return its path.
fn write_plugin(dir: &TempDir, name: &str, src: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(src.as_bytes()).unwrap();
    path
}

fn plugin_id(name: &str) -> PluginId {
    PluginId::parse(name).unwrap()
}

fn no_builtins() -> FxHashSet<String> {
    FxHashSet::default()
}

// ── Case 1: Declared → Loaded with a valid command body ──────────────────

#[test]
fn declared_to_loaded_registers_command() {
    let dir = TempDir::new().unwrap();
    let path = write_plugin(
        &dir,
        "plugin.scm",
        r#"(define-command! "test-cmd" "A test command." (lambda () 0))"#,
    );
    let id = plugin_id("core:test");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Declared { path });

    host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins())
        .unwrap();

    assert!(
        host.registries.command_table.contains_key("test-cmd"),
        "command must be in command_table after activation"
    );
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Loaded)
        ),
        "plugin must be in Loaded state after successful activation"
    );
}

// ── Case 2: Syntax error → Failed, Err returned ──────────────────────────

#[test]
fn syntax_error_transitions_to_failed() {
    let dir = TempDir::new().unwrap();
    let path = write_plugin(&dir, "bad.scm", "(((invalid syntax");
    let id = plugin_id("core:bad");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Declared { path });

    let result = host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins());

    assert!(
        result.is_ok(),
        "a failed plugin activation must be contained, not propagate; got: {result:?}"
    );
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Failed)
        ),
        "plugin must be in Failed state after syntax error"
    );
    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(level, msg)| {
            matches!(level, crate::log::LogLevel::Error)
                && msg.contains("core:bad")
                && msg.contains("failed to load")
        }),
        "must log an Error naming the failed plugin; messages: {messages:?}"
    );
}

/// A failed plugin body's log message names the plugin and points at the
/// exact file, line, and column of the failing reference, not just
/// `init.scm`, the file that happened to `load-plugin!` it.
/// `describe_steel_error` resolves the error's span against the engine's own
/// `Sources`. Without that, the message would read bare
/// `init.scm: Error: FreeIdentifier: …` with no way to tell which plugin,
/// file, or line was at fault.
#[test]
fn failed_activation_message_names_plugin_and_location() {
    let dir = TempDir::new().unwrap();
    // Line 2, column 2 (1-based): right at "call-does-not-exist", after the
    // opening paren. Hand-computed from the source text for the location
    // assertion below.
    let path = write_plugin(&dir, "located.scm", "(define x 1)\n(call-does-not-exist)\n");
    let id = plugin_id("core:located");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Declared { path });

    let result = host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins());
    assert!(
        result.is_ok(),
        "must be contained, not propagate; got: {result:?}"
    );

    let messages = host.peek_pending_messages();
    assert_eq!(
        messages.len(),
        1,
        "exactly one failure message expected; got: {messages:?}"
    );
    let (level, msg) = &messages[0];
    assert!(
        matches!(level, crate::log::LogLevel::Error),
        "must be logged at Error; got: {level:?}"
    );
    assert!(
        msg.contains("core:located"),
        "must name the plugin; got: {msg}"
    );
    assert!(
        msg.contains("located.scm"),
        "must name the failing file, not just init.scm; got: {msg}"
    );
    assert!(
        msg.contains(":2:2"),
        "must point at line 2, column 2, where the bad identifier starts; got: {msg}"
    );
    assert!(
        msg.contains("call-does-not-exist"),
        "must name the bad identifier; got: {msg}"
    );
    assert!(
        !msg.contains("bootstrap.scm") && !msg.contains("meta-continuation"),
        "must not leak bootstrap.scm's own activation-plumbing frames; got: {msg}"
    );
}

// ── Case 3: Idempotent no-ops for non-Declared states ────────────────────

#[test]
fn already_loaded_is_noop() {
    let id = plugin_id("core:loaded");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Loaded);

    host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins())
        .unwrap();

    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Loaded)
        ),
        "state must remain Loaded"
    );
}

#[test]
fn already_failed_is_noop() {
    let id = plugin_id("core:failed");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Failed);

    host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins())
        .unwrap();

    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Failed)
        ),
        "state must remain Failed"
    );
}

#[test]
fn absent_plugin_is_noop() {
    let id = plugin_id("core:absent");
    let mut host = ScriptingHost::new();

    host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins())
        .unwrap();

    assert!(
        !host.registries.lazy_registry.plugins.contains_key(&id),
        "absent plugin must not appear in registry after no-op"
    );
}

// ── Case 4: Loading re-entrancy guard → no-op ────────────────────────────

#[test]
fn loading_reentrancy_guard_is_noop() {
    let id = plugin_id("core:cycling");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Loading);

    host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins())
        .unwrap();

    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Loading)
        ),
        "state must remain Loading (re-entrancy guard must not overwrite)"
    );
}

// ── Case 5: Path containing '"' rejected before any eval ─────────────────

#[test]
fn path_with_quote_char_transitions_to_failed() {
    let id = plugin_id("core:quoted");
    let mut host = ScriptingHost::new();
    host.registries.lazy_registry.plugins.insert(
        id.clone(),
        PluginState::Declared {
            path: std::path::PathBuf::from("/some/path\"with/quote/plugin.scm"),
        },
    );

    let result = host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins());

    assert!(result.is_err(), "path with '\"' must be rejected");
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Failed)
        ),
        "plugin must be Failed after path-with-quote rejection"
    );
}

// ── Stage B: eval-string plumbing ─────────────────────────────────────────

/// A `define-command!` issued via `eval-string` inside a `run_steel` session
/// registers the command in `command_table`, proving that the nested eval-string
/// sees the same `ctx.registries` as the outer eval.
#[test]
fn eval_string_nested_registers_command_in_command_table() {
    let mut host = ScriptingHost::new();
    // Eval a snippet that eval-strings a define-command!. The outer eval is
    // in EvalMode::Init, which allows define-command!.
    let program = r#"
(hm.eval-string "(define-command! \"inner-cmd\" \"doc\" (lambda () 0))")
"#;
    host.eval_source(program, &mut NullHost).unwrap();
    assert!(
        host.registries.command_table.contains_key("inner-cmd"),
        "command defined via eval-string must appear in command_table"
    );
}

/// `%begin-lazy-activation!` on a `Declared` plugin transitions to `Loading`,
/// pushes onto `plugin_stack`, and returns the require-string.
#[test]
fn begin_lazy_activation_declared_returns_require_string() {
    let dir = TempDir::new().unwrap();
    let path = write_plugin(
        &dir,
        "p.scm",
        r#"(define-command! "p-cmd" "doc" (lambda () 0))"#,
    );
    let id = plugin_id("core:p");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Declared { path: path.clone() });

    let program = r#"(define result (%begin-lazy-activation! "core:p"))"#;
    host.eval_source(program, &mut NullHost).unwrap();

    // Plugin must be in Loading state.
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Loading)
        ),
        "Declared plugin must be Loading after %begin-lazy-activation!"
    );
    // plugin_stack must have grown by one: begin pushed the id.
    assert_eq!(
        host.plugin_stack_depth_for_test(),
        1,
        "plugin_stack depth must be 1"
    );
}

/// `%begin-lazy-activation!` on a `Loading` plugin returns `#f` (cycle guard).
#[test]
fn begin_lazy_activation_loading_returns_false() {
    let id = plugin_id("core:cycling");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Loading);

    // The result `#f` means the (when prog ...) in %activate-plugin-inline!
    // does nothing: activation is a no-op.
    let program = r#"
(define result (%begin-lazy-activation! "core:cycling"))
(when result (error "cycle guard must return #f!"))
"#;
    host.eval_source(program, &mut NullHost).unwrap();
    // State must remain Loading.
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Loading)
        ),
        "state must remain Loading (cycle guard)"
    );
}

/// `%finish-lazy-activation!` with `error = #f` (no exception caught)
/// transitions to `Loaded`.
#[test]
fn finish_lazy_activation_success_transitions_to_loaded() {
    let id = plugin_id("core:finishing");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Loading);
    // Seed the stack as begin_lazy_activation would have done.
    host.push_plugin_for_test(id.clone());

    let program = r#"(%finish-lazy-activation! "core:finishing" #f)"#;
    host.eval_source(program, &mut NullHost).unwrap();

    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Loaded)
        ),
        "plugin must be Loaded after successful finish"
    );
    assert_eq!(
        host.plugin_stack_depth_for_test(),
        0,
        "plugin_stack must be empty after finish"
    );
}

/// `%finish-lazy-activation!` with `error` bound to a caught exception value
/// transitions to `Failed`. `(with-handler (lambda (e) e) (error …))` is the
/// idiomatic way to get that exact value in hand outside an actual
/// `with-handler`-wrapped activation body. Steel hands the raised
/// `SteelErr`'s `into_steelval()` form to the handler lambda, which here
/// just returns it.
#[test]
fn finish_lazy_activation_failure_transitions_to_failed() {
    let id = plugin_id("core:failing");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Loading);
    host.push_plugin_for_test(id.clone());

    let program = r#"
(define err-val (with-handler (lambda (e) e) (error "intentional")))
(%finish-lazy-activation! "core:failing" err-val)
"#;
    host.eval_source(program, &mut NullHost).unwrap();

    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Failed)
        ),
        "plugin must be Failed after failed finish"
    );
}

// ── Partial-define rollback ───────────────────────────────────────────────

/// A plugin body that defines one command and then errors: the plugin
/// transitions to `Failed` and `finish_lazy_activation` rolls back the
/// partial `define-command!`, removing it from `command_table` and
/// `cmd_owners`.  A `Failed` plugin must not leave callable orphan commands.
#[test]
fn partial_define_before_failure_is_rolled_back() {
    let dir = TempDir::new().unwrap();
    let path = write_plugin(
        &dir,
        "partial.scm",
        r#"(define-command! "partial-cmd" "doc" (lambda () 0))
               (error "intentional mid-body error")"#,
    );
    let id = plugin_id("core:partial");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Declared { path });

    let result = host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins());

    assert!(
        result.is_ok(),
        "a failed plugin activation must be contained, not propagate; got: {result:?}"
    );
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Failed)
        ),
        "plugin must be Failed after mid-body error"
    );
    // The partial define must be rolled back: no callable orphan left behind.
    assert!(
        !host.registries.command_table.contains_key("partial-cmd"),
        "partial define-command! must be removed from command_table on failure"
    );
    assert!(
        !host.registries.cmd_owners.contains_key("partial-cmd"),
        "partial define-command! must be removed from cmd_owners on failure"
    );
}

/// A plugin body that queues an LSP server registration and a language
/// registration and then errors: both must be rolled back from the
/// effect log, not left for some later unrelated drain to silently apply.
///
/// Without `SteelCtx::pop_effect_marks` truncating on failure,
/// `effects_for_test()` would come back non-empty.
#[test]
fn queued_effects_before_failure_are_rolled_back() {
    let dir = TempDir::new().unwrap();
    let path = write_plugin(
        &dir,
        "effects.scm",
        r#"(register-lsp-server! "rust" #:command "rust-analyzer")
               (%define-language! "foo" '() '() '() #f)
               (error "intentional mid-body error")"#,
    );
    let id = plugin_id("core:effects");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Declared { path });

    let result = host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins());

    assert!(
        result.is_ok(),
        "a failed plugin activation must be contained, not propagate; got: {result:?}"
    );
    assert!(
        host.effects_for_test().is_empty(),
        "failed activation must not leave a queued LSP server op or language registration behind"
    );
}

// ── Committed-effects salvage across enclosing eval failure ──────────────

/// A command dispatched via `call_steel_cmd` `call!`s a lazy command owned
/// by plugin B; B activates inline mid-body and finishes successfully
/// (queuing `register-lsp-server!` and committing `Loaded`), then the
/// outer command errors afterward. B's committed effect must survive:
/// discarding it while B stays permanently `Loaded` would mean its LSP
/// server never registers (activation is one-shot). Effects the outer
/// command itself queued, before and after the nested activation, must
/// NOT survive.
///
/// A flat `self.effects.truncate(effects_start)` in `take_eval_effects`'s
/// `Err` arm would leave `e.effects` empty even though B is `Loaded`.
#[test]
fn committed_activation_effects_survive_failed_outer_command() {
    use crate::host::EditorHost;
    use crate::null_host::LazyStubHost;
    use crate::types::{Effect, PendingLspServerOp};

    let dir = TempDir::new().unwrap();
    let path = write_plugin(
        &dir,
        "b.scm",
        r#"(register-lsp-server! "b-lang" #:command "b-lsp")
               (define-command! "b-cmd" "doc" (lambda () 0))"#,
    );
    let id_b = plugin_id("core:b");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id_b.clone(), PluginState::Declared { path });

    let mut editor_host = LazyStubHost::default();
    editor_host
        .commands()
        .register_lazy_command("b-cmd", &id_b)
        .expect("stub claim must succeed on a fresh host");

    host.eval_source(
        r#"(define-command! "outer-a" "doc"
                 (lambda ()
                   (register-lsp-server! "before" #:command "x")
                   (call! "b-cmd")
                   (register-lsp-server! "after" #:command "y")
                   (error "intentional outer failure")))"#,
        &mut editor_host,
    )
    .expect("defining outer-a must not error");

    let result = host.call_steel_cmd("outer-a", None, vec![], &mut editor_host);

    let err = result.expect_err("outer-a's intentional error must propagate");
    assert!(
        err.message.contains("intentional outer failure"),
        "got: {}",
        err.message
    );
    assert_eq!(
        err.effects.len(),
        1,
        "only B's committed register-lsp-server! must survive; got: {:?}",
        err.effects
    );
    assert!(
        matches!(
            &err.effects[0],
            Effect::LspServerOp(PendingLspServerOp::Register(reg)) if reg.language == "b-lang"
        ),
        "surviving effect must be B's 'b-lang' registration, not 'before'/'after'; got: {:?}",
        err.effects[0]
    );
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id_b),
            Some(PluginState::Loaded)
        ),
        "B must be Loaded: its activation succeeded before A's own failure"
    );
    assert!(
        host.effects_for_test().is_empty(),
        "the effect log must be fully drained after take_eval_effects"
    );
}

/// One level deeper: plugin C activates successfully inside plugin B's
/// body (via `call!` to a command C owns), and B then fails. C's
/// committed `register-lsp-server!` must survive B's own rollback. C is
/// `Loaded` and its effect is irreversible-by-omission, same reasoning as
/// the outer-command case above, but exercised through nested
/// `pop_effect_marks` calls instead of `take_eval_effects` alone. B's own
/// failure is contained (`activate_plugin_inline` returns `Ok`), so C's
/// effect surfaces through the success path, not a salvaged `EvalError`.
#[test]
fn nested_activation_commit_survives_enclosing_plugin_failure() {
    use crate::host::EditorHost;
    use crate::null_host::LazyStubHost;
    use crate::types::{Effect, PendingLspServerOp};

    let dir = TempDir::new().unwrap();
    let path_c = write_plugin(
        &dir,
        "c.scm",
        r#"(register-lsp-server! "c-lang" #:command "c-lsp")
               (define-command! "c-cmd" "doc" (lambda () 0))"#,
    );
    let path_b = write_plugin(
        &dir,
        "b.scm",
        r#"(register-lsp-server! "b-lang" #:command "b-lsp")
               (call! "c-cmd")
               (error "b fails")"#,
    );
    let id_b = plugin_id("core:b");
    let id_c = plugin_id("core:c");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id_b.clone(), PluginState::Declared { path: path_b });
    host.registries
        .lazy_registry
        .plugins
        .insert(id_c.clone(), PluginState::Declared { path: path_c });

    let mut editor_host = LazyStubHost::default();
    editor_host
        .commands()
        .register_lazy_command("c-cmd", &id_c)
        .expect("stub claim must succeed on a fresh host");

    let result = host.activate_plugin_inline(&id_b, 10_000, &mut editor_host, &no_builtins());

    let effects = result.expect("B's failure must be contained, not propagate");
    assert_eq!(
        effects.len(),
        1,
        "only C's committed register-lsp-server! must survive; got: {effects:?}"
    );
    assert!(
        matches!(
            &effects[0],
            Effect::LspServerOp(PendingLspServerOp::Register(reg)) if reg.language == "c-lang"
        ),
        "surviving effect must be C's 'c-lang' registration, not B's 'b-lang'; got: {:?}",
        effects[0]
    );
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id_b),
            Some(PluginState::Failed)
        ),
        "B must be Failed"
    );
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id_c),
            Some(PluginState::Loaded)
        ),
        "C must be Loaded: its activation succeeded before B's own failure"
    );
    assert!(
        host.registries.command_table.contains_key("c-cmd"),
        "C's command must remain registered: C is Loaded, not rolled back"
    );
    assert!(
        host.effects_for_test().is_empty(),
        "the effect log must be fully drained after take_eval_effects"
    );
    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(level, msg)| {
            matches!(level, crate::log::LogLevel::Error)
                && msg.contains("core:b")
                && msg.contains("b fails")
        }),
        "must log an Error naming B and its failure; messages: {messages:?}"
    );
}

// ── Hook rollback on activation failure ───────────────────────────────────

/// A plugin body that registers a hook and then errors: the hook must not
/// survive: a `Failed` plugin's hooks must stop firing.
///
/// `finish_lazy_activation` drops the hook through
/// `HookRegistry::remove_owned_by`.
#[test]
fn hook_registered_before_failure_is_rolled_back() {
    let dir = TempDir::new().unwrap();
    let path = write_plugin(
        &dir,
        "hook_fail.scm",
        r#"(register-hook! 'on-buffer-save (lambda (bid) 0))
               (error "intentional mid-body error")"#,
    );
    let id = plugin_id("core:hookfail");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Declared { path });

    let result = host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins());

    assert!(
        result.is_ok(),
        "a failed plugin activation must be contained, not propagate; got: {result:?}"
    );
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Failed)
        ),
        "plugin must be Failed after mid-body error"
    );
    assert!(
        !host.has_hook_handlers("on-buffer-save", None),
        "a Failed plugin's hook must not survive rollback"
    );
}

/// One level deeper: plugin C registers a hook and activates successfully
/// inside plugin B's body, and B then fails afterward. C's hook must
/// survive B's rollback. Rollback is by owner identity, not by eval-scoped
/// position, so B's failure can never touch C's entries.
#[test]
fn nested_activation_hook_survives_enclosing_plugin_failure() {
    use crate::host::EditorHost;
    use crate::null_host::LazyStubHost;

    let dir = TempDir::new().unwrap();
    let path_c = write_plugin(
        &dir,
        "hook_c.scm",
        r#"(register-hook! 'on-buffer-save (lambda (bid) 0))
               (define-command! "hc-cmd" "doc" (lambda () 0))"#,
    );
    let path_b = write_plugin(
        &dir,
        "hook_b.scm",
        r#"(call! "hc-cmd")
               (error "b fails")"#,
    );
    let id_b = plugin_id("core:hb");
    let id_c = plugin_id("core:hc");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id_b.clone(), PluginState::Declared { path: path_b });
    host.registries
        .lazy_registry
        .plugins
        .insert(id_c.clone(), PluginState::Declared { path: path_c });

    let mut editor_host = LazyStubHost::default();
    editor_host
        .commands()
        .register_lazy_command("hc-cmd", &id_c)
        .expect("stub claim must succeed on a fresh host");

    let result = host.activate_plugin_inline(&id_b, 10_000, &mut editor_host, &no_builtins());

    assert!(
        result.is_ok(),
        "a failed plugin activation must be contained, not propagate; got: {result:?}"
    );
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id_b),
            Some(PluginState::Failed)
        ),
        "B must be Failed"
    );
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id_c),
            Some(PluginState::Loaded)
        ),
        "C must be Loaded: its activation succeeded before B's own failure"
    );
    assert!(
        host.has_hook_handlers("on-buffer-save", None),
        "C's hook must survive B's failure: rollback is scoped to B's own id"
    );
}

// ── Self-ownership exemption ────────────────────────────────────────────────

/// A lazy plugin is allowed to call `define-command!` for its own activation
/// command inside its body, even though that name is still claimed as its
/// `Lazy` stub at the time (the stub is only removed by
/// `unregister_lazy_stubs_of` *after* the body completes in
/// `finish_lazy_activation`).
///
/// The `is_self` exemption in `define_command` allows this. Without it the
/// call would be rejected and activation would return Err.
#[test]
fn lazy_plugin_can_define_its_own_activation_command() {
    use crate::host::EditorHost;
    use crate::null_host::LazyStubHost;

    let dir = TempDir::new().unwrap();
    let path = write_plugin(
        &dir,
        "self-act.scm",
        r#"(define-command! "self-act-cmd" "doc" (lambda () 0))"#,
    );
    let id = plugin_id("core:self-act");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Declared { path });

    // Simulate declare-plugin! having claimed self-act-cmd as the
    // activation entry, now tracked in the editor's registry (here,
    // the stateful test host), not a scripting-crate map.
    let mut editor_host = LazyStubHost::default();
    editor_host
        .commands()
        .register_lazy_command("self-act-cmd", &id)
        .expect("stub claim must succeed on a fresh host");

    let result = host.activate_plugin_inline(&id, 10_000, &mut editor_host, &no_builtins());

    assert!(
        result.is_ok(),
        "plugin defining its own activation command must activate successfully; got: {result:?}"
    );
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Loaded)
        ),
        "plugin must be Loaded after activation"
    );
    assert!(
        host.registries.command_table.contains_key("self-act-cmd"),
        "self-act-cmd must be in command_table after activation"
    );
}

// ── Interrupt during activation aborts the enclosing eval ─────────────────

/// A's body raises via `(hume/yield!)` while the interrupt flag is already
/// set (a real budget exhaustion sets this flag the same way; see
/// `EvalWatchdog`): A's own activation is contained exactly as any other
/// body error (Failed, rolled back), but the interrupt itself must not be:
/// it must abort the whole eval before B ever loads. Without that,
/// `%dispatch-command!`'s later `(load-plugin! "core:b")` would run past an
/// exhausted budget, and B would falsely be blamed as "failed to load" for
/// hitting the same still-set flag on its own first `(hume/yield!)`.
///
/// The abort comes from the `(hume/yield!)` re-check that
/// `%activate-plugin-inline!` runs after its `with-handler`.
#[test]
fn interrupt_during_activation_aborts_before_next_plugin_loads() {
    use crate::null_host::LazyStubHost;
    use std::sync::atomic::Ordering;

    let dir = TempDir::new().unwrap();
    let path_a = write_plugin(&dir, "a.scm", "(hume/yield!)");
    let path_b = write_plugin(
        &dir,
        "b.scm",
        r#"(define-command! "b-cmd" "doc" (lambda () 0))"#,
    );
    let id_a = plugin_id("core:a");
    let id_b = plugin_id("core:b");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id_a.clone(), PluginState::Declared { path: path_a });
    host.registries
        .lazy_registry
        .plugins
        .insert(id_b.clone(), PluginState::Declared { path: path_b });

    host.interrupt_flag_for_test()
        .store(true, Ordering::Relaxed);

    let src = r#"(load-plugin! "core:a") (load-plugin! "core:b")"#;
    let result = host.eval_source(src, &mut LazyStubHost::default());

    assert!(
        result.is_err(),
        "an interrupt mid-activation must abort the whole eval; got: {result:?}"
    );
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id_a),
            Some(PluginState::Failed)
        ),
        "A's own activation is still contained and Failed, not silently swallowed"
    );
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id_b),
            Some(PluginState::Declared { .. })
        ),
        "B must never have been reached once the abort fired"
    );
}

// ── `call!` of a command owned by a Failed plugin ──────────────────────────

/// A lazy plugin's command is `call!`ed (e.g. from another plugin's body);
/// activation runs inline, fails, and is contained. `%dispatch-command!`
/// must not then fall through to `%call-native!` and silently return `#f`
/// as if the name were simply unknown. The plugin failed, and the caller
/// needs to know that, not receive a value indistinguishable from success.
///
/// The `%lazy-command-owner` check after a failed inline activation is what
/// turns this into an error. Without it the call would return `Ok` and log
/// a native-command miss.
#[test]
fn call_of_command_owned_by_newly_failed_plugin_errors() {
    use crate::host::EditorHost;
    use crate::null_host::LazyStubHost;

    let dir = TempDir::new().unwrap();
    let path = write_plugin(&dir, "broken.scm", r#"(error "broken body")"#);
    let id = plugin_id("core:broken");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Declared { path });

    let mut editor_host = LazyStubHost::default();
    editor_host
        .commands()
        .register_lazy_command("broken-cmd", &id)
        .expect("stub claim must succeed on a fresh host");

    let err = host
        .eval_source(r#"(call! "broken-cmd")"#, &mut editor_host)
        .expect_err("call! of a command whose owner just failed to load must error");

    assert!(
        err.contains("core:broken") && err.contains("broken-cmd"),
        "error must name both the failed plugin and the command that couldn't be reached; got: {err}"
    );
}

// ── Manifest-declare rollback on a later error ─────────────────────────────

/// `manifest.scm` successfully declares itself with `#:commands` (a real,
/// direct `%declare-plugin!` call, not the zero-trigger path) and then a
/// later top-level form in the same file raises. The plugin must end up
/// `Failed` with no live command stub, not left half-`Declared` with a
/// callable stub for a plugin whose manifest never finished evaluating.
///
/// `finish_manifest_declare`'s error branch performs that rollback to
/// `Failed`.
#[test]
fn manifest_declare_self_declared_then_failed_rolls_back_to_failed() {
    use crate::host::EditorHost;
    use crate::null_host::LazyStubHost;

    let dir = TempDir::new().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("selfdecl");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    // The full (non-zero-trigger) `declare-plugin!` inside manifest.scm only
    // registers `Declared` state once `plugin.scm` resolves on disk. Absent,
    // it soft-logs and no-ops, which would make this test pass for the wrong
    // reason (never-declared, not rolled-back-after-declared).
    std::fs::write(plugin_dir.join("plugin.scm"), b"").unwrap();
    std::fs::write(
        plugin_dir.join("manifest.scm"),
        br#"(declare-plugin! "user/selfdecl" #:commands '("selfdecl-cmd"))
            (error "manifest body fails after self-declare")"#,
    )
    .unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    let mut editor_host = LazyStubHost::default();

    let result = host.eval_source(r#"(declare-plugin! "user/selfdecl")"#, &mut editor_host);

    assert!(
        result.is_ok(),
        "a failed manifest resolution must be contained, not propagate; got: {result:?}"
    );

    let id = plugin_id("user/selfdecl");
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Failed)
        ),
        "the plugin's own self-declare must be rolled back to Failed, not left Declared; got: {:?}",
        host.registries.lazy_registry.plugins.get(&id)
    );
    assert!(
        editor_host
            .commands()
            .lazy_command_owner("selfdecl-cmd")
            .is_none(),
        "the self-declared command stub must not survive the manifest's own later failure"
    );
}

// ── `%finish-lazy-activation!` balances stack/marks even on a bad `error` ──

/// `(%finish-lazy-activation! id garbage)` where `garbage` decodes as
/// neither `#f` nor a caught error value: the plugin stack and the
/// activation-effect marks must still be popped and balanced: a decode
/// failure must become the failure *reason*, not skip the bookkeeping that
/// every other failure path performs. `%begin-lazy-activation!` and a queued
/// `register-lsp-server!` run first in the same eval, since
/// `activation_effect_marks` is per-eval transient state (rebuilt fresh in
/// every `SteelCtx`), only observable by whether its own effect survives.
///
/// Decoding `error` must not short-circuit ahead of `plugin_stack.pop()` and
/// `pop_effect_marks`. If it did, the stack would stay at depth 1 and the
/// queued `register-lsp-server!` effect would never be rolled back.
#[test]
fn finish_lazy_activation_bad_error_value_still_balances_stack_and_marks() {
    let dir = TempDir::new().unwrap();
    let path = write_plugin(&dir, "badvalue.scm", "(define x 1)");
    let id = plugin_id("core:badvalue");
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Declared { path });

    let program = r#"
        (%begin-lazy-activation! "core:badvalue")
        (register-lsp-server! "badvalue-lang" #:command "bv")
        (%finish-lazy-activation! "core:badvalue" 42)
    "#;
    host.eval_source(program, &mut NullHost)
        .expect("a bad error value must be folded into the failure, not raised");

    assert_eq!(
        host.plugin_stack_depth_for_test(),
        0,
        "plugin_stack must be popped even when the error value fails to decode"
    );
    assert!(
        host.effects_for_test().is_empty(),
        "the queued register-lsp-server! must be rolled back: the activation-effect \
         mark must be popped even when the error value fails to decode"
    );
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Failed)
        ),
        "an undecodable error value must still fail the plugin, not leave it Loading"
    );
}
