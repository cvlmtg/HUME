use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::AtomicI32;
use std::sync::{Arc, RwLock};

use termina::event::KeyEvent;

use hume_engine::pipeline::{BufferId, EngineView, PaneId};

use self::registry::CommandRegistry;
use self::replay::{InsertSession, MacroPending, PendingRepeat, RepeatableAction, SelectionStep};
use crate::editor::buffer::Buffer;
use crate::editor::buffer::store::BufferStore;
use crate::editor::pane_state::PaneView;
use crate::lock_ext::LockExt;
use crate::settings::EditorSettings;
use hume_editing::selection::SelectionSet;
use hume_ops::register::{KillRing, RegisterSet};
use hume_treesitter::parse_worker::ParseBackend;
use hume_treesitter::registry::LanguageRegistry;

use self::keymap::{Keymap, WaitCharPending};

mod async_job;
mod async_source;
mod decoration_providers;
mod diff_bridge;
pub(crate) mod error;
mod frame;
pub(crate) mod host_impl;
mod lifecycle;
mod overlay_sync;
mod reload;
mod scripting_setup;

pub(crate) mod buffer;
mod clipboard;
mod commands;
pub(crate) mod completion;
pub(crate) mod cursor;
pub(crate) mod decorations;
mod dispatch;
pub(crate) mod doc_ops;
pub(crate) mod event;
pub(crate) mod fuzzy;
pub(crate) mod jump_list;
pub(crate) mod keymap;
#[cfg(test)]
mod lints;
pub(crate) mod lsp;
mod mappings;
mod message_log;
mod minibuf;
mod mouse;
pub(crate) mod pane_state;
pub(crate) mod picker;
mod picker_source;
pub(crate) mod register_ops;
mod registry;
mod replay;
pub(super) mod scroll;
pub(crate) mod search;
pub(crate) mod settings_ops;
pub(crate) mod syntax;
mod theme;
mod timer_bridge;
mod timers;
mod visual_move;

pub(crate) use search::SearchState;

// Re-export module-level helpers so sibling submodules can call `super::foo()`.
use scripting_setup::theme_search_paths;

pub(crate) use minibuf::MiniBuffer;

use message_log::MessageLog;
pub(crate) use message_log::Severity;

// ── Mode ──────────────────────────────────────────────────────────────────────
//
// The editor uses `hume_engine::types::EditorMode` directly. Sticky extend is
// represented as `EditorMode::Extend`. One-shot ctrl-extend is a per-dispatch
// local variable and is NOT a mode change.
//
// `pub(crate) use EditorMode as Mode;` lets all internal modules use `Mode`
// as an unqualified alias.
pub(crate) use hume_engine::types::EditorMode as Mode;

// ── InlineOutputDispatch ─────────────────────────────────────────────────────

/// State of the `#:inline-output` terminal bracket for the command currently
/// being dispatched. The alt-screen is entered lazily — only once a builtin
/// actually has terminal output to produce (`ensure_inline_output_screen`) —
/// so a command whose body only logs (`log!`, status line) never flashes an
/// empty screen or blocks on a keypress nobody needed to answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InlineOutputDispatch {
    /// Not dispatching an `#:inline-output` command.
    Inactive,
    /// Declared `#:inline-output #t` and `Editor::run` owns the terminal, but
    /// no builtin has produced output yet — the alt-screen is still up.
    /// Carries what `ensure_inline_output_screen` needs to enter: the kitty/
    /// mouse state to restore on the way back, and the command name for the
    /// running banner.
    Armed {
        kitty: bool,
        mouse: bool,
        name: String,
    },
    /// A builtin has produced output — the alt-screen has been left and the
    /// running banner printed. Dispatch must close the bracket (press-any-key
    /// prompt + restore the TUI) after the command body returns.
    Entered,
    /// Declared `#:inline-output #t` but off the event loop (tests, headless
    /// `run_keys`) — there is no alt-screen to leave and no interactive user
    /// to answer a keypress prompt. Raw stdout writes stay permitted (mirrors
    /// `Entered`'s effect on `is_inline_output_command`) but no bracket runs.
    Headless,
}

// ── ConfigState ───────────────────────────────────────────────────────────────

/// Every field a `config`/`open`/`cmd`-kind Steel builtin, `set-option!`, or
/// `init.scm` itself can write and that must go back to its compiled-in
/// default on `:reload-config` — the keymap, the registry of dynamic/lazy
/// commands, language identities, decorations, trigger chars, and the four
/// overlay models (popup/menu/drawer/picker), plus every deferred-call queue
/// rooted in the outgoing Steel engine.
///
/// Grouped into its own struct, rather than left as individual `EditorState`
/// fields, so `Editor::reset_config_state` resets by *construction*
/// (`self.state.config = ConfigState::new(kitty_enabled, prior_clock)`) instead of by a
/// hand-maintained list of field clears: a field added here is reset the
/// moment it's added, with no second place to remember. Fields that must
/// survive a reload (buffers, panes, undo history, registers, running LSP
/// servers, …) stay on `EditorState` itself — that's the explicit,
/// reviewable "preserved across reload" set.
pub(crate) struct ConfigState {
    /// The trie-based keymap for each mode.
    pub(crate) keymap: Keymap,
    /// Registry of all mappable commands (motions, selections, edits), plus
    /// every `%define-command!`/`%declare-plugin!` dynamic and lazy entry.
    pub(crate) registry: CommandRegistry,
    /// Registry of configured language identities.
    pub(crate) languages: LanguageRegistry,
    /// Chars that fire `OnTriggerChar` in Insert mode, keyed by
    /// `(source, language)` — a `(register-trigger-chars! source language
    /// chars)` call only ever replaces its own `(source, language)` entry,
    /// so two languages sharing a source (e.g. completion's `"lsp-
    /// completion"` source registered separately for `"rust"` and
    /// `"python"`) never clobber each other. An empty `chars` removes the
    /// entry entirely (matches `on-lsp-detach`'s clear-on-detach usage).
    pub(crate) trigger_chars: rustc_hash::FxHashMap<(String, String), Vec<char>>,
    /// Steel-writable decoration stores (inlay hints, signs, virtual
    /// lines, EOL text, extra highlights, line backgrounds) — the render
    /// providers read these.
    pub(crate) decorations: decorations::DecorationStores,
    /// Text pushed by `(set-statusline-text! source bid text)`, wholesale
    /// per `(bid, source)`, same replace semantics as `decorations`. Nested
    /// rather than flat like `trigger_chars` above: the render side needs a
    /// borrowed `(bid, &str)` lookup every frame, and a `HashMap` keyed on
    /// `(BufferId, Box<str>)` has no borrowed-key form — flat would force an
    /// allocation per element per frame just to look one up.
    ///
    /// Cleared per-buffer at `close_buffer_and_notify`, alongside
    /// `decorations.remove_buffer` — but deliberately *not* at the other
    /// `decorations.remove_buffer` call site, `:e!`'s reload path: that one
    /// clears decorations because their char offsets are invalidated by the
    /// reload, and pushed statusline text carries no offsets to invalidate.
    pub(crate) statusline_text:
        rustc_hash::FxHashMap<BufferId, rustc_hash::FxHashMap<Box<str>, Box<str>>>,
    /// Deferred Steel work — events enqueued during command dispatch
    /// (`EditorState::queue_event`) and specific-closure completions
    /// (`EditorState::queue_steel_call`: an `lsp-request` callback, a timer
    /// thunk, a prompt callback) — drained in FIFO order by `Editor::settle`.
    /// One queue, not two: see `event::PendingWork`'s doc for why the merge
    /// matters. No work item is ever evaluated inline during command
    /// execution or a completion callback.
    pub(crate) pending_work: VecDeque<event::PendingWork>,
    /// Buffers awaiting language detection, drained by
    /// `Editor::detect_pending_languages`. Detection needs `self.scripting`
    /// (lazy-plugin activation), which the disjoint-borrow buffer-open
    /// chokepoints (`buffer::lifecycle::open_buffer_and_notify` and callers
    /// with only `&mut EditorState`/`&mut EngineView`) never hold — so they
    /// queue the buffer id here instead of detecting inline. Every caller
    /// with a full `&mut Editor` drains this explicitly after opening
    /// buffers; every Steel-eval path drains it at the tail of
    /// `apply_script_effects`.
    pub(crate) pending_language_detection: Vec<hume_engine::pipeline::BufferId>,
    /// In-flight `spawn-async!` jobs, keyed by the id `spawn-async!`
    /// returned to Steel. Drained by `Editor::drain_async_jobs`; dropping
    /// this map (a `:reload-config` wholesale rebuild) kills every
    /// in-flight child for free — see `async_job::PendingJob`'s doc.
    pub(crate) async_jobs: rustc_hash::FxHashMap<u64, async_job::PendingJob>,
    /// Monotonic counter minting the next `spawn-async!` job id — mirrors
    /// the picker's `token`/timer's `TimerId` shape, but lives here (rather
    /// than reusing either) since a job id is neither.
    pub(crate) next_async_job_id: u64,
    /// The `(prompt! …)` callback — persists for as long as `minibuf` holds
    /// the prompt session (unlike a queued `PendingWork::Call`, which drains
    /// the same `settle()` it's pushed to). `handle_command`'s Confirm/Cancel
    /// arms take this and queue exactly one `(callback text-or-#f)` call via
    /// `queue_steel_call`.
    pub(crate) steel_prompt_callback: Option<steel::rvals::SteelVal>,
    /// `(show-popup! text)`'s raw content — resolved into a positioned
    /// `PopupState` each frame by `Editor::sync_popup_view` (geometry needs
    /// the focused pane's *current* rect, so it can't be pre-computed here).
    pub(crate) popup: Option<crate::ui::popup::PopupModel>,
    /// `(show-menu! items on-select)`'s raw content, including the
    /// not-yet-fired Steel callback — cleared by the key intercept in
    /// `handle_key`, not by `sync_menu_view`.
    pub(crate) menu: Option<crate::ui::popup::MenuModel>,
    /// `(show-drawer-list! items on-select)`'s raw content, including the
    /// callback — cleared by `Esc` or `close-drawer!`, *not* by `Enter` (the
    /// drawer stays open across selections, unlike the popup/menu).
    pub(crate) drawer: Option<crate::ui::drawer::DrawerModel>,
    /// The open picker session — driven by the key intercept in `handle_key`;
    /// opened via `editor::picker::open_picker` (Steel's `picker!` builtin,
    /// or directly in tests).
    pub(crate) picker: Option<crate::editor::picker::PickerSession>,
    /// The open native yes/no confirmation, if any — see
    /// [`crate::ui::confirm`]. Mode-agnostic: unlike `menu`/`drawer`, this
    /// intercepts before mode dispatch regardless of `Mode`, since a
    /// disk-change check can fire while the user is mid-Insert.
    pub(crate) confirm: Option<crate::ui::confirm::ConfirmModel>,
}

impl ConfigState {
    /// Build the config state every session, and every `:reload-config`,
    /// starts from — the compiled-in keymap (plus the kitty-only default
    /// binds when the terminal supports the protocol, matching
    /// [`Editor::set_kitty_support`]) and the compiled-in command registry,
    /// with every other field at its empty/`None` default.
    ///
    /// `prior_clock` is `0` at session start (nothing to carry forward) and
    /// the outgoing `ConfigState.decorations`'s own shared clock on
    /// `:reload-config` — see [`decorations::DecorationStores::reset`]'s
    /// doc for why this can't just be `Default::default()` like every other
    /// field here.
    pub(super) fn new(kitty_enabled: bool, prior_clock: u64) -> Self {
        Self {
            keymap: default_keymap_for(kitty_enabled),
            registry: CommandRegistry::with_defaults(),
            languages: LanguageRegistry::new(),
            trigger_chars: rustc_hash::FxHashMap::default(),
            decorations: decorations::DecorationStores::reset(prior_clock),
            statusline_text: rustc_hash::FxHashMap::default(),
            pending_work: VecDeque::new(),
            pending_language_detection: Vec::new(),
            async_jobs: rustc_hash::FxHashMap::default(),
            next_async_job_id: 0,
            steel_prompt_callback: None,
            popup: None,
            menu: None,
            drawer: None,
            picker: None,
            confirm: None,
        }
    }

    /// Close an open `Scrollable` popup, leaving a `Sticky` one alone. Called
    /// from both input paths (`Editor::handle_key`, `Editor::handle_mouse`):
    /// hover-style content is pinned to a cursor position the very event
    /// dismissing it is about to move, so it would otherwise stay painted
    /// describing a symbol the cursor has left. A `Sticky` popup (signature
    /// help) belongs to an ongoing Insert session instead, and is closed by
    /// the `on-mode-change` hook.
    pub(super) fn dismiss_scrollable_popup(&mut self) {
        if matches!(
            self.popup.as_ref().map(|p| p.kind),
            Some(hume_scripting::host::PopupKind::Scrollable)
        ) {
            self.popup = None;
        }
    }
}

/// The keymap every session and every `:reload-config` starts from: the
/// compiled-in trie, plus the kitty-only default binds when the terminal
/// supports the protocol. Shared by [`ConfigState::new`] (session start /
/// reload) and [`Editor::set_kitty_support`] (which re-derives the keymap
/// once the terminal probe result is known, before `init.scm` can override
/// it) so the two can't drift apart on what "kitty defaults installed"
/// means.
pub(super) fn default_keymap_for(kitty_enabled: bool) -> Keymap {
    let mut keymap = Keymap::default();
    if kitty_enabled {
        keymap.apply_kitty_defaults();
    }
    keymap
}

// ── EditorState ───────────────────────────────────────────────────────────────
//
// All command-mutable editor data. Separated from `Editor` so the Steel VM
// (`scripting.steel`) and editor data are sibling borrows that never alias —
// enabling EditorCmd to dispatch synchronously from within a Steel eval.

pub(crate) struct EditorState {
    /// All open buffers. SSOT for buffer content, history, and file metadata.
    pub(crate) buffers: BufferStore,
    /// Config-owned state reset wholesale by `:reload-config` — see
    /// [`ConfigState`]'s doc for exactly what that means and why it's a
    /// separate struct.
    pub(crate) config: ConfigState,
    /// Current editing mode. `EditorMode::Extend` represents the sticky extend
    /// state. Mode is the single source of truth for whether extend is active.
    /// Private: all transitions go through [`EditorState::set_mode`].
    mode: Mode,
    /// Keys consumed so far in the current multi-key sequence (max depth 3).
    pub(super) pending_keys: Vec<KeyEvent>,
    /// Accumulated numeric prefix for the next command (e.g. `3` in `3w`).
    pub(super) count: Option<usize>,
    /// Pending wait-char state for a f/t/F/T/r binding.
    pub(super) wait_char: Option<WaitCharPending>,
    /// Character argument for the current parameterized command (find/till/replace).
    pub(super) pending_char: Option<char>,
    pub(super) registers: RegisterSet,
    /// Kill ring — bounded history of yanked / deleted text.
    pub(super) kill_ring: KillRing,
    /// Wrapper around the OS clipboard (`arboard`).
    pub(super) clipboard: clipboard::SystemClipboard,
    /// State machine for the two-keystroke `"<reg>` register-prefix sequence.
    pub(super) register_prefix: Option<register_ops::RegisterPrefix>,
    /// Which source a bare paste reads next, and until when — see
    /// [`commands::PasteStamp`].
    pub(super) paste_stamp: Option<commands::PasteStamp>,
    pub(super) should_quit: bool,
    /// Set by the platform terminator thread to the process exit code when a
    /// signal asks the editor to quit — `0` means "no termination requested"
    /// (never a valid signal-termination exit code). Polled at the top of the
    /// run loop and re-read by `hume_editor::run` after it returns, so both
    /// sides use the same code without a second channel. `should_quit` stays
    /// the single-threaded, in-editor quit path (dirty-buffer prompts, `:q`
    /// semantics) — a signal bypasses all of that.
    pub(super) terminate_exit_code: Arc<AtomicI32>,
    /// Active when the user is typing a command (`:`) or a search (`/`).
    pub(crate) minibuf: Option<MiniBuffer>,
    /// Active completion session while a popup is showing.
    pub(crate) minibuf_completion: Option<completion::MinibufCompletionState>,
    /// Transient one-line message shown in the statusline after an action.
    pub(crate) status_msg: Option<String>,
    /// Keystrokes the message-log summary stays visible before auto-dismissing.
    /// Armed when `status_msg` clears with unseen entries; ticked down in `handle_key`.
    pub(crate) summary_ttl: u8,
    /// Persistent log of warnings, errors, and trace entries.
    pub(crate) message_log: MessageLog,
    /// All editor settings — global defaults and per-buffer-overridable values.
    pub(crate) settings: EditorSettings,
    /// The character and kind from the last find/till motion.
    pub(super) last_find: Option<commands::FindChar>,
    pub(super) search: SearchState,
    /// The single pane focused in the current editing session.
    pub(crate) focused_pane_id: PaneId,
    /// Per-pane maps: (pane,buffer) selections/groups, transient mode snapshots, jump history.
    pub(super) panes: PaneView,
    /// Bounded, in-memory history for `:`, `/`, and `?` prompts.
    pub(super) history: self::minibuf::history::HistoryStore,
    /// Set by the inline-output dispatch arm to trigger a full repaint.
    pub(crate) force_full_redraw: bool,
    /// State of the `#:inline-output` bracket for the Steel command currently
    /// being dispatched. Set just before `call_steel_cmd`; read and driven by
    /// `EditorHostImpl::ensure_inline_output_screen` / `is_inline_output_command`
    /// so `SteelCtx` (and the gated print shims) know it's safe to
    /// write to the real stdout, and so the screen is only entered lazily, on
    /// the first byte of actual output. See [`InlineOutputDispatch`].
    pub(crate) inline_output: InlineOutputDispatch,
    /// Test-only seam: flips `true` when a command body actually enters the
    /// inline-output terminal bracket (via `ensure_inline_output_screen`).
    /// Lets tests assert the bracket was skipped (rather than merely that it
    /// didn't hang, which depends on whether stdin happens to be a TTY)
    /// without capturing real terminal I/O.
    #[cfg(test)]
    pub(crate) inline_output_entered: bool,
    /// Reusable scratch buffer for format operations in visual-line movement.
    pub(super) motion_format_scratch: hume_engine::format::FormatScratch,
    /// Reusable sticky-column buffer for vertical motion — shared by all
    /// three units `apply_visual_vertical` handles (row-domain `j`/`k`,
    /// scroll/wheel, and buffer-line `9j`/`9k`).
    pub(super) visual_move_target_display_cols: Vec<u32>,
    /// The last repeatable editing action, available for replay via `.`.
    pub(super) last_repeatable_action: Option<RepeatableAction>,
    /// Accumulating selection-recipe buffer for the *next* edit's dot-repeat.
    ///
    /// Tracks how the current selection was built: Motion/Selection commands
    /// append or reset this buffer; a repeatable edit snapshots it into
    /// `RepeatableAction::selection_recipe` and clears it — the native path
    /// (`step_snapshot_recipe`) via `mem::take`, the Steel path
    /// (`Editor::dispatch`) via `.clone()` followed by an explicit clear, so
    /// an inner `call!` dispatch still sees the pre-body value to compose
    /// onto. Non-selection commands clear it. See
    /// `RepeatableAction::selection_recipe` for the invariant.
    pub(super) selection_recipe: Vec<SelectionStep>,
    /// Incremented once by every native dispatch's `step_update_recipe`
    /// (`commands/pipeline.rs`), regardless of what it did to the recipe.
    ///
    /// `Editor::dispatch`'s Steel branch reads this before and after running
    /// a command's body: unchanged means the body dispatched no native
    /// command at all (a pure-Steel body), so `selection_recipe` — left
    /// untouched from before the body ran — must still be cleared, the same
    /// as any other non-selection command would. Changed means an inner
    /// `call!` already ran `step_update_recipe` with its own correct
    /// decision, which the Steel branch must not override. A plain
    /// `selection_recipe == pre_recipe` snapshot comparison can't stand in
    /// for this: it would misfire when a body re-establishes the identical
    /// step it started with.
    pub(super) selection_recipe_writes: u64,
    /// Set by `refuse_if_read_only` (and by a native `EditorCmd` body
    /// returning `Err`) to tell `run_dispatch_pipeline`'s AFTER stage the
    /// command's body did not do its job — a read-only refusal, or an error
    /// mid-body. A repeatable command in that state must not stamp
    /// `last_repeatable_action`: there is nothing new to repeat, and doing so
    /// would silently discard whatever real action was recorded before (see
    /// `commands/pipeline.rs`'s `step_stamp_repeatable` call site).
    /// `run_dispatch_pipeline` resets this to `false` at its own BEFORE stage
    /// and is the only reader, immediately after BODY in that same call — the
    /// Steel dispatch path (`Editor::dispatch`) never reads or resets it
    /// directly, but any native `call!` it makes goes through
    /// `run_dispatch_pipeline` too, so a stale value from an earlier dispatch
    /// can never leak in.
    pub(super) command_refused: bool,
    /// Deferred dot-repeat job enqueued by `cmd_repeat`; consumed by
    /// `replay_dot` at the tail of `handle_key`.
    pub(super) pending_repeat: Option<PendingRepeat>,
    /// Active insert session, present between begin/end_insert_session.
    pub(super) insert_session: Option<InsertSession>,
    /// `true` when the cursor's current line's indent was auto-inserted by
    /// this insert session (an `insert_newline_indent` copy) and nothing has
    /// been typed on it since — the condition under which exiting Insert
    /// mode should vacate that indent (vim autoindent parity: `:help
    /// autoindent`, "if you do not type anything on the new line except
    /// `<BS>` ... the indent is deleted again"). Reset on session start, set
    /// by the Enter key handler, cleared by any other content-modifying key.
    /// Lives on `EditorState` rather than [`InsertSession`] because dot-repeat
    /// replay re-dispatches keys through the same key handlers with no
    /// `InsertSession` present (see `replay_dot`), so it must be visible
    /// there too.
    pub(super) autoindent_pending: bool,
    /// Whether the user explicitly typed a count prefix before the current command.
    pub(super) explicit_count: bool,
    /// `true` when the current multi-key sequence began with a kitty one-shot
    /// Ctrl+key that resolved to a prefix (Interior) node. Cleared on sequence
    /// completion or abort. At Leaf resolution, only applied if the command is
    /// extendable.
    pub(super) pending_ctrl_extend: bool,
    /// Active macro recording session.
    pub(super) macro_recording: Option<(char, Vec<KeyEvent>)>,
    /// Pending two-keystroke macro command.
    pub(super) macro_pending: Option<MacroPending>,
    /// Queue of keys to replay before reading the next terminal event.
    pub(super) replay_queue: VecDeque<KeyEvent>,
    /// Single-frame flag: skip recording the current key.
    pub(super) skip_macro_record: bool,
    /// `true` for the duration of a typed (`:`) command's synchronous
    /// dispatch. `execute_command` runs while `mode` still reads
    /// `Mode::Command` — it only flips back to `Normal` afterward — so this
    /// is the one signal that tells "a fully-submitted command is running"
    /// apart from "the user is still typing an unsubmitted command line".
    /// `check_buffer_disk_state`'s confirm gate is the only reader: a
    /// disk-change confirm may open mid-dispatch (`:e`/`:b`/`:bn`/`:bp`/
    /// `:checktime` all rely on this), but never while the user is simply
    /// sitting in Command mode with the next keystroke still meant for the
    /// minibuffer.
    pub(super) dispatching_typed_command: bool,
    /// `true` while draining the replay queue.
    pub(super) is_replaying: bool,
    /// Set by `Editor::handle_input` right after dispatch whenever that
    /// input logged a new warning or error, cleared by `Editor::settle` once
    /// its drain (including the buffer-enter disk check) has run — the
    /// window spans "input dispatched" to "its consequences settled", not
    /// one call. Read only by `can_open_confirm`, so a command's own failure
    /// message (`:qa` naming the first dirty buffer) can't be silently
    /// replaced by an unrelated disk-change confirm opened by the focus move
    /// that triggered it. Always `false` outside that window.
    pub(super) message_logged_this_input: bool,
    /// The buffer that owned focus as of the last `Editor::settle()` pass —
    /// `None` only before the first `settle()` ever runs, so the startup
    /// buffer legitimately raises `OnBufferEnter` (matching Vim's
    /// `BufEnter`). The single observation baseline for the focus diff; see
    /// `Editor::detect_buffer_enter`.
    pub(super) last_entered_buffer: Option<BufferId>,
    /// Anchor char offset set on mouse-left-down when `mouse_select` is enabled.
    pub(super) mouse_drag_anchor: Option<usize>,
    /// Current working directory. Set at startup; updated by `:cd`.
    pub(super) cwd: PathBuf,
    /// Set by `set_mode` on any exit from Insert — `set_mode` only has
    /// `&mut EditorState` (many callers are free functions that never touch
    /// `Editor`/`LspState`), but the LSP completion session it must dismiss
    /// lives on `LspState`. Consumed (session + ui + view all cleared)
    /// by `Editor::take_pending_lsp_completion_dismiss`, called
    /// unconditionally from `handle_key`, `handle_mouse`, and (top and tail)
    /// `Editor::settle` — the latter is called every frame by every settle
    /// site, so no separate render-time call is needed. Same deferral
    /// channel philosophy as `pending_work`.
    pub(super) lsp_completion_dismiss_pending: bool,
    /// Shared view for the LSP completion menu — reuses the popup/selection
    /// menu's generic
    /// `PopupState`/`PopupOverlay` (selected-row styling, same as the
    /// selection menu) via its own `Arc` and pane registration.
    pub(crate) completion_menu_view: Arc<RwLock<Option<crate::ui::popup::PopupState>>>,
    /// Shared completion-popup view: written by `prepare_frame`, read by provider.
    pub(crate) minibuf_completion_view:
        Arc<RwLock<Option<crate::ui::completion_overlay::MinibufCompletionView>>>,
    /// Shared popup-overlay view for `PopupLayout::Cursor`: written by
    /// `prepare_frame`, read by `PopupOverlay`. Empty whenever `config.popup`
    /// is `None` or docked (see `popup_band_view`).
    pub(crate) popup_view: Arc<RwLock<Option<crate::ui::popup::PopupState>>>,
    /// Shared popup-band view for `PopupLayout::Docked`: written by
    /// `prepare_frame`, read by `PopupBandWidget` (chrome, like the
    /// drawer). Empty whenever `config.popup` is `None` or cursor-anchored.
    pub(crate) popup_band_view: Arc<RwLock<Option<crate::ui::popup::PopupBandState>>>,
    /// Shared menu-overlay view: written by `prepare_frame`, read by its own
    /// `PopupOverlay` registration (separate from the hover popup's, so both
    /// can in principle show at once — the menu paints on top).
    pub(crate) menu_view: Arc<RwLock<Option<crate::ui::popup::PopupState>>>,
    /// Shared drawer-overlay view: written every frame by `prepare_frame`
    /// (self-healing against a direct `self.state.config.drawer = None` that
    /// bypasses the mutation-site sync — see `sync_drawer_view`'s doc), read
    /// by `DrawerWidget`.
    pub(crate) drawer_view: Arc<RwLock<Option<crate::ui::drawer::DrawerViewState>>>,
    /// Shared picker-overlay view: written per-frame by `sync_picker_view`
    /// (geometry depends on the current panes region, like popup/menu, not
    /// on-change like the drawer), read by `PickerOverlay`.
    pub(crate) picker_view: Arc<RwLock<Option<crate::ui::picker_panel::PickerViewState>>>,
    /// Cross-thread waker clone (see `Editor::open`'s `wake` param), reachable
    /// here so `EditorHostImpl` — which only ever holds a disjoint `&mut
    /// EditorState` borrow, never a whole `&mut Editor` — can hand it to a
    /// spawned picker source (`picker-source-spawn!`) so its reader thread
    /// can wake the event loop. A no-op `Arc` in tests/headless.
    pub(super) wake: Arc<dyn Fn() + Send + Sync>,
}

/// The trivial-field baseline both `EditorState` constructors build on.
///
/// Not a usable editor on its own — no buffers, no panes, a null
/// `focused_pane_id`, a clipboard with no handle, and a no-op waker. It exists
/// so the fields that are identical at both construction sites (`Editor::open`
/// and `Editor::for_testing`) are written once. Every field whose real value
/// differs between those two sites is set here to its inert (test) form and
/// named explicitly by `Editor::open`, so no production value is inherited by
/// accident. A field added later whose production value must differ from its
/// baseline needs the same treatment — the compiler will not ask for it.
impl Default for EditorState {
    fn default() -> Self {
        let settings = EditorSettings::default();
        let history_capacity = settings.history_capacity;
        Self {
            buffers: BufferStore::new(),
            // `kitty_enabled: false` matches: the real probe result isn't known
            // until `set_kitty_support` runs, after `Editor::open`.
            config: ConfigState::new(false, 0),
            mode: Mode::Normal,
            pending_keys: Vec::new(),
            count: None,
            wait_char: None,
            pending_char: None,
            registers: RegisterSet::new(),
            kill_ring: KillRing::new(),
            clipboard: clipboard::SystemClipboard::new_unavailable(),
            register_prefix: None,
            paste_stamp: None,
            should_quit: false,
            terminate_exit_code: Arc::new(AtomicI32::new(0)),
            minibuf: None,
            minibuf_completion: None,
            status_msg: None,
            summary_ttl: 0,
            message_log: MessageLog::new(),
            settings,
            last_find: None,
            search: SearchState::default(),
            focused_pane_id: PaneId::default(),
            panes: PaneView::default(),
            history: minibuf::history::HistoryStore::new(history_capacity),
            force_full_redraw: false,
            inline_output: InlineOutputDispatch::Inactive,
            #[cfg(test)]
            inline_output_entered: false,
            motion_format_scratch: hume_engine::format::FormatScratch::new(),
            visual_move_target_display_cols: Vec::new(),
            last_repeatable_action: None,
            selection_recipe: Vec::new(),
            selection_recipe_writes: 0,
            command_refused: false,
            pending_repeat: None,
            insert_session: None,
            autoindent_pending: false,
            explicit_count: false,
            pending_ctrl_extend: false,
            macro_recording: None,
            macro_pending: None,
            replay_queue: VecDeque::new(),
            skip_macro_record: false,
            dispatching_typed_command: false,
            is_replaying: false,
            message_logged_this_input: false,
            last_entered_buffer: None,
            mouse_drag_anchor: None,
            cwd: PathBuf::new(),
            lsp_completion_dismiss_pending: false,
            completion_menu_view: Arc::new(RwLock::new(None)),
            minibuf_completion_view: Arc::new(RwLock::new(None)),
            popup_view: Arc::new(RwLock::new(None)),
            popup_band_view: Arc::new(RwLock::new(None)),
            menu_view: Arc::new(RwLock::new(None)),
            drawer_view: Arc::new(RwLock::new(None)),
            picker_view: Arc::new(RwLock::new(None)),
            wake: Arc::new(|| {}),
        }
    }
}

impl EditorState {
    // ── Mode ──────────────────────────────────────────────────────────────────

    pub(crate) fn mode(&self) -> Mode {
        self.mode
    }

    // ── Quit ──────────────────────────────────────────────────────────────────

    /// Unconditional quit-the-whole-editor. Shared by `:qa!`'s force path and
    /// the `force-quit` named command — both mean "quit all, no confirmation".
    pub(crate) fn request_quit(&mut self) {
        self.should_quit = true;
    }

    // ── Drawer ──────────────────────────────────────────────────────────

    /// Mirror `self.config.drawer` into `self.drawer_view` for `DrawerWidget`
    /// to read. Called directly at every drawer mutation site (open,
    /// selection move, scroll, close) for immediacy, *and* unconditionally
    /// every frame from `Editor::prepare_frame` (like the popup/menu/picker
    /// `sync_*_view`s) so the view can never drift from the model — in
    /// particular, so a direct `self.state.config.drawer = None` (as
    /// `reset_config_state`'s wholesale `ConfigState` rebuild does,
    /// bypassing `close-drawer!`'s callback queueing) can't leave a stale
    /// view painting a closed drawer.
    pub(super) fn sync_drawer_view(&self) {
        let resolved = self
            .config
            .drawer
            .as_ref()
            .map(|d| crate::ui::drawer::DrawerViewState {
                rows: Arc::clone(&d.items),
                selected: d.selected,
                scroll: d.scroll,
            });
        *self.drawer_view.write_or_panic() = resolved;
    }

    /// Every source registered for `(ch, language)` — `OnTriggerChar`'s fire
    /// site (mappings/insert.rs) fires once per entry, so two sources
    /// registering the same char for the same language each get their own
    /// hook fire. A buffer with no language (`language: None`) never
    /// matches anything — trigger chars are always server-derived, and a
    /// server attach implies a language.
    pub(crate) fn trigger_sources_for(&self, ch: char, language: Option<&str>) -> Vec<String> {
        let Some(language) = language else {
            return Vec::new();
        };
        self.config
            .trigger_chars
            .iter()
            .filter(|((_, lang), chars)| lang == language && chars.contains(&ch))
            .map(|((source, _), _)| source.clone())
            .collect()
    }

    /// Single write path for all mode transitions.
    ///
    /// Captures the old mode, writes the new one, and enqueues `OnModeChange`
    /// for firing by `Editor::settle` at the next drain. The no-op guard
    /// prevents spurious hook fires when mode is already correct.
    ///
    /// The `mode` field is private so the compiler enforces that every
    /// transition goes through here.
    pub(crate) fn set_mode(&mut self, new: Mode) {
        let old = self.mode;
        if old == new {
            return;
        }
        // Any exit from Insert dismisses an open completion session —
        // `handle_completion_key`'s own `Esc`/Enter paths never reach here
        // (they return before the trie's `exit-insert` runs), so this
        // catches every *other* way Insert ends (Ctrl+C, a mouse click, a
        // Steel-triggered mode change) while a session happens to be open.
        // Deferred: the session lives on `LspState`, which `set_mode` (only
        // `&mut EditorState`) can't reach — `Editor::
        // take_pending_lsp_completion_dismiss` consumes this at every
        // chokepoint before the next render.
        if old == Mode::Insert {
            self.lsp_completion_dismiss_pending = true;
        }
        self.mode = new;
        self.queue_event(event::EditorEvent::OnModeChange { from: old, to: new });
    }

    /// Enqueue `event` to fire after the current command returns — the
    /// single raise path every event goes through, reached as
    /// `self.state.queue_event(…)` from `Editor` methods and directly, like
    /// `set_mode` above, from methods that only hold `&mut EditorState`.
    pub(crate) fn queue_event(&mut self, event: event::EditorEvent) {
        self.config
            .pending_work
            .push_back(event::PendingWork::Event(event));
    }

    /// Queue `(proc, args)` for evaluation at the next drain boundary —
    /// never called inline (LSP dispatch, timer fire, and minibuffer key
    /// handling all detect their completion from inside a borrow that can't
    /// re-enter Steel). Shared delivery mechanism for the `lsp-request`
    /// callback, timer thunks, and the prompt/menu/drawer/picker callbacks.
    /// Lives on `EditorState` (not `Editor`) so `picker::close_picker` and
    /// `EditorHostImpl`'s spawn-failure arm — which only hold `&mut
    /// EditorState` — can reach it too, the same reason `queue_event` lives
    /// here.
    pub(crate) fn queue_steel_call(
        &mut self,
        proc: steel::rvals::SteelVal,
        args: Vec<steel::rvals::SteelVal>,
    ) {
        self.config
            .pending_work
            .push_back(event::PendingWork::Call(proc, args));
    }
}

// ── Editor ────────────────────────────────────────────────────────────────────

pub(crate) struct Editor {
    /// All command-mutable editor data. Disjoint from `scripting` so Steel evals
    /// can borrow `state` and `scripting.steel` simultaneously without aliasing.
    pub(crate) state: EditorState,
    /// Engine rendering state: layout, panes, buffers, theme.
    pub(crate) view: EngineView,
    /// Whether the kitty keyboard protocol was successfully activated at startup.
    pub(crate) kitty_enabled: bool,
    /// The embedded Steel scripting host.
    pub(super) scripting: Option<hume_scripting::ScriptingHost>,
    /// `--config FILE` override for the config path `init_scripting` (and
    /// every later `:reload-config`) evaluates. `None` falls back to
    /// `<config_dir>/init.scm`. Set once via `set_config_path`, before the
    /// first `init_scripting` call — outlives startup so a reload re-runs
    /// the same file the session booted from.
    pub(super) config_path_override: Option<std::path::PathBuf>,
    /// Snapshot of Rust-builtin command names taken at end of `init_scripting`.
    pub(super) builtin_cmd_names: rustc_hash::FxHashSet<String>,
    /// Parse backend: threaded in production, synchronous-inline in tests.
    parse_worker: Box<dyn ParseBackend>,
    /// Whether the one-shot "parse worker disconnected" message has been logged.
    parse_worker_disconnect_logged: bool,
    /// Nearest-deadline timer registry; Steel-visible via the
    /// `after`/`debounce` builtins.
    timer_wheel: timers::TimerWheel,
    /// `TimerId -> {Steel thunk, or native action}`, keeping `timers.rs`
    /// itself payload-agnostic. Entry removed on fire or cancel — never
    /// leaked.
    timer_payloads: rustc_hash::FxHashMap<timers::TimerId, timer_bridge::TimerPayload>,
    /// This pane's currently-pending `OnViewportChange` debounce timer, if
    /// any — looked up to cancel-and-replace on the next change.
    viewport_debounce: rustc_hash::FxHashMap<hume_engine::pipeline::PaneId, timers::TimerId>,
    /// `(top_line, height)` as of the last frame, per pane — `prepare_frame`'s
    /// scroll step compares against this to detect a real viewport change
    /// worth debouncing, rather than firing every frame regardless.
    last_viewport_key: rustc_hash::FxHashMap<hume_engine::pipeline::PaneId, (usize, u16)>,
    /// `(buffer_id, decorations.generation(buffer_id))` as of each
    /// pane's last mirror into its `PaneVirtualLines` Arc — `prepare_frame`
    /// compares against this to skip the rebuild on frames where neither
    /// that buffer's stamp nor the pane's buffer changed, since this runs in
    /// scroll/cursor math too, not just render. The buffer id is part of the
    /// key so a pane switching buffers always rebuilds, even onto a buffer
    /// whose stamp happens to match the old one — otherwise it would keep
    /// mirroring the previous buffer's virtual lines.
    virtual_lines_synced: rustc_hash::FxHashMap<hume_engine::pipeline::PaneId, (BufferId, u64)>,
    /// LSP backend + client state: threaded in production,
    /// synchronous-inline in tests, mirroring `parse_worker` above.
    lsp: lsp::LspState,
    /// `true` once [`Editor::run`] has taken ownership of the terminal (the
    /// interactive event loop). Tests and headless `run_keys` dispatch
    /// commands directly and never enter `run`, so this stays `false` there —
    /// dispatch uses it to skip the inline-output terminal bracket (alt-screen
    /// toggle + "press any key to return" block) when there is no TUI to
    /// suspend and no interactive user to press a key.
    tui_active: bool,
    /// The shared terminal handle `run` reads/writes and the inline-output
    /// bracket (`host_impl.rs`, `dispatch.rs`) borrows to leave/re-enter the
    /// alt-screen. `Some` once [`Editor::attach_terminal`] has been called
    /// (always paired with entering `run`); `None` from `for_testing` and
    /// headless `run_keys` — those dispatch directly and never enter `run`,
    /// so there is no terminal to attach.
    terminal: Option<hume_platform::terminal::SharedTerm>,
    /// `(mouse_enabled, mouse_select)` as last applied to `terminal`'s mouse
    /// tracking mode. `prepare_frame` compares this against the live
    /// `state.settings` values every frame and re-applies the terminal mode
    /// when they differ, so `:set global mouse-enabled=…`/`mouse-select=…`
    /// take effect immediately instead of only at the next restart — see
    /// `hume_platform::terminal::set_mouse_mode`.
    applied_mouse_mode: (bool, bool),
}

impl Editor {
    // ── Buffer accessors ──────────────────────────────────────────────────────

    /// The `BufferId` the focused pane is currently viewing.
    pub(crate) fn focused_buffer_id(&self) -> BufferId {
        self.view.panes[self.state.focused_pane_id].buffer_id
    }

    /// Shared reference to the focused buffer.
    pub(crate) fn doc(&self) -> &Buffer {
        self.state.buffers.get(self.focused_buffer_id())
    }

    /// The most-recently-focused buffer other than the current one, or `None`
    /// when only one buffer is open. Derives from `BufferStore.mru` (SSOT).
    pub(crate) fn alternate_buffer(&self) -> Option<BufferId> {
        self.state.buffers.mru_excluding(self.focused_buffer_id())
    }

    /// Mutable reference to the focused buffer.
    ///
    /// Uses a split borrow — `buffers` and other fields on `Editor` are
    /// disjoint, so you can hold this reference while reading e.g. `self.state.settings`.
    /// Do NOT keep this reference live across a call that also borrows `self`.
    pub(crate) fn doc_mut(&mut self) -> &mut Buffer {
        let bid = self.focused_buffer_id();
        self.state.buffers.get_mut(bid)
    }

    /// `true` when the focused buffer rejects user edits.
    pub(crate) fn focused_buffer_read_only(&self) -> bool {
        self.doc().is_read_only()
    }

    /// The focused pane's selections for the current buffer.
    pub(super) fn current_selections(&self) -> &SelectionSet {
        &self.state.panes.state[self.state.focused_pane_id][self.focused_buffer_id()].selections
    }

    /// Replace the focused pane's selections for the current buffer.
    pub(super) fn set_current_selections(&mut self, sels: SelectionSet) {
        commands::set_current_selections(&mut self.state, &self.view, sels);
    }

    // ── Mode transitions ──────────────────────────────────────────────────────

    pub(super) fn end_insert_session(&mut self) {
        commands::end_insert_session(&mut self.state, &self.view);
    }

    /// Set the editing mode. The cursor shape reflecting the new mode will be
    /// emitted after the current frame's draw call.
    ///
    /// Enqueues `OnModeChange` through the unified `pending_work` channel
    /// (same path as the `EditorCmd` handlers); `settle` fires it at the
    /// next drain.
    ///
    /// For Insert mode entry and exit use `begin_insert_session` and
    /// [`crate::editor::commands::end_insert_session`] instead — they manage
    /// the undo group and dot-repeat recording alongside the mode change.
    pub(super) fn set_mode(&mut self, mode: Mode) {
        self.state.set_mode(mode);
    }
}

#[cfg(test)]
mod tests;
