//! Command registry queries, synchronous native dispatch, and Steel
//! command registration.

use crate::attribution::EntryId;
use crate::command_procs::CommandProc;
use crate::types::{SteelCmdDef, SteelTypedCmdDef};
use hume_core::{CommandArg, Count, PaneHandle};

/// How a script invokes a native command: everything
/// [`CommandHost::run_command_sync`] takes besides the command and its pane.
/// The default is a bare keypress: no count, no extend, no register, no
/// operand.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeCall {
    /// [`Count::UNTYPED`] runs the command as if no count was typed, which
    /// for `move-down`/`move-up` means visual-line movement; other commands
    /// run once. `parse_native_args` decodes a Steel count of `0` to it.
    pub count: Count,
    pub extend: bool,
    /// Armed as `state.register_prefix` before dispatch, for register-aware
    /// commands (`yank`, `delete`, `paste-after`, …).
    pub register: Option<char>,
    /// The command's operand. It must match what the command takes: a
    /// character command (`find-forward`, `replace`, …) needs one, a prompt
    /// command (`search-forward`, `sift-within`) runs with it in place of
    /// opening its prompt, and any other command takes none.
    pub arg: Option<CommandArg>,
}

/// The two kinds of command, which share one namespace: an editor command
/// (`define-command!`, `#:commands`) and a typed command
/// (`define-typed-command!`, `#:typed-commands`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Mappable,
    Typed,
}

impl CommandKind {
    /// The `declare-plugin!` keyword that lists names of this kind.
    fn keyword(self) -> &'static str {
        match self {
            CommandKind::Mappable => "#:commands",
            CommandKind::Typed => "#:typed-commands",
        }
    }
}

/// What holds a command name, as [`CommandHost::command_holder`] reports it.
#[derive(Debug, Clone)]
pub enum CommandHolder {
    /// A native command, or a native typed command's alias.
    Native,
    /// A Steel command.
    Defined(CommandProc),
    /// A lazy entry's activation stub.
    Lazy { entry: EntryId, kind: CommandKind },
}

/// Whether `name`, which `holder` holds, is free for a command of `kind`
/// defined while `activation`'s body runs. The one rule every registration
/// goes through: a lazy entry's own stub of the same kind is the only
/// holder a command may replace.
pub(crate) fn check_name_free(
    name: &str,
    holder: Option<CommandHolder>,
    kind: CommandKind,
    activation: Option<&EntryId>,
) -> Result<(), String> {
    match holder {
        None => Ok(()),
        Some(CommandHolder::Native) => Err(format!(
            "'{name}' conflicts with a built-in command and cannot be redefined"
        )),
        Some(CommandHolder::Defined(proc)) => Err(format!(
            "command '{name}' is already defined by '{}'",
            proc.owner()
        )),
        Some(CommandHolder::Lazy { entry, kind: stub }) if activation == Some(&entry) => {
            if stub == kind {
                Ok(())
            } else {
                Err(format!(
                    "command '{name}' is declared by lazy plugin '{entry}' in {}",
                    stub.keyword()
                ))
            }
        }
        Some(CommandHolder::Lazy { entry, .. }) => Err(format!(
            "command '{name}' is already claimed as an activation command by lazy plugin '{entry}'"
        )),
    }
}

/// Whether `declare-plugin!` may claim `name`, which `holder` holds, as an
/// activation command. No plugin body runs at the top level it is called
/// from, so any holder refuses it.
pub fn check_lazy_claim(name: &str, holder: Option<CommandHolder>) -> Result<(), String> {
    check_name_free(name, holder, CommandKind::Mappable, None)
}

/// Command registry queries, synchronous native dispatch, and Steel command
/// registration, accessed through [`EditorHost::commands`](super::EditorHost::commands).
pub trait CommandHost {
    /// `Ok(true)` if `name` is a native command, whose only `call!` args are
    /// `count`, `extend` and an optional trailing [`CommandArg`]; `Ok(false)`
    /// for a Steel-defined or `Lazy` command, which takes any positional
    /// args. `Err` with the message `call!` raises for a name `call!` cannot
    /// reach: unknown, a typed command, or a key-only command. Never runs the
    /// command. A host without a registry (a test stub) returns `Ok(false)`.
    fn command_is_native(&self, name: &str) -> Result<bool, String>;

    /// Runs the native command `name` synchronously; a later read in the
    /// same eval sees its effect. A non-native name is an `Err`, so callers
    /// need not check [`Self::command_is_native`] first.
    ///
    /// `pane` names the pane and buffer the command acts on. A command that
    /// needs any pane (`delete`, `yank`, …) runs through `pane.pane()`, which
    /// must be live and show `pane.buffer()` but need not be focused; a
    /// command bound to the focused pane (`insert-before`, a paste or
    /// search-prompt entry, a tab or mode switch, …) requires it to be the
    /// focused one. A buffer-only handle is refused. A `pane` that no longer
    /// resolves is an error, never a fallback to the focused pane.
    ///
    /// `Ok(false)` when the command refused outright (a too-small split, the
    /// last pane, a read-only buffer, no stashed insertion, …), already
    /// reported to the user as `Severity::Info`. `Ok(true)` otherwise, which
    /// does not prove anything changed: a no-op at a buffer edge, or undo
    /// past the last step, still returns it. This is the value `(call! …)`
    /// yields. `Err` when the name is unknown, not native or key-only,
    /// `pane`'s buffer isn't live, `pane` doesn't resolve as above, or
    /// `call.arg` doesn't match what the command takes.
    ///
    /// Valid only in command mode; gated by the caller's `cmd`-kind registration.
    fn run_command_sync(
        &mut self,
        name: &str,
        pane: PaneHandle,
        call: NativeCall,
    ) -> Result<bool, String>;

    /// Opens the group of native commands one Steel session runs through
    /// [`Self::run_command_sync`]: together they change each pane's
    /// selections by one selection-undo step. Called once per session,
    /// before its body; sessions never nest.
    fn open_dispatch_group(&mut self);

    /// Closes the group [`Self::open_dispatch_group`] opened, whether the
    /// session returned, raised or ran out of budget.
    fn close_dispatch_group(&mut self);

    /// What holds `name` (in any case), or `None` for a free name.
    fn command_holder(&self, name: &str) -> Option<CommandHolder>;

    /// Registers a Steel command from `define-command!`, inline during init
    /// or plugin load. Refuses a name [`CommandProc::check_define`] refuses;
    /// the one holder it replaces is the defining body's own `Lazy` stub.
    fn register_command(&mut self, def: SteelCmdDef) -> Result<(), String>;

    /// Registers a Steel typed command from `define-typed-command!`, under
    /// [`Self::register_command`]'s naming rule.
    fn register_typed_command(&mut self, def: SteelTypedCmdDef) -> Result<(), String>;

    /// The proc of the Steel command (mappable or typed) registered under
    /// `name` (in any case), or `None` for a native command, a `Lazy`
    /// stub, or an unknown name.
    fn defined_command(&self, name: &str) -> Option<CommandProc>;

    /// Remove every Steel command (mappable or typed) defined while
    /// `activation`'s body ran. The rollback of a failed activation. Native
    /// commands and `Lazy` stubs are never removed.
    fn unregister_commands_registered_in(&mut self, activation: &EntryId);

    /// Whether `ch` names a valid register (`0`–`9`, `k`, `c`, `b`).
    fn is_valid_register_name(&self, ch: char) -> bool;

    /// Registers `plugin`'s `Lazy` activation stubs for `declare-plugin!`,
    /// one per name in `commands` and in `typed_commands`, all distinct. All
    /// or nothing: `Err` for the first name [`check_lazy_claim`] refuses,
    /// with no stub registered.
    fn register_lazy_commands(
        &mut self,
        plugin: &EntryId,
        commands: &[String],
        typed_commands: &[String],
    ) -> Result<(), String>;

    /// The plugin that owns `name`'s `Lazy` stub (mappable or typed alike),
    /// or `None` if `name` is not a pending lazy activation entry (already
    /// activated, never declared, or a non-lazy command).
    fn lazy_command_owner(&self, name: &str) -> Option<EntryId>;

    /// The plugin that owns `name`'s mappable `Lazy` stub, or `None` if it
    /// has no pending mappable activation, even when a typed stub of that
    /// name exists. `call!`'s lazy-activation path uses it: `call!` reaches
    /// only mappable procs, so a typed-only name would load a plugin for an
    /// activation that cannot succeed.
    fn lazy_mappable_command_owner(&self, name: &str) -> Option<EntryId>;

    /// Removes every remaining `Lazy` stub `plugin` owns, mappable and typed
    /// alike, after its activation either way. On success an unreplaced stub
    /// is dead, since the body never runs again; on failure every claimed
    /// name must be freed for a later plugin. Resolved Steel commands stay.
    fn unregister_lazy_stubs_of(&mut self, plugin: &EntryId);
}
