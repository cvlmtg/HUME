//! Command registry queries, synchronous native dispatch, and Steel
//! command registration.

use crate::attribution::EntryId;
use crate::command_procs::CommandProc;
use crate::types::{SteelCmdDef, SteelTypedCmdDef};
use hume_core::{CommandArg, Count, PaneHandle};

/// How a native command is invoked from a script: everything
/// [`CommandHost::run_command_sync`] takes besides the command and its pane.
/// The default is a bare keypress: no count typed, no extend, no register,
/// no operand.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeCall {
    /// [`Count::UNTYPED`] means "as if no count was typed". For
    /// `move-down`/`move-up` this selects visual-line movement instead of
    /// buffer-line movement (every other native command runs it once).
    /// `parse_native_args` decodes a Steel-side count of `0` to it.
    pub count: Count,
    pub extend: bool,
    /// Armed as `state.register_prefix` before dispatch, so register-aware
    /// commands (`yank`, `delete`, `paste-after`, etc.) route to the right
    /// destination.
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
    /// Returns `Ok(true)` if `name` is a native (Rust-registered) command
    /// (`Motion`, `Selection`, `Edit`, or `EditorCmd`) whose only valid `call!`
    /// args are `count`, `extend`, and an optional trailing [`CommandArg`]. Returns `Ok(false)` for Steel-defined
    /// commands (`SteelBacked`, `Lazy`) that accept arbitrary positional args.
    /// Returns `Err(msg)`, the message `call!` raises, for a name `call!`
    /// cannot reach: unknown, a typed command, or a key-only command.
    ///
    /// Read-only: never executes the command. Hosts without a registry (test
    /// stubs) return `Ok(false)` to treat all commands as Steel/forward-raw.
    fn command_is_native(&self, name: &str) -> Result<bool, String>;

    /// Execute a named native command synchronously.
    ///
    /// All four native variants (`Motion`, `Selection`, `Edit`, `EditorCmd`) apply
    /// their effect immediately; a subsequent read in the same eval sees the new
    /// state. Non-native names (`SteelBacked`, `Lazy`) return `Err`: the
    /// implementation self-guards, so the caller need not pre-check via
    /// `command_is_native` (though doing so avoids a wasted lookup).
    ///
    /// `pane` names the pane (and its buffer) the command acts on, resolved
    /// against the command's own target requirement rather than always
    /// meaning "the focused pane": a command that needs any pane (`delete`,
    /// `yank`, …) runs through `pane.pane()` directly. It must still be
    /// live and show `pane.buffer()`, but need not be focused; a command
    /// bound to the focused pane specifically (`insert-before`, a paste or
    /// search-prompt entry, a tab or mode switch, …) requires `pane.pane()`
    /// to be the focused one. Every native command needs a pane; a
    /// buffer-only handle is refused. Either way the implementation errors rather than
    /// silently falling back to whatever is focused, so a hook or async
    /// callback whose captured `pane` no longer resolves gets a loud
    /// failure instead of quietly editing the wrong buffer.
    ///
    /// `call` carries the count, extend flag, register and operand; see
    /// [`NativeCall`].
    ///
    /// Returns `Ok(false)` if the command's body refused outright (a
    /// too-small split, the last pane, a read-only buffer, no stashed
    /// insertion, …). Refusal is reported to the user (as `Severity::Info`)
    /// before this returns, so the caller need not report it again. Returns
    /// `Ok(true)` otherwise: this is a negative signal only, not proof
    /// anything changed: a command that no-ops silently at a buffer edge, or
    /// one that exhausts mid-count without ever refusing (undo/redo past the
    /// last step), still returns `Ok(true)`. This is the value `(call! …)`
    /// yields for a native command.
    /// Returns `Err(msg)` when the name is not found, is not a native
    /// command, `pane`'s buffer isn't live, `pane` doesn't resolve
    /// against the command's own target requirement (see above), or
    /// `call.arg` doesn't match what the command takes.
    ///
    /// Valid only in command mode; gated by the caller's `cmd`-kind registration.
    fn run_command_sync(
        &mut self,
        name: &str,
        pane: PaneHandle,
        call: NativeCall,
    ) -> Result<bool, String>;

    /// What holds `name` (in any case), or `None` for a free name.
    fn command_holder(&self, name: &str) -> Option<CommandHolder>;

    /// Register a Steel command in the editor's `CommandRegistry`.
    ///
    /// Called inline from `define-command!` during init or plugin load.
    /// Refuses a name [`CommandProc::check_define`] refuses; the one holder
    /// it replaces is the defining body's own `Lazy` stub.
    fn register_command(&mut self, def: SteelCmdDef) -> Result<(), String>;

    /// Register a Steel *typed* command in the editor's `CommandRegistry`,
    /// invocable from the `:` command line.
    ///
    /// Called inline from `define-typed-command!` during init or plugin load.
    /// Same rule as [`Self::register_command`].
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

    /// Register `plugin`'s `Lazy` activation stubs: one per name in
    /// `commands` and in `typed_commands`, all distinct.
    ///
    /// Called from `declare-plugin!`, so the editor's `CommandRegistry` is
    /// the single place a name is claimed, with no separate scripting-side
    /// activation map. All or nothing: `Err` for the first name
    /// [`check_lazy_claim`] refuses, with no stub registered.
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

    /// The plugin that owns `name`'s *mappable* `Lazy` stub, or `None` if
    /// `name` has no pending mappable activation, even if a typed stub of
    /// the same name exists.
    ///
    /// Used by `%lazy-command-owner`, which backs `%dispatch-command!`'s
    /// (the `call!` path) lazy-activation branch: `call!` can only ever
    /// reach a mappable command's proc, so a typed-only name
    /// reported here would trigger a plugin load for an activation that can
    /// never succeed. [`Self::lazy_command_owner`] stays kind-agnostic for
    /// the callers that want either kind.
    fn lazy_mappable_command_owner(&self, name: &str) -> Option<EntryId>;

    /// Remove every remaining `Lazy` stub owned by `plugin`, mappable and
    /// typed alike.
    ///
    /// Called by `finish_lazy_activation` on both the success and failure
    /// path: on success, any stub the plugin body didn't itself replace via
    /// `define-command!`/`define-typed-command!` is dead weight (the plugin
    /// is now `Loaded` and will never re-run its body); on failure, every
    /// stub the plugin ever claimed must be freed so a later plugin can claim
    /// the name. Never removes a resolved `SteelBacked`/`Steel` command,
    /// only `Lazy` entries.
    fn unregister_lazy_stubs_of(&mut self, plugin: &EntryId);
}
