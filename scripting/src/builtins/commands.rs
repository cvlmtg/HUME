//! `(define-command! name doc proc)`, `(call! name args…)`, and
//! `(request-wait-char! cmd)` builtins.
//!
//! Steel commands are defined as lambdas and composed via `call!`, a BOOTSTRAP
//! macro that desugars `(call! name a b…)` to `(%call! name (list a b…))`.
//! The `%call!` Rust primitive queues `(name, args)` for dispatch after the
//! Steel proc returns.  The actual execution happens in
//! [`crate::ScriptingHost::call_steel_cmd`], which drains the
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
//! name in the Steel engine namespace).  This keeps the call site symmetric with
//! built-ins (which are Rust `MappableCommand` variants and have no Scheme
//! binding), and ensures every invocation goes through the registry path
//! that owns command attribution, watchdog protection, and dispatch parity
//! with `:cmd` and `bind-key!`.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use super::require_cmd_ctx;
use crate::attribution::Owner;
use crate::types::QueuedCommand;
use crate::types::PendingSteelCmd;
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
/// so this function always receives `(name, args-list)`.
///
/// **Init mode**: always queues with the full args list; sync dispatch requires a
/// live `EditorHostImpl`.
///
/// **Command mode**: classifies first (`command_is_native`), then branches:
/// - **Native** (`Motion`/`Selection`/`Edit`/`EditorCmd`): arg contract is
///   count/extend only — `[]`, `[n]`, or `[n bool]`. Any other shape is
///   rejected via `steel::stop!` before executing the command (fail-fast).
///   When no Steel-defined command is ahead in the queue, runs synchronously
///   via `run_command_sync`; otherwise defers to preserve source order.
/// - **Steel-defined** (`SteelBacked`/`Lazy`): all args forwarded raw to the
///   deferred queue unchanged — no count stripping, no validation.
/// - **Unknown**: deferred to the queue; `execute_keymap_command` logs a warning
///   at drain time and continues — no mid-body abort.
pub(crate) fn call_command_primitive(
    ctx: &mut SteelCtx,
    name: String,
    args: SteelVal,
) -> SteelResult {
    let args_vec = steel_list_to_vec(args)?;

    // Init mode: always queue; sync dispatch requires a live EditorHostImpl.
    if ctx.is_init {
        ctx.cmd_queue.push(QueuedCommand { name, args: args_vec, register: ctx.current_register_prefix });
        return Ok(SteelVal::Void);
    }

    match ctx.host.command_is_native(&name) {
        // Unknown or Steel-defined — defer. Unknown names produce a warning at drain
        // (execute_keymap_command) while the rest of the body continues unaffected.
        Err(_) | Ok(false) => {
            ctx.cmd_queue.push(QueuedCommand {
                name,
                args: args_vec,
                register: ctx.current_register_prefix,
            });
            Ok(SteelVal::Void)
        }
        Ok(true) => {
            // Native — validate count/extend first (fail-fast on malformed args).
            let (count, extend) = parse_count_extend(&args_vec)
                .map_err(|e| SteelErr::new(steel::rerrs::ErrorKind::Generic, format!("%call!: {e}")))?;

            if ctx.cmd_queue.is_empty() {
                // No deferred command ahead — run synchronously (Model A, Case B).
                // Note: source order is guaranteed only within cmd_queue; side-effect
                // queues (pending_language_sets, grammar_sweeps, plugin_loads) are
                // drained after the full eval and remain unordered relative to this
                // sync call.
                ctx.host.run_command_sync(&name, count, extend, ctx.current_register_prefix)
                    .map(|()| SteelVal::Void)
                    .map_err(|e| SteelErr::new(steel::rerrs::ErrorKind::Generic, format!("%call!: {e}")))
            } else {
                // A Steel-defined command is already queued — defer this native too
                // so the body executes in source order.
                ctx.cmd_queue.push(QueuedCommand { name, args: args_vec, register: ctx.current_register_prefix });
                Ok(SteelVal::Void)
            }
        }
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
/// **Queue-ordering note**: `(call! …)` runs *after* the proc returns;
/// `set-register-prefix!` only *routes* the register into each queued
/// command.  You cannot read a command's register side-effect back within
/// the same body — use a Steel `let` binding for that.
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
    use crate::types::QueuedCommand;
    use crate::test_support::SteelCtxTestHarness;

    fn make_list(vals: Vec<SteelVal>) -> SteelVal {
        SteelVal::ListV(vals.into_iter().collect())
    }

    #[test]
    fn call_bang_in_init_queues_to_cmd_queue() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        call_command_primitive(&mut ctx, "plum-ensure-grammars".to_string(), make_list(vec![])).unwrap();
        assert_eq!(
            ctx.cmd_queue,
            vec![QueuedCommand { name: "plum-ensure-grammars".to_string(), args: vec![], register: None }],
        );
    }

    #[test]
    fn call_bang_queues_multiple_names() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        call_command_primitive(&mut ctx, "move-right".to_string(), make_list(vec![])).unwrap();
        call_command_primitive(&mut ctx, "move-left".to_string(), make_list(vec![])).unwrap();
        let names: Vec<&str> = ctx.cmd_queue.iter().map(|qc| qc.name.as_str()).collect();
        assert_eq!(names, vec!["move-right", "move-left"]);
    }

    /// NullHost has no registry; `command_is_native` returns `Ok(false)` for every
    /// name, so all commands are treated as Steel/forward-raw regardless of args.
    #[test]
    fn call_bang_command_mode_defers_when_host_has_no_registry() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        call_command_primitive(&mut ctx, "move-right".to_string(), make_list(vec![])).unwrap();
        assert_eq!(
            ctx.cmd_queue,
            vec![QueuedCommand { name: "move-right".to_string(), args: vec![], register: None }]
        );
    }

    /// NullHost treats everything as Steel/forward-raw, so a count arg is forwarded
    /// unchanged rather than stripped. The native count-strip path is exercised in
    /// `editor/src/editor/tests/sync_dispatch.rs` where a real registry is available.
    #[test]
    fn call_bang_count_arg_forwarded_raw_by_null_host() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let count_arg = SteelVal::IntV(5);
        call_command_primitive(
            &mut ctx,
            "move-right".to_string(),
            make_list(vec![count_arg.clone()]),
        ).unwrap();
        // NullHost = Steel/forward-raw: Int arg forwarded unchanged, not stripped.
        assert_eq!(
            ctx.cmd_queue,
            vec![QueuedCommand { name: "move-right".to_string(), args: vec![count_arg], register: None }]
        );
    }

    /// Steel-defined commands accept arbitrary positional args — a string is
    /// forwarded unchanged. NullHost's `command_is_native → Ok(false)` makes this
    /// the Steel branch regardless of name.
    #[test]
    fn call_bang_string_arg_forwarded_as_steel_args() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let arg = SteelVal::StringV("hello".into());
        call_command_primitive(&mut ctx, "echo".to_string(), make_list(vec![arg.clone()])).unwrap();
        assert_eq!(
            ctx.cmd_queue,
            vec![QueuedCommand { name: "echo".to_string(), args: vec![arg], register: None }]
        );
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
