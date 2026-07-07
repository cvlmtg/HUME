use std::borrow::Cow;

use super::super::commands;
use super::super::{CmdCtx, Editor, Severity};
use hume_editing::selection::Selection;

impl Editor {
    /// Resolve a named command and dispatch it through the unified pipeline.
    ///
    /// Delegates to [`Editor::dispatch`] which handles all bookkeeping (paste
    /// session, jump list, dot-repeat, last_command) for both native and
    /// Steel-backed commands.
    pub(in super::super) fn execute_keymap_command(
        &mut self,
        name: Cow<'static, str>,
        count: Option<usize>,
        extend: bool,
        steel_args: Vec<steel::rvals::SteelVal>,
    ) {
        let Some(reg_cmd) = self.state.registry.get_mappable(name.as_ref()).cloned() else {
            self.report(Severity::Warning, format!("unknown command: {name}"));
            return;
        };

        let ctx = CmdCtx {
            count,
            extend,
            steel_args,
        };
        self.dispatch(reg_cmd, ctx);
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
