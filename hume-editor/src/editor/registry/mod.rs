//! Command registry — the single namespace for all user-facing commands.
//!
//! Two kinds of commands share this registry:
//!
//! - [`MappableCommand`] — bindable to keys. The keymap trie stores command
//!   *names*; the registry resolves them to `MappableCommand` values at
//!   dispatch time inside `execute_keymap_command` (`editor/mappings.rs`).
//! - [`TypedCommand`] — invocable from the `:` command line. The dispatcher
//!   in `execute_command` (`editor/mappings.rs`) calls
//!   [`CommandRegistry::get_typed`] to resolve name or alias to a
//!   `TypedCommand`.
//!
//! The shared namespace prevents name collisions between the two kinds and
//! provides a single source for `:help` and command-palette display.
//!
//! # Extend mode
//!
//! Extend mode is handled at dispatch time via a `MotionMode` parameter, not
//! via separate extend-variant commands. All Motion and Selection commands
//! accept `MotionMode` and branch on `Move` vs `Extend`. EditorCmds that
//! support extend carry `extendable: true`; the dispatcher passes the correct
//! `MotionMode` based on the current mode or Ctrl+letter state.
//!
//! # Mappable command variants
//!
//! 1. **Motion** — pure `fn(&Text, SelectionSet, usize, MotionMode) -> SelectionSet`
//! 2. **Selection** — pure `fn(&Text, SelectionSet, MotionMode) -> SelectionSet`
//! 3. **Edit** — pure `fn(Text, SelectionSet) -> (Text, SelectionSet, ChangeSet)`
//! 4. **EditorCmd** — `fn(&mut EditorState, &mut EngineView, usize, MotionMode) -> Result<(), CommandError>`
//!    for composite/side-effectful operations (mode changes, registers, undo
//!    groups, parameterized motions). Implemented in `editor/commands/`; stored
//!    and dispatched as a function pointer exactly like the other variants.

use std::borrow::Cow;
use std::collections::HashMap;

mod command;
mod defaults;

pub(crate) use command::{CmdMeta, EditorCmdFn, MappableCommand, TypedCommand};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Case-insensitive HashMap lookup: exact hit first, then linear-scan fallback.
///
/// Used only for the typed-command path (`:` command line) so a user typing
/// `:W` still resolves the canonical `:w`. Mappable commands are looked up by
/// exact name only (see [`CommandRegistry::get_mappable`]) since they are
/// resolved from key bindings, not user-typed names.
fn ci_get<'a, V>(map: &'a HashMap<Cow<'static, str>, V>, name: &str) -> Option<&'a V> {
    map.get(name).or_else(|| {
        map.iter()
            .find(|(k, _)| k.as_ref().eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
    })
}

// ── CommandRegistry ───────────────────────────────────────────────────────────

/// Registry of all commands — the single namespace for mappable and typed commands.
///
/// Built once via [`CommandRegistry::with_defaults`] and stored on the editor.
///
/// - **Mappable commands** are bound to keys. The keymap dispatcher
///   (`execute_keymap_command` in `editor/mappings.rs`) resolves them via
///   [`Self::get_mappable`].
/// - **Typed commands** are invoked from the `:` command line. The dispatcher
///   (`execute_command` in `editor/mappings.rs`) resolves them via
///   [`Self::get_typed`]. Aliases are supported via [`Self::alias_map`].
/// - The `:` command line also falls back to **mappable commands** when no
///   typed command matches — any mappable command can be invoked by name
///   from the command line with an implicit `count = 1`.
///
/// The single `commands` map prevents name collisions between the two kinds.
pub(crate) struct CommandRegistry {
    /// All commands keyed by canonical name.
    commands: HashMap<Cow<'static, str>, Command>,
    /// Maps typed-command alias → canonical name, for O(1) alias lookup.
    alias_map: HashMap<Cow<'static, str>, Cow<'static, str>>,
}

/// A command stored in [`CommandRegistry`].
///
/// Mappable and typed commands share the same namespace but have different
/// signatures and dispatch paths.
pub(crate) enum Command {
    Mappable(MappableCommand),
    Typed(TypedCommand),
}

impl CommandRegistry {
    /// Build a registry pre-populated with every default command.
    pub(crate) fn with_defaults() -> Self {
        let mut reg = Self {
            commands: HashMap::new(),
            alias_map: HashMap::new(),
        };
        reg.register_defaults();
        reg
    }

    /// Register a mappable command.
    ///
    /// The name is extracted from the command and used as the `HashMap` key.
    /// For static built-ins the clone is a pointer copy (zero allocation).
    pub(crate) fn register(&mut self, cmd: MappableCommand) {
        let key = match &cmd {
            MappableCommand::Motion { name, .. }
            | MappableCommand::Selection { name, .. }
            | MappableCommand::Edit { name, .. }
            | MappableCommand::EditorCmd { name, .. }
            | MappableCommand::SteelBacked { name, .. }
            | MappableCommand::Lazy { name, .. } => name.clone(),
        };
        self.commands.insert(key, Command::Mappable(cmd));
    }

    /// Remove a single command by name (both mappable and typed).
    pub(crate) fn unregister(&mut self, name: &str) {
        self.commands.remove(name);
    }

    /// Returns `true` if `name` is registered as either a mappable or typed command.
    ///
    /// Unlike [`Self::get_mappable`], this also matches typed commands — use it
    /// when checking whether a name is already claimed by anything in the registry.
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.commands.contains_key(name)
    }

    /// Remove every `SteelBacked` and `Lazy` mappable command in one pass.
    ///
    /// Used by `:reload-config` to clear stale entries before re-evaluating
    /// `init.scm` with a fresh engine — otherwise those names would appear in
    /// `builtin_cmd_names` and cause every `(define-command!)` to raise a
    /// phantom "conflicts with a built-in command" error. `Lazy` stubs must
    /// also be cleared so re-declared activation command names do not collide.
    pub(crate) fn unregister_dynamic_commands(&mut self) {
        self.commands
            .retain(|_, cmd| !matches!(cmd, Command::Mappable(mc) if !mc.is_native()));
    }

    /// Register a typed command.
    ///
    /// Inserts the canonical name into `commands` and each alias into
    /// `alias_map`. This is the future `define-typed-command!` entry point
    /// for the Steel scripting layer.
    pub(crate) fn register_typed(&mut self, cmd: TypedCommand) {
        let canonical = cmd.name.clone();
        for &alias in cmd.aliases {
            self.alias_map
                .insert(Cow::Borrowed(alias), canonical.clone());
        }
        self.commands.insert(canonical, Command::Typed(cmd));
    }

    /// Look up a mappable command by exact name.
    ///
    /// Returns `None` if the name is unknown or resolves to a typed command.
    /// Used by `execute_keymap_command` in `editor/mappings.rs`.
    ///
    /// Exact-only: mappable commands are resolved from key bindings, not
    /// user-typed names, so there is no user typo to tolerate and case-folding
    /// has no purpose here. Case-insensitivity is confined to the typed (`:`)
    /// path in [`Self::get_typed`].
    pub(crate) fn get_mappable(&self, name: &str) -> Option<&MappableCommand> {
        match self.commands.get(name)? {
            Command::Mappable(cmd) => Some(cmd),
            Command::Typed(_) => None,
        }
    }

    /// Look up a typed command by canonical name or alias (case-insensitive).
    ///
    /// Returns `None` if the name is unknown or resolves to a mappable command.
    /// The `:` command dispatcher falls back to [`Self::get_mappable`] when
    /// this returns `None` — see `execute_command` in `editor/mappings.rs`.
    pub(crate) fn get_typed(&self, name: &str) -> Option<&TypedCommand> {
        let canonical = ci_get(&self.alias_map, name)
            .map(|c| c.as_ref())
            .unwrap_or(name);
        match ci_get(&self.commands, canonical)? {
            Command::Typed(cmd) => Some(cmd),
            Command::Mappable(_) => None,
        }
    }

    /// Iterate over all registered canonical command names (not aliases).
    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        self.commands.keys().map(|k| k.as_ref())
    }

    /// Iterate over the names of native mappable commands only:
    /// `Motion`, `Selection`, `Edit`, and `EditorCmd` variants.
    ///
    /// Excludes `SteelBacked`, `Lazy` (plugin commands), and `TypedCommand`
    /// (`:` command-line only) entries.  Used to pre-register bare command
    /// bindings in the Steel engine so `(move-left)` etc. compile without a
    /// `FreeIdentifier` error.
    pub(crate) fn native_mappable_names(&self) -> impl Iterator<Item = &str> {
        self.commands.iter().filter_map(|(k, v)| match v {
            Command::Mappable(cmd) if cmd.is_native() => Some(k.as_ref()),
            _ => None,
        })
    }

    /// Iterate over all command names: canonical names and aliases combined.
    ///
    /// May yield duplicate strings only if a name is registered as both a
    /// canonical and an alias — which is a registry bug and must not happen.
    /// Callers that display results should sort and dedup.
    pub(crate) fn iter_names_and_aliases(&self) -> impl Iterator<Item = &str> {
        self.commands
            .keys()
            .map(|k| k.as_ref())
            .chain(self.alias_map.keys().map(|k| k.as_ref()))
    }

    /// Total number of registered commands (mappable + typed, not counting aliases).
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.commands.len()
    }
}

// ── Test-only helpers ─────────────────────────────────────────────────────────

#[cfg(test)]
impl CommandRegistry {
    /// Collect the canonical names of every `SteelBacked` command.
    /// Only used in tests — production code uses `unregister_dynamic_commands`.
    pub(crate) fn steel_backed_names(&self) -> Vec<String> {
        self.commands
            .values()
            .filter_map(|cmd| match cmd {
                Command::Mappable(MappableCommand::SteelBacked { name, .. }) => {
                    Some(name.as_ref().to_string())
                }
                _ => None,
            })
            .collect()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Exhaustiveness guard: if a command is added without a registry entry, this test catches it.
    const EXPECTED_COMMAND_COUNT: usize = 146;

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
        let cmd = reg
            .get_mappable("force-quit")
            .expect("force-quit should be registered");
        assert_eq!(cmd.name().as_ref(), "force-quit");
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
        assert!(reg.get_typed("force-quit").is_none());
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
            "select-next-word",
            "select-prev-word",
            "next-paragraph",
            "prev-paragraph",
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
            "force-quit",
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
            _mode: crate::ops::MotionMode,
        ) -> Result<(), crate::editor::error::CommandError> {
            Ok(())
        }
        let cmd = MappableCommand::EditorCmd {
            name: Cow::Owned("steel-test-cmd".to_string()),
            doc: Cow::Borrowed("A dummy Steel command for testing."),
            fun: dummy_fn,
            is_paste: false,
            defers_paste_commit: false,
            repeatable: false,
            jump: false,
            visual_move: false,
            extendable: false,
            stamps_last_command: true,
            clears_extend: false,
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
        // accessible as mappable so keybinds continue to work. The command line
        // reaches them via the fallback in execute_command, not via get_typed.
        let reg = CommandRegistry::with_defaults();
        assert!(reg.get_mappable("clear-search").is_some());
        assert!(reg.get_mappable("select-all-matches").is_some());
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
            fun: dummy_typed,
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
            _mode: crate::ops::MotionMode,
        ) -> Result<(), crate::editor::error::CommandError> {
            Ok(())
        }
        reg.register(MappableCommand::EditorCmd {
            name: Cow::Owned("%hume-cmd-decoy".to_string()),
            doc: Cow::Borrowed("doc"),
            fun: noop,
            is_paste: false,
            defers_paste_commit: false,
            repeatable: false,
            jump: false,
            visual_move: false,
            extendable: false,
            stamps_last_command: true,
            clears_extend: false,
        });

        let mut names = reg.steel_backed_names();
        names.sort();
        assert_eq!(
            names,
            vec!["another-steel-cmd".to_string(), "my-steel-cmd".to_string()]
        );
    }

    #[test]
    fn unregister_dynamic_commands_clears_steel_backed_and_lazy() {
        use hume_scripting::attribution::PluginId;
        let mut reg = CommandRegistry::with_defaults();
        reg.register(MappableCommand::SteelBacked {
            name: Cow::Owned("plugin-cmd-a".to_string()),
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
        assert!(!reg.steel_backed_names().is_empty());
        assert!(reg.get_mappable("lazy-cmd").is_some());

        reg.unregister_dynamic_commands();

        assert!(reg.steel_backed_names().is_empty());
        assert!(reg.get_mappable("plugin-cmd-a").is_none());
        assert!(reg.get_mappable("lazy-cmd").is_none());
        // Built-in commands are untouched.
        assert!(reg.get_mappable("move-left").is_some());
    }

    #[test]
    fn clears_extend_flag_matches_expected_commands() {
        let reg = CommandRegistry::with_defaults();

        // Selection-consuming edits: must be true.
        for name in &[
            "delete",
            "paste-after",
            "paste-before",
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
}
