use super::*;

use hume_treesitter::textobjects::ObjectKind;

/// Exhaustiveness guard: if a command is added without a registry entry,
/// this test catches it.
const EXPECTED_COMMAND_COUNT: usize = 184;

/// `STRUCTURAL_OBJECTS` is the only link between `ObjectKind` and the four
/// commands (plus the `m i`/`m a` key) each kind ships. Nothing in the type
/// system ties the two together: a variant added to `ObjectKind` but not to
/// the table still compiles, still collects spans, and silently ships zero
/// commands and zero keybindings — a quieter failure than the out-of-bounds
/// panic `object_enum!`'s own generated `ALL` exists to prevent.
///
/// Flip: delete a row and this fails naming the orphaned kind;
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
        "registered command count mismatch — did you add a command without registering it?"
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
    assert!(matches!(cmd, MappableCommand::Motion { .. }));

    // Selection
    let cmd = reg
        .get_mappable("collapse-selection")
        .expect("collapse-selection should be registered");
    assert_eq!(cmd.name().as_ref(), "collapse-selection");
    assert!(matches!(cmd, MappableCommand::Selection { .. }));

    // Edit
    let cmd = reg
        .get_mappable("delete-selection")
        .expect("delete-selection should be registered");
    assert_eq!(cmd.name().as_ref(), "delete-selection");
    assert!(matches!(cmd, MappableCommand::Edit { .. }));

    // EditorCmd
    let cmd = reg.get_mappable("undo").expect("undo should be registered");
    assert_eq!(cmd.name().as_ref(), "undo");
    assert!(matches!(cmd, MappableCommand::EditorCmd { .. }));

    let cmd = reg
        .get_mappable("find-forward")
        .expect("find-forward should be registered");
    assert!(matches!(cmd, MappableCommand::EditorCmd { .. }));

    let cmd = reg
        .get_mappable("delete")
        .expect("delete should be registered");
    assert!(matches!(cmd, MappableCommand::EditorCmd { .. }));
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
    // All Steel commands participate in Ctrl+key one-shot extend; the
    // body receives `extend` as a lambda arg and decides what to do.
    let cmd = MappableCommand::SteelBacked {
        name: "x".into(),
        doc: "".into(),
        arity: 0,
        is_variadic: false,
        inline_output: false,
        repeatable: false,
    };
    assert!(cmd.is_extendable());
}

#[test]
fn is_extendable_lazy_stub_true() {
    use hume_scripting::attribution::PluginId;
    // Lazy stubs resolve to SteelBacked; the keymap must treat them as
    // extendable on first Ctrl+key press so extend=true is forwarded even
    // before the plugin is activated.
    let cmd = MappableCommand::Lazy {
        name: "lazy-cmd".into(),
        plugin: PluginId::User {
            user: "u".to_string(),
            repo: "r".to_string(),
        },
    };
    assert!(cmd.is_extendable());
}

#[test]
fn all_names_are_unique() {
    // HashMap insertion silently overwrites duplicates — verify the final
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
        _count: usize,
        _mode: hume_ops::MotionMode,
    ) -> Result<(), crate::editor::error::CommandError> {
        Ok(())
    }
    let cmd = MappableCommand::EditorCmd {
        name: Cow::Owned("steel-test-cmd".to_string()),
        doc: Cow::Borrowed("A dummy Steel command for testing."),
        fun: dummy_fn,
        defers_paste_commit: false,
        repeatable: false,
        jump: false,
        visual_move: false,
        extendable: false,
        clears_extend: false,
        selection_tracking: SelectionTracking::Untracked,
    };
    reg.register(cmd);

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
    // accessible as mappable so keybinds continue to work — and, since `:`
    // resolves only typed commands, neither is reachable from the command
    // line at all (see registry/mod.rs's module doc).
    let reg = CommandRegistry::with_defaults();
    assert!(reg.get_mappable("clear-search").is_some());
    assert!(reg.get_mappable("select-all-matches").is_some());
    assert!(reg.get_typed("clear-search").is_none());
    assert!(reg.get_typed("select-all-matches").is_none());
}

#[test]
fn runtime_register_typed_and_lookup() {
    use crate::editor::Editor;
    let mut reg = CommandRegistry::with_defaults();
    let before = reg.len();

    fn dummy_typed(
        _ed: &mut Editor,
        _arg: Option<&str>,
        _force: bool,
    ) -> Result<(), crate::editor::error::CommandError> {
        Ok(())
    }
    reg.register_typed(TypedCommand {
        name: Cow::Owned("steel-typed-cmd".to_string()),
        doc: Cow::Borrowed("A dummy Steel typed command for testing."),
        aliases: &["stc"],
        body: TypedBody::Native(dummy_typed),
        completer: None,
    });

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

#[test]
fn steel_backed_names_filters_by_variant() {
    let mut reg = CommandRegistry::with_defaults();
    // Defaults contain no SteelBacked commands — every built-in is
    // Motion / Selection / Edit / EditorCmd / Typed.
    assert!(reg.steel_backed_names().is_empty());

    // Register two SteelBacked commands alongside a non-SteelBacked one.
    reg.register(MappableCommand::SteelBacked {
        name: Cow::Owned("my-steel-cmd".to_string()),
        doc: Cow::Borrowed("doc"),
        arity: 0,
        is_variadic: false,
        inline_output: false,
        repeatable: false,
    });
    reg.register(MappableCommand::SteelBacked {
        name: Cow::Owned("another-steel-cmd".to_string()),
        doc: Cow::Borrowed("doc"),
        arity: 0,
        is_variadic: false,
        inline_output: false,
        repeatable: false,
    });
    // An EditorCmd with a name that could be mistaken for a Steel proc —
    // the helper must still filter it out by variant, not by name shape.
    fn noop(
        _state: &mut crate::editor::EditorState,
        _view: &mut hume_engine::pipeline::EngineView,
        _count: usize,
        _mode: hume_ops::MotionMode,
    ) -> Result<(), crate::editor::error::CommandError> {
        Ok(())
    }
    reg.register(MappableCommand::EditorCmd {
        name: Cow::Owned("%hume-cmd-decoy".to_string()),
        doc: Cow::Borrowed("doc"),
        fun: noop,
        defers_paste_commit: false,
        repeatable: false,
        jump: false,
        visual_move: false,
        extendable: false,
        clears_extend: false,
        selection_tracking: SelectionTracking::Untracked,
    });

    let mut names = reg.steel_backed_names();
    names.sort();
    assert_eq!(
        names,
        vec!["another-steel-cmd".to_string(), "my-steel-cmd".to_string()]
    );
}

/// `unregister` removes dynamic (`SteelBacked`/`Lazy`) entries but refuses
/// native commands — a failed-plugin rollback must never be able to delete
/// a built-in for the rest of the session.
///
/// Fail oracle: revert `unregister` to an unconditional `remove` → the
/// `move-left` assert fires.
#[test]
fn unregister_removes_dynamic_but_not_native() {
    use hume_scripting::attribution::PluginId;
    let mut reg = CommandRegistry::with_defaults();
    reg.register(MappableCommand::SteelBacked {
        name: Cow::Owned("steel-cmd".to_string()),
        doc: Cow::Borrowed("doc"),
        arity: 0,
        is_variadic: false,
        inline_output: false,
        repeatable: false,
    });
    reg.register(MappableCommand::Lazy {
        name: Cow::Owned("lazy-cmd".to_string()),
        plugin: PluginId::User {
            user: "u".to_string(),
            repo: "r".to_string(),
        },
    });

    reg.unregister("steel-cmd");
    reg.unregister("lazy-cmd");
    reg.unregister("move-left"); // native — must be a no-op

    assert!(reg.get_mappable("steel-cmd").is_none());
    assert!(reg.get_mappable("lazy-cmd").is_none());
    assert!(
        reg.get_mappable("move-left").is_some(),
        "unregister must refuse to remove a native command"
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
    // rather than establishing one of its own — see `SelectionTracking::Composes`.
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

    // `Extends`: every `Motion` — a Move-mode result is a bare cursor (or,
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
/// and a selection-builder (a pure cursor movement) — `step_stamp_repeatable`
/// and `step_update_recipe` (`commands/pipeline.rs`) both fire off the same
/// AFTER stage and would conflict. This property is fixed at registration
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
