//! Unified command dispatch pipeline: the `&mut Editor` half.
//!
//! [`commands::run_dispatch_pipeline`] handles the `&mut EditorState + &mut
//! EngineView` half (native commands, and the BEFORE/AFTER pipeline stages
//! shared with Steel-backed commands). This module holds the Steel-backed
//! path, which additionally needs `self.scripting`, `self.lsp`, and the
//! timer bridge — fields only reachable through `&mut Editor`.

use super::registry::MappableCommand;
use super::{Editor, Severity, commands, timer_bridge};

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
    /// non-keybind origin's default (`:cmd`, insert-mode leaf, no-arg `call!`).
    pub count: Option<usize>,
    /// Whether this command runs in Extend mode.
    pub extend: bool,
    /// Pre-computed Steel lambda arguments (supplied by keymap trie leaf).
    /// Empty for native commands and keymap-navigated Steel commands.
    pub steel_args: Vec<steel::rvals::SteelVal>,
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
        // Pre-stamp last_command — inner dispatches via `call!` override it.
        commands::step_stamp_last_command(&mut self.state, name.clone(), meta.stamps_last_command);
        let char_arg = self.state.pending_char.take();
        // Always snapshot the recipe before the body — inner dispatches via `call!`
        // overwrite selection_recipe during the body, so the snapshot must be taken
        // before they run (the native path uses step_snapshot_recipe, which gates on
        // repeatable; here we snapshot unconditionally and decide after the body).
        let pre_recipe = std::mem::take(&mut self.state.selection_recipe);

        // BODY — consumes `cmd`.
        if !self.run_steel_command(cmd, name.as_ref(), &ctx, char_arg) {
            self.state.selection_recipe.clear();
            return;
        }

        // AFTER — re-query to get the resolved command's repeatable flag.
        // A Lazy stub becomes SteelBacked after activation; re-query reflects that.
        if self
            .state
            .registry
            .get_mappable(name.as_ref())
            .is_some_and(|c| c.meta().repeatable)
        {
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
        // Non-repeatable outer: leave inner dispatch's repeatable action intact.
        self.state.selection_recipe.clear();
        // Outer Steel commands skip step_record_jump and step_clear_extend: their meta
        // hardcodes is_jump = clears_extend = false. An inner native (call! …) still
        // fires both — it routes through run_dispatch_pipeline with its own meta.
    }

    /// Run the body of a Steel-backed or Lazy command.
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
            if !self.activate_lazy_plugin(&plugin, name) {
                self.report(Severity::Warning, format!("unknown command: {name}"));
                return false;
            }
        }

        let focused_pane_id = self.state.focused_pane_id;
        let focused_buffer_id = self.focused_buffer_id();

        let scripting = match self.scripting.as_mut() {
            Some(s) => s,
            None => return false,
        };

        // Re-query: a Lazy stub is now SteelBacked after activation above;
        // a SteelBacked entry is unchanged.
        let (inline_output, cmd_arity, cmd_is_variadic) =
            match self.state.registry.get_mappable(name) {
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
        let steel_args = &ctx.steel_args;
        if steel_args.is_empty() && cmd_arity > 2 {
            self.report(
                Severity::Error,
                format!(
                    "{name}: lambda declares {cmd_arity} required params; \
                     keymap injection supplies at most 2 (count, extend)"
                ),
            );
            return false;
        }
        let effective_args = if steel_args.is_empty() {
            match (cmd_arity, cmd_is_variadic) {
                (0, false) => vec![],
                (1, false) => vec![steel::rvals::SteelVal::IntV(count as isize)],
                _ => vec![
                    steel::rvals::SteelVal::IntV(count as isize),
                    steel::rvals::SteelVal::BoolV(extend),
                ],
            }
        } else {
            steel_args.clone()
        };

        // Alt-screen bracketing for inline-output commands. Only meaningful
        // when `Editor::run` owns the terminal — off the event loop (tests,
        // headless `run_keys`) there is no alt-screen to leave and no
        // interactive user to answer the "press any key" prompt, so skip the
        // whole bracket and just run the command body below.
        let bracket_inline_output = inline_output && self.tui_active;
        if bracket_inline_output {
            #[cfg(test)]
            {
                self.inline_output_entered = true;
            }
            let kitty = self.kitty_enabled;
            let mouse = self.state.settings.mouse_enabled;
            if let Err(e) = hume_platform::terminal::enter_inline_output(kitty, mouse) {
                self.report(Severity::Error, format!("inline-output enter failed: {e}"));
                return false;
            }
            hume_platform::terminal::print_running_banner(name);
        }

        // Declared flag (not `bracket_inline_output`) — SteelCtx must see it
        // even off the event loop (tests, headless `run_keys`), where no
        // alt-screen bracket runs but the print is harmless either way.
        self.state.dispatch_inline_output = inline_output;

        let result = {
            let mut impl_host = crate::editor::host_impl::EditorHostImpl {
                state: &mut self.state,
                view: &mut self.view,
                lsp: Some(&self.lsp),
                timers: Some(timer_bridge::TimerHandle {
                    wheel: &mut self.timer_wheel,
                    payloads: &mut self.timer_payloads,
                }),
            };
            scripting.call_steel_cmd(
                name,
                char_arg,
                effective_args,
                focused_pane_id,
                focused_buffer_id,
                &mut impl_host,
            )
        };

        // Scope the flag to the command body: reset it so a stale `true` can't
        // outlive this dispatch and leak into a later command's `SteelCtx`.
        self.state.dispatch_inline_output = false;

        if bracket_inline_output {
            hume_platform::terminal::print_return_prompt();
            hume_platform::terminal::wait_for_keypress();
            let kitty = self.kitty_enabled;
            let mouse = self.state.settings.mouse_enabled;
            let mouse_select = self.state.settings.mouse_select;
            let _ = hume_platform::terminal::leave_inline_output(kitty, mouse, mouse_select);
            self.state.force_full_redraw = true;
        }

        let (wait_char_cmd, effects) = match result {
            Ok(r) => (r.wait_char_request, r.effects),
            Err(e) => {
                self.report(Severity::Error, e);
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
}
