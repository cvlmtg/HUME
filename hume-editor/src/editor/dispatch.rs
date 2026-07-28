//! Unified command dispatch pipeline: the `&mut Editor` half.
//!
//! [`commands::run_dispatch_pipeline`] handles the `&mut EditorState + &mut
//! EngineView` half (native commands, and the BEFORE/AFTER pipeline stages
//! shared with Steel-backed commands). This module holds the Steel-backed
//! path, which additionally needs `self.scripting`, `self.lsp`, and the
//! timer bridge — fields only reachable through `&mut Editor`.

use super::registry::MappableCommand;
use super::{Editor, InlineOutputDispatch, Severity, commands, timer_bridge};

// ── Command dispatch context ──────────────────────────────────────────────────

/// Where a Steel-backed dispatch's positional lambda args come from.
///
/// The two origins marshal differently (see [`Editor::run_steel_command`]):
/// keymap dispatch injects `count`/`extend` by the target lambda's declared
/// arity; the `:` command line injects its typed argument (or a fixed `1`
/// when none was typed) and rejects any arity it can't satisfy with a single
/// value. A plain `Option<String>` on [`CmdCtx`] couldn't distinguish "not a
/// `:` dispatch" from "`:` dispatch with no typed arg" — the latter still
/// needs the minibuf marshalling rules (e.g. one arg for a variadic command,
/// not the two a keymap dispatch would inject), so the origin itself must be
/// explicit.
#[derive(Debug, Clone)]
pub(crate) enum ArgSource {
    /// Keymap trie leaf or dot-repeat replay.
    Keymap,
    /// The `:` command line, carrying the (possibly absent) typed argument —
    /// already `%`/`#`-expanded by the caller.
    Minibuf(Option<String>),
}

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
    /// Where this dispatch's Steel lambda arguments come from. `Keymap` for
    /// native commands too (unused there — only Steel-backed dispatch reads it).
    pub arg_source: ArgSource,
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
            .config
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

        // Marshal Steel lambda args — the rules differ by dispatch origin.
        let effective_args = match &ctx.arg_source {
            ArgSource::Keymap => {
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
                match (cmd_arity, cmd_is_variadic) {
                    (0, false) => vec![],
                    (1, false) => vec![steel::rvals::SteelVal::IntV(count as isize)],
                    _ => vec![
                        steel::rvals::SteelVal::IntV(count as isize),
                        steel::rvals::SteelVal::BoolV(extend),
                    ],
                }
            }
            ArgSource::Minibuf(arg) => {
                // Any mappable command can be invoked from the command line with
                // an implicit count of 1. This means `:clear-search`, `:undo`, etc.
                // all work without needing typed-command wrappers.
                if cmd_arity == 0 && !cmd_is_variadic {
                    vec![]
                } else if cmd_arity == 1 || cmd_is_variadic {
                    match arg {
                        // No arg typed: default count=1 for count-type lambdas;
                        // string-type lambdas reject IntV(1) via their own
                        // (string? x) guard.
                        Some(s) => vec![steel::rvals::SteelVal::StringV(s.clone().into())],
                        None => vec![steel::rvals::SteelVal::IntV(1)],
                    }
                } else {
                    self.report(
                        Severity::Error,
                        format!(
                            "{name}: requires {cmd_arity} args; the minibuffer can only supply 1"
                        ),
                    );
                    return false;
                }
            }
        };

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
        let result = {
            let mut impl_host = crate::editor::host_impl::EditorHostImpl {
                state: &mut self.state,
                view: &mut self.view,
                lsp: Some(&mut self.lsp),
                timers: Some(timer_bridge::TimerHandle {
                    wheel: &mut self.timer_wheel,
                    payloads: &mut self.timer_payloads,
                }),
                terminal: self.terminal.as_ref(),
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

        // Close the bracket only if a builtin actually opened it. This runs
        // before `match result` below so a Steel error raised after screen
        // entry still gets the TUI restored first. Scoped to this dispatch
        // either way: reset unconditionally so stale state can't outlive it
        // and leak into a later command's `SteelCtx`.
        //
        // `armed_or_entered` covers both shapes an inline-output command can
        // take: `Entered` (it produced output, so the alt-screen bracket
        // needs closing below) and `Armed` (it declared `#:inline-output` and
        // ran, but produced none — a formatter with no stdout, say — so
        // there's no bracket to close). Either way the subprocess ran with
        // the real terminal and may well have rewritten one of our open
        // files, so both are disk-change check trigger points; only
        // `Headless`/`Inactive` (no interactive user to answer a confirm)
        // are excluded.
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
            self.check_all_disk_state();
        }

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
}
