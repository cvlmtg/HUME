//! `:reload-config`: reset every piece of config-owned state, drop and
//! re-init the scripting engine, then replay the buffer-open lifecycle.
//! `Editor::init_scripting` is shared with startup; this module owns the two
//! reload-only halves around it and the [`ReloadSnapshot`] passed between
//! them.

use super::event::EditorEvent;
use super::{ConfigPath, Editor};
use crate::editor::error::CommandError;
use hume_core::{BufferId, Severity};

/// State `:reload-config` carries from `Editor::reset_config_state`, which
/// captures it before the reset, to `Editor::init_scripting` and
/// `Editor::resync_config_state`. A local of `typed_reload_config`, so an
/// early return cannot leave it stale on `EditorState`.
#[derive(Default)]
pub(crate) struct ReloadSnapshot {
    /// Every buffer open when the reload started. A versioned `BufferId`
    /// never names different content, so membership plus current liveness
    /// is identity.
    pre_reload_bids: rustc_hash::FxHashSet<BufferId>,
    /// Every buffer whose language was asserted (`:set buffer language=`,
    /// `set-buffer-option!`'s `"language"`) rather than detected. Consumed
    /// once by `init_scripting`'s post-reload sweep.
    explicit_languages: Vec<(BufferId, ExplicitLanguage)>,
}

/// A buffer's explicitly asserted language, kept across a reload.
pub(super) struct ExplicitLanguage {
    /// `None` when the assertion cleared the language; that must survive the
    /// reload too, not be repopulated by re-detection.
    pub(super) name: Option<String>,
    /// The origin that asserted it, which still ranks the next write.
    pub(super) by: hume_core::ConfigOrigin,
}

impl ReloadSnapshot {
    /// `true` when `bid` predates this reload and is still a live buffer;
    /// `false` for a buffer `init.scm` opened during the reload or one that
    /// has since closed.
    pub(super) fn survives(
        &self,
        bid: BufferId,
        buffers: &super::buffer::store::BufferStore,
    ) -> bool {
        self.pre_reload_bids.contains(&bid) && buffers.try_get(bid).is_some()
    }

    /// Take the explicit-language snapshot, leaving it empty.
    pub(super) fn take_explicit_languages(&mut self) -> Vec<(BufferId, ExplicitLanguage)> {
        std::mem::take(&mut self.explicit_languages)
    }

    /// A snapshot in which every id in `pre_reload_bids` predates the
    /// reload, for testing `resync_config_state` without a full reset.
    #[cfg(test)]
    pub(in crate::editor) fn for_test(pre_reload_bids: impl IntoIterator<Item = BufferId>) -> Self {
        Self {
            pre_reload_bids: pre_reload_bids.into_iter().collect(),
            explicit_languages: Vec::new(),
        }
    }
}

impl Editor {
    /// Reset every piece of editor-side state a Steel builtin, `set-option!`,
    /// or `init.scm` can write back to `Editor::bootstrap`'s defaults, and return
    /// the [`ReloadSnapshot`] the rest of the reload uses. Runs right before
    /// the old `ScriptingHost` is dropped: dropping the host wipes the
    /// Steel-side registries, and this wipes what those builtins wrote into
    /// the editor.
    ///
    /// Most of this state lives on [`super::ConfigState`] and resets by
    /// replacing it whole; the `field_classification` tests make every
    /// other `EditorState` field declare whether a reload resets it.
    ///
    /// Every Steel value rooted in the outgoing engine (queued callbacks,
    /// open overlay sessions, scheduled thunks) is dropped before the engine
    /// goes, so none is invoked against the new engine.
    pub(in crate::editor) fn reset_config_state(&mut self) -> ReloadSnapshot {
        // Captured first, so a buffer `init.scm` opens during the reload is
        // never taken for one that predates it.
        let pre_reload_bids = self.state.buffers.iter().map(|(bid, _)| bid).collect();

        // An open Insert or paste session holds undo state, not a Steel
        // value, so it gets a real teardown. `reset_input` ends it, then
        // drops every other mode layer and overlay without firing its
        // callback.
        self.state.reset_input(&self.view);

        // ── Steel values rooted in the outgoing engine ──
        //
        // `pending_work` drops with the old `ConfigState` below. The mode
        // layers and overlays live on `state.input`, which `reset_input`
        // already dropped, resetting Extend and the sticky popup with them.
        // An open picker's streaming child is killed on drop, and the
        // overlay views rebuild from `prepare_frame` every frame.
        self.state.forget_steel_payloads();
        self.state.pending_writes.release_all();
        self.state.lsp.reset_config();
        self.state.buffer_positions.lsp.clear_all_triggers();
        // Only the Steel `after!` thunks: native debounce timers hold
        // nothing Steel-specific.
        self.timers.cancel_steel_thunks();

        // ── Config-owned editor state ──
        // Kept by name, since the `LanguageId` below is about to dangle, so
        // the post-reload sweep restores an asserted language (an
        // extensionless file's, say) instead of losing it to detection.
        let explicit_languages = self
            .state
            .buffers
            .iter()
            .filter_map(|(bid, buf)| {
                let by = buf.language_by.by()?;
                let name = self.state.language_name(buf).map(str::to_owned);
                Some((bid, ExplicitLanguage { name, by }))
            })
            .collect();
        // A `LanguageId` indexes `state.config.languages`, replaced below.
        // Clearing it also makes the post-reload re-detect a `None -> Some`
        // transition, whose hooks `resync_config_state` relies on.
        self.state.buffers.clear_languages_all();
        self.state.buffers.clear_overrides_all();
        self.state.buffers.reset_plugin_views_all();
        let prior_clock = self.state.config.decorations.clock();
        self.state.config =
            super::ConfigState::new(self.kitty_enabled, prior_clock, &self.state.library_wait);
        // The scripts that held these tokens went with the outgoing engine.
        self.state.panes.tracked.clear();
        // Re-baseline so the hooks `init_scripting` registers never see a
        // transition for a mode change they did not observe, such as the
        // Insert session discarded above.
        self.state.last_observed_mode = self.state.mode();
        super::settings::ops::reset_globals(&mut self.state, &mut self.view);

        ReloadSnapshot {
            pre_reload_bids,
            explicit_languages,
        }
    }

    /// Replay the buffer-open lifecycle for every buffer that predates this
    /// reload, once `init_scripting()` has rebuilt the Steel engine and
    /// re-detected each buffer's language.
    ///
    /// The reset cleared state that hooks repopulate on a transition (a
    /// server attaching, a buffer opening, diagnostics being published),
    /// none of which a reload causes. Running LSP servers and their
    /// diagnostics survive the reload, so this re-fires the hooks from that
    /// state instead of round-tripping `didClose`/`didOpen`: a server that
    /// publishes diagnostics only on change would leave them blank until
    /// the next edit.
    ///
    /// `snapshot` limits every loop to buffers that predate the reload, since
    /// a buffer `init.scm` opened has already fired its hooks through the
    /// ordinary open path, and that are still live.
    ///
    /// The hooks run batched (every `OnLspAttach`, then every `OnBufferOpen`,
    /// and so on); `pending_work` is FIFO, so each buffer's own hooks keep a
    /// real open's relative order.
    pub(in crate::editor) fn resync_config_state(&mut self, snapshot: &ReloadSnapshot) {
        // Attachments to a server already running: reconcile keeps those the
        // new config still plans without reopening them, so their
        // `OnLspAttach` is replayed and their diagnostics pulled again (the
        // reset dropped any pull in flight). An attachment reconcile
        // announces itself is not replayed.
        let running_attachments: Vec<_> = self
            .state
            .lsp_running_attachments()
            .into_iter()
            .filter(|(bid, _)| snapshot.survives(*bid, &self.state.buffers))
            .collect();
        self.state.lsp.resume_reconcile();
        let announced = self.state.lsp_reconcile_all(&self.view);
        for (bid, sid) in running_attachments {
            if self.state.buffer_positions.lsp.is_attached(bid, sid)
                && !announced.contains(&(bid, sid))
            {
                self.state.queue_lsp_attach(bid, sid);
                self.state.lsp_repull_diagnostics(bid, sid);
            }
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

        // Keyed on the diagnostics store, not `running_attachments`: a
        // crashed server's last-published diagnostics stay in it and must be
        // replayed too.
        let diagnostic_bids: Vec<BufferId> = self
            .state
            .buffer_positions
            .diagnostics
            .buffers_with_diagnostics()
            .filter(|&bid| snapshot.survives(bid, &self.state.buffers))
            .collect();
        for bid in diagnostic_bids {
            self.state
                .queue_event(EditorEvent::OnDiagnosticsChanged { buffer: bid });
        }

        // `on-viewport-change` handlers (inlay hints) otherwise repopulate
        // only when the viewport moves. Active-tab panes only: a background
        // pane's scroll position is stale, and its first frame once its tab
        // is focused reads as a viewport change anyway.
        for pane_id in self.view.active_pane_ids() {
            if snapshot.survives(self.view.panes[pane_id].buffer_id(), &self.state.buffers) {
                self.queue_viewport_change(pane_id);
            }
        }

        // `Editor::detect_buffer_enter` raises `OnBufferEnter` as a diff
        // against `last_entered_buffer`, and a reload does not change the
        // focused buffer. Clearing the baseline makes the next pass, in
        // `typed_reload_config`'s `drain_pending_work()`, raise one
        // `OnBufferEnter` after the `OnBufferOpen` batch has re-hooked every
        // plugin, which brings back state such as `core:git-diff`'s branch
        // statusline element. Its Rust reaction also `stat`s the focused
        // buffer and can open the "changed on disk" confirm, as a real
        // reopen would.
        self.state.last_entered_buffer = None;
    }
}

/// `:reload-config`: reset all config-owned state to compiled-in defaults,
/// drop the Steel engine, and re-evaluate `init.scm` from scratch. Buffers,
/// panes, undo history, registers and running LSP servers are untouched.
///
/// The reset runs before `init_scripting()`, or the prior load's Steel
/// commands would still be registered and every `(define-command!)` in
/// `init.scm` would fail the builtin-conflict check.
pub(in crate::editor) fn typed_reload_config(
    ed: &mut Editor,
    _fp: crate::editor::commands::FocusedPane,
) -> Result<(), CommandError> {
    // Checked before anything is reset, so a reload that cannot proceed
    // leaves the editor as it was. `--no-config` holds for the whole
    // session, so a reload refuses rather than load the real config.
    match ed.config_path() {
        ConfigPath::Resolved { .. } => {}
        ConfigPath::Skipped => {
            return Err(CommandError::new(
                "reload-config: session started with --no-config",
            ));
        }
        ConfigPath::NoConfigDir => {
            return Err(CommandError::new(
                "reload-config: no config directory: HOME/XDG_CONFIG_HOME (APPDATA on Windows) unset",
            ));
        }
    }
    // Lifetime totals, which eviction past `MAX_ENTRIES` cannot skew.
    // Warnings count too: most failures `init_scripting` and the hooks hit
    // (no runtime dir, an unknown keymap target, …) report at
    // `Severity::Warning`, and "Config reloaded" must not bury them.
    let (errors_before, warnings_before) = ed.state.message_log.totals();
    let mut snapshot = ed.reset_config_state();
    ed.scripting = None;
    ed.init_scripting(&mut snapshot);
    ed.resync_config_state(&snapshot);
    // Drained inside the accounting window: `resync_config_state` only
    // queues its hooks, and their errors must count. Not `settle`, which
    // also drains async sources and would count an unrelated LSP, parse or
    // timer message against this reload.
    ed.drain_pending_work();
    let (errors_after, warnings_after) = ed.state.message_log.totals();
    if errors_after == errors_before && warnings_after == warnings_before {
        ed.report(Severity::Info, "Config reloaded");
    }
    Ok(())
}
