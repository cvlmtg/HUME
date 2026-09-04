use super::*;

/// Eval `src` against a fresh host and return the error it must produce.
fn declare_err(src: &str) -> String {
    use crate::ScriptingHost;
    use crate::null_host::LazyStubHost;
    let mut host = ScriptingHost::new();
    let mut editor_host = LazyStubHost::default();
    host.eval_source(src, &mut editor_host)
        .expect_err("expected declare-plugin to error")
}

#[test]
fn parse_core_plugin_name() {
    let id = PluginId::parse("core:helix-surround").unwrap();
    assert!(matches!(id, PluginId::Core(n) if n == "helix-surround"));
}

#[test]
fn parse_user_plugin_name() {
    let id = PluginId::parse("user/repo").unwrap();
    assert!(
        matches!(id, PluginId::User { ref user, ref repo } if user == "user" && repo == "repo")
    );
}

#[test]
fn parse_invalid_names() {
    assert!(PluginId::parse("bad").is_err());
    assert!(PluginId::parse("core:").is_err());
    assert!(PluginId::parse("a/b/c").is_err());
    assert!(PluginId::parse("/repo").is_err());
    assert!(PluginId::parse("user/").is_err());
    assert!(PluginId::parse("core:..").is_err());
    assert!(PluginId::parse("../evil").is_err());
}

#[test]
fn valid_core_segment() {
    // Segments that should pass through PluginId::parse successfully.
    assert!(PluginId::parse("core:helix-surround").is_ok());
    assert!(PluginId::parse("core:plum").is_ok());
    assert!(PluginId::parse("core:v1.2.3").is_ok());
}

#[test]
fn invalid_segments() {
    // Segment validation exercised via PluginId::parse.
    assert!(PluginId::parse("core:").is_err()); // empty
    assert!(PluginId::parse("core:.").is_err()); // dot
    assert!(PluginId::parse("core:..").is_err()); // dotdot
    assert!(PluginId::parse("./b").is_err()); // slash without user
    assert!(PluginId::parse("a\0b/repo").is_err()); // NUL in user
}

// ── Activation depth cap ──────────────────────────────────────────────────

/// `%begin-lazy-activation` refuses to start when `plugin_stack` depth is at
/// `MAX_ACTIVATION_DEPTH`, marks the plugin `Failed`, and returns a Steel error.
///
/// Fail oracle: remove the depth-cap check from `begin_lazy_activation` →
/// an infinite cycle would stack-overflow instead of hard-erroring.
#[test]
fn begin_lazy_activation_at_depth_cap_errors_and_marks_failed() {
    use crate::{ScriptingHost, null_host::NullHost};
    use std::io::Write as _;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("deep.scm");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(b"(define x 1)")
        .unwrap();

    let id = PluginId::parse("core:deep").unwrap();
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Declared { path });
    // Simulate maximum nesting depth already reached by seeding the stack.
    let dummy = PluginId::parse("core:dummy").unwrap();
    for _ in 0..MAX_ACTIVATION_DEPTH {
        host.push_plugin_for_test(dummy.clone());
    }

    let result = host.eval_source(r#"(%begin-lazy-activation "core:deep")"#, &mut NullHost);

    assert!(
        result.is_err(),
        "depth cap must raise a Steel error; got Ok"
    );
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Failed)
        ),
        "plugin must be marked Failed when depth cap exceeded"
    );
}

/// `%begin-lazy-activation` at depth cap − 1 succeeds (cap is exclusive).
///
/// Confirms the off-by-one is correct: depth 15 of 16 is still allowed.
#[test]
fn begin_lazy_activation_below_depth_cap_succeeds() {
    use crate::{ScriptingHost, null_host::NullHost};
    use std::io::Write as _;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("ok.scm");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(b"(define x 1)")
        .unwrap();

    let id = PluginId::parse("core:ok").unwrap();
    let mut host = ScriptingHost::new();
    host.registries
        .lazy_registry
        .plugins
        .insert(id.clone(), PluginState::Declared { path });
    // One below the cap — must still be allowed.
    let dummy = PluginId::parse("core:dummy").unwrap();
    for _ in 0..MAX_ACTIVATION_DEPTH - 1 {
        host.push_plugin_for_test(dummy.clone());
    }

    // Transition to Loading and return the require-string (not an error).
    let result = host.eval_source(r#"(%begin-lazy-activation "core:ok")"#, &mut NullHost);
    assert!(result.is_ok(), "depth below cap must be allowed; got Err");
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Loading)
        ),
        "plugin must be Loading after successful %begin-lazy-activation"
    );
}

/// `%begin-lazy-activation` failing at the depth cap must clean up exactly
/// like `%finish-lazy-activation`'s failure branch does — dropping the
/// plugin's activation-event/language entries and its `Lazy` command stub —
/// even though the body never ran and `%finish-lazy-activation` never fires
/// for it.
///
/// Fail oracle: revert the depth-cap branch in `begin_lazy_activation` to a
/// bare `plugins.insert(id, Failed)` (dropping the `fail_plugin_activation`
/// call) → the event/language entries and the stub survive the failure and
/// re-trigger a no-op activation attempt on every later matching event.
#[test]
fn begin_lazy_activation_depth_cap_cleans_up_activation_entries_and_stub() {
    use crate::ScriptingHost;
    use crate::host::EditorHost;
    use crate::null_host::LazyStubHost;
    use std::io::Write as _;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("deep.scm");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(b"(define x 1)")
        .unwrap();

    let id = PluginId::parse("core:deep").unwrap();
    let mut host = ScriptingHost::new();
    host.registries.lazy_registry.declare(
        id.clone(),
        Some(path),
        vec!["on-buffer-save".to_string()],
        vec!["rust".to_string()],
    );

    let mut editor_host = LazyStubHost::default();
    editor_host
        .commands()
        .register_lazy_command("deep-cmd", &id)
        .expect("stub claim must succeed on a fresh host");

    let dummy = PluginId::parse("core:dummy").unwrap();
    for _ in 0..MAX_ACTIVATION_DEPTH {
        host.push_plugin_for_test(dummy.clone());
    }

    let result = host.eval_source(r#"(%begin-lazy-activation "core:deep")"#, &mut editor_host);
    assert!(result.is_err(), "depth cap must raise; got Ok");

    assert!(
        host.registries
            .lazy_registry
            .activation_events
            .get("on-buffer-save")
            .map(|plugins| !plugins.contains(&id))
            .unwrap_or(true),
        "Failed plugin's event-activation entry must be dropped, not leaked"
    );
    assert!(
        host.registries
            .lazy_registry
            .activation_languages
            .get("rust")
            .map(|plugins| !plugins.contains(&id))
            .unwrap_or(true),
        "Failed plugin's language-activation entry must be dropped, not leaked"
    );
    assert!(
        editor_host
            .commands()
            .lazy_command_owner("deep-cmd")
            .is_none(),
        "Failed plugin's dead command stub must be unregistered, not leaked"
    );
}

// ── Decode errors name the builtin ─────────────────────────────────────────

/// `declare-plugin`'s argument decoders must name the builtin in their error,
/// matching every other builtin's naming idiom, with one spelling
/// (`#:commands`/`#:events`/`#:languages`) shared by all three.
///
/// Fail oracle: revert the label args back to bare `"commands"` →
/// the assertion on the `declare-plugin #:commands` prefix fails.
#[test]
fn declare_plugin_bad_commands_names_the_builtin() {
    let err = declare_err(r#"(declare-plugin "user/tp" #:commands '(1))"#);
    assert!(
        err.contains("declare-plugin #:commands"),
        "error must name the builtin; got: {err}"
    );
}

/// Same naming requirement for `#:typed-commands` — the sibling decoder no
/// `hume-scripting` test exercised before this: every other `declare-plugin`
/// decode test in this file supplies `#:commands` only.
///
/// Fail oracle: revert `declare_arg_label(ctx, "#:typed-commands")` back to a
/// bare `"typed-commands"` label → the assertion on the
/// `declare-plugin #:typed-commands` prefix fails.
#[test]
fn declare_plugin_bad_typed_commands_names_the_builtin() {
    let err = declare_err(r#"(declare-plugin "user/tp" #:typed-commands '(1))"#);
    assert!(
        err.contains("declare-plugin #:typed-commands"),
        "error must name the builtin; got: {err}"
    );
}

/// Same naming requirement for an unknown `#:events` hook name.
#[test]
fn declare_plugin_unknown_hook_names_the_builtin() {
    let err =
        declare_err(r#"(declare-plugin "user/tp" #:commands '("c") #:events '(not-a-real-hook))"#);
    assert!(
        err.contains("declare-plugin #:events"),
        "error must name the builtin; got: {err}"
    );
}

/// `#:events` entries are symbols, not strings — same rule `register-hook!`
/// enforces. A string entry hard-errors instead of being silently accepted.
///
/// Fail oracle: revert `declare_plugin`'s `#:events` decode back to
/// `list_to_strings` → the string entry is accepted and this test fails.
#[test]
fn declare_plugin_rejects_string_event_names() {
    let err = declare_err(r#"(declare-plugin "user/tp" #:events '("on-buffer-save"))"#);
    assert!(
        err.contains("expected an event-name symbol"),
        "error must name the expected form; got: {err}"
    );
}

/// A `#:events` entry rejected for being unknown/malformed must leave no
/// trace in `declared_plugins`/`plugin_configs` — PLUM reads the former to
/// decide what to install, and a name that appears there with no matching
/// `LazyRegistry` entry can never be reconciled short of a restart.
///
/// Fail oracle: move the decode back below the `plugin_configs`
/// write/`record_declared` call → this test fails because the name is
/// recorded despite the rejection.
#[test]
fn declare_plugin_rejected_events_records_nothing() {
    use crate::ScriptingHost;
    use crate::null_host::LazyStubHost;
    let mut host = ScriptingHost::new();
    let mut editor_host = LazyStubHost::default();
    host.eval_source(
        r#"(declare-plugin "user/tp" #:events '(not-a-real-hook))"#,
        &mut editor_host,
    )
    .expect_err("unknown hook name must be rejected");

    assert!(
        !host
            .registries
            .declared_plugins
            .iter()
            .any(|d| d == "user/tp"),
        "a rejected declaration must not be recorded for PLUM: {:?}",
        host.registries.declared_plugins
    );
    let id = PluginId::parse("user/tp").unwrap();
    assert!(
        !host.registries.plugin_configs.contains_key(&id),
        "a rejected declaration must not leave a stored config behind"
    );
}

// ── Command-name character validation ─────────────────────────────────────

/// `declare-plugin` hard-errors on a `#:commands` entry containing `"` or
/// `\` — the same rule `define-command!` enforces.
///
/// Fail oracle: remove the character check from the filter loop → the name
/// registers as an activation entry and the declare succeeds.
#[test]
fn declare_plugin_command_name_with_quote_errors() {
    use crate::ScriptingHost;
    use crate::host::EditorHost;
    use crate::null_host::LazyStubHost;
    let mut host = ScriptingHost::new();
    let mut editor_host = LazyStubHost::default();
    let result = host.eval_source(
        r#"(declare-plugin "user/tp" #:commands '("bad\"name"))"#,
        &mut editor_host,
    );
    let err = result.expect_err("quoted command name must be rejected");
    assert!(
        err.contains("must not contain"),
        "error must name the invalid character rule; got: {err}"
    );
    assert!(
        editor_host
            .commands()
            .lazy_command_owner("bad\"name")
            .is_none(),
        "no activation entry may be recorded for a malformed name"
    );
}

// ── cmd_owners not seeded for absent-path plugins ──────────────────────────

/// When a declared plugin is absent on disk, `cmd_owners` must NOT be pre-seeded.
///
/// Guards against `cmd_owners` being seeded before the path check: an absent
/// plugin would otherwise leave orphan attribution entries that
/// `drop_activations_for` could never clean up. The invariant: the
/// absent-path early-return fires before the pre-seed loop.
///
/// Fail oracle: remove the `if path.is_none() { return Ok(…) }` early-return →
/// cmd_owners gets seeded → assertion fires.
#[test]
fn declare_plugin_absent_on_disk_does_not_seed_cmd_owners() {
    use crate::{ScriptingHost, null_host::NullHost};
    let mut host = ScriptingHost::new();
    // `core:nonexistent-plugin` cannot exist on disk in any test environment.
    let result = host.eval_source(
        r#"(declare-plugin "core:nonexistent-plugin" #:commands '("my-cmd"))"#,
        &mut NullHost,
    );
    assert!(
        result.is_ok(),
        "absent-path declare-plugin must not error; got: {result:?}"
    );
    assert!(
        !host.cmd_owners_for_test().contains_key("my-cmd"),
        "cmd_owners must not be seeded when the plugin is absent on disk"
    );
}

// ── Absent plugin logging ──────────────────────────────────────────────────

/// `declare-plugin "core:X"` absent on disk → `Error` log (typo / broken
/// HUME_RUNTIME; PLUM never installs core: plugins so it can't catch this).
///
/// Fail oracle: remove `log_absent_core` call → no Error message → assertion fires.
#[test]
fn declare_plugin_core_absent_logs_error() {
    use crate::{ScriptingHost, null_host::NullHost};
    let mut host = ScriptingHost::new();
    let result = host.eval_source(
        r#"(declare-plugin "core:nonexistent-plugin" #:commands '("my-cmd"))"#,
        &mut NullHost,
    );
    assert!(
        result.is_ok(),
        "absent core: declare must be non-fatal; got: {result:?}"
    );
    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(level, msg)| {
            matches!(level, crate::log::LogLevel::Error)
                && msg.contains("unknown core plugin")
                && msg.contains("core:nonexistent-plugin")
        }),
        "must log Error for absent core: plugin; messages: {messages:?}"
    );
    assert!(
        !messages
            .iter()
            .any(|(_, msg)| msg.contains("install and reload")),
        "must not suggest install for core: plugin; messages: {messages:?}"
    );
}

/// `declare-plugin "user/X"` absent on disk → `Info` log (not yet installed;
/// PLUM will surface it on :plum-install-plugins — no change needed in HUME).
///
/// Fail oracle: swap Info→Error → assertion fires.
#[test]
fn declare_plugin_user_absent_logs_info() {
    use crate::{ScriptingHost, null_host::NullHost};
    let mut host = ScriptingHost::new();
    let result = host.eval_source(
        r#"(declare-plugin "user/definitely-absent-99" #:commands '("my-cmd-2"))"#,
        &mut NullHost,
    );
    assert!(
        result.is_ok(),
        "absent user/ declare must be non-fatal; got: {result:?}"
    );
    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(level, msg)| {
            matches!(level, crate::log::LogLevel::Info) && msg.contains("not found on disk")
        }),
        "must log Info for absent user/ plugin; messages: {messages:?}"
    );
}

/// `load-plugin "core:X"` absent on disk → `Error` log (was silently swallowed).
///
/// Fail oracle: remove `log_absent_core` call in load_plugin → no Error message → assertion fires.
#[test]
fn load_plugin_core_absent_logs_error() {
    use crate::{ScriptingHost, null_host::NullHost};
    let mut host = ScriptingHost::new();
    let result = host.eval_source(r#"(load-plugin "core:nonexistent-plugin")"#, &mut NullHost);
    assert!(
        result.is_ok(),
        "absent core: load must be non-fatal; got: {result:?}"
    );
    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(level, msg)| {
            matches!(level, crate::log::LogLevel::Error)
                && msg.contains("unknown core plugin")
                && msg.contains("core:nonexistent-plugin")
        }),
        "must log Error for absent core: load-plugin; messages: {messages:?}"
    );
}

// ── Activation command name collision (symmetric checks) ─────────────────

/// `define-command!` rejects a name already claimed as a lazy plugin's `Lazy`
/// stub, even when the eager `define-command!` runs first.
///
/// Fail oracle: remove the `lazy_command_owner` guard from `define_command`
/// → the eager define succeeds, the stub is orphaned, the plugin is stuck
/// `Declared` and can never load.
#[test]
fn define_command_rejects_name_claimed_by_lazy_plugin() {
    use crate::ScriptingHost;
    use crate::host::EditorHost;
    use crate::null_host::LazyStubHost;

    let id = PluginId::parse("core:my-plugin").unwrap();
    let mut host = ScriptingHost::new();
    // Simulate declare-plugin having claimed the name as a `Lazy` stub.
    let mut editor_host = LazyStubHost::default();
    editor_host
        .commands()
        .register_lazy_command("my-lazy-cmd", &id)
        .expect("stub claim must succeed on a fresh host");

    let result = host.eval_source(
        r#"(define-command! "my-lazy-cmd" "doc" (lambda () 0))"#,
        &mut editor_host,
    );

    assert!(
        result.is_err(),
        "define-command! must reject a name claimed by a lazy plugin; got Ok"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("claimed as an activation command"),
        "error must name the collision; got: {err}"
    );
    // The stub must survive — only unregister_lazy_stubs_of removes it (on load/fail).
    assert_eq!(
        editor_host.commands().lazy_command_owner("my-lazy-cmd"),
        Some(id),
        "Lazy stub must not be removed by the failed define-command!"
    );
}

/// The typed twin of [`define_command_rejects_name_claimed_by_lazy_plugin`] —
/// this section's "symmetric checks" had only the mappable half before this.
/// `define-typed-command!` must reject a name already claimed as a lazy
/// plugin's typed `Lazy` stub, even when the eager define runs first.
///
/// Fail oracle: remove the `lazy_command_owner` guard from
/// `define_typed_command` → the eager define succeeds, the stub is orphaned,
/// the plugin is stuck `Declared` and can never load.
#[test]
fn define_typed_command_rejects_name_claimed_by_lazy_plugin() {
    use crate::ScriptingHost;
    use crate::host::EditorHost;
    use crate::null_host::LazyStubHost;

    let id = PluginId::parse("core:my-plugin").unwrap();
    let mut host = ScriptingHost::new();
    // Simulate declare-plugin having claimed the name as a typed `Lazy` stub.
    let mut editor_host = LazyStubHost::default();
    editor_host
        .commands()
        .register_lazy_typed_command("my-lazy-cmd", &id)
        .expect("stub claim must succeed on a fresh host");

    let result = host.eval_source(
        r#"(define-typed-command! "my-lazy-cmd" "doc" (lambda () 0))"#,
        &mut editor_host,
    );

    assert!(
        result.is_err(),
        "define-typed-command! must reject a name claimed by a lazy plugin; got Ok"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("claimed as an activation command"),
        "error must name the collision; got: {err}"
    );
    // The stub must survive — only unregister_lazy_stubs_of removes it (on load/fail).
    assert_eq!(
        editor_host.commands().lazy_command_owner("my-lazy-cmd"),
        Some(id),
        "Lazy stub must not be removed by the failed define-typed-command!"
    );
}

// ── Windows path escaping ────────────────────────────────────────────────

/// `%begin-lazy-activation` must escape backslashes in the plugin path so
/// a Windows-style path (e.g. `C:\Users\x\plugin.scm`) survives embedding
/// inside a Steel string literal without producing an invalid-escape error.
///
/// This test inserts a synthetic backslash-bearing path without creating a
/// real file; it checks only the returned require-string by comparing against
/// the hand-computed expected value via Steel's `equal?`.
///
/// Fail oracle: remove the `replace('\\', "\\\\")` call from
/// `begin_lazy_activation` → the returned string is
/// `(require "C:\Users\x\plugin.scm")` (raw backslashes) while the expected
/// literal is `(require "C:\\Users\\x\\plugin.scm")` → `equal?` is `#f` →
/// `error` fires → `eval_source` returns `Err` → `unwrap` panics.
#[test]
fn begin_lazy_activation_escapes_backslashes_in_path() {
    use crate::{ScriptingHost, null_host::NullHost};
    use std::path::PathBuf;

    let id = PluginId::parse("core:winpath").unwrap();
    let mut host = ScriptingHost::new();
    host.registries.lazy_registry.plugins.insert(
        id.clone(),
        PluginState::Declared {
            path: PathBuf::from(r"C:\Users\x\plugin.scm"),
        },
    );

    // Oracle: each `\` in the path is doubled by the fix, so the Steel string
    // value held in `__result` is:
    //   (require "C:\\Users\\x\\plugin.scm")
    // To express that as a Steel string literal we double every `\` again:
    //   "(require \"C:\\\\Users\\\\x\\\\plugin.scm\")"
    // In a Rust raw string (r#"…"#) there is no further Rust escaping.
    let program = r#"
(define __result (%begin-lazy-activation "core:winpath"))
(when (not (equal? __result "(require \"C:\\\\Users\\\\x\\\\plugin.scm\")"))
  (error (string-append "backslash escaping wrong; got: " __result)))
"#;
    host.eval_source(program, &mut NullHost)
        .expect("backslashes in path must be doubled for Steel string embedding");

    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Loading)
        ),
        "plugin must be Loading after %begin-lazy-activation"
    );
}

// ── #:config / (plugin-config) ────────────────────────────────────────────

/// `(plugin-config)` called outside any plugin body (top-level init.scm) must
/// return an empty hash, not error.
///
/// Fail oracle: if `plugin_stack.current()` were mis-read (e.g. always
/// returning the last-ever-pushed id instead of `None` once popped), this
/// would return a stale plugin's config instead of empty.
#[test]
fn plugin_config_outside_plugin_body_is_empty() {
    use crate::{ScriptingHost, null_host::NullHost};
    let mut host = ScriptingHost::new();
    host.eval_source(
        r#"(when (not (hash-empty? (plugin-config))) (error "expected empty hash"))"#,
        &mut NullHost,
    )
    .expect("plugin-config outside a plugin body must be an empty hash");
}

// ── Zero-trigger backstop (direct %declare-plugin! call) ──────────────────

/// A direct `%declare-plugin!` call with all three activation lists empty must
/// hard-error — the pre-existing backstop the Scheme `declare-plugin` wrapper's
/// zero-trigger routing sits in front of. No prior test exercised this directly.
///
/// Fail oracle: remove the zero-entry check in `declare_plugin` → this call
/// silently registers nothing and returns `Ok`.
#[test]
fn declare_plugin_bang_direct_zero_trigger_call_errors() {
    use crate::{ScriptingHost, null_host::NullHost};
    let mut host = ScriptingHost::new();
    let result = host.eval_source(
        r#"(%declare-plugin! "user/direct-zero" '() '() '() '() (hash))"#,
        &mut NullHost,
    );
    let err = result.expect_err("direct %declare-plugin! with zero activation entries must error");
    assert!(
        err.contains("could never be activated"),
        "error must explain the plugin could never be activated; got: {err}"
    );
}

/// A zero-trigger declare of a plugin whose directory doesn't exist at all is a
/// soft no-op — Info log, `declared_plugins` recorded for PLUM, no plugin state.
/// Mirrors the existing `declare_plugin_user_absent_logs_info` behavior for the
/// trigger-ful path.
///
/// Fail oracle: routing "not installed yet" to the same hard error as "installed
/// but missing manifest.scm" would break the declare-then-:plum-install-plugins flow for
/// every zero-trigger declare of an as-yet-uninstalled plugin.
#[test]
fn manifest_declare_absent_dir_soft_logs_and_records_declared_plugins() {
    use crate::{ScriptingHost, null_host::NullHost};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());

    let result = host.eval_source(
        r#"(declare-plugin "user/definitely-absent-mf")"#,
        &mut NullHost,
    );
    assert!(
        result.is_ok(),
        "absent-dir zero-trigger declare must not error; got {result:?}"
    );

    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(level, msg)| {
            matches!(level, crate::log::LogLevel::Info) && msg.contains("not found on disk")
        }),
        "must log Info for an absent user/ plugin directory; messages: {messages:?}"
    );
    assert!(
        host.registries
            .declared_plugins
            .iter()
            .any(|d| d == "user/definitely-absent-mf"),
        "declared_plugins must record the name for PLUM even though nothing was declared"
    );
    let id = PluginId::parse("user/definitely-absent-mf").unwrap();
    assert!(
        !host.registries.lazy_registry.plugins.contains_key(&id),
        "no plugin state should be recorded when the directory is absent"
    );
}

#[cfg(unix)]
mod unix;
