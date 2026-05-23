use super::super::{Editor, Severity};
use super::super::registry::MappableCommand;
use crate::scripting::lazy::PluginState;

impl Editor {
    // ── Command execution ─────────────────────────────────────────────────────

    /// Shared core: activate `plugin`, register returned commands (or report the
    /// error), leaving messages unflushed.  Called by both the command-stub path
    /// and the event-trigger path so neither duplicates the activate→register→
    /// report triple.
    pub(super) fn activate_and_register(&mut self, plugin: &crate::scripting::attribution::PluginId) {
        let budget = self.settings.steel_init_budget_ms as u64;
        let result = {
            let Some(host) = self.scripting.as_mut() else { return };
            host.activate_plugin(
                plugin,
                &mut self.settings,
                &mut self.keymap,
                &self.builtin_cmd_names,
                budget,
            )
        };
        match result {
            Ok(cmds) => self.register_steel_cmds(cmds),
            Err(e) => self.report(Severity::Error, e),
        }
    }

    /// Activate `plugin`, emit a `Severity::Trace` message if it transitioned
    /// `Declared → Loaded`, and leave messages unflushed.  `trigger` is the
    /// human-readable trigger description for the log line.
    pub(super) fn activate_and_trace(
        &mut self,
        plugin: &crate::scripting::attribution::PluginId,
        trigger: &str,
    ) {
        let was_declared = self
            .scripting
            .as_ref()
            .is_some_and(|h| {
                matches!(h.lazy_registry.plugins.get(plugin), Some(PluginState::Declared { .. }))
            });
        self.activate_and_register(plugin);
        if was_declared {
            let is_loaded = self
                .scripting
                .as_ref()
                .is_some_and(|h| {
                    matches!(h.lazy_registry.plugins.get(plugin), Some(PluginState::Loaded))
                });
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
        plugin: &crate::scripting::attribution::PluginId,
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
            self.registry.get_mappable(name),
            Some(MappableCommand::Lazy { .. }) | None
        );
        if unresolved {
            self.registry.unregister(name);
            false
        } else {
            true
        }
    }

    /// Activate every still-`Declared` lazy plugin registered for `hook_id`.
    ///
    /// Called at the top of `fire_hook_silent` before the early-exit so that a
    /// plugin's `register-hook!` handlers are installed before the hook fires.
    /// No registry stub or loop-guard needed: `activate_plugin`'s `PluginState`
    /// machine and the `event_triggers` drop on load/fail make repeated fires
    /// idempotent without additional tracking here.
    pub(in super::super) fn activate_lazy_event_plugins(
        &mut self,
        hook_id: crate::scripting::hooks::HookId,
    ) {
        let pending: Vec<crate::scripting::attribution::PluginId> =
            match self.scripting.as_ref() {
                Some(host) => match host.lazy_registry.event_triggers.get(&hook_id) {
                    Some(plugins) if !plugins.is_empty() => plugins.clone(),
                    _ => return,
                },
                None => return,
            };
        for plugin in &pending {
            self.activate_and_trace(plugin, &format!("event trigger '{}'", hook_id.symbol()));
        }
        self.flush_script_messages();
    }

    /// Activate every still-`Declared` lazy plugin registered for language `lang`.
    ///
    /// Called from `set_buffer_language` before `OnLanguageSet` fires so a plugin's
    /// `register-hook!` handlers (incl. `on-language-set`) are installed in time to
    /// run on this very transition.  No stub/loop-guard needed: the `PluginState`
    /// machine + `language_triggers` drop on load/fail make repeated sets idempotent.
    pub(in super::super) fn activate_lazy_language_plugins(&mut self, lang: &str) {
        let pending: Vec<crate::scripting::attribution::PluginId> =
            match self.scripting.as_ref() {
                Some(host) => match host.lazy_registry.language_triggers.get(lang) {
                    Some(plugins) if !plugins.is_empty() => plugins.clone(),
                    _ => return,
                },
                None => return,
            };
        for plugin in &pending {
            self.activate_and_trace(plugin, &format!("language trigger '{lang}'"));
        }
        self.flush_script_messages();
    }
}
