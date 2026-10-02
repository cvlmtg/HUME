//! Unified command dispatch pipeline: the `&mut Editor` half.
//!
//! [`commands::run`] handles the `&mut EditorState + &mut
//! EngineView` half (native commands, and the BEFORE/AFTER pipeline stages
//! shared with Steel-backed commands). This module holds the Steel-backed
//! path, which additionally needs `self.scripting`, `self.lsp`, and the
//! timer bridge: fields only reachable through `&mut Editor`.

use super::event::EditorEvent;
use super::registry::MappableCommand;
use super::{Editor, Severity, commands};

// ── Command dispatch context ──────────────────────────────────────────────────

/// Per-dispatch context assembled by the key handler and passed through
/// [`Editor::dispatch`].
#[derive(Debug, Clone)]
pub(in crate::editor) struct CmdCtx {
    /// Numeric count prefix. `None` means "no count was typed": a bare
    /// keyboard press, which visual-move commands read as one visual line
    /// (`state.explicit_count`, set from this by `run_body`). Producible
    /// by the keymap trie leaves / WaitChar arm, and also by Steel: a script
    /// passes a count of `0` (`parse_count_extend` decodes it to `None`) to ask
    /// for the same "as if no count was typed" behavior. `Some(n)` is every
    /// other case: an explicit user count, a script's explicit `n`, or a
    /// non-keybind origin's default (insert-mode leaf, no-arg `call!`).
    pub count: Option<usize>,
    /// Whether this command runs in Extend mode.
    pub extend: bool,
}

impl Editor {
    // ── Unified command dispatch pipeline ──────────────────────────────────────

    /// Execute a `MappableCommand` through the unified dispatch pipeline.
    ///
    /// Native commands delegate to [`commands::run`].  Steel-backed
    /// commands run the pipeline's BEFORE/AFTER stages inline, with the body
    /// executed via [`Editor::run_steel_command`] (which needs `&mut Editor` for
    /// `self.scripting`).
    ///
    /// Dot-repeat replay bypasses this for the commands it replays. It
    /// calls [`commands::run_body`] or `run_steel_command` directly, though
    /// a replayed Steel body's own `call!`s still reach [`commands::run`].
    pub(in crate::editor) fn dispatch(&mut self, cmd: MappableCommand, ctx: CmdCtx) {
        // Native path: a keypress always acts at the focused pane. `Err`
        // hands `cmd` back unbound for a Steel-backed/Lazy command, which
        // the path below dispatches instead.
        let fp = commands::FocusedPane::current(&self.state);
        let cmd = match commands::BoundCommand::focused(cmd, fp) {
            Ok(bound) => {
                commands::run(&mut self.state, &mut self.view, bound, ctx);
                return;
            }
            Err(cmd) => cmd,
        };

        // Steel path, composed from shared step functions.
        let meta = cmd.meta();
        let name = cmd.name();

        // BEFORE
        commands::step_paste_commit(&mut self.state, meta.defers_paste_commit);
        let char_arg = self.state.pending_char.take();
        // Snapshot the recipe before the body by cloning, not `mem::take`: an
        // inner `call!` dispatch (e.g. vim-keybind's `C` wrapper calling
        // `copy-selection-on-next-line`) must see and compose onto whatever
        // the user already staged, the same as if that inner command were
        // dispatched directly from the keymap. `step_stamp_repeatable` below
        // still reads this snapshot, not the (possibly further-mutated) live
        // value, so a repeatable outer command's stamped recipe reflects
        // state as of entry, not whatever an inner dispatch built on top of it.
        let pre_recipe = self.state.selection_recipe.clone();
        let pre_writes = self.state.selection_recipe_writes;
        // Before the body; see `commands::repeat_slot_owned`'s own doc.
        let slot_owned = commands::repeat_slot_owned(&self.state);

        // BODY
        if !self.run_steel_command(name.as_ref(), &ctx, char_arg) {
            // Failed command: undo whatever a partial inner dispatch wrote,
            // so the failure has no residual effect on the recipe.
            self.state.selection_recipe = pre_recipe;
            return;
        }

        // AFTER: re-query to get the resolved command's repeatable flag.
        // A Lazy stub becomes SteelBacked after activation; re-query reflects that.
        let repeatable = self
            .state
            .config
            .registry
            .get_mappable(name.as_ref())
            .is_some_and(|c| c.meta().repeatable);
        if repeatable && !slot_owned {
            // Outer-name-wins: stamp the outer command so `.` replays it, not
            // any inner native command the body dispatched via `call!`.
            commands::step_stamp_repeatable(
                &mut self.state,
                name,
                ctx.count.unwrap_or(1),
                char_arg,
                Some(pre_recipe),
            );
        }
        // Clear only when this outer command owns the decision: a repeatable
        // command's recipe was just consumed by the stamp above (matching
        // the native `Edit` variant (always `Untracked`, so `step_update_recipe`
        // clears it too), and a body that dispatched nothing natively never
        // ran `step_update_recipe` to decide for itself, so this non-selection
        // outer command must clear it exactly as any other `Untracked`
        // command would. A body that DID dispatch natively already got the
        // correct decision from that inner call's own `step_update_recipe`,
        // which must not be overridden.
        if repeatable || self.state.selection_recipe_writes == pre_writes {
            self.state.selection_recipe.clear();
        }
        // Outer Steel commands skip with_jump, step_clear_extend, and
        // step_align_view: their meta hardcodes is_jump = clears_extend =
        // aligns_view = false. An inner native (call! …) still fires all three
        // because it routes through `commands::run` with its own meta.
    }

    /// Activate `plugin` (a `Lazy` stub's owner), reporting the standard
    /// warning on failure.
    fn activate_lazy_and_report(
        &mut self,
        plugin: &hume_scripting::attribution::PluginId,
        name: &str,
    ) -> bool {
        if self.activate_lazy_plugin(plugin, name) {
            true
        } else {
            self.report(Severity::Info, format!("unknown command: {name}"));
            false
        }
    }

    /// The registry lost `name`'s entry between resolving it and using it:
    /// activation replaced a `Lazy` stub but left nothing usable behind, or
    /// `name` wasn't the expected kind at all. Always `false`, so a caller
    /// returns it directly.
    fn report_command_lost(&mut self, name: &str) -> bool {
        self.report(
            Severity::Error,
            format!("{name}: internal error: command lost after activation"),
        );
        false
    }

    /// Activates the `:` line's current target command's owning plugin, if
    /// it is still a `TypedBody::Lazy` stub. Runs before
    /// `EditorState::trigger_minibuf_completion`, so
    /// a lazily-declared typed command's `#:complete` completer is visible
    /// on the command's very first use. Without this, `register_lazy_typed_
    /// command` (`host_impl/commands.rs`) hardcodes the stub's `completer`
    /// to `None`, and nothing before dispatch itself would otherwise
    /// activate the plugin to replace it. A no-op when there is no pending
    /// activation (a real command, an unresolvable name, or no `:` line).
    pub(in crate::editor) fn activate_minibuf_completion_target(&mut self) {
        let Some(name) = self.state.minibuf_target_command() else {
            return;
        };
        let Some(plugin) = self.state.config.registry.lazy_owner(&name).cloned() else {
            return;
        };
        self.activate_lazy_and_report(&plugin, &name);
    }

    /// The `:` line's Tab, end to end: [`Self::activate_minibuf_completion_
    /// target`] then `EditorState::trigger_minibuf_completion`, in one call so
    /// the two can't be split at a call site that forgets the first, which
    /// would silently leave a lazily-declared command's `#:complete`
    /// unresolved on its very first use (see `activate_minibuf_completion_
    /// target`'s own doc).
    pub(in crate::editor) fn start_minibuf_completion(&mut self) {
        self.activate_minibuf_completion_target();
        self.state.trigger_minibuf_completion(&self.view);
    }

    /// Run the body of a Steel-backed or Lazy mappable command (bound to a
    /// key, or dispatched via `call!`/dot-repeat).
    ///
    /// Returns `false` if the command aborted (lazy activation failure, scripting
    /// error, or `scripting` is `None`). On error, the caller skips AFTER stages.
    pub(super) fn run_steel_command(
        &mut self,
        name: &str,
        ctx: &CmdCtx,
        char_arg: Option<char>,
    ) -> bool {
        // Injected into the lambda's `count` param verbatim: `0` is the Scheme
        // spelling of `None` ("no count was typed"), so a wrapper that forwards
        // this value straight into `(call! "move-down" pane count extend)` round-trips
        // a bare keypress back to visual-line movement (`parse_count_extend`
        // decodes `0` back to `None` on the way in).
        let count = ctx.count.unwrap_or(0);
        let extend = ctx.extend;

        // Classified by `name` (never a passed-in `MappableCommand`): `name`
        // is the single source of truth for which command this call runs:
        // both callers (`dispatch`, `replay_command`) already derive it from
        // their own `cmd`, so re-deriving the entry from that same `name`
        // rules out a caller ever activating one command's plugin while
        // running another's body. Pure registry metadata, resolved (and its
        // arity/arg-count errors reported) before the `scripting` guard
        // below, so a `:cmd` arity mismatch is reported even in the
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
                Some(MappableCommand::Lazy { plugin, .. }) => {
                    // Activate the owning plugin now so we can read
                    // `inline_output` from the resolved SteelBacked entry before
                    // dispatch.
                    let plugin = plugin.clone();
                    if !self.activate_lazy_and_report(&plugin, name) {
                        return false;
                    }
                    // Re-query: activation replaced the stub with a SteelBacked
                    // entry.
                    match self.state.config.registry.get_mappable(name) {
                        Some(MappableCommand::SteelBacked {
                            inline_output,
                            arity,
                            is_variadic,
                            ..
                        }) => (*inline_output, *arity, *is_variadic),
                        _ => return self.report_command_lost(name),
                    }
                }
                _ => return self.report_command_lost(name),
            };

        // Inject pane, count, and extend as leading lambda args based on
        // declared arity.
        let effective_args = match marshal_leading_args(
            cmd_arity,
            cmd_is_variadic,
            crate::editor::commands::FocusedPane::current(&self.state).handle(&self.view),
            steel::rvals::SteelVal::IntV(count as isize),
            steel::rvals::SteelVal::BoolV(extend),
            "keymap injection supplies at most 3 (pane, count, extend)",
        ) {
            Ok(args) => args,
            Err(msg) => {
                self.report(Severity::Error, format!("{name}: {msg}"));
                return false;
            }
        };

        self.call_steel_command_body(name, char_arg, effective_args, inline_output)
    }

    /// Run the body of a Steel-backed or Lazy *typed* command, dispatched
    /// from the `:` command line via `(define-typed-command! …)`.
    ///
    /// Mirrors [`Self::run_steel_command`] but resolves metadata from
    /// [`super::registry::TypedBody`] rather than `MappableCommand`, and
    /// marshals `(arg force)` rather than `(count extend)`. The two never
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
            _ => return self.report_command_lost(name),
        };

        // Scheme-idiomatic absence: an untyped argument is `#f`, not a
        // sentinel string or a fabricated count; a lambda that only cares
        // whether an arg was given writes a plain `(if arg …)` guard.
        let arg_val = match &arg {
            Some(s) => steel::rvals::SteelVal::StringV(s.clone().into()),
            None => steel::rvals::SteelVal::BoolV(false),
        };
        let effective_args = match marshal_leading_args(
            cmd_arity,
            cmd_is_variadic,
            crate::editor::commands::FocusedPane::current(&self.state).handle(&self.view),
            arg_val,
            steel::rvals::SteelVal::BoolV(force),
            "typed-command injection supplies at most 3 (pane, arg, force)",
        ) {
            Ok(args) => args,
            Err(msg) => {
                self.report(Severity::Error, format!("{name}: {msg}"));
                return false;
            }
        };

        self.call_steel_command_body(name, None, effective_args, inline_output)
    }

    /// Invoke a Steel command lambda by name with pre-marshalled positional
    /// args, bracketing the call for `#:inline-output` commands.
    ///
    /// The Steel invocation, alt-screen bracket, and effect application are
    /// one funnel.
    fn call_steel_command_body(
        &mut self,
        name: &str,
        char_arg: Option<char>,
        effective_args: Vec<steel::rvals::SteelVal>,
        inline_output: bool,
    ) -> bool {
        let Some(scripting) = self.scripting.as_mut() else {
            return false;
        };

        // Alt-screen bracketing for inline-output commands is lazy: entering
        // the alt-screen and printing the running banner happens on the
        // command body's *first actual output* (see
        // `EditorHostImpl::ensure_inline_output_screen`), not eagerly here,
        // so a body that only logs (`log!`) never flashes an empty screen or
        // blocks on a keypress nobody needed to answer. Pushing a frame just
        // primes the state SteelCtx reads through `is_inline_output_command`;
        // the same `push` a nested `call!` uses (`EditorHostImpl::
        // arm_inline_output`), so top-level dispatch and `call!` share one
        // arming implementation. A command not declared `#:inline-output`
        // pushes nothing: `is_inline_output_command` must read closed for it
        // even if its own body later `call!`s into a declared one. Pushed
        // only after the no-scripting-host early return above, since only a
        // host can run the session that drains the frame. Pushing before that
        // return would leak the frame.
        if inline_output {
            self.state
                .inline_output
                .push(name, self.tui.as_active(), self.kitty_enabled);
        }

        let result = {
            let mut impl_host = crate::editor::host_impl::EditorHostImpl::full(
                &mut self.state,
                &mut self.view,
                &mut self.lsp,
                &mut self.timer_wheel,
                &mut self.timer_payloads,
                self.tui.clone(),
                self.kitty_enabled,
            );
            scripting.call_steel_cmd(name, char_arg, effective_args, &mut impl_host)
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
    /// and queue the disk-change sweep an entered/ran subprocess may
    /// warrant. Fits every Steel-session boundary that can arm one: the
    /// top-level dispatch that owns the whole bracket lifecycle
    /// ([`Self::call_steel_command_body`]), and a hook/queued-call batch
    /// that only ever arms one indirectly, through a nested `(call! …)` to
    /// an `#:inline-output` command
    /// (`hume_scripting::host::OutputHost::arm_inline_output`).
    ///
    /// By the time this runs, `InlineOutput`'s frame stack is usually already
    /// drained: every Steel session's own tail
    /// (`hume_scripting::activation::run_steel_session`) does that
    /// unconditionally, including for the top-level dispatch's own frame,
    /// which nothing else ever truncates. Truncated to zero again here
    /// regardless, as the backstop for the one caller
    /// ([`Self::call_steel_command_body`]) that can reach this without ever
    /// running a session at all: `call_steel_cmd`'s own registry/
    /// `command_table` desync check fails before `run_steel_session` starts.
    /// `entered`/`ran` survive either drain (see `InlineOutput`'s own doc);
    /// reading them here, once, is this boundary's whole job.
    pub(super) fn close_inline_output_bracket(&mut self) {
        self.state.inline_output.truncate(0);
        let ran = self.state.inline_output.take_ran();
        if let Some(entered) = self.state.inline_output.take_entered() {
            // `entered.tui` is the same `ActiveTui` `ensure_inline_output_screen`
            // captured on entry. Read here, not `self.tui` again, so this
            // always restores the terminal it actually left. `None` only
            // for the test-only headless shape.
            if let Some(term) = entered.tui.terminal() {
                hume_platform::terminal::print_return_prompt();
                hume_platform::terminal::wait_for_keypress(term);
                let _ = hume_platform::terminal::leave_inline_output(
                    term,
                    entered.kitty,
                    entered.mouse,
                    entered.mouse_select,
                );
            }
            self.state.force_full_redraw = true;
        }
        if ran {
            // The editor regained the terminal: same trigger class
            // as `TerminalEvent::FocusIn`, so it raises the same event rather
            // than sweeping directly; the reaction is `OnFocusGained`'s Rust
            // handler in `Editor::react_to_event`. That reaction runs inside
            // the next `settle()`, after `message_logged_this_input` has
            // already been set from this same dispatch's own message-log
            // delta. `confirm_permit`'s message-shadow clause is scoped to
            // `DiskCheckTrigger::BufferEnter` for exactly this reason, so a
            // warning this command logged itself can't suppress the reload
            // confirm its own subprocess just caused.
            self.state.queue_event(EditorEvent::OnFocusGained);
        }
    }

    /// Reports `Severity::Warning` for a command name that failed to
    /// resolve the way the caller needed. If the registry recognizes `name`
    /// under the *other* kind, names it and explains how it's actually
    /// reachable instead of `fallback`: a split that resolves only one
    /// kind would otherwise leave the other kind unexplained.
    ///
    /// Stays `Warning`, not `Info`, despite most of those being live-typo
    /// cases that would otherwise fit the transient rule: the post-init
    /// keymap lint caller is a config-time diagnostic the user won't see
    /// the moment it fires and needs to find later in `:messages`. The
    /// shared function can't carry two severities, so it keeps the one its
    /// least-ephemeral caller needs.
    pub(in crate::editor) fn report_unknown_command(&mut self, name: &str, fallback: String) {
        let msg = self
            .state
            .config
            .registry
            .other_kind_hint(name)
            .unwrap_or(fallback);
        self.report(Severity::Warning, msg);
    }

    /// Looks up `name` in the command registry, reporting (via
    /// [`Self::report_unknown_command`]) and returning `None` if it isn't
    /// there.
    pub(in crate::editor) fn resolve_mappable(&mut self, name: &str) -> Option<MappableCommand> {
        let cmd = self.state.config.registry.get_mappable(name).cloned();
        if cmd.is_none() {
            self.report_unknown_command(name, format!("unknown command: {name}"));
        }
        cmd
    }
}

/// Shared arity guard + argument marshalling behind
/// [`Editor::run_steel_command`] and [`Editor::run_typed_steel_command`]:
/// both cap a Steel command lambda at three leading injected params:
/// `pane` always first, then `first`/`second`, and marshal 0..=3 of them
/// based on declared arity. The two callers differ only in what `first`/
/// `second` are (`count`/`extend` vs `arg`/`force`) and how the overflow
/// error names them; `injection_desc` supplies that trailing clause
/// verbatim.
fn marshal_leading_args(
    cmd_arity: u16,
    cmd_is_variadic: bool,
    pane: hume_scripting::PaneHandle,
    first: steel::rvals::SteelVal,
    second: steel::rvals::SteelVal,
    injection_desc: &str,
) -> Result<Vec<steel::rvals::SteelVal>, String> {
    if cmd_arity > 3 {
        return Err(format!(
            "lambda declares {cmd_arity} required params; {injection_desc}"
        ));
    }
    Ok(match (cmd_arity, cmd_is_variadic) {
        (0, false) => vec![],
        (1, false) => vec![hume_scripting::SteelPane::new(pane).into_steel_val()],
        (2, false) => vec![hume_scripting::SteelPane::new(pane).into_steel_val(), first],
        _ => vec![
            hume_scripting::SteelPane::new(pane).into_steel_val(),
            first,
            second,
        ],
    })
}

#[cfg(test)]
mod marshal_leading_args_tests {
    use super::marshal_leading_args;
    use hume_engine::pipeline::BufferId;
    use steel::rvals::SteelVal;

    fn pane_stand_in() -> SteelVal {
        hume_scripting::SteelPane::new(hume_scripting::PaneHandle::buffer_only(BufferId::default()))
            .into_steel_val()
    }

    fn probe(cmd_arity: u16, cmd_is_variadic: bool) -> Vec<SteelVal> {
        marshal_leading_args(
            cmd_arity,
            cmd_is_variadic,
            hume_scripting::PaneHandle::buffer_only(BufferId::default()),
            SteelVal::IntV(2), // first stand-in
            SteelVal::IntV(3), // second stand-in
            "test injects at most 3 (pane, first, second)",
        )
        .expect("arity 0..=3 must not error")
    }

    /// Arity 0 stays a no-op injection: a command with no parameters at all
    /// (the common case) is unaffected by pane becoming the leading slot.
    #[test]
    fn arity_zero_gets_nothing() {
        assert_eq!(probe(0, false), Vec::<SteelVal>::new());
    }

    /// Arity 1 gets pane alone, not `first`: pane is always the leading slot.
    ///
    /// Injecting `first` here would silently bind the pane to whatever a
    /// `(lambda (arg) …)`-style command names its single parameter.
    #[test]
    fn arity_one_gets_pane_only() {
        assert_eq!(probe(1, false), vec![pane_stand_in()]);
    }

    /// Arity 2 gets `(pane, first)`.
    #[test]
    fn arity_two_gets_pane_and_first() {
        assert_eq!(probe(2, false), vec![pane_stand_in(), SteelVal::IntV(2)]);
    }

    /// Arity 3 (or variadic) gets all three: `(pane, first, second)`.
    #[test]
    fn arity_three_gets_all_three() {
        assert_eq!(
            probe(3, false),
            vec![pane_stand_in(), SteelVal::IntV(2), SteelVal::IntV(3)]
        );
    }

    #[test]
    fn variadic_gets_all_three_regardless_of_declared_arity() {
        assert_eq!(
            probe(0, true),
            vec![pane_stand_in(), SteelVal::IntV(2), SteelVal::IntV(3)]
        );
    }

    /// Arity 4 overflows the 3-slot cap (pane, first, second).
    #[test]
    fn arity_four_errors() {
        let result = marshal_leading_args(
            4,
            false,
            hume_scripting::PaneHandle::buffer_only(BufferId::default()),
            SteelVal::IntV(2),
            SteelVal::IntV(3),
            "test injects at most 3 (pane, first, second)",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("at most 3"));
    }
}
