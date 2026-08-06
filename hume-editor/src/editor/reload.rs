//! `:reload-config` — reset every piece of config-owned state, drop and
//! re-init the scripting engine, then replay the buffer-open lifecycle.
//!
//! `Editor::init_scripting` is shared with startup and stays in
//! `scripting_setup.rs`; this module owns the two reload-only halves around
//! it ([`Editor::reset_config_state`], [`Editor::resync_config_state`]) plus
//! the [`ReloadSnapshot`] threaded between them and the orchestrator,
//! `typed_reload_config`.

use hume_engine::pipeline::BufferId;

use super::event::EditorEvent;
use super::{Editor, Severity};
use crate::editor::error::CommandError;

// ── ReloadSnapshot ───────────────────────────────────────────────────────────

/// State `:reload-config` must carry across the gap between
/// `Editor::reset_config_state` (which captures it, right before the reset
/// it captures it *from*) and `Editor::init_scripting`/
/// `Editor::resync_config_state` (which consume it). Owned by
/// `typed_reload_config` as a local, not an `EditorState` field: a field
/// would stay stranded (and silently stale) on any early return between the
/// two halves.
#[derive(Default)]
pub(crate) struct ReloadSnapshot {
    /// `(bid, replace_stamp)` for every buffer open when the reload started,
    /// as of that moment. `Buffer::replace_stamp`'s doc explains why a
    /// versioned `BufferId` alone isn't enough to tell "the same buffer
    /// this snapshot meant" apart from "a fresh scratch buffer that reused
    /// the same key in place".
    buffer_stamps: rustc_hash::FxHashMap<BufferId, u64>,
    /// `(bid, explicit-language-name)` for every buffer whose language was
    /// an explicit assertion (`:set buffer language=`/`set-buffer-language!`)
    /// rather than detection. `None` means the user explicitly cleared the
    /// language — that must survive the reload too, not be silently
    /// repopulated by re-detection. Consumed (via `take_explicit_languages`)
    /// exactly once, by `init_scripting`'s post-reload sweep.
    explicit_languages: Vec<(BufferId, Option<String>)>,
}

impl ReloadSnapshot {
    /// `true` when `bid` is still the same buffer instance this snapshot
    /// captured. `false` both for a bid the snapshot never saw (a buffer
    /// `init.scm` opened fresh this reload) and for a bid whose buffer was
    /// silently swapped in place since (`close_buffer`'s last-buffer scratch
    /// replacement) — see `Buffer::replace_stamp`.
    pub(super) fn survives(
        &self,
        bid: BufferId,
        buffers: &super::buffer::store::BufferStore,
    ) -> bool {
        self.buffer_stamps.get(&bid).is_some_and(|&stamp| {
            buffers
                .try_get(bid)
                .is_some_and(|buf| buf.replace_stamp == stamp)
        })
    }

    /// Take ownership of the explicit-language snapshot, leaving this
    /// instance's copy empty. `mem::take`n rather than borrowed so
    /// `init_scripting`'s sweep can consume each entry by value without a
    /// second `self` borrow.
    pub(super) fn take_explicit_languages(&mut self) -> Vec<(BufferId, Option<String>)> {
        std::mem::take(&mut self.explicit_languages)
    }

    /// Build a snapshot as if every id in `pre_reload_bids` predated the
    /// reload, using each buffer's *current* `replace_stamp` — mirrors what
    /// `reset_config_state` itself captures. Test-only: production always
    /// gets a `ReloadSnapshot` from `reset_config_state`; this exists for
    /// `resync_config_state` unit tests that exercise the replay in
    /// isolation, without a full reset.
    #[cfg(test)]
    pub(crate) fn for_test(
        pre_reload_bids: impl IntoIterator<Item = BufferId>,
        buffers: &super::buffer::store::BufferStore,
    ) -> Self {
        Self {
            buffer_stamps: pre_reload_bids
                .into_iter()
                .filter_map(|bid| buffers.try_get(bid).map(|buf| (bid, buf.replace_stamp)))
                .collect(),
            explicit_languages: Vec::new(),
        }
    }
}

impl Editor {
    /// Reset every piece of editor-side state a Steel builtin, `set-option!`,
    /// or `init.scm` itself can write, back to the same defaults
    /// `Editor::open` starts with, and return the [`ReloadSnapshot`]
    /// `:reload-config` threads through the rest of the reload. Called
    /// immediately before dropping the old `ScriptingHost` — this is the
    /// other half of "from scratch": dropping the host wipes the
    /// Steel-VM-side registries (`cmd_owners`, hooks, plugin lifecycle, …),
    /// this wipes everything those builtins wrote *into* the editor.
    ///
    /// Most of what this must reset lives on [`super::ConfigState`], reset by
    /// *construction* — see its doc for why a field added there can't be
    /// forgotten the way a field added directly here still can (enforced by
    /// the `editor_state_fields_are_classified` lint in `lints/field_classification.rs`).
    ///
    /// Order matters: every Steel value rooted in the outgoing engine
    /// (queued callbacks, open overlay sessions, scheduled thunks) is
    /// dropped first, before the engine itself goes away — so nothing here
    /// ever gets invoked against the *new* engine that didn't create it.
    pub(crate) fn reset_config_state(&mut self) -> ReloadSnapshot {
        // Captured before anything below runs: `replace_stamp` is buffer
        // identity bookkeeping, untouched by this reset, but must reflect
        // each buffer's stamp *as of reload start* — see
        // `Buffer::replace_stamp`'s doc.
        let buffer_stamps = self
            .state
            .buffers
            .iter()
            .map(|(bid, buf)| (bid, buf.replace_stamp))
            .collect();

        // ── Steel values rooted in the outgoing engine ──
        //
        // `pending_work` and the five overlay models
        // (popup/menu/drawer/picker/confirm) all drop below when
        // `self.state.config = ConfigState::new(…)` runs — nothing here
        // reads any of them in between, so there's nothing to clear early.
        // `PickerSession::source` (if a picker was open) kills any streaming
        // child process on drop, same as any other `ConfigState` drop; the
        // overlay *views* (`popup_view`/`menu_view`/`drawer_view`/
        // `picker_view`) self-heal from `prepare_frame` every frame
        // regardless, so nothing here needs to touch them directly either.
        // `confirm` has no view/Steel callback of its own (its action is a
        // plain Rust enum, not a rooted `SteelVal`), so it needs even less
        // than the others — dropping it is the entire teardown.
        if self.state.config.steel_prompt_callback.is_some() {
            // A `(prompt! …)` session was open. Its callback belongs to the
            // outgoing engine and is discarded (not fired) by the
            // `ConfigState` rebuild below, same policy as the popup/menu/
            // drawer/picker overlays — but unlike those, a prompt also parks
            // the editor in `Mode::Command` with an open minibuf and an
            // in-progress history session (`host_impl.rs`'s `%prompt!` sets
            // all three together). Leaving those live would route the next
            // `:`/Enter through the *ordinary* command-line path, misreading
            // the abandoned prompt's half-typed answer as a `:` command.
            self.close_minibuf();
            self.state.set_mode(super::Mode::Normal);
        }
        self.lsp.reset_config();
        // Only the Steel `after` thunks — native `ViewportDebounce` timers
        // keep their wheel entries and their `viewport_debounce` back-index
        // intact, since nothing about them is Steel-VM-specific. Exhaustive
        // match, not `matches!`, so a future `TimerPayload` variant forces a
        // decision here instead of silently surviving the engine drop.
        let steel_timer_ids: Vec<super::timers::TimerId> = self
            .timer_payloads
            .iter()
            .filter(|(_, payload)| match payload {
                super::timer_bridge::TimerPayload::SteelThunk(_) => true,
                super::timer_bridge::TimerPayload::ViewportDebounce(_) => false,
            })
            .map(|(&id, _)| id)
            .collect();
        for id in steel_timer_ids {
            self.timer_wheel.cancel(id);
            self.timer_payloads.remove(&id);
        }

        // ── Config-owned editor state ──
        // Snapshot every buffer's own `:set buffer language=`/
        // `set-buffer-language!` assertion (by name, since the `LanguageId`
        // below is about to dangle) so `init_scripting`'s post-reload
        // re-detect sweep can restore it — otherwise a buffer whose language
        // was explicitly asserted rather than detected (e.g. an extensionless
        // file) would silently lose that assertion to whatever plain
        // detection finds, contradicting "buffers are untouched" — the
        // invariant `:reload-config` promises the rest of this function.
        let explicit_languages = self
            .state
            .buffers
            .iter()
            .filter(|(_, buf)| buf.language_explicit)
            .map(|(bid, buf)| {
                let name = buf
                    .language
                    .map(|id| self.state.config.languages.name_of(id).to_owned());
                (bid, name)
            })
            .collect();
        // Every buffer's `LanguageId` is an index into `state.config.languages` —
        // clear it before the registry it indexes into is replaced below, or
        // it dangles. This also makes `init_scripting`'s post-reload
        // re-detect sweep a real `None -> Some` transition again (see
        // `resync_config_state`, which relies on the hooks that transition
        // fires).
        self.state.buffers.clear_languages_all();
        self.state.buffers.clear_overrides_all();
        let prior_generation = self.state.config.decorations.generation();
        self.state.config = super::ConfigState::new(self.kitty_enabled, prior_generation);
        super::settings_ops::reset_globals(&mut self.state, &mut self.view);

        ReloadSnapshot {
            buffer_stamps,
            explicit_languages,
        }
    }

    /// Replay the buffer-open lifecycle for every buffer that predates this
    /// reload — called by `:reload-config`, once, after `init_scripting()`
    /// has rebuilt the Steel engine and re-detected each buffer's language.
    ///
    /// `reset_config_state` clears state that is normally repopulated by a
    /// hook fired on a *transition* (a server going from unattached to
    /// attached, a buffer opening, diagnostics being published) — none of
    /// which a reload, by itself, causes. Firing those hooks here is the
    /// other half of ":reload-config behaves like closing and reopening
    /// every already-open buffer": running LSP servers and their published
    /// diagnostics survive the reload (`LspState::reset_config` deliberately
    /// keeps `servers` and `diagnostics`), so this re-fires the attach and
    /// diagnostics hooks from that surviving state rather than re-opening
    /// documents over the wire — a real close+reopen would round-trip
    /// `textDocument/didClose`+`didOpen` to a server we're keeping alive,
    /// and a server that only republishes diagnostics on change would leave
    /// the buffer's diagnostics blank until the next edit.
    ///
    /// `snapshot` — captured by `reset_config_state` before this reload's
    /// reset ran — filters every loop below to buffers that (a) predate this
    /// reload and (b) are still the *same* buffer instance, per
    /// `Buffer::replace_stamp`. Without (a): the ordinary open path
    /// (`detect_pending_languages`, run inside `init_scripting` before this
    /// function is called) already fires hooks once for a genuinely new
    /// buffer, and by the time this function runs its `open_hook_pending` is
    /// already `false` again, same as every pre-reload buffer — so a buffer
    /// `init.scm` itself opens while re-running (a session-restore plugin, a
    /// first-run `open-buffer!`) would double-fire without this filter.
    /// Without (b): a bid whose only buffer `init.scm` closed (reusing the
    /// slot in place for a fresh scratch — see `close_buffer`) would have
    /// its pre-reload hooks replayed against unrelated scratch content.
    ///
    /// No `OnBufferClose` counterpart: that hook would have to run against
    /// the outgoing engine, before the reset, tearing down state the reset
    /// discards anyway — reload is a restart, not a close.
    ///
    /// Batched, not interleaved per buffer the way a real reopen would fire
    /// these: every `OnLspAttach` runs, then every `OnBufferOpen`, then every
    /// `OnDiagnosticsChanged`/`OnViewportChange`. `pending_work` is FIFO, so
    /// each buffer's *own* hooks still fire in the same relative order a real
    /// open would use — only the cross-buffer interleaving differs.
    pub(crate) fn resync_config_state(&mut self, snapshot: &ReloadSnapshot) {
        let running_attachments: Vec<_> = self
            .lsp
            .running_attached_buffers(&self.state.buffers)
            .into_iter()
            .filter(|(bid, _)| snapshot.survives(*bid, &self.state.buffers))
            .collect();
        for (bid, language) in &running_attachments {
            self.queue_lsp_attach(*bid, language);
        }

        let open_bids: Vec<BufferId> = self
            .state
            .buffers
            .iter()
            .map(|(id, _)| id)
            .filter(|&id| snapshot.survives(id, &self.state.buffers))
            .collect();
        for bid in open_bids {
            self.state
                .queue_event(EditorEvent::OnBufferOpen { buffer: bid });
        }

        // Diagnostics: pull-style hook, re-reads the surviving
        // `LspState::diagnostics` cache rather than needing a payload. Keyed
        // on the cache itself, not `running_attachments` — a crashed server's
        // last-published diagnostics stay in the cache (`reset_config`'s doc)
        // and must still be replayed, or a reload permanently blanks a
        // buffer's diagnostics that only `:lsp-restart` would otherwise
        // bring back.
        let diagnostic_bids: Vec<BufferId> = self
            .lsp
            .buffers_with_diagnostics()
            .filter(|&bid| snapshot.survives(bid, &self.state.buffers))
            .collect();
        for bid in diagnostic_bids {
            self.queue_diagnostics_changed(bid);
        }

        // Inlay hints (and anything else `on-viewport-change`-gated, e.g.
        // `core:lsp`'s inlay.scm) are otherwise only repopulated the next
        // time the pane's viewport genuinely moves — which a reload alone
        // never causes — so a clean buffer would show no inlay hints until
        // the user scrolls. Every pane, not just the focused one: each
        // pane's viewport is independent state a real reopen would restore
        // per-pane too.
        let panes_on_surviving_buffers: Vec<hume_engine::pipeline::PaneId> = self
            .view
            .panes
            .iter()
            .filter(|(_, pane)| snapshot.survives(pane.buffer_id, &self.state.buffers))
            .map(|(pid, _)| pid)
            .collect();
        for pane_id in panes_on_surviving_buffers {
            self.queue_viewport_change(pane_id);
        }
    }
}

/// `:reload-config` — reset every piece of config-owned state to its
/// compiled-in default, drop the scripting engine, and re-evaluate
/// `init.scm` from scratch.
///
/// `reset_config_state` is the full contract for what "from scratch" resets
/// (keymap, settings, LSP registrations, decorations, dynamic commands, …)
/// and why it must run — including clearing dynamic commands from the
/// registry — before `ed.scripting` is dropped and `init_scripting()` runs:
/// otherwise the new `builtin_names` set (built from `registry.names()`)
/// would contain every Steel command from the prior load, and every
/// `(define-command!)` in the re-evaluated `init.scm` would fail the
/// builtin-conflict check in `hume-scripting/src/builtins/commands.rs`
/// with "conflicts with a built-in command and cannot be redefined".
///
/// Buffers, panes, undo history, registers, and running LSP server
/// processes are untouched — only *config* resets, not editing state.
///
/// `resync_config_state` runs last, after `init_scripting` has rebuilt the
/// engine and re-detected every buffer's language: it replays the
/// buffer-open lifecycle (`OnLspAttach` for already-attached servers,
/// `OnBufferOpen`, `OnDiagnosticsChanged` from the surviving diagnostics
/// cache) so state a hook would normally repopulate — trigger characters,
/// inline diagnostics/inlay hints, buffer-open-driven decorations — doesn't
/// stay empty simply because reload never causes the transition that hook
/// is gated on. See `Editor::resync_config_state`'s doc for why this is
/// scoped to a replay rather than a literal LSP close+reopen.
pub(crate) fn typed_reload_config(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    // Checked before anything is touched: `init_scripting` needs this same
    // directory to re-evaluate `init.scm`, and failing here — before
    // `reset_config_state` wipes languages/keymap/theme/highlighting — means
    // a reload with no HOME/XDG_CONFIG_HOME leaves the editor exactly as it
    // was, rather than reset to compiled-in defaults with no way back.
    if hume_platform::dirs::config_dir().is_none() {
        return Err(CommandError::new(
            "reload-config: no config directory — HOME/XDG_CONFIG_HOME (APPDATA on Windows) unset",
        ));
    }
    // Lifetime totals, not `unseen_counts`: the log can evict old entries
    // past `MAX_ENTRIES`, which would otherwise skew a before/after unseen
    // count in either direction on a long session — see `MessageLog::totals`.
    // Warnings count too, not just errors: every failure mode `init_scripting`
    // and the hooks below can hit (no runtime dir, an unknown keymap target,
    // an unregistered restored language, …) reports at `Severity::Warning`,
    // and an unconditional success message would bury it under "it worked".
    let (errors_before, warnings_before) = ed.state.message_log.totals();
    let mut snapshot = ed.reset_config_state();
    ed.scripting = None;
    ed.init_scripting(&mut snapshot);
    ed.resync_config_state(&snapshot);
    // Drained here, inside the accounting window, rather than left for the
    // next loop iteration: `resync_config_state` only *enqueues* its hooks
    // (`queue_event`), and a handler error from one of them is exactly the
    // kind of failure "Config reloaded" must not paper over.
    //
    // `drain_pending_work`, not `settle`: `settle` also runs
    // `drain_async_sources` first, which would pull in an unrelated LSP/
    // parse/timer message that happens to arrive at this moment and count it
    // against this reload's own errors/warnings delta — see `settle`'s doc.
    ed.drain_pending_work();
    let (errors_after, warnings_after) = ed.state.message_log.totals();
    if errors_after == errors_before && warnings_after == warnings_before {
        ed.report(Severity::Info, "Config reloaded".to_string());
    }
    Ok(())
}
