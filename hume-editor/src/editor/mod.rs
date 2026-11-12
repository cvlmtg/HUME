use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicI32;

use hume_core::{BufferId, OnUnsaved, PaneId};
use hume_engine::pipeline::EngineView;

use self::registry::CommandRegistry;
use self::replay::{PendingRepeat, SelectionStep};
use crate::editor::buffer::Buffer;
use crate::editor::buffer::lifecycle::CloseBufferRefused;
use crate::editor::buffer::store::BufferStore;
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
#[cfg(any(test, feature = "test-util"))]
pub(crate) mod bench;
mod clock;
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
mod folds;
mod fuzzy;
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
mod selection_history;
pub(crate) mod session;
pub(crate) mod settings;
mod stamped_file;
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

use message_log::{MessageLog, MessagesView, StatusMsg};

// ── Mode ──────────────────────────────────────────────────────────────────────
//
// Sticky extend is `EditorMode::Extend`; a one-shot Ctrl-extend is a
// per-dispatch local, not a mode change.
pub(in crate::editor) use hume_engine::types::EditorMode as Mode;

use self::inline_output::InlineOutput;
use self::tui::Tui;

/// Every field a Steel builtin, `set-option!`, or `init.scm` can write that
/// goes back to its compiled-in default on `:reload-config`, plus every
/// deferred-call queue rooted in the outgoing Steel engine.
///
/// `Editor::reset_config_state` replaces it whole, so a field added here
/// resets with no second place to remember. State that survives a reload
/// (buffers, panes, undo history, registers, running LSP servers) stays on
/// `EditorState`; the input stack is reset through
/// `EditorState::reset_input`.
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
    /// `TypedCommand.completer` names a `Minibuf` entry here, and an
    /// Insert-mode trigger invokes the `Buffer` ones.
    pub(in crate::editor) completion_sources: completion::SourceRegistry,
    /// Registry of configured language identities.
    pub(crate) languages: LanguageRegistry,
    /// Trigger characters set for a language, by language name; a language
    /// with no set has no entry. An attachment-scoped set lives on the
    /// attachment instead.
    pub(in crate::editor) triggers: rustc_hash::FxHashMap<Box<str>, triggers::TriggerTable>,
    /// Steel-writable decoration stores (inlay hints, signs, virtual
    /// lines, EOL text, extra highlights, line backgrounds). The render
    /// providers read these.
    pub(in crate::editor) decorations: hume_decorations::DecorationStores,
    /// Whether `decorations` holds the `[messages]` highlights. A fresh
    /// `ConfigState` holds none, so the view reads stale.
    pub(in crate::editor) messages_highlighted: bool,
    /// Text pushed by `(set-statusline-text! bid source text)`, replaced
    /// whole per `(bid, source)`. Nested so the render side can look up a
    /// borrowed `(bid, &str)` every frame without allocating a key.
    ///
    /// Cleared per buffer when the buffer closes. A text reset (`:e!`, a
    /// view's new text) keeps it: pushed statusline text carries no offsets
    /// to invalidate.
    pub(crate) statusline_text:
        rustc_hash::FxHashMap<BufferId, rustc_hash::FxHashMap<Box<str>, Box<str>>>,
    /// Deferred Steel work: events queued during command dispatch
    /// (`EditorState::queue_event`) and closure completions
    /// (`EditorState::queue_steel_call` and its siblings), drained in FIFO
    /// order by `Editor::settle`. No item is evaluated inline during command
    /// execution or a completion callback.
    pub(in crate::editor) pending_work: VecDeque<event::PendingWork>,
    /// Buffers awaiting language detection, drained by
    /// `Editor::detect_pending_languages`. Detection needs `self.scripting`
    /// for lazy-plugin activation, which the buffer-open paths holding only
    /// `&mut EditorState` lack, so they queue the id here. Every
    /// `drain_pending_work` pass and every Steel eval's
    /// `apply_script_effects` drains it; nothing does while no scripting
    /// host is attached.
    pub(in crate::editor) pending_language_detection: Vec<BufferId>,
    /// In-flight `spawn-async!` jobs and `path-programs` scans, keyed by
    /// the handle their builtin returned to Steel, each kept until its last
    /// callback has run. Drained by `Editor::drain_async_jobs`; dropping
    /// the map on `:reload-config` stops all of them.
    pub(in crate::editor) async_jobs:
        rustc_hash::FxHashMap<hume_scripting::host::JobHandle, async_job::PendingJob>,
    /// Monotonic counter minting the next handle in `async_jobs`.
    pub(in crate::editor) next_async_job_id: u64,
}

impl ConfigState {
    /// The config state every session and every `:reload-config` starts
    /// from: the compiled-in keymap (with the kitty-only binds when the
    /// terminal supports the protocol) and command registry, every other
    /// field empty.
    ///
    /// `prior_clock` is `0` at session start and the outgoing decorations'
    /// clock on `:reload-config`, so no decoration stamp is reused across
    /// reloads.
    pub(super) fn new(
        kitty_enabled: bool,
        prior_clock: u64,
        library_wait: &hume_core::OsWait,
    ) -> Self {
        Self {
            keymap: Keymap::with_defaults(kitty_enabled),
            local_keymaps: LocalKeymaps::default(),
            registry: CommandRegistry::with_defaults(),
            completion_sources: completion::SourceRegistry::with_defaults(),
            languages: LanguageRegistry::new(library_wait.clone()),
            triggers: rustc_hash::FxHashMap::default(),
            decorations: hume_decorations::DecorationStores::reset(prior_clock),
            messages_highlighted: false,
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
    /// What the language registry opens grammar libraries through. Every
    /// scripting host this session starts connects its budget to it.
    pub(crate) library_wait: hume_core::OsWait,
    /// All open buffers. SSOT for buffer content, history, and file metadata.
    pub(crate) buffers: BufferStore,
    /// The time every deadline, grace and age in the editor is measured on.
    /// `buffers` holds a clone of it.
    pub(crate) clock: clock::Clock,
    /// Each buffer's diagnostics and last insertion, carried with its text.
    pub(in crate::editor) buffer_positions: position_stores::BufferPositions,
    /// LSP backend, server registry, running instances and in-flight
    /// requests: threaded in production, synchronous-inline in tests.
    pub(in crate::editor) lsp: lsp::LspState,
    /// Config-owned state, replaced whole by `:reload-config`.
    pub(crate) config: ConfigState,
    /// The stack of input-handling layers: `Base` (carrying the sticky
    /// Extend flag and a `Sticky` popup's slot) or another editing-mode
    /// layer (Insert, Command, Search, Sift, Prompt), plus any overlays
    /// pushed above it (disk-change confirm, picker, menu, drawer,
    /// completion session, `Scrollable` popup).
    ///
    /// Outside `ConfigState`: `Base` survives a `:reload-config`, and
    /// `EditorState::reset_input` drops the rest without firing their Steel
    /// callbacks.
    pub(in crate::editor) input: input_stack::InputStack,
    pub(in crate::editor) pending: input_stack::PendingSequence,
    /// Accumulated numeric prefix for the next command (e.g. `3` in `3w`).
    pub(super) count: hume_core::Count,
    pub(super) registers: RegisterSet,
    /// Kill ring: bounded history of yanked / deleted text.
    pub(super) kill_ring: KillRing,
    /// The `c` register's clipboard: native (`arboard`) or terminal OSC 52.
    pub(super) clipboard: clipboard::SystemClipboard,
    /// The register a `"<reg>` prefix named, armed for the next
    /// yank/delete/change/paste.
    pub(super) register_prefix: Option<char>,
    pub(super) should_quit: bool,
    /// The exit code the platform terminator thread sets when a signal asks
    /// the editor to quit; `0` means none was requested. Polled by the run
    /// loop and re-read by `hume_editor::run`. A signal bypasses
    /// `should_quit`'s dirty-buffer checks.
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
    pub(crate) status_msg: StatusMsg,
    /// Keystrokes the message-log summary stays visible. Armed when
    /// `status_msg` clears with unseen entries; ticked down in `handle_key`
    /// only while no minibuf-mode layer is open, so typing a long `:`
    /// command does not use it up.
    pub(in crate::editor) summary_ttl: u8,
    /// Persistent log of warnings, errors, and trace entries.
    pub(crate) message_log: MessageLog,
    /// What the `[messages]` text was last built from and holds, so a
    /// rebuild changes only the entries that changed. `None` before it
    /// first is.
    pub(in crate::editor) messages_view: Option<MessagesView>,
    /// All editor settings: global defaults and per-buffer-overridable values.
    pub(crate) settings: EditorSettings,
    /// Name of the theme `EngineView::theme` was loaded from; empty while the
    /// compiled-in fallback is installed. `settings.theme` is what config asks
    /// for, this is what is on screen, and a failed theme load rolls the
    /// setting back to it.
    pub(in crate::editor) shown_theme: String,
    /// The character and kind from the last find/till motion.
    pub(super) last_find: Option<commands::FindChar>,
    /// The permission to move the active tab's focused pane, usable only
    /// by the `focus` module.
    pub(in crate::editor) focus: focus::Focus,
    /// Every open tab's display order and stashed window layout. The active
    /// tab's layout lives in `EngineView::layout`.
    pub(in crate::editor) tabs: tab::TabStore,
    /// Per-pane maps: (pane,buffer) selections/groups, transient mode snapshots, jump history.
    pub(super) panes: PaneView,
    /// Bounded, in-memory history for `:`, `/`, and `?` prompts.
    pub(super) history: self::minibuf::history::HistoryStore,
    /// Set by the inline-output dispatch arm to trigger a full repaint.
    pub(in crate::editor) force_full_redraw: bool,
    /// The `#:inline-output` bracket for the Steel commands on the call
    /// stack. `EditorHostImpl::claim_terminal` reads it so `print!` and
    /// `run-inline-output!` know they may write to stdout, entering the
    /// screen only on the first byte of output.
    pub(in crate::editor) inline_output: InlineOutput,
    /// Reusable sticky-column buffer for `apply_visual_vertical`'s motions
    /// (`j`/`k` and `9j`/`9k`). Page and wheel scrolls carry the cursor
    /// through `commands::scroll_view` instead.
    pub(super) visual_move_target_display_cols: Vec<hume_editing::selection::StickyDisplayCol>,
    /// The change currently being recorded, and the last one settled; `.`
    /// replays the settled one.
    pub(in crate::editor) recording: Option<replay::Recording>,
    /// The code running now, if any: a recorded change's command body or an
    /// input's callback, set through [`replay::with_recording_scope`], or an
    /// Insert key's binding, set through [`replay::dispatch_insert_key`],
    /// each for the extent of that call.
    pub(in crate::editor) recording_scope: replay::ScopeSlot,
    /// How the current selection was built, for the next edit's dot-repeat.
    /// Motion and selection commands append to or reset it, other commands
    /// clear it, and a repeatable edit moves it into
    /// `RepeatableAction::selection_recipe`. A Steel command clones it before
    /// clearing it, so an inner `call!` still sees the value from before the
    /// body to compose onto.
    pub(super) selection_recipe: Vec<SelectionStep>,
    /// Set by every native dispatch's `update_recipe` and reset before a
    /// Steel command body runs. After the body, `close_steel_recipe` clears
    /// the recipe when this is `false` (the body dispatched no native
    /// command) and keeps an inner `call!`'s decision when it is `true`.
    /// Comparing the recipe before and after cannot stand in for it: a body
    /// may re-establish the step it started with.
    pub(super) selection_recipe_touched: bool,
    /// The (pane, buffer) pairs whose selections the running Steel session
    /// changed through native commands, recorded as one selection-undo step
    /// each when it ends; `None` outside a session.
    pub(super) selection_group: Option<Vec<(PaneId, BufferId)>>,
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
    /// The editor's open Paste session, if any.
    /// An open Insert session lives on the input stack instead;
    /// [`EditorState::open_session`] gives the open session of either kind.
    pub(in crate::editor) paste_session: Option<edit_session::PasteSession>,
    pub(super) macros: macros::MacroState,
    /// Whether the input being handled logged a new warning or error: set
    /// by `Editor::handle_input` after dispatch, cleared by `Editor::settle`
    /// once its drain has run. Read only by `confirm_permit`, so a command's
    /// own failure (`:qa` naming the first dirty buffer) is not replaced by
    /// a disk-change confirm the focus move it caused would open.
    pub(super) message_logged_this_input: bool,
    /// The buffer that owned focus as of the last `Editor::settle()`, the
    /// baseline `Editor::detect_buffer_enter` diffs against. `None` only
    /// before the first `settle()`, so the startup buffer raises
    /// `OnBufferEnter`.
    pub(super) last_entered_buffer: Option<BufferId>,
    /// Writes parked until their buffer's `on-before-buffer-save` handlers
    /// finish; see `before_save`.
    pub(in crate::editor) pending_writes: before_save::PendingWrites,
    /// The mode as of the last `drain_pending_work` pass, the baseline
    /// `Editor::detect_mode_change` diffs against to raise `OnModeChange`.
    /// `reset_config_state` re-baselines it, so a reload fires no transition
    /// the fresh hooks never observed.
    pub(super) last_observed_mode: Mode,
    /// What a left-button drag in progress moves, set on mouse-left-down.
    pub(super) mouse_drag: Option<mouse::MouseDrag>,
    /// Current working directory. Set at startup; updated by `:cd`.
    pub(super) cwd: AbsolutePath,
    /// The working directory HUME started in, which `session-root`
    /// defaults to on every reset of the settings.
    pub(in crate::editor) startup_dir: AbsolutePath,
    /// Every overlay view shared between the per-frame write side and the
    /// engine's render side: minibuf completion, popup (cursor and docked),
    /// menu, completion menu, drawer, picker.
    pub(in crate::editor) views: hume_ui::OverlayViews,
    /// Shared tab-bar view, written by `sync_tabline_view` and read by
    /// `TablineWidget`. Not in `OverlayViews`: it holds a `TabId`, which
    /// `hume-ui` cannot depend on.
    pub(in crate::editor) tabline_view:
        hume_engine::lock::SharedSlot<crate::tabline::TablineViewState>,
    /// Hash of everything `sync_tabline_view`'s rebuild depends on, as of
    /// its last rebuild; `None` before the first. An unchanged hash skips
    /// the per-tab label rebuild and the scroll probe.
    last_tabline_signature: Option<u64>,
    /// The cross-thread waker, here so `EditorHostImpl`, which holds only
    /// `&mut EditorState`, can hand it to a spawned picker source. A no-op in
    /// tests and headless runs.
    pub(super) wake: WakeCallback,
}

impl EditorState {
    /// The baseline both `EditorState` constructors (`Editor::bootstrap`,
    /// `Editor::for_testing`) build on, around the `focus` seeded with the
    /// first pane and the working directory `cwd`.
    ///
    /// Not a usable editor on its own: no buffers, an OSC 52 clipboard, a
    /// no-op waker. A field whose production value differs is set here to an
    /// inert form and named explicitly by `Editor::bootstrap`; a new such field
    /// needs the same treatment, which the compiler does not ask for.
    pub(in crate::editor) fn base(focus: focus::Focus, cwd: AbsolutePath) -> Self {
        let settings = EditorSettings::new(&cwd);
        let history_capacity = settings.prompt_history_levels;
        let library_wait = hume_core::OsWait::default();
        let clock = clock::Clock::default();
        Self {
            dirs: hume_platform::dirs::Dirs::none(),
            buffers: BufferStore::new(clock.clone()),
            clock,
            buffer_positions: position_stores::BufferPositions::default(),
            lsp: lsp::LspState::new_threaded(Arc::new(|| {})),
            // The real probe result arrives later, through `set_kitty_support`.
            config: ConfigState::new(false, 0, &library_wait),
            library_wait,
            input: input_stack::InputStack::new(),
            pending: input_stack::PendingSequence::default(),
            count: hume_core::Count::UNTYPED,
            registers: RegisterSet::new(),
            kill_ring: KillRing::new(),
            clipboard: clipboard::SystemClipboard::osc52(),
            register_prefix: None,
            should_quit: false,
            terminate_exit_code: Arc::new(AtomicI32::new(0)),
            headless: false,
            worker_panics: hume_platform::worker_panic::WorkerPanics::default(),
            status_msg: StatusMsg::default(),
            summary_ttl: 0,
            message_log: MessageLog::new(),
            messages_view: None,
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
            selection_group: None,
            pending_repeat: None,
            pipe_runs: pipe_run::PipeRuns::new(),
            sift_reuse: None,
            #[cfg(test)]
            sift_scans: 0,
            dot_replay: None,
            paste_session: None,
            macros: macros::MacroState::default(),
            message_logged_this_input: false,
            last_entered_buffer: None,
            pending_writes: before_save::PendingWrites::default(),
            last_observed_mode: Mode::Normal,
            mouse_drag: None,
            startup_dir: cwd.clone(),
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

    /// Drop a half-typed key sequence and its count.
    pub(in crate::editor) fn discard_pending_keys(&mut self) {
        self.pending.clear();
        self.count = hume_core::Count::UNTYPED;
    }

    /// Whether code of `visibility` acts on buffer `bid`
    /// ([`Buffer::visible_to`]); `false` for a closed buffer.
    pub(in crate::editor) fn buffer_visible_to(
        &self,
        bid: BufferId,
        visibility: &hume_scripting::BufferVisibility,
    ) -> bool {
        self.buffers
            .try_get(bid)
            .is_some_and(|buf| buf.visible_to(&self.config.languages, visibility))
    }

    // ── Mode ──────────────────────────────────────────────────────────────────

    pub(crate) fn mode(&self) -> Mode {
        self.input.mode()
    }

    /// The minibuf that owns the statusline row and the hardware cursor now:
    /// `Some` only when its layer is `top()`. A minibuf-mode layer buried
    /// under a picker or menu no longer reads the keys, so painting it here
    /// would show a dead prompt.
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
    /// `--no-config`, or the default `<config_dir>/init.scm`. Set before the
    /// first `init_scripting` and kept so a reload re-runs the same source.
    pub(super) config_source: crate::cli::ConfigSource,
    /// Parse backend: threaded in production, synchronous-inline in tests.
    parse_worker: Box<dyn ParseBackend>,
    /// Whether the one-shot "parse worker disconnected" message has been logged.
    parse_worker_disconnect_logged: bool,
    /// Every scheduled timer and what firing it does; Steel-visible via the
    /// `after!`/`debounce` builtins.
    timers: timer_bridge::Timers,
    /// Each pane's pending `OnViewportChange` debounce timer, replaced on
    /// the next change.
    viewport_debounce: rustc_hash::FxHashMap<PaneId, hume_core::TimerId>,
    /// Each buffer's pending diagnostics-pull debounce timer, replaced on
    /// the next edit.
    diagnostic_pull_debounce: rustc_hash::FxHashMap<BufferId, hume_core::TimerId>,
    /// Each pane's `(buffer_id, visible range)` as of the last frame it was
    /// visible, the range `OnViewportChange` carries. `prepare_frame` arms
    /// the hook only when this differs. The buffer id makes a buffer switch
    /// at an unchanged range (`:b#`) a change, and entries for panes outside
    /// the active set are dropped each frame, so a background tab's pane
    /// counts as changed when it returns.
    last_visible_range: rustc_hash::FxHashMap<
        PaneId,
        (
            BufferId,
            hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine>,
        ),
    >,
    /// `(buffer_id, decorations.generation(buffer_id))` as of each pane's
    /// last mirror into its `PaneVirtualLines`. `prepare_frame` skips the
    /// rebuild when neither changed; the buffer id makes a buffer switch
    /// rebuild even when the stamps happen to match.
    virtual_lines_synced: rustc_hash::FxHashMap<PaneId, (BufferId, u64)>,
    /// Whether [`Editor::run`]'s event loop owns the terminal, and the
    /// handle to drive it when it does. `Off` in tests and headless
    /// `run_keys`, where dispatch skips the inline-output terminal bracket.
    tui: Tui,
    /// The mouse-tracking mode the terminal has, as this editor last
    /// applied it. It starts `None` because `terminal::init` leaves tracking
    /// off.
    applied_mouse_mode: Option<hume_platform::terminal::MouseReporting>,
    /// The working directory last reported to the terminal (OSC 7).
    /// `prepare_frame` compares it against `state.cwd` every frame, so `:cd`
    /// and `(set-cwd! …)` need no hook of their own. `None` until the first
    /// frame, so that frame reports.
    applied_cwd: Option<AbsolutePath>,
    /// Startup cursor placements, one per CLI `path:line[:col]` argument,
    /// queued by `Editor::start` and drained by `apply_startup_positions`.
    startup_positions: Vec<(BufferId, crate::cli::PathPosition)>,
    /// Whether quitting saves the session: the run was started with no
    /// files, or `:save-session` started its session.
    keeps_session: bool,
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
