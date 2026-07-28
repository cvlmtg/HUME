use std::path::PathBuf;

use hume_engine::pipeline::BufferId;

use hume_scripting::SteelBufferId;
use hume_scripting::{Effect, hooks::HookId};
use steel::rvals::SteelVal;

use super::{Editor, Severity, host_impl::EditorHostImpl};

/// Upper bound on total hooks processed per `drain_hooks` boundary.
///
/// Bounding *passes* instead of total work would still let an amplifying
/// cascade (a handler that enqueues more hooks than it received) blow up the
/// batch size geometrically pass over pass — few passes, but exponential
/// total evals. Counting total hooks processed bounds both that shape and the
/// constant-width ping-pong loop; unreachable in any legitimate configuration.
const MAX_HOOK_DRAIN_HOOKS: usize = 1000;

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
    fn survives(&self, bid: BufferId, buffers: &super::buffer::store::BufferStore) -> bool {
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
    fn take_explicit_languages(&mut self) -> Vec<(BufferId, Option<String>)> {
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
    /// Apply every effect a Steel eval queued, in the exact order the
    /// script emitted them (`hume_scripting::Effect`) — one ordered log, not
    /// separate channels with a hardcoded apply order. Consecutive
    /// `Effect::LanguageReg` entries are grouped into one
    /// `apply_pending_language_regs` call so a large run (e.g.
    /// `languages.scm`'s ~700 `define-language!` calls) rebuilds the glob
    /// matcher once, not once per entry; every other effect kind applies one
    /// at a time. Shared tail for `call_steel_cmd`'s call site, `drain_hooks`,
    /// `drain_pending_steel_calls`, and `init_scripting`.
    ///
    /// Finishes by draining `state.config.pending_language_detection` — covers every
    /// buffer a disjoint-borrow Steel path opened via `buffer::lifecycle::
    /// open_buffer_and_notify` this eval (`(open-buffer! …)`, workspace edits,
    /// goto-definition). *After* the effect loop, not before: a script that
    /// opens a buffer and registers its language in the same eval must have
    /// the registration land first.
    pub(crate) fn apply_script_effects(&mut self, effects: Vec<Effect>) {
        let mut effects: std::collections::VecDeque<Effect> = effects.into();
        while let Some(effect) = effects.pop_front() {
            match effect {
                Effect::LanguageReg(reg) => {
                    let mut batch = vec![reg];
                    while matches!(effects.front(), Some(Effect::LanguageReg(_))) {
                        let Some(Effect::LanguageReg(next)) = effects.pop_front() else {
                            unreachable!("front() just confirmed a LanguageReg variant")
                        };
                        batch.push(next);
                    }
                    self.apply_pending_language_regs(batch);
                }
                Effect::LspServerOp(op) => self.apply_lsp_server_op(op),
                Effect::SetBufferLanguage { buffer, language } => {
                    let lang_id = language.map(|name| self.state.config.languages.intern(&name));
                    self.set_buffer_language_explicit(buffer, lang_id)
                }
                Effect::GrammarSweep(name) => {
                    let id = self.state.config.languages.id_of(&name).expect(
                        "GrammarSweep is only emitted right after attach_grammar interns the name",
                    );
                    self.sweep_buffers_for_grammars(vec![id])
                }
                Effect::LspRequest(req) => {
                    self.flush_lsp_pending_changes();
                    self.send_one_lsp_request(req);
                }
                Effect::LspNotify(notify) => {
                    self.flush_lsp_pending_changes();
                    self.send_one_lsp_notify(notify);
                }
                Effect::BindKey {
                    mode,
                    keys,
                    cmd,
                    force_extend,
                } => self.state.config.keymap.bind_user_with_extend(
                    to_editor_bind_mode(mode),
                    &keys,
                    std::borrow::Cow::Owned(cmd),
                    force_extend,
                ),
                Effect::BindWaitChar { mode, keys, cmd } => {
                    self.state.config.keymap.bind_wait_char_user(
                        to_editor_bind_mode(mode),
                        &keys,
                        std::borrow::Cow::Owned(cmd),
                    )
                }
                Effect::UnbindKey { mode, keys } => self
                    .state
                    .config
                    .keymap
                    .unbind_user(to_editor_bind_mode(mode), &keys),
            }
        }
        self.detect_pending_languages();
    }

    // ── Message reporting ─────────────────────────────────────────────────────

    /// Report a message, routing it based on severity:
    ///
    /// - `Info`    → set `status_msg` only (ephemeral, not logged)
    /// - `Warning` → push to `message_log` AND set `status_msg`
    /// - `Error`   → push to `message_log` AND set `status_msg`
    /// - `Trace`   → push to `message_log` only (not shown in statusline)
    pub(crate) fn report(&mut self, severity: Severity, text: String) {
        self.state.report(severity, text);
    }

    /// Take a persistent queue off the scripting host, or an empty `Vec` if
    /// scripting isn't initialized. Collecting into an owned `Vec` first
    /// (rather than draining in place) satisfies the borrow checker at every
    /// call site, which also needs `&mut self` to apply what's taken.
    fn take_from_host<T>(
        &mut self,
        taker: impl FnOnce(&mut hume_scripting::ScriptingHost) -> Vec<T>,
    ) -> Vec<T> {
        self.scripting.as_mut().map(taker).unwrap_or_default()
    }

    /// Drain any pending `(log! …)` messages from the scripting host and
    /// report each one.
    pub(crate) fn flush_script_messages(&mut self) {
        let msgs = self.take_from_host(hume_scripting::ScriptingHost::take_pending_messages);
        for (level, text) in msgs {
            self.report(log_level_to_severity(level), text);
        }
    }

    // ── Hook firing ──────────────────────────────────────────────────────────

    /// Fire `OnBufferSave` hooks for `bid`. Both `:w` write paths in
    /// `commands.rs` share this rather than duplicating the arg construction.
    pub(super) fn fire_hook_buffer_save(&mut self, bid: BufferId) {
        let val = SteelBufferId::new(bid).into_steel_val();
        self.fire_hook_silent(HookId::OnBufferSave, &[val]);
    }

    /// Fire `OnLspAttach (bid server-name)` — called both when a buffer
    /// attaches to an already-Running server (`lsp_attach_buffer`) and, for
    /// every buffer already attached, when a Starting client reaches
    /// Running (`dispatch_lsp_action`'s `BecameRunning` arm).
    pub(super) fn fire_hook_lsp_attach(&mut self, bid: BufferId, server_name: &str) {
        let bid_val = SteelBufferId::new(bid).into_steel_val();
        let name_val = SteelVal::StringV(server_name.into());
        self.fire_hook_silent(HookId::OnLspAttach, &[bid_val, name_val]);
    }

    /// Fire `OnLspDetach (bid server-name)` — called from `lsp_stop_one` for
    /// every buffer that was attached to the server being stopped, right
    /// after `buf.lsp_server` is cleared.
    pub(super) fn fire_hook_lsp_detach(&mut self, bid: BufferId, server_name: &str) {
        let bid_val = SteelBufferId::new(bid).into_steel_val();
        let name_val = SteelVal::StringV(server_name.into());
        self.fire_hook_silent(HookId::OnLspDetach, &[bid_val, name_val]);
    }

    /// Fire `OnDiagnosticsChanged (bid)` — payload-free signal, once per
    /// buffer a `publishDiagnostics` drain batch actually touched
    /// (`drain_lsp`). Handlers pull via `(diagnostics-for-buffer bid …)`.
    pub(super) fn fire_hook_diagnostics_changed(&mut self, bid: BufferId) {
        let val = SteelBufferId::new(bid).into_steel_val();
        self.fire_hook_silent(HookId::OnDiagnosticsChanged, &[val]);
    }

    /// Fire `OnViewportChange (bid first-line last-line)` for `pane_id` —
    /// called only when its debounce timer actually fires (`timer_bridge`),
    /// reading the pane's *current* bounds rather than whatever they were
    /// when the timer was armed. A no-op if the pane closed in the meantime.
    pub(super) fn fire_hook_viewport_change(&mut self, pane_id: hume_engine::pipeline::PaneId) {
        let Some(pane) = self.view.panes.get(pane_id) else {
            return;
        };
        let bid = pane.buffer_id;
        let total_lines = self.state.buffers.get(bid).text().len_lines();
        let (first_line, last_line) = super::lsp::introspect::pane_visible_range(pane, total_lines);
        let bid_val = SteelBufferId::new(bid).into_steel_val();
        self.fire_hook_silent(
            HookId::OnViewportChange,
            &[
                bid_val,
                SteelVal::IntV(first_line as isize),
                SteelVal::IntV(last_line as isize),
            ],
        );
    }

    /// Fire `OnTriggerChar (bid char-string source)` — Insert mode, after
    /// `ch` has already been inserted into `bid` (mappings/insert.rs), once
    /// per source registered for `ch` under `bid`'s language via
    /// `(register-trigger-chars! source language chars)`.
    pub(super) fn fire_hook_trigger_char(&mut self, bid: BufferId, ch: char, source: &str) {
        let bid_val = SteelBufferId::new(bid).into_steel_val();
        let ch_val = SteelVal::StringV(ch.to_string().into());
        let source_val = SteelVal::StringV(source.into());
        self.fire_hook_silent(HookId::OnTriggerChar, &[bid_val, ch_val, source_val]);
    }

    /// Fire all Steel handlers for `hook_id`, passing `args` to each.
    ///
    /// Enqueue `hook_id` to fire after the current command returns.
    ///
    /// The unified hook-firing path: all hook scheduling goes through
    /// `state.config.pending_hooks`; `Editor::drain_hooks` does the actual Steel eval.
    /// This prevents re-entrant Steel calls during command execution and gives
    /// a single drain point for both the keypress and sync-Steel paths.
    pub(super) fn fire_hook_silent(&mut self, hook_id: HookId, args: &[steel::rvals::SteelVal]) {
        self.state
            .config
            .pending_hooks
            .push((hook_id, args.to_vec()));
    }

    /// Fire every hook in `state.config.pending_hooks`, draining the queue.
    ///
    /// Called once per interactive input event by `handle_event` (the single
    /// interactive drain boundary), and once at startup in `lib.rs` before the
    /// event loop begins. Inner hook handlers may enqueue more hooks; the outer
    /// loop re-drains until the queue is empty — capped at
    /// [`MAX_HOOK_DRAIN_HOOKS`] total hooks processed (not passes) so neither a
    /// constant-width ping-pong loop (e.g. two `on-language-set` handlers
    /// flipping `set-buffer-language!` between two values) nor an amplifying
    /// cascade (a handler that enqueues more hooks than it received, doubling
    /// the batch pass over pass) can livelock the editor.  The watchdog only
    /// bounds each individual eval, not this loop.
    pub(crate) fn drain_hooks(&mut self) {
        // Any mode change queued between the last consumption point and now
        // (a hook handler earlier this same batch, or something that ran
        // before `drain_hooks` was even called) must not survive into the
        // handler calls below. Unconditional, at the top, so no early-return
        // branch below can skip it.
        self.take_pending_lsp_completion_dismiss();
        let mut total_processed = 0usize;
        while !self.state.config.pending_hooks.is_empty() {
            let hooks = std::mem::take(&mut self.state.config.pending_hooks);
            total_processed += hooks.len();
            if total_processed > MAX_HOOK_DRAIN_HOOKS {
                // `hooks` was just drained from `pending_hooks` above, so
                // nothing has been re-enqueued yet — it's the entire drop.
                let dropped = hooks.len();
                self.report(
                    Severity::Error,
                    format!(
                        "hook cascade exceeded {MAX_HOOK_DRAIN_HOOKS} total drained hook(s) — \
                         dropping {dropped} pending hook(s); handler feedback loop?"
                    ),
                );
                return;
            }
            for (hook_id, args) in hooks {
                // Activate lazy event plugins first so their register-hook! calls
                // land before the has_hook_handlers check below.
                self.activate_lazy_event_plugins(hook_id);
                if self
                    .scripting
                    .as_ref()
                    .is_none_or(|h| !h.has_hook_handlers(hook_id))
                {
                    continue;
                }
                let pid = self.state.focused_pane_id;
                let bid = self.focused_buffer_id();
                let result = {
                    let host_scr = self.scripting.as_mut().expect("checked above");
                    let mut impl_host = EditorHostImpl {
                        state: &mut self.state,
                        view: &mut self.view,
                        lsp: Some(&mut self.lsp),
                        timers: Some(super::timer_bridge::TimerHandle {
                            wheel: &mut self.timer_wheel,
                            payloads: &mut self.timer_payloads,
                        }),
                        terminal: self.terminal.as_ref(),
                    };
                    host_scr.fire_hook(hook_id, &args, pid, bid, &mut impl_host)
                };
                self.flush_script_messages();
                match result {
                    Ok(effects) => self.apply_script_effects(effects),
                    Err(e) => {
                        self.apply_script_effects(e.effects);
                        self.report(Severity::Error, format!("hook error: {}", e.message));
                    }
                }
            }
        }
    }

    /// Queue `(proc, args)` for evaluation at the next drain boundary —
    /// never called inline (LSP dispatch, timer fire, and minibuffer key
    /// handling all detect their completion from inside a borrow that can't
    /// re-enter Steel). Shared delivery mechanism for the `lsp-request`
    /// callback, timer thunks, and the prompt callback.
    pub(crate) fn queue_steel_call(&mut self, proc: SteelVal, args: Vec<SteelVal>) {
        self.state.config.pending_steel_calls.push((proc, args));
    }

    /// Drain `state.config.pending_steel_calls`, evaluating each queued call in one
    /// Steel session. Called once per frame from `prepare_frame` — the
    /// per-frame cadence LSP responses and timer fires already drain on, so
    /// a completion queued this frame runs before the next render rather
    /// than waiting for the next keystroke (unlike hooks, nothing here is
    /// naturally re-triggering, so a single pass is enough — anything a
    /// callback itself queues lands in next frame's drain, not this one).
    pub(crate) fn drain_pending_steel_calls(&mut self) {
        // Same reasoning as the top of `drain_hooks` — unconditional so no
        // early-return branch below can skip it. `prepare_frame` calls this
        // every frame, so no separate render-time consumption is needed.
        self.take_pending_lsp_completion_dismiss();
        let calls = std::mem::take(&mut self.state.config.pending_steel_calls);
        if calls.is_empty() {
            return;
        }
        let pid = self.state.focused_pane_id;
        let bid = self.focused_buffer_id();
        let Some(host_scr) = self.scripting.as_mut() else {
            return;
        };
        let result = {
            let mut impl_host = EditorHostImpl {
                state: &mut self.state,
                view: &mut self.view,
                lsp: Some(&mut self.lsp),
                timers: Some(super::timer_bridge::TimerHandle {
                    wheel: &mut self.timer_wheel,
                    payloads: &mut self.timer_payloads,
                }),
                terminal: self.terminal.as_ref(),
            };
            host_scr.run_steel_calls(calls, pid, bid, &mut impl_host)
        };
        self.flush_script_messages();
        match result {
            Ok(effects) => self.apply_script_effects(effects),
            Err(e) => {
                self.apply_script_effects(e.effects);
                self.report(Severity::Error, format!("steel call error: {}", e.message));
            }
        }
        // A call just run above (an LSP-request callback, a timer thunk) can
        // itself dispatch a command that exits Insert, setting the flag the
        // top-of-function consumption already passed. Consume it again so
        // `prepare_frame`'s later `sync_completion_menu_view` never repaints a
        // session `set_mode` asked to close mid-drain.
        self.take_pending_lsp_completion_dismiss();
    }

    // ── Scripting ─────────────────────────────────────────────────────────────

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

    /// Initialise the Steel scripting host and evaluate `init.scm`.
    ///
    /// Called once at startup, after `Editor::open` returns and before
    /// `Editor::run` starts (with `snapshot` at its `Default` — nothing to
    /// restore, no pre-reload buffers), and by `typed_reload_config` on
    /// every `:reload-config` (with the `ReloadSnapshot` `reset_config_state`
    /// just produced). Any error from `init.scm` is reported as
    /// `Severity::Error` and shown in the statusline.
    pub(crate) fn init_scripting(&mut self, snapshot: &mut ReloadSnapshot) {
        // Resolve the config path up front. `None` means neither XDG_CONFIG_HOME
        // nor HOME (APPDATA on Windows) is set — there is no meaningful place
        // to look for init.scm, so we skip scripting entirely and log a warning.
        let Some(config_dir) = hume_platform::dirs::config_dir() else {
            self.report(
                Severity::Warning,
                "scripting: no config directory — HOME/APPDATA unset; init.scm skipped".into(),
            );
            return;
        };
        let init_path = config_dir.join("init.scm");
        let mut host = hume_scripting::ScriptingHost::new();
        // Pre-register every native command name as a callable Steel binding before
        // any user code sees the engine.  This lets `init.scm` call `(move-left)`
        // directly without a FreeIdentifier compile error.  Only native commands
        // (Motion/Selection/Edit/EditorCmd) are registered — plugin commands
        // (`SteelBacked`/`Lazy`) don't exist yet and use `(call! …)` instead.
        {
            let names: Vec<&str> = self.state.config.registry.native_mappable_names().collect();
            host.register_command_names(&names);
        }
        // Trace the resolved directories so they're visible in `:messages`.
        // A missing runtime dir is a warning because `core:*` plugins need it.
        match host.runtime_dir() {
            Some(rt) => self.report(Severity::Trace, format!("scripting: runtime dir = {}", rt.display())),
            None => self.report(Severity::Warning, "scripting: no runtime directory found — core:* plugins unavailable; set HUME_RUNTIME to fix".into()),
        }
        match host.data_dir() {
            Some(d) => self.report(
                Severity::Trace,
                format!("scripting: data dir = {}", d.display()),
            ),
            None => self.report(
                Severity::Warning,
                "scripting: no data directory — HOME/APPDATA unset; user plugins unavailable"
                    .into(),
            ),
        }
        // Capture built-in names before any plugin code runs; stable for the
        // editor's lifetime.  Stored on Editor so dispatch-time activation can
        // borrow it disjointly from &mut self.scripting / settings / keymap.
        let builtin_names: rustc_hash::FxHashSet<String> = self
            .state
            .config
            .registry
            .names()
            .map(String::from)
            .collect();
        self.builtin_cmd_names = builtin_names.clone();
        // Load runtime/scheme/prelude.scm before init.scm so its macros
        // (bind-keys! etc.) are available to init.scm and plugin modules.
        // Then languages.scm ((define-language! …) identities) and
        // grammars.scm (registers already-compiled grammars — see its own
        // header) — languages.scm runs first so identity registration owns
        // the one glob-set rebuild (attach_grammar would otherwise create a
        // bare default identity per grammar, ahead of languages.scm's own).
        // Each is a silent no-op when the runtime dir or the file itself is
        // missing (optional layers); a file that exists but fails to
        // parse/eval is an error reported separately.
        //
        // Each eval gets a fresh EditorHostImpl over `state` + `view`; the
        // `require_cmd_ctx!` guard keeps command-mode builtins unreachable
        // during init evals.
        self.eval_runtime_scheme(&mut host, "scheme/prelude.scm", builtin_names.clone());
        self.eval_runtime_scheme(&mut host, "scheme/languages.scm", builtin_names.clone());
        self.eval_runtime_scheme(&mut host, "scheme/grammars.scm", builtin_names.clone());
        {
            let init_budget = self.state.settings.steel_init_budget_ms as u64;
            let result = {
                let mut ih = make_init_host(&mut self.state, &mut self.view);
                host.eval_init(&init_path, init_budget, &mut ih, builtin_names)
            };
            match result {
                Ok(effects) => self.apply_script_effects(effects),
                Err(e) => {
                    self.apply_script_effects(e.effects);
                    self.report(Severity::Error, format!("init.scm: {}", e.message));
                }
            }
        }
        // Snapshot language activation entries for the post-init lint below —
        // every eval's effects (identities, grammars, LSP server ops) are
        // already applied above, each right after its own eval, so
        // `self.state.config.languages` is fully populated by this point.
        let lang_activations = host.activation_languages();
        // Flush any `(log! …)` messages produced during init.scm evaluation.
        for (level, text) in host.take_pending_messages() {
            self.report(log_level_to_severity(level), text);
        }
        self.scripting = Some(host);
        // Post-init lint: warn on keymap leaves that target an unknown command.
        // Lazy stubs are registered live as each declare-plugin call runs during
        // init.scm eval, so they already count as valid commands by this point.
        // Built-in keymaps only reference registered built-ins, so any warnings
        // here come from user bind-key! calls to typos / undeclared commands.
        {
            let mut names = self.state.config.keymap.all_command_names();
            names.sort_unstable();
            names.dedup();
            for name in &names {
                if !self.state.config.registry.contains(name) {
                    self.report(
                        Severity::Warning,
                        format!("key bound to unknown command '{name}' — typo, or missing from #:commands?"),
                    );
                }
            }
        }
        // Post-init lint: warn on #:languages activation entries whose language name
        // is not in the final LanguageRegistry.  Runs after the message flush above
        // so both languages.scm and init.scm's own define-language! calls are visible.
        // Uses by_name() (identity registered), not has_grammar() (grammar attached):
        // an activation entry is valid for any known language identity even without a
        // grammar.  Not a hard error: inert but harmless, and language sets are
        // open/dynamic (a future define-language! + reload may make the name valid).
        for (lang, plugins) in &lang_activations {
            // "*" is the any-language wildcard (manifest.scm can't enumerate every
            // language it might ever support) — not a language identity to look up.
            if lang != "*" && self.state.config.languages.by_name(lang).is_none() {
                for plugin in plugins {
                    self.report(
                        Severity::Warning,
                        format!(
                            "plugin '{plugin}' declares #:languages activation for unknown \
                             language '{lang}' — typo, or missing (define-language!)?"
                        ),
                    );
                }
            }
        }
        // Re-detect language for every open buffer (the startup buffer is
        // opened in lib.rs before init_scripting is called; a
        // `:reload-config` cleared every buffer's language in
        // `reset_config_state`). A buffer with an explicit assertion —
        // restored from `snapshot` (this is a `:reload-config`) or made by
        // `init.scm` itself against a buffer that predates this call (e.g.
        // the startup buffer) — skips detection entirely: `set_buffer_
        // language_impl` stamps `language_explicit` unconditionally, so a
        // later detection pass would otherwise silently overwrite that
        // assertion with whatever plain detection finds. Restoring the
        // snapshot *inside* this loop, rather than as a second
        // detect-then-correct pass afterward, also avoids attaching an LSP
        // server for the buffer's *detected* language before its real,
        // explicit one goes back — `lsp_attach_buffer` is a no-op once
        // attached, so that wrong attach would otherwise stick.
        let explicit_restore: rustc_hash::FxHashMap<BufferId, Option<String>> =
            snapshot.take_explicit_languages().into_iter().collect();
        let open_bids: Vec<_> = self.state.buffers.iter().map(|(id, _)| id).collect();
        for bid in open_bids {
            if let Some(name) = explicit_restore.get(&bid) {
                // Only valid if `bid` is still the same buffer instance the
                // snapshot meant — `close_buffer`'s last-buffer scratch
                // replacement can otherwise alias a closed buffer's explicit
                // language onto unrelated fresh content (see
                // `ReloadSnapshot::survives`).
                if snapshot.survives(bid, &self.state.buffers) {
                    match name {
                        Some(name) => match self.state.config.languages.id_of(name) {
                            Some(lang_id) => self.set_buffer_language_explicit(bid, Some(lang_id)),
                            None => {
                                self.report(
                                    Severity::Warning,
                                    format!(
                                        "language '{name}' was explicitly set on a buffer \
                                         before reload, but is no longer registered — \
                                         falling back to detection"
                                    ),
                                );
                                self.detect_and_set_language(bid);
                            }
                        },
                        None => self.set_buffer_language_explicit(bid, None),
                    }
                    continue;
                }
            } else if self.state.buffers.get(bid).language_explicit {
                // Asserted during *this very* init.scm eval (e.g. on a
                // buffer that predates this call) — detection must not
                // clobber it either; see `set_buffer_language_impl`'s doc.
                continue;
            }
            self.detect_and_set_language(bid);
        }
    }

    /// Evaluate a bundled runtime Scheme file (`rel_path`, relative to the
    /// runtime dir) as an init-mode eval — shared by `init_scripting`'s
    /// prelude/languages/grammars loads. A missing runtime dir or a missing
    /// file at that path is a silent no-op (all three are optional layers);
    /// a file that exists but fails to parse/eval is reported as an error.
    fn eval_runtime_scheme(
        &mut self,
        host: &mut hume_scripting::ScriptingHost,
        rel_path: &str,
        builtin_names: rustc_hash::FxHashSet<String>,
    ) {
        let Some(path) = host.runtime_dir().map(|rt| rt.join(rel_path)) else {
            return;
        };
        let init_budget = self.state.settings.steel_init_budget_ms as u64;
        let result = {
            let mut ih = make_init_host(&mut self.state, &mut self.view);
            host.eval_init(&path, init_budget, &mut ih, builtin_names)
        };
        match result {
            Ok(effects) => self.apply_script_effects(effects),
            Err(e) => {
                self.apply_script_effects(e.effects);
                self.report(
                    Severity::Error,
                    format!("runtime/{rel_path}: {}", e.message),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Module-level helpers
// ---------------------------------------------------------------------------

/// Build an `EditorHostImpl` from disjoint borrows of the editor's state and view.
pub(crate) fn make_init_host<'a>(
    state: &'a mut super::EditorState,
    view: &'a mut hume_engine::pipeline::EngineView,
) -> EditorHostImpl<'a> {
    EditorHostImpl::new(state, view)
}

/// Map the scripting layer's `BindMode` (carried in `Effect::BindKey` and
/// friends) to the editor's own. Fully qualified on both sides: this module
/// works with `crate::editor::keymap::BindMode` too, so importing either name
/// bare would shadow the other.
fn to_editor_bind_mode(mode: hume_scripting::BindMode) -> crate::editor::keymap::BindMode {
    match mode {
        hume_scripting::BindMode::Normal => crate::editor::keymap::BindMode::Normal,
        hume_scripting::BindMode::Extend => crate::editor::keymap::BindMode::Extend,
        hume_scripting::BindMode::Insert => crate::editor::keymap::BindMode::Insert,
    }
}

/// Map scripting `LogLevel` → editor `Severity`.
pub(crate) fn log_level_to_severity(level: hume_scripting::LogLevel) -> Severity {
    use hume_scripting::LogLevel;
    match level {
        LogLevel::Info => Severity::Info,
        LogLevel::Warning => Severity::Warning,
        LogLevel::Error => Severity::Error,
        LogLevel::Trace => Severity::Trace,
    }
}

/// Ordered list of directories to search for theme TOML files.
///
/// Config themes (user-defined) are listed before runtime themes (bundled) so
/// that user overrides shadow built-in ones. Both `theme::load_theme_by_name`
/// and [`ThemeCompleter`] use this list as the single source of truth.
pub(super) fn theme_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(cfg) = hume_platform::dirs::config_dir() {
        paths.push(cfg.join("themes"));
    }
    if let Some(rt) = hume_platform::dirs::runtime_dir() {
        paths.push(rt.join("themes"));
    }
    paths
}
