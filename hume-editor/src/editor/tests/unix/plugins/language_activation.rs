//! Phase 3b lazy plugin loading — language/filetype activations, and
//! Phase 4 Polish — load-time activation reporting.

use super::*;
use hume_scripting::PluginStatus;

// ── Phase 3b lazy plugin loading — language/filetype activations ──────────────

/// `#:languages` plugin activates on first matching language set; its
/// `on-language-set` handler runs in the same call that caused activation.
///
/// Flip: without `activate_lazy_language_plugins` in `set_buffer_language`,
/// the plugin stays `Declared` and the cursor never moves.
#[test]
fn language_trigger_activates_on_set() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:languages '("rust"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Declared)
        ),
        "plugin must be Declared before first language set"
    );
    assert!(
        !ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("rust")
            .is_empty(),
        "activation_languages must be populated before first set"
    );

    let before = state(&ed);
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle();

    assert_ne!(
        state(&ed),
        before,
        "on-language-set handler must run and move cursor on first set"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Loaded)
        ),
        "plugin must be Loaded after first language set"
    );
    assert!(
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("rust")
            .is_empty(),
        "activation_languages must be cleared after plugin loads"
    );
}

/// Second set to the same language: handler still runs; no re-activation.
///
/// Flip: if `activation_languages` were not cleared on load, a second matching set
/// would attempt activation again — `activate_plugin`'s `Loaded` guard prevents
/// a crash, but the test documents the intended fast path.
#[test]
fn language_trigger_idempotent_on_round_trip() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:languages '("rust"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    let bid = ed.focused_buffer_id();

    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang)); // first set — activates
    ed.settle();
    assert!(
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("rust")
            .is_empty(),
        "activation_languages must be empty after first set"
    );

    let after_first = state(&ed);
    let lang = ed.state.config.languages.intern("toml");
    ed.set_buffer_language(bid, Some(lang)); // round-trip out
    ed.settle();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang)); // round-trip back — handler runs, no re-activation
    ed.settle();

    assert_ne!(
        state(&ed),
        after_first,
        "handler must run again on second rust set"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Loaded)
        ),
        "plugin must remain Loaded after round-trip (not re-enter Declared or fail)"
    );
    assert!(
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("rust")
            .is_empty(),
        "activation_languages must remain cleared after round-trip"
    );
}

/// 1:many: two plugins both declare `#:languages '("rust")`; a single language
/// set activates both.
///
/// Flip: if only the first plugin in the activation Vec were activated, the second
/// would stay `Declared` with its handler never registering.
#[test]
fn language_trigger_one_to_many_activates_all() {
    use hume_scripting::attribution::PluginId;

    let dir = safe_tempdir();
    let dir_a = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::write(
        dir_a.join("plugin.scm"),
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    )
    .unwrap();
    let dir_b = dir.path().join("plugins").join("user").join("tp2");
    std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(
        dir_b.join("plugin.scm"),
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(declare-plugin \"user/tp\"  #:languages '(\"rust\"))\n\
         (declare-plugin \"user/tp2\" #:languages '(\"rust\"))",
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b c d\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");
    ed.scripting = Some(host);

    let id_a = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    let id_b = PluginId::User {
        user: "user".to_string(),
        repo: "tp2".to_string(),
    };
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id_a),
            Some(PluginStatus::Loaded)
        ),
        "plugin A must be Loaded after language set"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id_b),
            Some(PluginStatus::Loaded)
        ),
        "plugin B must be Loaded after language set"
    );
    assert!(
        ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("rust")
            .is_empty(),
        "activation_languages must be fully cleared after both plugins load"
    );
}

/// Language set for an unregistered language → plugin stays `Declared`.
///
/// Flip: if `activate_lazy_language_plugins` looked up the wrong map or iterated
/// unconditionally, the plugin would load on any language set.
#[test]
fn language_trigger_does_not_fire_on_unrelated_language() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:languages '("rust"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    let bid = ed.focused_buffer_id();

    let lang = ed.state.config.languages.intern("toml");
    ed.set_buffer_language(bid, Some(lang)); // unrelated language

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Declared)
        ),
        "plugin must stay Declared when an unrelated language is set"
    );
    assert!(
        !ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("rust")
            .is_empty(),
        "activation_languages[\"rust\"] must remain intact after an unrelated set"
    );
}

/// `#:languages '("*")` activates on ANY language set, not just an exact match —
/// the wildcard a manifest.scm uses because it can't enumerate every language a
/// user might want it for.
///
/// Flip: if `activation_language_plugins` only checked the exact key, the
/// wildcard entry would never fire and the plugin would stay `Declared` forever.
#[test]
fn language_wildcard_trigger_activates_on_any_language() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:languages '("*"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };

    assert!(
        !ed.scripting
            .as_ref()
            .unwrap()
            .activation_language_plugins("toml")
            .is_empty(),
        "the \"*\" entry must be returned for a language it never named explicitly"
    );

    let before = state(&ed);
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("toml");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle();

    assert_ne!(
        state(&ed),
        before,
        "on-language-set handler must run and move cursor on a wildcard-matched set"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id),
            Some(PluginStatus::Loaded)
        ),
        "plugin must be Loaded after a wildcard-matched language set"
    );
}

/// A wildcard entry and a specific-language entry coexist without cross-firing:
/// a set for a language only the wildcard plugin matches activates that plugin
/// alone, leaving the specific-language plugin `Declared`.
///
/// Flip: if the union in `activation_language_plugins` deduped incorrectly or
/// dropped the specific-language map, either the wrong plugin would activate or
/// both would.
#[test]
fn language_wildcard_and_specific_entry_coexist() {
    use hume_scripting::attribution::PluginId;

    let dir = safe_tempdir();
    let dir_rust = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&dir_rust).unwrap();
    std::fs::write(dir_rust.join("plugin.scm"), "(+ 1 0)").unwrap();
    let dir_any = dir.path().join("plugins").join("user").join("tp2");
    std::fs::create_dir_all(&dir_any).unwrap();
    std::fs::write(dir_any.join("plugin.scm"), "(+ 1 0)").unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(declare-plugin \"user/tp\"  #:languages '(\"rust\"))\n\
         (declare-plugin \"user/tp2\" #:languages '(\"*\"))",
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b c d\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");
    ed.scripting = Some(host);

    let id_rust = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    let id_any = PluginId::User {
        user: "user".to_string(),
        repo: "tp2".to_string(),
    };
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("toml");
    ed.set_buffer_language(bid, Some(lang));

    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id_rust),
            Some(PluginStatus::Declared)
        ),
        "the \"rust\"-only plugin must stay Declared for an unrelated language"
    );
    assert!(
        matches!(
            ed.scripting.as_ref().unwrap().plugin_status(&id_any),
            Some(PluginStatus::Loaded)
        ),
        "the wildcard plugin must activate for any language, including \"toml\""
    );
}

// ── Phase 4 Polish — load-time activation reporting ──────────────────────────

/// Command activation: first dispatch of a lazy command logs a Trace entry naming
/// the activating command.
///
/// Flip: before dispatch, no such Trace exists — confirming the entry is
/// produced by the activation path, not during init.
#[test]
fn command_trigger_logs_trace_on_activation() {
    use crate::editor::Severity;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin "user/tp" #:typed-commands '("bar"))"#,
        r#"(define-typed-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );

    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Trace && e.text.contains("by command")),
        "no activation Trace before dispatch; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );

    type_cmd(&mut ed, ":bar");

    assert!(
        ed.state.message_log.entries().any(|e| {
            e.severity == Severity::Trace && e.text.contains("bar") && e.text.contains("by command")
        }),
        "expected Trace entry naming command activation 'bar' after dispatch; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}
