//! `(define-command! name doc proc)`, `(call! name args…)`, and
//! `(request-wait-char! cmd)` builtins.
//!
//! ## Dispatch model
//!
//! `(call! name args…)` expands (via the BOOTSTRAP macro) to
//! `(%dispatch-command name (list args…))`, a Steel function that routes:
//!
//! - **Activated plugin commands** (in `command_table`): applied directly as an
//!   ordinary Steel funcall via `(apply proc args)` — never leaving the VM.
//!   State reads after the call see its effects immediately.
//! - **Lazy triggers** (unactivated plugin): `%dispatch-command` activates the
//!   owner inline via `%activate-plugin-inline`, then retries.
//! - **Native commands**: forwarded to `%call-native!` → `run_command_sync` inline.
//!   Init mode: hard error (buffer access not available during init).
//! - **Unknown**: forwarded to `%call-native!` → warning logged, no-op.
//!
//! `request-wait-char!` allows a Steel command to request that after the
//! current eval finishes, the editor enters WaitChar mode for the named command.
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

use steel::rerrs::{ErrorKind, SteelErr};
use steel::rvals::SteelVal;

use super::require_cmd_ctx;
use crate::attribution::Owner;
use crate::log::LogLevel;
use crate::types::SteelCmdDef;
use crate::SteelCtx;

type SteelResult = Result<SteelVal, SteelErr>;

// ── Builtins ──────────────────────────────────────────────────────────────────

/// `(define-command! name doc proc)`
///
/// Registers `proc` (a Steel lambda) as a mappable command with the given
/// `name` and `doc` string.  `proc` may accept positional arguments passed via
/// `(call! name arg …)`.  The command can then be bound to a key via
/// `(bind-key! …)`.
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
    if !ctx.is_init && ctx.plugin_stack.is_empty() {
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
    if ctx.registries.command_table.contains_key(&name) {
        steel::stop!(Generic =>
            "{}: '{}' is already defined in this eval session", builtin_name, name);
    }
    let (arity, is_variadic) = match &proc {
        SteelVal::Closure(gc) => (gc.arity() as u16, gc.is_multi_arity()),
        _ => (0, true),
    };
    let current_owner = ctx.plugin_stack.current_owner();
    ctx.registries.command_table.insert(name.clone(), proc);
    ctx.registries.cmd_owners.insert(name.clone(), current_owner.to_string());
    // Register inline in the editor's CommandRegistry so subsequent keypresses
    // find SteelBacked entries immediately — no post-eval second pass.
    ctx.host.register_command(SteelCmdDef { name, doc, extendable, arity, is_variadic, inline_output })
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e))?;
    Ok(SteelVal::Void)
}

/// `%call-native!` — Rust leaf for native/unknown commands.
///
/// Called by `%dispatch-command` when `name` is NOT found in `command_table`
/// and has no lazy-trigger owner (i.e. it is a native or unknown command).
///
/// - **Native** (`Motion`/`Selection`/`Edit`/`EditorCmd`): in command mode,
///   validates count/extend args and runs synchronously via `run_command_sync`.
///   In init mode, raises a hard error — native commands touch buffers which
///   are not available during init.scm evaluation.
/// - **Unknown / Steel-but-not-in-table**: logs a `Warning` and returns `#void`.
///   This covers commands not yet registered (typo, missing plugin, unknown name).
pub(crate) fn call_command_primitive(
    ctx: &mut SteelCtx,
    name: String,
    args: SteelVal,
) -> SteelResult {
    let args_vec = steel_list_to_vec(args)?;

    match ctx.host.command_is_native(&name) {
        Ok(true) => {
            if ctx.is_init {
                steel::stop!(Generic =>
                    "%call-native!: '{}' cannot run during init.scm — buffer access not available",
                    name);
            }
            let (count, extend) = parse_count_extend(&args_vec)
                .map_err(|e| SteelErr::new(ErrorKind::Generic, format!("%call-native!: {e}")))?;
            ctx.host.run_command_sync(&name, count, extend, ctx.current_register_prefix)
                .map(|()| SteelVal::Void)
                .map_err(|e| SteelErr::new(ErrorKind::Generic, format!("%call-native!: {e}")))
        }
        Err(_) | Ok(false) => {
            ctx.log(LogLevel::Warning, format!("unknown command: {name}"));
            Ok(SteelVal::Void)
        }
    }
}

/// `%lookup-plugin-proc` — return the Steel closure for an activated plugin
/// command, or `#f` if the name is not in the `command_table`.
///
/// Works in both init and command mode: during init, `define-command!` populates
/// `command_table` inline, so `(call! "cmd")` that follows a `(load-plugin …)`
/// in the same init.scm body finds the closure immediately.
pub(crate) fn lookup_plugin_proc(ctx: &mut SteelCtx, name: String) -> SteelResult {
    match ctx.registries.command_table.get(&name) {
        Some(val) => Ok(val.clone()),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// Parse count/extend from a native command's args list.
///
/// Valid shapes: `[]` → `(1, false)`; `[n]` → `(n, false)`; `[n, bool]` → `(n, bool)`.
/// All other shapes (e.g. a leading string, extra args) return `Err`.
///
/// Re-exported from the crate root so the editor crate can reuse it when
/// draining a deferred native command whose count was stored in `QueuedCommand.args`.
pub fn parse_count_extend(args: &[SteelVal]) -> Result<(usize, bool), String> {
    match args {
        [] => Ok((1, false)),
        [SteelVal::IntV(n)] => Ok(((*n).max(1) as usize, false)),
        [SteelVal::IntV(n), SteelVal::BoolV(ext)] => Ok(((*n).max(1) as usize, *ext)),
        _ => Err(format!(
            "native command args must be [], [count], or [count extend]; got {:?}", args
        )),
    }
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
        .registries
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

/// `(set-register-prefix! name)` — arm a sticky register prefix for the
/// remaining `(call! …)` calls in this command body.
///
/// Every `(call! …)` after this point captures the given register, so
/// register-aware commands (paste, yank, delete, change) will use it.
/// The prefix persists until you call `set-register-prefix!` again with a
/// different name.
///
/// Valid register names: `0`–`9`, `k` (kill-ring head), `c` (clipboard),
/// `b` (black hole).  Any other name raises a Steel error immediately (fail
/// fast at command-body time, not at dispatch time).
///
/// Only valid inside a `SteelBacked` command or hook invocation.
pub(crate) fn set_register_prefix(ctx: &mut SteelCtx, name: String) -> SteelResult {
    require_cmd_ctx!(ctx, "set-register-prefix!");
    let mut chars = name.chars();
    let reg = match (chars.next(), chars.next()) {
        (Some(c), None) => c,
        _ => steel::stop!(Generic =>
            "set-register-prefix!: expected a single-character register name, got {:?}", name),
    };
    if !ctx.host.is_valid_register_name(reg) {
        steel::stop!(Generic =>
            "set-register-prefix!: invalid register '{}'; valid: 0-9, k, c, b", reg);
    }
    ctx.current_register_prefix = Some(reg);
    Ok(SteelVal::Void)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::SteelCtxTestHarness;

    fn make_list(vals: Vec<SteelVal>) -> SteelVal {
        SteelVal::ListV(vals.into_iter().collect())
    }

    /// NullHost returns `Ok(false)` for `command_is_native` → unknown path → warning logged.
    #[test]
    fn call_bang_unknown_command_logs_warning() {
        let mut h = SteelCtxTestHarness::new();
        {
            let mut ctx = h.ctx_init();
            call_command_primitive(&mut ctx, "plum-ensure-grammars".to_string(), make_list(vec![])).unwrap();
        }
        assert!(
            h.pending_messages.iter().any(|(_, msg)| msg.contains("plum-ensure-grammars")),
            "unknown command must log a warning; got: {:?}", h.pending_messages
        );
    }

    #[test]
    fn call_bang_unknown_command_command_mode_logs_warning() {
        let mut h = SteelCtxTestHarness::new();
        {
            let mut ctx = h.ctx();
            call_command_primitive(&mut ctx, "move-right".to_string(), make_list(vec![])).unwrap();
        }
        assert!(
            h.pending_messages.iter().any(|(_, msg)| msg.contains("move-right")),
            "unknown command in command mode must log a warning; got: {:?}", h.pending_messages
        );
    }

    #[test]
    fn call_bang_multiple_unknown_commands_each_log_warning() {
        let mut h = SteelCtxTestHarness::new();
        {
            let mut ctx = h.ctx();
            call_command_primitive(&mut ctx, "move-right".to_string(), make_list(vec![])).unwrap();
            call_command_primitive(&mut ctx, "move-left".to_string(), make_list(vec![])).unwrap();
        }
        let has_right = h.pending_messages.iter().any(|(_, msg)| msg.contains("move-right"));
        let has_left = h.pending_messages.iter().any(|(_, msg)| msg.contains("move-left"));
        assert!(has_right && has_left, "each unknown command must produce a warning");
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

    // ── parse_count_extend direct tests ──────────────────────────────────────

    #[test]
    fn parse_count_extend_empty_gives_defaults() {
        assert_eq!(parse_count_extend(&[]).unwrap(), (1, false));
    }

    #[test]
    fn parse_count_extend_count_only() {
        assert_eq!(parse_count_extend(&[SteelVal::IntV(5)]).unwrap(), (5, false));
    }

    #[test]
    fn parse_count_extend_count_and_extend() {
        assert_eq!(
            parse_count_extend(&[SteelVal::IntV(3), SteelVal::BoolV(true)]).unwrap(),
            (3, true)
        );
    }

    /// Negative counts clamp to 1 — `(*n).max(1) as usize`.
    #[test]
    fn parse_count_extend_negative_clamps_to_one() {
        assert_eq!(parse_count_extend(&[SteelVal::IntV(-7)]).unwrap(), (1, false));
        assert_eq!(
            parse_count_extend(&[SteelVal::IntV(-1), SteelVal::BoolV(false)]).unwrap(),
            (1, false)
        );
    }

    /// Zero also clamps to 1 (max(0, 1) = 1).
    #[test]
    fn parse_count_extend_zero_clamps_to_one() {
        assert_eq!(parse_count_extend(&[SteelVal::IntV(0)]).unwrap(), (1, false));
    }

    #[test]
    fn parse_count_extend_string_arg_is_err() {
        let bad = &[SteelVal::StringV("garbage".into())];
        assert!(parse_count_extend(bad).is_err());
    }

    #[test]
    fn parse_count_extend_extra_arg_is_err() {
        let bad = &[SteelVal::IntV(1), SteelVal::BoolV(false), SteelVal::IntV(0)];
        assert!(parse_count_extend(bad).is_err());
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
        h.registries
            .cmd_owners
            .insert("my-cmd".to_string(), "core:plum".to_string());
        let mut ctx = h.ctx();
        let result = command_plugin(&mut ctx, "my-cmd".to_string()).unwrap();
        assert_eq!(result, SteelVal::StringV("core:plum".into()));
    }
}
