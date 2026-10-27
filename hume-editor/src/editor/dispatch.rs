//! Unified command dispatch pipeline: the `&mut Editor` half.
//!
//! [`commands::run`] handles the `&mut EditorState + &mut
//! EngineView` half (native commands, and the BEFORE/AFTER pipeline stages
//! shared with Steel-backed commands). This module holds the Steel-backed
//! path, which additionally needs `self.scripting` and the
//! timer bridge: fields only reachable through `&mut Editor`.

use super::error::CommandError;
use super::registry::{Dispatchable, MappableCommand, SteelCommand};
use super::{Editor, commands, replay};

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
        let opened = self.state.open_steel_recipe();
        let change = meta.repeatable.then(|| replay::NewChange {
            command: name.clone(),
            count: ctx.count.unwrap_or(1),
            seed: None,
            recipe: opened.recipe().to_vec(),
        });

        // BODY.
        let ran = replay::record_change(self, change, |ed| {
            let ran = match ed.run_steel_command(&cmd, &ctx) {
                Ok(()) => true,
                Err(e) => {
                    ed.report_error(&e);
                    false
                }
            };
            (ran, ran)
        });

        // AFTER
        self.state.close_steel_recipe(opened, ran, meta.repeatable);
        if !ran {
            return false;
        }
        // Outer Steel commands skip with_jump, step_clear_extend, and
        // step_align_view: their meta hardcodes is_jump = clears_extend =
        // aligns_view = false. An inner native (call! …) still fires all three
        // because it routes through `commands::run` with its own meta.
        true
    }

    /// Activate `plugin` (a `Lazy` stub's owner). Fails as a Steel command
    /// does when there is no scripting host, and as an unknown name does
    /// when activation leaves no command named `name`.
    fn activate_lazy(
        &mut self,
        plugin: &hume_scripting::attribution::EntryId,
        name: &str,
    ) -> Result<(), CommandError> {
        if self.scripting.is_none() {
            return Err(scripting_unavailable(name));
        }
        self.activate_lazy_plugin(plugin, name);
        if self.state.config.registry.contains(name) {
            Ok(())
        } else {
            Err(self.unknown_command(name, unknown_command_text(name)))
        }
    }

    /// Activate a `Lazy` stub's `plugin` and return the Steel command that
    /// replaced it. Fails when activation fails or leaves no such entry.
    fn resolve_lazy(
        &mut self,
        plugin: &hume_scripting::attribution::EntryId,
        name: &str,
    ) -> Result<SteelCommand, CommandError> {
        self.activate_lazy(plugin, name)?;
        match self.state.config.registry.get_mappable(name) {
            Some(MappableCommand::SteelBacked(cmd)) => Ok(cmd.clone()),
            _ => Err(command_lost(name)),
        }
    }

    /// Resolve a named command and dispatch it through the unified pipeline.
    ///
    /// Delegates to [`Editor::dispatch`] which handles all bookkeeping (paste
    /// session, jump list, dot-repeat) for both native and Steel-backed
    /// commands.
    pub(in crate::editor) fn execute_keymap_command(
        &mut self,
        name: std::borrow::Cow<'static, str>,
        count: Option<usize>,
        extend: bool,
    ) {
        let Some(reg_cmd) = self.resolve_mappable(name.as_ref()) else {
            return;
        };

        let ctx = CmdCtx { count, extend };
        self.dispatch(reg_cmd, ctx);
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
        if let Err(e) = self.activate_lazy(&plugin, &name) {
            self.report_error(&e);
        }
    }

    /// A minibuffer line's Tab, end to end: [`Self::activate_minibuf_completion_
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
    /// Fails, unreported, if the command aborted (scripting error, or
    /// `scripting` is `None`). On error, the caller skips AFTER stages.
    fn run_steel_command(&mut self, cmd: &SteelCommand, ctx: &CmdCtx) -> Result<(), CommandError> {
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
        let effective_args = marshal_leading_args(
            cmd.body.arity,
            cmd.body.is_variadic,
            crate::editor::commands::FocusedPane::current(&self.view).handle(&self.view),
            steel::rvals::SteelVal::IntV(count as isize),
            steel::rvals::SteelVal::BoolV(extend),
            "keymap injection supplies at most 3 (pane, count, extend)",
        )
        .map_err(|msg| CommandError::new(format!("{name}: {msg}")))?;

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
    /// resolves it, so any casing names the command. Fails, unreported, on
    /// the same failure classes as `run_steel_command`, and when the lambda
    /// has no parameter for a given argument or `!`.
    pub(super) fn run_typed_steel_command(
        &mut self,
        name: &str,
        lazy_plugin: Option<hume_scripting::attribution::EntryId>,
        arg: Option<String>,
        force: bool,
    ) -> Result<(), CommandError> {
        let name = self
            .state
            .config
            .registry
            .typed_canonical_name(name)
            .map(str::to_owned)
            .ok_or_else(|| command_lost(name))?;
        let name = name.as_str();
        if let Some(plugin) = lazy_plugin {
            self.activate_lazy(&plugin, name)?;
        }

        let body = match self
            .state
            .config
            .registry
            .get_typed(name)
            .map(|tc| &tc.body)
        {
            Some(super::registry::TypedBody::Steel(body)) => body.clone(),
            _ => return Err(command_lost(name)),
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
                return Err(CommandError::new(format!("`:{name}` {reason}")));
            }
        }

        // Scheme-idiomatic absence: an untyped argument is `#f`, not a
        // sentinel string or a fabricated count; a lambda that only cares
        // whether an arg was given writes a plain `(if arg …)` guard.
        let arg_val = match &arg {
            Some(s) => steel::rvals::SteelVal::StringV(s.clone().into()),
            None => steel::rvals::SteelVal::BoolV(false),
        };
        let effective_args = marshal_leading_args(
            body.arity,
            body.is_variadic,
            crate::editor::commands::FocusedPane::current(&self.view).handle(&self.view),
            arg_val,
            steel::rvals::SteelVal::BoolV(force),
            "typed-command injection supplies at most 3 (pane, arg, force)",
        )
        .map_err(|msg| CommandError::new(format!("{name}: {msg}")))?;

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
    /// The session ends through [`Self::finish_script_session`], so a Steel
    /// error comes back unreported, after its log lines and effects.
    fn call_steel_command_body(
        &mut self,
        name: &str,
        proc: hume_scripting::CommandProc,
        effective_args: Vec<steel::rvals::SteelVal>,
        inline_output: bool,
    ) -> Result<(), CommandError> {
        if self.scripting.is_none() {
            return Err(scripting_unavailable(name));
        }

        // Alt-screen bracketing for inline-output commands is lazy: entering
        // the alt-screen and printing the running banner happens on the
        // command body's *first actual output* (see
        // `EditorHostImpl::claim_terminal`), not eagerly here,
        // so a body that only logs (`log!`) never flashes an empty screen or
        // blocks on a keypress nobody needed to answer. Pushing a frame just
        // primes the state `claim_terminal` reads; the same
        // `push` a nested `call!` uses (`EditorHostImpl::arm_inline_output`),
        // so top-level dispatch and `call!` share one arming implementation.
        // A command not declared `#:inline-output` pushes nothing:
        // `claim_terminal` must not grant the terminal to it even if its own
        // body later `call!`s into a declared one. Pushed
        // only after the no-scripting-host early return above, since only a
        // host can run the session that drains the frame. Pushing before that
        // return would leak the frame.
        if inline_output {
            self.state
                .inline_output
                .push(name, self.tui.as_active(), self.kitty_enabled);
        }

        let result = {
            let (scripting, mut impl_host) = self.scripting_with_host().expect("checked above");
            scripting.call_steel_cmd(proc, effective_args, &mut impl_host)
        };
        self.finish_script_session(result)
            .map_err(CommandError::new)
    }

    /// The `Severity::Warning` error for a command name that failed to
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
    pub(in crate::editor) fn unknown_command(&self, name: &str, fallback: String) -> CommandError {
        CommandError::warning(
            self.state
                .config
                .registry
                .other_kind_hint(name)
                .unwrap_or(fallback),
        )
    }

    /// Reports [`Self::unknown_command`]'s error.
    pub(in crate::editor) fn report_unknown_command(&mut self, name: &str, fallback: String) {
        let e = self.unknown_command(name, fallback);
        self.report_error(&e);
    }

    /// Report a key bound to `name`, which names no key-bindable command.
    pub(in crate::editor) fn report_unbound_key_command(&mut self, name: &str) {
        self.report_unknown_command(
            name,
            format!("key bound to unknown command '{name}': typo, or missing from #:commands?"),
        );
    }

    /// Looks up `name` in the command registry, reporting and returning
    /// `None` if it isn't there or its `Lazy` stub fails to activate. A
    /// `Lazy` stub is activated and its real entry returned, so every
    /// caller reads the command's own metadata: whether an Insert-key
    /// binding is repeatable is read before it dispatches.
    pub(in crate::editor) fn resolve_mappable(&mut self, name: &str) -> Option<Dispatchable> {
        match self.state.config.registry.get_mappable(name).cloned() {
            Some(MappableCommand::Native(cmd)) => Some(Dispatchable::Native(cmd)),
            Some(MappableCommand::SteelBacked(cmd)) => Some(Dispatchable::Steel(cmd)),
            Some(MappableCommand::Lazy { name, plugin }) => match self.resolve_lazy(&plugin, &name)
            {
                Ok(cmd) => Some(Dispatchable::Steel(cmd)),
                Err(e) => {
                    self.report_error(&e);
                    None
                }
            },
            None => {
                self.report_unknown_command(name, unknown_command_text(name));
                None
            }
        }
    }
}

/// The registry lost `name`'s entry between resolving it and using it:
/// activation replaced a `Lazy` stub but left nothing usable behind, or
/// `name` wasn't the expected kind at all.
fn command_lost(name: &str) -> CommandError {
    CommandError::new(format!(
        "{name}: internal error: command lost after activation"
    ))
}

/// The registry still holds the Steel command `name`, but there is no
/// scripting host to run it.
fn scripting_unavailable(name: &str) -> CommandError {
    CommandError::new(format!("{name}: scripting is not available"))
}

/// The message for a name that resolves to no command, from `:`, a key
/// binding, or `call!` alike.
pub(in crate::editor) fn unknown_command_text(name: &str) -> String {
    format!("unknown command: {name}")
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
    pane: hume_core::PaneHandle,
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
    use hume_core::BufferId;
    use steel::rvals::SteelVal;

    fn pane_stand_in() -> SteelVal {
        hume_scripting::SteelPane::new(hume_core::PaneHandle::buffer_only(BufferId::default()))
            .into_steel_val()
    }

    fn probe(cmd_arity: u16, cmd_is_variadic: bool) -> Vec<SteelVal> {
        marshal_leading_args(
            cmd_arity,
            cmd_is_variadic,
            hume_core::PaneHandle::buffer_only(BufferId::default()),
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
            hume_core::PaneHandle::buffer_only(BufferId::default()),
            SteelVal::IntV(2),
            SteelVal::IntV(3),
            "test injects at most 3 (pane, first, second)",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("at most 3"));
    }
}
