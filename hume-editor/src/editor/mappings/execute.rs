use std::borrow::Cow;

use super::super::commands::{self, RING_CYCLE_CMDS};
use super::super::keymap::WaitCharPending;
use super::super::registry::MappableCommand;
use super::super::{Editor, Severity};
use hume_editing::selection::Selection;
use crate::editor::host_impl::EditorHostImpl;
use steel::rvals::SteelVal;

impl Editor {
    /// Execute a named command with the given count and extend flag.
    ///
    /// `extend` is converted to `MotionMode::Extend` / `MotionMode::Move` and
    /// passed to the command function. The command itself decides what to do
    /// with the mode — motions and selections branch on it; edits ignore it.
    ///
    /// Native commands (`Motion`/`Selection`/`Edit`/`EditorCmd`) delegate to
    /// `commands::dispatch_native`, which is the single source of truth for all
    /// native-dispatch bookkeeping (paste session, jump list, dot-repeat,
    /// last_command).  Non-native commands (SteelBacked, Lazy) pre-stamp
    /// `last_command` to the outer command name; any inner native dispatch via
    /// `dispatch_native` overrides it with the inner name.
    pub(in super::super) fn execute_keymap_command(
        &mut self,
        name: Cow<'static, str>,
        count: usize,
        extend: bool,
        steel_args: Vec<steel::rvals::SteelVal>,
    ) {
        let Some(reg_cmd) = self.state.registry.get_mappable(name.as_ref()).cloned() else {
            self.report(Severity::Warning, format!("unknown command: {name}"));
            return;
        };

        if reg_cmd.is_native() {
            // All bookkeeping (paste session, jump list, dot-repeat, last_command)
            // lives in dispatch_native — no duplication with the Steel sync path.
            commands::dispatch_native(&mut self.state, &mut self.view, reg_cmd, name, count, extend);
        } else {
            // Snapshot pending_char before the Steel eval — commands consume it
            // via `.take()`, and `call_steel_cmd` passes it as `pending_char`.
            let char_arg = self.state.pending_char;

            // Snapshot the selection recipe built by the user's prior native
            // selection commands before the Steel body runs. Inner (call! …)
            // dispatches may overwrite selection_recipe; the snapshot captures
            // the pre-body extent for dot-repeat.
            //
            // Correctness: drain_pending_repeat replays the recipe AND
            // re-dispatches the whole Steel body, so any selection the body
            // builds internally via (call! …) is rebuilt automatically during
            // replay — including it in the recorded recipe would double-apply it.
            let recipe_snapshot = std::mem::take(&mut self.state.selection_recipe);

            // Commit any open paste session before Steel eval; same invariant as
            // the native path (ring-cycle commands bypass this).
            if !RING_CYCLE_CMDS.contains(&name.as_ref()) {
                self.state.commit_paste_session();
            }

            // Pre-stamp last_command with the outer name so that a SteelBacked
            // command that dispatches no inner native leaves a fresh name rather
            // than a stale one from the previous command. If any inner native runs
            // via dispatch_native it overrides this with the inner command's name —
            // preserving smart-p for Steel-wrapped kill cmds.
            self.state.last_command = Some(name.clone());

            // For a Lazy stub, activate the owning plugin now so we can read
            // `inline_output` from the resolved SteelBacked entry before dispatch.
            // `activate_lazy_plugin` unregisters the stub if activation fails or
            // the body never defined the command, so we always know the outcome.
            if let MappableCommand::Lazy { plugin, .. } = &reg_cmd {
                let plugin = plugin.clone();
                if !self.activate_lazy_plugin(&plugin, name.as_ref()) {
                    self.report(Severity::Warning, format!("unknown command: {name}"));
                    return;
                }
            }

            if self.scripting.is_none() {
                return;
            }

            // Re-query: a Lazy stub is now SteelBacked after activation above;
            // a SteelBacked entry is unchanged.  One extra HashMap get is
            // negligible next to a Scheme eval.
            let (inline_output, cmd_arity, cmd_is_variadic) =
                match self.state.registry.get_mappable(name.as_ref()) {
                    Some(MappableCommand::SteelBacked { inline_output, arity, is_variadic, .. }) => {
                        (*inline_output, *arity, *is_variadic)
                    }
                    _ => {
                        // Should not happen: activate_lazy_plugin's contract guarantees the
                        // stub is replaced by SteelBacked on success, and we returned early
                        // on failure. Degrade gracefully rather than panic with unsaved buffers.
                        self.report(Severity::Error, format!("{name}: internal error — command lost after activation"));
                        return;
                    }
                };
            let focused_pane_id = self.state.focused_pane_id;
            let focused_buffer_id = self.focused_buffer_id();

            // Inject count and extend as leading lambda args based on declared arity
            // (only when the keymap triggers with no explicit args; the `:command` path
            // passes its own steel_args):
            //   arity 0, non-variadic → []
            //   arity 1, non-variadic → [count]
            //   arity ≥ 2 or variadic → [count, extend]
            if steel_args.is_empty() && cmd_arity > 2 {
                self.report(
                    Severity::Error,
                    format!("{name}: lambda declares {cmd_arity} required params; \
                             keymap injection supplies at most 2 (count, extend)"),
                );
                return;
            }
            let effective_args = if steel_args.is_empty() {
                match (cmd_arity, cmd_is_variadic) {
                    (0, false) => vec![],
                    (1, false) => vec![SteelVal::IntV(count as isize)],
                    _ => vec![SteelVal::IntV(count as isize), SteelVal::BoolV(extend)],
                }
            } else {
                steel_args
            };

            // Alt-screen bracketing is intentionally at this site only — the top-level
            // keymap dispatch arm.  A nested `(call! "name")` routes through
            // `%dispatch-command` → `(apply proc args)` inline inside the running Steel
            // eval and never returns here, so a nested inline-output command runs its
            // body without the alt-screen swap, banner, or return prompt.  Defined
            // limitation, not an error.
            if inline_output {
                let kitty = self.kitty_enabled;
                let mouse = self.state.settings.mouse_enabled;
                if let Err(e) = hume_platform::terminal::enter_inline_output(kitty, mouse) {
                    self.report(Severity::Error, format!("inline-output enter failed: {e}"));
                    return;
                }
                hume_platform::terminal::print_running_banner(&name);
            }

            // `scripting` is disjoint from the other fields borrowed here;
            // Rust NLL splitting allows this simultaneous borrow.
            let result = {
                let host_scr = self.scripting.as_mut().expect("checked above");
                let mut impl_host = EditorHostImpl {
                    state: &mut self.state,
                    view: &mut self.view,
                };
                host_scr.call_steel_cmd(name.as_ref(), char_arg, effective_args, focused_pane_id, focused_buffer_id, &mut impl_host)
            };

            // Re-enter the alt-screen unconditionally — on both success and error.
            if inline_output {
                hume_platform::terminal::print_return_prompt();
                hume_platform::terminal::wait_for_keypress();
                let kitty = self.kitty_enabled;
                let mouse = self.state.settings.mouse_enabled;
                let mouse_select = self.state.settings.mouse_select;
                let _ = hume_platform::terminal::leave_inline_output(kitty, mouse, mouse_select);
                self.state.force_full_redraw = true;
            }

            let (wait_char_cmd, lang_sets, grammar_sweeps) = match result {
                Ok(r) => (r.wait_char_request, r.pending_language_sets, r.grammar_sweeps),
                Err(e) => {
                    self.report(Severity::Error, e);
                    self.state.selection_recipe.clear();
                    return;
                }
            };
            self.flush_script_messages();
            for (bid, lang) in lang_sets {
                self.set_buffer_language(bid, lang);
            }
            if !grammar_sweeps.is_empty() {
                self.sweep_buffers_for_grammars(grammar_sweeps);
            }
            if let Some(wc) = wait_char_cmd {
                self.state.wait_char = Some(WaitCharPending {
                    cmd_name: wc.into(),
                    ctrl_extend: false,
                });
            }

        // Dot-repeat: record opt-in Steel commands on the success path.
        //
        // Re-query the registry — a Lazy stub resolved to SteelBacked above,
        // so the entry now reflects the real command.
        //
        // Outer-name-wins: if an inner (call! …) dispatched a native repeatable
        // command and set last_repeatable_action, we overwrite it here so the
        // outer Steel command wins the repeat slot.
        if self.state.registry.get_mappable(name.as_ref()).is_some_and(|c| c.is_repeatable()) {
            self.state.last_repeatable_action = Some(super::super::RepeatableAction {
                command: name.clone(),
                count,
                char_arg,
                insert_keys: Vec::new(),
                selection_recipe: recipe_snapshot,
            });
        }
        // Clear any recipe entries written by inner dispatches so they don't
        // leak into the next command.
        self.state.selection_recipe.clear();
        }

    }

    // ── Selection helpers ─────────────────────────────────────────────────────

    /// Replace the primary selection and merge any resulting overlaps.
    ///
    /// If the new selection overlaps an existing secondary, both are merged
    /// into one — so the total selection count may decrease.
    pub(in super::super) fn set_primary_selection(&mut self, new_sel: Selection) {
        commands::set_primary_selection(&mut self.state, &self.view, new_sel);
    }
}
