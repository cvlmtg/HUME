//! Lazy plugin loading: language/filetype activations, and
//! load-time activation reporting.

use super::*;
use hume_platform::dirs::Dirs;
use hume_scripting::PluginStatus;

// ── Lazy plugin loading — language/filetype activations ──────────────

/// `#:languages` plugin activates on first matching language set; its
/// `on-language-set` handler runs in the same call that caused activation.
///
/// If `set_buffer_language` skipped `activate_lazy_language_plugins`, the
/// plugin would stay `Declared` and the cursor would not move.
#[test]
fn language_trigger_activates_on_set() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:languages '("rust"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right" (focused-pane))))"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };

    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
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
    ed.set_buffer_language(bid, Some(lang)).expect_open();
    ed.settle();

    assert_ne!(
        state(&ed),
        before,
        "on-language-set handler must run and move cursor on first set"
    );
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
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
/// With `activation_languages` left uncleared on load, a second matching set
/// would attempt activation again. `activate_plugin`'s `Loaded` guard keeps
/// that harmless, so this test is about the intended fast path.
#[test]
fn language_trigger_idempotent_on_round_trip() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:languages '("rust"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right" (focused-pane))))"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    let bid = ed.focused_buffer_id();

    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang)).expect_open(); // first set: activates
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
    ed.set_buffer_language(bid, Some(lang)).expect_open(); // round-trip out
    ed.settle();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang)).expect_open(); // round-trip back: handler runs, no re-activation
    ed.settle();

    assert_ne!(
        state(&ed),
        after_first,
        "handler must run again on second rust set"
    );
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
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
#[test]
fn language_trigger_one_to_many_activates_all() {
    use hume_scripting::attribution::PluginId;

    let dir = tempfile::tempdir().unwrap();
    let dir_a = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::write(
        dir_a.join("plugin.scm"),
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right" (focused-pane))))"#,
    )
    .unwrap();
    let dir_b = dir.path().join("plugins").join("user").join("tp2");
    std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(
        dir_b.join("plugin.scm"),
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right" (focused-pane))))"#,
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(declare-plugin! \"user/tp\"  #:languages '(\"rust\"))\n\
         (declare-plugin! \"user/tp2\" #:languages '(\"rust\"))",
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b c d\n");
    let mut host = ScriptingHost::for_testing(&Dirs {
        data: Some(dir.path().to_path_buf()),
        ..Dirs::none()
    });
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih)
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
    ed.set_buffer_language(bid, Some(lang)).expect_open();

    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id_a.clone())),
            Some(PluginStatus::Loaded)
        ),
        "plugin A must be Loaded after language set"
    );
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id_b.clone())),
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
#[test]
fn language_trigger_does_not_fire_on_unrelated_language() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:languages '("rust"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right" (focused-pane))))"#,
    );
    let id = PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    let bid = ed.focused_buffer_id();

    let lang = ed.state.config.languages.intern("toml");
    ed.set_buffer_language(bid, Some(lang)).expect_open(); // unrelated language

    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
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

/// `#:languages '("*")` activates on ANY language set, not just an exact match:
/// the wildcard a manifest.scm uses because it can't enumerate every language a
/// user might want it for.
///
/// An exact-key-only lookup in `activation_language_plugins` would never match
/// the wildcard entry, and the plugin would stay `Declared` forever.
#[test]
fn language_wildcard_trigger_activates_on_any_language() {
    use hume_scripting::attribution::PluginId;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:languages '("*"))"#,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right" (focused-pane))))"#,
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
    ed.set_buffer_language(bid, Some(lang)).expect_open();
    ed.settle();

    assert_ne!(
        state(&ed),
        before,
        "on-language-set handler must run and move cursor on a wildcard-matched set"
    );
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
            Some(PluginStatus::Loaded)
        ),
        "plugin must be Loaded after a wildcard-matched language set"
    );
}

/// A wildcard entry and a specific-language entry coexist without cross-firing:
/// a set for a language only the wildcard plugin matches activates that plugin
/// alone, leaving the specific-language plugin `Declared`.
///
/// A union in `activation_language_plugins` that deduped wrongly or dropped the
/// specific-language map would activate the wrong plugin, or both.
#[test]
fn language_wildcard_and_specific_entry_coexist() {
    use hume_scripting::attribution::PluginId;

    let dir = tempfile::tempdir().unwrap();
    let dir_rust = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&dir_rust).unwrap();
    std::fs::write(dir_rust.join("plugin.scm"), "(+ 1 0)").unwrap();
    let dir_any = dir.path().join("plugins").join("user").join("tp2");
    std::fs::create_dir_all(&dir_any).unwrap();
    std::fs::write(dir_any.join("plugin.scm"), "(+ 1 0)").unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(declare-plugin! \"user/tp\"  #:languages '(\"rust\"))\n\
         (declare-plugin! \"user/tp2\" #:languages '(\"*\"))",
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b c d\n");
    let mut host = ScriptingHost::for_testing(&Dirs {
        data: Some(dir.path().to_path_buf()),
        ..Dirs::none()
    });
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih)
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
    ed.set_buffer_language(bid, Some(lang)).expect_open();

    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id_rust.clone())),
            Some(PluginStatus::Declared)
        ),
        "the \"rust\"-only plugin must stay Declared for an unrelated language"
    );
    assert!(
        matches!(
            ed.scripting
                .as_ref()
                .unwrap()
                .plugin_status(&hume_scripting::attribution::EntryId::main(id_any.clone())),
            Some(PluginStatus::Loaded)
        ),
        "the wildcard plugin must activate for any language, including \"toml\""
    );
}

// ── Load-time activation reporting ──────────────────────────

/// Command activation: first dispatch of a lazy command logs a Trace entry naming
/// the activating command. No such Trace exists before dispatch, so the entry
/// comes from the activation path and not from init.
#[test]
fn command_trigger_logs_trace_on_activation() {
    use hume_core::Severity;

    let (mut ed, _dir) = setup_lazy_editor(
        r#"(declare-plugin! "user/tp" #:typed-commands '("bar"))"#,
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

/// The editor `hume <args>` starts in a directory holding `files`, with a
/// `#:languages '("rust")` plugin whose body is `plugin`. The directories
/// live as long as the returned guards; the files' is the cwd.
fn start_with_rust_plugin(
    plugin: &str,
    files: &[(&str, &str)],
    args: &[&str],
) -> (Editor, (ReloadFixture, tempfile::TempDir)) {
    let fixture = ReloadFixture::new(
        r#"(%define-language! "rust" '("rs") '() '())
           (declare-plugin! "user/rusty" #:languages '("rust"))"#,
    );
    let plugin_dir = fixture.data_dir.join("plugins/user/rusty");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), plugin).unwrap();
    let file_tmp = tempfile::tempdir().unwrap();
    for (name, content) in files {
        std::fs::write(file_tmp.path().join(name), content).unwrap();
    }
    let cwd = AbsolutePath::canonicalize(file_tmp.path()).unwrap();
    let dirs = Dirs {
        config: Some(fixture.config_dir.clone()),
        data: Some(fixture.data_dir.clone()),
        ..Dirs::none()
    };
    let ed = start_headless(args, &cwd, dirs, crate::cli::ConfigSource::Default);
    (ed, (fixture, file_tmp))
}

/// A `#:languages` plugin that the file `hume` starts on activates may close
/// that file while activating: the editor starts on the scratch buffer the
/// close leaves.
#[test]
fn a_plugin_the_startup_file_activates_may_close_it() {
    let (mut ed, _dirs) = start_with_rust_plugin(
        "(close-buffer! (focused-pane))\n",
        &[("main.rs", "fn main() {}\n")],
        &["main.rs"],
    );
    let closes: Vec<_> = queued_events(&ed)
        .into_iter()
        .filter(|e| matches!(e, crate::editor::event::EditorEvent::OnBufferClose { .. }))
        .collect();
    assert!(
        closes.is_empty(),
        "the startup buffer never announced its open, so it announces no close: {closes:?}"
    );
    ed.settle();

    assert!(ed.doc().path().is_none(), "{:?}", ed.doc().path());
    assert_eq!(text(&ed), "\n");
}

/// A command-line file whose plugin closes it before the first settle drops
/// the position given for it.
#[test]
fn a_position_for_a_file_its_plugin_closes_is_dropped() {
    let (mut ed, _dirs) = start_with_rust_plugin(
        r#"(for-each (lambda (b) (when (equal? (buffer-name b) "b.rs") (close-buffer! b)))
                     (buffers))"#,
        &[("a.txt", "a\n"), ("b.rs", "fn b() {}\n")],
        &["a.txt", "b.rs:10"],
    );
    ed.settle();

    let names: Vec<_> = ed
        .state
        .buffers
        .iter()
        .map(|(_, b)| b.display_name())
        .collect();
    assert_eq!(names, ["a.txt"]);
}

/// The first command-line file's position goes to that file, not to the
/// buffer focused once startup is done, and is dropped when a plugin closed
/// the file.
#[test]
fn the_first_files_position_never_lands_on_another_buffer() {
    let (mut ed, _dirs) = start_with_rust_plugin(
        r#"(define main (focused-pane))
           (switch-to-buffer! main (open-buffer! "other.txt"))
           (close-buffer! main)"#,
        &[
            ("main.rs", "fn main() {}\n"),
            ("other.txt", "1\n2\n3\n4\n5\n6\n7\n"),
        ],
        &["main.rs:5"],
    );
    ed.settle();

    assert_eq!(ed.doc().display_name(), "other.txt");
    assert_eq!(
        ed.current_view().primary().head().offset(),
        hume_rope::offset::CharOffset::default()
    );
}

/// A `#:languages` plugin that edits the file `hume` starts on while
/// activating edits it on top of the file's saved undo history: `u` undoes
/// the plugin's edit, then the edit saved before.
#[test]
fn a_plugin_the_startup_file_activates_edits_it_after_its_history_is_restored() {
    use super::super::super::undo_persist::{Disk, editor_on};

    let disk = Disk::new("one\n");
    let mut first = editor_on(&disk, &disk.file, &["global undo-persist=true"]);
    type_text(&mut first, "a");
    type_cmd(&mut first, ":w");

    let fixture = ReloadFixture::new(
        r#"(%define-language! "notes" '("txt") '() '())
           (declare-plugin! "user/stamp" #:languages '("notes"))"#,
    );
    let plugin_dir = fixture.data_dir.join("plugins/user/stamp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(
        plugin_dir.join("plugin.scm"),
        r#"(edit-text! (focused-pane) (list (hash 'start-char 0 'end-char 0 'text "// ")))"#,
    )
    .unwrap();
    let cwd = AbsolutePath::canonicalize(disk.dir.path()).unwrap();
    let dirs = Dirs {
        config: Some(fixture.config_dir.clone()),
        data: Some(fixture.data_dir.clone()),
        state: Some(disk.state()),
        ..Dirs::none()
    };
    let mut ed = start_headless(&[&disk.file], &cwd, dirs, crate::cli::ConfigSource::Default);
    ed.settle();
    assert_eq!(text(&ed), "// aone\n", "sanity: the plugin's edit applies");

    ed.feed_key(key('u'));
    ed.feed_key(key('u'));
    assert_eq!(text(&ed), "one\n");
}
