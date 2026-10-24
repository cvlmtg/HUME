//! `EditorHostImpl`'s terminal-safety state around `#:inline-output`
//! commands.

use crate::editor::inline_output::Entered;
use crate::editor::registry::{MappableCommand, SteelCommand};
use crate::editor::tui::ActiveTui;

use super::EditorHostImpl;
use hume_scripting::host::OutputHost;

impl<'a> OutputHost for EditorHostImpl<'a> {
    fn is_inline_output_command(&self) -> bool {
        self.state.inline_output.is_open()
    }

    fn ensure_inline_output_screen(&mut self) -> Result<(), String> {
        // Reads the frame's own captured `tui`/`kitty`, not `self.tui`/
        // `self.kitty_enabled`: this must work correctly even on a host
        // with no inline-output authority (`EditorHostImpl::new`) completing
        // a bracket a *different*, real host armed; see `ActiveTui`'s own
        // doc.
        let Some((name, tui, kitty)) = self.state.inline_output.needs_enter() else {
            return Ok(());
        };
        let entered = match tui {
            ActiveTui::On(term) => {
                let screen = hume_platform::terminal::enter_inline_output(
                    term,
                    kitty,
                    self.state.settings.mouse,
                    self.state.settings.mouse_select,
                )
                .map_err(|e| format!("inline-output enter failed: {e}"))?;
                hume_platform::terminal::print_running_banner(name);
                Entered::Screen(screen)
            }
            #[cfg(test)]
            ActiveTui::Headless => Entered::Headless { kitty },
        };
        self.state.inline_output.mark_entered(entered);
        Ok(())
    }

    fn arm_inline_output(&mut self, name: &str) -> Option<usize> {
        // No inline-output authority at all (`EditorHostImpl::new`), checked
        // first so a `declared` match never pushes a frame this host has no
        // standing to decide the captured device for. Computed as
        // `ActiveTui` up front (rather than keeping a `&Tui` borrow of
        // `self.tui` alive) so the `self.state`/`self.kitty_enabled` reads
        // below aren't fighting it for `self`.
        let active_tui = self.tui.as_ref()?.as_active();
        // Mappable only: `%dispatch-command!` (`call!`'s expansion) reaches
        // mappable procs/`get_mappable`, never a typed proc: a typed command
        // is not `call!`-reachable, so matching one here would arm for a path
        // that can't actually happen.
        let declared = matches!(
            self.state.config.registry.get_mappable(name),
            Some(MappableCommand::SteelBacked(SteelCommand {
                body: hume_scripting::SteelCmdBody {
                    inline_output: true,
                    ..
                },
                ..
            }))
        );
        if !declared {
            return None;
        }
        Some(
            self.state
                .inline_output
                .push(name, active_tui, self.kitty_enabled),
        )
    }

    fn truncate_inline_output(&mut self, depth: usize) {
        self.state.inline_output.truncate(depth);
    }
}
