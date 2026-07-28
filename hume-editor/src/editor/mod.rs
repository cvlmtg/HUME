use std::borrow::Cow;
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
use crate::ops::register::{KillRing, RegisterSet};
use crate::settings::EditorSettings;
use hume_editing::selection::SelectionSet;
use hume_treesitter::parse_worker::ParseBackend;
use hume_treesitter::registry::LanguageRegistry;

use self::keymap::{Keymap, WaitCharPending};

mod async_source;
pub(crate) mod error;
pub(crate) mod host_impl;
mod lifecycle;
mod scripting_setup;

pub(crate) mod buffer;
mod clipboard;
mod commands;
pub(crate) mod completion;
pub(crate) mod cursor;
pub(crate) mod decorations;
mod dispatch;
pub(crate) mod doc_ops;
pub(crate) mod fuzzy;
pub(crate) mod jump_list;
pub mod keymap;
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

pub(crate) use search::{SearchDirection, SearchState};

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

// ── EditorState ───────────────────────────────────────────────────────────────
//
// All command-mutable editor data. Separated from `Editor` so the Steel VM
// (`scripting.steel`) and editor data are sibling borrows that never alias —
// enabling EditorCmd to dispatch synchronously from within a Steel eval.

pub(crate) struct EditorState {
    /// All open buffers. SSOT for buffer content, history, and file metadata.
    pub(crate) buffers: BufferStore,
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
    /// Name of the most recently dispatched command.
    pub(super) last_command: Option<Cow<'static, str>>,
    /// Values of the most recent paste.
    pub(super) last_paste: Option<Vec<String>>,
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
    /// Registry of all mappable commands (motions, selections, edits).
    pub(super) registry: CommandRegistry,
    /// The trie-based keymap for each mode.
    pub(super) keymap: Keymap,
    /// The character and kind from the last find/till motion.
    pub(super) last_find: Option<commands::FindChar>,
    pub(super) search: SearchState,
    /// The single pane focused in the current editing session.
    pub(crate) focused_pane_id: PaneId,
    /// Per-pane maps: (pane,buffer) selections/groups, transient mode snapshots, jump history.
    pub(super) panes: PaneView,
    /// Bounded, in-memory history for `:`, `/`, and `?` prompts.
    pub(super) history: self::minibuf::history::HistoryStore,
    /// Set by the inline-output dispatch arm to trigger a full ratatui repaint.
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
    /// Reusable sticky-column buffer for visual j/k movement.
    pub(super) visual_move_target_cols: Vec<u16>,
    /// The last repeatable editing action, available for replay via `.`.
    pub(super) last_repeatable_action: Option<RepeatableAction>,
    /// Accumulating selection-recipe buffer for the *next* edit's dot-repeat.
    ///
    /// Tracks how the current selection was built: Motion/Selection commands
    /// append or reset this buffer; repeatable edits snapshot it into
    /// `RepeatableAction::selection_recipe` (via `mem::take`) and clear it.
    /// Non-selection commands clear it. Invariant: `[]` or
    /// `[Move-establish, Extend*]`.
    pub(super) selection_recipe: Vec<SelectionStep>,
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
    /// `true` while draining the replay queue.
    pub(super) is_replaying: bool,
    /// Anchor char offset set on mouse-left-down when `mouse_select` is enabled.
    pub(super) mouse_drag_anchor: Option<usize>,
    /// Registry of configured language identities.
    pub(super) languages: LanguageRegistry,
    /// Current working directory. Set at startup; updated by `:cd`.
    pub(super) cwd: PathBuf,
    /// Hooks enqueued during command dispatch, drained by `Editor::drain_hooks`
    /// after each command. The unified firing path — `fire_hook_silent` pushes
    /// here; no hook fires inline during command execution.
    pub(super) pending_hooks: Vec<(hume_scripting::hooks::HookId, Vec<steel::rvals::SteelVal>)>,
    /// Buffers awaiting language detection, drained by
    /// `Editor::detect_pending_languages`. Detection needs `self.scripting`
    /// (lazy-plugin activation), which the disjoint-borrow buffer-open
    /// chokepoints (`buffer::lifecycle::open_buffer_and_notify` and callers
    /// with only `&mut EditorState`/`&mut EngineView`) never hold — so they
    /// queue the buffer id here instead of detecting inline. Every caller
    /// with a full `&mut Editor` drains this explicitly after opening
    /// buffers; every Steel-eval path drains it at the tail of
    /// `apply_script_effects`.
    pub(super) pending_language_detection: Vec<hume_engine::pipeline::BufferId>,
    /// Rust-side completions that must reach a *specific* Steel closure
    /// rather than every handler for a hook id: an `lsp-request` callback,
    /// a timer thunk, a prompt callback. Queued (never evaluated
    /// inline — same discipline as `pending_hooks`) by whichever completion
    /// fires, drained by `Editor::drain_pending_steel_calls`.
    pub(super) pending_steel_calls: Vec<(steel::rvals::SteelVal, Vec<steel::rvals::SteelVal>)>,
    /// Chars that fire `OnTriggerChar` in Insert mode, keyed by
    /// `(source, language)` — a `(register-trigger-chars! source language
    /// chars)` call only ever replaces its own `(source, language)` entry,
    /// so two languages sharing a source (e.g. completion's `"lsp-
    /// completion"` source registered separately for `"rust"` and
    /// `"python"`) never clobber each other. An empty `chars` removes the
    /// entry entirely (matches `on-lsp-detach`'s clear-on-detach usage).
    pub(super) trigger_chars: rustc_hash::FxHashMap<(String, String), Vec<char>>,
    /// Steel-writable decoration stores (inlay hints, signs, virtual
    /// lines, extra highlights) — the render providers read these.
    pub(super) decorations: decorations::DecorationStores,
    /// The `(prompt! …)` callback — persists for as long as `minibuf` holds
    /// the prompt session (unlike `pending_steel_calls`, which drains the
    /// same frame it's pushed to). `handle_command`'s Confirm/Cancel arms
    /// take this and push exactly one `(callback text-or-#f)` call onto
    /// `pending_steel_calls`.
    pub(super) steel_prompt_callback: Option<steel::rvals::SteelVal>,
    /// Set by `set_mode` on any exit from Insert — `set_mode` only has
    /// `&mut EditorState` (many callers are free functions that never touch
    /// `Editor`/`LspState`), but the LSP completion session it must dismiss
    /// now lives on `LspState`. Consumed (session + ui + view all cleared)
    /// by `Editor::take_pending_lsp_completion_dismiss`, called
    /// unconditionally from `handle_key`, `handle_mouse`, `drain_hooks`, and
    /// `drain_pending_steel_calls` — the last of which `prepare_frame` also
    /// calls every frame, so no separate render-time call is needed. Same
    /// deferral channel philosophy as `pending_hooks`.
    pub(super) lsp_completion_dismiss_pending: bool,
    /// Shared view for the LSP completion menu — reuses the popup/selection
    /// menu's generic
    /// `PopupState`/`PopupOverlay` (selected-row styling, same as the
    /// selection menu) via its own `Arc` and pane registration.
    pub(crate) completion_menu_view: Arc<RwLock<Option<crate::ui::popup::PopupState>>>,
    /// Shared completion-popup view: written by `prepare_frame`, read by provider.
    pub(crate) minibuf_completion_view:
        Arc<RwLock<Option<crate::ui::completion_overlay::MinibufCompletionView>>>,
    /// Interned scope ids for the four diagnostic severities (`diagnostic.error`
    /// etc.), resolved lazily on first use — scope interning needs `&mut
    /// ScopeRegistry`, which lives on `Editor::view`, not `EditorState`.
    pub(super) diagnostic_scopes: Option<[hume_engine::types::ScopeId; 4]>,
    /// Interned scope id for `ui.virtual.inlay-hint`, resolved lazily
    /// on first use for the same reason as `diagnostic_scopes`.
    pub(super) inlay_hint_scope: Option<hume_engine::types::ScopeId>,
    /// Interned scope id for `ui.virtual` — the fallback for a
    /// virtual-line entry with no explicit scope — resolved lazily on first
    /// use for the same reason as `diagnostic_scopes`.
    pub(super) virtual_text_fallback_scope: Option<hume_engine::types::ScopeId>,
    /// Cache of interned `ScopeId`s for plugin-supplied scope name strings
    /// (extra highlights, signs, virtual lines) — avoids re-interning the
    /// same runtime name every frame.
    pub(super) runtime_scope_cache: rustc_hash::FxHashMap<String, hume_engine::types::ScopeId>,
    /// `(show-popup! text)`'s raw content — resolved into a positioned
    /// `PopupState` each frame by `Editor::sync_popup_view` (geometry needs
    /// the focused pane's *current* rect, so it can't be pre-computed here).
    pub(super) popup: Option<crate::ui::popup::PopupModel>,
    /// Shared popup-overlay view for `PopupLayout::Cursor`: written by
    /// `prepare_frame`, read by `PopupOverlay`. Empty whenever `popup` is
    /// `None` or docked (see `popup_band_view`).
    pub(crate) popup_view: Arc<RwLock<Option<crate::ui::popup::PopupState>>>,
    /// Shared popup-band view for `PopupLayout::Docked`: written by
    /// `prepare_frame`, read by `PopupBandWidget` (chrome, like the
    /// drawer). Empty whenever `popup` is `None` or cursor-anchored.
    pub(crate) popup_band_view: Arc<RwLock<Option<crate::ui::popup::PopupBandState>>>,
    /// `(show-menu! items on-select)`'s raw content, including the
    /// not-yet-fired Steel callback — cleared by the key intercept in
    /// `handle_key`, not by `sync_menu_view`.
    pub(super) menu: Option<crate::ui::popup::MenuModel>,
    /// Shared menu-overlay view: written by `prepare_frame`, read by its own
    /// `PopupOverlay` registration (separate from the hover popup's, so both
    /// can in principle show at once — the menu paints on top).
    pub(crate) menu_view: Arc<RwLock<Option<crate::ui::popup::PopupState>>>,
    /// `(show-drawer-list! items on-select)`'s raw content, including the
    /// callback — cleared by `Esc` or `close-drawer!`, *not* by `Enter` (the
    /// drawer stays open across selections, unlike the popup/menu).
    pub(super) drawer: Option<crate::ui::drawer::DrawerModel>,
    /// Shared drawer-overlay view: written on change (open/select-move/
    /// scroll/close) by `sync_drawer_view`, never per frame — the drawer has
    /// no cursor-relative geometry to re-resolve every frame.
    pub(crate) drawer_view: Arc<RwLock<Option<crate::ui::drawer::DrawerViewState>>>,
    /// The open picker session (`docs/FUZZY-FINDERS.md` B2 store) — driven
    /// by the key intercept in `handle_key`; opened via `Editor::open_picker`
    /// (tests today, B4's `picker!` builtin later).
    pub(super) picker: Option<crate::editor::picker::PickerSession>,
    /// Shared picker-overlay view: written per-frame by `sync_picker_view`
    /// (geometry depends on the current panes region, like popup/menu, not
    /// on-change like the drawer), read by `PickerOverlay`.
    pub(crate) picker_view: Arc<RwLock<Option<crate::ui::picker_panel::PickerViewState>>>,
    /// Cross-thread waker clone (see `Editor::open`'s `wake` param), reachable
    /// here so `EditorHostImpl` — which only ever holds a disjoint `&mut
    /// EditorState` borrow, never a whole `&mut Editor` — can hand it to a
    /// spawned picker source (`docs/FUZZY-FINDERS.md` B5) so its reader
    /// thread can wake the event loop. A no-op `Arc` in tests/headless.
    pub(super) wake: Arc<dyn Fn() + Send + Sync>,
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

    /// Mirror `self.drawer` into `self.drawer_view` for `DrawerWidget` to
    /// read. Called directly at every drawer mutation site (open, selection
    /// move, scroll, close) — never per frame, unlike the popup/menu's
    /// `sync_*_view` (the drawer has no cursor-relative geometry to
    /// re-resolve each frame).
    pub(super) fn sync_drawer_view(&self) {
        let resolved = self
            .drawer
            .as_ref()
            .map(|d| crate::ui::drawer::DrawerViewState {
                rows: d.items.clone(),
                selected: d.selected,
                scroll: d.scroll,
            });
        *self.drawer_view.write().expect("RwLock not poisoned") = resolved;
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
        self.trigger_chars
            .iter()
            .filter(|((_, lang), chars)| lang == language && chars.contains(&ch))
            .map(|((source, _), _)| source.clone())
            .collect()
    }

    /// Single write path for all mode transitions.
    ///
    /// Captures the old mode, writes the new one, and enqueues `OnModeChange`
    /// for firing by `Editor::drain_hooks` after the command returns. The
    /// no-op guard prevents spurious hook fires when mode is already correct.
    ///
    /// The `mode` field is private so the compiler enforces that every
    /// transition goes through here.
    pub(crate) fn set_mode(&mut self, new: Mode) {
        use hume_scripting::hooks::HookId;
        use steel::rvals::IntoSteelVal;
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
        let old_val = mode_name(old)
            .into_steelval()
            .expect("mode str into_steelval");
        let new_val = mode_name(new)
            .into_steelval()
            .expect("mode str into_steelval");
        self.pending_hooks
            .push((HookId::OnModeChange, vec![old_val, new_val]));
    }
}

fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Normal => "normal",
        Mode::Insert => "insert",
        Mode::Extend => "extend",
        Mode::Command => "command",
        Mode::Search => "search",
        Mode::Select => "select",
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
    /// `decorations.virtual_lines_generation()` as of each pane's last
    /// mirror into its `PaneVirtualLines` Arc — `prepare_frame`
    /// compares against this to skip the rebuild on frames where the store
    /// didn't change, since this runs in scroll/cursor math too, not just
    /// render.
    virtual_lines_synced: rustc_hash::FxHashMap<hume_engine::pipeline::PaneId, u64>,
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
}

#[cfg(test)]
mod tests;
