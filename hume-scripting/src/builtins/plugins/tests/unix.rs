//! Unix-only plugin-declaration tests, gated once at the
//! `mod unix;` declaration in the parent.

use super::*;

// ── G3: zero-entry error distinguishes collided vs not-supplied ───────────

/// When ALL provided `#:commands` entries collide with built-ins, the
/// error message must mention "conflicted", not suggest adding #:commands
/// (which the user already did).
///
/// Collision filtering only runs once the plugin is confirmed present on
/// disk (absent plugins skip it entirely — see G4 below), so this test needs
/// a real on-disk plugin, unlike a same-named `core:` plugin that would
/// otherwise hit the absent-path branch first.
///
/// Fail oracle: remove the "all filtered" branch → generic "Add #:commands"
/// message → second assertion fires.
#[test]
fn declare_plugin_all_on_command_collided_message_mentions_conflict() {
    use crate::ScriptingHost;
    use crate::null_host::NullHost;
    use rustc_hash::FxHashSet;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let plugin_dir = dir
        .path()
        .join("plugins")
        .join("user")
        .join("test-collision");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), b"").unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    // Mark "insert-mode" as a built-in so collision filtering drops it.
    let mut builtin_names = FxHashSet::default();
    builtin_names.insert("insert-mode".to_string());

    let result = host.eval_source_returning_defs(
        r#"(declare-plugin "user/test-collision" #:commands '("insert-mode"))"#.to_owned(),
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

/// `declare-plugin` drops `#:commands` entries that conflict with already-registered
/// eager commands; when the dropped entry was the sole activation signal, it errors
/// immediately (no orphan entry, no plugin stuck `Declared`).
///
/// Fail oracle: remove the eager-command check from `declare_plugin`'s filter
/// loop → the name slips through as a `Lazy` stub and the "no activation
/// entries" error is not raised.
#[test]
fn declare_plugin_drops_sole_command_conflicting_with_eager() {
    use crate::ScriptingHost;
    use crate::host::EditorHost;
    use crate::null_host::LazyStubHost;
    use crate::types::SteelCmdDef;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    // Plugin file must exist so declare-plugin proceeds past the path check.
    let plugin_dir = dir.path().join("plugins").join("user").join("test-repo");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), b"").unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    // Simulate an eager command already occupying the name in the editor's registry.
    let mut editor_host = LazyStubHost::default();
    editor_host
        .commands()
        .register_command(SteelCmdDef {
            name: "my-eager-cmd".to_string(),
            doc: String::new(),
            arity: 0,
            is_variadic: false,
            inline_output: false,
            repeatable: false,
        })
        .unwrap();

    let result = host.eval_source(
        r#"(declare-plugin "user/test-repo" #:commands '("my-eager-cmd"))"#,
        &mut editor_host,
    );

    // All entries filtered → declare-plugin must fail with "no activation entries".
    let err = result.expect_err(
        "declare-plugin must error when sole #:commands entry is taken by an eager command",
    );
    assert!(
        err.contains("no activation entries") || err.contains("conflicted"),
        "error must explain the cause; got: {err}"
    );
    // Must not register a Lazy stub for the conflicting eager command name.
    assert!(
        editor_host
            .commands()
            .lazy_command_owner("my-eager-cmd")
            .is_none(),
        "must not claim a Lazy stub for the conflicting eager command name"
    );
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

// ── manifest.scm (zero-trigger declare-plugin) ─────────────────────────────

/// A zero-trigger `(declare-plugin "id")` with a `manifest.scm` present resolves
/// and evaluates it, registering whatever the manifest declares for itself.
///
/// Fail oracle: if the Scheme wrapper didn't route to `%begin-manifest-declare!`
/// on empty lists, this would hit the "could never be activated" backstop error
/// instead of succeeding.
#[test]
fn manifest_declare_resolves_and_evaluates_manifest_scm() {
    use crate::ScriptingHost;
    use crate::host::EditorHost;
    use crate::null_host::LazyStubHost;
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
    let mut editor_host = LazyStubHost::default();

    host.eval_source(r#"(declare-plugin "user/mftest")"#, &mut editor_host)
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
        editor_host.commands().lazy_command_owner("mf-cmd"),
        Some(id),
        "manifest's #:commands entry must be recorded as a Lazy stub"
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

/// A zero-trigger declare of a plugin whose directory exists but has no
/// `manifest.scm` is a hard error, distinct from "not installed yet".
///
/// Fail oracle: treating this the same as an absent directory would silently
/// no-op instead of telling the user their plugin doesn't support default
/// activation.
#[test]
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
