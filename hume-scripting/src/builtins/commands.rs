//! `(define-command! name doc proc)` and `(call! name args…)` builtins.
//!
//! `call!` expands to `%dispatch-command!` (BOOTSTRAP), which routes:
//! - **Activated plugin commands**: `(apply proc args)` inside the VM; `call!`
//!   returns the body's value and later reads see its effects.
//! - **Lazy commands**: activates the owner inline, then retries.
//! - **Native commands**: `%call-native!` runs `run_command_sync`. A native
//!   command has no Steel parameter list to receive a pane, so the caller
//!   passes one as the first argument: `(call! "move-right" pane 5)`. A
//!   trailing char or string is the command's operand:
//!   `(call! "find-forward" pane #\x)`, `(call! "sift-within" pane "\\w+")`. Returns
//!   `#f` if the command refused, `#t` otherwise; in init mode it warns and
//!   returns `#f`.
//! - **Unknown**: error logged, `#f` returned.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use super::SteelResult;
use super::errors::{generic_err, require_cmd};
use crate::SteelCtx;
use crate::attribution::Owner;
use crate::command_procs::{CommandProc, MappableProc, TypedProc};
use crate::host::NativeCall;
use crate::types::{SteelCmdBody, SteelCmdDef, SteelTypedCmdBody, SteelTypedCmdDef};
use hume_core::{CommandArg, Count};

// ── Builtins ──────────────────────────────────────────────────────────────────

/// Shared guards behind `define-command!` and `define-typed-command!`: name
/// syntax, then [`check_name_free`].
fn check_definable(ctx: &mut SteelCtx, name: &str) -> Result<(), SteelErr> {
    crate::command_name::check_quotable(name).map_err(generic_err)?;
    check_name_free(ctx, name)
}

/// Raises unless `name` is free for the running code to claim: not a
/// built-in, not defined, and not another lazy entry's activation command.
/// `define-command!`, `define-typed-command!` and `declare-plugin!` share
/// it. A mappable and a typed command share one Steel-side proc namespace
/// just as they share one namespace in the editor's `CommandRegistry`.
pub(crate) fn check_name_free(ctx: &mut SteelCtx, name: &str) -> Result<(), SteelErr> {
    if ctx.builtin_cmd_names.contains(name) {
        steel::stop!(Generic =>
            "'{}' conflicts with a built-in command and cannot be redefined", name);
    }
    // Guard against true re-definition, mappable or typed. A lazy stub has no
    // proc, so a plugin defining its own activation command passes here.
    if let Some(defined) = ctx.host.commands().defined_command(name) {
        steel::stop!(Generic =>
            "command '{}' is already defined by '{}'", name, defined.owner());
    }
    // Guard against stealing a lazy plugin's activation command. A lazy plugin
    // can still define its own activation command during its own body (the
    // stub stays registered until unregister_lazy_stubs_of runs at the end of
    // finish_lazy_activation), so a definition made inside that body passes.
    if let Some(claimant) = ctx.host.commands().lazy_command_owner(name) {
        let is_self = ctx.activation().as_deref() == Some(&claimant);
        if !is_self {
            steel::stop!(Generic =>
                "command '{}' is already claimed as an activation command by lazy plugin '{}'",
                name, claimant);
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
/// - The same name, in any case, is already defined by another plugin or in
///   init.scm.
/// - The host refuses `name`, as the editor does one that is not lowercase
///   ([`crate::command_name::check_mappable`]).
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
            "'#:repeatable #t' and '#:inline-output #t' are mutually exclusive: \
             shell-out commands must not participate in dot-repeat");
    }
    check_definable(ctx, &name)?;
    let proc = super::args::callable_arg(proc, "third arg (proc)")?;
    let (arity, is_variadic) = super::args::command_shape(proc.value());
    let proc = MappableProc::new(proc, std::rc::Rc::clone(ctx.owner()), ctx.activation());
    ctx.host
        .commands()
        .register_command(SteelCmdDef {
            name,
            doc,
            body: SteelCmdBody {
                arity,
                is_variadic,
                inline_output,
                repeatable,
                proc,
            },
        })
        .map_err(generic_err)?;
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
/// Raises a Steel error under the same conditions as `define-command!`,
/// except that the editor refuses a name the `:` line can't read back whole
/// ([`crate::command_name::check_typed`]) rather than one that is not
/// lowercase.
pub(crate) fn define_typed_command(
    ctx: &mut SteelCtx,
    name: String,
    doc: String,
    proc: SteelVal,
    inline_output: bool,
    completer: SteelVal,
) -> SteelResult {
    check_definable(ctx, &name)?;
    let proc = super::args::callable_arg(proc, "third arg (proc)")?;
    let completer = super::args::optional_string_arg(completer, "#:complete")?;
    let (arity, is_variadic) = super::args::command_shape(proc.value());
    let proc = TypedProc::new(proc, std::rc::Rc::clone(ctx.owner()), ctx.activation());
    ctx.host
        .commands()
        .register_typed_command(SteelTypedCmdDef {
            name,
            doc,
            completer,
            body: SteelTypedCmdBody {
                arity,
                is_variadic,
                inline_output,
                proc,
            },
        })
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// `%call-native!`: `%dispatch-command!`'s fallback for a name that is neither
/// a defined mappable proc nor owned by a lazy plugin.
///
/// - **Native**: decodes the leading `pane` argument, validates count/extend
///   and the optional trailing operand, and returns `run_command_sync`'s
///   `#t`/`#f`. While the config loads it raises, naming the command: there
///   is no pane to run it on yet.
/// - **Steel command missing from the table**, or a name `call!` cannot
///   reach (unknown, typed, key-only): raises, naming it, like any other
///   caller mistake.
pub(crate) fn call_command_primitive(
    ctx: &mut SteelCtx,
    name: String,
    args: SteelVal,
) -> SteelResult {
    let args_vec = steel_list_to_vec(args)?;

    match ctx.host.commands().command_is_native(&name) {
        Ok(true) => {
            require_cmd(ctx).map_err(|_| {
                generic_err(format!(
                    "'{name}' can't run while loading config: bind it to a key or call it from a hook"
                ))
            })?;
            let (pane, rest) = args_vec.split_first().ok_or_else(|| {
                generic_err(format!(
                    "'{name}' needs a pane: (call! \"{name}\" pane [count [extend]] [arg])"
                ))
            })?;
            let pane = super::ids::downcast_pane(pane)
                .ok_or_else(|| generic_err(format!("'{name}': first arg must be a pane")))?;
            let (count, extend, arg) = parse_native_args(rest).map_err(generic_err)?;
            ctx.host
                .commands()
                .run_command_sync(
                    &name,
                    pane,
                    NativeCall {
                        count,
                        extend,
                        register: ctx.current_register_prefix,
                        arg,
                    },
                )
                .map(SteelVal::BoolV)
                .map_err(generic_err)
        }
        Ok(false) => Err(generic_err(format!("'{name}' is not a native command"))),
        Err(msg) => Err(generic_err(msg)),
    }
}

/// `(%arm-inline-output! name)`: see `%apply-command!` in `bootstrap.scm`.
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
/// `%apply-command!` in `bootstrap.scm`'s BOOTSTRAP comment (`builtins/mod.rs`)
/// for what happens when a body raises before reaching it.
pub(crate) fn restore_inline_output(ctx: &mut SteelCtx, depth: SteelVal) -> SteelResult {
    let depth = super::args::usize_arg(depth, "depth")?;
    if let Some(output) = ctx.host.output() {
        output.truncate_inline_output(depth);
    }
    Ok(SteelVal::Void)
}

/// `(%enter-command! name)`: see `%dispatch-command!` and `%apply-command!`
/// in `bootstrap.scm`. `#f` when no mappable Steel command is defined as
/// `name`. Otherwise enters the `call!` frame that runs it as the code that
/// defined it, and returns `(proc depth)`: the proc to apply, and the depth
/// the matching `%leave-command!` restores.
///
/// Works in both init and command mode: during init, `define-command!`
/// records the proc inline, so `(call! "cmd")` that follows a
/// `(load-plugin! …)` in the same init.scm body finds it immediately.
pub(crate) fn enter_command(ctx: &mut SteelCtx, name: String) -> SteelResult {
    let Some(command) = ctx.host.commands().defined_command(&name) else {
        return Ok(SteelVal::BoolV(false));
    };
    let CommandProc::Mappable(proc) = &command else {
        return Ok(SteelVal::BoolV(false));
    };
    let proc = proc.steel_val();
    let depth = ctx
        .attribution
        .enter_call(std::rc::Rc::clone(command.owner()));
    Ok(SteelVal::ListV(
        vec![proc, SteelVal::IntV(depth as isize)].into(),
    ))
}

/// `(%leave-command! depth)`: ends the `call!` frame
/// `%enter-command!` returned `depth` for. A plugin body the call
/// opened closes before it, so one still open above `depth` has lost its
/// handler, and fails as a session's end fails it.
pub(crate) fn leave_command(ctx: &mut SteelCtx, depth: SteelVal) -> SteelResult {
    let depth = super::args::usize_arg(depth, "depth")?;
    let open = ctx.attribution.drain_above(depth);
    if !open.is_empty() {
        let reason =
            generic_err("the plugin body was still running when the call! that started it ended");
        super::plugins::fail_open_bodies(ctx, open, &reason);
    }
    Ok(SteelVal::Void)
}

/// Parse count, extend, and the optional operand from a native command's
/// args list.
///
/// Valid shapes: `[]` → `(Count::typed(1), false)`; `[n]` → `(decode(n),
/// false)`; `[n, bool]` → `(decode(n), bool)`; each optionally followed by
/// one string or char, the command's [`CommandArg`] (`"x"` → `Text`, `#\x` →
/// `Char`). All other shapes (a second trailing arg, a number or boolean in
/// the operand's place) return `Err`. Whether the command takes that operand
/// is checked by the editor, not here.
///
/// `decode(0)` is [`Count::UNTYPED`]: the Scheme spelling of "no count typed"
/// (a bare keypress), since Scheme has no `Option` to pass across the
/// builtin-call boundary and `0` is otherwise unreachable as an explicit
/// count. [`count_to_steel`] is its inverse. An untyped count makes
/// `move-down`/`move-up` move by visual line instead of buffer line (see
/// `EditorHost::run_command_sync`); every other native command treats it the
/// same as a count of one. A positive `n` goes through [`Count::typed`], so a
/// huge one is clamped; a negative one is `Err`, since no keypress makes one.
pub(crate) fn parse_native_args(
    args: &[SteelVal],
) -> Result<(Count, bool, Option<CommandArg>), String> {
    fn decode(n: isize) -> Result<Count, String> {
        match usize::try_from(n) {
            Ok(0) => Ok(Count::UNTYPED),
            Ok(n) => Ok(Count::typed(n)),
            Err(_) => Err(format!("count must be 0 or more, got {n}")),
        }
    }
    let (head, arg) = match args.split_last() {
        Some((SteelVal::StringV(s), head)) => (head, Some(CommandArg::Text(s.to_string()))),
        Some((SteelVal::CharV(c), head)) => (head, Some(CommandArg::Char(*c))),
        _ => (args, None),
    };
    let (count, extend) = match head {
        [] => (Count::typed(1), false),
        [SteelVal::IntV(n)] => (decode(*n)?, false),
        [SteelVal::IntV(n), SteelVal::BoolV(ext)] => (decode(*n)?, *ext),
        _ => {
            return Err(format!(
                "native command args must be [count [extend]] then an optional string or char; got {:?}",
                args
            ));
        }
    };
    Ok((count, extend, arg))
}

/// `count` as a Steel command body's `count` argument: `0` when none was
/// typed, the encoding `parse_native_args` decodes, so a body that forwards
/// its `count` to `call!` passes the same count on.
pub fn count_to_steel(count: Count) -> SteelVal {
    let n = if count.is_typed() { count.get() } else { 0 };
    SteelVal::IntV(n as isize)
}

fn steel_list_to_vec(val: SteelVal) -> Result<Vec<SteelVal>, SteelErr> {
    match val {
        SteelVal::ListV(list) => Ok(list.into_iter().collect()),
        other => steel::stop!(TypeMismatch =>
            "second arg must be a list, got {:?}", other),
    }
}

/// `(command-plugin name)`: return the owner of command `name` as a string.
///
/// Returns the plugin id string (e.g. `"core:plum"`, `"user/repo"`) if the
/// command was registered by a plugin, `"user"` if registered from top-level
/// `init.scm`, or `"hume"` for built-in Rust commands (not Steel-registered).
/// A lazy stub names the plugin that declared it, before that plugin's body
/// has run.
///
/// Valid during both eval (e.g. conflict detection in `declare-plugin!`) and
/// command execution.  Returns `"hume"` for any name with neither a defined
/// proc nor a lazy stub (unknown commands are implicitly built-in).
pub(crate) fn command_plugin(ctx: &mut SteelCtx, name: String) -> SteelResult {
    let owner = match ctx.host.commands().defined_command(&name) {
        Some(defined) => (**defined.owner()).clone(),
        None => ctx
            .host
            .commands()
            .lazy_command_owner(&name)
            .map_or(Owner::Core, Owner::Plugin),
    };
    Ok(SteelVal::StringV(owner.to_string().into()))
}

/// `(%command-callable? name)`: whether `(call! name …)` can reach `name`,
/// i.e. it is in the mappable command table (native, Steel, or a `Lazy` stub).
/// Read-only: a `Lazy` stub is reported, not activated.
pub(crate) fn command_callable(ctx: &mut SteelCtx, name: String) -> SteelResult {
    Ok(SteelVal::BoolV(
        ctx.host.commands().command_is_native(&name).is_ok(),
    ))
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
    let reg = super::registers::register_arg(ctx, &name)?;
    ctx.current_register_prefix = Some(reg);
    Ok(SteelVal::Void)
}

#[cfg(test)]
mod tests;
