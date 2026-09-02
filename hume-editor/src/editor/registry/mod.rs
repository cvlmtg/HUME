//! Command registry — the single namespace for all user-facing commands.
//!
//! Two kinds of commands share this registry:
//!
//! - [`MappableCommand`] — bindable to keys. The keymap trie stores command
//!   *names*; the registry resolves them to `MappableCommand` values at
//!   dispatch time inside `execute_keymap_command` (`mappings/execute.rs`).
//! - [`TypedCommand`] — invocable from the `:` command line. The dispatcher
//!   in `execute_command` (`mappings/command_mode.rs`) calls
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
//! [`MappableCommand`] has four native shapes — see its own variant docs for
//! exact signatures:
//! 1. **Motion** — pure, repeats `count` times over a `SelectionSet`.
//! 2. **Selection** — pure, same shape without the repeat semantics.
//! 3. **Edit** — pure, takes/returns `BufferText` — never extendable.
//! 4. **EditorCmd** — side-effectful, for composite operations (mode
//!    changes, registers, undo groups, parameterized motions). Implemented
//!    in `editor/commands/`; stored and dispatched as a function pointer
//!    exactly like the other variants.

use rustc_hash::FxHashMap;
use std::borrow::Cow;

mod command;
mod defaults;

pub(crate) use command::{
    ArgCompleter, CmdMeta, EditorCmdFn, MappableCommand, SelectionBody, SelectionTracking,
    StructuralBody, TypedCommand,
};
pub(in crate::editor) use defaults::structural::STRUCTURAL_OBJECTS;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Case-insensitive FxHashMap lookup: exact hit first, then linear-scan fallback.
///
/// Used only for the typed-command path (`:` command line) so a user typing
/// `:W` still resolves the canonical `:w`. Mappable commands are looked up by
/// exact name only (see [`CommandRegistry::get_mappable`]) since they are
/// resolved from key bindings, not user-typed names.
fn ci_get<'a, V>(map: &'a FxHashMap<Cow<'static, str>, V>, name: &str) -> Option<&'a V> {
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
///   (`execute_keymap_command` in `mappings/execute.rs`) resolves them via
///   [`Self::get_mappable`].
/// - **Typed commands** are invoked from the `:` command line. The dispatcher
///   (`execute_command` in `mappings/command_mode.rs`) resolves them via
///   [`Self::get_typed`]. Aliases are supported via [`Self::alias_map`].
/// - The `:` command line also falls back to **mappable commands** when no
///   typed command matches — any mappable command can be invoked by name
///   from the command line with an implicit `count = 1`.
///
/// The single `commands` map prevents name collisions between the two kinds.
pub(crate) struct CommandRegistry {
    /// All commands keyed by canonical name.
    commands: FxHashMap<Cow<'static, str>, Command>,
    /// Maps typed-command alias → canonical name, for O(1) alias lookup.
    alias_map: FxHashMap<Cow<'static, str>, Cow<'static, str>>,
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
            commands: FxHashMap::default(),
            alias_map: FxHashMap::default(),
        };
        reg.register_defaults();
        reg
    }

    /// Register a mappable command.
    ///
    /// The name is extracted from the command and used as the `FxHashMap` key.
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

    /// Remove a single dynamic (`SteelBacked`/`Lazy`) command by name.
    ///
    /// Native mappables and typed commands are never removed: every caller
    /// (Lazy-stub cleanup, failed-plugin rollback) only legitimately owns
    /// dynamic entries, so refusing anything else keeps a buggy rollback from
    /// deleting a built-in command for the rest of the session.
    pub(crate) fn unregister(&mut self, name: &str) {
        if matches!(
            self.commands.get(name),
            Some(Command::Mappable(mc)) if !mc.is_native()
        ) {
            self.commands.remove(name);
        }
    }

    /// Returns `true` if `name` is registered as either a mappable or typed command.
    ///
    /// Unlike [`Self::get_mappable`], this also matches typed commands — use it
    /// when checking whether a name is already claimed by anything in the registry.
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.commands.contains_key(name)
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
    /// Used by `execute_keymap_command` in `mappings/execute.rs`.
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
    /// this returns `None` — see `execute_command` in `mappings/command_mode.rs`.
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

    /// Total number of registered commands (mappable + typed, not counting aliases).
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.commands.len()
    }

    /// Every current `Lazy` stub as `(name, owning plugin)`.
    ///
    /// Used by `:plugin-status` (via `lazy_status_string`) to report which
    /// commands a `Declared` plugin is still waiting on — the registry is the
    /// sole owner of `Lazy` stubs, so this is the only source for that list.
    pub(crate) fn lazy_stubs(&self) -> Vec<(String, hume_scripting::attribution::PluginId)> {
        self.commands
            .iter()
            .filter_map(|(name, cmd)| match cmd {
                Command::Mappable(MappableCommand::Lazy { plugin, .. }) => {
                    Some((name.as_ref().to_string(), plugin.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// Remove every remaining `Lazy` stub owned by `plugin`.
    ///
    /// Called by `finish_lazy_activation` (via `CommandHost::unregister_lazy_
    /// stubs_of`) on both the success and failure path — never touches a
    /// `SteelBacked` command, even one that just replaced a stub of the same
    /// name for this plugin.
    pub(crate) fn unregister_lazy_stubs_of(
        &mut self,
        plugin: &hume_scripting::attribution::PluginId,
    ) {
        self.commands.retain(|_, cmd| {
            !matches!(cmd, Command::Mappable(MappableCommand::Lazy { plugin: p, .. }) if p == plugin)
        });
    }
}

// ── Test-only helpers ─────────────────────────────────────────────────────────

#[cfg(test)]
impl CommandRegistry {
    /// Collect the canonical names of every `SteelBacked` command. Test-only.
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
mod tests;
