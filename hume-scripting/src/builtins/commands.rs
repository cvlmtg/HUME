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

use super::{require_cmd_ctx, require_config_ctx};
use crate::SteelCtx;
use crate::attribution::Owner;
use crate::log::LogLevel;
use crate::types::SteelCmdDef;

type SteelResult = Result<SteelVal, SteelErr>;

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
    define_command_inner(ctx, name, doc, proc, repeatable, inline_output)
}

fn define_command_inner(
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
    require_config_ctx!(ctx, builtin_name);
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
        let owner = ctx
            .registries
            .cmd_owners
            .get(&name)
            .map(|s| s.as_str())
            .unwrap_or("unknown");
        steel::stop!(Generic =>
            "{}: command '{}' is already defined by '{}'", builtin_name, name, owner);
    }
    // Guard against stealing a lazy plugin's activation command. A lazy plugin
    // can still define its own activation command during its own body (the name
    // stays in activation_commands until drop_activations_for runs at the end of
    // finish_lazy_activation), so we exempt the self-ownership case.
    if let Some(claimant) = ctx.registries.lazy_registry.activation_commands.get(&name) {
        let is_self = matches!(
            ctx.plugin_stack.current_owner(),
            Owner::Plugin(ref cur) if cur == claimant
        );
        if !is_self {
            steel::stop!(Generic =>
                "{}: command '{}' is already claimed as an activation command by lazy plugin '{}'",
                builtin_name, name, claimant);
        }
    }
    match &proc {
        SteelVal::Closure(_) | SteelVal::FuncV(_) | SteelVal::MutFunc(_) => {}
        _ => steel::stop!(TypeMismatch =>
            "{}: third arg (proc) must be a callable, got {:?}", builtin_name, proc),
    }
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
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e))?;
    let current_owner = ctx.plugin_stack.current_owner();
    ctx.registries.command_table.insert(name.clone(), proc);
    ctx.registries
        .cmd_owners
        .insert(name, current_owner.to_string());
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
/// - **Steel-but-not-in-table** (`Ok(false)`): logs a `Warning` naming the
///   command and returns `#void`. Reaching this arm at all means the
///   dispatcher's own lookup already missed the command in `command_table`,
///   so this is a fallback message, not the common case.
/// - **Unknown** (`Err(msg)`): the host's registry has no such command at
///   all (typo, missing plugin). Logs the host's own error message — it
///   already names the command — and returns `#void`.
pub(crate) fn call_command_primitive(
    ctx: &mut SteelCtx,
    name: String,
    args: SteelVal,
) -> SteelResult {
    let args_vec = steel_list_to_vec(args)?;

    match ctx.host.commands().command_is_native(&name) {
        Ok(true) => {
            if ctx.is_init {
                ctx.log(
                    LogLevel::Warning,
                    format!("init.scm: skipped runtime command '{name}' — it can't run while loading config; bind it to a key or call it from a hook instead"),
                );
                return Ok(SteelVal::Void);
            }
            let (count, extend) = parse_count_extend(&args_vec)
                .map_err(|e| SteelErr::new(ErrorKind::Generic, format!("%call-native!: {e}")))?;
            ctx.host
                .commands()
                .run_command_sync(&name, count, extend, ctx.current_register_prefix)
                .map(|()| SteelVal::Void)
                .map_err(|e| SteelErr::new(ErrorKind::Generic, format!("%call-native!: {e}")))
        }
        Ok(false) => {
            ctx.log(
                LogLevel::Warning,
                format!("'{name}' is not a native command"),
            );
            Ok(SteelVal::Void)
        }
        Err(msg) => {
            ctx.log(LogLevel::Warning, msg);
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
/// it the same as `Some(1)`. Negative counts still clamp to `Some(1)`.
///
/// Re-exported from the crate root so the editor crate can reuse it when
/// parsing the count/extend args passed to a native command from Steel.
pub fn parse_count_extend(args: &[SteelVal]) -> Result<(Option<usize>, bool), String> {
    fn decode(n: isize) -> Option<usize> {
        if n == 0 {
            None
        } else {
            Some(n.max(1) as usize)
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

fn steel_list_to_vec(val: SteelVal) -> Result<Vec<SteelVal>, steel::rerrs::SteelErr> {
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
    if !ctx.host.commands().is_valid_register_name(reg) {
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

    /// NullHost returns `Ok(false)` for `command_is_native` (no registry) →
    /// "not a native command" path → warning logged.
    #[test]
    fn call_bang_unknown_command_logs_warning() {
        let mut h = SteelCtxTestHarness::new();
        {
            let mut ctx = h.ctx_init();
            call_command_primitive(
                &mut ctx,
                "plum-ensure-grammars".to_string(),
                make_list(vec![]),
            )
            .unwrap();
        }
        assert!(
            h.pending_messages
                .iter()
                .any(|(_, msg)| msg.contains("plum-ensure-grammars")),
            "unknown command must log a warning; got: {:?}",
            h.pending_messages
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
            h.pending_messages
                .iter()
                .any(|(_, msg)| msg.contains("move-right")),
            "unknown command in command mode must log a warning; got: {:?}",
            h.pending_messages
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
        let has_right = h
            .pending_messages
            .iter()
            .any(|(_, msg)| msg.contains("move-right"));
        let has_left = h
            .pending_messages
            .iter()
            .any(|(_, msg)| msg.contains("move-left"));
        assert!(
            has_right && has_left,
            "each unknown command must produce a warning"
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
        assert_eq!(parse_count_extend(&[]).unwrap(), (Some(1), false));
    }

    #[test]
    fn parse_count_extend_count_only() {
        assert_eq!(
            parse_count_extend(&[SteelVal::IntV(5)]).unwrap(),
            (Some(5), false)
        );
    }

    #[test]
    fn parse_count_extend_count_and_extend() {
        assert_eq!(
            parse_count_extend(&[SteelVal::IntV(3), SteelVal::BoolV(true)]).unwrap(),
            (Some(3), true)
        );
    }

    /// Negative counts clamp to `Some(1)` — same as a native keypress count.
    #[test]
    fn parse_count_extend_negative_clamps_to_one() {
        assert_eq!(
            parse_count_extend(&[SteelVal::IntV(-7)]).unwrap(),
            (Some(1), false)
        );
        assert_eq!(
            parse_count_extend(&[SteelVal::IntV(-1), SteelVal::BoolV(false)]).unwrap(),
            (Some(1), false)
        );
    }

    /// Zero is the Scheme spelling of "no count typed" — decodes to `None`,
    /// not `Some(1)` (a bare keypress and an explicit count of 1 are different
    /// dispatch origins even though both apply a command once).
    #[test]
    fn parse_count_extend_zero_means_no_count() {
        assert_eq!(
            parse_count_extend(&[SteelVal::IntV(0)]).unwrap(),
            (None, false)
        );
        assert_eq!(
            parse_count_extend(&[SteelVal::IntV(0), SteelVal::BoolV(true)]).unwrap(),
            (None, true)
        );
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

    // ── define-command! validation ────────────────────────────────────────────

    #[test]
    fn define_command_name_with_double_quote_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let err = define_command(
            &mut ctx,
            "bad\"name".to_string(),
            "doc".to_string(),
            SteelVal::BoolV(false), // type check comes after name check
            false,
            false,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("must not contain"),
            "expected name rejection, got: {err}"
        );
    }

    #[test]
    fn define_command_name_with_backslash_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let err = define_command(
            &mut ctx,
            "bad\\name".to_string(),
            "doc".to_string(),
            SteelVal::BoolV(false),
            false,
            false,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("must not contain"),
            "expected name rejection, got: {err}"
        );
    }

    /// When the host rejects the registration, `command_table` and `cmd_owners`
    /// must stay clean — the host call runs *before* the table inserts.
    ///
    /// Fail oracle: move the inserts back above `host.register_command` → the
    /// entries linger after the Err and both cleanliness asserts fire.  A stale
    /// entry would later make the plugin-failure rollback unregister a command
    /// the plugin never actually owned.
    #[test]
    fn define_command_host_rejection_leaves_tables_clean() {
        fn dummy_proc(_args: &[SteelVal]) -> SteelResult {
            Ok(SteelVal::Void)
        }
        let mut h = SteelCtxTestHarness::new();
        let mut host = crate::null_host::FailingRegisterHost::default();
        {
            let mut ctx = h.ctx_init_with_host(&mut host);
            let err = define_command(
                &mut ctx,
                "rejected-cmd".to_string(),
                "doc".to_string(),
                SteelVal::FuncV(dummy_proc),
                false,
                false,
            )
            .unwrap_err();
            assert!(
                err.to_string().contains("rejected by the command registry"),
                "error must come from the host; got: {err}"
            );
        }
        assert!(
            !h.registries.command_table.contains_key("rejected-cmd"),
            "command_table must not record a command the host rejected"
        );
        assert!(
            !h.registries.cmd_owners.contains_key("rejected-cmd"),
            "cmd_owners must not record a command the host rejected"
        );
    }

    #[test]
    fn define_command_dup_names_error_names_existing_owner() {
        let mut h = SteelCtxTestHarness::new();
        // Simulate a command already fully defined by core:plum.
        // Both command_table (actually defined) and cmd_owners (attribution)
        // must be set — cmd_owners alone is pre-seeded by declare_plugin for
        // activation command ownership, so the guard checks command_table.
        h.registries
            .command_table
            .insert("my-cmd".to_string(), SteelVal::BoolV(false));
        h.registries
            .cmd_owners
            .insert("my-cmd".to_string(), "core:plum".to_string());
        let mut ctx = h.ctx_init();
        let err = define_command(
            &mut ctx,
            "my-cmd".to_string(),
            "doc".to_string(),
            SteelVal::BoolV(false),
            false,
            false,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("core:plum"),
            "error must name the existing owner; got: {msg}"
        );
        assert!(
            msg.contains("my-cmd"),
            "error must name the command; got: {msg}"
        );
    }
}
