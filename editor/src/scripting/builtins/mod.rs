//! Steel builtins for HUME's scripting layer.
//!
//! [`register_all`] registers every builtin on the engine and then evaluates
//! the Scheme bootstrap that defines `load-plugin`.  This must be called once
//! during [`ScriptingHost::new`] before any `eval_init` call.

pub(crate) mod keymap_bind;
pub(crate) mod plugins;
pub(crate) mod settings;

use steel::steel_vm::engine::Engine;
use steel::rvals::SteelVal;

use super::EvalCtx;

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

    // Keymap
    engine.register_value("bind-key!", SteelVal::FuncV(keymap_bind::bind_key));

    // Plugin lifecycle (called from the Scheme-side load-plugin)
    engine.register_value("push-declared-plugin!", SteelVal::FuncV(plugins::push_declared_plugin));
    engine.register_value("push-loaded-plugin!",   SteelVal::FuncV(plugins::push_loaded_plugin));
    engine.register_value("push-current-plugin!",  SteelVal::FuncV(plugins::push_current_plugin));
    engine.register_value("pop-current-plugin!",   SteelVal::FuncV(plugins::pop_current_plugin));
    engine.register_value("resolve-plugin-path",   SteelVal::FuncV(plugins::resolve_plugin_path));

    // Plugin introspection
    engine.register_value("loaded-plugins",   SteelVal::FuncV(plugins::loaded_plugins));
    engine.register_value("declared-plugins", SteelVal::FuncV(plugins::declared_plugins));

    // Evaluate the Scheme bootstrap (defines `load-plugin`).
    // Runs before any user init.scm, with no TLS context — safe because the
    // bootstrap only uses `define`, which never invokes builtins directly.
    engine
        .compile_and_run_raw_program(BOOTSTRAP.to_owned())
        .expect("HUME scripting bootstrap failed — this is a bug");
}
