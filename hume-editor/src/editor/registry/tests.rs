use super::*;

use hume_treesitter::textobjects::ObjectKind;

/// Exhaustiveness guard: if a command is added without a registry entry,
/// this test catches it.
const EXPECTED_COMMAND_COUNT: usize = 200;

/// `STRUCTURAL_OBJECTS` is the only link between `ObjectKind` and the four
/// commands (plus the `m i`/`m a` key) each kind ships. Nothing in the type
/// system ties the two together: a variant added to `ObjectKind` but not to
/// the table still compiles, still collects spans, and silently ships zero
/// commands and zero keybindings, a quieter failure than the out-of-bounds
/// panic `object_enum!`'s own generated `ALL` exists to prevent.
///
/// A missing row makes this fail with the orphaned kind's name, where
/// `registry_has_expected_count` alone would only report a number.
#[test]
fn structural_objects_cover_every_object_kind() {
    for kind in ObjectKind::ALL {
        let rows = STRUCTURAL_OBJECTS
            .iter()
            .filter(|o| o.kind == *kind)
            .count();
        assert_eq!(
            rows, 1,
            "ObjectKind::{kind:?} must have exactly one STRUCTURAL_OBJECTS row, found {rows}"
        );
    }

    // The `m i`/`m a` third-level keys are bound from this same table, so a
    // duplicate key would silently shadow one kind's text objects.
    let mut keys: Vec<char> = STRUCTURAL_OBJECTS.iter().map(|o| o.key).collect();
    keys.sort_unstable();
    let unique = keys.len();
    keys.dedup();
    assert_eq!(keys.len(), unique, "duplicate `m i`/`m a` key in {keys:?}");
}

#[test]
fn registry_has_expected_count() {
    let reg = CommandRegistry::with_defaults();
    assert_eq!(
        reg.len(),
        EXPECTED_COMMAND_COUNT,
        "registered command count mismatch: did you add a command without registering it?"
    );
}

#[test]
fn mappable_lookup_by_name_works() {
    let reg = CommandRegistry::with_defaults();

    // Motion
    let cmd = reg
        .get_mappable("move-right")
        .expect("move-right should be registered");
    assert_eq!(cmd.name().as_ref(), "move-right");
    assert!(matches!(
        cmd,
        MappableCommand::Native(NativeCommand::Motion { .. })
    ));

    // Selection
    let cmd = reg
        .get_mappable("collapse-selection")
        .expect("collapse-selection should be registered");
    assert_eq!(cmd.name().as_ref(), "collapse-selection");
    assert!(matches!(
        cmd,
        MappableCommand::Native(NativeCommand::Selection { .. })
    ));

    // Edit
    let cmd = reg
        .get_mappable("delete-selection")
        .expect("delete-selection should be registered");
    assert_eq!(cmd.name().as_ref(), "delete-selection");
    assert!(matches!(
        cmd,
        MappableCommand::Native(NativeCommand::Edit { .. })
    ));

    // EditorCmd
    let cmd = reg.get_mappable("undo").expect("undo should be registered");
    assert_eq!(cmd.name().as_ref(), "undo");
    assert!(matches!(
        cmd,
        MappableCommand::Native(NativeCommand::EditorCmd { .. })
    ));

    let cmd = reg
        .get_mappable("find-forward")
        .expect("find-forward should be registered");
    assert!(matches!(
        cmd,
        MappableCommand::Native(NativeCommand::EditorCmd { .. })
    ));

    let cmd = reg
        .get_mappable("delete")
        .expect("delete should be registered");
    assert!(matches!(
        cmd,
        MappableCommand::Native(NativeCommand::EditorCmd { .. })
    ));
}

#[test]
fn typed_lookup_by_canonical_name() {
    let reg = CommandRegistry::with_defaults();
    let tc = reg
        .get_typed("write")
        .expect("write should be a typed command");
    assert_eq!(tc.name, "write");
    assert!(!tc.doc.is_empty());
}

#[test]
fn typed_lookup_by_alias() {
    let reg = CommandRegistry::with_defaults();
    assert_eq!(reg.get_typed("w").expect("w alias").name, "write");
    assert_eq!(reg.get_typed("q").expect("q alias").name, "quit");
    assert_eq!(reg.get_typed("wq").expect("wq alias").name, "write-quit");
    assert_eq!(
        reg.get_typed("wrap").expect("wrap alias").name,
        "toggle-soft-wrap"
    );
}

#[test]
fn typed_lookup_does_not_return_mappable() {
    let reg = CommandRegistry::with_defaults();
    // Mappable commands are not accessible via get_typed.
    assert!(reg.get_typed("move-right").is_none());
    assert!(reg.get_typed("clear-search").is_none());
    assert!(reg.get_typed("select-all-matches").is_none());
}

#[test]
fn mappable_lookup_does_not_return_typed() {
    let reg = CommandRegistry::with_defaults();
    // Typed commands are not accessible via get_mappable.
    assert!(reg.get_mappable("write").is_none());
    assert!(reg.get_mappable("quit").is_none());
    assert!(reg.get_mappable("write-quit").is_none());
}

#[test]
fn unknown_name_returns_none() {
    let reg = CommandRegistry::with_defaults();
    assert!(reg.get_mappable("does-not-exist").is_none());
    assert!(reg.get_typed("does-not-exist").is_none());
}

#[test]
fn doc_strings_are_stored_and_accessible() {
    let reg = CommandRegistry::with_defaults();
    let cmd = reg.get_mappable("move-right").unwrap();
    assert!(
        !cmd.doc().is_empty(),
        "move-right should have a non-empty doc string"
    );
    let cmd = reg.get_mappable("delete-selection").unwrap();
    assert!(
        !cmd.doc().is_empty(),
        "delete-selection should have a non-empty doc string"
    );
    let tc = reg.get_typed("write").unwrap();
    assert!(
        !tc.doc.is_empty(),
        "write should have a non-empty doc string"
    );
}

#[test]
fn is_extendable_motion_and_selection_always_true() {
    let reg = CommandRegistry::with_defaults();
    // All Motion commands are extendable.
    for name in [
        "move-right",
        "move-left",
        "move-down",
        "move-up",
        "goto-first-line",
        "goto-last-line",
        "goto-line-start",
        "goto-line-end",
        "goto-first-nonblank",
        "goto-matching-pair",
        "select-next-word",
        "select-prev-word",
        "goto-next-paragraph",
        "goto-prev-paragraph",
    ] {
        let cmd = reg
            .get_mappable(name)
            .unwrap_or_else(|| panic!("{name} not found"));
        assert!(cmd.is_extendable(), "Motion '{name}' should be extendable");
    }
    // All Selection commands are extendable.
    for name in [
        "select-line",
        "select-line-backward",
        "collapse-selection",
        "flip-selections",
        "inner-word",
        "around-word",
        "inner-paren",
        "around-paren",
    ] {
        let cmd = reg
            .get_mappable(name)
            .unwrap_or_else(|| panic!("{name} not found"));
        assert!(
            cmd.is_extendable(),
            "Selection '{name}' should be extendable"
        );
    }
}

#[test]
fn is_extendable_editor_cmd_true_for_extendable() {
    let reg = CommandRegistry::with_defaults();
    // EditorCmds marked extendable: true.
    for name in [
        "find-forward",
        "find-backward",
        "till-forward",
        "till-backward",
        "repeat-find-forward",
        "repeat-find-backward",
        "page-down",
        "page-up",
        "half-page-down",
        "half-page-up",
        "search-next",
        "search-prev",
        "search-forward",
        "search-backward",
        "move-down",
        "move-up",
    ] {
        let cmd = reg
            .get_mappable(name)
            .unwrap_or_else(|| panic!("{name} not found"));
        assert!(
            cmd.is_extendable(),
            "EditorCmd '{name}' should be extendable"
        );
    }
}

#[test]
fn is_extendable_false_for_edits_and_non_extendable_editor_cmds() {
    let reg = CommandRegistry::with_defaults();
    // Edit commands are never extendable.
    for name in [
        "delete-selection",
        "delete-char-forward",
        "delete-char-backward",
    ] {
        let cmd = reg
            .get_mappable(name)
            .unwrap_or_else(|| panic!("{name} not found"));
        assert!(
            !cmd.is_extendable(),
            "Edit '{name}' should not be extendable"
        );
    }
    // Non-extendable EditorCmds.
    for name in [
        "undo",
        "redo",
        "insert-before",
        "insert-after",
        "open-line-below",
        "open-line-above",
        "exit-insert",
    ] {
        let cmd = reg
            .get_mappable(name)
            .unwrap_or_else(|| panic!("{name} not found"));
        assert!(
            !cmd.is_extendable(),
            "EditorCmd '{name}' should not be extendable"
        );
    }
}

#[test]
fn is_extendable_steel_backed_always_true() {
    // All Steel commands participate in Ctrl-key one-shot extend; the
    // body receives `extend` as a lambda arg and decides what to do.
    let cmd = MappableCommand::SteelBacked(SteelCommand {
        name: "x".into(),
        doc: "".into(),
        body: hume_scripting::SteelCmdBody {
            arity: 0,
            is_variadic: false,
            inline_output: false,
            repeatable: false,
            proc: hume_scripting::MappableProc::for_test(),
        },
    });
    assert!(cmd.is_extendable());
}

#[test]
fn is_extendable_lazy_stub_true() {
    use hume_scripting::attribution::PluginId;
    // Lazy stubs resolve to SteelBacked; the keymap must treat them as
    // extendable on first Ctrl-key press so extend=true is forwarded even
    // before the plugin is activated.
    let cmd = MappableCommand::Lazy {
        name: "lazy-cmd".into(),
        plugin: hume_scripting::attribution::EntryId::main(PluginId::User {
            user: "u".to_string(),
            repo: "r".to_string(),
        }),
    };
    assert!(cmd.is_extendable());
}

#[test]
fn all_names_are_unique() {
    // HashMap insertion silently overwrites duplicates, so verify the final
    // count matches the number of distinct registered names.
    let reg = CommandRegistry::with_defaults();
    let unique: std::collections::HashSet<&str> = reg.names().collect();
    assert_eq!(unique.len(), reg.len(), "duplicate command names detected");
}

#[test]
fn runtime_register_and_lookup() {
    let mut reg = CommandRegistry::with_defaults();
    let before = reg.len();

    fn dummy_fn(
        _state: &mut crate::editor::EditorState,
        _view: &mut hume_engine::pipeline::EngineView,
        _t: crate::editor::commands::CommandPane,
        _count: crate::editor::registry::Count,
        _mode: hume_ops::MotionMode,
    ) -> Result<(), crate::editor::error::CommandError> {
        Ok(())
    }
    let cmd = MappableCommand::Native(NativeCommand::EditorCmd {
        name: Cow::Owned("steel-test-cmd".to_string()),
        doc: Cow::Borrowed("A dummy Steel command for testing."),
        fun: crate::editor::commands::NativeBody::new(EditorCmdBody::Pane(dummy_fn)),
        defers_paste_commit: false,
        repeatable: false,
        jump: false,
        visual_move: false,
        extendable: false,
        clears_extend: false,
        selection_tracking: SelectionTracking::Untracked,
        manages_own_session: false,
        key_only: false,
    });
    reg.register(cmd).expect("a lowercase editor command name");

    assert_eq!(reg.len(), before + 1);
    assert!(reg.get_mappable("steel-test-cmd").is_some());
    assert_eq!(
        reg.get_mappable("steel-test-cmd").unwrap().name().as_ref(),
        "steel-test-cmd"
    );
}

#[test]
fn mappable_commands_not_shadowed_by_typed() {
    // Mappable commands like clear-search and select-all-matches must remain
    // accessible as mappable so keybinds continue to work, and, since `:`
    // resolves only typed commands, neither is reachable from the command
    // line at all (see registry/mod.rs's module doc).
    let reg = CommandRegistry::with_defaults();
    assert!(reg.get_mappable("clear-search").is_some());
    assert!(reg.get_mappable("select-all-matches").is_some());
    assert!(reg.get_typed("clear-search").is_none());
    assert!(reg.get_typed("select-all-matches").is_none());
}

fn dummy_typed(
    _ed: &mut crate::editor::Editor,
    _fp: crate::editor::commands::FocusedPane,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), crate::editor::error::CommandError> {
    Ok(())
}

fn typed_cmd(name: &str, aliases: &'static [&'static str], body: TypedBody) -> TypedCommand {
    TypedCommand {
        name: Cow::Owned(name.to_string()),
        doc: Cow::Borrowed(""),
        aliases,
        body,
        completer: None,
        arg_expansion: ArgExpansion::Paths,
    }
}

fn test_plugin() -> hume_scripting::attribution::EntryId {
    hume_scripting::attribution::EntryId::main(
        hume_scripting::attribution::PluginId::parse("core:test").expect("a plugin id"),
    )
}

#[test]
fn runtime_register_typed_and_lookup() {
    let mut reg = CommandRegistry::with_defaults();
    let before = reg.len();

    reg.register_typed(TypedCommand {
        name: Cow::Owned("steel-typed-cmd".to_string()),
        doc: Cow::Borrowed("A dummy Steel typed command for testing."),
        aliases: &["stc"],
        body: TypedBody::Native(dummy_typed),
        completer: None,
        arg_expansion: ArgExpansion::Paths,
    })
    .expect("a valid typed command name");

    assert_eq!(reg.len(), before + 1);
    // Reachable by canonical name.
    assert_eq!(
        reg.get_typed("steel-typed-cmd").unwrap().name,
        "steel-typed-cmd"
    );
    // Reachable by alias.
    assert_eq!(reg.get_typed("stc").unwrap().name, "steel-typed-cmd");
    // Not reachable as a mappable command.
    assert!(reg.get_mappable("steel-typed-cmd").is_none());
}

/// A typed command whose name or alias the `:` line can't read back whole
/// is refused and leaves the registry unchanged.
#[test]
fn register_typed_rejects_a_name_or_alias_the_colon_line_cannot_reach() {
    let cases: [(&str, &'static [&'static str]); 2] = [("b7", &[]), ("unreachable-alias", &["b7"])];
    for (name, aliases) in cases {
        let mut reg = CommandRegistry::with_defaults();
        let before = reg.len();
        let err = reg
            .register_typed(typed_cmd(name, aliases, TypedBody::Native(dummy_typed)))
            .expect_err(name);
        assert!(err.contains("'b7'"), "unexpected message: {err}");
        assert_eq!(reg.len(), before, "{name}: nothing may be registered");
        assert!(
            reg.get_typed("b7").is_none(),
            "{name}: 'b7' must not resolve"
        );
    }
}

/// An editor command's name is lowercase; any other is refused and leaves
/// the registry unchanged.
#[test]
fn register_refuses_an_editor_command_name_that_is_not_lowercase() {
    let mut reg = CommandRegistry::with_defaults();
    let before = reg.len();
    let err = reg
        .register(MappableCommand::Lazy {
            name: Cow::Borrowed("Probe"),
            plugin: test_plugin(),
        })
        .expect_err("an uppercase editor command name must be refused");
    assert!(
        err.contains("'Probe'") && err.contains("lowercase"),
        "unexpected message: {err}"
    );
    assert_eq!(reg.len(), before);
}

/// Every lookup ignores case, and a typed command keeps the spelling it was
/// registered with.
#[test]
fn lookups_ignore_case() {
    let plugin = test_plugin();
    let mut reg = CommandRegistry::with_defaults();
    reg.register_typed(typed_cmd("probe", &[], TypedBody::Lazy(plugin.clone())))
        .expect("a valid typed command name");

    assert_eq!(
        reg.get_typed("WRITE").map(|tc| tc.name.as_ref()),
        Some("write")
    );
    assert_eq!(reg.get_typed("W").map(|tc| tc.name.as_ref()), Some("write"));
    assert!(reg.get_mappable("MOVE-RIGHT").is_some());
    assert!(reg.contains("MOVE-RIGHT"));
    assert!(reg.other_kind_hint("MOVE-RIGHT").is_some());
    assert_eq!(reg.lazy_owner("PROBE"), Some(&plugin));

    let before = reg.len();
    reg.register_typed(typed_cmd("Probe", &[], TypedBody::Native(dummy_typed)))
        .expect("a valid typed command name");

    assert_eq!(reg.len(), before, "`Probe` replaces the `probe` stub");
    assert_eq!(
        reg.get_typed("probe").map(|tc| tc.name.as_ref()),
        Some("Probe")
    );
    assert_eq!(reg.lazy_owner("probe"), None);
}

#[test]
fn steel_backed_names_filters_by_variant() {
    let mut reg = CommandRegistry::with_defaults();
    // Defaults contain no SteelBacked commands: every built-in is
    // Motion / Selection / Edit / EditorCmd / Typed.
    assert!(reg.steel_backed_names().is_empty());

    // Register two SteelBacked commands alongside a non-SteelBacked one.
    reg.register(MappableCommand::SteelBacked(SteelCommand {
        name: Cow::Owned("my-steel-cmd".to_string()),
        doc: Cow::Borrowed("doc"),
        body: hume_scripting::SteelCmdBody {
            arity: 0,
            is_variadic: false,
            inline_output: false,
            repeatable: false,
            proc: hume_scripting::MappableProc::for_test(),
        },
    }))
    .expect("a lowercase editor command name");
    reg.register(MappableCommand::SteelBacked(SteelCommand {
        name: Cow::Owned("another-steel-cmd".to_string()),
        doc: Cow::Borrowed("doc"),
        body: hume_scripting::SteelCmdBody {
            arity: 0,
            is_variadic: false,
            inline_output: false,
            repeatable: false,
            proc: hume_scripting::MappableProc::for_test(),
        },
    }))
    .expect("a lowercase editor command name");
    // An EditorCmd with a name that could be mistaken for a Steel proc:
    // the helper must still filter it out by variant, not by name shape.
    fn noop(
        _state: &mut crate::editor::EditorState,
        _view: &mut hume_engine::pipeline::EngineView,
        _t: crate::editor::commands::CommandPane,
        _count: crate::editor::registry::Count,
        _mode: hume_ops::MotionMode,
    ) -> Result<(), crate::editor::error::CommandError> {
        Ok(())
    }
    reg.register(MappableCommand::Native(NativeCommand::EditorCmd {
        name: Cow::Owned("%hume-cmd-decoy".to_string()),
        doc: Cow::Borrowed("doc"),
        fun: crate::editor::commands::NativeBody::new(EditorCmdBody::Pane(noop)),
        defers_paste_commit: false,
        repeatable: false,
        jump: false,
        visual_move: false,
        extendable: false,
        clears_extend: false,
        selection_tracking: SelectionTracking::Untracked,
        manages_own_session: false,
        key_only: false,
    }))
    .expect("a lowercase editor command name");

    let mut names = reg.steel_backed_names();
    names.sort();
    assert_eq!(
        names,
        vec!["another-steel-cmd".to_string(), "my-steel-cmd".to_string()]
    );
}

/// A failed entry's rollback removes the Steel commands, mappable and typed,
/// that this entry defined. It leaves alone the commands of another entry of
/// the same plugin, of another plugin and of the user, and also the entry's
/// own `Lazy` stub and native commands.
#[test]
fn unregister_defined_by_removes_only_the_entrys_steel_commands() {
    use hume_scripting::attribution::{EntryFile, EntryId, PluginId};
    use hume_scripting::{MappableProc, SteelCmdBody, SteelTypedCmdBody, TypedProc};

    fn mappable(name: &str, proc: MappableProc) -> MappableCommand {
        MappableCommand::SteelBacked(SteelCommand {
            name: Cow::Owned(name.to_string()),
            doc: Cow::Borrowed(""),
            body: SteelCmdBody {
                arity: 0,
                is_variadic: false,
                inline_output: false,
                repeatable: false,
                proc,
            },
        })
    }
    fn typed(name: &str, proc: TypedProc) -> TypedCommand {
        typed_cmd(
            name,
            &[],
            TypedBody::Steel(SteelTypedCmdBody {
                arity: 0,
                is_variadic: false,
                inline_output: false,
                proc,
            }),
        )
    }
    let plugin = |repo: &str| PluginId::User {
        user: "u".to_string(),
        repo: repo.to_string(),
    };
    let failed = EntryId::main(plugin("tp"));
    let sibling = EntryId::new(
        plugin("tp"),
        EntryFile::parse("extra.scm").expect("a valid entry file"),
    );
    let other = EntryId::main(plugin("other"));

    let mut reg = CommandRegistry::with_defaults();
    reg.register(mappable(
        "failed-map",
        MappableProc::for_test_defined_by(failed.clone()),
    ))
    .expect("a lowercase editor command name");
    reg.register_typed(typed(
        "failed-typed",
        TypedProc::for_test_defined_by(failed.clone()),
    ))
    .expect("a valid typed command name");
    reg.register(MappableCommand::Lazy {
        name: Cow::Borrowed("failed-stub"),
        plugin: failed.clone(),
    })
    .expect("a lowercase editor command name");
    reg.register(mappable(
        "sibling-map",
        MappableProc::for_test_defined_by(sibling),
    ))
    .expect("a lowercase editor command name");
    reg.register_typed(typed("other-typed", TypedProc::for_test_defined_by(other)))
        .expect("a valid typed command name");
    reg.register(mappable("user-map", MappableProc::for_test()))
        .expect("a lowercase editor command name");
    let before = reg.len();

    reg.unregister_defined_by(&failed);

    assert!(!reg.contains("failed-map"));
    assert!(!reg.contains("failed-typed"));
    assert_eq!(
        reg.len(),
        before - 2,
        "only the two commands `failed` defined go"
    );
    for kept in ["failed-stub", "sibling-map", "other-typed", "user-map"] {
        assert!(reg.contains(kept), "'{kept}' must survive the rollback");
    }
    assert!(
        reg.get_mappable("move-left")
            .is_some_and(MappableCommand::is_native)
    );
}

#[test]
fn clears_extend_flag_matches_expected_commands() {
    let reg = CommandRegistry::with_defaults();

    // Selection-consuming edits: must be true.
    for name in &[
        "delete",
        "paste-after",
        "paste-before",
        "smart-paste-after",
        "smart-paste-before",
        "paste-ring-older",
        "paste-ring-newer",
        "replace",
        "surround-add",
    ] {
        let meta = reg
            .get_mappable(name)
            .unwrap_or_else(|| panic!("command '{name}' not found"))
            .meta();
        assert!(
            meta.clears_extend,
            "'{name}' should have clears_extend = true"
        );
    }

    // Intentionally excluded: yank (non-destructive), change (enters Insert),
    // undo/redo (history navigation), flip-selections (selection builder).
    for name in &["yank", "change", "undo", "redo", "flip-selections"] {
        let meta = reg
            .get_mappable(name)
            .unwrap_or_else(|| panic!("command '{name}' not found"))
            .meta();
        assert!(
            !meta.clears_extend,
            "'{name}' should have clears_extend = false"
        );
    }
}

#[test]
fn selection_tracking_matches_expected_commands() {
    let reg = CommandRegistry::with_defaults();

    // `Composes`: transforms or reduces whatever extent is already staged
    // rather than establishing one of its own. See `SelectionTracking::Composes`.
    for name in &[
        "collapse-selection",
        "flip-selections",
        "keep-primary-selection",
        "remove-primary-selection",
        "cycle-primary-forward",
        "cycle-primary-backward",
        "split-selection-on-newlines",
        "trim-selection-whitespace",
        "copy-selection-on-next-line",
        "copy-selection-on-prev-line",
    ] {
        let meta = reg
            .get_mappable(name)
            .unwrap_or_else(|| panic!("command '{name}' not found"))
            .meta();
        assert_eq!(
            meta.selection_tracking,
            SelectionTracking::Composes,
            "'{name}' should have selection_tracking = Composes"
        );
    }

    // `Establishes`: replayable on its own from a fresh cursor.
    // `select-all` is whole-buffer and position-independent, unlike its
    // `Composes` siblings above despite sharing their `Selection` variant.
    // `surround-paren` is a plain `selection!`-registered text object (the
    // macro's default arm); `select-word-nearest-on-line` is the one
    // `EditorCmd` that opts in via `.establishes_selection()` instead of a
    // `Selection` variant's own hardcoded field.
    for name in &[
        "select-line",
        "select-all",
        "select-all-matches",
        "surround-paren",
        "select-word-nearest-on-line",
        "inner-function",
    ] {
        let meta = reg
            .get_mappable(name)
            .unwrap_or_else(|| panic!("command '{name}' not found"))
            .meta();
        assert_eq!(
            meta.selection_tracking,
            SelectionTracking::Establishes,
            "'{name}' should have selection_tracking = Establishes"
        );
    }

    // `Extends`: every `Motion`. A Move-mode result is a bare cursor (or,
    // for the word motions, a selection reached by navigating away from the
    // cursor), so it never establishes a step of its own.
    for name in &["move-left", "select-next-word", "goto-next-function"] {
        let meta = reg
            .get_mappable(name)
            .unwrap_or_else(|| panic!("command '{name}' not found"))
            .meta();
        assert_eq!(
            meta.selection_tracking,
            SelectionTracking::Extends,
            "'{name}' should have selection_tracking = Extends"
        );
    }

    // `Untracked`: not a selection builder.
    for name in &["delete", "undo", "search-word-under-cursor"] {
        let meta = reg
            .get_mappable(name)
            .unwrap_or_else(|| panic!("command '{name}' not found"))
            .meta();
        assert_eq!(
            meta.selection_tracking,
            SelectionTracking::Untracked,
            "'{name}' should have selection_tracking = Untracked"
        );
    }
}

/// A command cannot be both repeatable (an edit that modifies the buffer)
/// and a selection-builder (a pure cursor movement): the fresh-recording begin
/// and `step_update_recipe` (`commands/pipeline.rs`) both fire on the same
/// dispatch and would conflict. This property is fixed at registration
/// time, so it is checked once here for every registered command rather than
/// re-probed on every dispatch.
#[test]
fn no_command_is_both_repeatable_and_selection_tracking() {
    let reg = CommandRegistry::with_defaults();

    for name in reg.names() {
        let Some(cmd) = reg.get_mappable(name) else {
            continue; // typed (`:`) commands carry no CmdMeta
        };
        let meta = cmd.meta();
        assert!(
            !(meta.repeatable && meta.selection_tracking != SelectionTracking::Untracked),
            "command '{name}' is both repeatable and selection-tracking"
        );
    }
}
