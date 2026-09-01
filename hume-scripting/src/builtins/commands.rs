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
//! - **Lazy activation commands** (unactivated plugin): `%dispatch-command`
//!   activates the owner inline via `%activate-plugin-inline`, then retries.
//! - **Native commands**: forwarded to `%call-native!` → `run_command_sync` inline.
//!   Init mode: warns and skips (buffer access not available during init).
//! - **Unknown**: forwarded to `%call-native!` → error logged, no-op.
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

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use super::SteelResult;
use super::errors::generic_err;
use crate::SteelCtx;
use crate::attribution::Owner;
use crate::log::LogLevel;
use crate::types::SteelCmdDef;
use hume_engine::types::MAX_COUNT;

// ── Builtins ──────────────────────────────────────────────────────────────────

/// `(%define-command! name doc proc repeatable inline-output)`
///
/// Native primitive behind the `(define-command! …)` Steel wrapper.
/// Registers `proc` (a Steel lambda) as a mappable command with the given
/// `name` and `doc` string.  The command can then be bound to a key via
/// `(bind-key! …)`.
///
/// `repeatable` and `inline_output` are mutually exclusive — passing both
/// `#t` raises a Steel error.
///
/// When triggered by a key binding the lambda receives leading `count` and
/// `extend` arguments based on its declared arity:
/// - `(lambda ())` — no injection; 0-arg commands keep working as before.
/// - `(lambda (count))` — receives the repeat count (integer ≥ 1).
/// - `(lambda (count extend))` — receives count and `#t`/`#f` extend flag.
/// - Variadic lambdas receive both count and extend.
///
/// Raises a Steel error if:
/// - `name` conflicts with a core built-in command.
/// - The same name is already defined by another plugin or in init.scm.
/// - Called from a command body (only valid during init.scm or plugin load).
pub(crate) fn define_command(
    ctx: &mut SteelCtx,
    name: String,
    doc: String,
    proc: SteelVal,
    repeatable: bool,
    inline_output: bool,
) -> SteelResult {
    if repeatable && inline_output {
        steel::stop!(Generic =>
            "define-command!: '#:repeatable #t' and '#:inline-output #t' are mutually exclusive \
             — shell-out commands must not participate in dot-repeat");
    }
    let builtin_name = "define-command!";
    if name.contains('"') || name.contains('\\') {
        steel::stop!(Generic =>
            "{}: command name '{}' must not contain '\"' or '\\'", builtin_name, name);
    }
    if ctx.builtin_cmd_names.contains(&name) {
        steel::stop!(Generic =>
            "{}: '{}' conflicts with a built-in command and cannot be redefined",
            builtin_name, name);
    }
    // Guard against true re-definition: command_table is set only when a
    // command body is actually registered. cmd_owners is pre-seeded by
    // declare_plugin for activation command ownership before the body runs, so
    // checking cmd_owners here would falsely reject a plugin defining its own
    // activation command.
    if ctx.registries.command_table.contains_key(&name) {
        // cmd_owners must have an entry whenever command_table does (both are
        // written together, see the insert pair below) — a miss here would be
        // a registries-desync bug, not a normal "unknown owner" case.
        let owner = ctx
            .registries
            .cmd_owners
            .get(&name)
            .expect("command_table entry implies a cmd_owners entry");
        steel::stop!(Generic =>
            "{}: command '{}' is already defined by '{}'", builtin_name, name, owner);
    }
    // Guard against stealing a lazy plugin's activation command. A lazy plugin
    // can still define its own activation command during its own body (the
    // stub stays registered until unregister_lazy_stubs_of runs at the end of
    // finish_lazy_activation), so we exempt the self-ownership case.
    if let Some(claimant) = ctx.host.commands().lazy_command_owner(&name) {
        let is_self = matches!(
            ctx.plugin_stack.current_owner(),
            Owner::Plugin(ref cur) if *cur == claimant
        );
        if !is_self {
            steel::stop!(Generic =>
                "{}: command '{}' is already claimed as an activation command by lazy plugin '{}'",
                builtin_name, name, claimant);
        }
    }
    let proc = super::args::callable_arg(proc, "define-command! third arg (proc)")?;
    let (arity, is_variadic) = match &proc {
        SteelVal::Closure(gc) => (gc.arity() as u16, gc.is_multi_arity()),
        // FuncV/MutFunc arity is not introspectable; treat as 0-arg non-variadic so
        // keymap injection passes no leading args rather than blindly injecting 2.
        _ => (0, false),
    };
    // Register in the editor's CommandRegistry first — it can still reject the
    // name (e.g. it shadows a native command the empty command-mode builtin set
    // missed).  Only on success do command_table/cmd_owners record the command;
    // otherwise a failed define would leave entries that the plugin-failure
    // rollback would then "clean up" by unregistering a command it never owned.
    ctx.host
        .commands()
        .register_command(SteelCmdDef {
            name: name.clone(),
            doc,
            arity,
            is_variadic,
            inline_output,
            repeatable,
        })
        .map_err(generic_err)?;
    let current_owner = ctx.plugin_stack.current_owner();
    ctx.registries.command_table.insert(name.clone(), proc);
    ctx.registries.cmd_owners.insert(name, current_owner);
    Ok(SteelVal::Void)
}

/// `%call-native!` — Rust leaf for native/unknown commands.
///
/// Called by `%dispatch-command` when `name` is NOT found in `command_table`
/// and has no lazy activation command owner (i.e. it is a native or unknown command).
///
/// - **Native** (`Motion`/`Selection`/`Edit`/`EditorCmd`): in command mode,
///   validates count/extend args and runs synchronously via `run_command_sync`.
///   In init mode, logs a warning and skips — native commands touch buffers,
///   which are not available during init.scm evaluation.
/// - **Steel-but-not-in-table** (`Ok(false)`): logs an `Error` naming the
///   command and returns `#void`. Reaching this arm at all means the
///   dispatcher's own lookup already missed the command in `command_table`,
///   so this is a fallback message, not the common case.
/// - **Unknown** (`Err(msg)`): the host's registry has no such command at
///   all (typo, missing plugin). Logs the host's own error message — it
///   already names the command — and returns `#void`.
///
/// Both misses log `Error`, not `Warning`: an unreachable `call!` target is
/// a plugin bug (typo or missing dependency), same severity as an unknown
/// `:` command (`command_mode.rs`). Still a no-op, not a raised Steel error —
/// raising from a native builtin risks the with-handler re-raise/VM-panic
/// hazard, and partial edits already committed earlier in the body are
/// undoable either way (each inner edit records its own undo revision).
/// Keybind misses (unknown name bound to a key) are a different path and
/// stay `Warning` — see `hume-editor/src/editor/dispatch.rs`.
pub(crate) fn call_command_primitive(
    ctx: &mut SteelCtx,
    name: String,
    args: SteelVal,
) -> SteelResult {
    let args_vec = steel_list_to_vec(args)?;

    match ctx.host.commands().command_is_native(&name) {
        Ok(true) => {
            if ctx.session == crate::context::EvalSession::Init {
                ctx.log(
                    LogLevel::Warning,
                    format!("skipped runtime command '{name}' — it can't run while loading config; bind it to a key or call it from a hook instead"),
                );
                return Ok(SteelVal::Void);
            }
            let (count, extend) = parse_count_extend(&args_vec)
                .map_err(|e| generic_err(format!("%call-native!: {e}")))?;
            ctx.host
                .commands()
                .run_command_sync(&name, count, extend, ctx.current_register_prefix)
                .map(|()| SteelVal::Void)
                .map_err(|e| generic_err(format!("%call-native!: {e}")))
        }
        Ok(false) => {
            ctx.log(LogLevel::Error, format!("'{name}' is not a native command"));
            Ok(SteelVal::Void)
        }
        Err(msg) => {
            ctx.log(LogLevel::Error, msg);
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
/// Valid shapes: `[]` → `(Some(1), false)`; `[n]` → `(decode(n), false)`;
/// `[n, bool]` → `(decode(n), bool)`. All other shapes (e.g. a leading string,
/// extra args) return `Err`.
///
/// `decode(0)` is `None` — the Scheme spelling of "no count typed" (a bare
/// keypress), since Scheme has no `Option` to pass across the builtin-call
/// boundary and `0` is otherwise unreachable as an explicit count. `None`
/// makes `move-down`/`move-up` move by visual row instead of buffer line
/// (see `EditorHost::run_command_sync`); every other native command treats
/// it the same as `Some(1)`. Negative counts clamp to `Some(1)`; counts above
/// [`MAX_COUNT`] clamp there — a script has no digit-by-digit accumulator to
/// cap, so this is the only ceiling standing between an arbitrary `isize` and
/// a command that loops the count with no fixed-point exit.
///
pub(crate) fn parse_count_extend(args: &[SteelVal]) -> Result<(Option<usize>, bool), String> {
    fn decode(n: isize) -> Option<usize> {
        if n == 0 {
            None
        } else {
            Some((n.max(1) as usize).min(MAX_COUNT))
        }
    }
    match args {
        [] => Ok((Some(1), false)),
        [SteelVal::IntV(n)] => Ok((decode(*n), false)),
        [SteelVal::IntV(n), SteelVal::BoolV(ext)] => Ok((decode(*n), *ext)),
        _ => Err(format!(
            "native command args must be [], [count], or [count extend]; got {:?}",
            args
        )),
    }
}

fn steel_list_to_vec(val: SteelVal) -> Result<Vec<SteelVal>, SteelErr> {
    match val {
        SteelVal::ListV(list) => Ok(list.into_iter().collect()),
        other => steel::stop!(TypeMismatch =>
            "%call-native!: second arg must be a list, got {:?}", other),
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
    let owner = ctx.registries.cmd_owners.get(&name).unwrap_or(&Owner::Core);
    Ok(SteelVal::StringV(owner.to_string().into()))
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
    let reg = super::registers::register_arg(ctx, &name, "set-register-prefix!")?;
    ctx.current_register_prefix = Some(reg);
    Ok(SteelVal::Void)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
