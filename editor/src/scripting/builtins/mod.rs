//! Steel builtins for HUME's scripting layer.
//!
//! [`register_all`] registers every builtin on the engine and then evaluates
//! the Scheme bootstrap that defines `load-plugin`.  This must be called once
//! during [`ScriptingHost::new`] before any `eval_init` call.

pub(crate) mod commands;
pub(crate) mod fs;
pub(crate) mod interrupt;
pub(crate) mod keymap_bind;
pub(crate) mod plugins;
pub(crate) mod settings;
pub(crate) mod shell;
pub(crate) mod statusline;

use steel::steel_vm::engine::Engine;
use steel::rvals::SteelVal;
use steel::rerrs::SteelErr;

use super::EvalCtx;

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Extract the single string argument from `args`, returning a Steel error on
/// arity or type mismatch.  Used by multiple builtin sub-modules.
pub(crate) fn one_string(args: &[SteelVal], name: &'static str) -> Result<String, SteelErr> {
    if args.len() != 1 {
        steel::stop!(ArityMismatch => "{name} expects 1 arg, got {}", args.len());
    }
    match &args[0] {
        SteelVal::StringV(s) => Ok(s.to_string()),
        _ => steel::stop!(TypeMismatch => "{name}: expected a string, got {:?}", args[0]),
    }
}

// ── with_ctx helper (used by all builtin sub-modules) ─────────────────────────

/// Call `f` with a mutable reference to the current eval context.
///
/// Every builtin uses this to access editor state.  Panics if called outside
/// a [`ScriptingHost::eval_init`] call — i.e., when the TLS slot is `None`.
/// In practice this only happens if someone calls a builtin from a context
/// that is not an active eval (programming error).
pub(crate) fn with_ctx<R>(f: impl FnOnce(&mut EvalCtx) -> R) -> R {
    super::EVAL_CTX.with(|cell| {
        f(cell
            .borrow_mut()
            .as_mut()
            .expect("scripting builtin called outside eval_init"))
    })
}

// ── Bootstrap Scheme ──────────────────────────────────────────────────────────

/// Scheme bootstrap evaluated once during engine init.
///
/// Defines `load-plugin` in terms of the Rust builtins registered below.
/// Uses `dynamic-wind` so `pop-current-plugin!` runs even if `(load path)`
/// raises an error, keeping the attribution stack consistent.
const BOOTSTRAP: &str = r#"
(define (load-plugin name)
  (push-declared-plugin! name)
  (let ((path (resolve-plugin-path name)))
    (when path
      (push-loaded-plugin! name)
      (dynamic-wind
        (lambda () (push-current-plugin! name))
        (lambda () (load path))
        (lambda () (pop-current-plugin!))))))
"#;

// ── Registration ──────────────────────────────────────────────────────────────

/// Register all HUME builtins on `engine` and evaluate the Scheme bootstrap.
///
/// Must be called exactly once during [`ScriptingHost::new`], before any
/// `eval_init` calls.
pub(crate) fn register_all(engine: &mut Engine) {
    // Config / settings
    engine.register_value("set-option!", SteelVal::FuncV(settings::set_option));
    engine.register_value("configure-statusline!", SteelVal::FuncV(statusline::configure_statusline));

    // Step budget
    engine.register_value("hume/yield!", SteelVal::FuncV(interrupt::hume_yield));

    // Keymap
    engine.register_value("bind-key!",       SteelVal::FuncV(keymap_bind::bind_key));
    engine.register_value("bind-wait-char!", SteelVal::FuncV(keymap_bind::bind_wait_char));

    // Plugin lifecycle (called from the Scheme-side load-plugin)
    engine.register_value("push-declared-plugin!", SteelVal::FuncV(plugins::push_declared_plugin));
    engine.register_value("push-loaded-plugin!",   SteelVal::FuncV(plugins::push_loaded_plugin));
    engine.register_value("push-current-plugin!",  SteelVal::FuncV(plugins::push_current_plugin));
    engine.register_value("pop-current-plugin!",   SteelVal::FuncV(plugins::pop_current_plugin));
    engine.register_value("resolve-plugin-path",   SteelVal::FuncV(plugins::resolve_plugin_path));

    // Plugin introspection
    engine.register_value("loaded-plugins",   SteelVal::FuncV(plugins::loaded_plugins));
    engine.register_value("declared-plugins", SteelVal::FuncV(plugins::declared_plugins));

    // Steel command definition and composition
    engine.register_value("define-command!",    SteelVal::FuncV(commands::define_command));
    engine.register_value("call-command!",      SteelVal::FuncV(commands::call_command));
    engine.register_value("request-wait-char!", SteelVal::FuncV(commands::request_wait_char));
    engine.register_value("pending-char",       SteelVal::FuncV(commands::pending_char));
    engine.register_value("cmd-arg",            SteelVal::FuncV(commands::cmd_arg));

    // Filesystem and directory access (sandboxed to <data>/plugins/ and <runtime>/plugins/)
    engine.register_value("data-dir",     SteelVal::FuncV(fs::data_dir));
    engine.register_value("runtime-dir",  SteelVal::FuncV(fs::runtime_dir));
    engine.register_value("path-exists?", SteelVal::FuncV(fs::path_exists));
    engine.register_value("list-dir",     SteelVal::FuncV(fs::list_dir));
    engine.register_value("make-dir",     SteelVal::FuncV(fs::make_dir));
    engine.register_value("delete-dir",   SteelVal::FuncV(fs::delete_dir));

    // Shell — narrow git wrappers only (no generic run-process)
    engine.register_value("git-clone", SteelVal::FuncV(shell::git_clone));
    engine.register_value("git-pull",  SteelVal::FuncV(shell::git_pull));

    // Logging — push messages to the editor message log
    engine.register_value("log!", SteelVal::FuncV(fs::log_msg));

    // Evaluate the Scheme bootstrap (defines `load-plugin`).
    // Runs before any user init.scm, with no TLS context — safe because the
    // bootstrap only uses `define`, which never invokes builtins directly.
    engine
        .compile_and_run_raw_program(BOOTSTRAP.to_owned())
        .expect("HUME scripting bootstrap failed — this is a bug");
}
