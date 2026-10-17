use super::super::{Editor, Severity, host_impl::EditorHostImpl};
use hume_scripting::PluginStatus;

impl Editor {
    // ── Command execution ─────────────────────────────────────────────────────

    /// Shared core: activate `plugin` inline, apply its queued side effects (or
    /// report the error), leaving messages unflushed.
    ///
    /// Applying effects here, rather than leaving them for some later drain,
    /// is what lets a lazily-activated plugin's own `register-lsp-server!` (or
    /// `set-buffer-option!`'s `"language"`, grammar sweep, ...) take effect before this call
    /// returns, so the buffer that triggered activation isn't skipped.
    pub(in crate::editor::mappings::lazy) fn activate_and_register(
        &mut self,
        plugin: &hume_scripting::attribution::EntryId,
    ) {
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
            host.activate_plugin_inline(plugin, init_budget, &mut ih, &self.builtin_cmd_names)
        };
        self.apply_script_result(result, "");
    }

    /// Activate `plugin`, emit a `Severity::Trace` message if it transitioned
    /// `Declared → Loaded`, and leave messages unflushed.  `activation` is the
    /// human-readable activation description for the log line.
    pub(in crate::editor::mappings::lazy) fn activate_and_trace(
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

    /// Activate the plugin owning a lazy stub, register its commands, and
    /// check whether `name` (in any case) is now registered.
    ///
    /// Returns `true` when the plugin body defined `name` and dispatch may
    /// proceed. Returns `false` when activation failed or the body never
    /// defined `name`: either way, the end of activation dropped the
    /// plugin's remaining stubs, so `name` is absent from the registry and
    /// the caller reports "unknown command".
    pub(in crate::editor) fn activate_lazy_plugin(
        &mut self,
        plugin: &hume_scripting::attribution::EntryId,
        name: &str,
    ) -> bool {
        if self.scripting.is_none() {
            return false;
        }
        self.activate_and_trace(plugin, &format!("by command '{name}'"));
        self.flush_script_messages();
        self.state.config.registry.contains(name)
    }

    /// Activate every still-`Declared` lazy plugin registered for the event
    /// named `name`.
    ///
    /// Called by `settle`'s `Event` arm for each queued event, before the
    /// handler check, so a plugin's `register-hook!` handlers are installed
    /// before the hook fires.
    pub(in super::super) fn activate_lazy_event_plugins(&mut self, name: &str) {
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
    pub(in super::super) fn activate_lazy_language_plugins(&mut self, lang: &str) {
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
        self.flush_script_messages();
    }
}
