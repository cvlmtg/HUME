//! `(define-command! name doc proc)`, `(call! name args…)`, and
//! `(request-wait-char! cmd)` builtins.
//!
//! Steel commands are defined as lambdas and composed via `call!`, a BOOTSTRAP
//! macro that desugars `(call! name a b…)` to `(%call! name (list a b…))`.
//! The `%call!` Rust primitive queues `(name, args)` for dispatch after the
//! Steel proc returns.  The actual execution happens in
//! [`crate::scripting::ScriptingHost::call_steel_cmd`], which drains the
//! queue through the normal `execute_keymap_command` path.
//!
//! `request-wait-char!` allows a Steel command to request that after the
//! queue is drained, the editor enters WaitChar mode for the named command.
//! This enables multi-step compositions like `mr` + old_char + new_char.
//!
//! ## Invocation contract
//!
//! All commands — Rust built-ins and `define-command!`-registered Steel
//! lambdas alike — are invoked uniformly by string name with optional args:
//!
//! ```scheme
//! (call! "collapse-selection")        ; built-in, no args
//! (call! "my-plugin-cmd" "arg1")      ; Steel command with one arg
//! ```
//!
//! Steel lambdas registered via `define-command!` are intentionally **not**
//! exposed as bare Scheme identifiers (they live under a private mangled
//! name in the engine namespace).  This keeps the call site symmetric with
//! built-ins (which are Rust `MappableCommand` variants and have no Scheme
//! binding), and ensures every invocation goes through the registry path
//! that owns command attribution, watchdog protection, and dispatch parity
//! with `:cmd` and `bind-key!`.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use super::require_cmd_ctx;
use crate::scripting::attribution::Owner;
use crate::scripting::{PendingSteelCmd, SteelCtx};

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
pub(crate) fn define_command(
    ctx: &mut SteelCtx,
    name: String,
    doc: String,
    proc: SteelVal,
) -> SteelResult {
    define_command_inner(ctx, "define-command!", name, doc, proc, false, false)
}

/// `(define-command-extend! name doc proc)`
///
/// Like `(define-command! …)` but marks the command as extendable
/// (`is_extendable() == true`).  Extendable Steel commands participate in the
/// sticky-Ctrl / strip-Ctrl one-shot extend mechanism: pressing `Ctrl-X` when
/// `X` is bound to an extendable Steel command dispatches it in extend mode,
/// just like built-in motion commands.
///
/// Use this for composite commands that end in a motion or selection step so
/// that `Ctrl-X` continues to work after binding `X` to the Steel command.
///
/// Same error conditions as `(define-command! …)`.
pub(crate) fn define_command_extend(
    ctx: &mut SteelCtx,
    name: String,
    doc: String,
    proc: SteelVal,
) -> SteelResult {
    define_command_inner(ctx, "define-command-extend!", name, doc, proc, true, false)
}

/// `(define-command-inline-output! name doc proc)`
///
/// Like `(define-command! …)` but brackets dispatch with an alt-screen exit so
/// the command's subprocess output streams live to the terminal rather than
/// dumping to the message bar. Use for shell-outs (plum install, formatters,
/// linters). The editor re-enters the alt-screen after a keypress.
///
/// Same error conditions as `(define-command! …)`.
pub(crate) fn define_command_inline_output(
    ctx: &mut SteelCtx,
    name: String,
    doc: String,
    proc: SteelVal,
) -> SteelResult {
    define_command_inner(ctx, "define-command-inline-output!", name, doc, proc, false, true)
}

fn define_command_inner(
    ctx: &mut SteelCtx,
    builtin_name: &str,
    name: String,
    doc: String,
    proc: SteelVal,
    extendable: bool,
    inline_output: bool,
) -> SteelResult {
    if !ctx.is_init {
        steel::stop!(Generic =>
            "{}: only valid during init.scm or plugin load, not from a Steel command body",
            builtin_name);
    }
    match &proc {
        SteelVal::Closure(_) | SteelVal::FuncV(_) | SteelVal::MutFunc(_) => {}
        _ => steel::stop!(TypeMismatch =>
            "{}: third arg (proc) must be a callable, got {:?}", builtin_name, proc),
    }
    if ctx.builtin_cmd_names.contains(&name) {
        steel::stop!(Generic =>
            "{}: '{}' conflicts with a built-in command and cannot be redefined",
            builtin_name, name);
    }
    if ctx.pending_steel_cmds.iter().any(|c| c.name == name) {
        steel::stop!(Generic =>
            "{}: '{}' is already defined in this eval session", builtin_name, name);
    }
    let current_owner = ctx.plugin_stack.current_owner();
    ctx.pending_steel_cmds.push(PendingSteelCmd {
        name,
        doc,
        proc,
        current_owner,
        extendable,
        inline_output,
    });
    Ok(SteelVal::Void)
}

/// `%call!` — fixed-arity-2 Rust primitive underlying the `(call! name args…)` macro.
///
/// The BOOTSTRAP macro `(call! name args…)` desugars to `(%call! name (list args…))`,
/// so this function always receives `(name, args-list)`.  Queues `(name, args_vec)`
/// for execution after the current Steel command proc returns.
///
/// Only valid inside a `SteelBacked` command invocation; raises a Steel error
/// if called from top-level `init.scm`.
pub(crate) fn call_command_primitive(
    ctx: &mut SteelCtx,
    name: String,
    args: SteelVal,
) -> SteelResult {
    require_cmd_ctx!(ctx, "%call!");
    let args_vec = steel_list_to_vec(args)?;
    ctx.cmd_queue.push((name, args_vec));
    Ok(SteelVal::Void)
}

fn steel_list_to_vec(val: SteelVal) -> Result<Vec<SteelVal>, steel::rerrs::SteelErr> {
    match val {
        SteelVal::ListV(list) => Ok(list.into_iter().collect()),
        other => steel::stop!(TypeMismatch =>
            "%call!: second arg must be a list, got {:?}", other),
    }
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

/// `(command-plugin name)` — return the owner of command `name` as a string.
///
/// Returns the plugin id string (e.g. `"core:plum"`, `"user/repo"`) if the
/// command was registered by a plugin, `"user"` if registered from top-level
/// `init.scm`, or `"hume"` for built-in Rust commands (not Steel-registered).
///
/// Valid during both eval (e.g. conflict detection in `declare-plugin`) and
/// command execution.  Returns `"hume"` for any name not in the owner cache
/// (unknown commands are implicitly built-in).
pub(crate) fn command_plugin(ctx: &mut SteelCtx, name: String) -> SteelResult {
    let owner = ctx
        .cmd_owners
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
        None => Ok(SteelVal::BoolV(false)),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripting::SteelCtxTestHarness;

    fn make_list(vals: Vec<SteelVal>) -> SteelVal {
        SteelVal::ListV(vals.into_iter().collect())
    }

    #[test]
    fn call_bang_outside_invocation_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        ctx.is_init = true;
        let err = call_command_primitive(
            &mut ctx,
            "move-right".to_string(),
            make_list(vec![]),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("not available during init"),
            "got: {err}"
        );
    }

    #[test]
    fn call_bang_with_no_args_queues_empty_vec() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        call_command_primitive(&mut ctx, "move-right".to_string(), make_list(vec![])).unwrap();
        assert_eq!(
            ctx.cmd_queue,
            vec![("move-right".to_string(), vec![])]
        );
    }

    #[test]
    fn call_bang_with_args_queues_name_and_args() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let arg = SteelVal::StringV("hello".into());
        call_command_primitive(&mut ctx, "echo".to_string(), make_list(vec![arg.clone()])).unwrap();
        assert_eq!(ctx.cmd_queue, vec![("echo".to_string(), vec![arg])]);
    }

    #[test]
    fn call_bang_queues_multiple_names() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        call_command_primitive(&mut ctx, "move-right".to_string(), make_list(vec![])).unwrap();
        call_command_primitive(&mut ctx, "move-left".to_string(), make_list(vec![])).unwrap();
        let names: Vec<&str> = ctx.cmd_queue.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["move-right", "move-left"]);
    }

    #[test]
    fn request_wait_char_outside_invocation_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        ctx.is_init = true;
        let err = request_wait_char(&mut ctx, "replace".to_string()).unwrap_err();
        assert!(
            err.to_string().contains("not available during init"),
            "got: {err}"
        );
    }

    #[test]
    fn request_wait_char_stores_cmd() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        request_wait_char(&mut ctx, "replace".to_string()).unwrap();
        assert_eq!(ctx.wait_char_request, Some("replace".to_string()));
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
        h.cmd_owners
            .insert("my-cmd".to_string(), "core:plum".to_string());
        let mut ctx = h.ctx();
        let result = command_plugin(&mut ctx, "my-cmd".to_string()).unwrap();
        assert_eq!(result, SteelVal::StringV("core:plum".into()));
    }
}
