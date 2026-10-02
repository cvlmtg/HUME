//! Unix-only tests for `declare-plugin! #:entry`, gated once at the
//! `mod unix_entries;` declaration in the parent.

use super::unix::{host_with_plugin, main_entry, secondary};
use crate::ScriptingHost;
use crate::attribution::{EntryId, PluginId};
use crate::host::EditorHost;
use crate::lazy::PluginState;
use crate::null_host::{LazyStubHost, NullHost};
use tempfile::TempDir;

fn state<'a>(host: &'a ScriptingHost, id: &EntryId) -> Option<&'a PluginState> {
    host.registries.lazy_registry.plugins.get(id)
}

const DECLARE_BOTH: &str = r#"
(declare-plugin! "user/multi" #:languages '("zlang"))
(declare-plugin! "user/multi" #:entry "extra.scm" #:typed-commands '("extra-cmd"))
"#;

#[test]
fn secondary_entry_is_declared_beside_the_main_entry() {
    let (_dir, mut host) = host_with_plugin("multi", &[("plugin.scm", ""), ("extra.scm", "")]);
    let mut editor_host = LazyStubHost::default();

    host.eval_source(DECLARE_BOTH, &mut editor_host).unwrap();

    assert!(matches!(
        state(&host, &main_entry("user/multi")),
        Some(PluginState::Declared { .. })
    ));
    assert!(matches!(
        state(&host, &secondary("user/multi", "extra.scm")),
        Some(PluginState::Declared { .. })
    ));
    assert_eq!(
        editor_host.commands().lazy_command_owner("extra-cmd"),
        Some(secondary("user/multi", "extra.scm")),
        "the stub must name the secondary entry as its owner"
    );
}

#[test]
fn activating_one_entry_leaves_the_other_declared() {
    let (_dir, mut host) = host_with_plugin(
        "multi",
        &[
            ("plugin.scm", r#"(log! 'info "main-ran")"#),
            ("extra.scm", r#"(log! 'info "extra-ran")"#),
        ],
    );
    host.eval_source(DECLARE_BOTH, &mut LazyStubHost::default())
        .unwrap();

    host.eval_source(
        r#"(%activate-plugin-inline! "user/multi" "extra.scm")"#,
        &mut NullHost,
    )
    .unwrap();

    assert!(matches!(
        state(&host, &secondary("user/multi", "extra.scm")),
        Some(PluginState::Loaded)
    ));
    assert!(matches!(
        state(&host, &main_entry("user/multi")),
        Some(PluginState::Declared { .. })
    ));
    let messages = host.peek_pending_messages();
    assert!(messages.iter().any(|(_, m)| m == "extra-ran"));
    assert!(!messages.iter().any(|(_, m)| m == "main-ran"));
}

#[test]
fn failed_secondary_entry_keeps_the_main_entrys_footprint() {
    let (_dir, mut host) = host_with_plugin(
        "multi",
        &[
            (
                "plugin.scm",
                r#"(define-command! "main-cmd" "doc" (lambda () 0))
               (register-hook! 'on-buffer-save (lambda (bid) 0))"#,
            ),
            (
                "extra.scm",
                r#"(define-command! "extra-cmd" "doc" (lambda () 0))
               (register-hook! 'on-buffer-open (lambda (bid) 0))
               (error "extra fails")"#,
            ),
        ],
    );
    host.eval_source(DECLARE_BOTH, &mut LazyStubHost::default())
        .unwrap();

    host.eval_source(
        r#"(%activate-plugin-inline! "user/multi" "plugin.scm")
           (%activate-plugin-inline! "user/multi" "extra.scm")"#,
        &mut NullHost,
    )
    .unwrap();

    assert!(matches!(
        state(&host, &main_entry("user/multi")),
        Some(PluginState::Loaded)
    ));
    assert!(matches!(
        state(&host, &secondary("user/multi", "extra.scm")),
        Some(PluginState::Failed)
    ));
    assert!(host.registries.command_table.contains_key("main-cmd"));
    assert!(!host.registries.command_table.contains_key("extra-cmd"));
    assert!(host.has_hook_handlers("on-buffer-save", None));
    assert!(!host.has_hook_handlers("on-buffer-open", None));
}

#[test]
fn redeclaring_an_entry_is_ignored_but_a_new_entry_is_added() {
    let (_dir, mut host) = host_with_plugin(
        "multi",
        &[("plugin.scm", ""), ("extra.scm", ""), ("more.scm", "")],
    );
    let mut editor_host = LazyStubHost::default();
    host.eval_source(DECLARE_BOTH, &mut editor_host).unwrap();

    host.eval_source(
        r#"(declare-plugin! "user/multi" #:entry "extra.scm" #:typed-commands '("other-cmd"))
           (declare-plugin! "user/multi" #:entry "more.scm" #:typed-commands '("more-cmd"))"#,
        &mut editor_host,
    )
    .unwrap();

    assert_eq!(
        editor_host.commands().lazy_command_owner("other-cmd"),
        None,
        "the second declaration of extra.scm must be ignored"
    );
    assert_eq!(
        editor_host.commands().lazy_command_owner("more-cmd"),
        Some(secondary("user/multi", "more.scm"))
    );
}

#[test]
fn declare_plugin_rejects_config_and_records_nothing() {
    let (_dir, mut host) = host_with_plugin("multi", &[("plugin.scm", ""), ("extra.scm", "")]);

    let err = host
        .eval_source(
            r#"(declare-plugin! "user/multi" #:entry "extra.scm"
                 #:typed-commands '("extra-cmd") #:config (hash "k" "v"))"#,
            &mut LazyStubHost::default(),
        )
        .expect_err("declare-plugin! must reject #:config");

    assert!(err.contains("#:config"), "got: {err}");
    assert!(state(&host, &secondary("user/multi", "extra.scm")).is_none());
    assert!(
        host.registries
            .plugin_records
            .config(&PluginId::parse("user/multi").unwrap())
            .is_none()
    );
}

#[test]
fn the_main_entrys_config_is_what_every_entry_reads() {
    let (_dir, mut host) = host_with_plugin(
        "multi",
        &[
            ("plugin.scm", ""),
            (
                "extra.scm",
                r#"(log! 'info (hash-ref (plugin-config) "k"))"#,
            ),
        ],
    );
    host.eval_source(
        r#"(declare-plugin! "user/multi" #:languages '("zlang"))
           (declare-plugin! "user/multi" #:entry "extra.scm" #:typed-commands '("extra-cmd"))
           (load-plugin! "user/multi" #:config (hash "k" "from-main"))"#,
        &mut LazyStubHost::default(),
    )
    .unwrap();

    host.eval_source(
        r#"(%activate-plugin-inline! "user/multi" "extra.scm")"#,
        &mut NullHost,
    )
    .unwrap();

    assert!(
        host.peek_pending_messages()
            .iter()
            .any(|(_, m)| m == "from-main")
    );
}

#[test]
fn invalid_entry_names_error() {
    let (_dir, mut host) = host_with_plugin("multi", &[("plugin.scm", "")]);

    for bad in ["../x.scm", "a/b.scm", "x.txt", ".scm"] {
        let src =
            format!(r#"(declare-plugin! "user/multi" #:entry "{bad}" #:typed-commands '("c"))"#);
        host.eval_source(&src, &mut LazyStubHost::default())
            .expect_err(&format!("{bad:?} must be rejected"));
    }
}

#[test]
fn a_secondary_entry_with_no_trigger_errors() {
    let (_dir, mut host) = host_with_plugin("multi", &[("plugin.scm", ""), ("extra.scm", "")]);

    let err = host
        .eval_source(
            r#"(declare-plugin! "user/multi" #:entry "extra.scm")"#,
            &mut LazyStubHost::default(),
        )
        .expect_err("an entry that can never activate must be rejected");

    assert!(err.contains("no activation entries"), "got: {err}");
}

#[test]
fn a_missing_entry_file_in_an_installed_plugin_errors() {
    let (_dir, mut host) = host_with_plugin("multi", &[("plugin.scm", "")]);

    let err = host
        .eval_source(
            r#"(declare-plugin! "user/multi" #:entry "gone.scm" #:typed-commands '("c"))"#,
            &mut LazyStubHost::default(),
        )
        .expect_err("a missing entry file must be a hard error");

    assert!(err.contains("gone.scm"), "got: {err}");
}

#[test]
fn load_plugin_rejects_an_entry() {
    let (_dir, mut host) = host_with_plugin("multi", &[("plugin.scm", ""), ("extra.scm", "")]);

    host.eval_source(
        r#"(load-plugin! "user/multi" #:entry "extra.scm")"#,
        &mut NullHost,
    )
    .expect_err("load-plugin! takes no #:entry");
}

#[test]
fn a_manifest_may_declare_several_entries() {
    let (_dir, mut host) = host_with_plugin(
        "multi",
        &[
            ("plugin.scm", ""),
            ("extra.scm", ""),
            (
                "manifest.scm",
                r#"(declare-plugin! "user/multi" #:languages '("zlang"))
               (declare-plugin! "user/multi" #:entry "extra.scm" #:typed-commands '("extra-cmd"))"#,
            ),
        ],
    );
    let mut editor_host = LazyStubHost::default();

    host.eval_source(r#"(load-plugin! "user/multi")"#, &mut editor_host)
        .unwrap();

    assert!(state(&host, &main_entry("user/multi")).is_some());
    assert!(state(&host, &secondary("user/multi", "extra.scm")).is_some());
}

#[test]
fn a_failed_manifest_fails_every_entry_it_declared() {
    let (_dir, mut host) = host_with_plugin(
        "multi",
        &[
            ("plugin.scm", ""),
            ("extra.scm", ""),
            (
                "manifest.scm",
                r#"(declare-plugin! "user/multi" #:languages '("zlang"))
               (declare-plugin! "user/multi" #:entry "extra.scm" #:typed-commands '("extra-cmd"))
               (error "manifest fails late")"#,
            ),
        ],
    );
    let mut editor_host = LazyStubHost::default();

    host.eval_source(r#"(load-plugin! "user/multi")"#, &mut editor_host)
        .unwrap();

    assert!(matches!(
        state(&host, &main_entry("user/multi")),
        Some(PluginState::Failed)
    ));
    assert!(matches!(
        state(&host, &secondary("user/multi", "extra.scm")),
        Some(PluginState::Failed)
    ));
    assert_eq!(editor_host.commands().lazy_command_owner("extra-cmd"), None);
}

#[test]
fn load_plugin_without_config_gives_the_body_an_empty_hash() {
    let (_dir, mut host) = host_with_plugin(
        "multi",
        &[
            (
                "manifest.scm",
                r#"(declare-plugin! "user/multi" #:languages '("zlang"))"#,
            ),
            (
                "plugin.scm",
                r#"(log! 'info (if (hash? (plugin-config)) "config-hash" "config-other"))"#,
            ),
        ],
    );
    host.eval_source(
        r#"(load-plugin! "user/multi")"#,
        &mut LazyStubHost::default(),
    )
    .unwrap();

    host.eval_source(
        r#"(%activate-plugin-inline! "user/multi" "plugin.scm")"#,
        &mut NullHost,
    )
    .unwrap();

    let messages = host.peek_pending_messages();
    assert!(
        messages.iter().any(|(_, m)| m == "config-hash"),
        "plugin-config must be a hash, got {messages:?}"
    );
}

#[test]
fn load_plugin_after_an_explicit_entry_declare_leaves_the_manifest_unread() {
    let (_dir, mut host) = host_with_plugin(
        "multi",
        &[
            (
                "manifest.scm",
                r#"(declare-plugin! "user/multi" #:languages '("zlang"))"#,
            ),
            ("plugin.scm", ""),
            ("extra.scm", ""),
        ],
    );
    host.eval_source(
        r#"(declare-plugin! "user/multi" #:entry "extra.scm" #:typed-commands '("extra-cmd"))
           (load-plugin! "user/multi")"#,
        &mut LazyStubHost::default(),
    )
    .unwrap();

    assert!(
        state(&host, &main_entry("user/multi")).is_none(),
        "the first declaration of a plugin wins, so the manifest must not run"
    );
    assert!(matches!(
        state(&host, &secondary("user/multi", "extra.scm")),
        Some(PluginState::Declared { .. })
    ));
}

#[test]
fn declaring_a_missing_entry_file_records_nothing() {
    let (_dir, mut host) = host_with_plugin("multi", &[("plugin.scm", "")]);

    let result = host.eval_source(
        r#"(declare-plugin! "user/multi" #:entry "gone.scm" #:typed-commands '("gone-cmd"))"#,
        &mut LazyStubHost::default(),
    );

    assert!(result.is_err(), "a missing entry file must be a hard error");
    assert!(
        host.declared_plugins().is_empty(),
        "a failed declare must not be listed for PLUM: {:?}",
        host.declared_plugins()
    );
}

#[test]
fn main_entry_spelled_in_any_case_is_still_the_main_entry() {
    let (_dir, mut host) = host_with_plugin("multi", &[("plugin.scm", "")]);

    host.eval_source(
        r#"(declare-plugin! "user/multi" #:entry "Plugin.scm" #:languages '("zlang"))"#,
        &mut LazyStubHost::default(),
    )
    .unwrap();
    assert!(matches!(
        state(&host, &main_entry("user/multi")),
        Some(PluginState::Declared { .. })
    ));

    let no_triggers = host.eval_source(
        r#"(declare-plugin! "user/other" #:entry "Plugin.scm")"#,
        &mut LazyStubHost::default(),
    );
    let err = format!("{:?}", no_triggers.unwrap_err());
    assert!(err.contains("declares no activation entries"), "{err}");
}

// ── load-plugin! decides lazy vs eager ────────────────────────────────────

const LAZY_MANIFEST: &str = r#"(declare-plugin! "user/multi" #:typed-commands '("lazy-cmd"))"#;

#[test]
fn load_plugin_of_a_manifest_plugin_stays_lazy() {
    let (_dir, mut host) = host_with_plugin(
        "multi",
        &[
            ("manifest.scm", LAZY_MANIFEST),
            ("plugin.scm", r#"(log! 'info "body-ran")"#),
        ],
    );
    let mut editor_host = LazyStubHost::default();

    host.eval_source(r#"(load-plugin! "user/multi")"#, &mut editor_host)
        .unwrap();

    assert!(matches!(
        state(&host, &main_entry("user/multi")),
        Some(PluginState::Declared { .. })
    ));
    assert_eq!(
        editor_host.commands().lazy_command_owner("lazy-cmd"),
        Some(main_entry("user/multi"))
    );
    assert!(
        !host
            .peek_pending_messages()
            .iter()
            .any(|(_, m)| m == "body-ran"),
        "the body must wait for a trigger"
    );
}

#[test]
fn load_plugin_after_a_declare_keeps_the_declared_triggers_and_skips_the_manifest() {
    let (_dir, mut host) = host_with_plugin(
        "multi",
        &[
            ("manifest.scm", LAZY_MANIFEST),
            ("plugin.scm", r#"(log! 'info "body-ran")"#),
        ],
    );
    let mut editor_host = LazyStubHost::default();

    host.eval_source(
        r#"(declare-plugin! "user/multi" #:typed-commands '("mine"))
           (load-plugin! "user/multi" #:config (hash "k" "v"))"#,
        &mut editor_host,
    )
    .unwrap();

    assert_eq!(
        editor_host.commands().lazy_command_owner("mine"),
        Some(main_entry("user/multi"))
    );
    assert_eq!(
        editor_host.commands().lazy_command_owner("lazy-cmd"),
        None,
        "the manifest must not run for a plugin the user already declared"
    );
    assert!(matches!(
        state(&host, &main_entry("user/multi")),
        Some(PluginState::Declared { .. })
    ));
}

#[test]
fn load_plugin_inside_a_manifest_errors_and_is_contained() {
    let (_dir, mut host) = host_with_plugin(
        "multi",
        &[
            ("manifest.scm", r#"(load-plugin! "user/multi")"#),
            ("plugin.scm", ""),
        ],
    );

    host.eval_source(r#"(load-plugin! "user/multi")"#, &mut NullHost)
        .unwrap();

    assert!(host.peek_pending_messages().iter().any(|(level, m)| {
        matches!(level, crate::log::LogLevel::Error)
            && m.contains("cannot be called from a manifest.scm")
    }));
}

#[test]
fn load_plugin_rejects_a_local_path() {
    let (_dir, mut host) = host_with_plugin("multi", &[("plugin.scm", "")]);

    let err = host
        .eval_source(r#"(load-plugin! "./my.scm")"#, &mut NullHost)
        .expect_err("a local file is declared, not loaded");

    assert!(err.contains("local file"), "got: {err}");
}

// ── local plugins: one .scm file beside init.scm ──────────────────────────

/// A host that has evaluated an `init.scm` holding `init_src`, with `files`
/// (relative path, source) written beside it.
fn init_host(
    files: &[(&str, &str)],
    init_src: &str,
    editor_host: &mut dyn EditorHost,
) -> (TempDir, ScriptingHost, Result<(), String>) {
    let dir = TempDir::new().unwrap();
    for (name, src) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, src).unwrap();
    }
    let init = dir.path().join("init.scm");
    std::fs::write(&init, init_src).unwrap();
    let mut host = ScriptingHost::new();
    let result = host
        .eval_init(&init, 10_000, editor_host, Default::default())
        .map(drop)
        .map_err(|e| e.to_string());
    (dir, host, result)
}

fn local(path: &str) -> EntryId {
    EntryId::main(PluginId::parse(path).unwrap())
}

#[test]
fn a_local_declare_resolves_beside_init_and_activates_on_its_trigger() {
    let mut editor_host = LazyStubHost::default();
    let (_dir, mut host, result) = init_host(
        &[("my.scm", r#"(log! 'info "local-ran")"#)],
        r#"(declare-plugin! "./my.scm" #:typed-commands '("local-cmd"))"#,
        &mut editor_host,
    );
    result.unwrap();

    assert_eq!(
        editor_host.commands().lazy_command_owner("local-cmd"),
        Some(local("./my.scm"))
    );

    host.eval_source(r#"(%activate-plugin-inline! "./my.scm" #f)"#, &mut NullHost)
        .unwrap();

    assert!(matches!(
        state(&host, &local("./my.scm")),
        Some(PluginState::Loaded)
    ));
    assert!(
        host.peek_pending_messages()
            .iter()
            .any(|(_, m)| m == "local-ran")
    );
}

#[test]
fn a_local_body_sees_its_own_directory_and_an_empty_config() {
    let mut editor_host = LazyStubHost::default();
    let (dir, mut host, result) = init_host(
        &[(
            "sub/x.scm",
            r#"(log! 'info (plugin-dir))
               (log! 'info (if (hash? (plugin-config)) "empty-hash" "other"))"#,
        )],
        r#"(declare-plugin! "./sub/x.scm" #:typed-commands '("x-cmd"))"#,
        &mut editor_host,
    );
    result.unwrap();

    host.eval_source(
        r#"(%activate-plugin-inline! "./sub/x.scm" #f)"#,
        &mut NullHost,
    )
    .unwrap();

    let messages: Vec<&str> = host
        .peek_pending_messages()
        .iter()
        .map(|(_, m)| m.as_str())
        .collect();
    assert!(
        messages.contains(&dir.path().join("sub").to_string_lossy().as_ref()),
        "got: {messages:?}"
    );
    assert!(messages.contains(&"empty-hash"), "got: {messages:?}");
}

#[test]
fn a_local_plugin_is_loaded_but_never_listed_for_plum() {
    let mut editor_host = LazyStubHost::default();
    let (_dir, mut host, result) = init_host(
        &[(
            "my.scm",
            r#"(log! 'info (if (member "./my.scm" (loaded-plugins)) "listed" "unlisted"))"#,
        )],
        r#"(declare-plugin! "./my.scm" #:typed-commands '("local-cmd"))"#,
        &mut editor_host,
    );
    result.unwrap();
    assert!(host.declared_plugins().is_empty());

    host.eval_source(
        r#"(%activate-plugin-inline! "./my.scm" #f)
           (log! 'info (if (member "./my.scm" (loaded-plugins)) "listed" "unlisted"))"#,
        &mut NullHost,
    )
    .unwrap();

    assert!(
        host.peek_pending_messages()
            .iter()
            .any(|(_, m)| m == "listed")
    );
    assert!(host.declared_plugins().is_empty());
}

#[test]
fn a_missing_local_file_errors() {
    let (_dir, _host, result) = init_host(
        &[],
        r#"(declare-plugin! "./nope.scm" #:typed-commands '("c"))"#,
        &mut LazyStubHost::default(),
    );

    let err = result.expect_err("a missing local file must error");
    assert!(err.contains("not found beside init.scm"), "got: {err}");
}

#[test]
fn a_local_declare_takes_no_entry() {
    let (_dir, _host, result) = init_host(
        &[("my.scm", "")],
        r#"(declare-plugin! "./my.scm" #:entry "x.scm" #:typed-commands '("c"))"#,
        &mut LazyStubHost::default(),
    );

    let err = result.expect_err("#:entry makes no sense for a single file");
    assert!(err.contains("single file"), "got: {err}");
}

#[test]
fn a_local_declare_without_an_init_file_errors() {
    let mut host = ScriptingHost::new();

    let err = host
        .eval_source(
            r#"(declare-plugin! "./my.scm" #:typed-commands '("c"))"#,
            &mut NullHost,
        )
        .expect_err("a local file needs an init.scm to resolve against");

    assert!(
        err.contains("can only be declared from init.scm"),
        "got: {err}"
    );
}

#[test]
fn a_runtime_file_does_not_set_the_local_plugin_directory() {
    let dir = TempDir::new().unwrap();
    let runtime = dir.path().join("rt.scm");
    std::fs::write(
        &runtime,
        r#"(declare-plugin! "./x.scm" #:typed-commands '("c"))"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("x.scm"), "").unwrap();
    let mut host = ScriptingHost::new();

    let err = host
        .eval_runtime(&runtime, 10_000, &mut NullHost, Default::default())
        .expect_err("a runtime file has no init.scm to resolve a local plugin against")
        .to_string();

    assert!(
        err.contains("can only be declared from init.scm"),
        "got: {err}"
    );
}
