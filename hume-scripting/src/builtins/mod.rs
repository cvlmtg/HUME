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
pub(crate) mod interrupt;
pub(crate) mod io;
pub(crate) mod json;
pub(crate) mod keymap_bind;
pub(crate) mod lsp;
pub(crate) mod plugins;
pub(crate) mod process;
pub(crate) mod registers;
pub(crate) mod settings;
pub(crate) mod syntax;
pub(crate) mod timers;
pub(crate) mod ui;
pub(crate) mod version;
pub(crate) mod wire;
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
/// The declared arg types are compiler-checked, not documentation: each entry
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
// A plugin is one or more entries: files in its directory, each with its own
// triggers, state and rollback scope. `declare-plugin! #:entry` names the file
// (default `plugin.scm`); `(plugin-config)` and `(plugin-dir)` belong to the
// plugin, whose config comes from `load-plugin!` alone. `load-plugin!` takes
// no `#:entry`: a plugin with a manifest.scm is lazy and the manifest declares
// its entries; one without loads `plugin.scm` eagerly.
//
// %activate-plugin-inline!: takes the plugin and entry file as two strings.
// %begin-lazy-activation! moves a Declared entry to
// Loading and returns its `(require "<abs>")` string (#f otherwise: cycle
// guard and idempotency), eval-string runs it in the live VM, and
// %finish-lazy-activation! records the outcome. The handler hands the error to
// %finish-lazy-activation! and returns normally, so a failing plugin never
// aborts the enclosing eval and nothing is re-raised. The trailing
// (hume/yield!) then re-checks the interrupt flag with a fresh raise: an
// exhausted step budget must abort the whole eval, not be recorded as every
// later plugin failing to load.
//
// load-plugin!: with a manifest.scm, evaluates it under the same handler/yield
// contract, %finish-manifest-load! recording any error.
//
// define-typed-command!: the `:` counterpart of define-command!, sharing its
// collision guard (commands::check_definable) and bookkeeping but registering
// a TypedBody::Steel entry. A name defined one way is never reachable the
// other way. No #:repeatable: dot-repeat means nothing for a `:` command.
//
// call! / %dispatch-command!: the dispatcher for calls from inside Steel (call!
// and the bare command-name lambdas); keypress and `:` dispatch use
// ScriptingHost::call_steel_cmd instead. The miss, activate, retry path lives
// in Scheme because a builtin can't re-enter the Rust dispatcher while the
// Engine is borrowed. A retry miss raises rather than falling through to
// %call-native!, which would misreport the name as unknown. call! is defined
// here, not only in prelude.scm, so harnesses without the prelude have it.
//
// command-exists?: one probe for "can the caller use `name`". The registry
// answer (%command-callable?) covers commands, including a lazy plugin's
// not-yet-activated ones. The fallback resolves `name` as a global in the VM,
// which covers builtins and prelude forms; it lives in Scheme because a
// builtin has no access to the Engine it runs in. The handler returns #f and
// never re-raises, so the hazard above does not apply. A macro name (`call!`)
// fails to evaluate as a value and reads as absent.
//
// %apply-command!: arms the #:inline-output alt-screen bracket for call!, as
// Editor::call_steel_command_body does for keypress and `:`. The restore
// truncates to the armed depth instead of popping, so an unpaired descendant
// frame is never taken for this call's own. No with-handler (hazard above): a
// raising body skips the restore; run_steel_session truncates to zero at end.
//
// %call-held-handler: starts a held event's handler that takes `done`, with
// the event's args followed by a thunk that releases hold `hold`.
// ScriptingHost::fire_held_hook calls every such handler through it.
//
// lsp-request!: callback is (lambda (err result)), one of them non-#f; err is
// (hash 'kind 'message ['code]).
// lsp-request-all!: callback is (lambda (err results)), results one hash per
// server. the method, #:feature and #:to choose the servers when the request is
// sent. #:supersede <key> cancels the caller's own pending request under the
// same key. #:require-focus drops the callback unless pane is still focused
// when the answers arrive.
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
// runs #:source per query and disables local filtering. Each keystroke
// stops the running source and re-arms the debounced requery. Old rows stay
// until the new source's first batch replaces them (no blank frame per
// keystroke); `picker-command-source` clears them when its argv function
// returns #f. Only the timer-dispatched requery gets the clear-and-re-raise
// handler, since it has no outer handler; the seed call for a non-empty
// #:query runs on the caller's stack and must stay unwrapped.
//
// A live session mints a new token for every query and passes it to
// #:source. picker-push!, picker-replace! and picker-source-spawn! accept
// only the current one, so an answer to an earlier query is dropped by the
// editor. picker-close! and picker-source-stop! also accept the token
// live-picker! returned.
//
// run-inline-output!: %run-inline-output! raises unless the command exits 0,
// and spawns it in an isolated process group (see
// hume-platform::process::run_inline_output).
//
// %port-safe?: a port is safe unless it is the real stdout, where the gate
// decides. Every print shim's explicit-port branch uses it (io.rs module doc).
//
// stems-with-suffix: sorts after stripping the suffix, since sorting full
// file names would put "a-b.x" ahead of "a.x" ('-' < '.').
//
// read-data-file: the one reader of a data file, returning its single
// top-level form. It parses with hm.read!, never `read`: steel-core's `read`
// keeps one buffer across calls, returns eof instead of raising on bad
// syntax, and leaves the bad text for the next call. The parse error is
// caught into a marker and a fresh error naming the file is raised after the
// handler returns, so nothing is re-raised (hazard above).
//
// data-file: a name resolves only when `<name>.scm` is spelled exactly like an
// entry in the directory's listing. That keeps a lookup inside the directory
// and gives the same answer on a case-insensitive filesystem as on a
// case-sensitive one.
//
// %read-language-data: the one reader of `<runtime>/scheme/languages/<name>.scm`
// (record format: runtime/scheme/README.md), on data-file and read-data-file;
// an unlisted name, or a missing runtime dir, reads as #f.
// %bundled-language-config is the editor's view: every entry but `grammar`,
// as a hash whose keys ScriptingHost::bundled_language's decoder defines and
// checks. bundled-language-grammar and bundled-grammar-languages are the
// plugins' view.
const BOOTSTRAP: &str = include_str!("bootstrap.scm");

// PRELUDE_SHIMS redefines the steel-core prelude names HUME takes over: the
// print functions and `which`. It is appended both to BOOTSTRAP (top level)
// and, verbatim, to steel-core's own prelude string via set_prelude_string.
// Since the prelude is prepended to every `(require "path.scm")` unit, this
// closes the gap where a required module's (every real plugin's) calls would
// otherwise resolve straight to steel-core's originals. See io.rs's module
// doc for the print gate's root-cause writeup and the rest-only
// parameter-list requirement every shim below follows.
//
// `which` searches the editor's search path (`%which`) instead of the process
// `PATH`, so the `PATH` a session's children get is also the one its
// lookups see.
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
const PRELUDE_SHIMS: &str = include_str!("prelude_shims.scm");

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

        // Step budget
        open "hume/yield!" interrupt::hume_yield();

        // Keymap
        open   "%bind-key!" keymap_bind::bind_key(mode: SteelVal, key_str: String, cmd_name: String, keymap: SteelVal);
        open   "%bind-key-extend!" keymap_bind::bind_key_extend(mode: SteelVal, key_str: String, cmd_name: String, keymap: SteelVal);
        open   "%unbind-key!" keymap_bind::unbind_key(mode: SteelVal, key_str: String, keymap: SteelVal);
        cmd    "attach-keymap!" keymap_bind::attach_keymap(pane: args::LivePane, name: SteelVal);
        cmd    "detach-keymap!" keymap_bind::detach_keymap(pane: args::LivePane, name: SteelVal);
        cmd    "set-register-prefix!" commands::set_register_prefix(name: String);

        // Registers: direct read/write, independent of set-register-prefix!'s
        // per-command targeting.
        open "read-register" registers::read_register(name: String);
        open "write-register!" registers::write_register(name: String, values: SteelVal);
        open "register-name?" registers::register_name(name: String);

        // Plugin lifecycle
        open "%declare-plugin!" plugins::declare_plugin(name: String, entry: SteelVal, commands: SteelVal, typed_commands: SteelVal, events: SteelVal, languages: SteelVal);
        open "resolve-plugin-path" plugins::resolve_plugin_path(name: String);

        // Plugin introspection and explicit activation
        open "loaded-plugins" plugins::loaded_plugins();
        open "declared-plugins" plugins::declared_plugins();
        open "declared-plugin-ref" plugins::declared_plugin_ref(name: String);
        open "plugin-config" plugins::plugin_config();
        open "plugin-dir" plugins::plugin_dir();
        open "%load-plugin!" plugins::load_plugin(name: String, config: SteelVal, git_ref: SteelVal);

        // Inline activation primitives, called from the %activate-plugin-inline!
        // Scheme helper to drive mid-eval plugin loading without &mut Engine.
        open "%begin-lazy-activation!" plugins::activation::begin_lazy_activation(plugin: String, entry: SteelVal);
        open "%finish-lazy-activation!" plugins::activation::finish_lazy_activation(error: SteelVal);
        open "%entry-loaded?" plugins::activation::entry_loaded(plugin: String, entry: SteelVal);
        open "%lazy-command-owner" plugins::activation::lazy_command_owner(name: String);

        // Manifest evaluation: %load-plugin! returns the require string of
        // <plugin-dir>/manifest.scm, and the wrapper reports the outcome here.
        open "%finish-manifest-load!" plugins::activation::finish_manifest_load(name: String, error: SteelVal);

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
        // (call! name args…) macro desugars to (%dispatch-command! …) which routes
        // activated plugin commands inline in Steel and falls back here for everything else.
        open "%call-native!" commands::call_command_primitive(name: String, args: SteelVal);
        // %lookup-plugin-proc: returns the Steel closure for an activated plugin command,
        // or #f. Called by %dispatch-command! in Steel to decide inline-apply vs. %call-native!.
        open "%lookup-plugin-proc" commands::lookup_plugin_proc(name: String);
        // %arm-inline-output!/%restore-inline-output!: the call!-path counterpart
        // of dispatch.rs's own inline-output arm/close, wrapping %dispatch-command!'s
        // in-VM apply. See the BOOTSTRAP comment block above for the full picture.
        open "%arm-inline-output!" commands::arm_inline_output(name: String);
        open "%restore-inline-output!" commands::restore_inline_output(depth: SteelVal);
        open "command-plugin" commands::command_plugin(name: String);
        open "%command-callable?" commands::command_callable(name: String);

        // Grammar compilation: sandbox-free, full-trust plugin model. A Rust
        // builtin for the reasons in grammar.rs's module doc;
        // `grammar-output-path` is plain Scheme.
        open "compile-grammar!" grammar::compile_grammar(src: String, out: String);

        // Inline-output subprocess: process-group-isolated spawn for
        // `#:inline-output` commands, sandbox-free like Steel's own stdlib.
        open  "%run-inline-output!" process::run_inline_output(cmd: String, args_val: SteelVal, cwd_val: SteelVal, env_val: SteelVal);

        // Logging: push messages to the editor message log
        open "log!" crate::log::log_msg(severity: SteelVal, message: String);

        // %stdout-gate! is the Rust leaf behind the gated print shims (displayln,
        // display, print, println, newline). See io.rs and PRELUDE_SHIMS above.
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
        // Context-free text transforms: no host/buffer needed.
        plain "split-words" words::split_words(line: SteelVal, word_chars: SteelVal);
        plain "string-common-prefix" words::string_common_prefix(strings: SteelVal);

        // Multi-buffer read-only builtins
        cmd "focused-pane" buffers::focused_pane();
        cmd "buffers" buffers::buffers();
        cmd "panes" buffers::panes();
        cmd "buffer-panes" buffers::buffer_panes(pane: args::LivePane);
        cmd "buffer-key" buffers::buffer_key(pane: args::ArgPane);
        cmd "buffer-path" buffers::buffer_path(pane: args::LivePane);
        cmd "%buffer-display-path" buffers::buffer_display_path(pane: args::LivePane, relative: SteelVal);
        cmd "buffer-name" buffers::buffer_name(pane: args::LivePane);
        cmd "buffer-dirty?" buffers::buffer_dirty(pane: args::LivePane);
        cmd "buffer-text" buffers::buffer_text(pane: args::LivePane);
        cmd "buffer-line-count" buffers::buffer_line_count(pane: args::LivePane);
        cmd "%buffer-lines" buffers::buffer_lines(pane: args::LivePane, range: SteelVal);
        // Live cursor read: reflects synchronous edits in the same eval.
        cmd "buffer-cursor-line" buffers::buffer_cursor_line(pane: args::LivePane);
        cmd "buffer-selections" buffers::buffer_selections(pane: args::LivePane);
        cmd "char-offset->line" buffers::char_offset_to_line(pane: args::LivePane, idx: args::Usize);
        cmd "line->char-offset" buffers::line_to_char_offset(pane: args::LivePane, line: args::Usize);

        // Multi-buffer mutating builtins
        cmd "open-buffer!" buffers::open_buffer(path: String);
        cmd "close-buffer!" buffers::close_buffer(pane: args::LivePane);
        cmd "switch-to-buffer!" buffers::switch_to_buffer(pane: args::LivePane, target: args::LivePane);
        cmd "show-read-only-buffer!" buffers::show_read_only_buffer(pane: args::LivePane, name: String, text: String);

        // Language identity and grammar builtins
        config "%define-language!" syntax::define_language(name: SteelVal, exts_val: SteelVal, globs_val: SteelVal, shebangs_val: SteelVal);
        config "%configure-language!" syntax::configure_language(name: SteelVal, fields_val: SteelVal);
        open   "%register-grammar!" syntax::register_grammar(name: SteelVal, grammar_path: SteelVal, symbol: SteelVal, highlights_path: SteelVal, optional_queries: SteelVal);
        plain  "grammar-optional-queries" syntax::grammar_optional_queries();

        // LSP server registration: last-wins, queued (like language regs) and
        // applied at the end of the current eval, from init, plugin activation,
        // or a command/hook body.
        open "%register-lsp-server!" lsp::register_lsp_server(name: SteelVal, command: SteelVal, args_val: SteelVal, init_options: SteelVal, settings: SteelVal, env_val: SteelVal);
        open "unregister-lsp-server!" lsp::unregister_lsp_server(name: SteelVal);
        open "set-language-servers!" lsp::set_language_servers(language: SteelVal, entries: SteelVal);
        open "set-default-language-servers!" lsp::set_default_language_servers(language: SteelVal, entries: SteelVal);
        open "lsp-language-servers" lsp::lsp_language_servers(language: SteelVal);
        // Lifecycle: stop/restart a running server, or open the status view.
        cmd "lsp-stop!" lsp::lsp_stop(target: lsp::LspTargetArg);
        cmd "lsp-restart!" lsp::lsp_restart(target: lsp::LspTargetArg);
        cmd "lsp-show-status!" lsp::lsp_show_status(pane: args::LivePane);
        // Generic LSP bridge: any protocol method reachable from Steel.
        cmd "%lsp-request!" lsp::lsp_request(pane: args::LivePane, method: SteelVal, params: SteelVal, callback: SteelVal, feature: SteelVal, to: SteelVal, unavailable: SteelVal, allow_stale: SteelVal, supersede: SteelVal, require_focus: SteelVal, tracked: SteelVal);
        cmd "%lsp-request-all!" lsp::lsp_request_all(pane: args::LivePane, method: SteelVal, params: SteelVal, callback: SteelVal, feature: SteelVal, unavailable: SteelVal, allow_stale: SteelVal, supersede: SteelVal, require_focus: SteelVal, tracked: SteelVal);
        cmd "%lsp-notify!" lsp::lsp_notify(pane: args::LivePane, method: SteelVal, params: SteelVal, feature: SteelVal, to: SteelVal);
        config "register-lsp-notification-hook!" lsp::register_lsp_notification_hook(methods: SteelVal, proc: SteelVal);
        // Introspection
        cmd  "lsp-capabilities" lsp::lsp_capabilities(server: SteelVal);
        cmd  "%lsp-capability" lsp::lsp_capability(server: SteelVal, feature: SteelVal, method: SteelVal);
        cmd  "%lsp-servers" lsp::lsp_servers(pane: args::LivePane, feature: SteelVal, method: SteelVal);
        plain "lsp-server-name" lsp::lsp_server_name(server: SteelVal);
        cmd  "lsp-server-status" lsp::lsp_server_status();
        open "lsp-server-registered?" lsp::lsp_server_registered(name: SteelVal);
        cmd "lsp-document-params" lsp::lsp_document_params(pane: args::LivePane);
        cmd "lsp-position-params" lsp::lsp_position_params(pane: args::LivePane);
        cmd "track-position!" lsp::track_position(pane: args::LivePane);
        cmd "tracked-position-params" lsp::tracked_position_params(token: SteelVal);
        cmd "untrack-position!" lsp::untrack_position(token: SteelVal);
        cmd "keep-tracked-position!" lsp::keep_tracked_position(token: SteelVal);
        cmd "lsp-primary-range-params" lsp::lsp_primary_range_params(pane: args::LivePane);
        cmd "lsp-linewise-ranges-params" lsp::lsp_linewise_ranges_params(pane: args::LivePane);
        cmd "lsp-position->char-offset" lsp::lsp_position_to_char_offset(pane: args::LivePane, position: SteelVal);
        cmd "lsp-range->char-range" lsp::lsp_range_to_char_range(pane: args::LivePane, range: SteelVal);
        cmd "lsp-label-offsets->text" lsp::lsp_label_offsets_to_text(label: SteelVal, offsets: SteelVal);
        plain "lsp-location" lsp::lsp_location(uri: SteelVal, range: SteelVal);
        cmd "lsp-locations->places" lsp::lsp_locations_to_places(locs: SteelVal);
        cmd "lsp-locations->merged-places" lsp::lsp_locations_to_merged_places(locs: SteelVal);
        cmd "viewport-range" buffers::viewport_range(pane: args::LivePane);
        cmd "buffer-generation" buffers::buffer_generation(pane: args::LivePane);
        cmd "buffer-undo-tree" buffers::buffer_undo_tree(pane: args::LivePane);
        cmd "buffer-live?" buffers::buffer_live(pane: args::ArgPane);
        cmd "pane-live?" buffers::pane_live(pane: args::ArgPane);
        open "set-hook-triggers!" completion::set_hook_triggers(source: SteelVal, language: SteelVal, chars: SteelVal);
        open "set-attachment-hook-triggers!" completion::set_attachment_hook_triggers(source: SteelVal, pane: args::LivePane, server: SteelVal, feature: SteelVal, chars: SteelVal);

        // Decoration stores + diagnostics pull.
        cmd "set-inlay-hints!" decorations::set_inlay_hints(source: SteelVal, pane: args::LivePane, hints: SteelVal);
        open "register-sign-source!" decorations::register_sign_source(name: SteelVal, pane: args::LivePane, priority: SteelVal);
        cmd "set-signs!" decorations::set_signs(source: SteelVal, pane: args::LivePane, signs: SteelVal);
        cmd "set-virtual-lines!" decorations::set_virtual_lines(source: SteelVal, pane: args::LivePane, lines: SteelVal);
        cmd "%set-eol-text!" decorations::set_eol_text(source: SteelVal, pane: args::LivePane, lines: SteelVal, hide_on_insert_line: SteelVal);
        cmd "set-extra-highlights!" decorations::set_extra_highlights(source: SteelVal, pane: args::LivePane, spans: SteelVal);
        cmd "set-line-backgrounds!" decorations::set_line_backgrounds(source: SteelVal, pane: args::LivePane, entries: SteelVal);
        cmd "set-statusline-text!" decorations::set_statusline_text(source: SteelVal, pane: args::LivePane, text: SteelVal);
        cmd "%diagnostics-for-buffer" decorations::diagnostics_for_buffer(pane: args::ArgPane, severity: SteelVal, range: SteelVal);
        cmd "diagnostic-counts" decorations::diagnostic_counts(pane: args::ArgPane);

        // Edit + navigation primitives.
        cmd "%apply-text-edits!" edits::apply_text_edits(pane: args::LivePane, edits: SteelVal, expect_gen: SteelVal);
        cmd "%apply-workspace-edit!" edits::apply_workspace_edit(pane: args::LivePane, wsedit: SteelVal, expect_gen: SteelVal);
        cmd "goto-location!" edits::goto_location(pane: args::LivePane, loc: SteelVal);
        cmd "goto-revision!" edits::goto_revision(pane: args::LivePane, revision: args::Usize);
        cmd "insert-key!" edits::insert_key(pane: args::LivePane, key: SteelVal);
        cmd "paste-text!" edits::paste_text(pane: args::LivePane, text: SteelVal);
        cmd "edit-text!" edits::edit_text(pane: args::LivePane, edits: SteelVal);
        cmd "selections-linewise?" buffers::selections_linewise(pane: args::LivePane);
        cmd "selections-charwise?" buffers::selections_charwise(pane: args::LivePane);

        // Minibuffer prompt.
        cmd "%prompt!" ui::prompt(pane: args::LivePane, label: SteelVal, prefill: SteelVal, on_confirm: SteelVal);
        cmd "on-next-char!" ui::on_next_char(callback: SteelVal);
        cmd "symbol-under-cursor" buffers::symbol_under_cursor(pane: args::LivePane);

        // Completion: sources register at config time; answers, the ranked
        // view, and accept/dismiss are command-time.
        config "%register-completion-source!" completion::register_completion_source(name: String, proc: SteelVal, target: SteelVal, match_kind: SteelVal, priority: SteelVal, resolve: SteelVal, token_chars: SteelVal);
        cmd "%completion-emit!" completion::completion_emit(id: SteelVal, items: SteelVal, incomplete: SteelVal, span: SteelVal, prefix: SteelVal);
        cmd "completion-top" completion::completion_top(n: SteelVal);
        cmd "completion-accept!" completion::completion_accept(idx: SteelVal);
        cmd "completion-dismiss!" completion::completion_dismiss();
        open "set-completion-triggers!" completion::set_completion_triggers(source: SteelVal, language: SteelVal, chars: SteelVal);
        open "set-attachment-completion-triggers!" completion::set_attachment_completion_triggers(source: SteelVal, pane: args::LivePane, server: SteelVal, feature: SteelVal, chars: SteelVal);

        // Cursor-anchored popup widget.
        cmd "%show-popup!" ui::show_popup(pane: args::LivePane, text: SteelVal, anchor: SteelVal, kind: SteelVal, lang: SteelVal);
        cmd "close-popup!" ui::close_popup(token: SteelVal);

        // Selection menu widget.
        cmd "show-menu!" ui::show_menu(pane: args::LivePane, items: SteelVal, on_select: SteelVal);
        cmd "close-menu!" ui::close_menu(token: SteelVal);

        // Bottom drawer.
        cmd "%show-drawer-list!" ui::show_drawer_list(pane: args::LivePane, items: SteelVal, on_select: SteelVal, selected: SteelVal, render: SteelVal);
        cmd "close-drawer!" ui::close_drawer(token: SteelVal);
        cmd "%update-drawer-list!" ui::update_drawer_list(token: SteelVal, items: SteelVal, on_select: SteelVal, selected: SteelVal, render: SteelVal);
        cmd "drawer-selected-index" ui::drawer_selected_index(token: SteelVal);

        // Fuzzy-picker widget.
        cmd "%picker!" ui::picker(pane: args::LivePane, items: SteelVal, on_select: SteelVal, prompt: SteelVal, pending: SteelVal, query: SteelVal, truncate: SteelVal, actions: SteelVal);
        cmd "%live-picker!" ui::live_picker(pane: args::LivePane, on_select: SteelVal, prompt: SteelVal, query: SteelVal, on_query_change: SteelVal, truncate: SteelVal, actions: SteelVal);
        cmd "picker-push!" ui::picker_push(token: SteelVal, items: SteelVal);
        cmd "picker-replace!" ui::picker_replace(token: SteelVal, items: SteelVal);
        cmd "%picker-source-spawn!" ui::picker_source_spawn(token: SteelVal, cmd: SteelVal, args: SteelVal, cwd: SteelVal, nul: SteelVal, ok_exit_codes: SteelVal);
        cmd "picker-source-stop!" ui::picker_source_stop(token: SteelVal);
        cmd "picker-close!" ui::picker_close(token: SteelVal);
        // Backs the #:source and argv-for checks only. See args::is_callable's doc.
        plain "%callable?" args::is_callable(val: SteelVal);

        cmd "%held-done" hooks::held_done(hold: SteelVal);

        // Timers: not LSP-specific, any plugin can schedule one.
        cmd "after!" timers::after(ms: SteelVal, thunk: SteelVal);
        cmd "cancel-timer!" timers::cancel_timer(id: SteelVal);

        // Generic async subprocess execution: one-shot capture, not a
        // streaming source (that's `picker-source-spawn!`'s shape).
        cmd "%spawn-async!" process::spawn_async(cmd: SteelVal, args: SteelVal, cwd: SteelVal, callback: SteelVal);
        cmd "cancel-async!" process::cancel_async(id: SteelVal);

        // Blocking subprocess capture, no callback. Backs `stdlib/run!`.
        // `open`, not `cmd`: Steel's own `spawn-process`/`wait`, which
        // `run_capture` stands in for (see its own doc), carry no legality
        // gate either, and `stdlib/run!` is a plain helper any plugin body can
        // reach, not a top-level dispatched command.
        open "%run-capture!" process::run_capture(cmd: SteelVal, args: SteelVal, cwd: SteelVal);

        cmd "diff-lines" diff::diff_lines(old: SteelVal, new: SteelVal);
        cmd "diff-buffer-lines" diff::diff_buffer_lines(pane: args::LivePane, ref_text: SteelVal);
        cmd "diff-words" diff::diff_words(old: SteelVal, new: SteelVal);
        cmd "buffer-revision-diff" diff::buffer_revision_diff(pane: args::LivePane, revision: args::Usize);
        open "language-has-grammar?" syntax::language_has_grammar(name: SteelVal);

        // Editor-integration directory info, read from `ctx.dirs` (computed
        // once by `ScriptingHost::new`). Callable from anywhere (`open`).
        open "data-dir" fs::data_dir();
        open "runtime-dir" fs::runtime_dir();
        open "cwd" fs::cwd();
        cmd "set-cwd!" fs::set_cwd(path: String);
        open "path-entries" fs::path_entries(dir: String, prefix: String);
        open "path-programs" fs::path_programs(prefix: String, extensions: SteelVal);
        open "%which" fs::which_program(name: String);
        plain "home-dir" fs::home_dir();
    }

    // Context-free builtins that don't fit the typed-arity table above: raw
    // `&[SteelVal]` FuncV, no SteelCtx. `path-join` is a pure string helper;
    // `hume-version` reads build info, not directory state; `json-ref`/
    // `json-contains?`/`json-ref-or` take a variadic path (`j seg ...`),
    // which the typed-arity table's fixed parameter list can't express.
    steel.register_value("hume-version", SteelVal::FuncV(version::hume_version));
    steel.register_value(
        "hume-version>=?",
        SteelVal::FuncV(version::hume_version_at_least),
    );
    steel.register_value("path-join", SteelVal::FuncV(fs::path_join));
    steel.register_value("path->display", SteelVal::FuncV(fs::path_to_display));
    steel.register_value("safe-path-segment?", SteelVal::FuncV(fs::safe_path_segment));
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

    // PRELUDE_SHIMS must compile as its OWN program, separate from
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
        .compile_and_run_raw_program(PRELUDE_SHIMS.to_owned())
        .expect("HUME scripting prelude shims failed: this is a bug");

    // Append the same shims to steel-core's prelude string. The prelude is
    // prepended to every `(require "path.scm")` compilation unit (steel-core
    // internals; see io.rs's module doc), so a plugin's own print and `which`
    // calls resolve to HUME's shims instead of steel-core's originals. Unlike the top-level case above, a required module's
    // import-then-body-define compiles as one unit without conflict (the
    // shim's `define` simply overwrites the mangled slot the prelude import
    // created moments earlier), so no splitting is needed here.
    steel.set_prelude_string(Cow::Owned(format!(
        "{}{PRELUDE_SHIMS}",
        steel::compiler::modules::PRELUDE_STRING
    )));
}

#[cfg(test)]
mod tests;
