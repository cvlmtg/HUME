use super::*;
use hume_platform::dirs::Dirs;

/// Eval `src` against a fresh host and return the error it must produce.
fn eval_err(src: &str) -> String {
    use crate::ScriptingHost;
    use crate::null_host::CommandStoreHost;
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut editor_host = CommandStoreHost::default();
    host.eval_source(src, &mut editor_host)
        .expect_err("expected the source to error")
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

// ── Decode errors name the builtin ─────────────────────────────────────────

/// `declare-plugin!`'s argument decoders must name the builtin in their error,
/// matching every other builtin's naming idiom, with one spelling
/// (`#:commands`/`#:events`/`#:languages`) shared by all three.
#[test]
fn declare_plugin_bad_commands_names_the_builtin() {
    let err = eval_err(r#"(declare-plugin! "user/tp" #:commands '(1))"#);
    assert!(
        err.contains("declare-plugin!: #:commands"),
        "error must name the builtin; got: {err}"
    );
}

/// Same naming requirement for `#:typed-commands`, the sibling decoder.
/// Every other `declare-plugin!` decode test in this file supplies
/// `#:commands` only.
#[test]
fn declare_plugin_bad_typed_commands_names_the_builtin() {
    let err = eval_err(r#"(declare-plugin! "user/tp" #:typed-commands '(1))"#);
    assert!(
        err.contains("declare-plugin!: #:typed-commands"),
        "error must name the builtin; got: {err}"
    );
}

/// Same naming requirement for an unknown `#:events` hook name.
#[test]
fn declare_plugin_unknown_hook_names_the_builtin() {
    let err =
        eval_err(r#"(declare-plugin! "user/tp" #:commands '("c") #:events '(not-a-real-hook))"#);
    assert!(
        err.contains("declare-plugin!: #:events"),
        "error must name the builtin; got: {err}"
    );
}

/// `#:events` entries are symbols, not strings, the same rule `register-hook!`
/// enforces. A string entry hard-errors instead of being silently accepted.
#[test]
fn declare_plugin_rejects_string_event_names() {
    let err = eval_err(r#"(declare-plugin! "user/tp" #:events '("on-buffer-save"))"#);
    assert!(
        err.contains("expected an event-name symbol"),
        "error must name the expected form; got: {err}"
    );
}

/// A `#:events` entry rejected for being unknown/malformed must leave no
/// trace in the plugin records: PLUM reads them to decide what to install, and
/// a name that appears there with no matching `LazyRegistry` entry can never
/// be reconciled short of a restart.
///
/// The decode therefore has to run before the plugin is noted.
#[test]
fn declare_plugin_rejected_events_records_nothing() {
    use crate::ScriptingHost;
    use crate::null_host::CommandStoreHost;
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut editor_host = CommandStoreHost::default();
    host.eval_source(
        r#"(declare-plugin! "user/tp" #:events '(not-a-real-hook))"#,
        &mut editor_host,
    )
    .expect_err("unknown hook name must be rejected");

    assert!(
        !host.declared_plugins().iter().any(|d| d == "user/tp"),
        "a rejected declaration must not be recorded for PLUM: {:?}",
        host.declared_plugins()
    );
    let id = PluginId::parse("user/tp").unwrap();
    assert!(
        host.registries.plugin_records.config(&id).is_none(),
        "a rejected declaration must not leave a stored config behind"
    );
}

// ── Command-name character validation ─────────────────────────────────────

/// `declare-plugin!` hard-errors on a malformed name, under the rules
/// `define-command!`/`define-typed-command!` enforce, before looking for the
/// plugin on disk (`user/tp` is absent here).
#[test]
fn declare_plugin_malformed_command_name_errors() {
    use crate::ScriptingHost;
    use crate::host::EditorHost;
    use crate::null_host::CommandStoreHost;
    for (keyword, literal, name, rule) in [
        (
            "#:commands",
            r#""bad\"name""#,
            "bad\"name",
            "must not contain",
        ),
        ("#:typed-commands", r#""b1""#, "b1", "typed command name"),
        ("#:commands", r#""Probe""#, "Probe", "lowercase"),
    ] {
        let mut host = ScriptingHost::new(&Dirs::none());
        let mut editor_host = CommandStoreHost::default();
        let err = host
            .eval_source(
                &format!(r#"(declare-plugin! "user/tp" {keyword} '({literal}))"#),
                &mut editor_host,
            )
            .expect_err("a malformed command name must be rejected");
        assert!(
            err.contains(rule),
            "{name}: error must name the '{rule}' rule; got: {err}"
        );
        assert!(
            editor_host.commands().lazy_command_owner(name).is_none(),
            "{name}: no activation entry may be recorded for a malformed name"
        );
    }
}

// ── Absent-path plugins attribute nothing ──────────────────────────────────

/// A plugin absent on disk claims no lazy stub, so `(command-plugin …)` does
/// not name it for a command it declared.
#[test]
fn declare_plugin_absent_on_disk_attributes_nothing() {
    use crate::{ScriptingHost, null_host::CommandStoreHost};
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut editor_host = CommandStoreHost::default();
    // `core:nonexistent-plugin` cannot exist on disk in any test environment.
    let result = host.eval_source(
        r#"(declare-plugin! "core:nonexistent-plugin" #:commands '("my-cmd"))"#,
        &mut editor_host,
    );
    assert!(
        result.is_ok(),
        "absent-path declare-plugin! must not error; got: {result:?}"
    );
    let owner = host.eval_source_for_value(r#"(command-plugin "my-cmd")"#, &mut editor_host);
    assert!(
        matches!(&owner, Ok(SteelVal::StringV(s)) if s.as_str() == "hume"),
        "an absent plugin must not own its declared command; got: {owner:?}"
    );
}

// ── Absent plugin logging ──────────────────────────────────────────────────

/// `declare-plugin! "core:X"` absent on disk → `Error` log (typo / broken
/// HUME_RUNTIME; PLUM never installs core: plugins so it can't catch this).
#[test]
fn declare_plugin_core_absent_logs_error() {
    use crate::{ScriptingHost, null_host::NullHost};
    let mut host = ScriptingHost::new(&Dirs::none());
    let result = host.eval_source(
        r#"(declare-plugin! "core:nonexistent-plugin" #:commands '("my-cmd"))"#,
        &mut NullHost,
    );
    assert!(
        result.is_ok(),
        "absent core: declare must be non-fatal; got: {result:?}"
    );
    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(level, msg)| {
            matches!(level, hume_core::Severity::Error)
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

/// `declare-plugin! "user/X"` absent on disk → `Info` log (not yet installed;
/// PLUM will surface it on :plum-install-plugins; no change needed in HUME).
#[test]
fn declare_plugin_user_absent_logs_info() {
    use crate::{ScriptingHost, null_host::NullHost};
    let mut host = ScriptingHost::new(&Dirs::none());
    let result = host.eval_source(
        r#"(declare-plugin! "user/definitely-absent-99" #:commands '("my-cmd-2"))"#,
        &mut NullHost,
    );
    assert!(
        result.is_ok(),
        "absent user/ declare must be non-fatal; got: {result:?}"
    );
    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(level, msg)| {
            matches!(level, hume_core::Severity::Info) && msg.contains("not found on disk")
        }),
        "must log Info for absent user/ plugin; messages: {messages:?}"
    );
}

/// `load-plugin! "core:X"` absent on disk → `Error` log.
#[test]
fn load_plugin_core_absent_logs_error() {
    use crate::{ScriptingHost, null_host::NullHost};
    let mut host = ScriptingHost::new(&Dirs::none());
    let result = host.eval_source(r#"(load-plugin! "core:nonexistent-plugin")"#, &mut NullHost);
    assert!(
        result.is_ok(),
        "absent core: load must be non-fatal; got: {result:?}"
    );
    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(level, msg)| {
            matches!(level, hume_core::Severity::Error)
                && msg.contains("unknown core plugin")
                && msg.contains("core:nonexistent-plugin")
        }),
        "must log Error for absent core: load-plugin!; messages: {messages:?}"
    );
}

// ── Activation command name collision (symmetric checks) ─────────────────

/// `define-command!` rejects a name already claimed as a lazy plugin's `Lazy`
/// stub, even when the eager `define-command!` runs first.
///
/// Without the `lazy_command_owner` guard in `define_command`, the eager
/// define would orphan the stub and leave the plugin stuck `Declared`,
/// never able to load.
#[test]
fn define_command_rejects_name_claimed_by_lazy_plugin() {
    use crate::ScriptingHost;
    use crate::host::EditorHost;
    use crate::null_host::CommandStoreHost;

    let id = EntryId::main(PluginId::parse("core:my-plugin").unwrap());
    let mut host = ScriptingHost::new(&Dirs::none());
    // Simulate declare-plugin! having claimed the name as a `Lazy` stub.
    let mut editor_host = CommandStoreHost::default();
    editor_host
        .commands()
        .register_lazy_commands(&id, &["my-lazy-cmd".into()], &[])
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
    // The stub must survive: only unregister_lazy_stubs_of removes it (on load/fail).
    assert_eq!(
        editor_host.commands().lazy_command_owner("my-lazy-cmd"),
        Some(id),
        "Lazy stub must not be removed by the failed define-command!"
    );
}

/// The typed twin of [`define_command_rejects_name_claimed_by_lazy_plugin`].
/// `define-typed-command!` must reject a name already claimed as a lazy
/// plugin's typed `Lazy` stub, even when the eager define runs first.
///
/// `define_typed_command` has its own `lazy_command_owner` guard for this.
#[test]
fn define_typed_command_rejects_name_claimed_by_lazy_plugin() {
    use crate::ScriptingHost;
    use crate::host::EditorHost;
    use crate::null_host::CommandStoreHost;

    let id = EntryId::main(PluginId::parse("core:my-plugin").unwrap());
    let mut host = ScriptingHost::new(&Dirs::none());
    // Simulate declare-plugin! having claimed the name as a typed `Lazy` stub.
    let mut editor_host = CommandStoreHost::default();
    editor_host
        .commands()
        .register_lazy_commands(&id, &[], &["my-lazy-cmd".into()])
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
    // The stub must survive: only unregister_lazy_stubs_of removes it (on load/fail).
    assert_eq!(
        editor_host.commands().lazy_command_owner("my-lazy-cmd"),
        Some(id),
        "Lazy stub must not be removed by the failed define-typed-command!"
    );
}

// ── #:config / (plugin-config) ────────────────────────────────────────────

/// `(plugin-config)` outside any plugin's code (top-level init.scm) raises:
/// an empty hash would read as "no config passed".
#[test]
fn plugin_config_outside_plugin_code_raises() {
    use crate::{ScriptingHost, null_host::NullHost};
    let mut host = ScriptingHost::new(&Dirs::none());
    let err = host
        .eval_source("(plugin-config)", &mut NullHost)
        .expect_err("plugin-config outside a plugin's code must raise");
    assert!(err.contains("outside a plugin's code"), "got: {err}");
}

/// `#:config` is a hash with symbol keys, checked where the user passes it.
#[test]
fn load_plugin_refuses_a_string_config_key() {
    use crate::{ScriptingHost, null_host::NullHost};
    let mut host = ScriptingHost::new(&Dirs::none());
    let err = host
        .eval_source(
            r#"(load-plugin! "core:none" #:config (hash 'ok #t "signs" #f))"#,
            &mut NullHost,
        )
        .expect_err("a string key must be refused");
    assert!(
        err.contains(r#"key "signs" must be a symbol: 'signs"#),
        "got: {err}"
    );
}

#[test]
fn load_plugin_refuses_a_config_that_is_not_a_hash() {
    use crate::{ScriptingHost, null_host::NullHost};
    let mut host = ScriptingHost::new(&Dirs::none());
    let err = host
        .eval_source(
            r#"(load-plugin! "core:none" #:config '(signs #t))"#,
            &mut NullHost,
        )
        .expect_err("a list must be refused");
    assert!(err.contains("#:config must be a hash"), "got: {err}");
}

// ── (plugin-dir) ──────────────────────────────────────────────────────────

/// `(plugin-dir)` outside any plugin's code raises, as `(plugin-config)` does.
#[test]
fn plugin_dir_outside_plugin_code_raises() {
    use crate::{ScriptingHost, null_host::NullHost};
    let mut host = ScriptingHost::new(&Dirs::none());
    let err = host
        .eval_source("(plugin-dir)", &mut NullHost)
        .expect_err("plugin-dir outside a plugin's code must raise");
    assert!(err.contains("outside a plugin's code"), "got: {err}");
}

/// Inside a plugin body `(plugin-dir)` is the directory holding that plugin's
/// own `plugin.scm`.
#[test]
fn plugin_dir_inside_plugin_body_is_the_plugins_own_directory() {
    use crate::ScriptingHost;
    use crate::null_host::CommandStoreHost;

    let dir = tempfile::TempDir::new().unwrap();
    let plugin_dir = dir.path().join("plugins").join("me").join("probe");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    let out = plugin_dir.join("seen-dir.txt");
    std::fs::write(
        plugin_dir.join("plugin.scm"),
        format!(
            "(let ((port (open-output-file {out:?})))\n\
             (write-string (plugin-dir) port)\n\
             (close-output-port port))",
            out = out.to_str().unwrap()
        ),
    )
    .unwrap();

    let mut host = ScriptingHost::new(&Dirs {
        data: Some(dir.path().to_path_buf()),
        ..Dirs::none()
    });
    let mut editor_host = CommandStoreHost::default();
    host.eval_source(r#"(load-plugin! "me/probe")"#, &mut editor_host)
        .expect("the probe plugin must load");

    let seen = std::fs::read_to_string(&out).unwrap();
    assert_eq!(std::path::Path::new(&seen), plugin_dir.as_path());
}

// ── No-trigger backstop (direct %declare-plugin! call) ────────────────────

/// A direct `%declare-plugin!` call with all three activation lists empty must
/// hard-error: an entry with no trigger could never activate.
#[test]
fn declare_plugin_bang_direct_no_trigger_call_errors() {
    use crate::{ScriptingHost, null_host::NullHost};
    let mut host = ScriptingHost::new(&Dirs::none());
    let result = host.eval_source(
        r#"(%declare-plugin! "user/direct-zero" "plugin.scm" '() '() '() '())"#,
        &mut NullHost,
    );
    let err = result.expect_err("direct %declare-plugin! with zero activation entries must error");
    assert!(
        err.contains("could never be activated"),
        "error must explain the plugin could never be activated; got: {err}"
    );
}

/// `load-plugin!` of a plugin whose directory doesn't exist at all is a soft
/// no-op: Info log, the plugin recorded for PLUM, no entry state.
///
/// Treating "not installed yet" like "installed but empty" would break the
/// load-then-:plum-install-plugins flow for every plugin that isn't installed
/// yet.
#[test]
fn load_plugin_absent_dir_soft_logs_and_records_declared_plugins() {
    use crate::{ScriptingHost, null_host::NullHost};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let mut host = ScriptingHost::new(&Dirs {
        data: Some(dir.path().to_path_buf()),
        ..Dirs::none()
    });

    let result = host.eval_source(
        r#"(load-plugin! "user/definitely-absent-mf")"#,
        &mut NullHost,
    );
    assert!(
        result.is_ok(),
        "load-plugin! of an absent directory must not error; got {result:?}"
    );

    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(level, msg)| {
            matches!(level, hume_core::Severity::Info) && msg.contains("not found on disk")
        }),
        "must log Info for an absent user/ plugin directory; messages: {messages:?}"
    );
    assert!(
        host.declared_plugins()
            .iter()
            .any(|d| d == "user/definitely-absent-mf"),
        "the plugin must be recorded for PLUM even though nothing was declared"
    );
    let id = EntryId::main(PluginId::parse("user/definitely-absent-mf").unwrap());
    assert!(
        !host.registries.lazy_registry.plugins.contains_key(&id),
        "no plugin state should be recorded when the directory is absent"
    );
}

/// `(declared-plugin-ref name)` after evaluating `src`, with an empty data dir.
fn declared_ref_after(src: &str, name: &str) -> SteelVal {
    use crate::{ScriptingHost, null_host::NullHost};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let mut host = ScriptingHost::new(&Dirs {
        data: Some(dir.path().to_path_buf()),
        ..Dirs::none()
    });
    host.eval_source(src, &mut NullHost)
        .unwrap_or_else(|e| panic!("{src} failed: {e}"));
    host.eval_source_for_value(&format!("(declared-plugin-ref \"{name}\")"), &mut NullHost)
        .unwrap_or_else(|e| panic!("declared-plugin-ref failed: {e}"))
}

#[test]
fn declared_plugin_ref_is_the_last_load_plugin_calls_ref() {
    assert_eq!(
        declared_ref_after(r#"(load-plugin! "user/p" #:ref "v1.0")"#, "user/p"),
        SteelVal::StringV("v1.0".into())
    );
    assert_eq!(
        declared_ref_after(
            r#"(load-plugin! "user/p" #:ref "v1.0") (load-plugin! "user/p" #:ref "stable")"#,
            "user/p"
        ),
        SteelVal::StringV("stable".into())
    );
    assert_eq!(
        declared_ref_after(r#"(load-plugin! "user/p" #:ref "release/1.x")"#, "user/p"),
        SteelVal::StringV("release/1.x".into())
    );
    assert_eq!(
        declared_ref_after(
            r#"(load-plugin! "user/p" #:ref "v1.0") (load-plugin! "user/p")"#,
            "user/p"
        ),
        SteelVal::BoolV(false)
    );
    assert_eq!(
        declared_ref_after(r#"(load-plugin! "user/p" #:ref "v1.0")"#, "user/other"),
        SteelVal::BoolV(false)
    );
}

#[test]
fn load_plugin_rejects_a_ref_git_cannot_take_or_a_core_plugin_cannot_use() {
    for src in [
        r#"(load-plugin! "user/p" #:ref 1)"#,
        r#"(load-plugin! "user/p" #:ref "")"#,
        r#"(load-plugin! "user/p" #:ref "-c")"#,
        r#"(load-plugin! "user/p" #:ref "v1 ")"#,
        r#"(load-plugin! "user/p" #:ref "a..b")"#,
        r#"(load-plugin! "user/p" #:ref "x:y")"#,
        r#"(load-plugin! "user/p" #:ref "v1^")"#,
        r#"(load-plugin! "user/p" #:ref "a\\b")"#,
        r#"(load-plugin! "user/p" #:ref "a@{1}")"#,
        r#"(load-plugin! "user/p" #:ref "@")"#,
        r#"(load-plugin! "user/p" #:ref "a.lock")"#,
        r#"(load-plugin! "user/p" #:ref "a/.b")"#,
        r#"(load-plugin! "user/p" #:ref "a//b")"#,
        r#"(load-plugin! "user/p" #:ref "a/")"#,
        r#"(load-plugin! "user/p" #:ref "a.")"#,
        r#"(load-plugin! "core:stdlib" #:ref "v1.0")"#,
    ] {
        let err = eval_err(src);
        assert!(
            err.contains("load-plugin!: #:ref"),
            "{src} must be rejected for its #:ref; got: {err}"
        );
    }
}

#[cfg(unix)]
mod unix;

#[cfg(unix)]
mod unix_entries;
