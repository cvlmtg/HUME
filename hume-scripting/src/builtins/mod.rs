//! Steel builtins for HUME's scripting layer.
//!
//! [`register_all`] registers every builtin on the Steel engine and then evaluates
//! the Scheme bootstrap that defines `load-plugin` and `declare-plugin`.
//! This must be called once during [`ScriptingHost::new`] before any
//! `eval_init` call.

pub(crate) mod args;
pub(crate) mod buffers;
pub(crate) mod commands;
pub(crate) mod completion;
pub(crate) mod decorations;
pub(crate) mod dirs;
pub(crate) mod edits;
pub(crate) mod errors;
pub(crate) mod fs;
pub(crate) mod grammar;
pub(crate) mod hooks;
pub(crate) mod ids;
pub(crate) mod install;
pub(crate) mod interrupt;
pub(crate) mod io;
pub(crate) mod json;
pub(crate) mod keymap_bind;
pub(crate) mod lsp;
pub(crate) mod plugins;
pub(crate) mod settings;
pub(crate) mod statusline;
pub(crate) mod syntax;
pub(crate) mod timers;
pub(crate) mod ui;

use std::borrow::Cow;

use steel::rvals::SteelVal;
use steel::steel_vm::engine::Engine;
use steel::steel_vm::register_fn::RegisterFn;

use super::HUME_CTX;

// ── Declarative registration table ───────────────────────────────────────────

/// Declarative builtin-registration table. Each entry is
/// `<kind> "<steel-name>" <rust-path>(<arg>: <Type>, …);` where `<kind>` is:
/// - `cmd`    — ctx-taking, gated by [`errors::require_cmd`] (buffer/pane/editor-state builtins)
/// - `config` — ctx-taking, gated by [`errors::require_config`] (init/plugin-load-only verbs)
/// - `open`   — ctx-taking, ungated (no legality gate, or a bespoke one the fn checks itself)
/// - `plain`  — no `&mut SteelCtx` param at all (context-free predicates)
///
/// The declared arg types are load-bearing, not documentation: each entry
/// expands to a closure with exactly that parameter list, so a mismatch
/// against the real function's signature is a compile error — a
/// compile-time link between a builtin's registered name and its gate.
macro_rules! builtins {
    (@one cmd, $steel:expr, $name:literal, $($modpath:ident)::+, ($($arg:ident : $ty:ty),*)) => {
        $steel.register_fn_with_ctx(
            HUME_CTX,
            $name,
            move |ctx: &mut crate::context::SteelCtx $(, $arg: $ty)*| {
                crate::builtins::errors::require_cmd(ctx, $name)?;
                $($modpath)::+(ctx $(, $arg)*)
            },
        );
    };
    (@one config, $steel:expr, $name:literal, $($modpath:ident)::+, ($($arg:ident : $ty:ty),*)) => {
        $steel.register_fn_with_ctx(
            HUME_CTX,
            $name,
            move |ctx: &mut crate::context::SteelCtx $(, $arg: $ty)*| {
                crate::builtins::errors::require_config(ctx, $name)?;
                $($modpath)::+(ctx $(, $arg)*)
            },
        );
    };
    (@one open, $steel:expr, $name:literal, $($modpath:ident)::+, ($($arg:ident : $ty:ty),*)) => {
        $steel.register_fn_with_ctx(
            HUME_CTX,
            $name,
            move |ctx: &mut crate::context::SteelCtx $(, $arg: $ty)*| {
                $($modpath)::+(ctx $(, $arg)*)
            },
        );
    };
    (@one plain, $steel:expr, $name:literal, $($modpath:ident)::+, ($($arg:ident : $ty:ty),*)) => {
        $steel.register_fn(
            $name,
            move |$($arg: $ty),*| {
                $($modpath)::+($($arg),*)
            },
        );
    };
    ($steel:expr, $($kind:ident $name:literal $($modpath:ident)::+ ( $($arg:ident : $ty:ty),* $(,)? ) ;)*) => {
        $(
            builtins!(@one $kind, $steel, $name, $($modpath)::+, ($($arg : $ty),*));
        )*
    };
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
// Known limitation, left as-is: a caller's with-handler around a
// zero-trigger call can hit steel-core 0.8.2's "no open continuation" panic
// if manifest.scm raises (same footgun as %activate-plugin-inline; see
// known_limitation_reraise_via_raise_error_inside_outer_tolerant_handler_corrupts_vm_stack
// in lib.rs). `error` panics identically; `dynamic-wind` avoids the panic but
// its cleanup thunk skips across an outer handler's unwind (see
// known_limitation_dynamic_wind_cleanup_does_not_run_across_an_outer_handlers_unwind),
// leaving manifest_resolving stuck for the rest of the session. Swallowing
// the error would break declare-plugin's tested propagate-to-caller contract
// (4 tests in builtins/plugins/tests.rs, via .expect_err). Pending an
// upstream steel-core fix.
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
// ('error 'warning 'info 'hint) or #f for none. #:range: (start . end)
// dotted-pair char-offset bound, or #f for the whole buffer.
//
// apply-text-edits! — one undoable transaction. edits: list of
// ((start-line . start-col) (end-line . end-col) text), wire positions as
// dotted pairs. #:expect-generation: staleness tag to check against; #f
// skips it.
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
// show-popup! — text panel, floating or docked. #:anchor: 'cursor (default)
// floats near the focused pane's cursor; 'bottom docks as a full-width band
// above the statusline, reserving pane space like the drawer (used for
// hover content too tall for the cursor layout). #:dismiss-on-key: the popup
// is cleared by the Editor::handle_key top-of-loop check on the *next* key
// press, whatever key it is (see `gn`/`gp`'s diagnostic overlay) — default
// #f keeps the existing on-mode-change-only dismissal (hover, signature
// help). #:scroll: Ctrl+u/Ctrl+d scroll the popup's content instead of the
// buffer, and every other key closes it — mutually exclusive with
// #:dismiss-on-key. #:lang: syntax-highlight the content like a real buffer
// when a grammar named #:lang is registered; plain text otherwise (default
// #f is always plain).
//
// show-drawer-list! — pick-list only, no #:lang: rows are plain display
// strings, never syntax-highlighted (that's the popup's job).
//
// picker! — fuzzy-finder panel (docs/FUZZY-FINDERS.md). items: list of
// (display . payload) dotted pairs; payload is opaque, handed back to
// on-select verbatim. Returns a token scoping later picker-push! calls to
// this session. Allowed from any mode, but closes any open completion
// session first (one modal owner at a time). on-select fires exactly once:
// the selected payload on Enter, or #f on Esc, picker-close!, or being
// replaced by a second picker! call.
//
// picker-push! — appends items to the open picker, gated by the token
// open_picker returned. A stale token (picker closed/replaced since) or no
// open picker is a silent #f, not an error — expected-normal for an async
// source racing the user. Returns whether the push was applied.
//
// picker-source-spawn! — attaches a streaming external-command source
// (docs/FUZZY-FINDERS.md B5) to the picker session `token` scopes: direct
// argv spawn, no shell, stdin closed immediately. Stdout lines flow straight
// into the store, Rust-side — Steel never sees the bulk output, only
// whichever single line the user accepts. A stale token or no open picker
// is a silent #f (same expected-normal-race contract as picker-push!),
// never an error; a genuine spawn failure (missing binary, bad #:cwd)
// raises. The child is owned by the picker session: closing or replacing
// the picker (including a second picker-source-spawn! on the same session)
// kills it automatically. #:nul splits on NUL instead of newline (for
// `git ls-files -z`/`fd -0`). No shell means no `cmd | other` pipelines and
// no Windows quoting hazards, at the cost of the caller building its own
// argv.
//
// picker-close! — ends the open picker, firing on-select with #f. Unlike
// close-menu!/close-drawer! (which drop the callback), this always invokes
// it — the picker's exactly-once lifecycle has no "silently dropped"
// state. Idempotent when no picker is open.
//
// Variadic call! macro — desugars to %dispatch-command, the in-VM dispatcher
// for calls originating inside Steel (call! from a plugin body, or the bare
// command-name lambdas register_command_names defines). Keypress/`:`-line
// dispatch skips this path: Editor::call_steel_cmd resolves and activates
// the target directly, applying its command_table closure via
// call_function_with_args. %dispatch-command's own miss→activate→retry
// exists only because a builtin can't re-enter the editor's Rust dispatcher
// while the Engine is already borrowed. Defined here (not only prelude.scm)
// so test harnesses without the full prelude still have it.
//
// Gated print — capture steel-core's original display*/print* fns and the
// real stdout port ONCE, before PRINT_GATE_SHIMS redefines the names and
// before set_prelude_string runs (register_all) — guaranteeing these are the
// originals.
//
// %port-safe? — writing to `port` is TUI-safe unless it IS the real stdout
// port, in which case defer to the gate. Shared by every shim's
// explicit-port branch.
const BOOTSTRAP: &str = include_str!("bootstrap.scm");

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
const PRINT_GATE_SHIMS: &str = include_str!("print_gate_shims.scm");

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

    builtins! { steel,
        // Config / settings
        config "set-option!" settings::set_option(key: String, value: SteelVal);
        cmd    "get-option" settings::get_option(key: String);
        config "configure-statusline!" statusline::configure_statusline(left: SteelVal, center: SteelVal, right: SteelVal);

        // Step budget
        open "hume/yield!" interrupt::hume_yield();

        // Keymap
        config "bind-key!" keymap_bind::bind_key(mode: SteelVal, key_str: String, cmd_name: String);
        config "bind-key-extend!" keymap_bind::bind_key_extend(mode: SteelVal, key_str: String, cmd_name: String);
        config "unbind-key!" keymap_bind::unbind_key(mode: SteelVal, key_str: String);
        config "bind-wait-char!" keymap_bind::bind_wait_char(mode: SteelVal, key_str: String, cmd_name: String);
        cmd    "set-register-prefix!" commands::set_register_prefix(name: String);

        // Plugin lifecycle
        open "%declare-plugin!" plugins::declare_plugin(name: String, commands: SteelVal, events: SteelVal, languages: SteelVal, config: SteelVal);
        open "resolve-plugin-path" plugins::resolve_plugin_path(name: String);

        // Plugin introspection and explicit activation
        open "loaded-plugins" plugins::loaded_plugins();
        open "declared-plugins" plugins::declared_plugins();
        open "plugin-config" plugins::plugin_config();
        open "%load-plugin!" plugins::load_plugin(name: String, config: SteelVal);

        // Inline activation primitives — called from the %activate-plugin-inline
        // Scheme helper to drive mid-eval plugin loading without &mut Engine.
        open "%begin-lazy-activation" plugins::begin_lazy_activation(id_str: String);
        open "%finish-lazy-activation" plugins::finish_lazy_activation(id_str: String, success: bool);
        open "%lazy-command-owner" plugins::lazy_command_owner(name: String);

        // Manifest resolution — zero-trigger declare-plugin routes here to eval
        // <plugin-dir>/manifest.scm so the plugin can declare its own defaults.
        open "%begin-manifest-declare!" plugins::begin_manifest_declare(name: String, config: SteelVal);
        open "%finish-manifest-declare!" plugins::finish_manifest_declare(name: String, success: bool);

        // Hook registration — init-only
        config "register-hook!" hooks::register_hook(name: SteelVal, proc: SteelVal);

        // Steel command definition. %define-command! is the native primitive;
        // the (define-command! …) Steel wrapper in BOOTSTRAP exposes keyword
        // args (#:repeatable, #:inline-output).
        config "%define-command!" commands::define_command(name: String, doc: String, proc: SteelVal, repeatable: bool, inline_output: bool);
        // %call-native! is the Rust leaf for native/unknown dispatch; the variadic
        // (call! name args…) macro desugars to (%dispatch-command …) which routes
        // activated plugin commands inline in Steel and falls back here for everything else.
        open "%call-native!" commands::call_command_primitive(name: String, args: SteelVal);
        // %lookup-plugin-proc: returns the Steel closure for an activated plugin command,
        // or #f. Called by %dispatch-command in Steel to decide inline-apply vs. %call-native!.
        open "%lookup-plugin-proc" commands::lookup_plugin_proc(name: String);
        cmd  "request-wait-char!" commands::request_wait_char(cmd: String);
        open "pending-char" commands::pending_char();
        open "command-plugin" commands::command_plugin(name: String);

        // Grammar compilation — sandbox-free, full-trust plugin model. Kept as a
        // Rust builtin only for the Windows compiler-selection dance (see
        // grammar.rs's module doc); `grammar-output-path` moved to Scheme.
        open "compile-grammar!" grammar::compile_grammar(src: String, out: String);

        // LSP server install pipeline — sha256 hashing, archive unpacking,
        // platform id, cross-process install lock (see docs/LSP-INSTALL.md).
        // Sandbox-free — full-trust plugin model. `verify-sha256!`/`exe-on-path?`
        // /`git-clone`/`curl-fetch`/`npm-install!` moved to Scheme + Steel's own
        // `steel/process` stdlib (`which`, `spawn-process`).
        open  "sha256-file" install::sha256_file(path: String);
        open  "unpack-gz" install::unpack_gz(src: String, dest: String);
        open  "unpack-zip" install::unpack_zip(src: String, dest_dir: String, bin_path: String);
        open  "acquire-install-lock!" install::acquire_install_lock();
        open  "release-install-lock!" install::release_install_lock();
        open  "%run-inline-output!" install::run_inline_output(cmd: String, args_val: SteelVal, cwd_val: SteelVal);

        // Logging — push messages to the editor message log
        open "log!" crate::log::log_msg(severity: SteelVal, message: String);

        // %stdout-gate! is the Rust leaf behind the gated print shims (displayln,
        // display, print, println, newline) — see io.rs and PRINT_GATE_SHIMS above.
        open "%stdout-gate!" io::stdout_gate();

        // Opaque ID predicates and equality — context-free; no SteelCtx needed.
        plain "buffer-id?" ids::is_buffer_id(val: SteelVal);
        plain "pane-id?" ids::is_pane_id(val: SteelVal);
        plain "buffer-id=?" ids::buffer_id_equal(a: SteelVal, b: SteelVal);
        plain "json-parse" json::json_parse(s: SteelVal);
        plain "pane-id=?" ids::pane_id_equal(a: SteelVal, b: SteelVal);

        // Multi-buffer read-only builtins
        cmd "current-buffer" buffers::current_buffer();
        cmd "current-pane" buffers::current_pane();
        cmd "buffers" buffers::buffers();
        cmd "panes" buffers::panes();
        cmd "buffer-path" buffers::buffer_path(bid: args::BidArg);
        cmd "buffer-name" buffers::buffer_name(bid: args::BidArg);
        cmd "buffer-dirty?" buffers::buffer_dirty(bid: args::BidArg);
        // Live cursor read — reflects synchronous edits in the same eval.
        cmd "current-line-number" buffers::current_line_number();
        cmd "current-selections" buffers::current_selections();
        cmd "char-index->line" buffers::char_index_to_line(idx: SteelVal);

        // Multi-buffer mutating builtins
        cmd "open-buffer!" buffers::open_buffer(path: String);
        cmd "close-buffer!" buffers::close_buffer(bid: args::BidArg);
        cmd "switch-to-buffer!" buffers::switch_to_buffer(bid: args::BidArg);

        // Language identity and grammar builtins
        config "%define-language!" syntax::define_language(name: SteelVal, exts_val: SteelVal, globs_val: SteelVal, shebangs_val: SteelVal);
        open   "%register-grammar!" syntax::register_grammar(name: SteelVal, grammar_path: SteelVal, symbol: SteelVal, highlights_path: SteelVal, injections_path: SteelVal);

        // LSP server registration — last-wins, queued (like language regs) and
        // applied at the end of the current eval, from init, plugin activation,
        // or a command/hook body.
        open "%register-lsp-server!" lsp::register_lsp_server(language: SteelVal, command: SteelVal, args_val: SteelVal, root_markers_val: SteelVal, init_options: SteelVal, settings: SteelVal);
        open "unregister-lsp-server!" lsp::unregister_lsp_server(language: SteelVal);
        // Lifecycle — stop/restart a running server, or open the status view.
        cmd "lsp-stop!" lsp::lsp_stop(language: SteelVal);
        cmd "lsp-restart!" lsp::lsp_restart(language: SteelVal);
        cmd "lsp-show-status!" lsp::lsp_show_status();
        // Generic LSP bridge — any protocol method reachable from Steel.
        cmd "%lsp-request" lsp::lsp_request(server: SteelVal, method: SteelVal, params: SteelVal, callback: SteelVal, allow_stale: SteelVal, supersede: SteelVal);
        cmd "lsp-notify" lsp::lsp_notify(server: SteelVal, method: SteelVal, params: SteelVal);
        config "on-lsp-notification" lsp::on_lsp_notification(method: SteelVal, handler: SteelVal);
        // Introspection
        cmd  "lsp-capabilities" lsp::lsp_capabilities(server: SteelVal);
        cmd  "lsp-server-status" lsp::lsp_server_status();
        cmd  "lsp-server-for-buffer" lsp::lsp_server_for_buffer(bid: args::BidArg);
        open "lsp-registered-for-language?" lsp::lsp_registered_for_language(language: SteelVal);
        cmd "lsp-position-params" lsp::lsp_position_params(bid: args::BidArg);
        cmd "lsp-range-params" lsp::lsp_range_params(bid: args::BidArg);
        cmd "viewport-range" buffers::viewport_range(bid: args::BidArg);
        cmd "buffer-generation" buffers::buffer_generation(bid: args::BidArg);
        open "register-trigger-chars!" completion::register_trigger_chars(source: SteelVal, language: SteelVal, chars: SteelVal);

        // Decoration stores + diagnostics pull.
        cmd "set-inlay-hints!" decorations::set_inlay_hints(bid: args::BidArg, hints: SteelVal);
        cmd "set-signs!" decorations::set_signs(source: SteelVal, bid: args::BidArg, signs: SteelVal);
        cmd "set-virtual-lines!" decorations::set_virtual_lines(source: SteelVal, bid: args::BidArg, lines: SteelVal);
        cmd "set-inline-diagnostics!" decorations::set_inline_diagnostics(bid: args::BidArg, lines: SteelVal);
        cmd "set-extra-highlights!" decorations::set_extra_highlights(source: SteelVal, bid: args::BidArg, spans: SteelVal);
        cmd "%diagnostics-for-buffer" decorations::diagnostics_for_buffer(bid: args::BidArg, severity: SteelVal, range: SteelVal);
        cmd "diagnostic-counts" decorations::diagnostic_counts(bid: args::BidArg);

        // Edit + navigation primitives.
        cmd "%apply-text-edits!" edits::apply_text_edits(bid: args::BidArg, edits: SteelVal, expect_gen: SteelVal);
        cmd "%apply-workspace-edit!" edits::apply_workspace_edit(wsedit: SteelVal);
        cmd "goto-location!" edits::goto_location(loc: SteelVal);
        cmd "selection-spans-full-line?" buffers::selection_spans_full_line(bid: args::BidArg);

        // Minibuffer prompt.
        cmd "%prompt!" ui::prompt(label: SteelVal, prefill: SteelVal, on_confirm: SteelVal);
        cmd "symbol-under-cursor" buffers::symbol_under_cursor(bid: args::BidArg);

        // Completion orchestration.
        cmd "%completion-begin!" completion::completion_begin(bid: args::BidArg, items: SteelVal, incomplete: SteelVal);
        cmd "completion-update-filter!" completion::completion_update_filter(text: SteelVal);
        cmd "completion-top" completion::completion_top(n: SteelVal);
        cmd "completion-accept!" completion::completion_accept(idx: SteelVal);
        cmd "completion-dismiss!" completion::completion_dismiss();

        // Cursor-anchored popup widget.
        cmd "%show-popup!" ui::show_popup(text: SteelVal, anchor: SteelVal, dismiss_on_key: SteelVal, scrollable: SteelVal, lang: SteelVal);
        cmd "close-popup!" ui::close_popup();

        // Selection menu widget.
        cmd "show-menu!" ui::show_menu(items: SteelVal, on_select: SteelVal);
        cmd "close-menu!" ui::close_menu();

        // Class B bottom drawer.
        cmd "show-drawer-list!" ui::show_drawer_list(items: SteelVal, on_select: SteelVal);
        cmd "close-drawer!" ui::close_drawer();

        // Fuzzy-picker widget (docs/FUZZY-FINDERS.md B4).
        cmd "%picker!" ui::picker(items: SteelVal, on_select: SteelVal, prompt: SteelVal);
        cmd "picker-push!" ui::picker_push(token: SteelVal, items: SteelVal);
        cmd "%picker-source-spawn!" ui::picker_source_spawn(token: SteelVal, cmd: SteelVal, args: SteelVal, cwd: SteelVal, nul: SteelVal);
        cmd "picker-close!" ui::picker_close();

        // Timers — not LSP-specific, but added as part of the LSP work.
        cmd "after" timers::after(ms: SteelVal, thunk: SteelVal);
        cmd "cancel-timer!" timers::cancel_timer(id: SteelVal);
        open "language-has-grammar?" syntax::language_has_grammar(name: SteelVal);
        cmd "buffer-language" buffers::buffer_language(bid: args::BidArg);
        cmd "set-buffer-language!" buffers::set_buffer_language_steel(bid: args::BidArg, lang: SteelVal);

        // Editor-integration directory info, read from `ctx.dirs` (computed
        // once by `ScriptingHost::new`). Callable from anywhere (`open`) —
        // same reach as the context-free builtins these replaced.
        open "data-dir" fs::data_dir();
        open "runtime-dir" fs::runtime_dir();
    }

    // Context-free builtins that don't fit the typed-arity table above: raw
    // `&[SteelVal]` FuncV, no SteelCtx. `path-join` is a pure string helper;
    // `hume-target` reads platform info, not directory state.
    steel.register_value("hume-target", SteelVal::FuncV(install::hume_target));
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

#[cfg(test)]
mod tests;
