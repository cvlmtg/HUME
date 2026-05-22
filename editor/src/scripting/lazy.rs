//! Lazy plugin loading data model.
//!
//! [`LazyRegistry`] is held on [`super::ScriptingHost`] and borrowed into
//! [`super::SteelCtx`] during every eval.  It tracks each plugin's lifecycle
//! state and the three trigger maps that later phases consume.
//!
//! Phase 0 populates the maps; later phases wire the consumption:
//! - **Phase 1** — command triggers (`MappableCommand::Lazy` stubs, dispatch
//!   interception).
//! - **Phase 2** — event triggers (pre-fire activation in `fire_hook`).
//! - **Phase 3b** — language triggers (pre-set activation in
//!   `set_buffer_language`).

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
    /// declaration time appear here; absent-path plugins are silently skipped
    /// (parity with pre-Phase-0 behavior).
    pub(crate) plugins: HashMap<PluginId, PluginState>,
    /// 1:1 map: command name → owning plugin.
    ///
    /// In Phase 0 a duplicate command name silently keeps the first claimant
    /// (first `load-plugin` wins).  Phase 1 replaces this with a fail-fast
    /// collision check at manifest time.
    pub(crate) command_triggers: HashMap<String, PluginId>,
    /// 1:many map: hook event → plugins that load on that event.
    pub(crate) event_triggers: HashMap<HookId, Vec<PluginId>>,
    /// 1:many map: language name → plugins that load when the language is set.
    pub(crate) language_triggers: HashMap<String, Vec<PluginId>>,
}

impl LazyRegistry {
    /// Record a plugin from a `%declare-plugin!` call.
    ///
    /// - Duplicate `id` (case-insensitive) → no-op (first declaration wins).
    /// - `path = None` → plugin absent on disk; skipped silently; triggers NOT
    ///   recorded (an absent plugin can never activate, so dangling trigger
    ///   entries would be dead weight until `:reload-config`).
    /// - `eager` plugins are inserted as `Declared` so the caller can drain
    ///   them immediately via `ScriptingHost::activate_plugin`; their state
    ///   transitions to `Loaded`/`Failed` during that drain.
    /// - Lazy plugins are also inserted as `Declared`; later phases wire their
    ///   trigger consumption.
    pub(crate) fn declare(
        &mut self,
        id: PluginId,
        path: Option<PathBuf>,
        on_command: Vec<String>,
        on_event: Vec<HookId>,
        on_language: Vec<String>,
    ) {
        if self.plugins.contains_key(&id) {
            return; // already declared — duplicate load-plugin call, ignore
        }
        let Some(path) = path else {
            return; // absent on disk — silently skip, no triggers
        };
        self.plugins.insert(id.clone(), PluginState::Declared { path });

        for cmd in on_command {
            // Collision already checked by declare_plugin before this call.
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
}
