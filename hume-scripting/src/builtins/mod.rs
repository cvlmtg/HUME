//! Steel builtins for HUME's scripting layer.
//!
//! [`register_all`] registers every builtin on the Steel engine and then evaluates
//! the Scheme bootstrap that defines `load-plugin` and `declare-plugin`.
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
// zero-trigger call (no #:commands/#:events/#:languages) evaluates
// <plugin-dir>/manifest.scm instead for its default entries (see
// %begin-manifest-declare!); caller's #:config wins over the manifest's.
// Known limitation, left as-is pending an upstream steel-core fix: a
// zero-trigger call inside an outer with-handler can hit the "no open
// continuation" panic documented at
// known_limitation_reraise_via_raise_error_inside_outer_tolerant_handler_corrupts_vm_stack
// (lib.rs) if manifest.scm raises — swallowing the error instead would break
// declare-plugin's tested propagate-to-caller contract (4 tests in
// builtins/plugins/tests.rs, via .expect_err).
//
// load-plugin — eager init-context activation; declares/resolves then
// delegates to %activate-plugin-inline. Valid only during init.scm /
// :reload-config (enforced by %load-plugin!). #:config as above.
//
// %activate-plugin-inline — shared by load-plugin and %dispatch-command's
// lazy-miss path. %begin-lazy-activation returns the require string for
// Declared plugins, #f otherwise (cycle guard + idempotency).
//
// lsp-request — generic LSP bridge. server: registered language name, or #f
// for the focused buffer's attached server. callback: (lambda (err result)),
// exactly one non-#f. #:allow-stale skips the staleness check. #:supersede
// <key> cancels the caller's own previous still-pending request filed under
// the same (server, key) — opt-in, not automatic by method/buffer.
//
// get-option — rest-only parameter list, not a mixed fixed-plus-rest one: a
// 2+-positional call site compiled inside a required module hits the
// steel-core 0.8.2 limitation in io.rs's module doc, and every plugin body is
// a required module. Arity dispatch and semantics: settings.rs's %get-option.
//
// debounce — trailing-edge: each call reschedules proc `ms` out, cancelling
// any still-pending call from a prior invocation. Pure Scheme, no Rust
// debouncer. An armed timer clears `pending` only if it's still the entry
// stored there, checked via the `my-id` box its own closure captures: a timer
// already popped-and-queued is past cancelling, so without the check it would
// clear the *next* call's id on the way out — orphaning that timer, no longer
// cancellable but still ticking, to fire a stray duplicate later. Routine
// under settle()'s always-draining loop, not a corner case.
//
// debounce-by — as debounce, but keyed per first-argument value instead of one
// shared pending timer, with the same current-entry check per key: a call
// keyed k1 never cancels a call keyed k2. The key is `(car args)`, not a
// separate keyfn argument, matching the single-bid handler shape every
// debounce call site already uses — so swapping one for the other at an
// existing site needs no further change.
//
// picker!/live-picker! — two constructors over one Rust store
// (hume-editor::editor::picker::PickerSession): picker! stays a plain
// items-plus-fuzzy-filter picker; live-picker! always drives an external
// #:command builder and disables local fuzzy filtering entirely (see
// PickerSession::rebuild_filtered's doc) — so "is this session live" is a
// name a caller chooses, not a keyword's side effect.
//
// live-picker!'s wrapper owns the whole requery lifecycle by construction —
// stop the running source, debounce, respawn via #:command — rather than
// making every author hand-wire it. The wrapper's own internal lambda — not
// the caller's #:command — is what's stop-then-debounce; it's called on
// *every* keystroke, unconditionally, so a query that debounces to empty
// still cancels whatever the previous non-empty keystroke armed, rather
// than stranding a timer that fires later for a pattern the query box no
// longer shows.
//
// The previous pattern's rows stay on screen through the whole stop/
// debounce/respawn gap — clearing immediately, the first design, produced
// a visible blank-then-repaint flash on every keystroke. `picker-source-stop!`
// still runs immediately (a still-running source for the old pattern must
// not keep appending rows while the query changes again); only the *clear*
// moved, into `spawn-for`'s #f branch, so it fires solely when a query
// settles on nothing to search rather than on every intermediate keystroke.
// The swap itself lives in `PickerSession::attach_source`/`push`
// (hume-editor::editor::picker): a live session's attached source is
// marked to replace `items` wholesale on its own first batch, instead of
// this wrapper clearing ahead of time — see `AttachedSource::supersedes_rows`'s
// doc for why that has to be scoped to the source, not the session, to stay
// race-free against a stale batch from the source just killed. Meanwhile
// `PickerSession::is_pending` (hence the panel's "…" marker) stays true
// across the same gap via `requery_armed`, so the on-screen rows read as
// "refreshing" rather than settled while they're stale. #:command
// returning `#f` means "nothing to spawn for this query" — the empty-query
// guard lives inside the builder a caller writes, not as a separate flag
// threaded through the wrapper, so there is nothing to forget.
//
// %live-picker! hands its own session token into the internal callback as
// an argument, rather than the callback closing over live-picker!'s return
// value, which isn't bound yet while live-picker! is still running — that
// argument is what lets the per-keystroke lambda reference its own session
// safely. A non-empty #:query, separately, spawns synchronously through
// live-picker!'s own let*-bound token, right after %live-picker! returns —
// no deferred tick needed, since that token is already bound by the time
// the seed spawn runs. %callable? (args::is_callable) backs live-picker!'s
// own #:command check — a stricter predicate than Steel's `procedure?`, see
// its doc for why.
//
// run-inline-output! — the Scheme wrapper (see bootstrap.scm) blocks and
// raises on nonzero exit or a signal-killed child. Same contract as
// plum/run!, so call sites need no manual exit-code checks. `%run-inline-output!`
// below is the process-group-isolated spawn behind it (see
// hume-platform::process::run_inline_output for why this can't be Steel's
// own spawn-process).
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
// %port-safe? — writing to `port` is TUI-safe unless it IS the real stdout
// port, in which case defer to the gate (see builtins/io.rs's module doc for
// why steel-core's original print fns are captured before PRINT_GATE_SHIMS
// redefines the names). Shared by every shim's
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
/// Must be called exactly once during [`crate::ScriptingHost::new`], before any
/// `eval_init` calls.
pub(crate) fn register_all(steel: &mut Engine) {
    // Pre-register HUME_CTX so supply_context_arg can generate its wrapper
    // functions without a FreeIdentifier error.  The real SteelVal::Reference
    // is injected at eval / dispatch time via steel.update_value.
    steel.register_value(HUME_CTX, SteelVal::Void);

    builtins! { steel,
        // Config / settings
        open "set-option!" settings::set_option(key: String, value: SteelVal);
        cmd  "set-buffer-option!" settings::set_buffer_option(bid: args::BidArg, key: String, value: SteelVal);
        open "%get-option" settings::get_option(key: String, bid: SteelVal);
        open "configure-statusline!" statusline::configure_statusline(left: SteelVal, center: SteelVal, right: SteelVal);

        // Step budget
        open "hume/yield!" interrupt::hume_yield();

        // Keymap
        config "bind-key!" keymap_bind::bind_key(mode: SteelVal, key_str: String, cmd_name: String);
        config "bind-key-extend!" keymap_bind::bind_key_extend(mode: SteelVal, key_str: String, cmd_name: String);
        config "unbind-key!" keymap_bind::unbind_key(mode: SteelVal, key_str: String);
        config "bind-wait-char!" keymap_bind::bind_wait_char(mode: SteelVal, key_str: String, cmd_name: String);
        cmd    "set-register-prefix!" commands::set_register_prefix(name: String);

        // Registers — direct read/write, independent of set-register-prefix!'s
        // per-command targeting.
        open "read-register" registers::read_register(name: String);
        open "write-register!" registers::write_register(name: String, values: SteelVal);

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
        // grammar.rs's module doc); `grammar-output-path` is plain Scheme.
        open "compile-grammar!" grammar::compile_grammar(src: String, out: String);

        // LSP server install pipeline — sha256 hashing, archive unpacking,
        // platform id, cross-process install lock.
        // Sandbox-free — full-trust plugin model. `verify-sha256!`/`exe-on-path?`
        // /`git-clone`/`curl-fetch`/`npm-install!` are plain Scheme, atop Steel's
        // own `steel/process` stdlib (`which`, `spawn-process`).
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
        cmd "buffer-display-path" buffers::buffer_display_path(bid: args::BidArg);
        cmd "buffer-name" buffers::buffer_name(bid: args::BidArg);
        cmd "buffer-dirty?" buffers::buffer_dirty(bid: args::BidArg);
        cmd "buffer-text" buffers::buffer_text(bid: args::BidArg);
        cmd "%buffer-lines" buffers::buffer_lines(bid: args::BidArg, start: SteelVal, end: SteelVal);
        // Live cursor read — reflects synchronous edits in the same eval.
        cmd "current-line-number" buffers::current_line_number();
        cmd "current-selections" buffers::current_selections();
        cmd "char-index->line" buffers::char_index_to_line(idx: SteelVal);
        cmd "line->offset" buffers::line_to_offset(bid: args::BidArg, line: SteelVal);

        // Multi-buffer mutating builtins
        cmd "open-buffer!" buffers::open_buffer(path: String);
        cmd "close-buffer!" buffers::close_buffer(bid: args::BidArg);
        cmd "switch-to-buffer!" buffers::switch_to_buffer(bid: args::BidArg);

        // Language identity and grammar builtins
        config "%define-language!" syntax::define_language(name: SteelVal, exts_val: SteelVal, globs_val: SteelVal, shebangs_val: SteelVal, lsp_language_id_val: SteelVal);
        open   "%register-grammar!" syntax::register_grammar(name: SteelVal, grammar_path: SteelVal, symbol: SteelVal, highlights_path: SteelVal, injections_path: SteelVal);

        // LSP server registration — last-wins, queued (like language regs) and
        // applied at the end of the current eval, from init, plugin activation,
        // or a command/hook body.
        open "%register-lsp-server!" lsp::register_lsp_server(language: SteelVal, command: SteelVal, args_val: SteelVal, root_markers_val: SteelVal, init_options: SteelVal, settings: SteelVal, env_val: SteelVal);
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
        cmd "lsp-primary-range-params" lsp::lsp_primary_range_params(bid: args::BidArg);
        cmd "lsp-linewise-ranges-params" lsp::lsp_linewise_ranges_params(bid: args::BidArg);
        cmd "lsp-position->offset" lsp::lsp_position_to_offset(bid: args::BidArg, position: SteelVal);
        cmd "lsp-range->offsets" lsp::lsp_range_to_offsets(bid: args::BidArg, range: SteelVal);
        cmd "lsp-label-offsets->text" lsp::lsp_label_offsets_to_text(bid: args::BidArg, label: SteelVal, offsets: SteelVal);
        cmd "lsp-locations->display-parts" lsp::lsp_locations_to_display_parts(locs: SteelVal);
        cmd "viewport-range" buffers::viewport_range(bid: args::BidArg);
        cmd "buffer-generation" buffers::buffer_generation(bid: args::BidArg);
        open "register-trigger-chars!" completion::register_trigger_chars(source: SteelVal, language: SteelVal, chars: SteelVal);

        // Decoration stores + diagnostics pull.
        cmd "set-inlay-hints!" decorations::set_inlay_hints(source: SteelVal, bid: args::BidArg, hints: SteelVal);
        open "register-sign-source!" decorations::register_sign_source(name: SteelVal, bid: args::BidArg, priority: SteelVal);
        cmd "set-signs!" decorations::set_signs(source: SteelVal, bid: args::BidArg, signs: SteelVal);
        cmd "set-virtual-lines!" decorations::set_virtual_lines(source: SteelVal, bid: args::BidArg, lines: SteelVal);
        cmd "set-eol-text!" decorations::set_eol_text(source: SteelVal, bid: args::BidArg, lines: SteelVal);
        cmd "set-extra-highlights!" decorations::set_extra_highlights(source: SteelVal, bid: args::BidArg, spans: SteelVal);
        cmd "set-line-backgrounds!" decorations::set_line_backgrounds(source: SteelVal, bid: args::BidArg, entries: SteelVal);
        cmd "set-statusline-text!" decorations::set_statusline_text(source: SteelVal, bid: args::BidArg, text: SteelVal);
        cmd "%diagnostics-for-buffer" decorations::diagnostics_for_buffer(bid: args::BidArg, severity: SteelVal, range: SteelVal);
        cmd "diagnostic-counts" decorations::diagnostic_counts(bid: args::BidArg);

        // Edit + navigation primitives.
        cmd "%apply-text-edits!" edits::apply_text_edits(bid: args::BidArg, edits: SteelVal, expect_gen: SteelVal);
        cmd "%apply-workspace-edit!" edits::apply_workspace_edit(wsedit: SteelVal);
        cmd "goto-location!" edits::goto_location(loc: SteelVal);
        cmd "selections-linewise?" buffers::selections_linewise(bid: args::BidArg);

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
        cmd "%show-popup!" ui::show_popup(text: SteelVal, anchor: SteelVal, kind: SteelVal, lang: SteelVal);
        cmd "close-popup!" ui::close_popup();

        // Selection menu widget.
        cmd "show-menu!" ui::show_menu(items: SteelVal, on_select: SteelVal);
        cmd "close-menu!" ui::close_menu();

        // Class B bottom drawer.
        cmd "show-drawer-list!" ui::show_drawer_list(items: SteelVal, on_select: SteelVal);
        cmd "close-drawer!" ui::close_drawer();

        // Fuzzy-picker widget.
        cmd "%picker!" ui::picker(items: SteelVal, on_select: SteelVal, prompt: SteelVal, pending: SteelVal, query: SteelVal, truncate: SteelVal);
        cmd "%live-picker!" ui::live_picker(on_select: SteelVal, prompt: SteelVal, query: SteelVal, on_query_change: SteelVal, truncate: SteelVal);
        cmd "picker-push!" ui::picker_push(token: SteelVal, items: SteelVal);
        cmd "picker-replace!" ui::picker_replace(token: SteelVal, items: SteelVal);
        cmd "%picker-source-spawn!" ui::picker_source_spawn(token: SteelVal, cmd: SteelVal, args: SteelVal, cwd: SteelVal, nul: SteelVal, ok_exit_codes: SteelVal);
        cmd "picker-source-stop!" ui::picker_source_stop(token: SteelVal);
        cmd "%picker-close!" ui::picker_close(token: SteelVal);
        // Backs live-picker!'s #:command validation only — see args::is_callable's doc.
        plain "%callable?" args::is_callable(val: SteelVal);

        // Timers — not LSP-specific; any plugin can schedule one.
        cmd "after" timers::after(ms: SteelVal, thunk: SteelVal);
        cmd "cancel-timer!" timers::cancel_timer(id: SteelVal);

        // Generic async subprocess execution — one-shot capture, not a
        // streaming source (that's `picker-source-spawn!`'s shape).
        cmd "spawn-async!" process::spawn_async(cmd: SteelVal, args: SteelVal, cwd: SteelVal, callback: SteelVal);
        cmd "cancel-async!" process::cancel_async(id: SteelVal);

        cmd "diff-lines" diff::diff_lines(old: SteelVal, new: SteelVal);
        cmd "diff-buffer-lines" diff::diff_buffer_lines(bid: args::BidArg, ref_text: SteelVal);
        cmd "diff-words" diff::diff_words(old: SteelVal, new: SteelVal);
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
    steel.register_value("path->display", SteelVal::FuncV(fs::path_to_display));

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
