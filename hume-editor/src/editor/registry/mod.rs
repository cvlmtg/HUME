//! Command registry: the single namespace for all user-facing commands.
//!
//! - [`MappableCommand`]: bindable to keys. The keymap trie stores names,
//!   resolved here at dispatch time by `execute_keymap_command`.
//! - [`TypedCommand`]: invoked from the `:` command line via
//!   [`CommandRegistry::get_typed`], which accepts aliases.
//!
//! Each kind is looked up by its own accessor with no cross-kind fallback, so
//! a mappable name is unreachable from `:` and vice versa. Sharing one
//! namespace prevents collisions and gives `:help` a single source.
//!
//! # Extend mode
//!
//! Motion and Selection commands take a `MotionMode` (`Move` or `Extend`)
//! instead of having extend variants. EditorCmds that support it carry
//! `extendable: true`; the dispatcher picks the mode from the current mode or
//! Ctrl-letter state.
//!
//! # Mappable command variants
//!
//! 1. **Motion**: pure, repeats `count` times over a `SelectionSet`.
//! 2. **Selection**: pure, same shape without the repeat.
//! 3. **Edit**: pure, takes/returns `BufferText`; never extendable.
//! 4. **EditorCmd**: side-effectful composite operations (mode changes,
//!    registers, undo groups, parameterized motions), in `editor/commands/`.

use rustc_hash::FxHashMap;
use std::borrow::Cow;

mod command;
mod defaults;

pub(in crate::editor) use command::{
    Accepted, ArgExpansion, ArgOrForceInput, CmdMeta, EditFn, FocusedCmdFn, NativeTyped,
    PaneCharCmdFn, PaneCmdFn, PaneTextCmdFn, SelectionTracking, Takes, TextPrompt, TypedBody,
    TypedCommand,
};
// Narrower than the re-exports above: these carry a native command's `fun`
// function pointer, wrapped in `commands::NativeBody` so only
// `commands::pipeline::run_body` can call it; see `MappableCommand`'s
// own doc.
pub(in crate::editor) use command::{
    Dispatchable, EditorCmdBody, MappableCommand, NativeCommand, SelectionBody, SteelCommand,
    StructuralBody,
};
pub(in crate::editor) use defaults::structural::STRUCTURAL_OBJECTS;

use hume_scripting::command_name::fold;

/// Registry of all commands: the single namespace for mappable and typed commands.
///
/// Built once via [`CommandRegistry::with_defaults`] and stored on the editor.
///
/// - **Mappable commands** are bound to keys. The keymap dispatcher
///   (`execute_keymap_command` in `dispatch.rs`) resolves them via
///   [`Self::get_mappable`].
/// - **Typed commands** are invoked from the `:` command line. The dispatcher
///   (`run_command_line` in `input_stack/command.rs`) resolves them via
///   [`Self::get_typed`]. Aliases are supported via [`Self::alias_map`].
///
/// The two are strictly separate: `:` resolves only typed commands ([`Self::get_typed`]),
/// a key binding resolves only mappable commands ([`Self::get_mappable`]). The single
/// `commands` map still prevents name collisions between them.
///
/// Names ignore case: every entry is keyed by its name folded
/// ([`hume_scripting::command_name::fold`]), and every lookup folds the name
/// it is given, through [`Self::entry`] or [`Self::get_typed`].
pub(in crate::editor) struct CommandRegistry {
    /// All commands keyed by folded canonical name.
    commands: FxHashMap<Cow<'static, str>, Command>,
    /// Maps folded typed-command alias → folded canonical name, for O(1)
    /// alias lookup.
    alias_map: FxHashMap<Cow<'static, str>, Cow<'static, str>>,
}

/// A command stored in [`CommandRegistry`].
///
/// Mappable and typed commands share the same namespace but have different
/// signatures and dispatch paths.
pub(in crate::editor) enum Command {
    Mappable(MappableCommand),
    Typed(TypedCommand),
}

impl CommandRegistry {
    /// Build a registry pre-populated with every default command.
    pub(in crate::editor) fn with_defaults() -> Self {
        let mut reg = Self {
            commands: FxHashMap::default(),
            alias_map: FxHashMap::default(),
        };
        reg.register_defaults();
        reg
    }

    /// Register a mappable command.
    ///
    /// The name is copied from the command into the `FxHashMap` key.
    /// Refuses the command, registering nothing, when its name is not
    /// lowercase ([`hume_scripting::command_name::check_mappable`]), so the
    /// name is its own folded key.
    pub(in crate::editor) fn register(&mut self, cmd: MappableCommand) -> Result<(), String> {
        hume_scripting::command_name::check_mappable(cmd.name())?;
        let key = Cow::Owned(cmd.name().to_string());
        self.commands.insert(key, Command::Mappable(cmd));
        Ok(())
    }

    /// Remove the command registered under `name`, standing in for a plugin
    /// reload that drops a Steel command.
    #[cfg(test)]
    pub(in crate::editor) fn unregister(&mut self, name: &str) {
        self.commands.remove(fold(name).as_ref());
    }

    /// The entry registered under `name`, in any case.
    fn entry(&self, name: &str) -> Option<&Command> {
        self.commands.get(fold(name).as_ref())
    }

    /// Returns `true` if `name` is registered as either a mappable or typed command.
    ///
    /// Unlike [`Self::get_mappable`], this also matches typed commands. Use it
    /// when checking whether a name is already claimed by anything in the registry.
    pub(in crate::editor) fn contains(&self, name: &str) -> bool {
        self.entry(name).is_some()
    }

    /// If `name` is registered as the *other* kind, a user-facing clause
    /// explaining which and how it's actually reachable; `None` if `name`
    /// is unregistered entirely.
    ///
    /// Shared by every "unknown command" site that can otherwise only say
    /// a name doesn't exist when it actually exists as the wrong kind. One
    /// wording, so the sites can't drift into subtly different advice for
    /// the same mistake.
    pub(in crate::editor) fn other_kind_hint(&self, name: &str) -> Option<String> {
        let entry = self.entry(name)?;
        let name = name_of(entry);
        Some(match entry {
            Command::Mappable(_) => format!(
                "'{name}' is an editor command; bind it to a key, or run it with call!, not `:`"
            ),
            Command::Typed(_) => {
                format!(
                    "'{name}' is a typed command: run it as `:{name}`; keys and call! can't reach it"
                )
            }
        })
    }

    /// The message for `name` where a site expected a command it is not:
    /// [`Self::other_kind_hint`] when `name` is the other kind, else
    /// `fallback`, the site's own wording for a name nothing registers.
    pub(in crate::editor) fn unknown_command_message(
        &self,
        name: &str,
        fallback: String,
    ) -> String {
        self.other_kind_hint(name).unwrap_or(fallback)
    }

    /// The mappable command a script's `call!` reaches by `name`. A name that
    /// is unknown, typed, or bound to a key-only command is refused with the
    /// message `call!` raises, so `command-exists?` and `call!` agree.
    pub(in crate::editor) fn callable_by_script(
        &self,
        name: &str,
    ) -> Result<&MappableCommand, String> {
        let cmd = self.get_mappable(name).ok_or_else(|| {
            self.unknown_command_message(name, crate::editor::dispatch::unknown_command_text(name))
        })?;
        if !cmd.runs_from_script() {
            return Err(format!("'{name}': runs only from a key"));
        }
        Ok(cmd)
    }

    /// Register a typed command.
    ///
    /// Inserts the canonical name into `commands` and each alias into
    /// `alias_map`, both folded; the command keeps its own spelling. Refuses
    /// the command, registering nothing, when its name or an alias is one the
    /// `:` line can't read back whole
    /// ([`hume_scripting::command_name::check_typed`]).
    pub(in crate::editor) fn register_typed(&mut self, cmd: TypedCommand) -> Result<(), String> {
        std::iter::once(cmd.name.as_ref())
            .chain(cmd.aliases.iter().copied())
            .try_for_each(hume_scripting::command_name::check_typed)?;
        let canonical = match fold(&cmd.name) {
            Cow::Owned(folded) => Cow::Owned(folded),
            Cow::Borrowed(_) => cmd.name.clone(),
        };
        for &alias in cmd.aliases {
            self.alias_map.insert(fold(alias), canonical.clone());
        }
        self.commands.insert(canonical, Command::Typed(cmd));
        Ok(())
    }

    /// Look up a mappable command by name, in any case.
    ///
    /// Returns `None` if the name is unknown or resolves to a typed command.
    pub(in crate::editor) fn get_mappable(&self, name: &str) -> Option<&MappableCommand> {
        match self.entry(name)? {
            Command::Mappable(cmd) => Some(cmd),
            Command::Typed(_) => None,
        }
    }

    /// Look up a typed command by canonical name or alias, in any case.
    ///
    /// Returns `None` if the name is unknown or resolves to a mappable
    /// command: `:` never falls back to a mappable command; see
    /// `run_command_line` in `input_stack/command.rs`.
    pub(in crate::editor) fn get_typed(&self, name: &str) -> Option<&TypedCommand> {
        let key = fold(name);
        let canonical = self
            .alias_map
            .get(key.as_ref())
            .map_or(key.as_ref(), |c| c.as_ref());
        match self.commands.get(canonical)? {
            Command::Typed(cmd) => Some(cmd),
            Command::Mappable(_) => None,
        }
    }

    /// The name of the typed command [`Self::get_typed`] resolves `name` to,
    /// spelled as it was registered, for an alias or any casing of the name.
    pub(in crate::editor) fn typed_canonical_name(&self, name: &str) -> Option<&str> {
        self.get_typed(name).map(|tc| tc.name.as_ref())
    }

    /// Iterate over the canonical names of every registered typed command
    /// (not aliases), each spelled as it was registered. Feeds `:` Tab
    /// completion (`complete_command`), the typed-only counterpart of
    /// [`Self::native_mappable_names`].
    pub(in crate::editor) fn typed_names(&self) -> impl Iterator<Item = &str> {
        self.commands.values().filter_map(|v| match v {
            Command::Typed(cmd) => Some(cmd.name.as_ref()),
            _ => None,
        })
    }

    /// Iterate over the names of native mappable commands a script can
    /// reach: `Motion`, `Selection`, `Edit`, and `EditorCmd` variants that
    /// are not key-only.
    ///
    /// Excludes `SteelBacked`, `Lazy` (plugin commands), `TypedCommand`
    /// (`:` command-line only), and key-only entries. Used to pre-register
    /// bare command bindings in the Steel engine so `(move-left)` etc.
    /// compile without a `FreeIdentifier` error, which is also what
    /// `command-exists?` finds.
    pub(in crate::editor) fn native_mappable_names(&self) -> impl Iterator<Item = &str> {
        self.commands.iter().filter_map(|(k, v)| match v {
            Command::Mappable(cmd) if cmd.is_native() && cmd.runs_from_script() => Some(k.as_ref()),
            _ => None,
        })
    }

    /// Total number of registered commands (mappable + typed, not counting aliases).
    #[cfg(test)]
    pub(in crate::editor) fn len(&self) -> usize {
        self.commands.len()
    }

    /// The plugin owning `name`'s `Lazy` stub (mappable or typed alike), or
    /// `None` if `name` is not a pending lazy activation entry.
    ///
    /// Since the two kinds share one namespace, `command-plugin` and `:`
    /// Tab's activation of a lazy command's plugin read this rather than
    /// matching on `Command::Mappable`/`Command::Typed` themselves.
    pub(in crate::editor) fn lazy_owner(
        &self,
        name: &str,
    ) -> Option<&hume_scripting::attribution::EntryId> {
        lazy_owner_of(self.entry(name)?)
    }

    /// The plugin owning `name`'s *mappable* `Lazy` stub, or `None` if `name`
    /// has no pending mappable activation, even when a typed stub of the
    /// same name exists.
    ///
    /// Used only by `%lazy-command-owner` (the `call!`/`%dispatch-command!`
    /// lazy-activation check): `call!` can never reach a typed command, so
    /// that path must not see a typed-only stub as activatable. Every other
    /// caller wants [`Self::lazy_owner`]'s kind-agnostic answer.
    pub(in crate::editor) fn lazy_mappable_owner(
        &self,
        name: &str,
    ) -> Option<&hume_scripting::attribution::EntryId> {
        match self.entry(name)? {
            Command::Mappable(MappableCommand::Lazy { plugin, .. }) => Some(plugin),
            _ => None,
        }
    }

    /// Every current `Lazy` stub as `(name, owning plugin, is_typed)`,
    /// mappable and typed alike.
    ///
    /// Used by `:plugin-status` (via `lazy_status_string`) to report which
    /// commands a `Declared` plugin is still waiting on. The registry is the
    /// sole owner of `Lazy` stubs, so this is the only source for that list.
    /// The `is_typed` tag lets the display tell the user whether a pending
    /// name will need a key binding or `:` once its plugin loads.
    pub(in crate::editor) fn lazy_stubs(
        &self,
    ) -> Vec<(String, hume_scripting::attribution::EntryId, bool)> {
        self.commands
            .values()
            .filter_map(|cmd| {
                lazy_owner_of(cmd)
                    .map(|plugin| (name_of(cmd).to_string(), plugin.clone(), is_typed_lazy(cmd)))
            })
            .collect()
    }

    /// What holds `name` (in any case): a native command (a native typed
    /// command's alias included), a Steel command, or a lazy stub.
    pub(in crate::editor) fn holder(
        &self,
        name: &str,
    ) -> Option<hume_scripting::host::CommandHolder> {
        use hume_scripting::host::{CommandHolder, CommandKind};
        let key = fold(name);
        let canonical = self
            .alias_map
            .get(key.as_ref())
            .map_or(key.as_ref(), |c| c.as_ref());
        let cmd = self.commands.get(canonical)?;
        Some(if let Some(entry) = lazy_owner_of(cmd) {
            CommandHolder::Lazy {
                entry: entry.clone(),
                kind: if is_typed_lazy(cmd) {
                    CommandKind::Typed
                } else {
                    CommandKind::Mappable
                },
            }
        } else if let Some(proc) = steel_proc_of(cmd) {
            CommandHolder::Defined(proc)
        } else {
            CommandHolder::Native
        })
    }

    /// The proc of the Steel command registered under `name`
    /// (in any case), mappable or typed. `None` for a native command, a
    /// `Lazy` stub, or an unknown name.
    pub(in crate::editor) fn steel_proc(&self, name: &str) -> Option<hume_scripting::CommandProc> {
        steel_proc_of(self.entry(name)?)
    }

    /// Remove every Steel command (mappable or typed) defined while
    /// `activation`'s body ran. Native commands and `Lazy` stubs are never
    /// matched.
    pub(in crate::editor) fn unregister_registered_in(
        &mut self,
        activation: &hume_scripting::attribution::EntryId,
    ) {
        self.commands.retain(|_, cmd| {
            !steel_proc_of(cmd).is_some_and(|proc| proc.registered_in(activation))
        });
    }

    /// Remove every remaining `Lazy` stub owned by `plugin`, mappable and
    /// typed alike.
    ///
    /// Never touches a resolved `SteelBacked`/`Steel` command, even one that
    /// just replaced a stub of the same name for this plugin.
    pub(in crate::editor) fn unregister_lazy_stubs_of(
        &mut self,
        plugin: &hume_scripting::attribution::EntryId,
    ) {
        self.commands
            .retain(|_, cmd| lazy_owner_of(cmd) != Some(plugin));
    }
}

/// A free function (not a `&self` method) so it can be called from inside
/// `self.commands`'s own iterator/`retain` closures without a borrow conflict.
fn lazy_owner_of(cmd: &Command) -> Option<&hume_scripting::attribution::EntryId> {
    match cmd {
        Command::Mappable(MappableCommand::Lazy { plugin, .. }) => Some(plugin),
        Command::Typed(TypedCommand {
            body: TypedBody::Lazy(plugin),
            ..
        }) => Some(plugin),
        _ => None,
    }
}

/// The proc of `cmd` if it is a Steel command, mappable or typed. A free
/// function for the same reason as [`lazy_owner_of`].
fn steel_proc_of(cmd: &Command) -> Option<hume_scripting::CommandProc> {
    match cmd {
        Command::Mappable(MappableCommand::SteelBacked(cmd)) => {
            Some(hume_scripting::CommandProc::Mappable(cmd.body.proc.clone()))
        }
        Command::Typed(TypedCommand {
            body: TypedBody::Steel(body),
            ..
        }) => Some(hume_scripting::CommandProc::Typed(body.proc.clone())),
        _ => None,
    }
}

/// `cmd`'s name as it was registered, where its key is folded.
fn name_of(cmd: &Command) -> &str {
    match cmd {
        Command::Mappable(cmd) => cmd.name(),
        Command::Typed(cmd) => &cmd.name,
    }
}

/// `true` if `cmd` is a *typed* `Lazy` stub.
fn is_typed_lazy(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::Typed(TypedCommand {
            body: TypedBody::Lazy(_),
            ..
        })
    )
}

// ── Test-only helpers ─────────────────────────────────────────────────────────

#[cfg(test)]
impl CommandRegistry {
    /// Iterate over all registered canonical command names (not aliases),
    /// folded.
    pub(in crate::editor) fn names(&self) -> impl Iterator<Item = &str> {
        self.commands.keys().map(|k| k.as_ref())
    }

    /// Collect the canonical names of every `SteelBacked` command. Test-only.
    pub(in crate::editor::registry) fn steel_backed_names(&self) -> Vec<String> {
        self.commands
            .values()
            .filter_map(|cmd| match cmd {
                Command::Mappable(MappableCommand::SteelBacked(cmd)) => {
                    Some(cmd.name.as_ref().to_string())
                }
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
