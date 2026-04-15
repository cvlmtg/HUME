//! `(define-command! name doc proc)` and `(call-command! name)` builtins.
//!
//! Steel commands are defined as zero-argument lambdas and composed via
//! `call-command!`, which queues named commands for dispatch after the
//! Steel proc returns.  The actual execution happens in
//! [`crate::scripting::ScriptingHost::call_steel_cmd`], which drains the
//! queue through the normal `execute_keymap_command` path.

use std::cell::RefCell;

use steel::rvals::SteelVal;
use steel::rerrs::{ErrorKind, SteelErr};

use crate::scripting::PendingSteelCmd;

type SteelResult = Result<SteelVal, SteelErr>;

// ── CMD_QUEUE ─────────────────────────────────────────────────────────────────

thread_local! {
    /// Commands queued by `(call-command! …)` during a Steel command invocation.
    ///
    /// `Some(queue)` while a `SteelBacked` command is executing; `None` otherwise.
    /// Accessing this outside a Steel command invocation raises a Steel error.
    pub(crate) static CMD_QUEUE: RefCell<Option<Vec<String>>> = RefCell::new(None);
}

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
pub(crate) fn define_command(args: &[SteelVal]) -> SteelResult {
    if args.len() != 3 {
        steel::stop!(ArityMismatch =>
            "define-command! expects 3 args (name doc proc), got {}", args.len());
    }

    let name = match &args[0] {
        SteelVal::StringV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "define-command!: first arg (name) must be a string, got {:?}", args[0]),
    };
    let doc = match &args[1] {
        SteelVal::StringV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "define-command!: second arg (doc) must be a string, got {:?}", args[1]),
    };
    // Accept any callable value as the proc.
    let proc = match &args[2] {
        SteelVal::Closure(_) | SteelVal::FuncV(_) | SteelVal::MutFunc(_) => args[2].clone(),
        _ => steel::stop!(TypeMismatch =>
            "define-command!: third arg (proc) must be a callable, got {:?}", args[2]),
    };

    super::with_ctx(|ctx| {
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
        ctx.pending_steel_cmds.push(PendingSteelCmd {
            name,
            doc,
            proc,
            current_owner,
        });
        Ok(SteelVal::Void)
    })
}

/// `(call-command! name)`
///
/// Queues `name` for execution after the current Steel command proc returns.
/// Commands are dispatched in order through the normal keymap path, which
/// means they have full access to editor state, jump-list tracking, etc.
///
/// Only valid inside a `SteelBacked` command invocation; raises a Steel error
/// if called from top-level `init.scm` (where `CMD_QUEUE` is `None`).
pub(crate) fn call_command(args: &[SteelVal]) -> SteelResult {
    if args.len() != 1 {
        steel::stop!(ArityMismatch =>
            "call-command! expects 1 arg (name), got {}", args.len());
    }
    let name = match &args[0] {
        SteelVal::StringV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "call-command!: arg must be a string, got {:?}", args[0]),
    };

    CMD_QUEUE.with(|cell| {
        match cell.borrow_mut().as_mut() {
            Some(queue) => {
                queue.push(name);
                Ok(SteelVal::Void)
            }
            None => Err(SteelErr::new(
                ErrorKind::Generic,
                "call-command!: not inside a Steel command invocation".to_string(),
            )),
        }
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_command_arity_error() {
        let err = call_command(&[]).unwrap_err();
        assert!(err.to_string().contains("expects 1 arg"), "got: {err}");
    }

    #[test]
    fn call_command_type_error() {
        let err = call_command(&[SteelVal::IntV(1)]).unwrap_err();
        assert!(err.to_string().contains("string"), "got: {err}");
    }

    #[test]
    fn call_command_outside_invocation_errors() {
        // CMD_QUEUE is None when not inside a Steel command execution.
        let err = call_command(&[SteelVal::StringV("move-right".into())]).unwrap_err();
        assert!(err.to_string().contains("not inside"), "got: {err}");
    }

    #[test]
    fn call_command_queues_name() {
        CMD_QUEUE.with(|cell| *cell.borrow_mut() = Some(Vec::new()));
        call_command(&[SteelVal::StringV("move-right".into())]).unwrap();
        let queue = CMD_QUEUE.with(|cell| cell.borrow_mut().take().unwrap());
        assert_eq!(queue, vec!["move-right"]);
    }
}
