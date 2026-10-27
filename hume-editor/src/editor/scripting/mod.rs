//! The editor's side of the Steel scripting host: starting it and
//! evaluating config (`init`), applying what each eval queued (`effects`),
//! and activating lazily declared plugins (`lazy`).

use std::path::PathBuf;

use super::{Editor, host_impl::EditorHostImpl};
use hume_core::Severity;

mod effects;
mod init;
mod lazy;

pub(in crate::editor) use init::theme_search_paths;

/// The `init.scm` a session should evaluate, or why there is none:
/// `Editor::config_path`'s result. Carries the reason rather than collapsing
/// it to `Option<PathBuf>` so callers (`init_scripting`, `typed_reload_config`)
/// don't each have to re-derive it from `config_source` separately.
pub(in crate::editor) enum ConfigPath {
    /// The file to evaluate. `required` marks a `--config` override: the user
    /// asserted the file exists, so a missing one is an error (checked at
    /// `:reload-config` time in `init_scripting`), where a missing default
    /// `<config_dir>/init.scm` is normal and silently skipped.
    Resolved { path: PathBuf, required: bool },
    /// `--no-config` (`ConfigSource::Skip`): the user asked for no `init.scm`.
    Skipped,
    /// Default source with no resolvable config directory
    /// (`HOME`/`XDG_CONFIG_HOME`, `APPDATA` on Windows all unset).
    NoConfigDir,
}

impl Editor {
    /// The Steel host with an [`EditorHostImpl`] over the rest of the editor,
    /// built together because the two borrow disjoint fields of `self`; `None`
    /// when scripting is not initialized.
    pub(in crate::editor) fn scripting_with_host(
        &mut self,
    ) -> Option<(&mut hume_scripting::ScriptingHost, EditorHostImpl<'_>)> {
        let scripting = self.scripting.as_mut()?;
        let host = EditorHostImpl::full(
            &mut self.state,
            &mut self.view,
            &mut self.timers,
            self.tui.clone(),
            self.kitty_enabled,
        );
        Some((scripting, host))
    }

    /// Drain any pending `(log! …)` messages from the scripting host and
    /// report each one. No-op if scripting isn't initialized.
    pub(in crate::editor) fn flush_script_messages(&mut self) {
        let msgs = self
            .scripting
            .as_mut()
            .map(hume_scripting::ScriptingHost::take_pending_messages)
            .unwrap_or_default();
        self.report_script_messages(msgs);
    }

    /// Report each drained `(log! …)` message at its own severity.
    fn report_script_messages(&mut self, msgs: Vec<(Severity, String)>) {
        for (severity, text) in msgs {
            self.report(severity, text);
        }
    }
}
