//! `(define-command! name doc proc)`, `(call! name)`, and
//! `(request-wait-char! cmd)` builtins.
//!
//! Steel commands are defined as zero-argument lambdas and composed via
//! `call!`, which queues named commands for dispatch after the Steel proc
//! returns.  The actual execution happens in
//! [`crate::scripting::ScriptingHost::call_steel_cmd`], which drains the
//! queue through the normal `execute_keymap_command` path.
//!
//! `call-command!` is a back-compat alias for `call!`; prefer `call!` in new code.
//!
//! `request-wait-char!` allows a Steel command to request that after the
//! queue is drained, the editor enters WaitChar mode for the named command.
//! This enables multi-step compositions like `mr` + old_char + new_char.
//!
//! ## Invocation contract
//!
//! All commands — Rust built-ins and `define-command!`-registered Steel
//! lambdas alike — are invoked uniformly by string name:
//!
//! ```scheme
//! (call! "collapse-selection")   ; built-in
//! (call! "my-plugin-cmd")        ; defined by another (or the same) plugin
//! ```
//!
//! Steel lambdas registered via `define-command!` are intentionally **not**
//! exposed as bare Scheme identifiers (they live under a private mangled
//! name in the engine namespace).  This keeps the call site symmetric with
//! built-ins (which are Rust `MappableCommand` variants and have no Scheme
//! binding), and ensures every invocation goes through the registry path
//! that owns ledger attribution, watchdog protection, and dispatch parity
//! with `:cmd` and `bind-key!`.

use steel::rvals::SteelVal;
use steel::rerrs::SteelErr;

use crate::scripting::{PendingSteelCmd, SteelCtx};
use crate::scripting::ledger::Owner;
use super::require_cmd_ctx;

type SteelResult = Result<SteelVal, SteelErr>;

// ── Builtins ──────────────────────────────────────────────────────────────────

/// `(define-command! name doc proc)`
///
/// Registers `proc` (a zero-argument Steel lambda) as a mappable command with
/// the given `name` and `doc` string.  The command can then be bound to a key
/// via `(bind-key! …)`.
///
/// Raises a Steel error if:
/// - `name` conflicts with a core built-in command.
/// - The same name is defined twice within one eval session.
/// - Called from a command body (only valid during init.scm or plugin load).
pub(crate) fn define_command(ctx: &mut SteelCtx, name: String, doc: String, proc: SteelVal) -> SteelResult {
    if !ctx.is_init {
        steel::stop!(Generic =>
            "define-command!: only valid during init.scm or plugin load, not from a Steel command body");
    }
    // Accept any callable value as the proc.
    match &proc {
        SteelVal::Closure(_) | SteelVal::FuncV(_) | SteelVal::MutFunc(_) => {}
        _ => steel::stop!(TypeMismatch =>
            "define-command!: third arg (proc) must be a callable, got {:?}", proc),
    }
    // Conflict against core/user built-ins known at eval start.
    if ctx.builtin_cmd_names.contains(&name) {
        steel::stop!(Generic =>
            "define-command!: '{}' conflicts with a built-in command and cannot be redefined",
            name);
    }
    // Conflict within this single eval session (e.g. two `define-command!` for same name).
    if ctx.pending_steel_cmds.iter().any(|c| c.name == name) {
        steel::stop!(Generic =>
            "define-command!: '{}' is already defined in this eval session", name);
    }
    let current_owner = ctx.plugin_stack.current_owner();
    ctx.pending_steel_cmds.push(PendingSteelCmd { name, doc, proc, current_owner });
    Ok(SteelVal::Void)
}

/// `(call! name)` — also available as `(call-command! name)` (back-compat alias)
///
/// Queues `name` for execution after the current Steel command proc returns.
/// Commands are dispatched in order through the normal keymap path, which
/// means they have full access to editor state, jump-list tracking, etc.
///
/// Only valid inside a `SteelBacked` command invocation; raises a Steel error
/// if called from top-level `init.scm`.
pub(crate) fn call_command(ctx: &mut SteelCtx, name: String) -> SteelResult {
    require_cmd_ctx!(ctx, "call!");
    ctx.cmd_queue.push(name);
    Ok(SteelVal::Void)
}

/// `(request-wait-char! cmd-name)`
///
/// Requests that after the current Steel command's queue is fully drained,
/// the editor enters WaitChar mode for `cmd-name`.  The next character the
/// user types becomes `pending_char` and `cmd-name` is dispatched.
///
/// Typical use: composing surround-select with replace.
///   `(call! "surround-paren") (request-wait-char! "replace")`
/// selects the surrounding `()` pair, then waits for the replacement char.
///
/// Only valid inside a `SteelBacked` command invocation.
pub(crate) fn request_wait_char(ctx: &mut SteelCtx, cmd: String) -> SteelResult {
    require_cmd_ctx!(ctx, "request-wait-char!");
    ctx.wait_char_request = Some(cmd);
    Ok(SteelVal::Void)
}

/// `(cmd-arg)` — return the command-line argument string as a string,
/// or `#f` if no argument was supplied.
///
/// Meaningful only when the command was invoked via `:cmd arg` in the
/// mini-buffer.  Returns `#f` when invoked via a key binding or from
/// top-level `init.scm`.
pub(crate) fn cmd_arg(ctx: &mut SteelCtx) -> SteelResult {
    match ctx.cmd_arg.as_deref() {
        Some(s) => Ok(SteelVal::StringV(s.to_owned().into())),
        None    => Ok(SteelVal::BoolV(false)),
    }
}

/// `(command-plugin name)` — return the owner of command `name` as a string.
///
/// Returns the plugin id string (e.g. `"core:plum"`, `"user/repo"`) if the
/// command was registered by a plugin, `"user"` if registered from top-level
/// `init.scm`, or `"hume"` for built-in Rust commands (not Steel-registered).
///
/// Valid during both eval (e.g. conflict detection in `load-plugin`) and
/// command execution.  Returns `"hume"` for any name not in the owner cache
/// (unknown commands are implicitly built-in).
pub(crate) fn command_plugin(ctx: &mut SteelCtx, name: String) -> SteelResult {
    let owner = ctx.cmd_owners
        .get(&name)
        .cloned()
        .unwrap_or_else(|| Owner::Core.to_string());
    Ok(SteelVal::StringV(owner.into()))
}

/// `(pending-char)` — return the pending character as a one-character string,
/// or `#f` if no character is waiting.
///
/// Only meaningful inside a `SteelBacked` command invocation reached via a
/// WaitChar keymap node (e.g. `bind-wait-char!`).  Returns `#f` at any other
/// call site (top-level init.scm, commands not triggered via WaitChar, etc.).
pub(crate) fn pending_char(ctx: &mut SteelCtx) -> SteelResult {
    match ctx.pending_char {
        Some(ch) => Ok(SteelVal::StringV(ch.to_string().into())),
        None     => Ok(SteelVal::BoolV(false)),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripting::SteelCtxTestHarness;

    #[test]
    fn call_command_outside_invocation_errors() {
        // is_init = true simulates "not inside a Steel command invocation"
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        ctx.is_init = true;
        let err = call_command(&mut ctx, "move-right".to_string()).unwrap_err();
        assert!(err.to_string().contains("not available during init"), "got: {err}");
    }

    #[test]
    fn call_command_queues_name() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        call_command(&mut ctx, "move-right".to_string()).unwrap();
        assert_eq!(ctx.cmd_queue, vec!["move-right"]);
    }

    #[test]
    fn call_bang_queues_multiple_names() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        call_command(&mut ctx, "move-right".to_string()).unwrap();
        call_command(&mut ctx, "move-left".to_string()).unwrap();
        assert_eq!(ctx.cmd_queue, vec!["move-right", "move-left"]);
    }

    #[test]
    fn request_wait_char_outside_invocation_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        ctx.is_init = true;
        let err = request_wait_char(&mut ctx, "replace".to_string()).unwrap_err();
        assert!(err.to_string().contains("not available during init"), "got: {err}");
    }

    #[test]
    fn request_wait_char_stores_cmd() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        request_wait_char(&mut ctx, "replace".to_string()).unwrap();
        assert_eq!(ctx.wait_char_request, Some("replace".to_string()));
    }

    #[test]
    fn cmd_arg_returns_false_when_none() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = cmd_arg(&mut ctx).unwrap();
        assert_eq!(result, SteelVal::BoolV(false));
    }

    #[test]
    fn cmd_arg_returns_string_when_set() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        ctx.cmd_arg = Some("user/repo".to_string());
        let result = cmd_arg(&mut ctx).unwrap();
        assert_eq!(result, SteelVal::StringV("user/repo".into()));
    }

    #[test]
    fn pending_char_returns_false_when_none() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = pending_char(&mut ctx).unwrap();
        assert_eq!(result, SteelVal::BoolV(false));
    }

    #[test]
    fn pending_char_returns_string_when_set() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        ctx.pending_char = Some('(');
        let result = pending_char(&mut ctx).unwrap();
        assert_eq!(result, SteelVal::StringV("(".into()));
    }

    #[test]
    fn command_plugin_unknown_returns_hume() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = command_plugin(&mut ctx, "move-right".to_string()).unwrap();
        assert_eq!(result, SteelVal::StringV("hume".into()));
    }

    #[test]
    fn command_plugin_known_returns_owner() {
        let mut h = SteelCtxTestHarness::new();
        h.cmd_owners.insert("my-cmd".to_string(), "core:plum".to_string());
        let mut ctx = h.ctx();
        let result = command_plugin(&mut ctx, "my-cmd".to_string()).unwrap();
        assert_eq!(result, SteelVal::StringV("core:plum".into()));
    }
}
