//! Post-init keymap lint, and post-init
//! language-activation lint.

use super::*;
use crate::editor::registry::MappableCommand;
use hume_platform::dirs::Dirs;
use hume_scripting::PluginStatus;

// ── Post-init keymap lint ────────────────────────────────────

/// Keymap lint warns when a bind-key! targets a name not in the command registry.
///
/// A known command such as "move-down" gets no warning, so this one is about
/// the unknown name and is not a lint that fires unconditionally.
#[test]
fn keymap_lint_warns_on_unknown_command() {
    use crate::editor::Severity;

    let (ed, _dirs) =
        setup_editor_with_init_scripting(r#"(bind-key! 'normal "Q" "bogus-unknown-cmd")"#, None);

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| { e.severity == Severity::Warning && e.text.contains("bogus-unknown-cmd") }),
        "expected Warning about unknown command 'bogus-unknown-cmd'; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// A `bind-key!` targeting a real typed-only command's name (`:`-only, never
/// key-bindable) must warn with a hint naming the actual kind, not the bare
/// "unknown command" the lint gives a truly unregistered name: the same
/// kind confusion keypress dispatch guards against, here caught at init time
/// instead of silently waiting for the first press.
///
/// A name absent from the registry (`bogus-unknown-cmd` in
/// `keymap_lint_warns_on_unknown_command` above) keeps the generic message,
/// since there is no other kind to name.
#[test]
fn keymap_lint_warns_with_kind_hint_for_typed_only_command() {
    use crate::editor::Severity;

    let (ed, _dirs) = setup_editor_with_init_scripting(r#"(bind-key! 'normal "Q" "write")"#, None);

    assert!(
        ed.state.message_log.entries().any(|e| {
            e.severity == Severity::Warning
                && e.text
                    == "'write' is a typed command: run it as `:write`; it can't be bound to a key"
        }),
        "expected a kind-aware hint for 'write'; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// Native default keymaps must never bind a key to a command that isn't a Rust
/// built-in. Ctrl-Space's `completion-trigger` (`keymap/defaults.rs`) is
/// one (`registry/defaults/editor_cmds.rs`), so an editor that never loads
/// or declares any plugin must start up with no keymap-lint warning naming
/// it.
#[test]
fn no_keymap_lint_warning_for_completion_trigger_without_plugins() {
    use crate::editor::Severity;

    let (ed, _dirs) = setup_editor_with_init_scripting("", None);

    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| { e.severity == Severity::Warning && e.text.contains("completion-trigger") }),
        "no warning should name 'completion-trigger' when no plugin is loaded/declared; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// `(load-plugin! …)` called from a plugin body during *runtime* activation
/// (command activation) is rejected: registration verbs are top-level-only.
/// The parent plugin is marked `Failed` and an `Error` is logged.
#[test]
fn load_plugin_in_runtime_plugin_body_fails_fast() {
    use crate::editor::Severity;
    use hume_scripting::attribution::PluginId;

    let dir = tempfile::tempdir().unwrap();
    let tp_dir = dir.path().join("plugins").join("user").join("tp");
    let dep_dir = dir.path().join("plugins").join("user").join("dep");
    std::fs::create_dir_all(&tp_dir).unwrap();
    std::fs::create_dir_all(&dep_dir).unwrap();
    std::fs::write(
        tp_dir.join("plugin.scm"),
        // Plugin body calls (load-plugin!) at runtime: hard error expected.
        r#"(define-typed-command! "bar" "doc" (lambda () (+ 1 0)))
           (load-plugin! "user/dep")"#,
    )
    .unwrap();
    std::fs::write(dep_dir.join("plugin.scm"), r#"(+ 1 0)"#).unwrap();
    let init = dir.path().join("init.scm");
    std::fs::write(
        &init,
        r#"(declare-plugin! "user/tp" #:typed-commands '("bar"))"#,
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs {
        data: Some(dir.path().to_path_buf()),
        ..Dirs::none()
    });
    let init_path = dir.path().join("init.scm");
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":bar");

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
            Some(PluginStatus::Failed)
        ),
        "plugin must be Failed when body calls (load-plugin!) at runtime"
    );
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error),
        "error must be logged when (load-plugin!) is called from a runtime plugin body"
    );
}

/// `define-command!` with a built-in name is rejected first-wins.
///
/// Defining "move-right" in init.scm must leave the built-in intact and log
/// Severity::Error.  Without this check the Steel definition would silently
/// replace the built-in.
#[test]
fn define_command_collision_with_builtin_keeps_builtin() {
    use crate::editor::Severity;

    let (ed, _dirs) = setup_editor_with_init_scripting(
        r#"(define-command! "move-right" "redefine" (lambda () (+ 1 0)))"#,
        None,
    );

    // The built-in "move-right" must survive, not replaced by SteelBacked.
    assert!(
        !matches!(
            ed.state.config.registry.get_mappable("move-right"),
            Some(MappableCommand::SteelBacked { .. })
        ),
        "built-in move-right must not be replaced by Steel; got: {:?}",
        ed.state
            .config
            .registry
            .get_mappable("move-right")
            .map(|c| c.name())
    );
    // A Severity::Error must have been logged about the conflict.
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| { e.severity == Severity::Error && e.text.contains("move-right") }),
        "collision must produce an Error; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// A lazy plugin whose body contains a top-level `(call! "move-right" bid)` must
/// have that command executed when the plugin is activated at runtime (command
/// activation).  `activate_plugin_inline` runs with `session = EvalSession::Runtime`
/// so `%call-native!` dispatches synchronously via `run_command_sync`.
///
/// With `new_init` in place of `new_activation` in `activate_plugin_inline`, the
/// session would be `EvalSession::Init`, so `%call-native!` would warn and skip
/// and the cursor would stay put.
#[test]
fn lazy_plugin_call_bang_at_body_top_level_is_drained_on_runtime_activation() {
    // Plugin defines "trigger-me" (the command stub key) + calls move-right at
    // load time.  When "trigger-me" is dispatched, the plugin activates and the
    // body-level (call! "move-right" bid) should execute.
    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:typed-commands '("trigger-me"))"#,
        r#"(define-typed-command! "trigger-me" "doc" (lambda () (+ 1 0)))
           (call! "move-right" (focused-pane))"#,
    );
    let before = state(&ed);

    type_cmd(&mut ed, ":trigger-me");

    assert_ne!(
        state(&ed),
        before,
        "body-level (call! \"move-right\" bid) must execute when the plugin activates at runtime"
    );
}

// ── Post-init language-activation lint ─────────────────────────────────────────────

/// Language-activation lint warns when `#:languages` names a language that no
/// `define-language!` has registered.
#[test]
fn language_activation_lint_warns_on_unknown_language() {
    use crate::editor::Severity;

    let (ed, _dirs) =
        setup_lang_lint_editor(r#"(declare-plugin! "user/tp" #:languages '("rsut"))"#);

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| { e.severity == Severity::Warning && e.text.contains("rsut") }),
        "expected Warning about unknown language 'rsut'; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// Language-activation lint is silent when the declared language was registered via
/// `define-language!` earlier in the same init.scm.
///
/// The lint has to run after the second language flush. Run any earlier, it
/// would warn here, since `define-language!` would not yet have reached
/// `state.config.languages`.
#[test]
fn language_trigger_lint_silent_for_known_language() {
    use crate::editor::Severity;

    // %define-language! (the Rust primitive) works without prelude.scm
    // (the macro wrapper in languages.scm is absent in the test environment).
    let (ed, _dirs) = setup_lang_lint_editor(
        r#"(%define-language! "foo" '() '() '() #f '())
           (declare-plugin! "user/tp" #:languages '("foo"))"#,
    );

    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| { e.severity == Severity::Warning && e.text.contains("foo") }),
        "must not warn about known language 'foo'; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// Forward-reference order-independence: `declare-plugin! #:languages '("foo")`
/// appearing BEFORE `define-language! "foo"` in the same init.scm must not warn.
///
/// A declare-time check would see "foo" absent from the live registry and falsely
/// reject it.  The post-init placement (after every eval's effects are applied)
/// makes the check order-independent.
#[test]
fn language_trigger_lint_silent_for_forward_defined_language() {
    use crate::editor::Severity;

    // declare-plugin! BEFORE define-language!: the forward-reference case.
    let (ed, _dirs) = setup_lang_lint_editor(
        r#"(declare-plugin! "user/tp" #:languages '("foo"))
           (%define-language! "foo" '() '() '() #f '())"#,
    );

    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| { e.severity == Severity::Warning && e.text.contains("foo") }),
        "forward-defined language must not warn; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// Language-activation lint never warns about `"*"`: it's the any-language
/// wildcard, not a language identity to look up in the registry.
///
/// Without the `lang != "*"` guard, the lint would look "*" up in
/// `state.config.languages`, never find it, and warn on every startup for any
/// manifest.scm using the wildcard.
#[test]
fn language_activation_lint_silent_for_wildcard() {
    use crate::editor::Severity;

    let (ed, _dirs) = setup_lang_lint_editor(r#"(declare-plugin! "user/tp" #:languages '("*"))"#);

    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| { e.severity == Severity::Warning && e.text.contains('*') }),
        "must not warn about the \"*\" wildcard; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// A real core plugin with no `manifest.scm` of its own (`core:vim-keybind`)
/// loads eagerly under `load-plugin!`: only a manifest makes a plugin lazy.
#[test]
fn core_vim_keybind_without_a_manifest_loads_eagerly() {
    use hume_scripting::PluginStatus;
    use hume_scripting::attribution::{EntryId, PluginId};

    let runtime_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("hume-editor/ must have a parent (the repo root)")
        .join("runtime");
    assert!(
        !runtime_dir
            .join("plugins")
            .join("core")
            .join("vim-keybind")
            .join("manifest.scm")
            .exists(),
        "sanity: core:vim-keybind must NOT ship a manifest.scm for this check to be meaningful"
    );

    let (ed, _dirs) = setup_editor_with_init_scripting(
        "(load-plugin! \"core:stdlib\")\n(load-plugin! \"core:vim-keybind\")",
        Some(&runtime_dir),
    );

    let id = EntryId::main(PluginId::parse("core:vim-keybind").unwrap());
    assert_eq!(
        ed.scripting.as_ref().unwrap().plugin_status(&id),
        Some(PluginStatus::Loaded)
    );
}
