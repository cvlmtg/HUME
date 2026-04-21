//! Steel builtins for HUME's scripting layer.
//!
//! [`register_all`] registers every builtin on the engine and then evaluates
//! the Scheme bootstrap that defines `load-plugin`.  This must be called once
//! during [`ScriptingHost::new`] before any `eval_init` call.

pub(crate) mod buffers;
pub(crate) mod commands;
pub(crate) mod fs;
pub(crate) mod hooks;
pub(crate) mod ids;
pub(crate) mod interrupt;
pub(crate) mod keymap_bind;
pub(crate) mod plugins;
pub(crate) mod settings;
pub(crate) mod shell;
pub(crate) mod statusline;

use steel::steel_vm::engine::Engine;
use steel::steel_vm::register_fn::RegisterFn;
use steel::rvals::SteelVal;
use steel::rerrs::SteelErr;

use super::HUME_CTX;

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Return `Err` if we're inside an init eval (editor refs are None).
macro_rules! require_cmd_ctx {
    ($ctx:expr, $name:literal) => {
        if $ctx.is_init {
            steel::stop!(Generic => "{}: not available during init evaluation", $name);
        }
    };
}
pub(crate) use require_cmd_ctx;

/// Extract the single string argument from `args`, returning a Steel error on
/// arity or type mismatch.  Used by fs builtins that still take `&[SteelVal]`.
pub(crate) fn one_string(args: &[SteelVal], name: &'static str) -> Result<String, SteelErr> {
    if args.len() != 1 {
        steel::stop!(ArityMismatch => "{name} expects 1 arg, got {}", args.len());
    }
    match &args[0] {
        SteelVal::StringV(s) => Ok(s.to_string()),
        _ => steel::stop!(TypeMismatch => "{name}: expected a string, got {:?}", args[0]),
    }
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
    // Pre-register HUME_CTX so supply_context_arg can generate its wrapper
    // functions without a FreeIdentifier error.  The real SteelVal::Reference
    // is injected at eval / dispatch time via engine.update_value.
    engine.register_value(HUME_CTX, SteelVal::Void);

    // Context-injected builtins: Steel auto-injects the HUME_CTX global as
    // the first `&mut SteelCtx` argument via register_fn_with_ctx.

    // Config / settings
    engine.register_fn_with_ctx(HUME_CTX, "set-option!",          settings::set_option);
    engine.register_fn_with_ctx(HUME_CTX, "configure-statusline!", statusline::configure_statusline);

    // Step budget
    engine.register_fn_with_ctx(HUME_CTX, "hume/yield!", interrupt::hume_yield);

    // Keymap
    engine.register_fn_with_ctx(HUME_CTX, "bind-key!",       keymap_bind::bind_key);
    engine.register_fn_with_ctx(HUME_CTX, "bind-wait-char!", keymap_bind::bind_wait_char);

    // Plugin lifecycle (called from the Scheme-side load-plugin)
    engine.register_fn_with_ctx(HUME_CTX, "push-declared-plugin!", plugins::push_declared_plugin);
    engine.register_fn_with_ctx(HUME_CTX, "push-loaded-plugin!",   plugins::push_loaded_plugin);
    engine.register_fn_with_ctx(HUME_CTX, "push-current-plugin!",  plugins::push_current_plugin);
    engine.register_fn_with_ctx(HUME_CTX, "pop-current-plugin!",   plugins::pop_current_plugin);
    engine.register_fn_with_ctx(HUME_CTX, "resolve-plugin-path",   plugins::resolve_plugin_path);

    // Plugin introspection
    engine.register_fn_with_ctx(HUME_CTX, "loaded-plugins",   plugins::loaded_plugins);
    engine.register_fn_with_ctx(HUME_CTX, "declared-plugins", plugins::declared_plugins);

    // Hook registration — init-only
    engine.register_fn_with_ctx(HUME_CTX, "register-hook!", hooks::register_hook);

    // Steel command definition and composition
    engine.register_fn_with_ctx(HUME_CTX, "define-command!",    commands::define_command);
    engine.register_fn_with_ctx(HUME_CTX, "call!",              commands::call_command);
    // Back-compat alias — prefer call! in new code.
    engine.register_fn_with_ctx(HUME_CTX, "call-command!",      commands::call_command);
    engine.register_fn_with_ctx(HUME_CTX, "request-wait-char!", commands::request_wait_char);
    engine.register_fn_with_ctx(HUME_CTX, "pending-char",       commands::pending_char);
    engine.register_fn_with_ctx(HUME_CTX, "cmd-arg",            commands::cmd_arg);
    engine.register_fn_with_ctx(HUME_CTX, "command-plugin",     commands::command_plugin);

    // Shell — narrow git wrappers only (no generic run-process)
    engine.register_fn_with_ctx(HUME_CTX, "git-clone", shell::git_clone);
    engine.register_fn_with_ctx(HUME_CTX, "git-pull",  shell::git_pull);

    // Logging — push messages to the editor message log
    engine.register_fn_with_ctx(HUME_CTX, "log!", fs::log_msg);

    // Opaque ID predicates — context-free; no SteelCtx needed.
    engine.register_fn("buffer-id?", ids::is_buffer_id);
    engine.register_fn("pane-id?",   ids::is_pane_id);

    // Multi-buffer read-only builtins
    engine.register_fn_with_ctx(HUME_CTX, "current-buffer", buffers::current_buffer);
    engine.register_fn_with_ctx(HUME_CTX, "current-pane",   buffers::current_pane);
    engine.register_fn_with_ctx(HUME_CTX, "buffers",        buffers::buffers);
    engine.register_fn_with_ctx(HUME_CTX, "panes",          buffers::panes);
    engine.register_fn_with_ctx(HUME_CTX, "buffer-path",    buffers::buffer_path);
    engine.register_fn_with_ctx(HUME_CTX, "buffer-name",    buffers::buffer_name);
    engine.register_fn_with_ctx(HUME_CTX, "buffer-dirty?",  buffers::buffer_dirty);

    // Multi-buffer mutating builtins
    engine.register_fn_with_ctx(HUME_CTX, "open-buffer!",      buffers::open_buffer);
    engine.register_fn_with_ctx(HUME_CTX, "close-buffer!",     buffers::close_buffer);
    engine.register_fn_with_ctx(HUME_CTX, "switch-to-buffer!", buffers::switch_to_buffer);

    // Pane stubs — reserved names for M9+ :split feature
    engine.register_fn_with_ctx(HUME_CTX, "open-pane!",        buffers::open_pane);
    engine.register_fn_with_ctx(HUME_CTX, "close-pane!",       buffers::close_pane);
    engine.register_fn_with_ctx(HUME_CTX, "focus-pane!",       buffers::focus_pane);
    engine.register_fn_with_ctx(HUME_CTX, "pane-buffer",       buffers::pane_buffer);
    engine.register_fn_with_ctx(HUME_CTX, "pane-set-buffer!",  buffers::pane_set_buffer);

    // Context-free builtins: sandboxed filesystem ops that read from SCRIPT_DIRS TLS.
    engine.register_value("data-dir",     SteelVal::FuncV(fs::data_dir));
    engine.register_value("runtime-dir",  SteelVal::FuncV(fs::runtime_dir));
    engine.register_value("path-join",    SteelVal::FuncV(fs::path_join));
    engine.register_value("path-exists?", SteelVal::FuncV(fs::path_exists));
    engine.register_value("list-dir",     SteelVal::FuncV(fs::list_dir));
    engine.register_value("make-dir",     SteelVal::FuncV(fs::make_dir));
    engine.register_value("delete-dir",   SteelVal::FuncV(fs::delete_dir));

    // Evaluate the Scheme bootstrap (defines `load-plugin`).
    // Runs before any user init.scm; HUME_CTX is not yet set but the
    // bootstrap only uses `define`, so no builtins are called at this point.
    engine
        .compile_and_run_raw_program(BOOTSTRAP.to_owned())
        .expect("HUME scripting bootstrap failed — this is a bug");
}
