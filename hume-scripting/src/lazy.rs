//! Lazy plugin loading data model.
//!
//! [`LazyRegistry`] is held on [`super::ScriptingHost`] and borrowed into
//! [`super::SteelCtx`] during every eval.  It tracks each plugin's lifecycle
//! state and the two activation maps consulted by event firing and
//! language-set to activate lazy plugins on demand.  Command activation
//! routing is *not* tracked here — the editor's `CommandRegistry` is the sole
//! owner of `Lazy` command stubs (see `CommandHost::register_lazy_command`),
//! reached through the host rather than a parallel map.

use std::collections::HashMap;
use std::path::PathBuf;

use super::attribution::PluginId;
use super::hooks::HookId;

// ── PluginState ───────────────────────────────────────────────────────────────

/// Lifecycle state of a declared plugin.
#[derive(Debug)]
pub enum PluginState {
    /// Declared and located on disk, waiting for activation.
    Declared { path: PathBuf },
    /// Body is currently being evaluated.  Prevents re-entrant activation
    /// (activation cycle A→B→A sees `Loading` and skips without looping).
    Loading,
    /// Body evaluated and commands registered successfully.
    Loaded,
    /// Body failed to evaluate; no retry until `:reload-config`.
    Failed,
}

// ── LazyRegistry ──────────────────────────────────────────────────────────────

/// Persistent plugin state and activation maps.
///
/// Borrowed into [`super::SteelCtx`] for the duration of each eval so that
/// `%declare-plugin!` can write directly.
///
/// Keys are **not** stored here; they use the ordinary keymap as trie leaves
/// that point to command names.  The command name resolves through the
/// editor's `CommandRegistry` (a `Lazy` stub), so dispatch finds the owning
/// plugin without any key-specific machinery or a parallel map here.
#[derive(Debug, Default)]
pub struct LazyRegistry {
    /// Per-plugin lifecycle state.  Only plugins whose path was resolved at
    /// declaration time appear here; absent-path plugins are silently skipped.
    pub plugins: HashMap<PluginId, PluginState>,
    /// 1:many map: hook event → plugins that activate on that event.
    pub activation_events: HashMap<HookId, Vec<PluginId>>,
    /// 1:many map: language name → plugins that activate when the language is set.
    pub activation_languages: HashMap<String, Vec<PluginId>>,
}

impl LazyRegistry {
    /// Record a plugin from a `%declare-plugin!` call (always lazy).
    ///
    /// Command activation entries are registered separately, directly in the
    /// editor's `CommandRegistry` via `CommandHost::register_lazy_command`
    /// (see `declare_plugin` in `builtins/plugins.rs`) — this method only
    /// records plugin lifecycle state and the event/language activation maps.
    ///
    /// - Duplicate `id` (case-insensitive) → no-op (first declaration wins).
    /// - `path = None` → plugin absent on disk; skipped silently; activation
    ///   entries NOT recorded (an absent plugin can never activate, so dangling
    ///   entries would be dead weight until `:reload-config`).
    /// - All plugins are inserted as `Declared`; they activate when an entry is exercised.
    pub fn declare(
        &mut self,
        id: PluginId,
        path: Option<PathBuf>,
        events: Vec<HookId>,
        languages: Vec<String>,
    ) {
        if self.plugins.contains_key(&id) {
            return; // already declared — duplicate declare-plugin call, ignore
        }
        let Some(path) = path else {
            return; // absent on disk — silently skip, no activation entries
        };
        self.plugins
            .insert(id.clone(), PluginState::Declared { path });

        for hook in events {
            self.activation_events
                .entry(hook)
                .or_default()
                .push(id.clone());
        }
        for lang in languages {
            self.activation_languages
                .entry(lang)
                .or_default()
                .push(id.clone());
        }
    }

    /// Drop all activation-map entries owned by `id` (called on load or fail).
    ///
    /// After `activate_plugin` completes (success or error), the plugin's
    /// lazy stubs are superseded by real commands or cleaned up entirely.
    /// Dangling activation entries would re-fire activation, so they must be
    /// removed unconditionally on both code paths. Command stubs are dropped
    /// separately via `CommandHost::unregister_lazy_stubs_of`.
    pub(super) fn drop_activations_for(&mut self, id: &PluginId) {
        self.activation_events.retain(|_, plugins| {
            plugins.retain(|p| p != id);
            !plugins.is_empty()
        });
        self.activation_languages.retain(|_, plugins| {
            plugins.retain(|p| p != id);
            !plugins.is_empty()
        });
    }

    /// Build a human-readable status table for `:plugin-status`.
    ///
    /// Rows are sorted by plugin id for stable output.  For plugins still in
    /// the `Declared` state (not yet loaded), the pending activation entries are
    /// read from the live maps — exactly the entries the plugin is still waiting
    /// on.  Once a plugin loads or fails, `activate_plugin` drops its entries
    /// from the maps, so `Loaded`/`Failed` rows show no activations.
    ///
    /// `lazy_cmds` is the editor's current `Lazy`-stub list (`name`, owning
    /// plugin) — the sole source of pending command activations; this
    /// registry does not track them itself.
    ///
    /// Returns `""` if no plugins are declared; the caller reports "No plugins
    /// declared" rather than opening an empty scratch view.
    pub fn format_status(&self, lazy_cmds: &[(String, PluginId)]) -> String {
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
                let activations = if matches!(state, PluginState::Declared { .. }) {
                    self.pending_activations(id, lazy_cmds)
                } else {
                    String::new()
                };
                (id_s, state_label, activations)
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
            "activations",
            w = id_width
        );
        for (id, state, activations) in &rows {
            out.push_str(&format!(
                "{:<w$}  {:<8}  {}\n",
                id,
                state,
                activations,
                w = id_width
            ));
        }
        out
    }

    /// Invert the live activation maps (plus the caller-supplied `Lazy`-stub
    /// list) to collect the pending entries for `id`.
    ///
    /// Only meaningful for `Declared` plugins — on load/fail `activate_plugin`
    /// drops the plugin's entries, so a non-`Declared` id yields nothing.
    fn pending_activations(&self, id: &PluginId, lazy_cmds: &[(String, PluginId)]) -> String {
        let mut parts = Vec::new();

        let mut cmds: Vec<&str> = lazy_cmds
            .iter()
            .filter(|(_, p)| p == id)
            .map(|(c, _)| c.as_str())
            .collect();
        cmds.sort_unstable();
        if !cmds.is_empty() {
            parts.push(format!("cmd:{}", cmds.join(",")));
        }

        let mut evts: Vec<&str> = self
            .activation_events
            .iter()
            .filter(|(_, ps)| ps.contains(id))
            .map(|(h, _)| h.symbol())
            .collect();
        evts.sort_unstable();
        if !evts.is_empty() {
            parts.push(format!("event:{}", evts.join(",")));
        }

        let mut langs: Vec<&str> = self
            .activation_languages
            .iter()
            .filter(|(_, ps)| ps.contains(id))
            .map(|(l, _)| l.as_str())
            .collect();
        langs.sort_unstable();
        if !langs.is_empty() {
            parts.push(format!("lang:{}", langs.join(",")));
        }

        if parts.is_empty() {
            // Defensive fallback: policy in declare_plugin rejects zero-activation-entry
            // declarations, but the data layer does not enforce this invariant.
            "\u{2014}".to_string()
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
        reg.declare(id.clone(), Some(fake_path()), vec![], vec![]);
        assert!(
            matches!(reg.plugins[&id], PluginState::Declared { .. }),
            "resolved plugin must be Declared"
        );
    }

    #[test]
    fn declare_absent_path_not_in_plugins_map() {
        let mut reg = LazyRegistry::default();
        reg.declare(id_core("missing"), None, vec![], vec![]);
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
            vec![HookId::OnBufferOpen],
            vec!["rust".to_string()],
        );
        assert!(reg.activation_events.is_empty());
        assert!(reg.activation_languages.is_empty());
    }

    // ── Dedup ─────────────────────────────────────────────────────────────

    #[test]
    fn duplicate_declare_is_noop() {
        let mut reg = LazyRegistry::default();
        let id = id_user("alice", "foo");
        reg.declare(
            id.clone(),
            Some(fake_path()),
            vec![HookId::OnBufferSave],
            vec![],
        );
        // Second declare with a different path and an additional activation entry — both ignored.
        reg.declare(
            id.clone(),
            Some(PathBuf::from("/other/plugin.scm")),
            vec![HookId::OnBufferOpen],
            vec![],
        );
        // State unchanged from first declare.
        assert!(reg.plugins.len() == 1);
        // Second declare's event entry not recorded.
        assert!(
            !reg.activation_events
                .get(&HookId::OnBufferOpen)
                .is_some_and(|ps| ps.contains(&id)),
            "second declare's activation entry must not be recorded"
        );
    }

    #[test]
    fn case_insensitive_dedup() {
        let mut reg = LazyRegistry::default();
        reg.declare(id_user("Alice", "Foo"), Some(fake_path()), vec![], vec![]);
        // Same plugin, different casing — PluginId equality is case-insensitive.
        reg.declare(id_user("alice", "foo"), Some(fake_path()), vec![], vec![]);
        assert_eq!(reg.plugins.len(), 1, "case-insensitive dedup must fire");
    }

    // ── Activation map recording ───────────────────────────────────────────

    #[test]
    fn activation_events_are_one_to_many() {
        let mut reg = LazyRegistry::default();
        let a = id_user("a", "x");
        let b = id_user("b", "y");
        reg.declare(
            a.clone(),
            Some(fake_path()),
            vec![HookId::OnBufferSave],
            vec![],
        );
        reg.declare(
            b.clone(),
            Some(fake_path()),
            vec![HookId::OnBufferSave],
            vec![],
        );
        let handlers = &reg.activation_events[&HookId::OnBufferSave];
        assert_eq!(
            handlers.len(),
            2,
            "two plugins must both register for the hook"
        );
        assert!(handlers.contains(&a));
        assert!(handlers.contains(&b));
    }

    #[test]
    fn activation_languages_are_one_to_many() {
        let mut reg = LazyRegistry::default();
        let a = id_user("a", "x");
        let b = id_user("b", "y");
        reg.declare(
            a.clone(),
            Some(fake_path()),
            vec![],
            vec!["rust".to_string()],
        );
        reg.declare(
            b.clone(),
            Some(fake_path()),
            vec![],
            vec!["rust".to_string()],
        );
        let handlers = &reg.activation_languages["rust"];
        assert_eq!(handlers.len(), 2);
        assert!(handlers.contains(&a));
        assert!(handlers.contains(&b));
    }

    #[test]
    fn multiple_events_for_one_plugin() {
        let mut reg = LazyRegistry::default();
        let id = id_user("user", "repo");
        reg.declare(
            id.clone(),
            Some(fake_path()),
            vec![HookId::OnBufferOpen, HookId::OnBufferSave],
            vec![],
        );
        assert!(reg.activation_events[&HookId::OnBufferOpen].contains(&id));
        assert!(reg.activation_events[&HookId::OnBufferSave].contains(&id));
    }

    // ── Loaded-plugins derivation (used by (loaded-plugins) builtin) ──────

    #[test]
    fn loaded_plugins_derived_from_state() {
        let mut reg = LazyRegistry::default();
        let loaded_id = id_user("loaded", "one");
        let declared_id = id_user("pending", "two");

        reg.declare(loaded_id.clone(), Some(fake_path()), vec![], vec![]);
        reg.declare(declared_id.clone(), Some(fake_path()), vec![], vec![]);

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
        assert_eq!(
            reg.format_status(&[]),
            "",
            "empty registry must return empty"
        );
    }

    #[test]
    fn format_status_waiting_with_triggers() {
        let mut reg = LazyRegistry::default();
        let id = id_user("alice", "lazy");
        reg.declare(
            id.clone(),
            Some(fake_path()),
            vec![HookId::OnBufferSave],
            vec!["rust".to_string()],
        );
        let lazy_cmds = vec![("my-cmd".to_string(), id)];
        let out = reg.format_status(&lazy_cmds);
        assert!(out.contains("alice/lazy"), "plugin id must appear");
        assert!(out.contains("declared"), "state must be 'declared'");
        assert!(
            out.contains("cmd:my-cmd"),
            "command activation entry must appear"
        );
        assert!(
            out.contains("event:on-buffer-save"),
            "event activation entry must appear"
        );
        assert!(
            out.contains("lang:rust"),
            "language activation entry must appear"
        );
    }

    // The data layer accepts zero-activation plugins (LazyRegistry::declare has no
    // policy gate); the policy guard lives in declare_plugin (builtins layer).
    // This test exercises the defensive em-dash fallback in pending_activations.
    #[test]
    fn format_status_zero_trigger_shows_em_dash() {
        let mut reg = LazyRegistry::default();
        reg.declare(id_user("bob", "bare"), Some(fake_path()), vec![], vec![]);
        let out = reg.format_status(&[]);
        assert!(out.contains("bob/bare"));
        assert!(out.contains("declared"));
        assert!(
            out.contains('\u{2014}'),
            "zero-entry plugin must show em dash"
        );
        assert!(!out.contains("cmd:"), "no cmd prefix for zero-entry plugin");
    }

    #[test]
    fn format_status_loaded_shows_no_triggers() {
        let mut reg = LazyRegistry::default();
        let id = id_user("carol", "eager");
        reg.declare(id.clone(), Some(fake_path()), vec![], vec![]);
        // Simulate activate_plugin: set Loaded. The editor's Lazy stub for
        // "eager-cmd" is gone by now too (unregister_lazy_stubs_of already
        // ran), so the caller passes an empty lazy_cmds — exactly what a real
        // post-activation `:plugin-status` call would see.
        *reg.plugins.get_mut(&id).unwrap() = PluginState::Loaded;

        let out = reg.format_status(&[]);
        assert!(out.contains("carol/eager"), "plugin id must appear");
        assert!(out.contains("loaded"), "state must be 'loaded'");
        assert!(
            !out.contains("eager-cmd"),
            "loaded plugin must not show its old activation entry"
        );
    }

    #[test]
    fn format_status_failed_shows_no_triggers() {
        let mut reg = LazyRegistry::default();
        let id = id_core("broken");
        reg.declare(id.clone(), Some(fake_path()), vec![], vec![]);
        *reg.plugins.get_mut(&id).unwrap() = PluginState::Failed;

        let out = reg.format_status(&[]);
        assert!(out.contains("core:broken"));
        assert!(out.contains("failed"));
        assert!(
            !out.contains("broken-cmd"),
            "failed plugin must not show activation entry"
        );
    }

    #[test]
    fn format_status_sorts_by_id() {
        let mut reg = LazyRegistry::default();
        reg.declare(id_user("z", "last"), Some(fake_path()), vec![], vec![]);
        reg.declare(id_user("a", "first"), Some(fake_path()), vec![], vec![]);
        let out = reg.format_status(&[]);
        let z_pos = out.find("z/last").expect("z/last must appear");
        let a_pos = out.find("a/first").expect("a/first must appear");
        assert!(a_pos < z_pos, "rows must be sorted alphabetically by id");
    }
}
