//! [`EditorHostImpl`] — the production implementation of the scripting crate's
//! [`EditorHost`] trait.
//!
//! Holds disjoint borrows of `EditorState` and `EngineView`, which enables the
//! Steel VM (`scripting.steel`) to take `&mut Engine` simultaneously without
//! aliasing editor data. Two construction sites create this:
//!
//! - **Command dispatch** (`editor/mod.rs`, `run_steel_command`): called with the
//!   live editor state and view from the focused pane.
//! - **Init dispatch** (`scripting_setup.rs`): called with the same fields
//!   during `init_scripting`; init-only builtins set settings.
//!
//! Every capability trait impl lives one-per-file below, one module per
//! `EditorHost` accessor — isomorphic with `hume_scripting::host`'s own
//! per-capability split, since these traits and their impls are what this
//! type exists to pair up.

use hume_engine::pipeline::{BufferId, EngineView};

use crate::editor::lsp::LspState;
use crate::editor::timer_bridge::TimerHandle;
use hume_scripting::host::{
    AsyncProcessHost, BufferHost, CommandHost, CompletionHost, CursorHost, DecorationHost,
    DiffHost, EditHost, EditorHost, EventHost, LanguageHost, LspHost, OutputHost, RegisterHost,
    SettingsHost, TimerHost, UiHost,
};

use super::EditorState;
use super::tui::Tui;

mod async_process;
mod buffers;
mod commands;
mod completion;
mod cursor;
mod decorations;
mod diff;
mod edits;
mod events;
mod language;
mod lsp;
mod output;
mod registers;
mod settings;
mod timers;
mod ui;

pub(in crate::editor) struct EditorHostImpl<'a> {
    state: &'a mut EditorState,
    view: &'a mut EngineView,
    /// `Some` only at the three call sites that can reach an introspection
    /// builtin (command dispatch, hook fire, queued-call drain) — `None`
    /// everywhere else (init evals, which `require_cmd_ctx!` already blocks
    /// LSP builtins from anyway), so those sites don't need to thread it in.
    /// `&mut` (not `&`) because the LSP completion session lives on
    /// `LspState` — the completion builtins need to write it.
    lsp: Option<&'a mut LspState>,
    /// Same `Some`-at-three-sites shape as `lsp`, for the `(after …)` /
    /// `(cancel-timer! …)` — these mutate (schedule/cancel), so `&LspState`'s
    /// shared-borrow shape doesn't fit; `TimerHandle` bundles the two
    /// `&mut` pieces this needs.
    timers: Option<TimerHandle<'a>>,
    /// This host's inline-output authority: `None` for [`Self::new`]'s
    /// callers, which have no business touching the bracket at all — not
    /// `Some(Tui::Off)`, which means something different (a `full`/`init`
    /// host legitimately running outside `Editor::run`'s loop, still
    /// expected to drive the frame stack's state machine correctly). `Some`
    /// is a clone of `Editor::tui`, not a borrow; see [`Tui`]'s own doc for
    /// why. Read only by `arm_inline_output`, to decide what a *new* frame
    /// captures — [`OutputHost::ensure_inline_output_screen`] never reads
    /// this field; it reads the already-armed frame's own captured
    /// [`super::tui::ActiveTui`] instead (see that type's doc), so it works
    /// correctly even on a host built after the frame it's completing was
    /// pushed by a different one.
    tui: Option<Tui>,
    /// Whether the kitty keyboard protocol is active — read by
    /// `arm_inline_output`, alongside `tui`, to decide what a *new* frame
    /// captures. Like `tui`, [`OutputHost::ensure_inline_output_screen`]
    /// never reads this field directly; it reads the already-armed frame's
    /// own captured kitty state instead, for the same cross-host reason.
    kitty_enabled: bool,
}

impl<'a> EditorHostImpl<'a> {
    /// Constructor for the three init/activation call sites: `init_scripting`
    /// (init.scm + runtime scheme evals), `typed_reload_config`'s re-eval,
    /// and `Editor::activate_and_register`'s runtime lazy-plugin activation
    /// (`mappings/lazy.rs`). Unlike `full`, has no LSP/timer access —
    /// none of these three eval kinds reach an LSP or timer builtin
    /// (`require_cmd_ctx!` blocks command-mode builtins during init; lazy
    /// activation's body may itself later call a real command via `call!`,
    /// which re-enters through `full`, not this path).
    ///
    /// Takes `tui`/`kitty_enabled` explicitly rather than hardcoding them —
    /// unlike init.scm's own two evals (never a live alt-screen), runtime
    /// lazy activation can run with `Editor::run` already owning the
    /// terminal, so a hardcoded `Tui::Off` here would silently let a
    /// `call!`-armed nested command corrupt a live screen.
    pub(in crate::editor) fn init(
        state: &'a mut EditorState,
        view: &'a mut EngineView,
        tui: Tui,
        kitty_enabled: bool,
    ) -> Self {
        Self::with_tui(state, view, Some(tui), kitty_enabled)
    }

    /// Convenience constructor for callers with no terminal/`OutputHost`
    /// needs — general-purpose in shape (a Rust-side helper reaching for an
    /// unrelated capability such as `DecorationHost` via `EditorHostImpl`
    /// would use it too), though today every caller is the test suite,
    /// driving a host through some other trait directly (bypassing the
    /// declare/activate plugin ceremony `init`/`full` sit behind). `tui:
    /// None` gives this host no inline-output authority, so it can be built
    /// at any point — including inside a live `Editor::run` loop — with no
    /// risk of it entering or mis-reading a bracket armed by whichever host
    /// actually owns the current dispatch.
    pub(in crate::editor) fn new(state: &'a mut EditorState, view: &'a mut EngineView) -> Self {
        Self::with_tui(state, view, None, false)
    }

    /// Shared body for [`Self::init`] and [`Self::new`] — the only
    /// difference between them is whether this host has inline-output
    /// authority at all (see the `tui` field's own doc).
    fn with_tui(
        state: &'a mut EditorState,
        view: &'a mut EngineView,
        tui: Option<Tui>,
        kitty_enabled: bool,
    ) -> Self {
        Self {
            state,
            view,
            lsp: None,
            timers: None,
            tui,
            kitty_enabled,
        }
    }

    /// Constructor for the three call sites that thread every capability:
    /// command dispatch, hook fire, and queued-call drain. Takes the fields
    /// already split out (rather than `&mut Editor`) because each call site
    /// holds a simultaneous disjoint borrow of `self.scripting` — passing
    /// `self` as a whole would conflict with that borrow.
    pub(in crate::editor) fn full(
        state: &'a mut EditorState,
        view: &'a mut EngineView,
        lsp: &'a mut LspState,
        timer_wheel: &'a mut super::timers::TimerWheel,
        timer_payloads: &'a mut rustc_hash::FxHashMap<
            super::timers::TimerId,
            super::timer_bridge::TimerPayload,
        >,
        tui: Tui,
        kitty_enabled: bool,
    ) -> Self {
        Self {
            state,
            view,
            lsp: Some(lsp),
            timers: Some(TimerHandle {
                wheel: timer_wheel,
                payloads: timer_payloads,
            }),
            tui: Some(tui),
            kitty_enabled,
        }
    }

    /// Look up a buffer by id.
    fn buffer(&self, id: BufferId) -> Option<&crate::editor::buffer::Buffer> {
        self.state.buffers.try_get(id)
    }
}

impl<'a> EditorHost for EditorHostImpl<'a> {
    // ── Optional capability accessors ────────────────────────────────────────
    fn ui(&mut self) -> Option<&mut dyn UiHost> {
        Some(self)
    }
    fn edits(&mut self) -> Option<&mut dyn EditHost> {
        Some(self)
    }
    fn completions(&mut self) -> Option<&mut dyn CompletionHost> {
        Some(self)
    }
    fn decorations(&mut self) -> Option<&mut dyn DecorationHost> {
        Some(self)
    }
    // `Some(self)` unconditionally, even though `self.lsp` is itself an
    // `Option` — every method below already self-guards on `self.lsp.as_deref()`,
    // and a conditional accessor here would change what "no attached server"
    // vs. "no LSP state at all" reports at the Steel boundary.
    fn lsp(&mut self) -> Option<&mut dyn LspHost> {
        Some(self)
    }
    // Same unconditional-Some rationale as `lsp()` above.
    fn timers(&mut self) -> Option<&mut dyn TimerHost> {
        Some(self)
    }
    // The job registry lives on `self.state.config` — always reachable, no
    // `Option`-wrapped upstream field to gate on (unlike `timers`/`lsp`).
    fn async_process(&mut self) -> Option<&mut dyn AsyncProcessHost> {
        Some(self)
    }
    fn diff(&mut self) -> Option<&mut dyn DiffHost> {
        Some(self)
    }
    fn output(&mut self) -> Option<&mut dyn OutputHost> {
        Some(self)
    }
    fn registers(&mut self) -> Option<&mut dyn RegisterHost> {
        Some(self)
    }
    fn cursor(&mut self) -> &mut dyn CursorHost {
        self
    }
    fn commands(&mut self) -> &mut dyn CommandHost {
        self
    }
    fn language(&mut self) -> &mut dyn LanguageHost {
        self
    }
    fn settings(&mut self) -> &mut dyn SettingsHost {
        self
    }
    fn buffers(&mut self) -> &mut dyn BufferHost {
        self
    }
    fn events(&mut self) -> &mut dyn EventHost {
        self
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
