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

/// Command registry queries, synchronous native dispatch, and Steel command
/// registration, accessed through [`EditorHost::commands`](super::EditorHost::commands).
pub trait CommandHost {
    /// Returns `Ok(true)` if `name` is a native (Rust-registered) command
    /// (`Motion`, `Selection`, `Edit`, or `EditorCmd`) whose only valid `call!`
    /// args are `count`, `extend`, and an optional trailing [`CommandArg`]. Returns `Ok(false)` for Steel-defined
    /// commands (`SteelBacked`, `Lazy`) that accept arbitrary positional args.
    /// Returns `Err(msg)` if the name is unknown.
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

    /// Register a Steel command in the editor's `CommandRegistry`.
    ///
    /// Called inline from `define-command!` during init or plugin load.
    /// Overwrites a `Lazy` stub for the same name (expected path: a lazy plugin
    /// body's `define-command!` replaces the activation command stub).
    /// Returns `Err(msg)` if the name conflicts with any non-Lazy existing command.
    fn register_command(&mut self, def: SteelCmdDef) -> Result<(), String>;

    /// Register a Steel *typed* command in the editor's `CommandRegistry`,
    /// invocable from the `:` command line.
    ///
    /// Called inline from `define-typed-command!` during init or plugin load.
    /// Overwrites a typed `Lazy` stub for the same name (expected path: a
    /// lazy plugin body's `define-typed-command!` replaces the activation
    /// stub declared via `#:typed-commands`).
    /// Returns `Err(msg)` if the name conflicts with any non-Lazy existing command.
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

    /// Register a `Lazy` activation stub for `name`, owned by `plugin`.
    ///
    /// Called from `declare-plugin!`'s `#:commands` processing, once per
    /// accepted command name, so the editor's `CommandRegistry` is the single
    /// place a name is claimed, with no separate scripting-side activation map.
    ///
    /// Returns `Err(msg)` if `name` is already claimed by any existing
    /// command (native, `SteelBacked`, or another plugin's `Lazy` stub); the
    /// message names the conflicting owner for a specific declare-time log.
    fn register_lazy_command(&mut self, name: &str, plugin: &EntryId) -> Result<(), String>;

    /// Register a typed `Lazy` activation stub for `name`, owned by `plugin`.
    ///
    /// Called from `declare-plugin!`'s `#:typed-commands` processing: the
    /// typed counterpart of [`Self::register_lazy_command`]. Same conflict
    /// rules, same message shape.
    fn register_lazy_typed_command(&mut self, name: &str, plugin: &EntryId) -> Result<(), String>;

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
