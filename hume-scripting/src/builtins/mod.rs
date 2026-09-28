//! Steel builtins for HUME's scripting layer.
//!
//! [`register_all`] registers every builtin on the Steel engine and then evaluates
//! the Scheme bootstrap that defines `load-plugin!` and `declare-plugin!`.
//! This must be called once during [`crate::ScriptingHost::new`] before any
//! `eval_init` call.

pub(crate) mod args;
pub(crate) mod buffers;
pub(crate) mod commands;
pub(crate) mod completion;
pub(crate) mod decorations;
pub(crate) mod diff;
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
pub(crate) mod process;
pub(crate) mod registers;
pub(crate) mod settings;
pub(crate) mod statusline;
pub(crate) mod syntax;
pub(crate) mod timers;
pub(crate) mod ui;
pub(crate) mod words;

use std::borrow::Cow;

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;
use steel::steel_vm::engine::Engine;
use steel::steel_vm::register_fn::RegisterFn;

use super::HUME_CTX;

/// The return type of every ctx-taking builtin below. One definition shared
/// by every `builtins/*.rs` submodule instead of each declaring its own
/// identical local alias.
pub(crate) type SteelResult = Result<SteelVal, SteelErr>;

// ── Declarative registration table ───────────────────────────────────────────

/// Declarative builtin-registration table. Each entry is
/// `<kind> "<steel-name>" <rust-path>(<arg>: <Type>, …);` where `<kind>` is:
/// - `cmd`: ctx-taking, gated by [`errors::require_cmd`] (buffer/pane/editor-state builtins)
/// - `config`: ctx-taking, gated by [`errors::require_config`] (init/plugin-load-only verbs)
/// - `open`: ctx-taking, ungated (no legality gate, or a bespoke one the fn checks itself)
/// - `plain`: no `&mut SteelCtx` param at all (context-free predicates)
///
/// The declared arg types are load-bearing, not documentation: each entry
/// expands to a closure with exactly that parameter list, so a mismatch
/// against the real function's signature is a compile error, a
/// compile-time link between a builtin's registered name and its gate.
///
/// Every ctx-taking kind (`cmd`/`config`/`open`) runs each decoded argument
/// through `args::BuiltinArg::resolve` right after the gate, shadowing
/// `$arg` with its `Out` type: the seam `args::LivePane` (raises on a
/// closed buffer, resolving to a plain `PaneHandle`) hooks into; every
/// other declared type resolves to itself. `plain` (no ctx) never runs this
/// step, so it can't declare a `LivePane` argument.
macro_rules! builtins {
    (@one cmd, $steel:expr, $name:literal, $($modpath:ident)::+, ($($arg:ident : $ty:ty),*)) => {
        $steel.register_fn_with_ctx(
            HUME_CTX,
            $name,
            move |ctx: &mut crate::context::SteelCtx $(, $arg: $ty)*| {
                crate::builtins::errors::require_cmd(ctx, $name)?;
                $(let $arg = crate::builtins::args::BuiltinArg::resolve($arg, ctx, $name)?;)*
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
                $(let $arg = crate::builtins::args::BuiltinArg::resolve($arg, ctx, $name)?;)*
                $($modpath)::+(ctx $(, $arg)*)
            },
        );
    };
    (@one open, $steel:expr, $name:literal, $($modpath:ident)::+, ($($arg:ident : $ty:ty),*)) => {
        $steel.register_fn_with_ctx(
            HUME_CTX,
            $name,
            move |ctx: &mut crate::context::SteelCtx $(, $arg: $ty)*| {
                $(let $arg = crate::builtins::args::BuiltinArg::resolve($arg, ctx, $name)?;)*
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

/// Scheme bootstrap evaluated once during Steel engine init: keyword wrappers
/// over the Rust builtins, plugin activation, and in-VM command dispatch.
///
/// Hazard for every `with-handler` here: re-raising a caught native error from
/// a handler nested inside an outer `with-handler` corrupts Steel's VM stack
/// (`known_limitation_reraise_via_raise_error_inside_outer_tolerant_handler_corrupts_vm_stack`
/// in `tests/unix.rs`).
//
// %activate-plugin-inline: %begin-lazy-activation moves a Declared plugin to
// Loading and returns its `(require "<abs>")` string (#f otherwise: cycle
// guard and idempotency), eval-string runs it in the live VM, and
// %finish-lazy-activation records the outcome. The handler hands the error to
// %finish-lazy-activation and returns normally, so a failing plugin never
// aborts the enclosing eval and nothing is re-raised. The trailing
// (hume/yield!) then re-checks the interrupt flag with a fresh raise: an
// exhausted step budget must abort the whole eval, not be recorded as every
// later plugin failing to load.
//
// declare-plugin!: with no triggers, evaluates <plugin-dir>/manifest.scm for
// default entries (caller's #:config wins) under the same handler/yield
// contract, %finish-manifest-declare! recording any error.
//
// define-typed-command!: the `:` counterpart of define-command!, sharing its
// collision guard (commands::check_definable) and bookkeeping but registering
// a TypedBody::Steel entry. A name defined one way is never reachable the
// other way. No #:repeatable: dot-repeat means nothing for a `:` command.
//
// call! / %dispatch-command: the dispatcher for calls from inside Steel (call!
// and the bare command-name lambdas); keypress and `:` dispatch use
// ScriptingHost::call_steel_cmd instead. The miss, activate, retry path lives
// in Scheme because a builtin can't re-enter the Rust dispatcher while the
// Engine is borrowed. A retry miss raises rather than falling through to
// %call-native!, which would misreport the name as unknown. call! is defined
// here, not only in prelude.scm, so harnesses without the prelude have it.
//
// %apply-command: arms the #:inline-output alt-screen bracket for call!, as
// Editor::call_steel_command_body does for keypress and `:`. The restore
// truncates to the armed depth instead of popping, so an unpaired descendant
// frame is never taken for this call's own. No with-handler (hazard above): a
// raising body skips the restore; run_steel_session truncates to zero at end.
//
// lsp-request!: callback is (lambda (err result)), exactly one non-#f.
// #:supersede <key> cancels the caller's own pending request under the same
// (server, key). #:require-focus drops the callback unless pane is still
// focused when the response arrives.
//
// debounce / debounce-by: trailing-edge; debounce-by keeps one timer per
// #:key (default: first argument). A firing timer clears its pending entry
// only if it still holds its own id (my-id): an already-queued timer can't be
// cancelled, and clearing unconditionally would orphan the next call's timer.
//
// register-completion-source!: #:match binds to `match-kind` because `match`
// is Steel's pattern-matching macro and `[match 'fuzzy]` would be expanded.
//
// picker! / live-picker!: two constructors over one PickerSession; live-picker!
// respawns #:command per query and disables local filtering. Each keystroke
// stops the running source and re-arms the debounced respawn. Old rows stay
// until the new source's first batch replaces them (no blank frame per
// keystroke); #:command returning #f clears them. Only the timer-dispatched
// respawn gets the clear-and-re-raise handler, since it has no outer handler;
// the seed spawn for a non-empty #:query runs on the caller's stack and must
// stay unwrapped.
//
// run-inline-output!: raises on a nonzero exit. %run-inline-output! spawns in
// an isolated process group (see hume-platform::process::run_inline_output).
//
// %port-safe?: a port is safe unless it is the real stdout, where the gate
// decides. Every print shim's explicit-port branch uses it (io.rs module doc).
const BOOTSTRAP: &str = include_str!("bootstrap.scm");

// PRINT_GATE_SHIMS is appended both to BOOTSTRAP (top level) and, verbatim,
// to steel-core's own prelude string via set_prelude_string. Since the
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
// `write-string`/`write-char` are shimmed for the same reason as `display`:
// their implicit-arg case still defaults straight to real stdout unless
// redirected, so they're exactly as unsafe and need the same gate.
// `simple-display`/`simple-displayln` always resolve `(current-output-port)`
// themselves, so they're gated the same way.
const PRINT_GATE_SHIMS: &str = include_str!("print_gate_shims.scm");

// ── Registration ──────────────────────────────────────────────────────────────

/// Register all HUME builtins on `steel` and evaluate the Scheme bootstrap.
///
/// Must be called exactly once during [`crate::ScriptingHost::new`], before any
/// `eval_init` calls.
pub(crate) fn register_all(steel: &mut Engine) {
    // Pre-register HUME_CTX so supply_context_arg can generate its wrapper
    // functions without a FreeIdentifier error.  The real SteelVal::Reference
    // is injected at eval / dispatch time via steel.update_value.
    steel.register_value(HUME_CTX, SteelVal::Void);

    // A name ends in `!` when calling it mutates editor state, registers
    // something, spawns a process, or writes to disk. Reads and closure
    // constructors (`debounce`) carry no `!`.
    builtins! { steel,
        // Config / settings
        open "set-option!" settings::set_option(key: String, value: SteelVal);
        cmd  "set-buffer-option!" settings::set_buffer_option(pane: args::LivePane, key: String, value: SteelVal);
        open "get-option" settings::get_option(key: String);
        cmd  "get-buffer-option" settings::get_buffer_option(pane: args::LivePane, key: String);
        open "configure-statusline!" statusline::configure_statusline(left: SteelVal, center: SteelVal, right: SteelVal);

        // Step budget
        open "hume/yield!" interrupt::hume_yield();

        // Keymap
        config "bind-key!" keymap_bind::bind_key(mode: SteelVal, key_str: String, cmd_name: String);
        config "bind-key-extend!" keymap_bind::bind_key_extend(mode: SteelVal, key_str: String, cmd_name: String);
        config "unbind-key!" keymap_bind::unbind_key(mode: SteelVal, key_str: String);
        config "bind-wait-char!" keymap_bind::bind_wait_char(mode: SteelVal, key_str: String, cmd_name: String);
        cmd    "set-register-prefix!" commands::set_register_prefix(name: String);

        // Registers: direct read/write, independent of set-register-prefix!'s
        // per-command targeting.
        open "read-register" registers::read_register(name: String);
        open "write-register!" registers::write_register(name: String, values: SteelVal);

        // Plugin lifecycle
        open "%declare-plugin!" plugins::declare_plugin(name: String, commands: SteelVal, typed_commands: SteelVal, events: SteelVal, languages: SteelVal, config: SteelVal);
        open "resolve-plugin-path" plugins::resolve_plugin_path(name: String);

        // Plugin introspection and explicit activation
        open "loaded-plugins" plugins::loaded_plugins();
        open "declared-plugins" plugins::declared_plugins();
        open "plugin-config" plugins::plugin_config();
        open "%load-plugin!" plugins::load_plugin(name: String, config: SteelVal);

        // Inline activation primitives, called from the %activate-plugin-inline
        // Scheme helper to drive mid-eval plugin loading without &mut Engine.
        open "%begin-lazy-activation" plugins::begin_lazy_activation(id_str: String);
        open "%finish-lazy-activation" plugins::finish_lazy_activation(id_str: String, error: SteelVal);
        open "%lazy-command-owner" plugins::lazy_command_owner(name: String);

        // Manifest resolution: zero-trigger declare-plugin! routes here to eval
        // <plugin-dir>/manifest.scm so the plugin can declare its own defaults.
        open "%begin-manifest-declare!" plugins::begin_manifest_declare(name: String, config: SteelVal);
        open "%finish-manifest-declare!" plugins::finish_manifest_declare(name: String, error: SteelVal);

        // Hook registration (init-only)
        config "register-hook!" hooks::register_hook(name: SteelVal, proc: SteelVal);

        // Steel command definition. %define-command! is the native primitive;
        // the (define-command! …) Steel wrapper in BOOTSTRAP exposes keyword
        // args (#:repeatable, #:inline-output).
        config "%define-command!" commands::define_command(name: String, doc: String, proc: SteelVal, repeatable: bool, inline_output: bool);
        // Typed (`:` command line) counterpart. See the module-doc paragraph
        // above BOOTSTRAP. No #:repeatable keyword arg.
        config "%define-typed-command!" commands::define_typed_command(name: String, doc: String, proc: SteelVal, inline_output: bool, completer: SteelVal);
        // %call-native! is the Rust leaf for native/unknown dispatch; the variadic
        // (call! name args…) macro desugars to (%dispatch-command …) which routes
        // activated plugin commands inline in Steel and falls back here for everything else.
        open "%call-native!" commands::call_command_primitive(name: String, args: SteelVal);
        // %lookup-plugin-proc: returns the Steel closure for an activated plugin command,
        // or #f. Called by %dispatch-command in Steel to decide inline-apply vs. %call-native!.
        open "%lookup-plugin-proc" commands::lookup_plugin_proc(name: String);
        // %arm-inline-output!/%restore-inline-output!: the call!-path counterpart
        // of dispatch.rs's own inline-output arm/close, wrapping %dispatch-command's
        // in-VM apply. See the BOOTSTRAP comment block above for the full picture.
        open "%arm-inline-output!" commands::arm_inline_output(name: String);
        open "%restore-inline-output!" commands::restore_inline_output(depth: SteelVal);
        cmd  "request-wait-char!" commands::request_wait_char(cmd: String);
        open "pending-char" commands::pending_char();
        open "command-plugin" commands::command_plugin(name: String);

        // Grammar compilation: sandbox-free, full-trust plugin model. Kept as a
        // Rust builtin only for the Windows compiler-selection dance (see
        // grammar.rs's module doc); `grammar-output-path` is plain Scheme.
        open "compile-grammar!" grammar::compile_grammar(src: String, out: String);

        // LSP server install pipeline: sha256 hashing, archive unpacking,
        // platform id, cross-process install lock.
        // Sandbox-free, full-trust plugin model. `verify-sha256!`/`exe-on-path?`
        // /`git-clone`/`curl-fetch`/`npm-install!` are plain Scheme, atop Steel's
        // own `steel/process` stdlib (`which`, `spawn-process`).
        open  "sha256-file" install::sha256_file(path: String);
        open  "unpack-gz!" install::unpack_gz(src: String, dest: String);
        open  "unpack-zip!" install::unpack_zip(src: String, dest_dir: String, bin_path: String);
        open  "acquire-install-lock!" install::acquire_install_lock();
        open  "release-install-lock!" install::release_install_lock();
        open  "%run-inline-output!" install::run_inline_output(cmd: String, args_val: SteelVal, cwd_val: SteelVal);

        // Logging: push messages to the editor message log
        open "log!" crate::log::log_msg(severity: SteelVal, message: String);

        // %stdout-gate! is the Rust leaf behind the gated print shims (displayln,
        // display, print, println, newline). See io.rs and PRINT_GATE_SHIMS above.
        open "%stdout-gate!" io::stdout_gate();

        // Opaque pane predicate: context-free, no SteelCtx needed. Equality
        // is `equal?` (Custom::equality_hint), not a dedicated builtin.
        plain "pane?" ids::is_pane(val: SteelVal);
        plain "json-parse" json::json_parse(s: SteelVal);
        // json-list and the two predicates are fixed-arity, registered
        // directly here; json-ref/json-contains?/json-ref-or take a
        // variadic path and are registered as raw FuncVs below instead
        // (see that registration's own comment).
        plain "json-list" json::json_list(handle: SteelVal);
        plain "json-array?" json::is_json_array(val: SteelVal);
        plain "json-object?" json::is_json_object(val: SteelVal);
        // Word tokenization: context-free text transform, no host/buffer needed.
        plain "split-words" words::split_words(line: SteelVal, word_chars: SteelVal);

        // Multi-buffer read-only builtins
        cmd "focused-pane" buffers::focused_pane();
        cmd "buffers" buffers::buffers();
        cmd "panes" buffers::panes();
        cmd "buffer-panes" buffers::buffer_panes(pane: args::LivePane);
        cmd "buffer-key" buffers::buffer_key(pane: args::ArgPane);
        cmd "buffer-path" buffers::buffer_path(pane: args::LivePane);
        cmd "buffer-display-path" buffers::buffer_display_path(pane: args::LivePane);
        cmd "buffer-name" buffers::buffer_name(pane: args::LivePane);
        cmd "buffer-dirty?" buffers::buffer_dirty(pane: args::LivePane);
        cmd "buffer-text" buffers::buffer_text(pane: args::LivePane);
        cmd "buffer-line-count" buffers::buffer_line_count(pane: args::LivePane);
        cmd "%buffer-lines" buffers::buffer_lines(pane: args::LivePane, start: args::OptUsize, end: args::OptUsize);
        // Live cursor read: reflects synchronous edits in the same eval.
        cmd "buffer-cursor-line" buffers::buffer_cursor_line(pane: args::LivePane);
        cmd "buffer-selections" buffers::buffer_selections(pane: args::LivePane);
        cmd "offset->line" buffers::offset_to_line(pane: args::LivePane, idx: args::Usize);
        cmd "line->offset" buffers::line_to_offset(pane: args::LivePane, line: args::Usize);

        // Multi-buffer mutating builtins
        cmd "open-buffer!" buffers::open_buffer(path: String);
        cmd "close-buffer!" buffers::close_buffer(pane: args::LivePane);
        cmd "switch-to-buffer!" buffers::switch_to_buffer(pane: args::LivePane, target: args::LivePane);

        // Language identity and grammar builtins
        config "%define-language!" syntax::define_language(name: SteelVal, exts_val: SteelVal, globs_val: SteelVal, shebangs_val: SteelVal, lsp_language_id_val: SteelVal);
        open   "%register-grammar!" syntax::register_grammar(name: SteelVal, grammar_path: SteelVal, symbol: SteelVal, highlights_path: SteelVal, injections_path: SteelVal, textobjects_path: SteelVal);

        // LSP server registration: last-wins, queued (like language regs) and
        // applied at the end of the current eval, from init, plugin activation,
        // or a command/hook body.
        open "%register-lsp-server!" lsp::register_lsp_server(language: SteelVal, command: SteelVal, args_val: SteelVal, root_markers_val: SteelVal, init_options: SteelVal, settings: SteelVal, env_val: SteelVal);
        open "unregister-lsp-server!" lsp::unregister_lsp_server(language: SteelVal);
        // Lifecycle: stop/restart a running server, or open the status view.
        cmd "lsp-stop!" lsp::lsp_stop(target: args::LspTargetArg);
        cmd "lsp-restart!" lsp::lsp_restart(target: args::LspTargetArg);
        cmd "lsp-show-status!" lsp::lsp_show_status(pane: args::LivePane);
        // Generic LSP bridge: any protocol method reachable from Steel.
        cmd "%lsp-request!" lsp::lsp_request(pane: args::LivePane, method: SteelVal, params: SteelVal, callback: SteelVal, allow_stale: SteelVal, supersede: SteelVal, require_focus: SteelVal);
        cmd "lsp-notify!" lsp::lsp_notify(pane: args::LivePane, method: SteelVal, params: SteelVal);
        config "on-lsp-notification" lsp::on_lsp_notification(method: SteelVal, handler: SteelVal);
        // Introspection
        cmd  "lsp-capabilities" lsp::lsp_capabilities(pane: args::ArgPane);
        cmd  "lsp-server-status" lsp::lsp_server_status();
        cmd  "lsp-server-for-buffer" lsp::lsp_server_for_buffer(pane: args::ArgPane);
        open "lsp-registered-for-language?" lsp::lsp_registered_for_language(language: SteelVal);
        cmd "lsp-position-params" lsp::lsp_position_params(pane: args::LivePane);
        cmd "lsp-primary-range-params" lsp::lsp_primary_range_params(pane: args::LivePane);
        cmd "lsp-linewise-ranges-params" lsp::lsp_linewise_ranges_params(pane: args::LivePane);
        cmd "lsp-position->offset" lsp::lsp_position_to_offset(pane: args::LivePane, position: SteelVal);
        cmd "lsp-range->offsets" lsp::lsp_range_to_offsets(pane: args::LivePane, range: SteelVal);
        cmd "lsp-label-offsets->text" lsp::lsp_label_offsets_to_text(label: SteelVal, offsets: SteelVal);
        cmd "lsp-locations->display-parts" lsp::lsp_locations_to_display_parts(locs: SteelVal);
        cmd "viewport-range" buffers::viewport_range(pane: args::LivePane);
        cmd "buffer-generation" buffers::buffer_generation(pane: args::LivePane);
        cmd "buffer-live?" buffers::buffer_live(pane: args::ArgPane);
        cmd "pane-live?" buffers::pane_live(pane: args::ArgPane);
        open "set-hook-triggers!" completion::set_hook_triggers(source: SteelVal, language: SteelVal, chars: SteelVal);

        // Decoration stores + diagnostics pull.
        cmd "set-inlay-hints!" decorations::set_inlay_hints(source: SteelVal, pane: args::LivePane, hints: SteelVal);
        open "register-sign-source!" decorations::register_sign_source(name: SteelVal, pane: args::LivePane, priority: SteelVal);
        cmd "set-signs!" decorations::set_signs(source: SteelVal, pane: args::LivePane, signs: SteelVal);
        cmd "set-virtual-lines!" decorations::set_virtual_lines(source: SteelVal, pane: args::LivePane, lines: SteelVal);
        cmd "set-eol-text!" decorations::set_eol_text(source: SteelVal, pane: args::LivePane, lines: SteelVal);
        cmd "set-extra-highlights!" decorations::set_extra_highlights(source: SteelVal, pane: args::LivePane, spans: SteelVal);
        cmd "set-line-backgrounds!" decorations::set_line_backgrounds(source: SteelVal, pane: args::LivePane, entries: SteelVal);
        cmd "set-statusline-text!" decorations::set_statusline_text(source: SteelVal, pane: args::LivePane, text: SteelVal);
        cmd "%diagnostics-for-buffer" decorations::diagnostics_for_buffer(pane: args::ArgPane, severity: SteelVal, range: SteelVal);
        cmd "diagnostic-counts" decorations::diagnostic_counts(pane: args::ArgPane);

        // Edit + navigation primitives.
        cmd "%apply-text-edits!" edits::apply_text_edits(pane: args::LivePane, edits: SteelVal, expect_gen: SteelVal);
        cmd "%apply-workspace-edit!" edits::apply_workspace_edit(pane: args::LivePane, wsedit: SteelVal, expect_gen: SteelVal);
        cmd "goto-location!" edits::goto_location(pane: args::LivePane, loc: SteelVal);
        cmd "insert-key!" edits::insert_key(pane: args::LivePane, key: SteelVal);
        cmd "selections-linewise?" buffers::selections_linewise(pane: args::LivePane);
        cmd "selections-charwise?" buffers::selections_charwise(pane: args::LivePane);

        // Minibuffer prompt.
        cmd "%prompt!" ui::prompt(pane: args::LivePane, label: SteelVal, prefill: SteelVal, on_confirm: SteelVal);
        cmd "symbol-under-cursor" buffers::symbol_under_cursor(pane: args::LivePane);

        // Completion: sources register at config time; answers, the ranked
        // view, and accept/dismiss are command-time.
        config "%register-completion-source!" completion::register_completion_source(name: String, proc: SteelVal, target: SteelVal, match_kind: SteelVal, priority: SteelVal, resolve: SteelVal);
        cmd "%completion-emit!" completion::completion_emit(id: SteelVal, items: SteelVal, incomplete: SteelVal);
        cmd "completion-top" completion::completion_top(n: SteelVal);
        cmd "completion-accept!" completion::completion_accept(idx: SteelVal);
        cmd "completion-dismiss!" completion::completion_dismiss();
        open "set-completion-triggers!" completion::set_completion_triggers(source: SteelVal, language: SteelVal, chars: SteelVal);

        // Cursor-anchored popup widget.
        cmd "%show-popup!" ui::show_popup(pane: args::LivePane, text: SteelVal, anchor: SteelVal, kind: SteelVal, lang: SteelVal);
        cmd "close-popup!" ui::close_popup(token: SteelVal);

        // Selection menu widget.
        cmd "show-menu!" ui::show_menu(pane: args::LivePane, items: SteelVal, on_select: SteelVal);
        cmd "close-menu!" ui::close_menu(token: SteelVal);

        // Bottom drawer.
        cmd "show-drawer-list!" ui::show_drawer_list(pane: args::LivePane, items: SteelVal, on_select: SteelVal);
        cmd "close-drawer!" ui::close_drawer(token: SteelVal);
        cmd "update-drawer-list!" ui::update_drawer_list(token: SteelVal, items: SteelVal, on_select: SteelVal, selected: SteelVal);
        cmd "drawer-selected-index" ui::drawer_selected_index(token: SteelVal);

        // Fuzzy-picker widget.
        cmd "%picker!" ui::picker(pane: args::LivePane, items: SteelVal, on_select: SteelVal, prompt: SteelVal, pending: SteelVal, query: SteelVal, truncate: SteelVal, actions: SteelVal);
        cmd "%live-picker!" ui::live_picker(pane: args::LivePane, on_select: SteelVal, prompt: SteelVal, query: SteelVal, on_query_change: SteelVal, truncate: SteelVal, actions: SteelVal);
        cmd "picker-push!" ui::picker_push(token: SteelVal, items: SteelVal);
        cmd "picker-replace!" ui::picker_replace(token: SteelVal, items: SteelVal);
        cmd "%picker-source-spawn!" ui::picker_source_spawn(token: SteelVal, cmd: SteelVal, args: SteelVal, cwd: SteelVal, nul: SteelVal, ok_exit_codes: SteelVal);
        cmd "picker-source-stop!" ui::picker_source_stop(token: SteelVal);
        cmd "picker-close!" ui::picker_close(token: SteelVal);
        // Backs live-picker!'s #:command validation only. See args::is_callable's doc.
        plain "%callable?" args::is_callable(val: SteelVal);

        // Timers: not LSP-specific, any plugin can schedule one.
        cmd "after!" timers::after(ms: SteelVal, thunk: SteelVal);
        cmd "cancel-timer!" timers::cancel_timer(id: SteelVal);

        // Generic async subprocess execution: one-shot capture, not a
        // streaming source (that's `picker-source-spawn!`'s shape).
        cmd "%spawn-async!" process::spawn_async(cmd: SteelVal, args: SteelVal, cwd: SteelVal, callback: SteelVal);
        cmd "cancel-async!" process::cancel_async(id: SteelVal);

        // Blocking subprocess capture, no callback. Backs `stdlib/run`.
        // `open`, not `cmd`: Steel's own `spawn-process`/`wait`, which
        // `run_capture` stands in for (see its own doc), carry no legality
        // gate either, and `stdlib/run` is a plain helper any plugin body can
        // reach, not a top-level dispatched command.
        open "%run-capture!" process::run_capture(cmd: SteelVal, args: SteelVal, cwd: SteelVal);

        cmd "diff-lines" diff::diff_lines(old: SteelVal, new: SteelVal);
        cmd "diff-buffer-lines" diff::diff_buffer_lines(pane: args::LivePane, ref_text: SteelVal);
        cmd "diff-words" diff::diff_words(old: SteelVal, new: SteelVal);
        open "language-has-grammar?" syntax::language_has_grammar(name: SteelVal);
        cmd "buffer-language" buffers::buffer_language(pane: args::LivePane);
        cmd "set-buffer-language!" buffers::set_buffer_language_steel(pane: args::LivePane, lang: args::OptString);

        // Editor-integration directory info, read from `ctx.dirs` (computed
        // once by `ScriptingHost::new`). Callable from anywhere (`open`).
        open "data-dir" fs::data_dir();
        open "runtime-dir" fs::runtime_dir();
    }

    // Context-free builtins that don't fit the typed-arity table above: raw
    // `&[SteelVal]` FuncV, no SteelCtx. `path-join` is a pure string helper;
    // `hume-target` reads platform info, not directory state; `json-ref`/
    // `json-contains?`/`json-ref-or` take a variadic path (`j seg ...`),
    // which the typed-arity table's fixed parameter list can't express.
    steel.register_value("hume-target", SteelVal::FuncV(install::hume_target));
    steel.register_value("path-join", SteelVal::FuncV(fs::path_join));
    steel.register_value("path->display", SteelVal::FuncV(fs::path_to_display));
    steel.register_value("json-ref", SteelVal::FuncV(json::json_ref));
    steel.register_value("json-contains?", SteelVal::FuncV(json::json_contains));
    steel.register_value("json-ref-or", SteelVal::FuncV(json::json_ref_or));

    // Evaluate the Scheme bootstrap (defines `load-plugin!`, and, at its
    // tail, captures steel-core's original print functions/port before
    // anything shadows them). Runs before any user init.scm; HUME_CTX is not
    // yet set but the bootstrap only uses `define`, so no builtins are
    // called at this point.
    steel
        .compile_and_run_raw_program(BOOTSTRAP.to_owned())
        .expect("HUME scripting bootstrap failed: this is a bug");

    // PRINT_GATE_SHIMS must compile as its OWN program, separate from
    // BOOTSTRAP: steel-core rejects a single compiled unit that both
    // references a name (the `%raw-*` captures above) and redefines that
    // same name later in the same unit: "variable redefined within the top
    // level definition" / "cannot reference an identifier before its
    // definition" (verified empirically against steel-core 0.8.3; see
    // io.rs's module doc). Splitting into two sequential top-level programs
    // sidesteps this: by the time this call compiles, `displayln` etc. are
    // ordinary already-bound globals, and redefining them here is a plain
    // global rebind, with no self-reference within the same unit.
    steel
        .compile_and_run_raw_program(PRINT_GATE_SHIMS.to_owned())
        .expect("HUME scripting print-gate shims failed: this is a bug");

    // Append the same shims to steel-core's prelude string. The prelude is
    // prepended to every `(require "path.scm")` compilation unit (steel-core
    // internals; see io.rs's module doc), so this closes the gap where a
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
