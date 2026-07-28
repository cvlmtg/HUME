//! `:reload-config` — reset every piece of config-owned state, drop and
//! re-init the scripting engine, then replay the buffer-open lifecycle.
//!
//! `Editor::init_scripting` is shared with startup and stays in
//! `scripting_setup.rs`; this module owns the two reload-only halves around
//! it ([`Editor::reset_config_state`], [`Editor::resync_config_state`]) plus
//! the [`ReloadSnapshot`] threaded between them and the orchestrator,
//! `typed_reload_config`.

use hume_engine::pipeline::BufferId;
use hume_scripting::SteelBufferId;
use hume_scripting::hooks::HookId;

use super::Editor;

// ── ReloadSnapshot ───────────────────────────────────────────────────────────

/// State `:reload-config` must carry across the gap between
/// `Editor::reset_config_state` (which captures it, right before the reset
/// it captures it *from*) and `Editor::init_scripting`/
/// `Editor::resync_config_state` (which consume it). Owned by
/// `typed_reload_config` as a local — not smuggled through an `EditorState`
/// field the way `pending_reload_explicit_languages` used to be, which left
/// it stranded (and silently stale) on any early return between the two
/// halves.
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
    /// the `editor_state_fields_are_classified` lint in `lints.rs`).
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
        self.state.config.pending_hooks.clear();
        self.state.config.pending_steel_calls.clear();
        if self.state.config.steel_prompt_callback.take().is_some() {
            // A `(prompt! …)` session was open. Its callback belongs to the
            // outgoing engine and is discarded here, not fired (same policy
            // as the popup/menu/drawer/picker overlays below) — but unlike
            // those, a prompt also parks the editor in `Mode::Command` with
            // an open minibuf and an in-progress history session
            // (`host_impl.rs`'s `%prompt!` sets all three together). Leaving
            // those live would route the next `:`/Enter through the
            // *ordinary* command-line path, misreading the abandoned
            // prompt's half-typed answer as a `:` command.
            self.close_minibuf();
            self.state.set_mode(super::Mode::Normal);
        }
        // Assigned directly, not via close_popup!/close_menu!/close_picker:
        // those queue an on-close/on-select callback into
        // `pending_steel_calls`, which the line above already drops.
        // `PickerSession::source` kills any streaming child process on drop.
        // The overlay *views* (`popup_view`/`menu_view`/`drawer_view`/
        // `picker_view`) self-heal from `prepare_frame` every frame, so
        // nothing here needs to touch them directly.
        self.state.config.popup = None;
        self.state.config.menu = None;
        self.state.config.drawer = None;
        self.state.config.picker = None;
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
        // detection finds, contradicting "buffers are untouched" (see
        // `docs/ROADMAP.md`'s `:reload-config` decision).
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
        let prior_virtual_lines_generation =
            self.state.config.decorations.virtual_lines_generation();
        self.state.config =
            super::ConfigState::new(self.kitty_enabled, prior_virtual_lines_generation);
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
    /// `Buffer::replace_stamp`. Without (a), a buffer that `init.scm` itself
    /// opens while re-running (a session-restore plugin, a first-run
    /// `open-buffer!`) would double-fire: the ordinary open path
    /// (`detect_pending_languages`, run inside `init_scripting` before this
    /// function is even called) already fires its hooks once for a
    /// genuinely new buffer, and by the time this function runs that
    /// buffer's `open_hook_pending` is already `false` again — the same as
    /// every buffer that predates the reload — so nothing per-buffer is left
    /// to tell the two cases apart except this snapshot. Without (b), a bid
    /// whose only buffer `init.scm` closed (reusing the slot in place for a
    /// fresh scratch — see `close_buffer`) would have its pre-reload hooks
    /// replayed against unrelated scratch content.
    ///
    /// No `OnBufferClose` counterpart: that hook would have to run against
    /// the outgoing engine, before the reset, tearing down state the reset
    /// discards anyway — reload is a restart, not a close.
    ///
    /// Batched, not interleaved per buffer the way a real reopen would fire
    /// these: every `OnLspAttach` runs, then every `OnBufferOpen`, then every
    /// `OnDiagnosticsChanged`/`OnViewportChange`. `pending_hooks` is FIFO, so
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
            self.fire_hook_lsp_attach(*bid, language);
        }

        let open_bids: Vec<BufferId> = self
            .state
            .buffers
            .iter()
            .map(|(id, _)| id)
            .filter(|&id| snapshot.survives(id, &self.state.buffers))
            .collect();
        for bid in open_bids {
            let val = SteelBufferId::new(bid).into_steel_val();
            self.fire_hook_silent(HookId::OnBufferOpen, &[val]);
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
            self.fire_hook_diagnostics_changed(bid);
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
            self.fire_hook_viewport_change(pane_id);
        }
    }
}
