//! Unified command dispatch pipeline: the `&mut Editor` half.
//!
//! [`commands::run`] handles the `&mut EditorState + &mut
//! EngineView` half (native commands, and the BEFORE/AFTER pipeline stages
//! shared with Steel-backed commands). This module holds the Steel-backed
//! path, which additionally needs `self.scripting` and the
//! timer bridge: fields only reachable through `&mut Editor`.

use super::event::EditorEvent;
use super::registry::{Dispatchable, MappableCommand, SteelCommand};
use super::{Editor, Severity, commands, replay};

// ── Command dispatch context ──────────────────────────────────────────────────

/// Per-dispatch context assembled by the key handler and passed through
/// [`Editor::dispatch`].
#[derive(Debug, Clone)]
pub(in crate::editor) struct CmdCtx {
    /// Numeric count prefix. `None` means "no count was typed": a bare
    /// keyboard press, which visual-move commands read as one visual line
    /// (through the [`Count`](super::registry::Count) `run_body` builds from
    /// this). Producible
    /// by the keymap trie leaves, and also by Steel: a script
    /// passes a count of `0` (`parse_native_args` decodes it to `None`) to ask
    /// for the same "as if no count was typed" behavior. `Some(n)` is every
    /// other case: an explicit user count, a script's explicit `n`, or a
    /// non-keybind origin's default (insert-mode leaf, no-arg `call!`).
    pub count: Option<usize>,
    /// Whether this command runs in Extend mode.
    pub extend: bool,
}

impl Editor {
    // ── Unified command dispatch pipeline ──────────────────────────────────────

    /// Execute a resolved command through the unified dispatch pipeline.
    ///
    /// Native commands delegate to [`commands::run`].  Steel-backed
    /// commands run the pipeline's BEFORE/AFTER stages inline, with the body
    /// executed via [`Editor::run_steel_command`] (which needs `&mut Editor` for
    /// `self.scripting`).
    ///
    /// A native command dispatched without the operand it takes reads it
    /// from the user first: the next key's character, or its prompt. During
    /// `.` that read takes the recorded answer instead.
    ///
    /// Returns `false` when the command refused or failed, or its operand
    /// read was refused; `true` otherwise, including while it waits on its
    /// operand. During `.` the recorded operand is applied once this
    /// returns, by `Editor::finish_replayed_answers`, which reports whether
    /// the command then ran.
    pub(in crate::editor) fn dispatch(&mut self, cmd: Dispatchable, ctx: CmdCtx) -> bool {
        let cmd = match cmd {
            Dispatchable::Native(cmd) => {
                // A keypress always acts at the focused pane.
                let fp = commands::FocusedPane::current(&self.view);
                return match commands::BoundCommand::focused(cmd, fp, None) {
                    Ok(bound) => commands::run(&mut self.state, &mut self.view, bound, ctx),
                    Err(commands::FocusedBindError::NeedsOperand(missing)) => {
                        let pane = fp.handle(&self.view);
                        commands::collect_operand(
                            &mut self.state,
                            &mut self.view,
                            missing,
                            pane,
                            ctx,
                        )
                    }
                    Err(commands::FocusedBindError::Arg { name, error }) => {
                        commands::report_arg_error(&mut self.state, &name, &error);
                        false
                    }
                };
            }
            Dispatchable::Steel(cmd) => cmd,
        };

        // Steel path, composed from shared step functions.
        let meta = cmd.meta();
        let name = &cmd.name;
        // BEFORE
        commands::step_paste_commit(&mut self.state, meta.defers_paste_commit);
        // Snapshot the recipe before the body by cloning, not `mem::take`: an
        // inner `call!` dispatch (e.g. vim-keybind's `C` wrapper calling
        // `copy-selection-on-next-line`) must see and compose onto whatever the
        // user already staged. The settled recording is filled from this
        // snapshot, not the (possibly further-mutated) live value.
        let pre_recipe = self.state.selection_recipe.clone();
        let change = meta.repeatable.then(|| replay::NewChange {
            command: name.clone(),
            count: ctx.count.unwrap_or(1),
            seed: None,
            recipe: pre_recipe.clone(),
        });
        // Reset the recipe-touched flag so the AFTER clear decision reflects
        // only what this body dispatched natively; restore it afterward for
        // any outer context.
        let pre_touched = std::mem::replace(&mut self.state.selection_recipe_touched, false);

        // BODY.
        let ran = replay::record_change(self, change, |ed| {
            let ran = ed.run_steel_command(&cmd, &ctx);
            (ran, ran)
        });

        let touched = std::mem::replace(&mut self.state.selection_recipe_touched, pre_touched);

        if !ran {
            // Failed command: undo whatever a partial inner dispatch wrote to
            // the recipe.
            self.state.selection_recipe = pre_recipe;
            return false;
        }

        // AFTER
        // Clear the live recipe when this outer command owns the decision: a
        // repeatable command's recipe was just consumed, and a body that
        // dispatched nothing natively is a non-selection command that clears
        // it like any `Untracked` one. A body that DID dispatch natively
        // already got the correct decision from that inner call, which must
        // not be overridden.
        if meta.repeatable || !touched {
            self.state.selection_recipe.clear();
        }
        // Outer Steel commands skip with_jump, step_clear_extend, and
        // step_align_view: their meta hardcodes is_jump = clears_extend =
        // aligns_view = false. An inner native (call! …) still fires all three
        // because it routes through `commands::run` with its own meta.
        true
    }

    /// Activate `plugin` (a `Lazy` stub's owner), reporting the standard
    /// warning on failure.
    fn activate_lazy_and_report(
        &mut self,
        plugin: &hume_scripting::attribution::EntryId,
        name: &str,
    ) -> bool {
        if self.activate_lazy_plugin(plugin, name) {
            true
        } else {
            self.report(Severity::Info, format!("unknown command: {name}"));
            false
        }
    }

    /// Activate a `Lazy` stub's `plugin` and return the Steel command that
    /// replaced it. `None`, reported, when activation fails or leaves no
    /// such entry.
    fn resolve_lazy(
        &mut self,
        plugin: &hume_scripting::attribution::EntryId,
        name: &str,
    ) -> Option<SteelCommand> {
        if !self.activate_lazy_and_report(plugin, name) {
            return None;
        }
        match self.state.config.registry.get_mappable(name) {
            Some(MappableCommand::SteelBacked(cmd)) => Some(cmd.clone()),
            _ => {
                self.report_command_lost(name);
                None
            }
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
        let Some(target) = self.state.minibuf_target_command() else {
            return;
        };
        let registry = &self.state.config.registry;
        let Some(name) = registry.typed_canonical_name(&target).map(str::to_owned) else {
            return;
        };
        let Some(plugin) = registry.lazy_owner(&name).cloned() else {
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

    /// Run the body of the Steel command `cmd`.
    ///
    /// Returns `false` if the command aborted (scripting error, or
    /// `scripting` is `None`). On error, the caller skips AFTER stages.
    fn run_steel_command(&mut self, cmd: &SteelCommand, ctx: &CmdCtx) -> bool {
        // Injected into the lambda's `count` param verbatim: `0` is the Scheme
        // spelling of `None` ("no count was typed"), so a wrapper that forwards
        // this value straight into `(call! "move-down" pane count extend)` round-trips
        // a bare keypress back to visual-line movement (`parse_native_args`
        // decodes `0` back to `None` on the way in).
        let count = ctx.count.unwrap_or(0);
        let extend = ctx.extend;

        let name = cmd.name.as_ref();

        // Inject pane, count, and extend as leading lambda args based on
        // declared arity.
        let effective_args = match marshal_leading_args(
            cmd.body.arity,
            cmd.body.is_variadic,
            crate::editor::commands::FocusedPane::current(&self.view).handle(&self.view),
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

        self.call_steel_command_body(
            name,
            hume_scripting::CommandProc::Mappable(cmd.body.proc.clone()),
            effective_args,
            cmd.body.inline_output,
        )
    }

    /// Run the body of a Steel-backed or Lazy *typed* command, dispatched
    /// from the `:` command line via `(define-typed-command! …)`.
    ///
    /// Mirrors [`Self::run_steel_command`] but resolves metadata from
    /// [`super::registry::TypedBody`] rather than `MappableCommand`, and
    /// marshals `(arg force)` rather than `(count extend)`. The two never
    /// share a lookup because the registry keeps the kinds strictly separate
    /// (see `registry/mod.rs`'s module doc). `name` is resolved as `:`
    /// resolves it, so any casing names the command. Returns `false` on the
    /// same failure classes as `run_steel_command`.
    pub(super) fn run_typed_steel_command(
        &mut self,
        name: &str,
        lazy_plugin: Option<hume_scripting::attribution::EntryId>,
        arg: Option<String>,
        force: bool,
    ) -> bool {
        let Some(name) = self
            .state
            .config
            .registry
            .typed_canonical_name(name)
            .map(str::to_owned)
        else {
            return self.report_command_lost(name);
        };
        let name = name.as_str();
        if let Some(plugin) = lazy_plugin
            && !self.activate_lazy_and_report(&plugin, name)
        {
            return false;
        }

        let body = match self
            .state
            .config
            .registry
            .get_typed(name)
            .map(|tc| &tc.body)
        {
            Some(super::registry::TypedBody::Steel(body)) => body.clone(),
            _ => return self.report_command_lost(name),
        };

        // A fixed parameter list is the lambda's whole input, so an argument
        // or `!` it has no parameter for is the user's mistake, not noise to
        // drop. A variadic lambda takes everything.
        if !body.is_variadic {
            let refused = if arg.is_some() && body.arity < 2 {
                Some("takes no argument")
            } else if force && body.arity < 3 {
                Some("takes no `!`")
            } else {
                None
            };
            if let Some(reason) = refused {
                self.report(Severity::Error, format!("`:{name}` {reason}"));
                return false;
            }
        }

        // Scheme-idiomatic absence: an untyped argument is `#f`, not a
        // sentinel string or a fabricated count; a lambda that only cares
        // whether an arg was given writes a plain `(if arg …)` guard.
        let arg_val = match &arg {
            Some(s) => steel::rvals::SteelVal::StringV(s.clone().into()),
            None => steel::rvals::SteelVal::BoolV(false),
        };
        let effective_args = match marshal_leading_args(
            body.arity,
            body.is_variadic,
            crate::editor::commands::FocusedPane::current(&self.view).handle(&self.view),
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

        self.call_steel_command_body(
            name,
            hume_scripting::CommandProc::Typed(body.proc),
            effective_args,
            body.inline_output,
        )
    }

    /// Invoke the Steel command `name`'s `proc` with pre-marshalled positional
    /// args, bracketing the call for `#:inline-output` commands.
    ///
    /// The Steel invocation, alt-screen bracket, and effect application are
    /// one funnel.
    fn call_steel_command_body(
        &mut self,
        name: &str,
        proc: hume_scripting::CommandProc,
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
                &mut self.timer_wheel,
                &mut self.timer_payloads,
                self.tui.clone(),
                self.kitty_enabled,
            );
            scripting.call_steel_cmd(proc, effective_args, &mut impl_host)
        };

        // Close the bracket only if a builtin actually opened it. This runs
        // before `match result` below so a Steel error raised after screen
        // entry still gets the TUI restored first.
        self.close_inline_output_bracket();

        let effects = match result {
            Ok(effects) => effects,
            Err(e) => {
                self.apply_script_effects(e.effects);
                self.report(Severity::Error, e.message);
                return false;
            }
        };

        self.flush_script_messages();
        self.apply_script_effects(effects);
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
    /// By the time this runs, `InlineOutput`'s frame stack is already
    /// drained: every Steel session's own tail
    /// (`hume_scripting::activation::run_steel_session`) does that whenever
    /// its host has an output capability, as the editor's host always does,
    /// including for the top-level dispatch's own frame, which nothing else
    /// ever truncates. Truncated to zero again here regardless: this
    /// boundary owns the guarantee that no frame outlives the dispatch, and
    /// the session drain is only what satisfies it today.
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
    /// there. A `Lazy` stub is activated and its real entry returned, so
    /// every caller reads the command's own metadata: whether an Insert-key
    /// binding is repeatable is read before it dispatches.
    pub(in crate::editor) fn resolve_mappable(&mut self, name: &str) -> Option<Dispatchable> {
        match self.state.config.registry.get_mappable(name).cloned() {
            Some(MappableCommand::Native(cmd)) => Some(Dispatchable::Native(cmd)),
            Some(MappableCommand::SteelBacked(cmd)) => Some(Dispatchable::Steel(cmd)),
            Some(MappableCommand::Lazy { name, plugin }) => {
                self.resolve_lazy(&plugin, &name).map(Dispatchable::Steel)
            }
            None => {
                self.report_unknown_command(name, format!("unknown command: {name}"));
                None
            }
        }
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
