//! Unix-only plugin-declaration tests, gated once at the
//! `mod unix;` declaration in the parent.

use super::*;
use crate::ScriptingHost;
use crate::attribution::EntryFile;
use tempfile::TempDir;

pub(super) fn main_entry(plugin: &str) -> EntryId {
    EntryId::main(PluginId::parse(plugin).unwrap())
}

pub(super) fn secondary(plugin: &str, file: &str) -> EntryId {
    EntryId::new(
        PluginId::parse(plugin).unwrap(),
        EntryFile::parse(file).unwrap(),
    )
}

/// A `user/<repo>` plugin directory holding `files` (name, source), and a host
/// whose data dir points at it.
pub(super) fn host_with_plugin(repo: &str, files: &[(&str, &str)]) -> (TempDir, ScriptingHost) {
    let dir = TempDir::new().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join(repo);
    std::fs::create_dir_all(&plugin_dir).unwrap();
    for (name, src) in files {
        std::fs::write(plugin_dir.join(name), src).unwrap();
    }
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    (dir, host)
}

// ── Zero-entry error distinguishes collided vs not-supplied ────────────────

/// When ALL provided `#:commands` entries collide with built-ins, the
/// error message must mention "conflicted", not suggest adding #:commands
/// (which the user already did).
///
/// Collision filtering only runs once the plugin is confirmed present on
/// disk (absent plugins skip it entirely), so this test needs
/// a real on-disk plugin, unlike a same-named `core:` plugin that would
/// otherwise hit the absent-path branch first.
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

    let result = host
        .eval_source_raw(
            r#"(declare-plugin! "user/test-collision" #:commands '("insert-mode"))"#.to_owned(),
            builtin_names,
            10_000,
            &mut NullHost,
        )
        .map_err(|e| e.message);

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

/// `declare-plugin!` drops `#:commands` entries that conflict with already-registered
/// eager commands; when the dropped entry was the sole activation signal, it errors
/// immediately (no orphan entry, no plugin stuck `Declared`).
#[test]
fn declare_plugin_drops_sole_command_conflicting_with_eager() {
    use crate::ScriptingHost;
    use crate::host::EditorHost;
    use crate::null_host::LazyStubHost;
    use crate::types::SteelCmdDef;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    // Plugin file must exist so declare-plugin! proceeds past the path check.
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
        r#"(declare-plugin! "user/test-repo" #:commands '("my-eager-cmd"))"#,
        &mut editor_host,
    );

    // All entries filtered → declare-plugin! must fail with "no activation entries".
    let err = result.expect_err(
        "declare-plugin! must error when sole #:commands entry is taken by an eager command",
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

/// The typed twin of [`declare_plugin_drops_sole_command_conflicting_with_eager`],
/// but a *partial* collision. It drives `#:typed-commands` through a real
/// on-disk `declare-plugin!` and covers "one entry collides, one survives".
/// One colliding name (`existing-typed`, already defined) must log an `Error`
/// and be dropped; the plugin still declares because `fresh-typed` survives.
#[test]
fn declare_plugin_typed_commands_drops_colliding_entry_but_keeps_the_rest() {
    use crate::ScriptingHost;
    use crate::host::EditorHost;
    use crate::null_host::LazyStubHost;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), b"").unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    let mut editor_host = LazyStubHost::default();

    host.eval_source(
        r#"(define-typed-command! "existing-typed" "doc" (lambda () 0))"#,
        &mut editor_host,
    )
    .expect("pre-existing definition must succeed");

    host.eval_source(
        r#"(declare-plugin! "user/tp" #:typed-commands '("existing-typed" "fresh-typed"))"#,
        &mut editor_host,
    )
    .expect("declare-plugin! must still succeed: one entry survives");

    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(level, msg)| {
            matches!(level, crate::log::LogLevel::Error)
                && msg.contains("existing-typed")
                && msg.contains("activation entry ignored")
        }),
        "colliding entry must log an Error naming it; messages: {messages:?}"
    );

    let id = EntryId::main(PluginId::parse("user/tp").unwrap());
    assert_eq!(
        editor_host.commands().lazy_command_owner("existing-typed"),
        None,
        "colliding entry must not become a lazy stub"
    );
    assert_eq!(
        editor_host.commands().lazy_command_owner("fresh-typed"),
        Some(id),
        "the surviving entry must become a lazy stub owned by the plugin"
    );
}

/// `#:config` passed to `(load-plugin! …)` must be observable by the body of an
/// entry declared earlier, via `(plugin-config)`, whenever activation
/// eventually runs it: declare and activation are separated in time.
///
/// This depends on `load_plugin` storing `config` in the plugin's record
/// and on `plugin_config` resolving the right `PluginId` from
/// `plugin_stack`.
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
        r#"(declare-plugin! "user/cfgtest" #:commands '("probe"))
           (load-plugin! "user/cfgtest" #:config (hash "key" "val"))"#,
        &mut NullHost,
    )
    .expect("declare then load with #:config must succeed");

    // Activation happens later, decoupled from declare: exactly the lazy
    // scenario the config channel must survive.
    host.eval_source(
        r#"(%activate-plugin-inline! "user/cfgtest" "plugin.scm")"#,
        &mut NullHost,
    )
    .expect("lazy activation must succeed");

    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(_, msg)| msg == "val"),
        "plugin body must observe #:config passed to load-plugin!; messages: {messages:?}"
    );
}

// ── manifest.scm (load-plugin!) ─────────────────────────────────────────────

/// `(load-plugin! "id")` with a `manifest.scm` present evaluates it, registering
/// whatever the manifest declares for itself.
#[test]
fn load_plugin_evaluates_manifest_scm() {
    use crate::host::EditorHost;
    use crate::null_host::LazyStubHost;

    let (_dir, mut host) = host_with_plugin(
        "mftest",
        &[
            ("plugin.scm", ""),
            (
                "manifest.scm",
                r#"(declare-plugin! "user/mftest" #:commands '("mf-cmd"))"#,
            ),
        ],
    );
    let mut editor_host = LazyStubHost::default();

    host.eval_source(r#"(load-plugin! "user/mftest")"#, &mut editor_host)
        .expect("load-plugin! with a manifest.scm present must succeed");

    let id = main_entry("user/mftest");
    assert!(
        matches!(
            host.registries.lazy_registry.plugins.get(&id),
            Some(PluginState::Declared { .. })
        ),
        "manifest's own declare-plugin! must register the plugin as Declared"
    );
    assert_eq!(
        editor_host.commands().lazy_command_owner("mf-cmd"),
        Some(id),
        "manifest's #:commands entry must be recorded as a Lazy stub"
    );
}

/// `#:config` on `load-plugin!` reaches an entry the manifest declared: a plugin
/// body reading `(plugin-config)` at activation sees the user's value.
#[test]
fn load_plugin_config_reaches_a_manifest_declared_entry() {
    use crate::null_host::NullHost;

    let (_dir, mut host) = host_with_plugin(
        "cfgmftest",
        &[
            (
                "plugin.scm",
                r#"(log! 'info (hash-ref (plugin-config) "key"))"#,
            ),
            (
                "manifest.scm",
                r#"(declare-plugin! "user/cfgmftest" #:commands '("probe"))"#,
            ),
        ],
    );

    host.eval_source(
        r#"(load-plugin! "user/cfgmftest" #:config (hash "key" "val"))"#,
        &mut NullHost,
    )
    .expect("load-plugin! with #:config must succeed");

    host.eval_source(
        r#"(%activate-plugin-inline! "user/cfgmftest" "plugin.scm")"#,
        &mut NullHost,
    )
    .expect("lazy activation must succeed");

    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(_, msg)| msg == "val"),
        "the config passed to load-plugin! must reach the entry; messages: {messages:?}"
    );
}

/// A plugin directory without a `manifest.scm` has nothing to declare lazily,
/// so `load-plugin!` loads its `plugin.scm` right away.
#[test]
fn load_plugin_without_manifest_loads_eagerly() {
    use crate::null_host::NullHost;

    let (_dir, mut host) =
        host_with_plugin("nomf", &[("plugin.scm", r#"(log! 'info "body-ran")"#)]);

    host.eval_source(r#"(load-plugin! "user/nomf")"#, &mut NullHost)
        .unwrap();

    let id = main_entry("user/nomf");
    assert!(matches!(
        host.registries.lazy_registry.plugins.get(&id),
        Some(PluginState::Loaded)
    ));
    assert!(
        host.peek_pending_messages()
            .iter()
            .any(|(_, m)| m == "body-ran")
    );
}

/// A plugin directory with neither `manifest.scm` nor `plugin.scm` is a
/// misconfigured plugin, not a "not installed yet" state.
#[test]
fn load_plugin_dir_with_neither_manifest_nor_plugin_scm_errors() {
    use crate::{ScriptingHost, null_host::NullHost};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("plugins").join("user").join("empty")).unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());

    let err = host
        .eval_source(r#"(load-plugin! "user/empty")"#, &mut NullHost)
        .expect_err("a plugin directory with no entry file must error");
    assert!(
        err.contains("neither manifest.scm nor plugin.scm"),
        "got: {err}"
    );
}

/// A manifest.scm that declares a *different* plugin id than the one it was
/// resolved for must be rejected: a manifest for "user/wrongname" cannot smuggle
/// in a declaration for "user/somebody-else".
///
/// The `manifest_resolving` mismatch guard in `declare_plugin` enforces this.
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
        br#"(declare-plugin! "user/somebody-else" #:commands '("evil-cmd"))"#,
    )
    .unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());

    let result = host.eval_source(r#"(load-plugin! "user/wrongname")"#, &mut NullHost);
    assert!(
        result.is_ok(),
        "a failed manifest resolution must be contained, not propagate; got: {result:?}"
    );
    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(level, msg)| {
            matches!(level, crate::log::LogLevel::Error)
                && msg.contains("user/wrongname")
                && msg.contains("user/somebody-else")
        }),
        "must log an Error naming both the expected and actual ids; messages: {messages:?}"
    );

    let evil_id = EntryId::main(PluginId::parse("user/somebody-else").unwrap());
    assert!(
        !host.registries.lazy_registry.plugins.contains_key(&evil_id),
        "the smuggled-in plugin id must not be registered"
    );
}

/// A malformed activation entry inside a manifest.scm's own `declare-plugin!`
/// call must name `manifest.scm` and the plugin. The user's `init.scm` only
/// contains the `load-plugin!` call, so a bare `declare-plugin! #:events`
/// error would point at a line that doesn't exist in their config.
#[test]
fn manifest_bad_events_names_manifest_scm_and_plugin() {
    use crate::{ScriptingHost, null_host::NullHost};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("badevt");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), b"").unwrap();
    std::fs::write(
        plugin_dir.join("manifest.scm"),
        br#"(declare-plugin! "user/badevt" #:events '("on-buffer-save"))"#,
    )
    .unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());

    let result = host.eval_source(r#"(load-plugin! "user/badevt")"#, &mut NullHost);
    assert!(
        result.is_ok(),
        "a failed manifest resolution must be contained, not propagate; got: {result:?}"
    );
    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(level, msg)| {
            matches!(level, crate::log::LogLevel::Error)
                && msg.contains("manifest.scm")
                && msg.contains("user/badevt")
        }),
        "must log an Error naming manifest.scm and the plugin it belongs to, not init.scm; \
         messages: {messages:?}"
    );
}

/// A manifest.scm whose own `declare-plugin!` call names no triggers must error
/// and be contained: the entry could never activate.
#[test]
fn manifest_whose_declare_has_no_triggers_errors_and_is_contained() {
    use crate::{ScriptingHost, null_host::NullHost};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("selfmf");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), b"").unwrap();
    std::fs::write(
        plugin_dir.join("manifest.scm"),
        br#"(declare-plugin! "user/selfmf")"#,
    )
    .unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());

    let result = host.eval_source(r#"(load-plugin! "user/selfmf")"#, &mut NullHost);
    assert!(
        result.is_ok(),
        "a manifest.scm whose own declare-plugin! names no triggers must error but be contained, \
         not recurse or propagate; got: {result:?}"
    );
    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(level, msg)| {
            matches!(level, crate::log::LogLevel::Error)
                && msg.contains("declares no activation entries")
        }),
        "must log an Error naming the missing triggers; messages: {messages:?}"
    );
}

/// A manifest.scm that evaluates without error but never calls `declare-plugin!`
/// must still be rejected. Otherwise the outer declare silently no-ops with no
/// plugin ever registered.
///
/// The post-eval check in `%finish-manifest-load!` catches this case.
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

    let result = host.eval_source(r#"(load-plugin! "user/nodeclare")"#, &mut NullHost);
    assert!(
        result.is_ok(),
        "manifest.scm that never calls declare-plugin! must error but be contained, not \
         propagate; got: {result:?}"
    );
    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(level, msg)| {
            matches!(level, crate::log::LogLevel::Error) && msg.contains("did not declare")
        }),
        "must log an Error explaining the manifest never declared the plugin; \
         messages: {messages:?}"
    );
}

/// A second `load-plugin!` of an already-declared plugin is a silent no-op:
/// `manifest.scm` is not re-evaluated.
///
/// Without the declared-entry check in `load_plugin`, the manifest would run
/// twice and register its activation entries twice.
#[test]
fn load_plugin_second_call_is_silent_noop() {
    use crate::{ScriptingHost, null_host::NullHost};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("twicemf");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), b"").unwrap();
    std::fs::write(
        plugin_dir.join("manifest.scm"),
        br#"(log! 'info "manifest-ran") (declare-plugin! "user/twicemf" #:commands '("twice-cmd"))"#,
    )
    .unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());

    host.eval_source(r#"(load-plugin! "user/twicemf")"#, &mut NullHost)
        .expect("first load-plugin! must succeed");
    host.eval_source(r#"(load-plugin! "user/twicemf")"#, &mut NullHost)
        .expect("second load-plugin! of the same plugin must be a silent no-op, not an error");

    let ran_count = host
        .peek_pending_messages()
        .iter()
        .filter(|(_, msg)| msg == "manifest-ran")
        .count();
    assert_eq!(
        ran_count, 1,
        "manifest.scm must be evaluated once across repeated load-plugin! calls"
    );
}

// ── plugin-level state: absent and failed plugins ──────────────────────────

/// Two `declare-plugin!` calls for a plugin that is not installed report the
/// absence once.
#[test]
fn repeated_declare_of_an_absent_plugin_reports_it_once() {
    use crate::null_host::NullHost;

    let (_dir, mut host) = host_with_plugin("present", &[("plugin.scm", "")]);
    host.eval_source(
        r#"(declare-plugin! "user/gone" #:commands '("a"))
(declare-plugin! "user/gone" #:commands '("b"))"#,
        &mut NullHost,
    )
    .expect("an absent user plugin is not an error");

    let absent = host
        .peek_pending_messages()
        .iter()
        .filter(|(_, msg)| msg.contains("user/gone") && msg.contains("not found on disk"))
        .count();
    assert_eq!(absent, 1, "got: {:?}", host.peek_pending_messages());
}

/// `:plugin-status` lists a plugin that is not installed, with its own state.
#[test]
fn status_lists_an_absent_plugin() {
    use crate::null_host::NullHost;

    let (_dir, mut host) = host_with_plugin("present", &[("plugin.scm", "")]);
    host.eval_source(r#"(load-plugin! "user/gone")"#, &mut NullHost)
        .unwrap();

    let status = host.lazy_status_string(&[]);
    let row = status
        .lines()
        .find(|l| l.contains("user/gone"))
        .unwrap_or_else(|| panic!("no row for user/gone in:\n{status}"));
    assert!(row.contains("absent"), "row: {row}");
}

/// A manifest that raises before declaring anything leaves no entry row, and
/// `:plugin-status` still shows the plugin as failed.
#[test]
fn failed_manifest_without_entries_is_listed_and_has_no_entry_row() {
    use crate::null_host::NullHost;

    let (_dir, mut host) = host_with_plugin(
        "badmf",
        &[("plugin.scm", ""), ("manifest.scm", r#"(error "boom")"#)],
    );
    host.eval_source(r#"(load-plugin! "user/badmf")"#, &mut NullHost)
        .expect("a failing manifest is contained");

    assert!(
        host.plugin_status(&main_entry("user/badmf")).is_none(),
        "a manifest that declared nothing must not leave an entry row"
    );
    let status = host.lazy_status_string(&[]);
    let row = status
        .lines()
        .find(|l| l.contains("user/badmf"))
        .unwrap_or_else(|| panic!("no row for user/badmf in:\n{status}"));
    assert!(row.contains("failed"), "row: {row}");
}
