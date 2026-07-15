//! Steel builtins for HUME's scripting layer.
//!
//! [`register_all`] registers every builtin on the Steel engine and then evaluates
//! the Scheme bootstrap that defines `load-plugin` and `declare-plugin`.
//! This must be called once during [`ScriptingHost::new`] before any
//! `eval_init` call.

pub(crate) mod buffers;
pub(crate) mod commands;
pub(crate) mod fs;
pub(crate) mod grammar;
pub(crate) mod hooks;
pub(crate) mod ids;
pub(crate) mod install;
pub(crate) mod interrupt;
pub(crate) mod io;
pub(crate) mod keymap_bind;
pub(crate) mod lsp;
pub(crate) mod panes;
pub(crate) mod plugins;
pub(crate) mod sandbox;
pub(crate) mod settings;
pub(crate) mod statusline;
pub(crate) mod syntax;
pub(crate) mod timers;
pub(crate) mod ui;

use std::borrow::Cow;

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;
use steel::steel_vm::engine::Engine;
use steel::steel_vm::register_fn::RegisterFn;

use super::HUME_CTX;

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Return `Err` if we're inside an init eval (buffer/pane builtins are
/// command-mode only).
macro_rules! require_cmd_ctx {
    ($ctx:expr, $name:literal) => {
        if $ctx.is_init {
            steel::stop!(Generic => "{}: not available during init evaluation", $name);
        }
    };
}
pub(crate) use require_cmd_ctx;

/// Return `Err` unless we're at init.scm top level or inside a plugin
/// activation (`is_init` true, or `plugin_stack` non-empty) — the gate
/// config builtins (`set-option!`, `bind-key!`, hook/LSP-server/language
/// registration, …) share, permitting plugin-activation bodies but blocking
/// plain command bodies. See `SteelCtx::is_init`'s doc for why this is a
/// distinct, looser gate than `require_cmd_ctx!`'s.
macro_rules! require_config_ctx {
    ($ctx:expr, $name:expr) => {
        if !$ctx.is_init && $ctx.plugin_stack.is_empty() {
            steel::stop!(Generic =>
                "{}: only valid during init.scm or plugin load, not from a Steel command body",
                $name);
        }
    };
}
pub(crate) use require_config_ctx;

/// Map an `IntoSteelVal` conversion failure to a Steel `ConversionError`.
pub(crate) fn conv_err(e: impl std::fmt::Display) -> SteelErr {
    SteelErr::new(steel::rerrs::ErrorKind::ConversionError, e.to_string())
}

/// Extract a `Vec<String>` from a Steel list value.
///
/// Returns a typed error if the value is not a `ListV` or if any element is not
/// a string.  `param` names the argument for the error message.
pub(crate) fn list_to_strings(val: SteelVal, param: &str) -> Result<Vec<String>, SteelErr> {
    match val {
        SteelVal::ListV(list) => list
            .iter()
            .map(|v| match v {
                SteelVal::StringV(s) => Ok(s.to_string()),
                _ => Err(SteelErr::new(
                    steel::rerrs::ErrorKind::TypeMismatch,
                    format!("{param}: list must contain only strings"),
                )),
            })
            .collect(),
        _ => Err(SteelErr::new(
            steel::rerrs::ErrorKind::TypeMismatch,
            format!("{param}: expected a list"),
        )),
    }
}

/// Extract a `String` from a single positional `SteelVal` argument (the
/// calling convention `register_fn_with_ctx` builtins use — one Rust param
/// per Steel arg, not a `&[SteelVal]` slice). Accepts both strings and
/// symbols, since Scheme callers often pass unquoted symbol literals where a
/// string is semantically expected.
pub(crate) fn string_arg(val: SteelVal, ctx_name: &str) -> Result<String, SteelErr> {
    match val {
        SteelVal::StringV(s) => Ok(s.to_string()),
        SteelVal::SymbolV(s) => Ok(s.to_string()),
        _ => steel::stop!(TypeMismatch => "{}: expected a string", ctx_name),
    }
}

// ── Bootstrap Scheme ──────────────────────────────────────────────────────────

/// Scheme bootstrap evaluated once during Steel engine init.
///
/// Defines `load-plugin`, `declare-plugin` (plugin manifest), and the inline
/// activation machinery (`%activate-plugin-inline`, `%dispatch-command`) atop
/// the Rust builtins below and Steel's `eval-string` (from `steel/meta`).
///
/// Inline activation: `%begin-lazy-activation` (Rust) moves the plugin to
/// `Loading` and returns its `(require "<abs>")` string; `eval-string` runs
/// that inside the live VM (same module pipeline as the engine API, but
/// VM-aware — no `&mut Engine` needed); `%finish-lazy-activation` (Rust)
/// finalizes the state; `with-handler` guarantees the `Failed` transition on
/// any body exception.
///
/// `%dispatch-command` routes: activated plugin command → `command_table`
/// lookup, apply inline; lazy-activation miss → activate inline, retry;
/// native/unknown → `%call-native!`.
//
// declare-plugin — manifest; entries forwarded to %declare-plugin!. A
// zero-trigger call (no #:commands/#:events/#:languages) instead evaluates
// <plugin-dir>/manifest.scm for its own default entries (see
// %begin-manifest-declare!); caller's #:config wins over the manifest's.
// Known limitation, left as-is: a caller's own with-handler around a
// zero-trigger call can hit steel-core 0.8.2's "no open continuation" VM
// panic if manifest.scm raises (same footgun as %activate-plugin-inline;
// see known_limitation_reraise_via_raise_error_inside_outer_tolerant_handler_corrupts_vm_stack
// in lib.rs). `error` instead of `raise-error` panics identically;
// `dynamic-wind` avoids the panic but its cleanup thunk silently skips
// across an outer handler's unwind (see
// known_limitation_dynamic_wind_cleanup_does_not_run_across_an_outer_handlers_unwind),
// which would leave manifest_resolving stuck for every later call this
// session. Swallowing the error instead would break declare-plugin's tested
// propagate-to-caller contract (4 tests in builtins/plugins/tests.rs assert
// it via .expect_err). Left pending an upstream steel-core fix.
//
// load-plugin — eager init-context activation; declares/resolves then
// delegates to %activate-plugin-inline. Valid only during init.scm /
// :reload-config (enforced by %load-plugin!). #:config as above.
//
// %activate-plugin-inline — shared by load-plugin and %dispatch-command's
// lazy-miss path. %begin-lazy-activation returns the require string for
// Declared plugins, #f otherwise (cycle guard + idempotency).
//
// define-command! — register a Steel lambda as a keymap command.
// Positional: name, doc, proc. Keyword (mutually exclusive):
//   #:repeatable #t    — `.` replays this command at the new cursor;
//                        self-contained buffer edits only.
//   #:inline-output #t — bracket dispatch with an alt-screen exit so shell
//                        output streams live.
//
// register-lsp-server! — queues a last-wins registration, applied (with any
// already-open matching buffers attached) at the end of the current eval.
// init-options/settings: Steel data, decoded to JSON at the boundary.
//
// lsp-request — generic LSP bridge. server: registered language name, or #f
// for the focused buffer's attached server. callback: (lambda (err result)),
// exactly one non-#f. #:allow-stale skips the staleness check. #:supersede
// <key> cancels the caller's own previous still-pending request filed under
// the same (server, key) — opt-in, not automatic by method/buffer.
//
// debounce — trailing-edge: each call reschedules proc `ms` out, cancelling
// any still-pending call from a prior invocation. Pure Scheme, no Rust
// debouncer.
//
// diagnostics-for-buffer — bounded, filtered pull. #:severity: floor symbol
// ('error 'warning 'info 'hint) or #f for none. #:range: (list start end)
// char-offset bound, or #f for the whole buffer.
//
// apply-text-edits! — one undoable transaction. edits: list of
// ((start-line start-col) (end-line end-col) text), wire positions.
// #:expect-generation: staleness tag to check against; #f skips it.
//
// apply-workspace-edit! — multi-file engine; reports the modified-buffer
// count.
//
// prompt! — one-shot minibuffer prompt; on-confirm fires once with the
// confirmed text or #f on cancel. No history/completion — a second prompt!
// while one is open errors rather than stacking.
//
// completion-begin! — starts a session (replacing any open one). items:
// decoded CompletionItem hashmaps. #:incomplete: server's isIncomplete flag.
//
// run-inline-output! — process-group-isolated spawn for #:inline-output
// commands (see hume-platform::process::run_inline_output for why this
// can't be Steel's own spawn-process). Blocks; raises on nonzero exit or a
// signal-killed child. Same contract as plum/run!, so call sites need no
// manual exit-code checks.
//
// show-popup! — cursor-anchored floating text panel. #:anchor is reserved;
// 'cursor is the only v1 value. #:dismiss-on-key: the popup is cleared by
// the Editor::handle_key top-of-loop check on the *next* key press, whatever
// key it is (see `gn`/`gp`'s diagnostic overlay) — default #f keeps the
// existing on-mode-change-only dismissal (hover, signature help).
//
// Variadic call! macro — desugars to %dispatch-command. Defined here (not
// only prelude.scm) so test harnesses without the full prelude still have it.
//
// Gated print — capture steel-core's original display*/print* fns and the
// real stdout port ONCE, before PRINT_GATE_SHIMS redefines the names and
// before set_prelude_string runs (register_all) — guaranteeing these are the
// originals.
//
// %port-safe? — writing to `port` is TUI-safe unless it IS the real stdout
// port, in which case defer to the gate. Shared by every shim's
// explicit-port branch.
const BOOTSTRAP: &str = r#"
(require-builtin steel/meta as hm.)

(define (declare-plugin name #:commands  [commands  '()]
                             #:events    [events    '()]
                             #:languages [languages '()]
                             #:config    [config    (hash)])
  (if (and (null? commands) (null? events) (null? languages))
      (let ((prog (%begin-manifest-declare! name config)))
        (when prog
          (with-handler
            (lambda (e) (%finish-manifest-declare! name #f) (raise-error e))
            (begin (hm.eval-string prog) (%finish-manifest-declare! name #t)))))
      (%declare-plugin! name commands events languages config)))

(define (load-plugin name #:config [config (hash)])
  (%load-plugin! name config)
  (%activate-plugin-inline name))

(define (%activate-plugin-inline id)
  (let ((prog (%begin-lazy-activation id)))
    (when prog
      (with-handler
        (lambda (e) (%finish-lazy-activation id #f) (raise-error e))
        (begin (hm.eval-string prog) (%finish-lazy-activation id #t))))))

(define (define-command! name doc proc
                         #:repeatable    [repeatable    #f]
                         #:inline-output [inline-output #f])
  (%define-command! name doc proc repeatable inline-output))

(define (%dispatch-command name args)
  (let ((proc (%lookup-plugin-proc name)))
    (if proc
        (apply proc args)
        (let ((owner (%lazy-command-owner name)))
          (if owner
              (begin
                (%activate-plugin-inline owner)
                (let ((proc2 (%lookup-plugin-proc name)))
                  (if proc2 (apply proc2 args) (%call-native! name args))))
              (%call-native! name args))))))

(define (register-lsp-server! language #:command command
                                        #:args [args '()]
                                        #:root-markers [root-markers '()]
                                        #:init-options [init-options #f]
                                        #:settings [settings #f])
  (%register-lsp-server! language command args root-markers init-options settings))

(define (lsp-request server method params callback #:allow-stale [allow-stale #f]
                                                     #:supersede [supersede #f])
  (%lsp-request server method params callback allow-stale supersede))

(define (debounce ms proc)
  (let ((pending (box #f)))
    (lambda args
      (let ((prev (unbox pending)))
        (when prev (cancel-timer! prev)))
      (set-box! pending (after ms (lambda () (apply proc args)))))))

;; debounce-by — like `debounce`, but keyed per first-argument value instead
;; of one shared pending timer: a call keyed `k1` never cancels a call keyed
;; `k2`. Same trailing-edge semantics per key. Relies on the calling
;; convention already used everywhere `debounce` wraps a single-bid handler
;; (`(lambda (bid) ...)`) — the key is `(car args)`, not a separate keyfn
;; argument, so swapping `debounce` for `debounce-by` at an existing call
;; site needs no other change.
(define (debounce-by ms proc)
  (let ((pending (box (hash))))
    (lambda args
      (let* ((key (car args))
             (table (unbox pending)))
        (when (hash-contains? table key)
          (cancel-timer! (hash-ref table key)))
        (set-box! pending
          (hash-insert (unbox pending) key
            (after ms (lambda ()
                        (set-box! pending (hash-remove (unbox pending) key))
                        (apply proc args)))))))))

(define (diagnostics-for-buffer bid #:severity [severity #f] #:range [range #f])
  (%diagnostics-for-buffer bid severity range))

(define (apply-text-edits! bid edits #:expect-generation [gen #f])
  (%apply-text-edits! bid edits gen))

(define (apply-workspace-edit! wsedit)
  (let ((n (%apply-workspace-edit! wsedit)))
    (log! 'info (to-string n " buffers modified — :wa writes all"))
    n))

(define (prompt! label on-confirm #:prefill [prefill ""])
  (%prompt! label prefill on-confirm))

(define (completion-begin! bid items #:incomplete [incomplete #f])
  (%completion-begin! bid items incomplete))

(define (run-inline-output! cmd args #:cwd [cwd #f])
  (let ([code (%run-inline-output! cmd args cwd)])
    (unless (= code 0)
      (error (string-append cmd ": failed (exit " (number->string code) ")")))))

(define (show-popup! text #:anchor [anchor 'cursor] #:dismiss-on-key [dismiss-on-key #f])
  (%show-popup! text anchor dismiss-on-key))

(define-syntax call!
  (syntax-rules ()
    ((_ name args ...)
     (%dispatch-command name (list args ...)))))

(define %raw-displayln displayln)
(define %raw-display display)
(define %raw-print print)
(define %raw-println println)
(define %raw-newline newline)
(define %raw-write write)
(define %raw-write-string write-string)
(define %raw-write-char write-char)
(define %raw-simple-display simple-display)
(define %raw-simple-displayln simple-displayln)
(define %stdout-port (current-output-port))
(define (%port-safe? port)
  (if (eq? port %stdout-port)
      (%stdout-gate!)
      #t))
(define (%stdout-safe?) (%port-safe? (current-output-port)))
"#;

// PRINT_GATE_SHIMS is appended both to BOOTSTRAP (top level) and, verbatim,
// to steel-core's own prelude string via set_prelude_string — since the
// prelude is prepended to every `(require "path.scm")` unit, this closes the
// gap where required-module (every real plugin's) print calls would
// otherwise resolve straight to steel-core's raw, ungated originals. See
// io.rs's module doc for the full root-cause writeup and the rest-only
// parameter-list requirement every shim below follows.
//
// Explicit-port forms (`(display obj port)`, …) consult `%port-safe?` on the
// supplied port rather than forwarding unconditionally, since the caller (or
// steel-core's own error printer) can pass `(current-output-port)` itself.
// Accepted side effect: an explicit call to the real stdout port while the
// gate is closed now silently suppresses instead of raising an arity error.
//
// `write-string`/`write-char` are shimmed even though their steel-core
// originals bypass `(current-output-port)` in the implicit-arg case — they
// default straight to real stdout regardless, so they're exactly as unsafe
// as `display` and need the same gate. `simple-display`/`simple-displayln`
// always resolve `(current-output-port)` themselves, so they're gated the
// same way.
const PRINT_GATE_SHIMS: &str = r#"
(define (displayln . args) (when (%stdout-safe?) (apply %raw-displayln args)))
(define (display . args)
  (if (pair? (cdr args))
      (when (%port-safe? (cadr args)) (apply %raw-display args))
      (when (%stdout-safe?) (%raw-display (car args)))))
(define (print . args)
  (if (pair? (cdr args))
      (when (%port-safe? (cadr args)) (apply %raw-print args))
      (when (%stdout-safe?) (%raw-print (car args)))))
(define (println . args)
  (if (pair? (cdr args))
      (when (%port-safe? (cadr args)) (apply %raw-println args))
      (when (%stdout-safe?) (%raw-println (car args)))))
(define (write . args)
  (if (pair? (cdr args))
      (when (%port-safe? (cadr args)) (apply %raw-write args))
      (when (%stdout-safe?) (%raw-write (car args)))))
(define (write-string . args)
  (if (pair? (cdr args))
      (when (%port-safe? (cadr args)) (apply %raw-write-string args))
      (when (%stdout-safe?) (%raw-write-string (car args) (current-output-port)))))
(define (write-char . args)
  (if (pair? (cdr args))
      (when (%port-safe? (cadr args)) (apply %raw-write-char args))
      (when (%stdout-safe?) (%raw-write-char (car args) (current-output-port)))))
(define (simple-display . args) (when (%stdout-safe?) (apply %raw-simple-display args)))
(define (simple-displayln . args) (when (%stdout-safe?) (apply %raw-simple-displayln args)))
(define (newline . args)
  (if (pair? args)
      (when (%port-safe? (car args)) (apply %raw-newline args))
      (when (%stdout-safe?) (%raw-newline))))
"#;

// ── Registration ──────────────────────────────────────────────────────────────

/// Register all HUME builtins on `steel` and evaluate the Scheme bootstrap.
///
/// Must be called exactly once during [`ScriptingHost::new`], before any
/// `eval_init` calls.
pub(crate) fn register_all(steel: &mut Engine) {
    // Pre-register HUME_CTX so supply_context_arg can generate its wrapper
    // functions without a FreeIdentifier error.  The real SteelVal::Reference
    // is injected at eval / dispatch time via steel.update_value.
    steel.register_value(HUME_CTX, SteelVal::Void);

    // Context-injected builtins: Steel auto-injects the HUME_CTX global as
    // the first `&mut SteelCtx` argument via register_fn_with_ctx.

    // Config / settings
    steel.register_fn_with_ctx(HUME_CTX, "set-option!", settings::set_option);
    steel.register_fn_with_ctx(HUME_CTX, "get-option", settings::get_option);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "configure-statusline!",
        statusline::configure_statusline,
    );

    // Step budget
    steel.register_fn_with_ctx(HUME_CTX, "hume/yield!", interrupt::hume_yield);

    // Keymap
    steel.register_fn_with_ctx(HUME_CTX, "bind-key!", keymap_bind::bind_key);
    steel.register_fn_with_ctx(HUME_CTX, "bind-key-extend!", keymap_bind::bind_key_extend);
    steel.register_fn_with_ctx(HUME_CTX, "unbind-key!", keymap_bind::unbind_key);
    steel.register_fn_with_ctx(HUME_CTX, "bind-wait-char!", keymap_bind::bind_wait_char);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "set-register-prefix!",
        commands::set_register_prefix,
    );

    // Plugin lifecycle
    steel.register_fn_with_ctx(HUME_CTX, "%declare-plugin!", plugins::declare_plugin);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "resolve-plugin-path",
        plugins::resolve_plugin_path,
    );

    // Plugin introspection and explicit activation
    steel.register_fn_with_ctx(HUME_CTX, "loaded-plugins", plugins::loaded_plugins);
    steel.register_fn_with_ctx(HUME_CTX, "declared-plugins", plugins::declared_plugins);
    steel.register_fn_with_ctx(HUME_CTX, "plugin-config", plugins::plugin_config);
    steel.register_fn_with_ctx(HUME_CTX, "%load-plugin!", plugins::load_plugin);

    // Inline activation primitives — called from the %activate-plugin-inline
    // Scheme helper to drive mid-eval plugin loading without &mut Engine.
    steel.register_fn_with_ctx(
        HUME_CTX,
        "%begin-lazy-activation",
        plugins::begin_lazy_activation,
    );
    steel.register_fn_with_ctx(
        HUME_CTX,
        "%finish-lazy-activation",
        plugins::finish_lazy_activation,
    );
    steel.register_fn_with_ctx(HUME_CTX, "%lazy-command-owner", plugins::lazy_command_owner);

    // Manifest resolution — zero-trigger declare-plugin routes here to eval
    // <plugin-dir>/manifest.scm so the plugin can declare its own defaults.
    steel.register_fn_with_ctx(
        HUME_CTX,
        "%begin-manifest-declare!",
        plugins::begin_manifest_declare,
    );
    steel.register_fn_with_ctx(
        HUME_CTX,
        "%finish-manifest-declare!",
        plugins::finish_manifest_declare,
    );

    // Hook registration — init-only
    steel.register_fn_with_ctx(HUME_CTX, "register-hook!", hooks::register_hook);

    // Steel command definition.
    // %define-command! is the native primitive; the (define-command! …) Steel wrapper
    // in BOOTSTRAP exposes keyword args (#:repeatable, #:inline-output).
    steel.register_fn_with_ctx(HUME_CTX, "%define-command!", commands::define_command);
    // %call-native! is the Rust leaf for native/unknown dispatch; the variadic
    // (call! name args…) macro desugars to (%dispatch-command …) which routes
    // activated plugin commands inline in Steel and falls back here for everything else.
    steel.register_fn_with_ctx(HUME_CTX, "%call-native!", commands::call_command_primitive);
    // %lookup-plugin-proc: returns the Steel closure for an activated plugin command,
    // or #f. Called by %dispatch-command in Steel to decide inline-apply vs. %call-native!.
    steel.register_fn_with_ctx(
        HUME_CTX,
        "%lookup-plugin-proc",
        commands::lookup_plugin_proc,
    );
    steel.register_fn_with_ctx(HUME_CTX, "request-wait-char!", commands::request_wait_char);
    steel.register_fn_with_ctx(HUME_CTX, "pending-char", commands::pending_char);
    steel.register_fn_with_ctx(HUME_CTX, "command-plugin", commands::command_plugin);

    // Grammar compilation — sandbox-free, full-trust plugin model. Kept as a
    // Rust builtin only for the Windows compiler-selection dance (see
    // grammar.rs's module doc); `grammar-output-path` moved to Scheme.
    steel.register_fn_with_ctx(HUME_CTX, "compile-grammar!", grammar::compile_grammar);

    // LSP server install pipeline — sha256 hashing, archive unpacking,
    // platform id, cross-process install lock (see docs/LSP-INSTALL.md).
    // Sandbox-free — full-trust plugin model. `verify-sha256!`/`exe-on-path?`
    // /`git-clone`/`curl-fetch`/`npm-install!` moved to Scheme + Steel's own
    // `steel/process` stdlib (`which`, `spawn-process`).
    steel.register_value("hume-target", SteelVal::FuncV(install::hume_target));
    steel.register_fn_with_ctx(HUME_CTX, "sha256-file", install::sha256_file);
    steel.register_fn_with_ctx(HUME_CTX, "unpack-gz", install::unpack_gz);
    steel.register_fn_with_ctx(HUME_CTX, "unpack-zip", install::unpack_zip);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "acquire-install-lock!",
        install::acquire_install_lock,
    );
    steel.register_fn("release-install-lock!", install::release_install_lock);
    steel.register_fn_with_ctx(HUME_CTX, "%run-inline-output!", install::run_inline_output);

    // Logging — push messages to the editor message log
    steel.register_fn_with_ctx(HUME_CTX, "log!", crate::log::log_msg);

    // %stdout-gate! is the Rust leaf behind the gated print shims (displayln,
    // display, print, println, newline) — see io.rs and PRINT_GATE_SHIMS above.
    steel.register_fn_with_ctx(HUME_CTX, "%stdout-gate!", io::stdout_gate);

    // Opaque ID predicates and equality — context-free; no SteelCtx needed.
    steel.register_fn("buffer-id?", ids::is_buffer_id);
    steel.register_fn("pane-id?", ids::is_pane_id);
    steel.register_fn("buffer-id=?", ids::buffer_id_equal);
    steel.register_fn("pane-id=?", ids::pane_id_equal);

    // Multi-buffer read-only builtins
    steel.register_fn_with_ctx(HUME_CTX, "current-buffer", buffers::current_buffer);
    steel.register_fn_with_ctx(HUME_CTX, "current-pane", buffers::current_pane);
    steel.register_fn_with_ctx(HUME_CTX, "buffers", buffers::buffers);
    steel.register_fn_with_ctx(HUME_CTX, "panes", buffers::panes);
    steel.register_fn_with_ctx(HUME_CTX, "buffer-path", buffers::buffer_path);
    steel.register_fn_with_ctx(HUME_CTX, "buffer-name", buffers::buffer_name);
    steel.register_fn_with_ctx(HUME_CTX, "buffer-dirty?", buffers::buffer_dirty);
    // Live cursor read — reflects synchronous edits in the same eval.
    steel.register_fn_with_ctx(
        HUME_CTX,
        "current-line-number",
        buffers::current_line_number,
    );
    steel.register_fn_with_ctx(HUME_CTX, "current-selections", buffers::current_selections);
    steel.register_fn_with_ctx(HUME_CTX, "char-index->line", buffers::char_index_to_line);

    // Multi-buffer mutating builtins
    steel.register_fn_with_ctx(HUME_CTX, "open-buffer!", buffers::open_buffer);
    steel.register_fn_with_ctx(HUME_CTX, "close-buffer!", buffers::close_buffer);
    steel.register_fn_with_ctx(HUME_CTX, "switch-to-buffer!", buffers::switch_to_buffer);

    // Language identity and grammar builtins
    steel.register_fn_with_ctx(HUME_CTX, "%define-language!", syntax::define_language);
    steel.register_fn_with_ctx(HUME_CTX, "%register-grammar!", syntax::register_grammar);

    // LSP server registration — last-wins, queued (like language regs) and
    // applied at the end of the current eval, from init, plugin activation,
    // or a command/hook body.
    steel.register_fn_with_ctx(HUME_CTX, "%register-lsp-server!", lsp::register_lsp_server);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "unregister-lsp-server!",
        lsp::unregister_lsp_server,
    );
    // Lifecycle — stop/restart a running server, or open the status view.
    // Command-context only (core:lsp's `:lsp-status`/`:lsp-stop`/`:lsp-restart`
    // typed commands are the only callers).
    steel.register_fn_with_ctx(HUME_CTX, "lsp-stop!", lsp::lsp_stop);
    steel.register_fn_with_ctx(HUME_CTX, "lsp-restart!", lsp::lsp_restart);
    steel.register_fn_with_ctx(HUME_CTX, "lsp-show-status!", lsp::lsp_show_status);
    // Generic LSP bridge — any protocol method reachable from Steel.
    steel.register_fn_with_ctx(HUME_CTX, "%lsp-request", lsp::lsp_request);
    steel.register_fn_with_ctx(HUME_CTX, "lsp-notify", lsp::lsp_notify);
    steel.register_fn_with_ctx(HUME_CTX, "on-lsp-notification", lsp::on_lsp_notification);
    // Introspection
    steel.register_fn_with_ctx(HUME_CTX, "lsp-capabilities", lsp::lsp_capabilities);
    steel.register_fn_with_ctx(HUME_CTX, "lsp-server-status", lsp::lsp_server_status);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "lsp-server-for-buffer",
        lsp::lsp_server_for_buffer,
    );
    steel.register_fn_with_ctx(
        HUME_CTX,
        "lsp-registered-for-language?",
        lsp::lsp_registered_for_language,
    );
    steel.register_fn_with_ctx(HUME_CTX, "lsp-position-params", lsp::lsp_position_params);
    steel.register_fn_with_ctx(HUME_CTX, "lsp-range-params", lsp::lsp_range_params);
    steel.register_fn_with_ctx(HUME_CTX, "viewport-range", lsp::viewport_range);
    steel.register_fn_with_ctx(HUME_CTX, "buffer-generation", buffers::buffer_generation);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "register-trigger-chars!",
        lsp::register_trigger_chars,
    );

    // Decoration stores + diagnostics pull.
    steel.register_fn_with_ctx(HUME_CTX, "set-inlay-hints!", lsp::set_inlay_hints);
    steel.register_fn_with_ctx(HUME_CTX, "set-signs!", lsp::set_signs);
    steel.register_fn_with_ctx(HUME_CTX, "set-virtual-lines!", lsp::set_virtual_lines);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "set-inline-diagnostics!",
        lsp::set_inline_diagnostics,
    );
    steel.register_fn_with_ctx(HUME_CTX, "set-extra-highlights!", lsp::set_extra_highlights);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "%diagnostics-for-buffer",
        lsp::diagnostics_for_buffer,
    );
    steel.register_fn_with_ctx(HUME_CTX, "diagnostic-counts", lsp::diagnostic_counts);

    // Edit + navigation primitives.
    steel.register_fn_with_ctx(HUME_CTX, "%apply-text-edits!", lsp::apply_text_edits);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "%apply-workspace-edit!",
        lsp::apply_workspace_edit,
    );
    steel.register_fn_with_ctx(HUME_CTX, "goto-location!", lsp::goto_location);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "selection-spans-full-line?",
        lsp::selection_spans_full_line,
    );

    // Minibuffer prompt.
    steel.register_fn_with_ctx(HUME_CTX, "%prompt!", lsp::prompt);
    steel.register_fn_with_ctx(HUME_CTX, "symbol-under-cursor", lsp::symbol_under_cursor);

    // Completion orchestration.
    steel.register_fn_with_ctx(HUME_CTX, "%completion-begin!", lsp::completion_begin);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "completion-update-filter!",
        lsp::completion_update_filter,
    );
    steel.register_fn_with_ctx(HUME_CTX, "completion-top", lsp::completion_top);
    steel.register_fn_with_ctx(HUME_CTX, "completion-accept!", lsp::completion_accept);
    steel.register_fn_with_ctx(HUME_CTX, "completion-dismiss!", lsp::completion_dismiss);

    // Cursor-anchored popup widget.
    steel.register_fn_with_ctx(HUME_CTX, "%show-popup!", ui::show_popup);
    steel.register_fn_with_ctx(HUME_CTX, "close-popup!", ui::close_popup);

    // Selection menu widget.
    steel.register_fn_with_ctx(HUME_CTX, "show-menu!", ui::show_menu);
    steel.register_fn_with_ctx(HUME_CTX, "close-menu!", ui::close_menu);

    // Class B bottom drawer.
    steel.register_fn_with_ctx(HUME_CTX, "show-drawer-list!", ui::show_drawer_list);
    steel.register_fn_with_ctx(HUME_CTX, "close-drawer!", ui::close_drawer);

    // Timers — not LSP-specific, but added as part of the LSP work.
    steel.register_fn_with_ctx(HUME_CTX, "after", timers::after);
    steel.register_fn_with_ctx(HUME_CTX, "cancel-timer!", timers::cancel_timer);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "language-has-grammar?",
        syntax::language_has_grammar,
    );
    steel.register_fn_with_ctx(HUME_CTX, "buffer-language", buffers::buffer_language);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "set-buffer-language!",
        buffers::set_buffer_language_steel,
    );

    // Pane stubs — reserved names for M9+ :split feature.
    // These never use SteelCtx so they register as plain register_fn.
    steel.register_fn("open-pane!", panes::open_pane);
    steel.register_fn("close-pane!", panes::close_pane);
    steel.register_fn("focus-pane!", panes::focus_pane);
    steel.register_fn("pane-buffer", panes::pane_buffer);
    steel.register_fn("pane-set-buffer!", panes::pane_set_buffer);

    // Context-free builtins: editor-integration info that reads from
    // SCRIPT_DIRS TLS, plus path-join's Windows UNC-prefix stripping.
    // General filesystem access is Steel's own steel/filesystem stdlib.
    steel.register_value("data-dir", SteelVal::FuncV(fs::data_dir));
    steel.register_value("runtime-dir", SteelVal::FuncV(fs::runtime_dir));
    steel.register_value("path-join", SteelVal::FuncV(fs::path_join));

    // Evaluate the Scheme bootstrap (defines `load-plugin`, and — at its
    // tail — captures steel-core's original print functions/port before
    // anything shadows them). Runs before any user init.scm; HUME_CTX is not
    // yet set but the bootstrap only uses `define`, so no builtins are
    // called at this point.
    steel
        .compile_and_run_raw_program(BOOTSTRAP.to_owned())
        .expect("HUME scripting bootstrap failed — this is a bug");

    // PRINT_GATE_SHIMS must compile as its OWN program, separate from
    // BOOTSTRAP: steel-core rejects a single compiled unit that both
    // references a name (the `%raw-*` captures above) and redefines that
    // same name later in the same unit — "variable redefined within the top
    // level definition" / "cannot reference an identifier before its
    // definition" (verified empirically against steel-core 0.8.2; see
    // io.rs's module doc). Splitting into two sequential top-level programs
    // sidesteps this: by the time this call compiles, `displayln` etc. are
    // ordinary already-bound globals, and redefining them here is a plain
    // global rebind — no self-reference within the same unit.
    steel
        .compile_and_run_raw_program(PRINT_GATE_SHIMS.to_owned())
        .expect("HUME scripting print-gate shims failed — this is a bug");

    // Append the same shims to steel-core's prelude string. The prelude is
    // prepended to every `(require "path.scm")` compilation unit (steel-core
    // internals — see io.rs's module doc), so this closes the gap where a
    // plugin's own displayln/display/print/println/newline calls would
    // otherwise resolve to steel-core's raw, ungated originals instead of
    // HUME's gate. Unlike the top-level case above, a required module's
    // import-then-body-define compiles as one unit without conflict (the
    // shim's `define` simply overwrites the mangled slot the prelude import
    // created moments earlier), so no splitting is needed here.
    steel.set_prelude_string(Cow::Owned(format!(
        "{}{PRINT_GATE_SHIMS}",
        steel::compiler::modules::PRELUDE_STRING
    )));
}
