use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicI32;

use hume_engine::pipeline::{BufferId, EngineView, PaneId};

use self::registry::CommandRegistry;
use self::replay::{PendingRepeat, SelectionStep};
use crate::editor::buffer::lifecycle::CloseBufferRefused;
use crate::editor::buffer::store::BufferStore;
use crate::editor::buffer::{Buffer, OnUnsaved};
use crate::editor::pane_state::PaneView;
use crate::editor::pipe_run::PipeRunning;
use crate::editor::settings::EditorSettings;
use hume_editing::selection::SelectionSet;
use hume_ops::register::{KillRing, RegisterSet};
use hume_platform::path::AbsolutePath;
use hume_platform::wake::WakeCallback;
use hume_treesitter::parse_worker::ParseBackend;
use hume_treesitter::registry::LanguageRegistry;

use self::keymap::{Keymap, LocalKeymaps};

mod async_job;
mod async_source;
mod before_save;
mod decoration_providers;
mod edit_session;
mod error;
mod focus;
mod frame;
mod geometry;
mod host_impl;
mod inline_output;
mod input;
mod input_stack;
mod run;
pub(crate) use run::Driver;
mod reload;
mod scripting;
mod tui;

pub(crate) mod buffer;
mod clipboard;
mod commands;
mod completion;
mod cursor;
mod dispatch;
mod doc_ops;
pub(crate) mod event;
mod fuzzy;
mod host_token;
mod jump_list;
mod keymap;
pub(crate) mod lsp;
mod macros;
mod message_log;
mod minibuf;
mod mouse;
mod pane_state;
mod picker_source;
mod pipe_run;
pub(crate) use pipe_run::PipeProgress;
mod popup_syntax;
pub(crate) mod position_stores;
mod register_ops;
mod registry;
mod replay;
pub(crate) mod search;
pub(crate) mod settings;
mod syntax;
pub(crate) mod tab;
mod theme;
mod timer_bridge;
mod timers;
mod tracked_positions;
mod triggers;
mod visual_move;

// Re-export module-level helpers so sibling submodules can call `super::foo()`.
pub(in crate::editor) use scripting::ConfigPath;
use scripting::theme_search_paths;

pub(in crate::editor) use minibuf::MiniBuffer;

use message_log::MessageLog;
pub(in crate::editor) use message_log::Severity;

// ── Mode ──────────────────────────────────────────────────────────────────────
//
// The editor uses `hume_engine::types::EditorMode` directly. Sticky extend is
// represented as `EditorMode::Extend`. One-shot ctrl-extend is a per-dispatch
// local variable and is NOT a mode change.
//
// `pub(in crate::editor) use EditorMode as Mode;` lets all internal modules use `Mode`
// as an unqualified alias.
pub(in crate::editor) use hume_engine::types::EditorMode as Mode;

use self::inline_output::InlineOutput;
use self::tui::Tui;

/// Every field a `config`/`open`/`cmd`-kind Steel builtin, `set-option!`, or
/// `init.scm` itself can write and that must go back to its compiled-in
/// default on `:reload-config`: the keymap, the registry of dynamic/lazy
/// commands, language identities, decorations, trigger chars, and the
/// cursor-anchored/docked popup, plus every deferred-call queue rooted in
/// the outgoing Steel engine. Every editing-mode layer (Insert/Command/
/// Search/Sift/Prompt, each carrying its own minibuf and any Steel-rooted
/// payload) and the menu, drawer, picker, and disk-change confirm overlays
/// live on `EditorState.input` instead (an `InputStack`), and `Editor::
/// reset_config_state` resets them through `EditorState::reset_input`
/// rather than by this struct's wholesale rebuild.
///
/// Grouped into its own struct, rather than left as individual `EditorState`
/// fields, so `Editor::reset_config_state` resets by *construction*
/// (`self.state.config = ConfigState::new(kitty_enabled, prior_clock)`) instead of by a
/// hand-maintained list of field clears: a field added here is reset the
/// moment it's added, with no second place to remember. Fields that must
/// survive a reload (buffers, panes, undo history, registers, running LSP
/// servers, …) stay on `EditorState` itself: that's the explicit,
/// reviewable "preserved across reload" set.
pub(crate) struct ConfigState {
    /// The trie-based keymap for each mode.
    pub(in crate::editor) keymap: Keymap,
    /// Named keymaps and the buffers each is attached to, looked up before
    /// `keymap`.
    pub(in crate::editor) local_keymaps: LocalKeymaps,
    /// Registry of all mappable commands (motions, selections, edits), plus
    /// every `%define-command!`/`%declare-plugin!` dynamic and lazy entry.
    pub(in crate::editor) registry: CommandRegistry,
    /// Every completion source, native or Steel-registered, keyed by name.
    /// `TypedCommand.completer` names a `Minibuf` entry here, an Insert-mode
    /// trigger invokes the `Buffer` ones. See `completion/registry.rs`'s
    /// module doc.
    pub(in crate::editor) completion_sources: completion::SourceRegistry,
    /// Registry of configured language identities.
    pub(crate) languages: LanguageRegistry,
    /// Trigger characters set for a language, by language name; a language
    /// with no set has no entry. An attachment-scoped set lives on the
    /// attachment instead (`lsp/document.rs`); see `triggers`.
    pub(in crate::editor) triggers: rustc_hash::FxHashMap<Box<str>, triggers::TriggerTable>,
    /// Steel-writable decoration stores (inlay hints, signs, virtual
    /// lines, EOL text, extra highlights, line backgrounds). The render
    /// providers read these.
    pub(in crate::editor) decorations: hume_decorations::DecorationStores,
    /// Text pushed by `(set-statusline-text! source bid text)`, wholesale
    /// per `(bid, source)`, same replace semantics as `decorations`. Nested
    /// rather than flat: the render side needs a
    /// borrowed `(bid, &str)` lookup every frame, and a `HashMap` keyed on
    /// `(BufferId, Box<str>)` has no borrowed-key form, so flat would force an
    /// allocation per element per frame just to look one up.
    ///
    /// Cleared per-buffer when the buffer closes, alongside
    /// `decorations.remove_buffer`. A text reset (`:e!` of a changed file, a
    /// view's new text) keeps it, as it keeps sign-source registrations:
    /// pushed statusline text carries no offsets to invalidate.
    pub(crate) statusline_text:
        rustc_hash::FxHashMap<BufferId, rustc_hash::FxHashMap<Box<str>, Box<str>>>,
    /// Deferred Steel work: events enqueued during command dispatch
    /// (`EditorState::queue_event`) and specific-closure completions
    /// (`EditorState::queue_steel_call` and its siblings: an `lsp-request!`
    /// callback, a timer thunk, a prompt callback), drained in FIFO order by
    /// `Editor::settle`.
    /// One queue, not two: see `event::PendingWork`'s doc for why a shared
    /// queue matters. No work item is ever evaluated inline during command
    /// execution or a completion callback.
    pub(in crate::editor) pending_work: VecDeque<event::PendingWork>,
    /// Buffers awaiting language detection, drained by
    /// `Editor::detect_pending_languages`. Detection needs `self.scripting`
    /// (lazy-plugin activation), which the disjoint-borrow buffer-open
    /// chokepoints (`buffer::lifecycle::open_buffer_and_notify` and callers
    /// with only `&mut EditorState`/`&mut EngineView`) never hold, so they
    /// queue the buffer id here instead of detecting inline. Every
    /// `drain_pending_work` pass drains it; every Steel-eval path also drains
    /// it at the tail of `apply_script_effects`. Nothing drains it while no
    /// scripting host is attached.
    pub(in crate::editor) pending_language_detection: Vec<hume_engine::pipeline::BufferId>,
    /// In-flight `spawn-async!` jobs and `path-programs` scans, keyed by
    /// the handle their builtin returned to Steel, each kept until its last
    /// callback has run. Drained by `Editor::drain_async_jobs`; dropping
    /// this map (a `:reload-config` wholesale rebuild, which drops
    /// `pending_work` with it) stops all of them for free; see
    /// `async_job::PendingJob`'s doc.
    pub(in crate::editor) async_jobs:
        rustc_hash::FxHashMap<hume_scripting::host::JobHandle, async_job::PendingJob>,
    /// Monotonic counter minting the next handle in `async_jobs`.
    pub(in crate::editor) next_async_job_id: u64,
}

impl ConfigState {
    /// Build the config state every session, and every `:reload-config`,
    /// starts from: the compiled-in keymap (plus the kitty-only default
    /// binds when the terminal supports the protocol, matching
    /// [`Editor::set_kitty_support`]) and the compiled-in command registry,
    /// with every other field at its empty/`None` default.
    ///
    /// `prior_clock` is `0` at session start (nothing to carry forward) and
    /// the outgoing `ConfigState.decorations`'s own shared clock on
    /// `:reload-config`. See [`hume_decorations::DecorationStores::reset`]'s
    /// doc for why this can't just be `Default::default()` like every other
    /// field here.
    pub(super) fn new(kitty_enabled: bool, prior_clock: u64) -> Self {
        Self {
            keymap: Keymap::with_defaults(kitty_enabled),
            local_keymaps: LocalKeymaps::default(),
            registry: CommandRegistry::with_defaults(),
            completion_sources: completion::SourceRegistry::with_defaults(),
            languages: LanguageRegistry::new(),
            triggers: rustc_hash::FxHashMap::default(),
            decorations: hume_decorations::DecorationStores::reset(prior_clock),
            statusline_text: rustc_hash::FxHashMap::default(),
            pending_work: VecDeque::new(),
            pending_language_detection: Vec::new(),
            async_jobs: rustc_hash::FxHashMap::default(),
            next_async_job_id: 0,
        }
    }
}

// ── EditorState ───────────────────────────────────────────────────────────────
//
// All command-mutable editor data. Separated from `Editor` so the Steel VM
// (`scripting.steel`) and editor data are sibling borrows that never alias,
// enabling EditorCmd to dispatch synchronously from within a Steel eval.

pub(crate) struct EditorState {
    /// The config, data, runtime and temp directories this session works from,
    /// resolved once at startup. Every consumer reads them here instead of
    /// the process environment.
    pub(crate) dirs: hume_platform::dirs::Dirs,
    /// All open buffers. SSOT for buffer content, history, and file metadata.
    pub(crate) buffers: BufferStore,
    /// Each buffer's diagnostics and last insertion, carried with its text.
    pub(in crate::editor) buffer_positions: position_stores::BufferPositions,
    /// LSP backend, server registry, running instances and in-flight
    /// requests: threaded in production, synchronous-inline in tests.
    pub(in crate::editor) lsp: lsp::LspState,
    /// Config-owned state reset wholesale by `:reload-config`. See
    /// [`ConfigState`]'s doc for exactly what that means and why it's a
    /// separate struct.
    pub(crate) config: ConfigState,
    /// The stack of active input-handling layers above the base editing
    /// surface: `Base` (carrying the sticky Extend flag and a `Sticky`
    /// popup's slot) or one of the four other editing-mode layers
    /// (Insert, which carries the slot too, or Command/Search/Sift/Prompt),
    /// plus whatever overlay widgets (disk-change confirm, fuzzy picker,
    /// selection menu, bottom drawer, an open LSP completion session, a
    /// `Scrollable` popup) are pushed above that.
    ///
    /// Not reset by `ConfigState`'s wholesale rebuild: `Base` must survive
    /// a `:reload-config`, and a still-open mode layer or overlay's Steel
    /// callback must be dropped, not fired, the same as everything
    /// `ConfigState::new` resets. `Editor::reset_config_state` resets it
    /// explicitly through `EditorState::reset_input`.
    pub(in crate::editor) input: input_stack::InputStack,
    pub(in crate::editor) pending: input_stack::PendingSequence,
    /// Accumulated numeric prefix for the next command (e.g. `3` in `3w`).
    pub(super) count: Option<usize>,
    pub(super) registers: RegisterSet,
    /// Kill ring: bounded history of yanked / deleted text.
    pub(super) kill_ring: KillRing,
    /// The `c` register's clipboard: native (`arboard`) or terminal OSC 52.
    pub(super) clipboard: clipboard::SystemClipboard,
    /// The register a `"<reg>` prefix named, armed for the next
    /// yank/delete/change/paste.
    pub(super) register_prefix: Option<char>,
    /// Which source a bare paste reads next, and until when. See
    /// [`commands::PasteStamp`].
    pub(super) paste_stamp: Option<commands::PasteStamp>,
    pub(super) should_quit: bool,
    /// Set by the platform terminator thread to the process exit code when a
    /// signal asks the editor to quit. `0` means "no termination requested"
    /// (never a valid signal-termination exit code). Polled at the top of the
    /// run loop and re-read by `hume_editor::run` after it returns, so both
    /// sides use the same code without a second channel. `should_quit` stays
    /// the single-threaded, in-editor quit path (dirty-buffer prompts, `:q`
    /// semantics); a signal bypasses all of that.
    pub(super) terminate_exit_code: Arc<AtomicI32>,
    /// Set by `Editor::start` for a `Driver::Headless` run, where no one can
    /// answer a prompt: a confirm never opens, so scripted keys are not taken
    /// as answers.
    pub(super) headless: bool,
    /// Panics on background threads, queued by the panic hook
    /// `hume_editor::run` installs. The run loop reports each new one as an
    /// Error; empty (and never filled) when no hook is installed.
    pub(super) worker_panics: hume_platform::worker_panic::WorkerPanics,
    /// Transient one-line message shown in the statusline after an action.
    pub(crate) status_msg: Option<String>,
    /// Keystrokes the message-log summary stays visible before auto-dismissing.
    /// Armed when `status_msg` clears with unseen entries; ticked down in
    /// `handle_key`, but only while no minibuf-mode layer is on the stack
    /// (`input.minibuf().is_none()`), since a long `:` command shouldn't burn the
    /// budget invisibly while it's being typed.
    pub(in crate::editor) summary_ttl: u8,
    /// Persistent log of warnings, errors, and trace entries.
    pub(crate) message_log: MessageLog,
    /// All editor settings: global defaults and per-buffer-overridable values.
    pub(crate) settings: EditorSettings,
    /// Name of the theme `EngineView::theme` was loaded from; empty while the
    /// compiled-in fallback is installed. `settings.theme` is what config asks
    /// for, this is what is on screen, and a failed theme load rolls the
    /// setting back to it.
    pub(in crate::editor) shown_theme: String,
    /// The character and kind from the last find/till motion.
    pub(super) last_find: Option<commands::FindChar>,
    /// The permission to move the active tab's focused pane, which the
    /// engine stores (`EngineView::focused_pane`). See `focus`'s module doc
    /// for why only that module can use it.
    pub(in crate::editor) focus: focus::Focus,
    /// Every open tab's display order and stashed window layout. See
    /// `tab::store`'s module doc for the model (the active tab's layout
    /// lives in `EngineView::layout`, not here).
    pub(in crate::editor) tabs: tab::TabStore,
    /// Per-pane maps: (pane,buffer) selections/groups, transient mode snapshots, jump history.
    pub(super) panes: PaneView,
    /// Bounded, in-memory history for `:`, `/`, and `?` prompts.
    pub(super) history: self::minibuf::history::HistoryStore,
    /// Set by the inline-output dispatch arm to trigger a full repaint.
    pub(in crate::editor) force_full_redraw: bool,
    /// State of the `#:inline-output` bracket for the Steel command(s)
    /// currently on the call stack. Pushed/truncated by `OutputHost::
    /// arm_inline_output`/`truncate_inline_output`, read and driven by
    /// `EditorHostImpl::claim_terminal` so
    /// `SteelCtx` (`print!`, `run-inline-output!`) knows it's safe to write
    /// to the real stdout, and so the screen is only entered lazily, on the
    /// first byte of actual output. See [`InlineOutput`].
    pub(in crate::editor) inline_output: InlineOutput,
    /// Reusable sticky-column buffer for vertical motion, shared by both
    /// units `apply_visual_vertical` handles (row-domain `j`/`k` and
    /// buffer-line `9j`/`9k`). Screen-relative scroll (page/half-page, mouse
    /// wheel) carries its cursor through `commands::scroll_view` + `carry`
    /// instead: a view command, not a motion, so it does not go through
    /// `apply_visual_vertical` at all (see `move_vertical`'s doc).
    pub(super) visual_move_target_display_cols: Vec<hume_editing::selection::StickyDisplayCol>,
    /// The change currently being recorded, and the last one settled; `.`
    /// replays the settled one. See [`replay::Recording`].
    pub(in crate::editor) recording: Option<replay::Recording>,
    /// The code running now, if any: a recorded change's command body
    /// (`replay::record_change`) or an input's callback
    /// (`Editor::run_answered`), set through
    /// [`replay::with_recording_scope`], or an Insert key's binding
    /// (`input_stack::insert::handle_insert`), set through
    /// [`replay::dispatch_insert_key`], each for the extent of that call.
    /// See [`replay::ScopeSlot`].
    pub(in crate::editor) recording_scope: replay::ScopeSlot,
    /// Accumulating selection-recipe buffer for the *next* edit's dot-repeat.
    ///
    /// Tracks how the current selection was built: Motion/Selection commands
    /// append or reset this buffer; a repeatable edit snapshots it into
    /// `RepeatableAction::selection_recipe` and clears it, the native path
    /// (`take_recipe`) via `mem::take`, the Steel path
    /// (`open_steel_recipe`/`close_steel_recipe`) via `.clone()` followed by
    /// an explicit clear, so an inner `call!` dispatch still sees the
    /// pre-body value to compose onto. Non-selection commands clear it. See
    /// `RepeatableAction::selection_recipe` for the invariant.
    pub(super) selection_recipe: Vec<SelectionStep>,
    /// Set by every native dispatch's `update_recipe` (`replay/recipe.rs`),
    /// regardless of what it did to the recipe, and reset before a Steel
    /// command body runs.
    ///
    /// `close_steel_recipe` reads it after a Steel body: `false`
    /// means the body dispatched no native command at all (a pure-Steel body),
    /// so `selection_recipe` (left untouched from before the body ran) must
    /// still be cleared, the same as any other non-selection command would.
    /// `true` means an inner `call!` already ran `update_recipe` with its
    /// own correct decision, which the Steel branch must not override. A plain
    /// `selection_recipe == pre_recipe` snapshot comparison can't stand in for
    /// this: it would misfire when a body re-establishes the identical step it
    /// started with.
    pub(super) selection_recipe_touched: bool,
    /// Deferred dot-repeat job enqueued by `cmd_repeat`; consumed by
    /// `replay_dot` at the tail of `handle_key`.
    pub(super) pending_repeat: Option<PendingRepeat>,
    /// Every `pipe` run in flight; the statusline's `Pipe` element shows the
    /// focused buffer's.
    pub(crate) pipe_runs: pipe_run::PipeRuns,
    /// The live sift preview's result for the `sift-within` its Enter runs,
    /// taken by the next `sift-within` whatever it sifts.
    pub(in crate::editor) sift_reuse: Option<input_stack::SiftReuse>,
    /// How many times a sift has scanned selections for a pattern, the live
    /// preview and `sift-within` alike.
    #[cfg(test)]
    pub(in crate::editor) sift_scans: usize,
    /// `Some` for `replay_dot`'s whole extent. Read by
    /// [`EditorState::interactive_input`], which every interactive input
    /// passes before it opens, to refuse one reached during `.`, and by each
    /// read, picker, prompt, menu and text prompt, which takes its answer
    /// from it.
    pub(in crate::editor) dot_replay: Option<replay::DotReplay>,
    /// The editor's open Paste session or dot-repeat placeholder, if any.
    /// An open Insert session lives on the input stack instead; see
    /// [`EditorState::open_session`] for the one open session of any kind.
    pub(in crate::editor) active_session: Option<edit_session::EditSession>,
    pub(super) macros: macros::MacroState,
    /// Set by `Editor::handle_input` right after dispatch whenever that
    /// input logged a new warning or error, cleared by `Editor::settle` once
    /// its drain (including the buffer-enter disk check) has run. The
    /// window spans "input dispatched" to "its consequences settled", not
    /// one call. Read only by `confirm_permit`, so a command's own failure
    /// message (`:qa` naming the first dirty buffer) can't be silently
    /// replaced by an unrelated disk-change confirm opened by the focus move
    /// that triggered it. Always `false` outside that window.
    pub(super) message_logged_this_input: bool,
    /// The buffer that owned focus as of the last `Editor::settle()` pass.
    /// `None` only before the first `settle()` ever runs, so the startup
    /// buffer legitimately raises `OnBufferEnter` (matching Vim's
    /// `BufEnter`). The single observation baseline for the focus diff; see
    /// `Editor::detect_buffer_enter`.
    pub(super) last_entered_buffer: Option<BufferId>,
    /// Writes parked until their buffer's `on-before-buffer-save` handlers
    /// finish; see `before_save`.
    pub(in crate::editor) pending_writes: before_save::PendingWrites,
    /// The mode observed as of the last `drain_pending_work` pass: the
    /// baseline `Editor::detect_mode_change` diffs `mode()` against to raise
    /// `OnModeChange`, the same observation-point shape `last_entered_buffer`
    /// uses for `OnBufferEnter`. Re-baselined by `reset_config_state` after
    /// its `reset_input` call so a reload never fires a phantom
    /// transition for a mode the fresh hooks never observed.
    pub(super) last_observed_mode: Mode,
    /// The pane a left-button drag started in, set on mouse-left-down. The
    /// drag's anchor is that pane's primary selection anchor, read live.
    pub(super) mouse_drag: Option<PaneId>,
    /// Current working directory. Set at startup; updated by `:cd`.
    pub(super) cwd: AbsolutePath,
    /// Every overlay view shared between the per-frame write side and the
    /// engine's render side:
    /// minibuf-completion, popup (cursor + docked), menu, completion menu,
    /// drawer, picker. One
    /// `hume_ui::OverlayViews` instead of seven hand-allocated `Arc`s, each
    /// wired through `build_pane`'s parameter list and `Editor::open`'s
    /// bootstrap by hand; see that type's own doc for why.
    pub(in crate::editor) views: hume_ui::OverlayViews,
    /// Shared tab-bar view: written by `EditorState::sync_band_views`
    /// (`sync_tabline_view`), read by `TablineWidget`. Not part of
    /// `hume_ui::OverlayViews` above: `TablineViewState` holds a `TabId`,
    /// an `editor`-crate type `hume-ui` cannot depend on (see
    /// `crate::tabline`'s own module doc).
    pub(in crate::editor) tabline_view:
        hume_engine::lock::SharedSlot<crate::tabline::TablineViewState>,
    /// Hash of everything `sync_tabline_view`'s rebuild depends on, as of
    /// the last time it ran the rebuild. `None` before the first one. An
    /// unchanged hash means `tabline_view` is still correct, so the per-tab
    /// label rebuild (one allocating `display_name()` call per tab) and the
    /// scroll probe are skipped whenever nothing about the tab bar changed,
    /// which is almost every time. See `EditorState::tabline_signature`'s
    /// doc for what it covers.
    last_tabline_signature: Option<u64>,
    /// Cross-thread waker clone (see `Editor::open`'s `wake` param), reachable
    /// here so `EditorHostImpl` (which only ever holds a disjoint `&mut
    /// EditorState` borrow, never a whole `&mut Editor`) can hand it to a
    /// spawned picker source (`picker-source-spawn!`) so its reader thread
    /// can wake the event loop. A no-op `Arc` in tests/headless.
    pub(super) wake: WakeCallback,
}

impl EditorState {
    /// The trivial-field baseline both `EditorState` constructors build on,
    /// around the `focus` they seeded with the first pane and their working
    /// directory `cwd`, which has no inert form.
    ///
    /// Not a usable editor on its own: no buffers, an OSC 52 clipboard, and
    /// a no-op waker. It exists so the fields that are identical at both
    /// construction sites (`Editor::open` and `Editor::for_testing`) are
    /// written once. Every field whose real value differs between those two
    /// sites is set here to an inert form (`lsp` is the threaded backend
    /// with a no-op waker) and named explicitly by `Editor::open`, so no
    /// production value is inherited by accident. A field added later whose
    /// production value must differ from its baseline needs the same
    /// treatment; the compiler will not ask for it.
    pub(in crate::editor) fn base(focus: focus::Focus, cwd: AbsolutePath) -> Self {
        let settings = EditorSettings::default();
        let history_capacity = settings.prompt_history_levels;
        Self {
            dirs: hume_platform::dirs::Dirs::none(),
            buffers: BufferStore::new(),
            buffer_positions: position_stores::BufferPositions::default(),
            lsp: lsp::LspState::new_threaded(Arc::new(|| {})),
            // `kitty_enabled: false` matches: the real probe result isn't known
            // until `set_kitty_support` runs, after `Editor::open`.
            config: ConfigState::new(false, 0),
            input: input_stack::InputStack::new(),
            pending: input_stack::PendingSequence::default(),
            count: None,
            registers: RegisterSet::new(),
            kill_ring: KillRing::new(),
            clipboard: clipboard::SystemClipboard::osc52(),
            register_prefix: None,
            paste_stamp: None,
            should_quit: false,
            terminate_exit_code: Arc::new(AtomicI32::new(0)),
            headless: false,
            worker_panics: hume_platform::worker_panic::WorkerPanics::default(),
            status_msg: None,
            summary_ttl: 0,
            message_log: MessageLog::new(),
            settings,
            shown_theme: String::new(),
            last_find: None,
            focus,
            tabs: tab::TabStore::new(),
            panes: PaneView::default(),
            history: minibuf::history::HistoryStore::new(history_capacity),
            force_full_redraw: false,
            inline_output: InlineOutput::default(),
            visual_move_target_display_cols: Vec::new(),
            recording: None,
            recording_scope: replay::ScopeSlot::default(),
            selection_recipe: Vec::new(),
            selection_recipe_touched: false,
            pending_repeat: None,
            pipe_runs: pipe_run::PipeRuns::new(),
            sift_reuse: None,
            #[cfg(test)]
            sift_scans: 0,
            dot_replay: None,
            active_session: None,
            macros: macros::MacroState::default(),
            message_logged_this_input: false,
            last_entered_buffer: None,
            pending_writes: before_save::PendingWrites::default(),
            last_observed_mode: Mode::Normal,
            mouse_drag: None,
            cwd,
            views: hume_ui::OverlayViews::default(),
            tabline_view: hume_engine::lock::SharedSlot::default(),
            last_tabline_signature: None,
            wake: Arc::new(|| {}),
        }
    }
}

impl EditorState {
    /// Whether a signal asked the editor to terminate
    /// (`terminate_exit_code` is non-zero).
    pub(super) fn terminate_requested(&self) -> bool {
        self.terminate_exit_code
            .load(std::sync::atomic::Ordering::Acquire)
            != 0
    }

    // ── Mode ──────────────────────────────────────────────────────────────────

    pub(crate) fn mode(&self) -> Mode {
        self.input.mode()
    }

    /// The minibuf that owns the statusline row and the hardware cursor
    /// right now: `crate::statusline`'s reader and `Editor::run`'s cursor
    /// placement. Not the same query as `InputStack::minibuf()`: a
    /// minibuf-mode layer can be buried under a picker or menu (neither is a
    /// mode layer, so `mode()` doesn't change, but `top()` does), in which
    /// case it no longer owns the keyboard or this row, and painting/placing
    /// the cursor against it here would show a dead prompt while whatever's
    /// on top reads the keys. `Some` only when the minibuf-owning layer is
    /// `top()`. See [`Self::confirm`] (`input_stack/confirm.rs`) for the
    /// identical carve-out and why this wrapper exists instead of exposing
    /// `input` itself.
    pub(crate) fn minibuf(&self) -> Option<&MiniBuffer> {
        self.input.top_minibuf()
    }

    // ── Quit ──────────────────────────────────────────────────────────────────

    /// Quit HUME, refused without `force` while a buffer has unsaved changes,
    /// `first`'s named first, or while a pipe runs: exiting drops every
    /// buffer and stops every run, with the process.
    pub(in crate::editor) fn quit(
        &mut self,
        first: BufferId,
        force: bool,
    ) -> Result<(), CloseBufferRefused> {
        let on_unsaved = OnUnsaved::from_force(force);
        std::iter::once(first)
            .chain(self.buffers.iter().map(|(id, _)| id))
            .try_for_each(|bid| self.refuse_unsaved(bid, on_unsaved))
            .map_err(CloseBufferRefused::Unsaved)?;
        if !force && let Some(busy) = self.buffers.first_busy() {
            return Err(CloseBufferRefused::PipeRunning(PipeRunning(busy)));
        }
        self.should_quit = true;
        Ok(())
    }
}

pub(crate) struct Editor {
    /// All command-mutable editor data. Disjoint from `scripting` so Steel evals
    /// can borrow `state` and `scripting.steel` simultaneously without aliasing.
    pub(crate) state: EditorState,
    /// Engine rendering state: layout, panes, buffers, theme.
    pub(in crate::editor) view: EngineView,
    /// Whether the kitty keyboard protocol was successfully activated at startup.
    pub(in crate::editor) kitty_enabled: bool,
    /// The embedded Steel scripting host.
    pub(super) scripting: Option<hume_scripting::ScriptingHost>,
    /// Where this session's config comes from: `--config FILE`,
    /// `--no-config`, or the default `<config_dir>/init.scm`. Set once via
    /// `set_config_source`, before the first `init_scripting` call. It
    /// outlives startup so a reload re-runs the same source the session
    /// booted from (and `:reload-config` refuses to run at all under
    /// `ConfigSource::Skip`, see `typed_reload_config`).
    pub(super) config_source: crate::cli::ConfigSource,
    /// Snapshot of Rust-builtin command names taken at end of `init_scripting`.
    pub(super) builtin_cmd_names: hume_scripting::command_name::CommandNameSet,
    /// Parse backend: threaded in production, synchronous-inline in tests.
    parse_worker: Box<dyn ParseBackend>,
    /// Whether the one-shot "parse worker disconnected" message has been logged.
    parse_worker_disconnect_logged: bool,
    /// Every scheduled timer and what firing it does; Steel-visible via the
    /// `after!`/`debounce` builtins.
    timers: timer_bridge::Timers,
    /// This pane's currently-pending `OnViewportChange` debounce timer, if
    /// any, looked up to cancel-and-replace on the next change.
    viewport_debounce: rustc_hash::FxHashMap<hume_engine::pipeline::PaneId, timers::TimerId>,
    /// This buffer's currently-pending diagnostics-pull debounce timer, if
    /// any, looked up to cancel-and-replace on the next edit.
    diagnostic_pull_debounce:
        rustc_hash::FxHashMap<hume_engine::pipeline::BufferId, timers::TimerId>,
    /// `(buffer_id, visible range)` as of the last frame this pane was
    /// *visible*, per pane, the range being what `OnViewportChange` carries
    /// (`geometry::pane_visible_range`). `prepare_frame`'s scroll step
    /// compares against this so the hook arms only when its payload would
    /// differ. The buffer id is part of the key for the same reason
    /// `virtual_lines_synced` below carries one: a pane switching buffers at
    /// an unchanged range (`:b#`, a tab switch back onto identical geometry)
    /// must still count as a change, or the newly-shown buffer's
    /// viewport-driven consumers (LSP inlay hints) never re-fire. Entries for
    /// panes outside the active set are dropped each frame rather than left
    /// to go stale (see `prepare_frame`'s scroll pass): a background tab's
    /// pane isn't observed at all while hidden, so its return must itself be
    /// treated as a change, never a match against a snapshot from before it
    /// left.
    last_visible_range: rustc_hash::FxHashMap<
        hume_engine::pipeline::PaneId,
        (
            BufferId,
            hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine>,
        ),
    >,
    /// `(buffer_id, decorations.generation(buffer_id))` as of each
    /// pane's last mirror into its `PaneVirtualLines` Arc. `prepare_frame`
    /// compares against this to skip the rebuild on frames where neither
    /// that buffer's stamp nor the pane's buffer changed, since this runs in
    /// scroll/cursor math too, not just render. The buffer id is part of the
    /// key so a pane switching buffers always rebuilds, even onto a buffer
    /// whose stamp happens to match the old one; otherwise it would keep
    /// mirroring the previous buffer's virtual lines.
    virtual_lines_synced: rustc_hash::FxHashMap<hume_engine::pipeline::PaneId, (BufferId, u64)>,
    /// Whether [`Editor::run`]'s event loop owns the terminal, and the
    /// handle to drive it when it does. See [`Tui`]'s own module doc for
    /// why these are one field. Tests and headless `run_keys` dispatch
    /// commands directly and never enter `run`, so this stays `Off` there:
    /// dispatch reads it to skip the inline-output terminal bracket
    /// (alt-screen toggle + "press any key to return" block) when there is
    /// no TUI to suspend and no interactive user to press a key.
    tui: Tui,
    /// `(mouse, mouse_select)` as last applied to the terminal's
    /// mouse tracking mode. `prepare_frame` compares this against the live
    /// `state.settings` values every frame and re-applies the terminal mode
    /// when they differ, so `:set global mouse=…`/`mouse-select=…`
    /// take effect immediately instead of only at the next restart. See
    /// `hume_platform::terminal::set_mouse_mode`.
    applied_mouse_mode: (bool, bool),
    /// The working directory last reported to the terminal (OSC 7).
    /// `prepare_frame` compares it against `state.cwd` every frame, so `:cd`
    /// and `(set-cwd! …)` need no hook of their own. `None` until the first
    /// frame, so that frame reports.
    applied_cwd: Option<AbsolutePath>,
    /// Startup cursor placements, one per CLI `path:line[:col]` argument,
    /// queued by `Editor::start` and drained by `apply_startup_positions`.
    startup_positions: Vec<(BufferId, crate::cli::PathPosition)>,
    /// The receiving end of `state.wake`, which a pipe run with no terminal
    /// waits on. `None` when `state.wake` wakes a terminal's reader.
    pub(in crate::editor) detached: Option<pipe_run::Detached>,
    /// Terminal input a pipe wait read and left to the run loop, oldest
    /// first. The run loop handles it before reading the terminal again.
    pub(in crate::editor) input_backlog: VecDeque<termina::event::Event>,
    /// The console the next pipe run during a replay waits on, in place of
    /// the terminal.
    #[cfg(test)]
    pub(in crate::editor) inline_pipe_console: Option<Box<dyn pipe_run::PipeConsole>>,
}

impl Editor {
    // ── Buffer accessors ──────────────────────────────────────────────────────

    /// The `BufferId` the focused pane is currently viewing.
    pub(crate) fn focused_buffer_id(&self) -> BufferId {
        self.view.panes[self.view.focused_pane()].buffer_id()
    }

    /// Shared reference to the focused buffer.
    pub(crate) fn doc(&self) -> &Buffer {
        self.state.buffers.get(self.focused_buffer_id())
    }

    /// Mutable reference to the focused buffer.
    ///
    /// Uses a split borrow: `buffers` and other fields on `Editor` are
    /// disjoint, so you can hold this reference while reading e.g. `self.state.settings`.
    /// Do NOT keep this reference live across a call that also borrows `self`.
    #[cfg(test)]
    pub(in crate::editor) fn doc_mut(&mut self) -> &mut Buffer {
        let bid = self.focused_buffer_id();
        self.state.buffers.get_mut(bid)
    }

    /// The focused pane's selections for the current buffer.
    pub(super) fn current_selections(&self) -> &SelectionSet {
        let t = commands::FocusedPane::current(&self.view).pane();
        commands::pane_selections(&self.state, &self.view, t)
    }

    /// The focused pane's selections, bound to the current buffer's text.
    pub(in crate::editor) fn current_view(&self) -> hume_editing::selection::EditView<'_> {
        let t = commands::FocusedPane::current(&self.view).pane();
        commands::pane_view(&self.state, &self.view, t)
    }

    /// Replace the focused pane's selections for the current buffer.
    pub(in crate::editor) fn set_current_selections(&mut self, sels: SelectionSet) {
        let t = commands::FocusedPane::current(&self.view).pane();
        commands::set_pane_selections(&mut self.state, &self.view, t, sels);
    }

    // ── Mode transitions ──────────────────────────────────────────────────────

    pub(in crate::editor) fn end_insert_session(&mut self) {
        commands::end_insert_session(&mut self.state, &self.view);
    }
}

#[cfg(test)]
mod field_classification;

#[cfg(test)]
pub(crate) mod tests;
