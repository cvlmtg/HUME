use super::*;

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

// ── Command-name character validation ─────────────────────────────────────

/// `declare-plugin` hard-errors on a `#:commands` entry containing `"` or
/// `\` — the same rule `define-command!` enforces.
///
/// Fail oracle: remove the character check from the filter loop → the name
/// registers as an activation entry and the declare succeeds.
#[test]
fn declare_plugin_command_name_with_quote_errors() {
    use crate::{ScriptingHost, null_host::NullHost};
    let mut host = ScriptingHost::new();
    let result = host.eval_source(
        r#"(declare-plugin "user/tp" #:commands '("bad\"name"))"#,
        &mut NullHost,
    );
    let err = result.expect_err("quoted command name must be rejected");
    assert!(
        err.contains("must not contain"),
        "error must name the invalid character rule; got: {err}"
    );
    assert!(
        host.registries.lazy_registry.activation_commands.is_empty(),
        "no activation entry may be recorded for a malformed name"
    );
}

// ── G1: cmd_owners not seeded for absent-path plugins ─────────────────────

/// When a declared plugin is absent on disk, `cmd_owners` must NOT be pre-seeded.
///
/// The old bug: `cmd_owners` was seeded before the path check, so an absent
/// plugin left orphan attribution entries that could never be cleaned up by
/// `drop_activations_for`.  The fix: the absent-path early-return fires before
/// the pre-seed loop.
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

// ── G3: zero-entry error distinguishes collided vs not-supplied ───────────

/// When ALL provided `#:commands` entries collide with built-ins, the
/// error message must mention "conflicted", not suggest adding #:commands
/// (which the user already did).
///
/// Fail oracle: remove the `had_commands` branch → generic "Add #:commands"
/// message → second assertion fires.
#[test]
fn declare_plugin_all_on_command_collided_message_mentions_conflict() {
    use crate::{ScriptingHost, null_host::NullHost};
    use std::collections::HashSet;
    let mut host = ScriptingHost::new();
    // Mark "insert-mode" as a built-in so collision filtering drops it.
    let mut builtin_names = HashSet::new();
    builtin_names.insert("insert-mode".to_string());

    let result = host.eval_source_returning_defs(
        r#"(declare-plugin "core:test-collision" #:commands '("insert-mode"))"#.to_owned(),
        builtin_names,
        &mut NullHost,
    );

    let err = result.expect_err("must error when all entries collide");
    assert!(
        err.contains("conflicted"),
        "error must mention the collision; got: {err}"
    );
    assert!(
        !err.contains("Add #:commands"),
        "must not suggest adding what user already provided; got: {err}"
    );
}

// ── G4: absent plugin logging ──────────────────────────────────────────────

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
/// PLUM will surface it on :plum-install — no change needed in HUME).
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

// ── G4: activation command name collision (symmetric checks) ─────────────

/// `define-command!` rejects a name already in `activation_commands` (claimed
/// by a lazy plugin), even when the eager `define-command!` runs first.
///
/// Fail oracle: remove the `activation_commands` guard from `define_command_inner`
/// → the eager define succeeds, the activation entry is orphaned, the plugin
/// is stuck `Declared` and can never load.
#[test]
fn define_command_rejects_name_claimed_by_lazy_plugin() {
    use crate::{ScriptingHost, null_host::NullHost};

    let id = PluginId::parse("core:my-plugin").unwrap();
    let mut host = ScriptingHost::new();
    // Simulate declare-plugin having claimed the name as an activation entry.
    host.registries
        .lazy_registry
        .activation_commands
        .insert("my-lazy-cmd".to_string(), id);

    let result = host.eval_source(
        r#"(define-command! "my-lazy-cmd" "doc" (lambda () 0))"#,
        &mut NullHost,
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
    // The activation entry must survive — only drop_activations_for removes it (on load/fail).
    assert!(
        host.registries
            .lazy_registry
            .activation_commands
            .contains_key("my-lazy-cmd"),
        "activation_commands entry must not be removed by the failed define-command!"
    );
}

/// `declare-plugin` drops `#:commands` entries that conflict with already-registered
/// eager commands; when the dropped entry was the sole activation signal, it errors
/// immediately (no orphan entry, no plugin stuck `Declared`).
///
/// Fail oracle: remove the `command_table` check from the `declare_plugin` filter
/// loop → the name slips into `activation_commands` as an orphan and the
/// "no activation entries" error is not raised.
#[test]
#[cfg(not(windows))]
fn declare_plugin_drops_sole_command_conflicting_with_eager() {
    use crate::{ScriptingHost, null_host::NullHost};
    use steel::rvals::SteelVal;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    // Plugin file must exist so declare-plugin proceeds past the path check.
    let plugin_dir = dir.path().join("plugins").join("user").join("test-repo");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), b"").unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    // Simulate an eager command already occupying the name.
    host.registries
        .command_table
        .insert("my-eager-cmd".to_string(), SteelVal::Void);

    let result = host.eval_source(
        r#"(declare-plugin "user/test-repo" #:commands '("my-eager-cmd"))"#,
        &mut NullHost,
    );

    // All entries filtered → declare-plugin must fail with "no activation entries".
    let err = result.expect_err(
        "declare-plugin must error when sole #:commands entry is taken by an eager command",
    );
    assert!(
        err.contains("no activation entries") || err.contains("conflicted"),
        "error must explain the cause; got: {err}"
    );
    // Must not pollute activation_commands with the orphan entry.
    assert!(
        !host
            .registries
            .lazy_registry
            .activation_commands
            .contains_key("my-eager-cmd"),
        "activation_commands must not be polluted with the conflicting eager command name"
    );
}

// ── ScriptingHost::drop_activation_command ────────────────────────────────

/// `drop_activation_command` removes the named command from both
/// `activation_commands` and `cmd_owners`, leaving unrelated entries intact.
///
/// Flip: if the method cleared ALL entries instead of just the named one,
/// `"y"` would be absent and the second assertion would fire.  If it did
/// nothing, `"x"` would still be present and the first assertion would fire.
#[test]
fn drop_activation_command_removes_entry_and_leaves_others() {
    use crate::ScriptingHost;

    let id = PluginId::parse("user/tp").unwrap();
    let id2 = PluginId::parse("user/other").unwrap();

    let mut host = ScriptingHost::new();
    // Seed two entries: only "x" will be dropped.
    host.registries
        .lazy_registry
        .activation_commands
        .insert("x".to_string(), id.clone());
    host.registries
        .cmd_owners
        .insert("x".to_string(), id.to_string());
    host.registries
        .lazy_registry
        .activation_commands
        .insert("y".to_string(), id2.clone());
    host.registries
        .cmd_owners
        .insert("y".to_string(), id2.to_string());

    host.drop_activation_command("x");

    assert!(
        !host.activation_commands().contains_key("x"),
        "activation_commands must not contain dropped entry 'x'"
    );
    assert!(
        !host.cmd_owners_for_test().contains_key("x"),
        "cmd_owners must not contain dropped entry 'x'"
    );
    // Unrelated entry must survive.
    assert!(
        host.activation_commands().contains_key("y"),
        "unrelated entry 'y' must not be removed"
    );
    assert!(
        host.cmd_owners_for_test().contains_key("y"),
        "unrelated cmd_owner 'y' must not be removed"
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

/// `#:config` passed to `(declare-plugin …)` at declare time must be observable
/// by the plugin body via `(plugin-config)` whenever activation eventually runs
/// it — the general mechanism this feature relies on, exercised on the lazy
/// path where declare and activation are separated in time.
///
/// Fail oracle: if `declare_plugin` didn't store `config` into
/// `plugin_configs`, or `plugin_config` didn't resolve the right `PluginId`
/// from `plugin_stack`, the body would observe an empty hash instead of "val"
/// and `log!` would never record it.
#[test]
#[cfg(not(windows))]
fn plugin_config_survives_lazy_declare_to_activation() {
    use crate::{ScriptingHost, null_host::NullHost};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("cfgtest");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(
        plugin_dir.join("plugin.scm"),
        br#"(log! 'info (hash-ref (plugin-config) "key"))"#,
    )
    .unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());

    host.eval_source(
        r#"(declare-plugin "user/cfgtest" #:commands '("probe") #:config (hash "key" "val"))"#,
        &mut NullHost,
    )
    .expect("declare-plugin with #:config must succeed");

    // Activation happens later, decoupled from declare — exactly the lazy
    // scenario the config channel must survive.
    host.eval_source(r#"(%activate-plugin-inline "user/cfgtest")"#, &mut NullHost)
        .expect("lazy activation must succeed");

    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(_, msg)| msg == "val"),
        "plugin body must observe #:config passed at declare-plugin time; messages: {messages:?}"
    );
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
        r#"(%declare-plugin! "user/direct-zero" '() '() '() (hash))"#,
        &mut NullHost,
    );
    let err = result.expect_err("direct %declare-plugin! with zero activation entries must error");
    assert!(
        err.contains("could never be activated"),
        "error must explain the plugin could never be activated; got: {err}"
    );
}

// ── manifest.scm (zero-trigger declare-plugin) ─────────────────────────────

/// A zero-trigger `(declare-plugin "id")` with a `manifest.scm` present resolves
/// and evaluates it, registering whatever the manifest declares for itself.
///
/// Fail oracle: if the Scheme wrapper didn't route to `%begin-manifest-declare!`
/// on empty lists, this would hit the "could never be activated" backstop error
/// instead of succeeding.
#[test]
#[cfg(not(windows))]
fn manifest_declare_resolves_and_evaluates_manifest_scm() {
    use crate::{ScriptingHost, null_host::NullHost};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("mftest");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), b"").unwrap();
    std::fs::write(
        plugin_dir.join("manifest.scm"),
        br#"(declare-plugin "user/mftest" #:commands '("mf-cmd"))"#,
    )
    .unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());

    host.eval_source(r#"(declare-plugin "user/mftest")"#, &mut NullHost)
        .expect("zero-trigger declare with a manifest.scm present must succeed");

    let id = PluginId::parse("user/mftest").unwrap();
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Declared { .. })
        ),
        "manifest's own declare-plugin must register the plugin as Declared"
    );
    assert_eq!(
        host.registries
            .lazy_registry
            .activation_commands
            .get("mf-cmd"),
        Some(&id),
        "manifest's #:commands entry must be recorded as an activation entry"
    );
}

/// `#:config` on the outer zero-trigger `declare-plugin` call wins over whatever
/// the manifest's own `declare-plugin` passes (the manifest here passes none,
/// i.e. the empty-hash default) — a plugin body reading `(plugin-config)` at
/// activation must see the user's value.
///
/// Fail oracle: if `declare_plugin`'s config store were an unconditional insert
/// during manifest resolution (instead of `or_insert`), the manifest's own
/// default would clobber the user's value and the message would never appear.
#[test]
#[cfg(not(windows))]
fn manifest_declare_user_config_wins_over_manifest_default() {
    use crate::{ScriptingHost, null_host::NullHost};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("cfgmftest");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(
        plugin_dir.join("plugin.scm"),
        br#"(log! 'info (hash-ref (plugin-config) "key"))"#,
    )
    .unwrap();
    std::fs::write(
        plugin_dir.join("manifest.scm"),
        br#"(declare-plugin "user/cfgmftest" #:commands '("probe"))"#,
    )
    .unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());

    host.eval_source(
        r#"(declare-plugin "user/cfgmftest" #:config (hash "key" "val"))"#,
        &mut NullHost,
    )
    .expect("zero-trigger declare with #:config must succeed");

    host.eval_source(
        r#"(%activate-plugin-inline "user/cfgmftest")"#,
        &mut NullHost,
    )
    .expect("lazy activation must succeed");

    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(_, msg)| msg == "val"),
        "the outer #:config must win over the manifest's own default; messages: {messages:?}"
    );
}

/// A zero-trigger declare of a plugin whose directory doesn't exist at all is a
/// soft no-op — Info log, `declared_plugins` recorded for PLUM, no plugin state.
/// Mirrors the existing `declare_plugin_user_absent_logs_info` behavior for the
/// trigger-ful path.
///
/// Fail oracle: routing "not installed yet" to the same hard error as "installed
/// but missing manifest.scm" would break the declare-then-:plum-install flow for
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

/// A zero-trigger declare of a plugin whose directory exists but has no
/// `manifest.scm` is a hard error, distinct from "not installed yet".
///
/// Fail oracle: treating this the same as an absent directory would silently
/// no-op instead of telling the user their plugin doesn't support default
/// activation.
#[test]
#[cfg(not(windows))]
fn manifest_declare_dir_present_without_manifest_scm_errors() {
    use crate::{ScriptingHost, null_host::NullHost};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("nomf");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), b"").unwrap();
    // No manifest.scm written.

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());

    let result = host.eval_source(r#"(declare-plugin "user/nomf")"#, &mut NullHost);
    let err = result.expect_err("zero-trigger declare must hard-error without manifest.scm");
    assert!(
        err.contains("manifest.scm"),
        "error must name the missing manifest.scm; got: {err}"
    );
}

/// A manifest.scm that declares a *different* plugin id than the one it was
/// resolved for must be rejected — a manifest for "user/wrongname" cannot smuggle
/// in a declaration for "user/somebody-else".
///
/// Fail oracle: without the `manifest_resolving` mismatch guard in
/// `declare_plugin`, the smuggled-in id would register successfully.
#[test]
#[cfg(not(windows))]
fn manifest_declaring_different_plugin_name_errors() {
    use crate::{ScriptingHost, null_host::NullHost};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("wrongname");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), b"").unwrap();
    std::fs::write(
        plugin_dir.join("manifest.scm"),
        br#"(declare-plugin "user/somebody-else" #:commands '("evil-cmd"))"#,
    )
    .unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());

    let result = host.eval_source(r#"(declare-plugin "user/wrongname")"#, &mut NullHost);
    let err = result.expect_err("manifest declaring a different plugin id must error");
    assert!(
        err.contains("user/wrongname") && err.contains("user/somebody-else"),
        "error must name both the expected and actual ids; got: {err}"
    );

    let evil_id = PluginId::parse("user/somebody-else").unwrap();
    assert!(
        !host.registries.lazy_registry.plugins.contains_key(&evil_id),
        "the smuggled-in plugin id must not be registered"
    );
}

/// A manifest.scm whose own `declare-plugin` call is itself zero-trigger must
/// error immediately instead of recursing into manifest resolution again.
///
/// Fail oracle: without the `manifest_resolving.is_some()` guard in
/// `%begin-manifest-declare!`, this would loop (bounded only by the Steel VM's
/// own stack, not a designed guard) instead of erroring cleanly.
#[test]
#[cfg(not(windows))]
fn manifest_with_zero_trigger_self_declare_errors_without_recursing() {
    use crate::{ScriptingHost, null_host::NullHost};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("selfmf");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), b"").unwrap();
    std::fs::write(
        plugin_dir.join("manifest.scm"),
        br#"(declare-plugin "user/selfmf")"#,
    )
    .unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());

    let result = host.eval_source(r#"(declare-plugin "user/selfmf")"#, &mut NullHost);
    let err = result.expect_err(
        "a manifest.scm whose own declare-plugin is zero-trigger must error, not recurse",
    );
    assert!(
        err.contains("cannot itself be"),
        "error must explain the recursion guard; got: {err}"
    );
}

/// A manifest.scm that evaluates without error but never calls `declare-plugin`
/// must still be rejected — otherwise the outer declare silently no-ops with no
/// plugin ever registered.
///
/// Fail oracle: without the post-eval check in `%finish-manifest-declare!`, this
/// call would return `Ok` with nothing declared.
#[test]
#[cfg(not(windows))]
fn manifest_that_never_declares_errors() {
    use crate::{ScriptingHost, null_host::NullHost};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("nodeclare");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), b"").unwrap();
    std::fs::write(plugin_dir.join("manifest.scm"), b"(define x 1)").unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());

    let result = host.eval_source(r#"(declare-plugin "user/nodeclare")"#, &mut NullHost);
    let err = result.expect_err("manifest.scm that never calls declare-plugin must error");
    assert!(
        err.contains("did not declare"),
        "error must explain the manifest never declared the plugin; got: {err}"
    );
}

/// A second zero-trigger declare of an already-declared plugin is a silent
/// no-op — `manifest.scm` is not re-evaluated.
///
/// Fail oracle: without the pre-eval state check in `%begin-manifest-declare!`,
/// the manifest would run twice, double-registering activation entries.
#[test]
#[cfg(not(windows))]
fn manifest_declare_second_call_is_silent_noop() {
    use crate::{ScriptingHost, null_host::NullHost};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("twicemf");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), b"").unwrap();
    std::fs::write(
        plugin_dir.join("manifest.scm"),
        br#"(log! 'info "manifest-ran") (declare-plugin "user/twicemf" #:commands '("twice-cmd"))"#,
    )
    .unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());

    host.eval_source(r#"(declare-plugin "user/twicemf")"#, &mut NullHost)
        .expect("first zero-trigger declare must succeed");
    host.eval_source(r#"(declare-plugin "user/twicemf")"#, &mut NullHost)
        .expect(
            "second zero-trigger declare of the same plugin must be a silent no-op, not an error",
        );

    let ran_count = host
        .peek_pending_messages()
        .iter()
        .filter(|(_, msg)| msg == "manifest-ran")
        .count();
    assert_eq!(
        ran_count, 1,
        "manifest.scm must be evaluated exactly once across repeated zero-trigger declares"
    );
}
