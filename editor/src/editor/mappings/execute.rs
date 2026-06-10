use std::borrow::Cow;

use super::super::commands::{self, RING_CYCLE_CMDS};
use super::super::keymap::WaitCharPending;
use super::super::registry::MappableCommand;
use super::super::{Editor, RegisterPrefix, Severity};
use editing::selection::Selection;
use crate::editor::host_impl::EditorHostImpl;
use scripting::QueuedCommand;

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
    /// `dispatch_native` / `drain_command_queue` overrides it with the inner name.
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

            // Commit any open paste session before Steel eval; same invariant as
            // the native path (ring-cycle commands bypass this).
            if !RING_CYCLE_CMDS.contains(&name.as_ref()) {
                self.state.commit_paste_session();
            }

            // Pre-stamp last_command with the outer name so that a SteelBacked
            // command that dispatches no inner native leaves a fresh name rather
            // than a stale one from the previous command. If any inner native runs
            // via dispatch_native / drain_command_queue it overrides this with the
            // inner command's name — preserving smart-p for Steel-wrapped kill cmds.
            self.state.last_command = Some(name.clone());

            match reg_cmd {
                MappableCommand::Lazy { ref plugin, .. } => {
                    let plugin = plugin.clone();
                    if self.activate_lazy_plugin(&plugin, name.as_ref()) {
                        self.execute_keymap_command(name, count, extend, steel_args);
                    } else {
                        self.report(Severity::Warning, format!("unknown command: {name}"));
                    }
                    return;
                }
                MappableCommand::SteelBacked { ref steel_proc, ref name, inline_output, .. } => {
                    if self.scripting.is_none() {
                        return;
                    }
                    let focused_pane_id = self.state.focused_pane_id;
                    let focused_buffer_id = self.focused_buffer_id();

                    if inline_output {
                        let kitty = self.kitty_enabled;
                        let mouse = self.state.settings.mouse_enabled;
                        if let Err(e) = platform::terminal::enter_inline_output(kitty, mouse) {
                            self.report(Severity::Error, format!("inline-output enter failed: {e}"));
                            return;
                        }
                        platform::terminal::print_running_banner(name);
                    }

                    // `scripting` is disjoint from the other fields borrowed
                    // here; Rust NLL splitting allows this simultaneous borrow.
                    // `registry` is shared-borrowed alongside the exclusive
                    // borrows of other fields — different fields, NLL allows it.
                    let result = {
                        let host_scr = self.scripting.as_mut().expect("checked above");
                        let mut impl_host = EditorHostImpl {
                            state: &mut self.state,
                            view: &mut self.view,
                        };
                        host_scr.call_steel_cmd(steel_proc, char_arg, steel_args, focused_pane_id, focused_buffer_id, &mut impl_host)
                    };

                    // Re-enter the alt-screen unconditionally — on both success and error.
                    if inline_output {
                        platform::terminal::print_return_prompt();
                        platform::terminal::wait_for_keypress();
                        let kitty = self.kitty_enabled;
                        let mouse = self.state.settings.mouse_enabled;
                        let mouse_select = self.state.settings.mouse_select;
                        let _ = platform::terminal::leave_inline_output(kitty, mouse, mouse_select);
                        self.state.force_full_redraw = true;
                    }

                    let (queue, wait_char_cmd, lang_sets, grammar_sweeps) = match result {
                        Ok(r) => (r.cmd_queue, r.wait_char_request, r.pending_language_sets, r.grammar_sweeps),
                        Err(e) => {
                            self.report(Severity::Error, e);
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
                    self.drain_command_queue(queue, count, extend);
                    if let Some(wc) = wait_char_cmd {
                        self.state.wait_char = Some(WaitCharPending {
                            cmd_name: wc.into(),
                            ctrl_extend: false,
                        });
                    }
                }
                _ => unreachable!("non-native variants exhausted above"),
            }

        }

        // Drain any hooks queued during command execution (mode changes, etc.).
        // Called after BOTH paths so hooks always fire after the full command.
        self.drain_hooks();
    }

    /// Dispatch every command in `queue`, re-arming the register prefix before
    /// each entry that was captured with one.
    ///
    /// Entries whose `register` is `None` leave `self.state.register_prefix` untouched
    /// (a user-typed `"5` prefix flows into the first non-prefixed command, matching
    /// interactive behavior).  When at least one entry carried a register, the prefix
    /// is cleared after the queue finishes so it does not bleed into the next
    /// user keystroke.
    ///
    /// Native entries (`Motion`/`Selection`/`Edit`/`EditorCmd`) are dispatched
    /// directly through `commands::dispatch_native` using the count/extend stored
    /// in their own `args` (parsed via `scripting::parse_count_extend`). This
    /// ensures a deferred native command `(call! "move-down" 3)` moves 3 lines
    /// regardless of the outer `count` passed to this function.  Non-native entries
    /// continue to use the outer `count`/`extend` as before.
    pub(in super::super) fn drain_command_queue(
        &mut self,
        queue: Vec<QueuedCommand>,
        count: usize,
        extend: bool,
    ) {
        let mut armed_any = false;
        for qc in queue {
            if let Some(r) = qc.register {
                self.state.register_prefix = Some(RegisterPrefix::Selected(r));
                armed_any = true;
            }
            // Classify: native commands run through dispatch_native (bookkeeping
            // included); non-native (SteelBacked/Lazy) or unknown route through
            // execute_keymap_command which will warn on unknown names.
            match self.state.registry.get_mappable(&qc.name).cloned() {
                Some(cmd) if cmd.is_native() => {
                    match scripting::parse_count_extend(&qc.args) {
                        Ok((n, ext)) => commands::dispatch_native(
                            &mut self.state, &mut self.view,
                            cmd, Cow::Owned(qc.name), n, ext,
                        ),
                        Err(e) => self.report(
                            Severity::Warning, format!("{}: {e}", qc.name),
                        ),
                    }
                }
                _ => self.execute_keymap_command(
                    Cow::Owned(qc.name), count, extend, qc.args,
                ),
            }
        }
        if armed_any {
            self.state.register_prefix = None;
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
