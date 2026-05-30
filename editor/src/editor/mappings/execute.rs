use std::borrow::Cow;

use super::super::commands::RING_CYCLE_CMDS;
use super::super::keymap::WaitCharPending;
use super::super::registry::MappableCommand;
use super::super::{doc_ops, Editor, RegisterPrefix, RepeatableAction, Severity};
use super::super::jump_list::JumpEntry;
use editing::selection::Selection;
use crate::ops::MotionMode;
use crate::editor::host_impl::EditorHostImpl;
use scripting::QueuedCommand;

impl Editor {
    /// Execute a named command with the given count and extend flag.
    ///
    /// `extend` is converted to `MotionMode::Extend` / `MotionMode::Move` and
    /// passed to the command function. The command itself decides what to do
    /// with the mode — motions and selections branch on it; edits ignore it.
    pub(in super::super) fn execute_keymap_command(
        &mut self,
        name: Cow<'static, str>,
        count: usize,
        extend: bool,
        steel_args: Vec<steel::rvals::SteelVal>,
    ) {
        let Some(reg_cmd) = self.registry.get_mappable(name.as_ref()).cloned() else {
            self.report(Severity::Warning, format!("unknown command: {name}"));
            return;
        };
        {
            // Commit any open paste session before non-cycle dispatch so all
            // `[`/`]` cycles fold into a single undo step. Must happen before
            // the actual dispatch so that `undo` sees a committed revision.
            if !RING_CYCLE_CMDS.contains(&name.as_ref()) {
                self.commit_paste_session();
            }

            // Snapshot pending_char before dispatch — commands consume it via `.take()`.
            let char_arg = self.pending_char;

            // ── Jump list: capture pre-command state ─────────────────────────
            // Motions, explicit jump commands, and vertical visual-line EditorCmds
            // can all produce large enough line jumps to warrant a jump entry.
            let is_explicit_jump = reg_cmd.is_jump();
            let is_vertical_visual = reg_cmd.is_visual_move();
            let pre_jump = if is_explicit_jump
                || is_vertical_visual
                || matches!(reg_cmd, MappableCommand::Motion { .. })
            {
                let bid = self.focused_buffer_id();
                let primary = self.current_selections().primary();
                Some((primary, self.doc().text().char_to_line(primary.head()), bid))
            } else {
                None
            };

            let motion_mode = if extend {
                MotionMode::Extend
            } else {
                MotionMode::Move
            };

            let focused = self.focused_pane_id;
            let buf = self.focused_buffer_id();
            match reg_cmd {
                MappableCommand::Motion { fun, .. } => {
                    // Motion functions take (buf, sels, count, mode). count defaults to 1
                    // if the user typed no prefix.
                    doc_ops::apply_doc_motion(
                        &self.buffers, &mut self.pane_state, focused, buf,
                        |b, s| fun(b, s, count, motion_mode),
                    );
                }
                MappableCommand::Selection { fun, .. } => {
                    // Selection / text-object functions don't take count.
                    doc_ops::apply_doc_motion(
                        &self.buffers, &mut self.pane_state, focused, buf,
                        |b, s| fun(b, s, motion_mode),
                    );
                }
                MappableCommand::Edit { fun, .. } => {
                    doc_ops::apply_doc_edit(
                        &mut self.buffers, &mut self.pane_state, focused, buf,
                        fun,
                    );
                }
                MappableCommand::EditorCmd { fun, .. } => {
                    if let Err(e) = fun(self, count, motion_mode) {
                        self.report(Severity::Error, e.message().to_owned());
                    }
                }
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
                    let focused_pane_id = self.focused_pane_id;
                    let focused_buffer_id = self.focused_buffer_id();

                    if inline_output {
                        let kitty = self.kitty_enabled;
                        let mouse = self.settings.mouse_enabled;
                        if let Err(e) = platform::terminal::enter_inline_output(kitty, mouse) {
                            self.report(Severity::Error, format!("inline-output enter failed: {e}"));
                            return;
                        }
                        platform::terminal::print_running_banner(name);
                    }

                    // `scripting` is disjoint from the other fields borrowed
                    // here; Rust NLL splitting allows this simultaneous borrow.
                    let result = {
                        let host_scr = self.scripting.as_mut().expect("checked above");
                        let mut impl_host = EditorHostImpl {
                            settings: &mut self.settings,
                            keymap: &mut self.keymap,
                            focused_pane_id,
                            focused_buffer_id,
                            buffers: Some(&mut self.buffers),
                            engine_view: Some(&mut self.engine_view),
                            pane_state: Some(&mut self.pane_state),
                            pane_jumps: Some(&mut self.pane_jumps),
                            languages: Some(&mut self.languages),
                        };
                        host_scr.call_steel_cmd(steel_proc, char_arg, steel_args, &mut impl_host)
                    };

                    // Re-enter the alt-screen unconditionally — on both success and error.
                    if inline_output {
                        platform::terminal::print_return_prompt();
                        platform::terminal::wait_for_keypress();
                        let kitty = self.kitty_enabled;
                        let mouse = self.settings.mouse_enabled;
                        let mouse_select = self.settings.mouse_select;
                        let _ = platform::terminal::leave_inline_output(kitty, mouse, mouse_select);
                        self.force_full_redraw = true;
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
                        self.wait_char = Some(WaitCharPending {
                            cmd_name: wc.into(),
                            ctrl_extend: false,
                        });
                    }
                }
            }

            // ── Jump list: record if this was a jump ─────────────────────────
            if let Some((pre_primary, pre_line, pre_bid)) = pre_jump {
                let post_line = self
                    .doc()
                    .text()
                    .char_to_line(self.current_selections().primary().head());
                if is_explicit_jump
                    || pre_line.abs_diff(post_line) > self.settings.jump_line_threshold
                {
                    self.pane_jumps[self.focused_pane_id].push(JumpEntry::from_pre_motion(
                        pre_primary,
                        pre_line,
                        pre_bid,
                    ));
                }
            }

            // Record repeatable actions for `.` replay.
            // Skips non-repeatable commands (motions, selections, undo, etc.).
            // During replay `cmd_repeat` restores `last_repeatable_action` after the fact,
            // so any transient overwrite here is harmless.
            if reg_cmd.is_repeatable() {
                self.last_repeatable_action = Some(RepeatableAction {
                    command: name.clone(),
                    count,
                    char_arg,
                    insert_keys: Vec::new(),
                });
            }

            // Update last_command AFTER dispatch so do_paste reads the *previous*
            // command, not the paste command itself. Updated during macro replay
            // too — Smart-p must work inside macros (e.g. `xdp` in a macro
            // should paste the deleted line, not the clipboard). The post-replay
            // reset to "macro-replay" in drain_replay_queue handles the
            // after-macro case.
            self.last_command = Some(name);
        }
    }

    /// Dispatch every command in `queue`, re-arming the register prefix before
    /// each entry that was captured with one.
    ///
    /// Entries whose `register` is `None` leave `self.register_prefix` untouched
    /// (a user-typed `"5` prefix flows into the first non-prefixed command, matching
    /// interactive behavior).  When at least one entry carried a register, the prefix
    /// is cleared after the queue finishes so it does not bleed into the next
    /// user keystroke.
    pub(in super::super) fn drain_command_queue(
        &mut self,
        queue: Vec<QueuedCommand>,
        count: usize,
        extend: bool,
    ) {
        let mut armed_any = false;
        for qc in queue {
            if let Some(r) = qc.register {
                self.register_prefix = Some(RegisterPrefix::Selected(r));
                armed_any = true;
            }
            self.execute_keymap_command(qc.name.into(), count, extend, qc.args);
        }
        if armed_any {
            self.register_prefix = None;
        }
    }

    // ── Selection helpers ─────────────────────────────────────────────────────

    /// Replace the primary selection and merge any resulting overlaps.
    ///
    /// If the new selection overlaps an existing secondary, both are merged
    /// into one — so the total selection count may decrease.
    pub(in super::super) fn set_primary_selection(&mut self, new_sel: Selection) {
        let pid = self.focused_pane_id;
        let bid = self.focused_buffer_id();
        let idx = self.pane_state[pid][bid].selections.primary_index();
        // mem::take avoids a clone: move out, compute, write back.
        let sels = std::mem::take(&mut self.pane_state[pid][bid].selections);
        self.pane_state[pid][bid].selections = sels.replace(idx, new_sel).merge_overlapping();
    }
}
