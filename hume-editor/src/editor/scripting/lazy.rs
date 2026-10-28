//! Activating a lazily declared plugin when one of its commands, events or
//! languages is first used.

use crate::editor::{Editor, host_impl::EditorHostImpl};
use hume_core::Severity;
use hume_scripting::PluginStatus;

impl Editor {
    // ── Command execution ─────────────────────────────────────────────────────

    /// Shared core: activate `plugin` inline, report its messages, and apply
    /// its queued side effects (or report the error).
    ///
    /// Applying effects here, rather than leaving them for some later drain,
    /// is what lets a lazily-activated plugin's own `register-lsp-server!` (or
    /// `set-buffer-option!`'s `"language"`, grammar sweep, ...) take effect before this call
    /// returns, so the buffer that triggered activation isn't skipped.
    fn activate_and_register(&mut self, plugin: &hume_scripting::attribution::EntryId) {
        let init_budget = self.state.settings.steel_init_budget_ms as u64;
        let result = {
            let Some(host) = self.scripting.as_mut() else {
                return;
            };
            let mut ih = EditorHostImpl::init(
                &mut self.state,
                &mut self.view,
                self.tui.clone(),
                self.kitty_enabled,
            );
            host.activate_plugin_inline(plugin, init_budget, &mut ih)
        };
        self.apply_script_result(result, "");
    }

    /// Activate `plugin`, then emit a `Severity::Trace` message if it
    /// transitioned `Declared → Loaded`. `activation` is the human-readable
    /// activation description for the log line.
    fn activate_and_trace(
        &mut self,
        plugin: &hume_scripting::attribution::EntryId,
        activation: &str,
    ) {
        let was_declared = self
            .scripting
            .as_ref()
            .is_some_and(|h| matches!(h.plugin_status(plugin), Some(PluginStatus::Declared)));
        self.activate_and_register(plugin);
        if was_declared {
            let is_loaded = self
                .scripting
                .as_ref()
                .is_some_and(|h| matches!(h.plugin_status(plugin), Some(PluginStatus::Loaded)));
            if is_loaded {
                self.report(
                    Severity::Trace,
                    format!("plugin '{plugin}' activated ({activation})"),
                );
            }
        }
    }

    /// Activate the plugin owning the lazy stub for command `name` and
    /// register its commands. The end of activation drops the plugin's
    /// remaining stubs, so `name` stays registered only when the body
    /// defined it.
    pub(in crate::editor) fn activate_lazy_plugin(
        &mut self,
        plugin: &hume_scripting::attribution::EntryId,
        name: &str,
    ) {
        self.activate_and_trace(plugin, &format!("by command '{name}'"));
    }

    /// Activate every still-`Declared` lazy plugin registered for the event
    /// named `name`.
    ///
    /// Called by `settle`'s `Event` arm for each queued event, before the
    /// handler check, so a plugin's `register-hook!` handlers are installed
    /// before the hook fires.
    pub(in crate::editor) fn activate_lazy_event_plugins(&mut self, name: &str) {
        let pending = match self.scripting.as_ref() {
            Some(host) => {
                let plugins = host.activation_event_plugins(name);
                if plugins.is_empty() {
                    return;
                }
                plugins
            }
            None => return,
        };
        self.activate_pending_plugins(pending, &format!("by event '{name}'"));
    }

    /// Activate every still-`Declared` lazy plugin registered for language `lang`.
    ///
    /// Called from `set_buffer_language` before `OnLanguageSet` fires so a plugin's
    /// `register-hook!` handlers are installed in time to run on this very transition.
    pub(in crate::editor) fn activate_lazy_language_plugins(&mut self, lang: &str) {
        let pending = match self.scripting.as_ref() {
            Some(host) => {
                let plugins = host.activation_language_plugins(lang);
                if plugins.is_empty() {
                    return;
                }
                plugins
            }
            None => return,
        };
        self.activate_pending_plugins(pending, &format!("by language '{lang}'"));
    }

    fn activate_pending_plugins(
        &mut self,
        pending: Vec<hume_scripting::attribution::EntryId>,
        activation: &str,
    ) {
        for plugin in &pending {
            self.activate_and_trace(plugin, activation);
        }
    }
}
