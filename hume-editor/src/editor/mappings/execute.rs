use std::borrow::Cow;

use super::super::Editor;
use super::super::dispatch::CmdCtx;

impl Editor {
    /// Resolve a named command and dispatch it through the unified pipeline.
    ///
    /// Delegates to [`Editor::dispatch`] which handles all bookkeeping (paste
    /// session, jump list, dot-repeat) for both native and Steel-backed
    /// commands. `arg` is the character a wait-char binding read, if any.
    pub(in super::super) fn execute_keymap_command(
        &mut self,
        name: Cow<'static, str>,
        count: Option<usize>,
        extend: bool,
        arg: Option<char>,
    ) {
        let Some(reg_cmd) = self.resolve_mappable(name.as_ref()) else {
            return;
        };

        let ctx = CmdCtx { count, extend };
        self.dispatch(reg_cmd, ctx, arg);
    }
}
