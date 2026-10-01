//! End-to-end: real manifest.scm.

use super::*;
use crate::editor::registry::{MappableCommand, TypedBody};
use hume_scripting::PluginStatus;

// ── End-to-end: real manifest.scm ─────────────────────────────────────────

/// The real `core:lsp` plugin's own shipped `manifest.scm` (not a synthetic
/// fixture) resolves and evaluates via a zero-trigger `(declare-plugin!
/// "core:lsp")`, through the full production `init_scripting` path against the
/// repo's actual `runtime/` tree.
///
/// A syntax error, a wrong plugin name, or a stale command list in the real
/// `runtime/plugins/core/lsp/manifest.scm` would fail this test even while
/// every synthetic-fixture test elsewhere in this file still passes.
#[test]
fn core_lsp_real_manifest_scm_resolves_via_zero_trigger_declare() {
    use crate::editor::Severity;
    use hume_scripting::attribution::PluginId;

    let runtime_dir = repo_runtime_dir();
    assert!(
        runtime_dir
            .join("plugins")
            .join("core")
            .join("lsp")
            .join("manifest.scm")
            .exists(),
        "sanity: the real manifest.scm must exist at the expected repo path"
    );

    let (ed, _dirs) = setup_editor_with_init_scripting(
        r#"(load-plugin! "core:stdlib")
           (register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))
           (declare-plugin! "core:lsp")"#,
        Some(&runtime_dir),
    );

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        errors.is_empty(),
        "init.scm with core:lsp's real manifest.scm must not log errors; got: {errors:?}"
    );

    let id = PluginId::Core("lsp".to_string());
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Declared)
        ),
        "core:lsp must be Declared (not yet activated) once the zero-trigger \
         declare resolves its manifest.scm"
    );
    assert!(
        matches!(
            ed.state
                .config
                .registry
                .get_typed("lsp-status")
                .map(|tc| &tc.body),
            Some(TypedBody::Lazy(_))
        ),
        "manifest.scm's #:typed-commands entries must be registered as Lazy stubs, \
         including \"lsp-status\""
    );
    assert!(
        !ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("some-made-up-language")
            .is_empty(),
        "manifest.scm's #:languages '(\"*\") must match any language, including an unregistered one"
    );
}

/// The real `core:lsp-install` plugin's own shipped `manifest.scm` resolves and
/// evaluates via a zero-trigger `(declare-plugin! "core:lsp-install")`, through
/// the full production `init_scripting` path against the repo's actual
/// `runtime/` tree.
#[test]
fn core_lsp_install_real_manifest_scm_resolves_via_zero_trigger_declare() {
    use crate::editor::Severity;
    use hume_scripting::attribution::PluginId;

    let runtime_dir = repo_runtime_dir();
    assert!(
        runtime_dir
            .join("plugins")
            .join("core")
            .join("lsp-install")
            .join("manifest.scm")
            .exists(),
        "sanity: the real manifest.scm must exist at the expected repo path"
    );

    let (ed, _dirs) = setup_editor_with_init_scripting(
        r#"(load-plugin! "core:stdlib")
           (declare-plugin! "core:lsp-install")"#,
        Some(&runtime_dir),
    );

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        errors.is_empty(),
        "init.scm with core:lsp-install's real manifest.scm must not log errors; got: {errors:?}"
    );

    let id = PluginId::Core("lsp-install".to_string());
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Declared)
        ),
        "core:lsp-install must be Declared (not yet activated) once the zero-trigger \
         declare resolves its manifest.scm"
    );
    assert!(
        matches!(
            ed.state
                .config
                .registry
                .get_typed("lsp-install")
                .map(|tc| &tc.body),
            Some(TypedBody::Lazy(_))
        ),
        "manifest.scm's #:typed-commands entries must be registered as Lazy stubs, \
         including \"lsp-install\""
    );
    assert!(
        !ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("some-made-up-language")
            .is_empty(),
        "manifest.scm's #:languages '(\"*\") must match any language, including an unregistered one"
    );
}

/// The real `core:stdlib` plugin's own shipped `manifest.scm` resolves and evaluates via a
/// zero-trigger `(declare-plugin! "core:stdlib")`, through the full production
/// `init_scripting` path against the repo's actual `runtime/` tree.
///
/// The synthetic-fixture tests in this file cannot see breakage in the real
/// `runtime/plugins/core/stdlib/manifest.scm` (a syntax error, a wrong plugin
/// name, a stale command list); this one can.
#[test]
fn core_stdlib_real_manifest_scm_resolves_via_zero_trigger_declare() {
    use crate::editor::Severity;
    use hume_scripting::attribution::PluginId;

    let runtime_dir = repo_runtime_dir();
    assert!(
        runtime_dir
            .join("plugins")
            .join("core")
            .join("stdlib")
            .join("manifest.scm")
            .exists(),
        "sanity: the real manifest.scm must exist at the expected repo path"
    );

    let (ed, _dirs) =
        setup_editor_with_init_scripting(r#"(declare-plugin! "core:stdlib")"#, Some(&runtime_dir));

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        errors.is_empty(),
        "init.scm with core:stdlib's real manifest.scm must not log errors; got: {errors:?}"
    );

    let id = PluginId::Core("stdlib".to_string());
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Declared)
        ),
        "core:stdlib must be Declared (not yet activated) once the zero-trigger \
         declare resolves its manifest.scm"
    );
    assert!(
        matches!(
            ed.state
                .config
                .registry
                .get_mappable("stdlib/all-single-char?"),
            Some(MappableCommand::Lazy { .. })
        ),
        "manifest.scm's #:commands entries must be registered as Lazy stubs, \
         including \"stdlib/all-single-char?\""
    );
}

/// A bare `(declare-plugin! "core:stdlib")` leaves it `Declared` (proven
/// above) with a live `Lazy` stub for every helper its own `manifest.scm`
/// exports. Loading `core:pickers` next must still succeed: its body-time
/// `call!` into `stdlib/config-boolean` hits that stub, and
/// `%dispatch-command!`'s lazy-miss retry
/// (`hume-scripting/src/builtins/bootstrap.scm`) inline-activates
/// `core:stdlib` to `Loaded` before the config read runs.
///
/// A dependency guard that checked `(loaded-plugins)` would reject this, since
/// `core:stdlib` is still `Declared` when `core:pickers`'s guard runs, and
/// `init_scripting` would log an error naming `core:stdlib` before
/// `core:pickers`'s config read ever ran. The guard checks
/// `(declared-plugins)` for that reason.
#[test]
fn declared_core_stdlib_serves_dependent_body_time_call() {
    use crate::editor::Severity;
    use hume_scripting::attribution::PluginId;

    let runtime_dir = repo_runtime_dir();

    let (ed, _dirs) = setup_editor_with_init_scripting(
        "(declare-plugin! \"core:stdlib\")\n(load-plugin! \"core:pickers\")",
        Some(&runtime_dir),
    );

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        errors.is_empty(),
        "declared (not loaded) core:stdlib must serve core:pickers's body-time \
         config read without error; got: {errors:?}"
    );

    let id_stdlib = PluginId::Core("stdlib".to_string());
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id_stdlib),
            Some(PluginStatus::Loaded)
        ),
        "core:stdlib must be inline-activated to Loaded by core:pickers's \
         body-time call!; staying Declared would mean the config read \
         either errored or silently reached an unactivated stub"
    );
    let id_pickers = PluginId::Core("pickers".to_string());
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id_pickers),
            Some(PluginStatus::Loaded)
        ),
        "core:pickers must itself be Loaded: its own eager load-plugin! completed"
    );
}

/// The real `core:plum` plugin's own shipped `manifest.scm` resolves and evaluates via a
/// zero-trigger `(declare-plugin! "core:plum")`, through the full production `init_scripting`
/// path against the repo's actual `runtime/` tree.
///
/// Only this test would notice a syntax error, a wrong plugin name, or a stale
/// command/language list in the real `runtime/plugins/core/plum/manifest.scm`;
/// the other tests here use synthetic fixtures.
#[test]
fn core_plum_real_manifest_scm_resolves_via_zero_trigger_declare() {
    use crate::editor::Severity;
    use hume_scripting::attribution::PluginId;

    let runtime_dir = repo_runtime_dir();
    assert!(
        runtime_dir
            .join("plugins")
            .join("core")
            .join("plum")
            .join("manifest.scm")
            .exists(),
        "sanity: the real manifest.scm must exist at the expected repo path"
    );

    let (ed, _dirs) =
        setup_editor_with_init_scripting(r#"(declare-plugin! "core:plum")"#, Some(&runtime_dir));

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        errors.is_empty(),
        "init.scm with core:plum's real manifest.scm must not log errors; got: {errors:?}"
    );

    let id = PluginId::Core("plum".to_string());
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Declared)
        ),
        "core:plum must be Declared (not yet activated) once the zero-trigger \
         declare resolves its manifest.scm"
    );
    assert!(
        matches!(
            ed.state
                .config
                .registry
                .get_typed("plum-list-plugins")
                .map(|tc| &tc.body),
            Some(TypedBody::Lazy(_))
        ),
        "manifest.scm's #:typed-commands entries must be registered as Lazy stubs, \
         including \"plum-list-plugins\""
    );
    assert!(
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("some-made-up-language")
            .is_empty(),
        "manifest.scm declares no #:languages: startup grammar registration is \
         core's job, so core:plum has no reason to activate on a language set"
    );
}

/// The scenario plugin-activation containment exists for: a third-party
/// `load-plugin!` fails, and `core:plum` (the user's only in-editor path to
/// update and fix a broken plugin) is declared afterward, on a later line of
/// the same `init.scm`. If the failed load re-raised and aborted `init.scm`,
/// `core:plum` and everything after the failing line would never be declared,
/// leaving no `:plum-update-plugins` to reach for short of hand-editing the
/// plugin on disk. The `with-handler` in `bootstrap.scm` does not re-raise,
/// which is what keeps `plum-update-plugins` registered here.
#[test]
fn failed_third_party_load_does_not_block_plum_declared_afterward() {
    use crate::editor::Severity;

    let _lock = TEST_GLOBALS.claim(Global::Env);

    let config_tmp = safe_tempdir();
    let data_tmp = safe_tempdir();
    let runtime_dir = repo_runtime_dir();

    let broken_plugin_dir = data_tmp
        .path()
        .join("hume")
        .join("plugins")
        .join("user")
        .join("broken");
    std::fs::create_dir_all(&broken_plugin_dir).unwrap();
    std::fs::write(
        broken_plugin_dir.join("plugin.scm"),
        "(call-does-not-exist)",
    )
    .unwrap();

    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(
        hume_config.join("init.scm"),
        "(load-plugin! \"core:stdlib\")\n\
         (load-plugin! \"user/broken\")\n\
         (declare-plugin! \"core:plum\")",
    )
    .unwrap();

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", config_tmp.path());
        std::env::set_var("HUME_RUNTIME", &runtime_dir);
        std::env::set_var("XDG_DATA_HOME", data_tmp.path());
    }

    let mut ed = editor_from("-[a]>b\n");
    ed.init_scripting(&mut Default::default());

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HUME_RUNTIME");
        std::env::remove_var("XDG_DATA_HOME");
    }

    // The failed load must still be reported, not silently swallowed.
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error && e.text.contains("user/broken")),
        "the failed plugin's error must still be reported; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );

    // core:plum, declared AFTER the failing line, must still have been
    // reached and resolved its manifest.scm. A failed plugin load must
    // not abort the rest of init.scm.
    assert!(
        matches!(
            ed.state
                .config
                .registry
                .get_typed("plum-update-plugins")
                .map(|tc| &tc.body),
            Some(TypedBody::Lazy(_))
        ),
        "core:plum's plum-update-plugins must be registered, proving init.scm \
         continued past the failed third-party plugin instead of aborting"
    );
}

/// The real `core:git-diff` plugin's own shipped `manifest.scm` resolves and evaluates via a
/// zero-trigger `(declare-plugin! "core:git-diff")`, through the full production
/// `init_scripting` path against the repo's actual `runtime/` tree.
///
/// It guards the real `runtime/plugins/core/git-diff/manifest.scm` against a
/// syntax error, a wrong plugin name, or a stale command/event list, none of
/// which the synthetic-fixture tests in this file would catch.
#[test]
fn core_git_diff_real_manifest_scm_resolves_via_zero_trigger_declare() {
    use crate::editor::Severity;
    use hume_scripting::attribution::PluginId;

    let runtime_dir = repo_runtime_dir();
    assert!(
        runtime_dir
            .join("plugins")
            .join("core")
            .join("git-diff")
            .join("manifest.scm")
            .exists(),
        "sanity: the real manifest.scm must exist at the expected repo path"
    );

    let (ed, _dirs) = setup_editor_with_init_scripting(
        r#"(declare-plugin! "core:git-diff")"#,
        Some(&runtime_dir),
    );

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        errors.is_empty(),
        "init.scm with core:git-diff's real manifest.scm must not log errors; got: {errors:?}"
    );

    let id = PluginId::Core("git-diff".to_string());
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Declared)
        ),
        "core:git-diff must be Declared (not yet activated) once the zero-trigger \
         declare resolves its manifest.scm"
    );
    assert!(
        matches!(
            ed.state
                .config
                .registry
                .get_typed("toggle-git-signs")
                .map(|tc| &tc.body),
            Some(TypedBody::Lazy(_))
        ),
        "manifest.scm's #:typed-commands entries must be registered as Lazy stubs, \
         including \"toggle-git-signs\""
    );
    assert!(
        matches!(
            ed.state
                .config
                .registry
                .get_typed("toggle-inline-diff")
                .map(|tc| &tc.body),
            Some(TypedBody::Lazy(_))
        ),
        "manifest.scm's #:typed-commands entries must be registered as Lazy stubs, \
         including \"toggle-inline-diff\""
    );
    assert!(
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_event_plugins("on-buffer-open")
            .contains(&id),
        "manifest.scm's #:events '(on-buffer-open) must register the plugin to \
         activate on the first buffer opened"
    );
}

/// Keymap lint is silent when every bound key targets a registered command.
#[test]
fn keymap_lint_silent_for_known_command() {
    use crate::editor::Severity;
    use crate::editor::keymap::BindMode;

    let (ed, _dirs) =
        setup_editor_with_init_scripting(r#"(bind-key! 'normal "Q" "move-down")"#, None);

    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| { e.severity == Severity::Warning && e.text.contains("move-down") }),
        "must not warn about known command 'move-down'; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
    // The lint reads all three tries at once, so it can't tell Normal from
    // Insert. This is what pins `Effect::BindKey`'s mode all the way through
    // `to_editor_bind_mode` into the right trie.
    assert!(
        ed.state
            .config
            .keymap
            .lookup_command(BindMode::Normal, &[key('Q')])
            .is_some(),
        "init.scm's bind-key! must land in the Normal trie specifically"
    );
}
