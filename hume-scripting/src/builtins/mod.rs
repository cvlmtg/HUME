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
pub(crate) mod interrupt;
pub(crate) mod io;
pub(crate) mod keymap_bind;
pub(crate) mod lsp;
pub(crate) mod panes;
pub(crate) mod plugins;
pub(crate) mod sandbox;
pub(crate) mod settings;
pub(crate) mod shell;
pub(crate) mod statusline;
pub(crate) mod syntax;
pub(crate) mod timers;
pub(crate) mod ui;

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

/// Scheme bootstrap evaluated once during Steel engine init.
///
/// Defines `load-plugin`, `declare-plugin` (plugin manifest), and the inline
/// activation machinery (`%activate-plugin-inline`, `%dispatch-command`) in
/// terms of the Rust builtins registered below and Steel's `eval-string`
/// (imported from `steel/meta`).
///
/// Inline activation: `%activate-plugin-inline` drives the mid-eval plugin load.
/// `%begin-lazy-activation` (Rust) transitions the plugin to `Loading` and returns
/// the `(require "<abs>")` string.  `eval-string` runs that require inside the live
/// VM (same module pipeline as the engine API, but VM-aware — no `&mut Engine`
/// needed).  `%finish-lazy-activation` (Rust) finalises the state.  `with-handler`
/// from the stdlib guarantees the `Failed` transition on any body exception.
///
/// `%dispatch-command` routes by command type:
///   - activated plugin command → `command_table` lookup → apply closure inline;
///   - lazy activation command  → activate inline, then retry;
///   - native / unknown         → `%call-native!` (Rust, sync for native).
const BOOTSTRAP: &str = r#"
(require-builtin steel/meta as hm.)

; declare-plugin — plugin manifest; activation entries forwarded to %declare-plugin!.
; At least one activation entry (#:commands/#:events/#:languages) is required.
; #:config is an opaque value (typically a hash) the plugin body reads back via
; (plugin-config) whenever activation eventually runs it.
(define (declare-plugin name #:commands  [commands  '()]
                             #:events    [events    '()]
                             #:languages [languages '()]
                             #:config    [config    (hash)])
  (%declare-plugin! name commands events languages config))

; load-plugin — eager init-context activation; delegates to the shared
; inline-activation helper after declaring/resolving the plugin.
; Valid only during init.scm / :reload-config (enforced by %load-plugin!).
; #:config is an opaque value (typically a hash) the plugin body reads back via
; (plugin-config).
(define (load-plugin name #:config [config (hash)])
  (%load-plugin! name config)
  (%activate-plugin-inline name))

; %activate-plugin-inline — shared activation helper used by both load-plugin
; and %dispatch-command's lazy-miss path.
; %begin-lazy-activation returns the (require "…") string for Declared plugins,
; or #f for Loading/Loaded/Failed/absent (cycle guard + idempotency).
(define (%activate-plugin-inline id)
  (let ((prog (%begin-lazy-activation id)))
    (when prog
      (with-handler
        (lambda (e) (%finish-lazy-activation id #f) (raise-error e))
        (begin (hm.eval-string prog) (%finish-lazy-activation id #t))))))

; define-command! — register a Steel lambda as a keymap command.
;
; Positional args: name (string), doc (string), proc (lambda).
; Optional keyword args (after proc):
;   #:repeatable #t    — pressing `.` replays this command at the new cursor.
;                        Use only for self-contained buffer edits. Mutually
;                        exclusive with #:inline-output.
;   #:inline-output #t — bracket dispatch with an alt-screen exit so shell
;                        output streams live to the terminal. Mutually exclusive
;                        with #:repeatable.
(define (define-command! name doc proc
                         #:repeatable    [repeatable    #f]
                         #:inline-output [inline-output #f])
  (%define-command! name doc proc repeatable inline-output))

; %dispatch-command — routes by command type:
;   activated plugin command → apply closure inline (stays in Steel, synchronous);
;   lazy activation miss      → activate inline, retry, then fall through to native;
;   native / unknown          → %call-native! (Rust leaf, sync for native).
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

; register-lsp-server! — queues an LSP server for spawn-on-first-open.
; Init-only (like define-language!). init-options/settings are any Steel
; data (typically a hash), decoded to JSON at the boundary.
(define (register-lsp-server! language #:command command
                                        #:args [args '()]
                                        #:root-markers [root-markers '()]
                                        #:init-options [init-options #f]
                                        #:settings [settings #f])
  (%register-lsp-server! language command args root-markers init-options settings))

; lsp-request — generic LSP bridge. server: a registered language name, or
; #f for "the focused buffer's attached server". callback: (lambda (err
; result) …) — exactly one of err/result is non-#f. #:allow-stale skips the
; drop-if-buffer-moved-on staleness check.
(define (lsp-request server method params callback #:allow-stale [allow-stale #f])
  (%lsp-request server method params callback allow-stale))

; debounce — trailing-edge: each call (re)schedules `proc` `ms` in the
; future, cancelling whichever call from a previous invocation is still
; pending. Pure Scheme over (after)/(cancel-timer!) — no Rust debouncer.
(define (debounce ms proc)
  (let ((pending (box #f)))
    (lambda args
      (let ((prev (unbox pending)))
        (when prev (cancel-timer! prev)))
      (set-box! pending (after ms (lambda () (apply proc args)))))))

; diagnostics-for-buffer — bounded, filtered pull over the diagnostics
; store. #:severity: a floor symbol ('error 'warning 'info 'hint), or #f for
; no floor (everything). #:range: a (list start end) char-offset bound, or
; #f for the whole buffer.
(define (diagnostics-for-buffer bid #:severity [severity #f] #:range [range #f])
  (%diagnostics-for-buffer bid severity range))

; apply-text-edits! — one undoable transaction. edits: list of ((start-line
; start-col) (end-line end-col) text), wire positions. #:expect-generation:
; the text_gen the edits were computed against (B2's staleness tag) — #f
; skips the check.
(define (apply-text-edits! bid edits #:expect-generation [gen #f])
  (%apply-text-edits! bid edits gen))

; apply-workspace-edit! — multi-file engine; reports the modified-buffer
; count so the user can see the effect of e.g. a rename before saving.
(define (apply-workspace-edit! wsedit)
  (let ((n (%apply-workspace-edit! wsedit)))
    (log! 'info (to-string n " buffers modified — :wa writes all"))
    n))

; prompt! — one-shot minibuffer prompt (B9). on-confirm fires exactly once:
; the confirmed text, or #f on Esc/cancel. No history, no completion — a
; second prompt! while one is already open errors rather than stacking.
(define (prompt! label on-confirm #:prefill [prefill ""])
  (%prompt! label prefill on-confirm))

; completion-begin! — starts a new completion session (replacing any open
; one). items: list of decoded CompletionItem hashmaps. #:incomplete: the
; server's isIncomplete flag, #f if the response was exhaustive.
(define (completion-begin! bid items #:incomplete [incomplete #f])
  (%completion-begin! bid items incomplete))

; show-popup! — cursor-anchored floating text panel (U4). #:anchor is
; reserved for future anchor kinds; 'cursor is the only value v1 accepts.
; close-popup! needs no wrapper — no keyword defaults to supply.
(define (show-popup! text #:anchor [anchor 'cursor])
  (%show-popup! text anchor))

; Variadic call! macro — desugars to %dispatch-command.
; Defined here (not only in prelude.scm) so it is available in every Steel engine
; context, including test harnesses that do not load the full prelude.
(define-syntax call!
  (syntax-rules ()
    ((_ name args ...)
     (%dispatch-command name (list args ...)))))

; displayln — shadows steel-core's kernel.scm binding (raw, ungated stdout
; print) with a version gated on SteelCtx::is_inline_output (see io.rs): a
; no-op unless the alt-screen TUI is guaranteed not to own the terminal.
(define (displayln . args) (%displayln! args))
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

    // Shell — narrow git/curl wrappers only (no generic run-process)
    steel.register_fn_with_ctx(HUME_CTX, "git-clone", shell::git_clone);
    steel.register_fn_with_ctx(HUME_CTX, "git-pull", shell::git_pull);
    steel.register_fn_with_ctx(HUME_CTX, "git-clone-rev", shell::git_clone_rev);
    steel.register_fn_with_ctx(HUME_CTX, "curl-fetch", shell::curl_fetch);

    // Grammar compilation
    steel.register_fn_with_ctx(
        HUME_CTX,
        "grammar-output-path",
        grammar::grammar_output_path,
    );
    steel.register_fn_with_ctx(HUME_CTX, "compile-grammar!", grammar::compile_grammar);

    // Logging — push messages to the editor message log
    steel.register_fn_with_ctx(HUME_CTX, "log!", crate::log::log_msg);

    // %displayln! is the Rust leaf behind the BOOTSTRAP `displayln` shim below,
    // which shadows steel-core's kernel.scm `displayln` (raw, ungated stdout
    // print) with a version gated on SteelCtx::is_inline_output — see io.rs.
    steel.register_fn_with_ctx(HUME_CTX, "%displayln!", io::displayln);

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

    // LSP server registration — init-only, queued like language regs.
    steel.register_fn_with_ctx(HUME_CTX, "%register-lsp-server!", lsp::register_lsp_server);
    // Generic LSP bridge — any protocol method reachable from Steel.
    steel.register_fn_with_ctx(HUME_CTX, "%lsp-request", lsp::lsp_request);
    steel.register_fn_with_ctx(HUME_CTX, "lsp-notify", lsp::lsp_notify);
    steel.register_fn_with_ctx(HUME_CTX, "on-lsp-notification", lsp::on_lsp_notification);
    // B3 — introspection
    steel.register_fn_with_ctx(HUME_CTX, "lsp-capabilities", lsp::lsp_capabilities);
    steel.register_fn_with_ctx(HUME_CTX, "lsp-server-status", lsp::lsp_server_status);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "lsp-server-for-buffer",
        lsp::lsp_server_for_buffer,
    );
    steel.register_fn_with_ctx(HUME_CTX, "lsp-position-params", lsp::lsp_position_params);
    steel.register_fn_with_ctx(HUME_CTX, "lsp-range-params", lsp::lsp_range_params);
    steel.register_fn_with_ctx(HUME_CTX, "buffer-generation", buffers::buffer_generation);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "register-trigger-chars!",
        lsp::register_trigger_chars,
    );

    // B5 — decoration stores + diagnostics pull.
    steel.register_fn_with_ctx(HUME_CTX, "set-inlay-hints!", lsp::set_inlay_hints);
    steel.register_fn_with_ctx(HUME_CTX, "set-signs!", lsp::set_signs);
    steel.register_fn_with_ctx(HUME_CTX, "set-virtual-lines!", lsp::set_virtual_lines);
    steel.register_fn_with_ctx(HUME_CTX, "set-extra-highlights!", lsp::set_extra_highlights);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "%diagnostics-for-buffer",
        lsp::diagnostics_for_buffer,
    );
    steel.register_fn_with_ctx(HUME_CTX, "diagnostic-counts", lsp::diagnostic_counts);

    // B6 — edit + navigation primitives.
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

    // B9 — minibuffer prompt.
    steel.register_fn_with_ctx(HUME_CTX, "%prompt!", lsp::prompt);
    steel.register_fn_with_ctx(HUME_CTX, "symbol-under-cursor", lsp::symbol_under_cursor);

    // B8 — completion orchestration.
    steel.register_fn_with_ctx(HUME_CTX, "%completion-begin!", lsp::completion_begin);
    steel.register_fn_with_ctx(
        HUME_CTX,
        "completion-update-filter!",
        lsp::completion_update_filter,
    );
    steel.register_fn_with_ctx(HUME_CTX, "completion-top", lsp::completion_top);
    steel.register_fn_with_ctx(HUME_CTX, "completion-accept!", lsp::completion_accept);
    steel.register_fn_with_ctx(HUME_CTX, "completion-dismiss!", lsp::completion_dismiss);

    // U4 — cursor-anchored popup widget.
    steel.register_fn_with_ctx(HUME_CTX, "%show-popup!", ui::show_popup);
    steel.register_fn_with_ctx(HUME_CTX, "close-popup!", ui::close_popup);

    // Timers — not LSP-specific, but B4 was scoped as part of the LSP step.
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

    // Context-free builtins: sandboxed filesystem ops that read from SCRIPT_DIRS TLS.
    steel.register_value("data-dir", SteelVal::FuncV(fs::data_dir));
    steel.register_value("runtime-dir", SteelVal::FuncV(fs::runtime_dir));
    steel.register_value("path-join", SteelVal::FuncV(fs::path_join));
    steel.register_value("path-exists?", SteelVal::FuncV(fs::path_exists));
    steel.register_value("list-dir", SteelVal::FuncV(fs::list_dir));
    steel.register_value("read-file", SteelVal::FuncV(fs::read_file));
    steel.register_value("make-dir", SteelVal::FuncV(fs::make_dir));
    steel.register_value("delete-dir", SteelVal::FuncV(fs::delete_dir));
    steel.register_value("delete-file", SteelVal::FuncV(fs::delete_file));
    steel.register_value("write-file", SteelVal::FuncV(fs::write_file));

    // Evaluate the Scheme bootstrap (defines `load-plugin`).
    // Runs before any user init.scm; HUME_CTX is not yet set but the
    // bootstrap only uses `define`, so no builtins are called at this point.
    steel
        .compile_and_run_raw_program(BOOTSTRAP.to_owned())
        .expect("HUME scripting bootstrap failed — this is a bug");
}
