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

use steel::rerrs::SteelErr;
use steel::rvals::{FromSteelVal, IntoSteelVal, SteelVal};
use steel::steel_vm::engine::Engine;
use steel::steel_vm::register_fn::RegisterFn;

use super::HUME_CTX;

/// The return type of every ctx-taking builtin below. One definition shared
/// by every `builtins/*.rs` submodule instead of each declaring its own
/// identical local alias.
// steel-val-transient: a builtin's return value, handed straight back to the VM.
pub(crate) type SteelResult = Result<SteelVal, SteelErr>;

// ── Declarative registration table ───────────────────────────────────────────

/// Declarative builtin-registration table. Each entry is
/// `<kind> "<steel-name>" [=> "<name>"] <rust-path>(<arg>: <Type>, …);`
/// where `<kind>` is:
/// - `cmd`: ctx-taking, gated by [`errors::require_cmd`] (buffer/pane/editor-state builtins)
/// - `config`: ctx-taking, gated by [`errors::require_config`] (init/plugin-load-only verbs)
/// - `open`: ctx-taking, ungated (no legality gate, or a bespoke one the fn checks itself)
/// - `plain`: no `&mut SteelCtx` param at all (context-free predicates)
/// - `rest`: no `&mut SteelCtx`, the raw `&[SteelVal]` of a variadic builtin
///   (`path-join`, `json-ref`), declared with no argument list
///
/// Every error a builtin's argument decode, resolve or body raises starts
/// with its name ([`errors::named`]), the only place that name enters such a
/// message. A wrong argument count never reaches the builtin: Steel refuses
/// it with its own `ArityMismatch`, which starts with the registered name
/// and no `: `. A `%` internal
/// declares after `=>` the name a plugin calls to reach it; one reached through more than
/// one public function names itself. An entry without `=>` must not be a
/// `%` internal, which a compile-time assertion checks.
///
/// The declared arg types are compiler-checked, not documentation: each entry
/// decodes its arguments into those types and passes them on, so a
/// mismatch against the real function's signature is a compile error.
/// Decoding runs inside the funnel, so a type error names the builtin too.
///
/// Every ctx-taking kind (`cmd`/`config`/`open`) runs its gate, then
/// decodes every argument, then runs each argument through
/// `args::BuiltinArg::resolve`, shadowing `$arg` with its `Out` type: the
/// seam where a pane argument type (`args::pane`) checks that its buffer is
/// open; most other declared types resolve to themselves. Every argument
/// decodes before any resolves, so a malformed typed argument is reported
/// ahead of a closed buffer. An argument declared `SteelVal` is parsed by
/// the body, after a closed buffer and before the host checks a shown or
/// focused pane. `plain` (no ctx) never runs the resolve step, so it can't
/// declare a checked pane argument.
///
/// The table defines `register_table`, which registers every entry, and
/// for tests `pane_needs`: each builtin with a pane-checking argument type
/// and the most demanding [`args::PaneNeed`] among its arguments. Only a
/// `cmd` entry may declare one, which a compile-time assertion checks: no
/// init context has a pane value to pass.
macro_rules! builtins {
    (@display $name:literal) => {{
        const _: () = assert!(
            $name.as_bytes()[0] != b'%',
            concat!($name, " is internal: declare the name a plugin calls after `=>`")
        );
        $name
    }};
    (@display $name:literal, $display:literal) => {
        $display
    };
    (@ctx $gate:ident, $steel:expr, $name:literal, $display:expr, $($modpath:ident)::+, ($($arg:ident : $ty:ty),*)) => {
        $steel.register_fn_with_ctx(
            HUME_CTX,
            $name,
            move |ctx: &mut crate::context::SteelCtx $(, $arg: SteelVal)*| {
                errors::named($display, || {
                    builtins!(@gate $gate, ctx);
                    $(let $arg = <$ty as FromSteelVal>::from_steelval(&$arg)?;)*
                    $(let $arg = args::BuiltinArg::resolve($arg, ctx, stringify!($arg))?;)*
                    $($modpath)::+(ctx $(, $arg)*).into_steelval()
                })
            },
        );
    };
    (@gate cmd, $ctx:ident) => { errors::require_cmd($ctx)? };
    (@gate config, $ctx:ident) => { errors::require_config($ctx)? };
    (@gate open, $ctx:ident) => {};
    (@one cmd, $($rest:tt)*) => { builtins!(@ctx cmd, $($rest)*) };
    (@one config, $($rest:tt)*) => { builtins!(@ctx config, $($rest)*) };
    (@one open, $($rest:tt)*) => { builtins!(@ctx open, $($rest)*) };
    (@one plain, $steel:expr, $name:literal, $display:expr, $($modpath:ident)::+, ($($arg:ident : $ty:ty),*)) => {
        $steel.register_fn(
            $name,
            move |$($arg: SteelVal),*| {
                errors::named($display, || {
                    $(let $arg = <$ty as FromSteelVal>::from_steelval(&$arg)?;)*
                    $($modpath)::+($($arg),*).into_steelval()
                })
            },
        );
    };
    (@one rest, $steel:expr, $name:literal, $display:expr, $($modpath:ident)::+, ()) => {{
        fn named_rest(args: &[SteelVal]) -> SteelResult {
            errors::named($display, || $($modpath)::+(args))
        }
        $steel.register_value($name, SteelVal::FuncV(named_rest));
    }};
    (@pane_gate cmd, $name:literal, ($($ty:ty),*)) => {};
    (@pane_gate $kind:ident, $name:literal, ($($ty:ty),*)) => {
        const _: () = {
            $(assert!(
                <$ty as args::BuiltinArg>::PANE_NEED.is_none(),
                concat!($name, " takes a pane, which only a `cmd` entry may")
            );)*
        };
    };
    (@need cmd, ($($ty:ty),*)) => {
        <[Option<args::PaneNeed>]>::iter(&[$(<$ty as args::BuiltinArg>::PANE_NEED),*])
            .flatten()
            .max()
            .copied()
    };
    (@need $kind:ident, ($($ty:ty),*)) => {
        None
    };
    ($($kind:ident $name:literal $(=> $display:literal)? $($modpath:ident)::+ $(( $($arg:ident : $ty:ty),* $(,)? ))? ;)*) => {
        fn register_table(steel: &mut Engine) {
            $(
                builtins!(
                    @one $kind,
                    steel,
                    $name,
                    builtins!(@display $name $(, $display)?),
                    $($modpath)::+,
                    ($($($arg : $ty),*)?)
                );
            )*
        }

        $(
            builtins!(@pane_gate $kind, $name, ($($($ty),*)?));
        )*

        #[cfg(any(test, feature = "test-util"))]
        pub(crate) fn pane_needs() -> Vec<(&'static str, args::PaneNeed)> {
            let needs = [$((
                builtins!(@display $name $(, $display)?),
                builtins!(@need $kind, ($($($ty),*)?)),
            )),*];
            needs
                .into_iter()
                .filter_map(|(name, need)| Some((name, need?)))
                .collect()
        }
    };
}

// ── Bootstrap Scheme ──────────────────────────────────────────────────────────

/// Scheme bootstrap evaluated once during Steel engine init: keyword wrappers
/// over the Rust builtins, plugin activation, and in-VM command dispatch.
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
// aborts the enclosing eval. A body stopped by the budget never reaches the
// handler: the session ending closes its activation instead
// (abandon_open_activations), and the whole eval fails.
//
// load-plugin!: with a manifest.scm, evaluates it under the same handler
// contract, %finish-manifest-load! recording any error.
//
// define-typed-command!: the `:` counterpart of define-command!, sharing its
// name rule (CommandProc::check_define) and bookkeeping but registering
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
// builtin has no access to the Engine it runs in. The handler returns #f. A
// macro name (`call!`) fails to evaluate as a value and reads as absent.
//
// %apply-command!: applies the proc %enter-command! returned inside the call!
// frame it entered, so the command runs as the code that defined it, and arms
// the #:inline-output alt-screen bracket for call!, as call_steel_cmd and
// Editor::call_steel_command_body do for keypress and `:`. Each restore
// truncates to its own depth instead of popping, so an unpaired descendant
// frame is never taken for this call's own. dynamic-wind runs the restores
// when the body raises too, so a caller that catches the raise goes on as
// itself and with its own bracket state. A session stopped by its budget
// runs neither; the session's end drops both.
//
// %call-held-handler: starts a held event's handler that takes `done`, with
// the event's args followed by a thunk that releases hold `hold`.
// ScriptingHost::fire_held_hook calls every such handler through it.
//
// lsp-request!: callback is (lambda (err result)), one of them non-#f; err is
// (hash 'kind 'message ['code]).
// lsp-request-all!: callback is (lambda (err results)), results one hash per
// server. the method, #:feature and #:to choose the servers when the request is
// sent. #:supersede <key>, a key from lsp-supersede-key, cancels the pending
// request under the same key. #:require-focus drops the callback unless pane is still focused
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
// handler; the seed call for a non-empty #:query runs on the caller's stack,
// so its raise reaches the caller directly.
//
// A live session mints a new token for every query and passes it to
// #:source. picker-push!, picker-replace! and picker-source-spawn! accept
// only the current one, so an answer to an earlier query is dropped by the
// editor. picker-close! and picker-source-stop! also accept the token
// live-picker! returned.
//
// run-inline-output!: %run-inline-output! raises outside an #:inline-output
// command and unless the command exits 0, and spawns it in the editor's
// process group so a Ctrl-c reaches it (see
// hume-platform::process::run_inline_output).
//
// stems-with-suffix: sorts after stripping the suffix, since sorting full
// file names would put "a-b.x" ahead of "a.x" ('-' < '.').
//
// read-data-file: the one reader of a data file, returning its single
// top-level form. It parses with hm.read!, never `read`: steel-core's `read`
// keeps one buffer across calls, returns eof instead of raising on bad
// syntax, and leaves the bad text for the next call. The parse error is
// raised again with the file's path in front.
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

    register_table(steel);

    // Evaluate the Scheme bootstrap (defines `load-plugin!` and the rest of
    // HUME's Scheme-side API). Runs before any user init.scm; HUME_CTX is not
    // yet set but the bootstrap only uses `define`, so no builtins are
    // called at this point.
    steel
        .compile_and_run_raw_program(BOOTSTRAP.to_owned())
        .expect("HUME scripting bootstrap failed: this is a bug");
}

// A name ends in `!` when calling it mutates editor state, registers
// something, spawns a process, or writes to disk. Reads and closure
// constructors (`debounce`) carry no `!`.
builtins! {
    // Config / settings
    open "set-option!" settings::set_option(key: String, value: SteelVal);
    cmd  "set-buffer-option!" settings::set_buffer_option(pane: args::LiveBuffer, key: String, value: SteelVal);
    open "get-option" settings::get_option(key: String);
    cmd  "get-buffer-option" settings::get_buffer_option(pane: args::LiveBuffer, key: String);

    // Keymap
    open   "%bind-key!" => "bind-key!" keymap_bind::bind_key(mode: SteelVal, key_str: String, cmd_name: String, keymap: SteelVal);
    open   "%bind-key-extend!" => "bind-key-extend!" keymap_bind::bind_key_extend(mode: SteelVal, key_str: String, cmd_name: String, keymap: SteelVal);
    open   "%unbind-key!" => "unbind-key!" keymap_bind::unbind_key(mode: SteelVal, key_str: String, keymap: SteelVal);
    cmd    "attach-keymap!" keymap_bind::attach_keymap(pane: args::LiveBuffer, name: SteelVal);
    cmd    "detach-keymap!" keymap_bind::detach_keymap(pane: args::LiveBuffer, name: SteelVal);
    cmd    "set-register-prefix!" commands::set_register_prefix(name: String);

    // Registers: direct read/write, independent of set-register-prefix!'s
    // per-command targeting.
    open "read-register" registers::read_register(name: String);
    open "write-register!" registers::write_register(name: String, values: SteelVal);
    open "register-name?" registers::register_name(name: String);

    // Plugin lifecycle
    open "%declare-plugin!" => "declare-plugin!" plugins::declare_plugin(name: String, entry: SteelVal, commands: SteelVal, typed_commands: SteelVal, events: SteelVal, languages: SteelVal);
    open "resolve-plugin-path" plugins::resolve_plugin_path(name: String);

    // Plugin introspection and explicit activation
    open "loaded-plugins" plugins::loaded_plugins();
    open "declared-plugins" plugins::declared_plugins();
    open "declared-plugin-ref" plugins::declared_plugin_ref(name: String);
    open "plugin-config" plugins::plugin_config();
    open "plugin-dir" plugins::plugin_dir();
    open "%load-plugin!" => "load-plugin!" plugins::load_plugin(name: String, config: SteelVal, git_ref: SteelVal);

    // Inline activation primitives, called from the %activate-plugin-inline!
    // Scheme helper to drive mid-eval plugin loading without &mut Engine.
    open "%begin-lazy-activation!" => "%begin-lazy-activation!" plugins::activation::begin_lazy_activation(plugin: String, entry: SteelVal);
    open "%finish-lazy-activation!" => "%finish-lazy-activation!" plugins::activation::finish_lazy_activation(error: SteelVal);
    open "%entry-loaded?" => "call!" plugins::activation::entry_loaded(plugin: String, entry: SteelVal);
    open "%lazy-command-owner" => "call!" plugins::activation::lazy_command_owner(name: String);

    // Manifest evaluation: %load-plugin! returns the require string of
    // <plugin-dir>/manifest.scm, and the wrapper reports the outcome here.
    open "%finish-manifest-load!" => "load-plugin!" plugins::activation::finish_manifest_load(name: String, error: SteelVal);

    // Hook registration (init-only)
    config "register-hook!" hooks::register_hook(name: SteelVal, proc: SteelVal);

    // Steel command definition. %define-command! is the native primitive;
    // the (define-command! …) Steel wrapper in BOOTSTRAP exposes keyword
    // args (#:repeatable, #:inline-output).
    config "%define-command!" => "define-command!" commands::define_command(name: String, doc: String, proc: SteelVal, repeatable: bool, inline_output: bool);
    // Typed (`:` command line) counterpart. See the module-doc paragraph
    // above BOOTSTRAP. No #:repeatable keyword arg.
    config "%define-typed-command!" => "define-typed-command!" commands::define_typed_command(name: String, doc: String, proc: SteelVal, inline_output: bool, completer: SteelVal);
    // %call-native! is the Rust leaf for native/unknown dispatch; the variadic
    // (call! name args…) macro desugars to (%dispatch-command! …) which routes
    // activated plugin commands inline in Steel and falls back here for everything else.
    open "%call-native!" => "call!" commands::call_command_primitive(name: String, args: SteelVal);
    // %enter-command!/%leave-command!: %enter-command! enters a defined Steel
    // command's call! frame and returns (proc depth), or #f, which
    // %dispatch-command! reads to decide inline-apply vs. %call-native!; the
    // pair is the call!-path counterpart of call_steel_cmd running a command
    // as its definer.
    open "%enter-command!" => "call!" commands::enter_command(name: String);
    open "%leave-command!" => "call!" commands::leave_command(depth: SteelVal);
    // %arm-inline-output!/%restore-inline-output!: the call!-path counterpart
    // of dispatch.rs's own inline-output arm/close, wrapping %dispatch-command!'s
    // in-VM apply. See the BOOTSTRAP comment block above for the full picture.
    open "%arm-inline-output!" => "call!" commands::arm_inline_output(name: String);
    open "%restore-inline-output!" => "call!" commands::restore_inline_output(depth: SteelVal);
    open "command-plugin" commands::command_plugin(name: String);
    open "%command-callable?" => "command-exists?" commands::command_callable(name: String);

    // Grammar compilation: sandbox-free, full-trust plugin model. A Rust
    // builtin for the reasons in grammar.rs's module doc;
    // `grammar-output-path` is plain Scheme.
    open "compile-grammar!" grammar::compile_grammar(src: String, out: String);

    // Inline-output subprocess: process-group-isolated spawn for
    // `#:inline-output` commands, sandbox-free like Steel's own stdlib.
    open  "%run-inline-output!" => "run-inline-output!" process::run_inline_output(cmd: String, args_val: SteelVal, cwd_val: SteelVal, env_val: SteelVal);

    // Logging: push messages to the editor message log
    open "log!" crate::log::log_msg(severity: SteelVal, message: String);

    // print!: a line to the terminal, inside an #:inline-output command.
    open "print!" io::print(text: String);

    // Opaque pane predicate: context-free, no SteelCtx needed. Equality
    // is `equal?` (Custom::equality_hint), not a dedicated builtin.
    plain "pane?" ids::is_pane(val: SteelVal);
    plain "json-parse" json::json_parse(s: SteelVal);
    // Readers of one `buffer-selections` entry.
    plain "selection-anchor" buffers::selection_anchor(sel: SteelVal);
    plain "selection-head" buffers::selection_head(sel: SteelVal);
    plain "selection-start-char" buffers::selection_start_char(sel: SteelVal);
    plain "selection-end-char" buffers::selection_end_char(sel: SteelVal);
    plain "selection-primary?" buffers::selection_primary(sel: SteelVal);
    // json-list and the two predicates are fixed-arity; json-ref,
    // json-contains? and json-ref-or take a variadic path and are
    // `rest` entries below.
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
    cmd "buffer-panes" buffers::buffer_panes(pane: args::LiveBuffer);
    plain "buffer-key" buffers::buffer_key(pane: args::ArgPane);
    cmd "buffer-path" buffers::buffer_path(pane: args::LiveBuffer);
    cmd "%buffer-display-path" => "buffer-display-path" buffers::buffer_display_path(pane: args::LiveBuffer, relative: SteelVal);
    cmd "buffer-name" buffers::buffer_name(pane: args::LiveBuffer);
    cmd "buffer-dirty?" buffers::buffer_dirty(pane: args::LiveBuffer);
    cmd "buffer-text" buffers::buffer_text(pane: args::LiveBuffer);
    cmd "buffer-line-count" buffers::buffer_line_count(pane: args::LiveBuffer);
    cmd "buffer-char-count" buffers::buffer_char_count(pane: args::LiveBuffer);
    cmd "%buffer-lines" => "buffer-lines" buffers::buffer_lines(pane: args::LiveBuffer, range: SteelVal);
    // Live cursor read: reflects synchronous edits in the same eval.
    cmd "buffer-cursor-line" buffers::buffer_cursor_line(pane: args::ShownPane);
    cmd "buffer-selections" buffers::buffer_selections(pane: args::ShownPane);
    cmd "char-offset->line" buffers::char_offset_to_line(pane: args::LiveBuffer, idx: args::Usize);
    cmd "line->char-offset" buffers::line_to_char_offset(pane: args::LiveBuffer, line: args::Usize);

    // Multi-buffer mutating builtins
    cmd "open-buffer!" buffers::open_buffer(path: args::ScriptPath);
    cmd "%close-buffer!" => "close-buffer!" buffers::close_buffer(pane: args::LiveBuffer, discard_unsaved: SteelVal);
    cmd "switch-to-buffer!" buffers::switch_to_buffer(pane: args::ShownPane, target: args::LiveBuffer);
    cmd "show-read-only-buffer!" buffers::show_read_only_buffer(pane: args::ShownPane, name: String, text: String);

    // Language identity and grammar builtins
    config "%define-language!" => "define-language!" syntax::define_language(name: SteelVal, exts_val: SteelVal, globs_val: SteelVal, shebangs_val: SteelVal);
    config "%configure-language!" => "configure-language!" syntax::configure_language(name: SteelVal, fields_val: SteelVal);
    open   "%register-grammar!" => "register-grammar!" syntax::register_grammar(name: SteelVal, grammar_path: SteelVal, symbol: SteelVal, highlights_path: SteelVal, optional_queries: SteelVal);
    plain  "grammar-optional-queries" syntax::grammar_optional_queries();

    // LSP server registration: ranked by whose behalf it is made, last-wins
    // within a rank, queued (like language regs) and applied at the end of
    // the current eval, from init, plugin activation, or a command/hook body.
    open "%register-lsp-server!" => "register-lsp-server!" lsp::register_lsp_server(name: SteelVal, command: SteelVal, args_val: SteelVal, init_options: SteelVal, settings: SteelVal, env_val: SteelVal);
    open "unregister-lsp-server!" lsp::unregister_lsp_server(name: SteelVal);
    open "set-language-servers!" lsp::set_language_servers(language: SteelVal, entries: SteelVal);
    open "set-default-language-servers!" lsp::set_default_language_servers(language: SteelVal, entries: SteelVal);
    open "lsp-language-servers" lsp::lsp_language_servers(language: SteelVal);
    // Lifecycle: stop/restart a running server, or open the status view.
    cmd "lsp-stop!" lsp::lsp_stop(target: lsp::LspTargetArg);
    cmd "lsp-restart!" lsp::lsp_restart(target: lsp::LspTargetArg);
    cmd "lsp-show-status!" lsp::lsp_show_status(pane: args::FocusedPane);
    // Generic LSP bridge: any protocol method reachable from Steel.
    cmd "%lsp-request!" => "lsp-request!" lsp::lsp_request(pane: args::RequesterPane, method: SteelVal, params: SteelVal, callback: args::ScriptProc<2>, feature: SteelVal, to: SteelVal, unavailable: SteelVal, allow_stale: SteelVal, supersede: SteelVal, require_focus: SteelVal, tracked: SteelVal);
    cmd "%lsp-request-all!" => "lsp-request-all!" lsp::lsp_request_all(pane: args::RequesterPane, method: SteelVal, params: SteelVal, callback: args::ScriptProc<2>, feature: SteelVal, unavailable: SteelVal, allow_stale: SteelVal, supersede: SteelVal, require_focus: SteelVal, tracked: SteelVal);
    cmd "%lsp-notify!" => "lsp-notify!" lsp::lsp_notify(pane: args::LiveBuffer, method: SteelVal, params: SteelVal, feature: SteelVal, to: SteelVal);
    config "register-lsp-notification-hook!" lsp::register_lsp_notification_hook(methods: SteelVal, proc: SteelVal);
    // Introspection
    cmd  "lsp-capabilities" lsp::lsp_capabilities(server: SteelVal);
    cmd  "%lsp-capability" => "lsp-capability" lsp::lsp_capability(server: SteelVal, feature: SteelVal, method: SteelVal);
    cmd  "%lsp-servers" => "lsp-servers" lsp::lsp_servers(pane: args::LiveBuffer, feature: SteelVal, method: SteelVal);
    plain "lsp-server-name" lsp::lsp_server_name(server: SteelVal);
    cmd  "lsp-server-status" lsp::lsp_server_status();
    open "lsp-server-registered?" lsp::lsp_server_registered(name: SteelVal);
    open "lsp-supersede-key" lsp::lsp_supersede_key();
    cmd "lsp-document-params" lsp::lsp_document_params(pane: args::LiveBuffer);
    cmd "lsp-position-params" lsp::lsp_position_params(pane: args::ShownPane);
    cmd "track-position!" lsp::track_position(pane: args::ShownPane);
    cmd "tracked-position-params" lsp::tracked_position_params(token: SteelVal);
    cmd "untrack-position!" lsp::untrack_position(token: SteelVal);
    cmd "keep-tracked-position!" lsp::keep_tracked_position(token: SteelVal);
    cmd "lsp-primary-range-params" lsp::lsp_primary_range_params(pane: args::ShownPane);
    cmd "lsp-linewise-ranges-params" lsp::lsp_linewise_ranges_params(pane: args::ShownPane);
    cmd "lsp-position->char-offset" lsp::lsp_position_to_char_offset(pane: args::LiveBuffer, position: SteelVal);
    cmd "lsp-range->char-range" lsp::lsp_range_to_char_range(pane: args::LiveBuffer, range: SteelVal);
    plain "lsp-label-offsets->text" lsp::lsp_label_offsets_to_text(label: SteelVal, offsets: SteelVal);
    plain "lsp-location" lsp::lsp_location(uri: SteelVal, range: SteelVal);
    cmd "lsp-locations->places" lsp::lsp_locations_to_places(locs: SteelVal);
    cmd "lsp-locations->merged-places" lsp::lsp_locations_to_merged_places(locs: SteelVal);
    cmd "viewport-range" buffers::viewport_range(pane: args::ShownPane);
    cmd "buffer-generation" buffers::buffer_generation(pane: args::LiveBuffer);
    cmd "buffer-search" buffers::buffer_search(pane: args::LiveBuffer, pattern: String);
    cmd "word-search-pattern" buffers::word_search_pattern(pane: args::LiveBuffer, word: String);
    cmd "buffer-undo-tree" buffers::buffer_undo_tree(pane: args::LiveBuffer);
    cmd "buffer-live?" buffers::buffer_live(pane: args::ArgPane);
    cmd "pane-live?" buffers::pane_live(pane: args::ArgPane);
    open "set-hook-triggers!" completion::set_hook_triggers(language: SteelVal, source: args::SourceName, chars: SteelVal);
    cmd "set-attachment-hook-triggers!" completion::set_attachment_hook_triggers(pane: args::LiveBuffer, source: args::SourceName, server: SteelVal, feature: SteelVal, chars: SteelVal);

    // Decoration stores + diagnostics pull.
    cmd "set-inlay-hints!" decorations::set_inlay_hints(pane: args::LiveBuffer, source: args::SourceName, hints: SteelVal);
    cmd "register-sign-source!" decorations::register_sign_source(pane: args::LiveBuffer, source: args::SourceName, priority: SteelVal);
    cmd "set-signs!" decorations::set_signs(pane: args::LiveBuffer, source: args::SourceName, signs: SteelVal);
    cmd "set-virtual-lines!" decorations::set_virtual_lines(pane: args::LiveBuffer, source: args::SourceName, lines: SteelVal);
    cmd "%set-eol-text!" => "set-eol-text!" decorations::set_eol_text(pane: args::LiveBuffer, source: args::SourceName, lines: SteelVal, hide_on_insert_line: SteelVal);
    cmd "set-extra-highlights!" decorations::set_extra_highlights(pane: args::LiveBuffer, source: args::SourceName, spans: SteelVal);
    cmd "set-line-backgrounds!" decorations::set_line_backgrounds(pane: args::LiveBuffer, source: args::SourceName, entries: SteelVal);
    cmd "set-statusline-text!" decorations::set_statusline_text(pane: args::LiveBuffer, source: args::SourceName, text: SteelVal);
    cmd "%diagnostics-for-buffer" => "diagnostics-for-buffer" decorations::diagnostics_for_buffer(pane: args::ArgPane, severity: SteelVal, range: SteelVal);
    cmd "%diagnostic-counts" => "diagnostic-counts" decorations::diagnostic_counts(pane: args::ArgPane, severity: SteelVal);

    // Edit + navigation primitives.
    cmd "%apply-text-edits!" => "apply-text-edits!" edits::apply_text_edits(pane: args::RequesterPane, edits: SteelVal, expect_gen: SteelVal);
    cmd "%apply-workspace-edit!" => "apply-workspace-edit!" edits::apply_workspace_edit(pane: args::RequesterPane, wsedit: SteelVal, expect_gen: SteelVal);
    cmd "goto-location!" edits::goto_location(pane: args::ShownPane, loc: SteelVal);
    cmd "goto-revision!" edits::goto_revision(pane: args::ShownPane, revision: args::Usize);
    cmd "insert-key!" edits::insert_key(pane: args::FocusedPane, key: SteelVal);
    cmd "paste-text!" edits::paste_text(pane: args::FocusedPane, text: SteelVal);
    cmd "%edit-text!" => "edit-text!" edits::edit_text(pane: args::RequesterPane, edits: SteelVal, expect_gen: SteelVal);
    cmd "selections-linewise?" buffers::selections_linewise(pane: args::ShownPane);
    cmd "selections-charwise?" buffers::selections_charwise(pane: args::ShownPane);

    // Minibuffer prompt.
    cmd "%prompt!" => "prompt!" ui::prompt(pane: args::FocusedPane, label: SteelVal, prefill: SteelVal, on_confirm: args::ScriptProc<1>);
    cmd "on-next-char!" ui::on_next_char(callback: args::ScriptProc<1>);
    cmd "symbol-under-cursor" buffers::symbol_under_cursor(pane: args::ShownPane);

    // Completion: sources register at config time; answers, the ranked
    // view, and accept/dismiss are command-time.
    config "%register-completion-source!" => "register-completion-source!" completion::register_completion_source(name: args::SourceName, proc: args::ScriptProc<3>, target: SteelVal, match_kind: SteelVal, priority: SteelVal, resolve: SteelVal, token_chars: SteelVal, cancel: SteelVal);
    cmd "%completion-emit!" => "completion-emit!" completion::completion_emit(id: SteelVal, items: SteelVal, incomplete: SteelVal, more: SteelVal, span: SteelVal, prefix: SteelVal);
    cmd "completion-top" completion::completion_top(n: SteelVal);
    cmd "completion-accept!" completion::completion_accept(idx: SteelVal);
    cmd "completion-dismiss!" completion::completion_dismiss();
    open "set-completion-triggers!" completion::set_completion_triggers(language: SteelVal, source: args::SourceName, chars: SteelVal);
    cmd "set-attachment-completion-triggers!" completion::set_attachment_completion_triggers(pane: args::LiveBuffer, source: args::SourceName, server: SteelVal, feature: SteelVal, chars: SteelVal);

    // Cursor-anchored popup widget.
    cmd "%show-popup!" => "show-popup!" ui::show_popup(pane: args::FocusedPane, text: SteelVal, anchor: SteelVal, kind: SteelVal, lang: SteelVal);
    cmd "close-popup!" ui::close_popup(token: SteelVal);

    // Selection menu widget.
    cmd "show-menu!" ui::show_menu(pane: args::FocusedPane, items: SteelVal, on_select: args::ScriptProc<1>);
    cmd "close-menu!" ui::close_menu(token: SteelVal);

    // Bottom drawer.
    cmd "%show-drawer-list!" => "show-drawer-list!" ui::show_drawer_list(pane: args::FocusedPane, items: SteelVal, on_select: args::ScriptProc<2>, selected: SteelVal, render: SteelVal);
    cmd "close-drawer!" ui::close_drawer(token: SteelVal);
    cmd "%update-drawer-list!" => "update-drawer-list!" ui::update_drawer_list(token: SteelVal, items: SteelVal, on_select: args::ScriptProc<2>, selected: SteelVal, render: SteelVal);
    cmd "drawer-selected-index" ui::drawer_selected_index(token: SteelVal);

    // Fuzzy-picker widget.
    cmd "%picker!" => "picker!" ui::picker(pane: args::FocusedPane, items: SteelVal, on_select: args::ScriptProc<1>, prompt: SteelVal, pending: SteelVal, query: SteelVal, truncate: SteelVal, actions: SteelVal);
    cmd "%live-picker!" => "live-picker!" ui::live_picker(pane: args::FocusedPane, on_select: args::ScriptProc<1>, prompt: SteelVal, query: SteelVal, on_query_change: SteelVal, truncate: SteelVal, actions: SteelVal);
    cmd "picker-push!" ui::picker_push(token: SteelVal, items: SteelVal);
    cmd "picker-replace!" ui::picker_replace(token: SteelVal, items: SteelVal);
    cmd "%picker-source-spawn!" => "picker-source-spawn!" ui::picker_source_spawn(token: SteelVal, cmd: SteelVal, args: SteelVal, cwd: SteelVal, nul: SteelVal, ok_exit_codes: SteelVal);
    cmd "picker-source-stop!" ui::picker_source_stop(token: SteelVal);
    cmd "picker-close!" ui::picker_close(token: SteelVal);
    // Backs the #:source and argv-for checks only. See args::is_callable's doc.
    plain "%callable?" => "%callable?" args::is_callable(val: SteelVal, args: SteelVal);

    cmd "%held-done" => "done" hooks::held_done(hold: SteelVal);

    // Timers: not LSP-specific, any plugin can schedule one.
    cmd "after!" timers::after(ms: SteelVal, thunk: args::ScriptProc<0>);
    cmd "cancel-timer!" timers::cancel_timer(id: SteelVal);

    // Background work with a callback, stopped by `cancel-async!`: a
    // subprocess's one-shot capture (a streaming picker source is
    // `picker-source-spawn!`'s shape) and a program scan's reports.
    cmd "%spawn-async!" => "spawn-async!" process::spawn_async(cmd: SteelVal, args: SteelVal, cwd: SteelVal, callback: args::ScriptProc<1>);
    cmd "cancel-async!" process::cancel_async(id: SteelVal);
    cmd "path-programs" fs::path_programs(prefix: String, extensions: SteelVal, on_names: args::ScriptProc<2>);

    // Blocking subprocess capture, no callback.
    // `open`, not `cmd`: Steel's own `spawn-process`/`wait`, which
    // `run_capture` stands in for (see its own doc), carry no legality
    // gate either, and `run-capture!` is a plain helper any plugin body
    // can reach, not a top-level dispatched command.
    open "%run-capture!" => "run-capture!" process::run_capture(cmd: SteelVal, args: SteelVal, cwd: SteelVal);

    plain "diff-lines" diff::diff_lines(old: SteelVal, new: SteelVal);
    cmd "diff-buffer-lines" diff::diff_buffer_lines(pane: args::LiveBuffer, ref_text: SteelVal);
    plain "diff-words" diff::diff_words(old: SteelVal, new: SteelVal);
    cmd "buffer-revision-diff" diff::buffer_revision_diff(pane: args::LiveBuffer, revision: args::Usize);
    cmd "buffer-changes-since" diff::buffer_changes_since(pane: args::LiveBuffer, generation: SteelVal);
    open "language-has-grammar?" syntax::language_has_grammar(name: SteelVal);

    // Editor-integration directory info, read from `ctx.dirs` (computed
    // once by `ScriptingHost::new`). Callable from anywhere (`open`).
    open "data-dir" fs::data_dir();
    open "runtime-dir" fs::runtime_dir();
    open "cwd" fs::cwd();
    // `open`: it touches `state.cwd` alone, which exists before init.scm
    // runs, so a config can change directory.
    open "set-cwd!" fs::set_cwd(path: args::ScriptPath);
    open "replace-dir!" fs::replace_dir(src: args::ScriptPath, dest: args::ScriptPath);
    open "remove-dir!" fs::remove_dir(dir: args::ScriptPath);
    open "first-path-outside" fs::first_path_outside(dir: args::ScriptPath, paths: SteelVal);
    open "try-lock-file!" fs::try_lock_file(path: args::ScriptPath);
    open "unlock-file!" fs::unlock_file(path: args::ScriptPath);
    open "path-entries" fs::path_entries(dir: args::ScriptPath, prefix: String);
    open "find-program" fs::find_program(name: String);
    plain "home-dir" fs::home_dir();

    // Build info of the running editor.
    plain "hume-version" version::hume_version();
    plain "hume-version>=?" version::hume_version_at_least(major: SteelVal, minor: SteelVal, patch: SteelVal);

    // Variadic, context-free: `path-join` is a pure string helper;
    // `json-ref`/`json-contains?`/`json-ref-or` take a variadic path
    // (`j seg ...`).
    rest "path-join" fs::path_join;
    rest "path->display" fs::path_to_display;
    rest "safe-path-segment?" fs::safe_path_segment;
    rest "json-ref" json::json_ref;
    rest "json-contains?" json::json_contains;
    rest "json-ref-or" json::json_ref_or;
}

#[cfg(test)]
mod tests;
