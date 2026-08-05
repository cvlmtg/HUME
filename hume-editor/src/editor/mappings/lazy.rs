use super::super::registry::MappableCommand;
use super::super::{Editor, Severity, scripting_setup::make_init_host};
use hume_scripting::PluginStatus;

impl Editor {
    // ── Command execution ─────────────────────────────────────────────────────

    /// Shared core: activate `plugin` inline, apply its queued side effects (or
    /// report the error), leaving messages unflushed.  Called by both the
    /// command-stub path and the event-/language-activation path.
    ///
    /// Applying effects here — rather than leaving them for some later drain —
    /// is what lets a lazily-activated plugin's own `register-lsp-server!` (or
    /// `set-buffer-language!`, grammar sweep, ...) take effect before this call
    /// returns, so the buffer that triggered activation isn't skipped.
    pub(super) fn activate_and_register(&mut self, plugin: &hume_scripting::attribution::PluginId) {
        let init_budget = self.state.settings.steel_init_budget_ms as u64;
        let result = {
            let Some(host) = self.scripting.as_mut() else {
                return;
            };
            let mut ih = make_init_host(&mut self.state, &mut self.view);
            host.activate_plugin_inline(plugin, init_budget, &mut ih, &self.builtin_cmd_names)
        };
        self.apply_script_result(result, "");
    }

    /// Activate `plugin`, emit a `Severity::Trace` message if it transitioned
    /// `Declared → Loaded`, and leave messages unflushed.  `activation` is the
    /// human-readable activation description for the log line.
    pub(super) fn activate_and_trace(
        &mut self,
        plugin: &hume_scripting::attribution::PluginId,
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
    /// check whether `name` is now a real (non-Lazy) command.
    ///
    /// Returns `true` when `name` resolved to a real command after activation
    /// and dispatch may proceed.  Returns `false` when activation failed or
    /// the plugin body never defined `name`; in that case the stub is removed
    /// (preventing an infinite retry loop) and the caller should report an
    /// "unknown command" warning.
    pub(crate) fn activate_lazy_plugin(
        &mut self,
        plugin: &hume_scripting::attribution::PluginId,
        name: &str,
    ) -> bool {
        if self.scripting.is_none() {
            return false;
        }
        self.activate_and_trace(plugin, &format!("by command '{name}'"));
        self.flush_script_messages();
        // Loop guard: if name is still Lazy (body never defined it) or gone,
        // remove the stub and signal failure so the caller does not re-enter.
        let unresolved = matches!(
            self.state.config.registry.get_mappable(name),
            Some(MappableCommand::Lazy { .. }) | None
        );
        if unresolved {
            self.state.config.registry.unregister(name);
            false
        } else {
            true
        }
    }

    /// Activate every still-`Declared` lazy plugin registered for the event
    /// named `name`.
    ///
    /// Called by `drain_events` for each queued hook, before the handler check,
    /// so a plugin's `register-hook!` handlers are installed before the hook fires.
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
        pending: Vec<hume_scripting::attribution::PluginId>,
        activation: &str,
    ) {
        for plugin in &pending {
            self.activate_and_trace(plugin, activation);
        }
        self.flush_script_messages();
    }
}
