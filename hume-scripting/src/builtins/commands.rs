//! `(define-command! name doc proc)`, `(call! name args…)`, and
//! `(request-wait-char! cmd)` builtins.
//!
//! `call!` expands to `%dispatch-command` (BOOTSTRAP), which routes:
//! - **Activated plugin commands**: `(apply proc args)` inside the VM; `call!`
//!   returns the body's value and later reads see its effects.
//! - **Lazy commands**: activates the owner inline, then retries.
//! - **Native commands**: `%call-native!` runs `run_command_sync`. A native
//!   command has no Steel parameter list to receive a pane, so the caller
//!   passes one as the first argument: `(call! "move-right" pane 5)`. Returns
//!   `#f` if the command refused, `#t` otherwise; in init mode it warns and
//!   returns `#f`.
//! - **Unknown**: error logged, `#f` returned.
//!
//! `request-wait-char!` makes the editor enter WaitChar mode for the named
//! command once the current eval finishes.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use super::SteelResult;
use super::errors::generic_err;
use crate::SteelCtx;
use crate::attribution::Owner;
use crate::log::LogLevel;
use crate::types::{SteelCmdDef, SteelTypedCmdDef};
use hume_engine::types::MAX_COUNT;

// ── Builtins ──────────────────────────────────────────────────────────────────

/// The owner of `name`'s already-registered proc body (mappable
/// (`command_table`) or typed (`typed_command_table`)), or `None` if no body
/// has been defined yet under either table.
///
/// A pre-seeded `cmd_owners` entry with no matching table entry (a lazy
/// stub's activation-command ownership, written by `declare_plugin` before
/// its body ever runs) doesn't count as "defined"; see `check_definable`'s
/// own doc for why that distinction matters.
fn defined_owner<'a>(ctx: &'a SteelCtx, name: &str) -> Option<&'a Owner> {
    if ctx.registries.command_table.contains_key(name)
        || ctx.registries.typed_command_table.contains_key(name)
    {
        // cmd_owners must have an entry whenever either table does (both are
        // written together, see the insert pairs in `define_command`/
        // `define_typed_command`). A miss here would be a registries-desync
        // bug, not a normal "unknown owner" case.
        Some(
            ctx.registries.cmd_owners.get(name).expect(
                "a defined command_table/typed_command_table entry implies a cmd_owners entry",
            ),
        )
    } else {
        None
    }
}

/// Shared guards behind `define-command!` and `define-typed-command!`: name
/// syntax, built-in shadowing, true re-definition, and lazy-stub self-
/// ownership. `builtin_name` only changes the error text: both callers
/// check the same `command_table`/`typed_command_table`/`cmd_owners`, since a
/// mappable and a typed command share one Steel-side proc namespace just as
/// they share one namespace in the editor's `CommandRegistry`.
fn check_definable(ctx: &mut SteelCtx, builtin_name: &str, name: &str) -> Result<(), SteelErr> {
    if name.contains('"') || name.contains('\\') {
        steel::stop!(Generic =>
            "{}: command name '{}' must not contain '\"' or '\\'", builtin_name, name);
    }
    if ctx.builtin_cmd_names.contains(name) {
        steel::stop!(Generic =>
            "{}: '{}' conflicts with a built-in command and cannot be redefined",
            builtin_name, name);
    }
    // Guard against true re-definition, mappable or typed: `cmd_owners` alone
    // can't tell (see `defined_owner`'s doc), and checking it here would falsely
    // reject a plugin defining its own lazy activation command.
    if let Some(owner) = defined_owner(ctx, name) {
        steel::stop!(Generic =>
            "{}: command '{}' is already defined by '{}'", builtin_name, name, owner);
    }
    // Guard against stealing a lazy plugin's activation command. A lazy plugin
    // can still define its own activation command during its own body (the
    // stub stays registered until unregister_lazy_stubs_of runs at the end of
    // finish_lazy_activation), so we exempt the self-ownership case.
    if let Some(claimant) = ctx.host.commands().lazy_command_owner(name) {
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
    Ok(())
}

/// `(%define-command! name doc proc repeatable inline-output)`
///
/// Native primitive behind the `(define-command! …)` Steel wrapper.
/// Registers `proc` (a Steel lambda) as a mappable command with the given
/// `name` and `doc` string.  The command can then be bound to a key via
/// `(bind-key! …)`.
///
/// `repeatable` and `inline_output` are mutually exclusive: passing both
/// `#t` raises a Steel error.
///
/// When triggered by a key binding the lambda receives leading `pane`,
/// `count`, and `extend` arguments based on its declared arity:
/// - `(lambda ())`: no injection.
/// - `(lambda (pane))`: receives the pane the command was invoked through.
/// - `(lambda (pane count))`: pane and the repeat count (integer ≥ 1).
/// - `(lambda (pane count extend))`: pane, count, and `#t`/`#f` extend flag.
/// - Variadic lambdas receive all three.
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
            "define-command!: '#:repeatable #t' and '#:inline-output #t' are mutually exclusive: \
             shell-out commands must not participate in dot-repeat");
    }
    check_definable(ctx, "define-command!", &name)?;
    let proc = super::args::callable_arg(proc, "define-command! third arg (proc)")?;
    let (arity, is_variadic) = match &proc {
        SteelVal::Closure(gc) => (gc.arity() as u16, gc.is_multi_arity()),
        // FuncV/MutFunc arity is not introspectable; treat as 0-arg non-variadic so
        // keymap injection passes no leading args rather than blindly injecting 2.
        _ => (0, false),
    };
    // Register in the editor's CommandRegistry first: it can still reject the
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

/// `(%define-typed-command! name doc proc inline-output)`
///
/// Native primitive behind the `(define-typed-command! …)` Steel wrapper.
/// Registers `proc` as a typed command invocable from the `:` command line,
/// the typed counterpart of [`define_command`]. No `repeatable` parameter:
/// dot-repeat is meaningless for a `:` command, so there is nothing to
/// mutually-exclude against `inline_output` the way `define-command!` does.
///
/// When dispatched, the lambda receives leading `pane`/`arg`/`force`
/// arguments based on its declared arity:
/// - `(lambda ())`: no injection.
/// - `(lambda (pane))`: the pane the command was invoked through.
/// - `(lambda (pane arg))`: pane and the typed argument (a string), or `#f` if none.
/// - `(lambda (pane arg force))`: pane, the argument, and whether `!` was appended.
///
/// Raises a Steel error under the same conditions as `define-command!`.
pub(crate) fn define_typed_command(
    ctx: &mut SteelCtx,
    name: String,
    doc: String,
    proc: SteelVal,
    inline_output: bool,
    completer: SteelVal,
) -> SteelResult {
    check_definable(ctx, "define-typed-command!", &name)?;
    let proc = super::args::callable_arg(proc, "define-typed-command! third arg (proc)")?;
    let completer =
        super::args::optional_string_arg(completer, "define-typed-command! #:complete")?;
    let (arity, is_variadic) = match &proc {
        SteelVal::Closure(gc) => (gc.arity() as u16, gc.is_multi_arity()),
        _ => (0, false),
    };
    ctx.host
        .commands()
        .register_typed_command(SteelTypedCmdDef {
            name: name.clone(),
            doc,
            arity,
            is_variadic,
            inline_output,
            completer,
        })
        .map_err(generic_err)?;
    let current_owner = ctx.plugin_stack.current_owner();
    // typed_command_table, not command_table: see its own doc for why the
    // separation matters (keeps `call!` from reaching a `:`-only command).
    ctx.registries
        .typed_command_table
        .insert(name.clone(), proc);
    ctx.registries.cmd_owners.insert(name, current_owner);
    Ok(SteelVal::Void)
}

/// `%call-native!`: `%dispatch-command`'s fallback for a name that is neither
/// in `command_table` nor owned by a lazy plugin.
///
/// - **Native**: decodes the leading `pane` argument, validates count/extend,
///   and returns `run_command_sync`'s `#t`/`#f`. In init mode it warns and
///   returns `#f`, since buffers aren't available yet.
/// - **Steel command missing from the table** or **unknown**: logs an `Error`
///   and returns `#f`.
///
/// A miss logs at `Error` (a plugin bug, like an unknown `:` command) but does
/// not raise: raising from a native builtin risks the `with-handler` re-raise
/// VM hazard.
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
                    format!("skipped runtime command '{name}': it can't run while loading config; bind it to a key or call it from a hook instead"),
                );
                return Ok(SteelVal::BoolV(false));
            }
            let (pane, rest) = args_vec.split_first().ok_or_else(|| {
                generic_err(format!(
                    "%call-native!: '{name}' needs a pane: (call! \"{name}\" pane [count [extend]])"
                ))
            })?;
            let pane = super::ids::downcast_pane(pane).ok_or_else(|| {
                generic_err(format!("%call-native!: '{name}': first arg must be a pane"))
            })?;
            let (count, extend) =
                parse_count_extend(rest).map_err(|e| generic_err(format!("%call-native!: {e}")))?;
            ctx.host
                .commands()
                .run_command_sync(&name, pane, count, extend, ctx.current_register_prefix)
                .map(SteelVal::BoolV)
                .map_err(|e| generic_err(format!("%call-native!: {e}")))
        }
        Ok(false) => {
            ctx.log(LogLevel::Error, format!("'{name}' is not a native command"));
            Ok(SteelVal::BoolV(false))
        }
        Err(msg) => {
            ctx.log(LogLevel::Error, msg);
            Ok(SteelVal::BoolV(false))
        }
    }
}

/// `(%arm-inline-output! name)`: see `%apply-command` in `bootstrap.scm`.
/// Arms the alt-screen bracket for a `call!`-dispatched `name` if it is a
/// Steel command declared `#:inline-output #t`. Returns the depth to
/// truncate back to at the matching `%restore-inline-output!`, so the
/// Scheme caller knows whether to pair a restore and, if so, with what. A
/// `#f` result (a native, unknown, or un-activated `Lazy` `name`, or a host
/// with no inline-output authority at all) touches no state and needs no
/// restore.
pub(crate) fn arm_inline_output(ctx: &mut SteelCtx, name: String) -> SteelResult {
    let depth = ctx
        .host
        .output()
        .and_then(|output| output.arm_inline_output(&name));
    Ok(match depth {
        Some(d) => SteelVal::IntV(d as isize),
        None => SteelVal::BoolV(false),
    })
}

/// `(%restore-inline-output! depth)`: truncates the bracket's frame stack
/// back to `depth` (the value `%arm-inline-output!` returned for this same
/// call). Only ever called after `%arm-inline-output!` returned non-`#f`; see
/// `%apply-command` in `bootstrap.scm`'s BOOTSTRAP comment (`builtins/mod.rs`)
/// for what happens when a body raises before reaching it.
pub(crate) fn restore_inline_output(ctx: &mut SteelCtx, depth: SteelVal) -> SteelResult {
    let depth = super::args::usize_arg(depth, "%restore-inline-output!")?;
    if let Some(output) = ctx.host.output() {
        output.truncate_inline_output(depth);
    }
    Ok(SteelVal::Void)
}

/// `%lookup-plugin-proc`: return the Steel closure for an activated plugin
/// command, or `#f` if the name is not in the `command_table`.
///
/// Works in both init and command mode: during init, `define-command!` populates
/// `command_table` inline, so `(call! "cmd")` that follows a `(load-plugin! …)`
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
/// `decode(0)` is `None`: the Scheme spelling of "no count typed" (a bare
/// keypress), since Scheme has no `Option` to pass across the builtin-call
/// boundary and `0` is otherwise unreachable as an explicit count. `None`
/// makes `move-down`/`move-up` move by visual line instead of buffer line
/// (see `EditorHost::run_command_sync`); every other native command treats
/// it the same as `Some(1)`. Negative counts clamp to `Some(1)`; counts above
/// [`MAX_COUNT`] clamp there, since a script has no digit-by-digit accumulator to
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
///   `(call! "surround-paren" pane) (request-wait-char! "replace")`
/// selects the surrounding `()` pair, then waits for the replacement char.
///
/// Only valid inside a `SteelBacked` command invocation.
pub(crate) fn request_wait_char(ctx: &mut SteelCtx, cmd: String) -> SteelResult {
    ctx.wait_char_request = Some(cmd);
    Ok(SteelVal::Void)
}

/// `(command-plugin name)`: return the owner of command `name` as a string.
///
/// Returns the plugin id string (e.g. `"core:plum"`, `"user/repo"`) if the
/// command was registered by a plugin, `"user"` if registered from top-level
/// `init.scm`, or `"hume"` for built-in Rust commands (not Steel-registered).
///
/// Valid during both eval (e.g. conflict detection in `declare-plugin!`) and
/// command execution.  Returns `"hume"` for any name not in the owner cache
/// (unknown commands are implicitly built-in).
pub(crate) fn command_plugin(ctx: &mut SteelCtx, name: String) -> SteelResult {
    let owner = ctx.registries.cmd_owners.get(&name).unwrap_or(&Owner::Core);
    Ok(SteelVal::StringV(owner.to_string().into()))
}

/// `(pending-char)`: return the pending character as a one-character string,
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

/// `(set-register-prefix! name)`: arm a sticky register prefix for the
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

#[cfg(test)]
mod tests;
