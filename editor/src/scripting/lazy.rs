//! Lazy plugin loading data model.
//!
//! [`LazyRegistry`] is held on [`super::ScriptingHost`] and borrowed into
//! [`super::SteelCtx`] during every eval.  It tracks each plugin's lifecycle
//! state and the three trigger maps consulted by dispatch, event firing, and
//! language-set to activate lazy plugins on demand.

use std::collections::HashMap;
use std::path::PathBuf;

use super::attribution::PluginId;
use super::hooks::HookId;

// ── PluginState ───────────────────────────────────────────────────────────────

/// Lifecycle state of a declared plugin.
#[derive(Debug)]
pub(crate) enum PluginState {
    /// Declared and located on disk, waiting for activation.
    Declared { path: PathBuf },
    /// Body is currently being evaluated.  Prevents re-entrant activation
    /// (trigger cycle A→B→A sees `Loading` and skips without looping).
    Loading,
    /// Body evaluated and commands registered successfully.
    Loaded,
    /// Body failed to evaluate; no retry until `:reload-config`.
    Failed,
}

// ── LazyRegistry ──────────────────────────────────────────────────────────────

/// Persistent plugin state and trigger maps.
///
/// Borrowed into [`super::SteelCtx`] for the duration of each eval so that
/// `%declare-plugin!` can write directly without `mem::take`/put-back.
///
/// Keys are **not** stored here; they use the ordinary keymap as trie leaves
/// that point to command names.  The command name appears in
/// `command_triggers`, so dispatch finds the lazy stub without any
/// key-specific machinery.
#[derive(Debug, Default)]
pub(crate) struct LazyRegistry {
    /// Per-plugin lifecycle state.  Only plugins whose path was resolved at
    /// declaration time appear here; absent-path plugins are silently skipped.
    pub(crate) plugins: HashMap<PluginId, PluginState>,
    /// 1:1 map: command name → owning plugin.
    ///
    /// Duplicate command name → first claimant wins; the collision is logged
    /// as a non-fatal error visible in `:messages`.
    pub(crate) command_triggers: HashMap<String, PluginId>,
    /// 1:many map: hook event → plugins that load on that event.
    pub(crate) event_triggers: HashMap<HookId, Vec<PluginId>>,
    /// 1:many map: language name → plugins that load when the language is set.
    pub(crate) language_triggers: HashMap<String, Vec<PluginId>>,
}

impl LazyRegistry {
    /// Record a plugin from a `%declare-plugin!` call (always lazy).
    ///
    /// - Duplicate `id` (case-insensitive) → no-op (first declaration wins).
    /// - `path = None` → plugin absent on disk; skipped silently; triggers NOT
    ///   recorded (an absent plugin can never activate, so dangling trigger
    ///   entries would be dead weight until `:reload-config`).
    /// - All plugins are inserted as `Declared`; they activate when a trigger
    ///   fires or when `(load-plugin name)` is called explicitly.
    pub(crate) fn declare(
        &mut self,
        id: PluginId,
        path: Option<PathBuf>,
        on_command: Vec<String>,
        on_event: Vec<HookId>,
        on_language: Vec<String>,
    ) {
        if self.plugins.contains_key(&id) {
            return; // already declared — duplicate declare-plugin call, ignore
        }
        let Some(path) = path else {
            return; // absent on disk — silently skip, no triggers
        };
        self.plugins.insert(id.clone(), PluginState::Declared { path });

        for cmd in on_command {
            // Collisions already filtered by declare_plugin; on_command is clean.
            self.command_triggers.insert(cmd, id.clone());
        }
        for hook in on_event {
            self.event_triggers.entry(hook).or_default().push(id.clone());
        }
        for lang in on_language {
            self.language_triggers
                .entry(lang)
                .or_default()
                .push(id.clone());
        }
    }

    /// Drop all trigger-map entries owned by `id` (called on load or fail).
    ///
    /// After `activate_plugin` completes (success or error), the plugin's
    /// lazy stubs are superseded by real commands or cleaned up entirely.
    /// Dangling trigger entries would re-fire activation, so they must be
    /// removed unconditionally on both code paths.
    pub(super) fn drop_triggers_for(&mut self, id: &PluginId) {
        self.command_triggers.retain(|_, p| p != id);
        self.event_triggers.retain(|_, plugins| {
            plugins.retain(|p| p != id);
            !plugins.is_empty()
        });
        self.language_triggers.retain(|_, plugins| {
            plugins.retain(|p| p != id);
            !plugins.is_empty()
        });
    }

    /// Build a human-readable status table for `:plugin-status`.
    ///
    /// Rows are sorted by plugin id for stable output.  For plugins still in
    /// the `Declared` state (not yet loaded), the pending trigger lists are
    /// read from the live maps — exactly the triggers the plugin is still
    /// waiting on.  Once a plugin loads or fails, `activate_plugin` drops its
    /// entries from the maps, so `Loaded`/`Failed` rows show no triggers.
    ///
    /// Returns `""` if no plugins are declared; the caller reports "No plugins
    /// declared" rather than opening an empty scratch view.
    pub(crate) fn format_status(&self) -> String {
        if self.plugins.is_empty() {
            return String::new();
        }

        let mut rows: Vec<(String, &'static str, String)> = self
            .plugins
            .iter()
            .map(|(id, state)| {
                let id_s = id.to_string();
                let state_label = match state {
                    PluginState::Declared { .. } => "declared",
                    PluginState::Loading => "loading",
                    PluginState::Loaded => "loaded",
                    PluginState::Failed => "failed",
                };
                let triggers = if matches!(state, PluginState::Declared { .. }) {
                    self.pending_triggers(id)
                } else {
                    String::new()
                };
                (id_s, state_label, triggers)
            })
            .collect();

        rows.sort_by(|a, b| a.0.cmp(&b.0));

        let id_width = rows
            .iter()
            .map(|(id, _, _)| id.len())
            .max()
            .expect("rows non-empty")
            .max(6);

        let mut out = format!(
            "{:<w$}  {:<8}  {}\n",
            "plugin",
            "state",
            "triggers",
            w = id_width
        );
        for (id, state, triggers) in &rows {
            out.push_str(&format!(
                "{:<w$}  {:<8}  {}\n",
                id,
                state,
                triggers,
                w = id_width
            ));
        }
        out
    }

    /// Invert the three live trigger maps to collect the pending triggers for `id`.
    ///
    /// Only meaningful for `Declared` plugins — on load/fail `activate_plugin`
    /// drops the plugin's entries, so a non-`Declared` id yields nothing.
    fn pending_triggers(&self, id: &PluginId) -> String {
        let mut parts = Vec::new();

        let mut cmds: Vec<&str> = self
            .command_triggers
            .iter()
            .filter(|(_, p)| *p == id)
            .map(|(c, _)| c.as_str())
            .collect();
        cmds.sort_unstable();
        if !cmds.is_empty() {
            parts.push(format!("cmd:{}", cmds.join(",")));
        }

        let mut evts: Vec<&str> = self
            .event_triggers
            .iter()
            .filter(|(_, ps)| ps.contains(id))
            .map(|(h, _)| h.symbol())
            .collect();
        evts.sort_unstable();
        if !evts.is_empty() {
            parts.push(format!("event:{}", evts.join(",")));
        }

        let mut langs: Vec<&str> = self
            .language_triggers
            .iter()
            .filter(|(_, ps)| ps.contains(id))
            .map(|(l, _)| l.as_str())
            .collect();
        langs.sort_unstable();
        if !langs.is_empty() {
            parts.push(format!("lang:{}", langs.join(",")));
        }

        if parts.is_empty() {
            "\u{2014}".to_string() // — (em dash): bare declare-plugin, waits on explicit load-plugin
        } else {
            parts.join("  ")
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_path() -> PathBuf {
        PathBuf::from("/fake/plugin.scm")
    }

    fn id_core(name: &str) -> PluginId {
        PluginId::Core(name.to_string())
    }

    fn id_user(user: &str, repo: &str) -> PluginId {
        PluginId::User {
            user: user.to_string(),
            repo: repo.to_string(),
        }
    }

    // ── PluginState inserted for resolved plugins ─────────────────────────

    #[test]
    fn declare_resolved_path_inserts_declared_state() {
        let mut reg = LazyRegistry::default();
        let id = id_core("plum");
        reg.declare(id.clone(), Some(fake_path()), vec![], vec![], vec![]);
        assert!(
            matches!(reg.plugins[&id], PluginState::Declared { .. }),
            "resolved plugin must be Declared"
        );
    }

    #[test]
    fn declare_absent_path_not_in_plugins_map() {
        let mut reg = LazyRegistry::default();
        reg.declare(id_core("missing"), None, vec![], vec![], vec![]);
        assert!(
            reg.plugins.is_empty(),
            "absent-path plugin must not enter the plugins map"
        );
    }

    #[test]
    fn declare_absent_path_records_no_triggers() {
        let mut reg = LazyRegistry::default();
        reg.declare(
            id_core("missing"),
            None,
            vec!["foo".to_string()],
            vec![HookId::OnBufferOpen],
            vec!["rust".to_string()],
        );
        assert!(reg.command_triggers.is_empty());
        assert!(reg.event_triggers.is_empty());
        assert!(reg.language_triggers.is_empty());
    }

    // ── Dedup ─────────────────────────────────────────────────────────────

    #[test]
    fn duplicate_declare_is_noop() {
        let mut reg = LazyRegistry::default();
        let id = id_user("alice", "foo");
        reg.declare(
            id.clone(),
            Some(fake_path()),
            vec!["cmd".to_string()],
            vec![],
            vec![],
        );
        // Second declare with a different path and additional trigger — both ignored.
        reg.declare(
            id.clone(),
            Some(PathBuf::from("/other/plugin.scm")),
            vec!["cmd2".to_string()],
            vec![],
            vec![],
        );
        // State unchanged from first declare.
        assert!(reg.plugins.len() == 1);
        // Second command trigger not recorded.
        assert!(!reg.command_triggers.contains_key("cmd2"));
    }

    #[test]
    fn case_insensitive_dedup() {
        let mut reg = LazyRegistry::default();
        reg.declare(id_user("Alice", "Foo"), Some(fake_path()), vec![], vec![], vec![]);
        // Same plugin, different casing — PluginId equality is case-insensitive.
        reg.declare(id_user("alice", "foo"), Some(fake_path()), vec![], vec![], vec![]);
        assert_eq!(reg.plugins.len(), 1, "case-insensitive dedup must fire");
    }

    // ── Trigger recording ─────────────────────────────────────────────────

    #[test]
    fn command_triggers_insert_overwrites() {
        // declare() unconditionally inserts; collision prevention lives in
        // declare_plugin (one layer up) before this function is called.
        let mut reg = LazyRegistry::default();
        let a = id_user("a", "x");
        let b = id_user("b", "y");
        reg.declare(a.clone(), Some(fake_path()), vec!["foo".to_string()], vec![], vec![]);
        reg.declare(b.clone(), Some(fake_path()), vec!["foo".to_string()], vec![], vec![]);
        // Second declare overwrites — collisions are caught before this is called.
        assert_eq!(reg.command_triggers["foo"], b);
    }

    #[test]
    fn event_triggers_are_one_to_many() {
        let mut reg = LazyRegistry::default();
        let a = id_user("a", "x");
        let b = id_user("b", "y");
        reg.declare(
            a.clone(),
            Some(fake_path()),
            vec![],
            vec![HookId::OnBufferSave],
            vec![],
        );
        reg.declare(
            b.clone(),
            Some(fake_path()),
            vec![],
            vec![HookId::OnBufferSave],
            vec![],
        );
        let handlers = &reg.event_triggers[&HookId::OnBufferSave];
        assert_eq!(handlers.len(), 2, "two plugins must both register for the hook");
        assert!(handlers.contains(&a));
        assert!(handlers.contains(&b));
    }

    #[test]
    fn language_triggers_are_one_to_many() {
        let mut reg = LazyRegistry::default();
        let a = id_user("a", "x");
        let b = id_user("b", "y");
        reg.declare(a.clone(), Some(fake_path()), vec![], vec![], vec!["rust".to_string()]);
        reg.declare(b.clone(), Some(fake_path()), vec![], vec![], vec!["rust".to_string()]);
        let handlers = &reg.language_triggers["rust"];
        assert_eq!(handlers.len(), 2);
        assert!(handlers.contains(&a));
        assert!(handlers.contains(&b));
    }

    #[test]
    fn multiple_commands_for_one_plugin() {
        let mut reg = LazyRegistry::default();
        let id = id_user("user", "repo");
        reg.declare(
            id.clone(),
            Some(fake_path()),
            vec!["cmd-a".to_string(), "cmd-b".to_string()],
            vec![],
            vec![],
        );
        assert_eq!(reg.command_triggers["cmd-a"], id);
        assert_eq!(reg.command_triggers["cmd-b"], id);
    }

    #[test]
    fn multiple_events_for_one_plugin() {
        let mut reg = LazyRegistry::default();
        let id = id_user("user", "repo");
        reg.declare(
            id.clone(),
            Some(fake_path()),
            vec![],
            vec![HookId::OnBufferOpen, HookId::OnBufferSave],
            vec![],
        );
        assert!(reg.event_triggers[&HookId::OnBufferOpen].contains(&id));
        assert!(reg.event_triggers[&HookId::OnBufferSave].contains(&id));
    }

    // ── Loaded-plugins derivation (used by (loaded-plugins) builtin) ──────

    #[test]
    fn loaded_plugins_derived_from_state() {
        let mut reg = LazyRegistry::default();
        let loaded_id = id_user("loaded", "one");
        let declared_id = id_user("pending", "two");

        reg.declare(loaded_id.clone(), Some(fake_path()), vec![], vec![], vec![]);
        reg.declare(declared_id.clone(), Some(fake_path()), vec![], vec![], vec![]);

        // Manually advance one to Loaded to simulate activate_plugin.
        *reg.plugins.get_mut(&loaded_id).unwrap() = PluginState::Loaded;

        let loaded: Vec<_> = reg
            .plugins
            .iter()
            .filter(|(_, s)| matches!(s, PluginState::Loaded))
            .map(|(id, _)| id.to_string())
            .collect();

        assert_eq!(loaded, vec!["loaded/one"]);
    }

    // ── format_status ─────────────────────────────────────────────────────

    #[test]
    fn format_status_empty_returns_empty_string() {
        let reg = LazyRegistry::default();
        assert_eq!(reg.format_status(), "", "empty registry must return empty");
    }

    #[test]
    fn format_status_waiting_with_triggers() {
        let mut reg = LazyRegistry::default();
        reg.declare(
            id_user("alice", "lazy"),
            Some(fake_path()),
            vec!["my-cmd".to_string()],
            vec![HookId::OnBufferSave],
            vec!["rust".to_string()],
        );
        let out = reg.format_status();
        assert!(out.contains("alice/lazy"), "plugin id must appear");
        assert!(out.contains("declared"), "state must be 'declared'");
        assert!(out.contains("cmd:my-cmd"), "command trigger must appear");
        assert!(out.contains("event:on-buffer-save"), "event trigger must appear");
        assert!(out.contains("lang:rust"), "language trigger must appear");
    }

    #[test]
    fn format_status_bare_lazy_shows_em_dash() {
        let mut reg = LazyRegistry::default();
        reg.declare(
            id_user("bob", "bare"),
            Some(fake_path()),
            vec![],
            vec![],
            vec![],
        );
        let out = reg.format_status();
        assert!(out.contains("bob/bare"));
        assert!(out.contains("declared"));
        assert!(out.contains('\u{2014}'), "bare lazy must show em dash");
        assert!(!out.contains("cmd:"), "no cmd prefix for bare lazy");
    }

    #[test]
    fn format_status_loaded_shows_no_triggers() {
        let mut reg = LazyRegistry::default();
        let id = id_user("carol", "eager");
        reg.declare(
            id.clone(),
            Some(fake_path()),
            vec!["eager-cmd".to_string()],
            vec![],
            vec![],
        );
        // Simulate activate_plugin: drop the plugin's trigger-map entries and
        // set Loaded, mirroring what mod.rs:activate_plugin does.
        reg.command_triggers.retain(|_, p| p != &id);
        *reg.plugins.get_mut(&id).unwrap() = PluginState::Loaded;

        let out = reg.format_status();
        assert!(out.contains("carol/eager"), "plugin id must appear");
        assert!(out.contains("loaded"), "state must be 'loaded'");
        assert!(!out.contains("eager-cmd"), "loaded plugin must not show its old trigger");
    }

    #[test]
    fn format_status_failed_shows_no_triggers() {
        let mut reg = LazyRegistry::default();
        let id = id_core("broken");
        reg.declare(
            id.clone(),
            Some(fake_path()),
            vec!["broken-cmd".to_string()],
            vec![],
            vec![],
        );
        reg.command_triggers.retain(|_, p| p != &id);
        *reg.plugins.get_mut(&id).unwrap() = PluginState::Failed;

        let out = reg.format_status();
        assert!(out.contains("core:broken"));
        assert!(out.contains("failed"));
        assert!(!out.contains("broken-cmd"), "failed plugin must not show trigger");
    }

    #[test]
    fn format_status_sorts_by_id() {
        let mut reg = LazyRegistry::default();
        reg.declare(id_user("z", "last"), Some(fake_path()), vec![], vec![], vec![]);
        reg.declare(id_user("a", "first"), Some(fake_path()), vec![], vec![], vec![]);
        let out = reg.format_status();
        let z_pos = out.find("z/last").expect("z/last must appear");
        let a_pos = out.find("a/first").expect("a/first must appear");
        assert!(a_pos < z_pos, "rows must be sorted alphabetically by id");
    }
}
