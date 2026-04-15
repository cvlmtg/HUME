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

use std::cell::RefCell;

use steel::rvals::SteelVal;
use steel::rerrs::{ErrorKind, SteelErr};

use crate::scripting::PendingSteelCmd;

type SteelResult = Result<SteelVal, SteelErr>;

// ── TLS slots ─────────────────────────────────────────────────────────────────

thread_local! {
    /// Commands queued by `(call! …)` (or its alias `call-command!`) during a Steel command invocation.
    ///
    /// `Some(queue)` while a `SteelBacked` command is executing; `None` otherwise.
    /// Accessing this outside a Steel command invocation raises a Steel error.
    pub(crate) static CMD_QUEUE: RefCell<Option<Vec<String>>> = RefCell::new(None);

    /// Snapshot of the command-owner index for the current eval or command
    /// invocation.  Populated from `ScriptFacingCtx::cmd_owners` before each
    /// eval and before each `call_steel_cmd`.  Cleared afterwards.
    ///
    /// Maps command name → owner display string (`"hume"`, `"user"`, plugin id).
    pub(crate) static COMMAND_OWNER_CACHE: RefCell<std::collections::HashMap<String, String>> =
        RefCell::new(std::collections::HashMap::new());

    /// WaitChar command requested by `(request-wait-char! cmd)`.
    ///
    /// `Some(None)` = inside invocation, no request yet.
    /// `Some(Some(name))` = request pending for `name`.
    /// `None` = outside invocation (error if accessed).
    pub(crate) static WAIT_CHAR_REQUEST: RefCell<Option<Option<String>>> = RefCell::new(None);

    /// The pending character passed to the current Steel command from a WaitChar
    /// keymap node (e.g. `md` + `(` sets `pending_char = '('`).
    ///
    /// `Some(ch)` while a `SteelBacked` command is executing with a pending char;
    /// `None` if no char was waiting (or outside a command invocation).
    /// Accessible via the `(pending-char)` Steel builtin.
    pub(crate) static PENDING_CHAR: RefCell<Option<char>> = RefCell::new(None);

    /// The command-line argument string for the current Steel command
    /// (e.g. `:plum-install user/repo` sets arg to `"user/repo"`).
    ///
    /// `Some(arg)` when the command was invoked via `:cmd arg` in the mini-buffer;
    /// `None` when invoked via a key binding (no arg) or outside a command invocation.
    /// Accessible via the `(cmd-arg)` Steel builtin.
    pub(crate) static CMD_ARG: RefCell<Option<String>> = RefCell::new(None);
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

    super::with_ctx("define-command!", |ctx| {
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

/// `(call! name)` — also available as `(call-command! name)` (back-compat alias)
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
            "call! expects 1 arg (name), got {}", args.len());
    }
    let name = match &args[0] {
        SteelVal::StringV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "call!: arg must be a string, got {:?}", args[0]),
    };

    CMD_QUEUE.with(|cell| {
        match cell.borrow_mut().as_mut() {
            Some(queue) => {
                queue.push(name);
                Ok(SteelVal::Void)
            }
            None => Err(SteelErr::new(
                ErrorKind::Generic,
                "call!: not inside a Steel command invocation".to_string(),
            )),
        }
    })
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
pub(crate) fn request_wait_char(args: &[SteelVal]) -> SteelResult {
    if args.len() != 1 {
        steel::stop!(ArityMismatch =>
            "request-wait-char! expects 1 arg (cmd-name), got {}", args.len());
    }
    let cmd = match &args[0] {
        SteelVal::StringV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "request-wait-char!: arg must be a string, got {:?}", args[0]),
    };

    WAIT_CHAR_REQUEST.with(|cell| {
        match cell.borrow_mut().as_mut() {
            Some(slot) => { *slot = Some(cmd); Ok(SteelVal::Void) }
            None => Err(SteelErr::new(
                ErrorKind::Generic,
                "request-wait-char!: not inside a Steel command invocation".to_string(),
            )),
        }
    })
}

/// `(cmd-arg)` — return the command-line argument string as a string,
/// or `#f` if no argument was supplied.
///
/// Meaningful only when the command was invoked via `:cmd arg` in the
/// mini-buffer.  Returns `#f` when invoked via a key binding or from
/// top-level `init.scm`.
pub(crate) fn cmd_arg(args: &[SteelVal]) -> SteelResult {
    if !args.is_empty() {
        steel::stop!(ArityMismatch => "cmd-arg expects 0 args, got {}", args.len());
    }
    CMD_ARG.with(|cell| match cell.borrow().as_deref() {
        Some(s) => Ok(SteelVal::StringV(s.to_owned().into())),
        None    => Ok(SteelVal::BoolV(false)),
    })
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
pub(crate) fn command_plugin(args: &[SteelVal]) -> SteelResult {
    let name = super::one_string(args, "command-plugin")?;
    COMMAND_OWNER_CACHE.with(|cell| {
        let owner = cell.borrow()
            .get(&name)
            .cloned()
            .unwrap_or_else(|| "hume".to_string());
        Ok(SteelVal::StringV(owner.into()))
    })
}

/// `(pending-char)` — return the pending character as a one-character string,
/// or `#f` if no character is waiting.
///
/// Only meaningful inside a `SteelBacked` command invocation reached via a
/// WaitChar keymap node (e.g. `bind-wait-char!`).  Returns `#f` at any other
/// call site (top-level init.scm, commands not triggered via WaitChar, etc.).
pub(crate) fn pending_char(args: &[SteelVal]) -> SteelResult {
    if !args.is_empty() {
        steel::stop!(ArityMismatch => "pending-char expects 0 args, got {}", args.len());
    }
    PENDING_CHAR.with(|cell| match *cell.borrow() {
        Some(ch) => {
            let s = ch.to_string();
            Ok(SteelVal::StringV(s.into()))
        }
        None => Ok(SteelVal::BoolV(false)),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_command_arity_error() {
        let err = call_command(&[]).unwrap_err();
        assert!(err.to_string().contains("call! expects 1 arg"), "got: {err}");
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

    /// Both `call!` and `call-command!` route to the same Rust function, so
    /// calling it twice (once per alias) queues both names correctly.
    #[test]
    fn call_bang_alias_queues_same_as_call_command() {
        CMD_QUEUE.with(|cell| *cell.borrow_mut() = Some(Vec::new()));
        // Simulate `(call! "move-right")` — both aliases call the same fn.
        call_command(&[SteelVal::StringV("move-right".into())]).unwrap();
        call_command(&[SteelVal::StringV("move-left".into())]).unwrap();
        let queue = CMD_QUEUE.with(|cell| cell.borrow_mut().take().unwrap());
        assert_eq!(queue, vec!["move-right", "move-left"]);
    }

    #[test]
    fn request_wait_char_arity_error() {
        let err = request_wait_char(&[]).unwrap_err();
        assert!(err.to_string().contains("expects 1 arg"), "got: {err}");
    }

    #[test]
    fn request_wait_char_type_error() {
        let err = request_wait_char(&[SteelVal::IntV(1)]).unwrap_err();
        assert!(err.to_string().contains("string"), "got: {err}");
    }

    #[test]
    fn request_wait_char_outside_invocation_errors() {
        let err = request_wait_char(&[SteelVal::StringV("replace".into())]).unwrap_err();
        assert!(err.to_string().contains("not inside"), "got: {err}");
    }

    #[test]
    fn request_wait_char_stores_cmd() {
        WAIT_CHAR_REQUEST.with(|cell| *cell.borrow_mut() = Some(None));
        request_wait_char(&[SteelVal::StringV("replace".into())]).unwrap();
        let result = WAIT_CHAR_REQUEST.with(|cell| cell.borrow_mut().take().flatten());
        assert_eq!(result, Some("replace".to_string()));
    }

    #[test]
    fn cmd_arg_returns_false_when_none() {
        CMD_ARG.with(|cell| *cell.borrow_mut() = None);
        let result = cmd_arg(&[]).unwrap();
        assert_eq!(result, SteelVal::BoolV(false));
    }

    #[test]
    fn cmd_arg_returns_string_when_set() {
        CMD_ARG.with(|cell| *cell.borrow_mut() = Some("user/repo".to_string()));
        let result = cmd_arg(&[]).unwrap();
        assert_eq!(result, SteelVal::StringV("user/repo".into()));
        CMD_ARG.with(|cell| *cell.borrow_mut() = None);
    }

    #[test]
    fn cmd_arg_arity_error() {
        let err = cmd_arg(&[SteelVal::BoolV(false)]).unwrap_err();
        assert!(err.to_string().contains("expects 0 args"), "got: {err}");
    }

    #[test]
    fn pending_char_returns_false_when_none() {
        PENDING_CHAR.with(|cell| *cell.borrow_mut() = None);
        let result = pending_char(&[]).unwrap();
        assert_eq!(result, SteelVal::BoolV(false));
    }

    #[test]
    fn pending_char_returns_string_when_set() {
        PENDING_CHAR.with(|cell| *cell.borrow_mut() = Some('('));
        let result = pending_char(&[]).unwrap();
        assert_eq!(result, SteelVal::StringV("(".into()));
        PENDING_CHAR.with(|cell| *cell.borrow_mut() = None);
    }

    #[test]
    fn pending_char_arity_error() {
        let err = pending_char(&[SteelVal::BoolV(false)]).unwrap_err();
        assert!(err.to_string().contains("expects 0 args"), "got: {err}");
    }

    #[test]
    fn command_plugin_arity_error() {
        let err = command_plugin(&[]).unwrap_err();
        assert!(err.to_string().contains("expects 1 arg"), "got: {err}");
    }

    #[test]
    fn command_plugin_type_error() {
        let err = command_plugin(&[SteelVal::IntV(1)]).unwrap_err();
        assert!(err.to_string().contains("string"), "got: {err}");
    }

    #[test]
    fn command_plugin_unknown_returns_hume() {
        // Empty cache — any name not in the cache returns "hume".
        COMMAND_OWNER_CACHE.with(|cell| cell.borrow_mut().clear());
        let result = command_plugin(&[SteelVal::StringV("move-right".into())]).unwrap();
        assert_eq!(result, SteelVal::StringV("hume".into()));
    }

    #[test]
    fn command_plugin_known_returns_owner() {
        COMMAND_OWNER_CACHE.with(|cell| {
            cell.borrow_mut().insert("my-cmd".to_string(), "core:plum".to_string());
        });
        let result = command_plugin(&[SteelVal::StringV("my-cmd".into())]).unwrap();
        assert_eq!(result, SteelVal::StringV("core:plum".into()));
        COMMAND_OWNER_CACHE.with(|cell| cell.borrow_mut().clear());
    }
}
