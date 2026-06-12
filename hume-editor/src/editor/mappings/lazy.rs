use super::super::{Editor, Severity, scripting_setup::make_init_host};
use super::super::registry::MappableCommand;
use hume_scripting::PluginStatus;

impl Editor {
    // ── Command execution ─────────────────────────────────────────────────────

    /// Shared core: activate `plugin`, register returned commands (or report the
    /// error), leaving messages unflushed.  Called by both the command-stub path
    /// and the event-trigger path so neither duplicates the activate→register→
    /// report triple.
    pub(super) fn activate_and_register(&mut self, plugin: &hume_scripting::attribution::PluginId) {
        let startup_base = self
            .scripting
            .as_ref()
            .map_or(0, |h| h.pending_startup_commands_len());
        let init_budget = self.state.settings.steel_init_budget_ms as u64;
        let result = {
            let Some(host) = self.scripting.as_mut() else { return };
            let mut ih = make_init_host(&mut self.state, &mut self.view);
            host.activate_plugin_inline(plugin, init_budget, &mut ih, &self.builtin_cmd_names)
        };
        match result {
            Ok(cmds) => self.register_steel_cmds(cmds),
            Err(e) => {
                self.report(Severity::Error, e);
                return;
            }
        }
        let queued = match self.scripting.as_mut() {
            Some(h) if h.pending_startup_commands_len() > startup_base => {
                h.split_off_startup_commands(startup_base)
            }
            _ => return,
        };
        self.drain_command_queue(queued, 1, false);
    }

    /// Activate `plugin`, emit a `Severity::Trace` message if it transitioned
    /// `Declared → Loaded`, and leave messages unflushed.  `trigger` is the
    /// human-readable trigger description for the log line.
    pub(super) fn activate_and_trace(
        &mut self,
        plugin: &hume_scripting::attribution::PluginId,
        trigger: &str,
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
                    format!("plugin '{plugin}' loaded ({trigger})"),
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
    pub(super) fn activate_lazy_plugin(
        &mut self,
        plugin: &hume_scripting::attribution::PluginId,
        name: &str,
    ) -> bool {
        if self.scripting.is_none() {
            return false;
        }
        self.activate_and_trace(plugin, &format!("command trigger '{name}'"));
        self.flush_script_messages();
        // Loop guard: if name is still Lazy (body never defined it) or gone,
        // remove the stub and signal failure so the caller does not re-enter.
        let unresolved = matches!(
            self.state.registry.get_mappable(name),
            Some(MappableCommand::Lazy { .. }) | None
        );
        if unresolved {
            self.state.registry.unregister(name);
            false
        } else {
            true
        }
    }

    /// Activate every still-`Declared` lazy plugin registered for `hook_id`.
    ///
    /// Called by `drain_hooks` for each queued hook, before the handler check,
    /// so a plugin's `register-hook!` handlers are installed before the hook fires.
    pub(in super::super) fn activate_lazy_event_plugins(
        &mut self,
        hook_id: hume_scripting::hooks::HookId,
    ) {
        let pending = match self.scripting.as_ref() {
            Some(host) => {
                let plugins = host.event_trigger_plugins(hook_id);
                if plugins.is_empty() { return; }
                plugins
            }
            None => return,
        };
        self.activate_pending_plugins(pending, &format!("event trigger '{}'", hook_id.symbol()));
    }

    /// Activate every still-`Declared` lazy plugin registered for language `lang`.
    ///
    /// Called from `set_buffer_language` before `OnLanguageSet` fires so a plugin's
    /// `register-hook!` handlers are installed in time to run on this very transition.
    pub(in super::super) fn activate_lazy_language_plugins(&mut self, lang: &str) {
        let pending = match self.scripting.as_ref() {
            Some(host) => {
                let plugins = host.language_trigger_plugins(lang);
                if plugins.is_empty() { return; }
                plugins
            }
            None => return,
        };
        self.activate_pending_plugins(pending, &format!("language trigger '{lang}'"));
    }

    fn activate_pending_plugins(
        &mut self,
        pending: Vec<hume_scripting::attribution::PluginId>,
        trigger: &str,
    ) {
        for plugin in &pending {
            self.activate_and_trace(plugin, trigger);
        }
        self.flush_script_messages();
    }
}
