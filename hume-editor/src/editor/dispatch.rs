//! Unified command dispatch pipeline: the `&mut Editor` half.
//!
//! [`commands::run_dispatch_pipeline`] handles the `&mut EditorState + &mut
//! EngineView` half (native commands, and the BEFORE/AFTER pipeline stages
//! shared with Steel-backed commands). This module holds the Steel-backed
//! path, which additionally needs `self.scripting`, `self.lsp`, and the
//! timer bridge — fields only reachable through `&mut Editor`.

use super::event::EditorEvent;
use super::registry::MappableCommand;
use super::{Editor, InlineOutputDispatch, Severity, commands};

// ── Command dispatch context ──────────────────────────────────────────────────

/// Per-dispatch context assembled by the key handler and passed through
/// [`Editor::dispatch`].
#[derive(Debug, Clone)]
pub(crate) struct CmdCtx {
    /// Numeric count prefix. `None` means "no count was typed" — a bare
    /// keyboard press, which visual-move commands read as one visual row
    /// (`state.explicit_count`, set from this by `run_native_body`). Producible
    /// by the keymap trie leaves / WaitChar arm, and also by Steel: a script
    /// passes a count of `0` (`parse_count_extend` decodes it to `None`) to ask
    /// for the same "as if no count was typed" behavior. `Some(n)` is every
    /// other case — an explicit user count, a script's explicit `n`, or a
    /// non-keybind origin's default (insert-mode leaf, no-arg `call!`).
    pub count: Option<usize>,
    /// Whether this command runs in Extend mode.
    pub extend: bool,
}

impl Editor {
    // ── Unified command dispatch pipeline ──────────────────────────────────────

    /// Execute a `MappableCommand` through the unified dispatch pipeline.
    ///
    /// Native commands delegate to [`commands::run_dispatch_pipeline`].  Steel-backed
    /// commands run the pipeline's BEFORE/AFTER stages inline, with the body
    /// executed via [`Editor::run_steel_command`] (which needs `&mut Editor` for
    /// `self.scripting`).
    ///
    /// Dot-repeat replay bypasses this entirely — it calls
    /// [`commands::run_native_body`] directly.
    pub(crate) fn dispatch(&mut self, cmd: MappableCommand, ctx: CmdCtx) {
        let is_steel = matches!(
            &cmd,
            MappableCommand::SteelBacked { .. } | MappableCommand::Lazy { .. }
        );
        if !is_steel {
            // Native path — delegate to the standalone pipeline.
            commands::run_dispatch_pipeline(&mut self.state, &mut self.view, cmd, ctx);
            return;
        }

        // Steel path — composed from shared step functions.
        let meta = cmd.meta();
        // Clone the name once, before the body consumes `cmd`.
        let name = cmd.name().clone();

        // BEFORE
        commands::step_paste_commit(&mut self.state, &self.view, meta.defers_paste_commit);
        let char_arg = self.state.pending_char.take();
        // Snapshot the recipe before the body by cloning, not `mem::take`: an
        // inner `call!` dispatch (e.g. vim-keybind's `C` wrapper calling
        // `copy-selection-on-next-line`) must see and compose onto whatever
        // the user already staged, the same as if that inner command were
        // dispatched directly from the keymap. `step_stamp_repeatable` below
        // still reads this snapshot, not the (possibly further-mutated) live
        // value, so a repeatable outer command's stamped recipe reflects
        // state as of entry — not whatever an inner dispatch built on top of it.
        let pre_recipe = self.state.selection_recipe.clone();
        let pre_writes = self.state.selection_recipe_writes;

        // BODY — consumes `cmd`.
        if !self.run_steel_command(cmd, name.as_ref(), &ctx, char_arg) {
            // Failed command: undo whatever a partial inner dispatch wrote,
            // so the failure has no residual effect on the recipe.
            self.state.selection_recipe = pre_recipe;
            return;
        }

        // AFTER — re-query to get the resolved command's repeatable flag.
        // A Lazy stub becomes SteelBacked after activation; re-query reflects that.
        let repeatable = self
            .state
            .config
            .registry
            .get_mappable(name.as_ref())
            .is_some_and(|c| c.meta().repeatable);
        if repeatable {
            // Outer-name-wins: stamp the outer command so `.` replays it, not
            // any inner native command the body dispatched via `call!`.
            commands::step_stamp_repeatable(
                &mut self.state,
                &name,
                ctx.count.unwrap_or(1),
                char_arg,
                Some(pre_recipe),
            );
        }
        // Clear only when this outer command owns the decision: a repeatable
        // command's recipe was just consumed by the stamp above — matching
        // the native `Edit` variant (always `Untracked`, so `step_update_recipe`
        // clears it too) — and a body that dispatched nothing natively never
        // ran `step_update_recipe` to decide for itself, so this non-selection
        // outer command must clear it exactly as any other `Untracked`
        // command would. A body that DID dispatch natively already got the
        // correct decision from that inner call's own `step_update_recipe`,
        // which must not be overridden.
        if repeatable || self.state.selection_recipe_writes == pre_writes {
            self.state.selection_recipe.clear();
        }
        // Outer Steel commands skip step_record_jump and step_clear_extend: their meta
        // hardcodes is_jump = clears_extend = false. An inner native (call! …) still
        // fires both — it routes through run_dispatch_pipeline with its own meta.
    }

    /// Activate `plugin` (a `Lazy` stub's owner), reporting the standard
    /// warning on failure.
    ///
    /// Shared by [`Self::run_steel_command`] (a mappable `Lazy` stub) and
    /// [`Self::run_typed_steel_command`] (a typed `TypedBody::Lazy` stub) —
    /// the two Lazy-stub call sites, one per registry kind.
    fn activate_lazy_and_report(
        &mut self,
        plugin: &hume_scripting::attribution::PluginId,
        name: &str,
    ) -> bool {
        if self.activate_lazy_plugin(plugin, name) {
            true
        } else {
            self.report(Severity::Warning, format!("unknown command: {name}"));
            false
        }
    }

    /// Run the body of a Steel-backed or Lazy mappable command (bound to a
    /// key, or dispatched via `call!`/dot-repeat).
    ///
    /// Returns `false` if the command aborted (lazy activation failure, scripting
    /// error, or `scripting` is `None`). On error, the caller skips AFTER stages.
    pub(super) fn run_steel_command(
        &mut self,
        cmd: MappableCommand,
        name: &str,
        ctx: &CmdCtx,
        char_arg: Option<char>,
    ) -> bool {
        // Injected into the lambda's `count` param verbatim — `0` is the Scheme
        // spelling of `None` ("no count was typed"), so a wrapper that forwards
        // this value straight into `(call! "move-down" count extend)` round-trips
        // a bare keypress back to visual-row movement (`parse_count_extend`
        // decodes `0` back to `None` on the way in).
        let count = ctx.count.unwrap_or(0);
        let extend = ctx.extend;

        // For a Lazy stub, activate the owning plugin now so we can read
        // `inline_output` from the resolved SteelBacked entry before dispatch.
        if let MappableCommand::Lazy { plugin, .. } = &cmd {
            let plugin = plugin.clone();
            if !self.activate_lazy_and_report(&plugin, name) {
                return false;
            }
        }

        // Re-query: a Lazy stub is now SteelBacked after activation above;
        // a SteelBacked entry is unchanged. Pure registry metadata — resolved
        // (and its arity/arg-count errors reported) before the `scripting`
        // guard below, so a `:cmd` arity mismatch is reported even in the
        // (test-only) case where a SteelBacked entry exists in the registry
        // but no scripting host is installed.
        let (inline_output, cmd_arity, cmd_is_variadic) =
            match self.state.config.registry.get_mappable(name) {
                Some(MappableCommand::SteelBacked {
                    inline_output,
                    arity,
                    is_variadic,
                    ..
                }) => (*inline_output, *arity, *is_variadic),
                _ => {
                    self.report(
                        Severity::Error,
                        format!("{name}: internal error — command lost after activation"),
                    );
                    return false;
                }
            };

        // Inject count and extend as leading lambda args based on declared arity.
        if cmd_arity > 2 {
            self.report(
                Severity::Error,
                format!(
                    "{name}: lambda declares {cmd_arity} required params; \
                     keymap injection supplies at most 2 (count, extend)"
                ),
            );
            return false;
        }
        let effective_args = match (cmd_arity, cmd_is_variadic) {
            (0, false) => vec![],
            (1, false) => vec![steel::rvals::SteelVal::IntV(count as isize)],
            _ => vec![
                steel::rvals::SteelVal::IntV(count as isize),
                steel::rvals::SteelVal::BoolV(extend),
            ],
        };

        self.call_steel_command_body(name, char_arg, effective_args, inline_output)
    }

    /// Run the body of a Steel-backed or Lazy *typed* command, dispatched
    /// from the `:` command line via `(define-typed-command! …)`.
    ///
    /// Mirrors [`Self::run_steel_command`] but resolves metadata from
    /// [`super::registry::TypedBody`] rather than `MappableCommand`, and
    /// marshals `(arg force)` rather than `(count extend)` — the two never
    /// share a lookup because the registry keeps the kinds strictly separate
    /// (see `registry/mod.rs`'s module doc). Returns `false` on the same
    /// failure classes as `run_steel_command`.
    pub(super) fn run_typed_steel_command(
        &mut self,
        name: &str,
        lazy_plugin: Option<hume_scripting::attribution::PluginId>,
        arg: Option<String>,
        force: bool,
    ) -> bool {
        if let Some(plugin) = lazy_plugin
            && !self.activate_lazy_and_report(&plugin, name)
        {
            return false;
        }

        let (inline_output, cmd_arity, cmd_is_variadic) = match self
            .state
            .config
            .registry
            .get_typed(name)
            .map(|tc| &tc.body)
        {
            Some(super::registry::TypedBody::Steel {
                inline_output,
                arity,
                is_variadic,
            }) => (*inline_output, *arity, *is_variadic),
            _ => {
                self.report(
                    Severity::Error,
                    format!("{name}: internal error — command lost after activation"),
                );
                return false;
            }
        };

        if cmd_arity > 2 {
            self.report(
                Severity::Error,
                format!(
                    "{name}: lambda declares {cmd_arity} required params; \
                     typed-command injection supplies at most 2 (arg, force)"
                ),
            );
            return false;
        }
        // Scheme-idiomatic absence: an untyped argument is `#f`, not a
        // sentinel string or a fabricated count — a lambda that only cares
        // whether an arg was given writes a plain `(if arg …)` guard.
        let arg_val = match &arg {
            Some(s) => steel::rvals::SteelVal::StringV(s.clone().into()),
            None => steel::rvals::SteelVal::BoolV(false),
        };
        let effective_args = match (cmd_arity, cmd_is_variadic) {
            (0, false) => vec![],
            (1, false) => vec![arg_val],
            _ => vec![arg_val, steel::rvals::SteelVal::BoolV(force)],
        };

        self.call_steel_command_body(name, None, effective_args, inline_output)
    }

    /// Invoke a Steel command lambda by name with pre-marshalled positional
    /// args, bracketing the call for `#:inline-output` commands.
    ///
    /// Shared by [`Self::run_steel_command`] and
    /// [`Self::run_typed_steel_command`] — the two differ only in how they
    /// resolve `name`'s metadata and marshal `effective_args`; the Steel
    /// invocation, alt-screen bracket, and effect application are one funnel.
    fn call_steel_command_body(
        &mut self,
        name: &str,
        char_arg: Option<char>,
        effective_args: Vec<steel::rvals::SteelVal>,
        inline_output: bool,
    ) -> bool {
        let focused_pane_id = self.state.focused_pane_id;
        let focused_buffer_id = self.focused_buffer_id();

        // Alt-screen bracketing for inline-output commands is lazy: entering
        // the alt-screen and printing the running banner happens on the
        // command body's *first actual output* (see
        // `EditorHostImpl::ensure_inline_output_screen`), not eagerly here —
        // a body that only logs (`log!`) never flashes an empty screen or
        // blocks on a keypress nobody needed to answer. `Armed` just primes
        // the state SteelCtx reads through `is_inline_output_command`; off
        // the event loop (tests, headless `run_keys`) there is no alt-screen
        // to leave and no interactive user to answer a keypress prompt, so a
        // declared command goes `Headless` instead — stdout writes stay
        // permitted, but no bracket ever runs.
        self.state.inline_output = if inline_output && self.tui_active {
            InlineOutputDispatch::Armed {
                kitty: self.kitty_enabled,
                mouse: self.state.settings.mouse_enabled,
                name: name.to_string(),
            }
        } else if inline_output {
            InlineOutputDispatch::Headless
        } else {
            InlineOutputDispatch::Inactive
        };

        let Some(scripting) = self.scripting.as_mut() else {
            return false;
        };
        let tui_active = self.tui_active;
        let kitty_enabled = self.kitty_enabled;
        let result = {
            let mut impl_host = crate::editor::host_impl::EditorHostImpl::full(
                &mut self.state,
                &mut self.view,
                &mut self.lsp,
                &mut self.timer_wheel,
                &mut self.timer_payloads,
                self.terminal.as_ref(),
                tui_active,
                kitty_enabled,
            );
            scripting.call_steel_cmd(
                name,
                char_arg,
                effective_args,
                focused_pane_id,
                focused_buffer_id,
                &mut impl_host,
            )
        };

        // Close the bracket only if a builtin actually opened it. This runs
        // before `match result` below so a Steel error raised after screen
        // entry still gets the TUI restored first.
        self.close_inline_output_bracket();

        let (wait_char_cmd, effects) = match result {
            Ok(r) => (r.wait_char_request, r.effects),
            Err(e) => {
                self.apply_script_effects(e.effects);
                self.report(Severity::Error, e.message);
                return false;
            }
        };

        self.flush_script_messages();
        self.apply_script_effects(effects);
        if let Some(wc) = wait_char_cmd {
            self.state.wait_char = Some(crate::editor::keymap::WaitCharPending {
                cmd_name: wc.into(),
                ctrl_extend: false,
            });
        }

        true
    }

    /// Close the `#:inline-output` bracket if a builtin actually opened it,
    /// and queue the disk-change sweep an entered/armed subprocess may
    /// warrant. Scoped to whatever `state.inline_output` holds right now, so
    /// it fits every Steel-session boundary that can arm one: the top-level
    /// dispatch that owns the whole bracket lifecycle
    /// ([`Self::call_steel_command_body`]), and a hook/queued-call batch
    /// that only ever *arms* one indirectly, through a nested
    /// `(call! …)` to an `#:inline-output` command
    /// (`hume_scripting::host::OutputHost::arm_inline_output`).
    ///
    /// Reset unconditionally so stale state can't outlive this call and leak
    /// into a later dispatch's `SteelCtx`.
    ///
    /// `armed_or_entered` covers both shapes an inline-output command can
    /// take: `Entered` (it produced output, so the alt-screen bracket needs
    /// closing below) and `Armed` (it declared `#:inline-output` and ran, but
    /// produced none — a formatter with no stdout, say — so there's no
    /// bracket to close). Either way the subprocess ran with the real
    /// terminal and may well have rewritten one of our open files, so both
    /// are disk-change check trigger points; only `Headless`/`Inactive` (no
    /// interactive user to answer a confirm) are excluded.
    pub(super) fn close_inline_output_bracket(&mut self) {
        let armed_or_entered = matches!(
            self.state.inline_output,
            InlineOutputDispatch::Armed { .. } | InlineOutputDispatch::Entered
        );
        let entered = matches!(self.state.inline_output, InlineOutputDispatch::Entered);
        self.state.inline_output = InlineOutputDispatch::Inactive;
        if entered {
            let term = self
                .terminal
                .as_ref()
                .expect("Entered implies tui_active implies terminal is Some");
            hume_platform::terminal::print_return_prompt();
            hume_platform::terminal::wait_for_keypress(term);
            let kitty = self.kitty_enabled;
            let mouse = self.state.settings.mouse_enabled;
            let mouse_select = self.state.settings.mouse_select;
            let _ = hume_platform::terminal::leave_inline_output(term, kitty, mouse, mouse_select);
            self.state.force_full_redraw = true;
        }
        if armed_or_entered {
            // The editor genuinely regained the terminal — same trigger class
            // as `TerminalEvent::FocusIn`, so it raises the same event rather
            // than sweeping directly; the reaction is `OnFocusGained`'s Rust
            // handler in `Editor::react_to_event`. That reaction runs inside
            // the next `settle()`, after `message_logged_this_input` has
            // already been set from this same dispatch's own message-log
            // delta — `can_open_confirm`'s message-shadow clause is scoped to
            // `DiskCheckTrigger::BufferEnter` for exactly this reason, so a
            // warning this command logged itself can't suppress the reload
            // confirm its own subprocess just caused.
            self.state.queue_event(EditorEvent::OnFocusGained);
        }
    }
}
