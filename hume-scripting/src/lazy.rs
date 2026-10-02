//! Lazy plugin loading data model.
//!
//! [`LazyRegistry`] is held on [`super::ScriptingHost`] and borrowed into
//! [`super::SteelCtx`] during every eval.  It tracks each plugin's lifecycle
//! state and the two activation maps consulted by event firing and
//! language-set to activate lazy plugins on demand.  Command activation
//! routing is *not* tracked here: the editor's `CommandRegistry` is the sole
//! owner of `Lazy` command stubs (see `CommandHost::register_lazy_command`),
//! reached through the host rather than a parallel map.

use rustc_hash::FxHashMap;
use std::path::PathBuf;

use super::attribution::{EntryId, PluginId};

/// The activations column of a row with nothing waiting to trigger it.
const NO_ACTIVATIONS: &str = "\u{2014}";

/// Lifecycle state of a declared plugin.
#[derive(Debug)]
pub(crate) enum PluginState {
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
pub(crate) struct LazyRegistry {
    /// Per-entry lifecycle state. An entry appears once it is declared with its
    /// file on disk, or as `Failed` when its activation fails or its plugin's
    /// manifest fails after declaring it; an entry whose file is absent has no
    /// row (see `PluginRecords` for the plugin as a whole).
    pub(crate) plugins: FxHashMap<EntryId, PluginState>,
    /// 1:many map: event name → plugins that activate on that event.
    pub(crate) activation_events: FxHashMap<String, Vec<EntryId>>,
    /// 1:many map: language name → plugins that activate when the language is set.
    pub(crate) activation_languages: FxHashMap<String, Vec<EntryId>>,
}

impl LazyRegistry {
    /// Record a plugin from a `%declare-plugin!` call (always lazy).
    ///
    /// Command activation entries are registered separately, directly in the
    /// editor's `CommandRegistry` via `CommandHost::register_lazy_command`
    /// (see `declare_plugin` in `builtins/plugins.rs`); this method only
    /// records plugin lifecycle state and the event/language activation maps.
    ///
    /// - Duplicate `id` (case-insensitive) → no-op (first declaration wins).
    /// - All plugins are inserted as `Declared`; they activate when an entry is exercised.
    pub(crate) fn declare(
        &mut self,
        id: EntryId,
        path: PathBuf,
        events: Vec<String>,
        languages: Vec<String>,
    ) {
        if self.plugins.contains_key(&id) {
            return; // already declared: duplicate declare-plugin! call, ignore
        }
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

    /// Whether any entry of `plugin` has been declared.
    pub(crate) fn declares_plugin(&self, plugin: &PluginId) -> bool {
        self.plugins.keys().any(|entry| entry.plugin == *plugin)
    }

    /// Every declared entry of `plugin`.
    pub(crate) fn entries_of(&self, plugin: &PluginId) -> Vec<EntryId> {
        self.plugins
            .keys()
            .filter(|entry| entry.plugin == *plugin)
            .cloned()
            .collect()
    }

    /// Drop all activation-map entries owned by `id` (called on load or fail).
    ///
    /// After `finish_lazy_activation` completes (success or error), the
    /// plugin's lazy stubs are superseded by real commands or cleaned up entirely.
    /// Dangling activation entries would re-fire activation, so they must be
    /// removed unconditionally on both code paths. Command stubs are dropped
    /// separately via `CommandHost::unregister_lazy_stubs_of`.
    pub(super) fn drop_activations_for(&mut self, id: &EntryId) {
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
    /// One row per plugin entry, sorted by id for stable output: a secondary
    /// entry reads `plugin (file)` and sorts right after its plugin's main
    /// entry.  For entries still in the `Declared` state (not yet loaded), the
    /// pending activation entries are read from the live maps: the
    /// triggers the entry is still waiting on.  Once an entry loads or fails,
    /// `finish_lazy_activation` drops its triggers from the maps, so
    /// `Loaded`/`Failed` rows show no activations.
    ///
    /// `lazy_cmds` is the editor's current `Lazy`-stub list (`name`, owning
    /// entry, `is_typed`), the sole source of pending command activations;
    /// this registry does not track them itself.
    ///
    /// Returns `""` if there is nothing to list; the caller reports "No plugins
    /// declared" rather than opening an empty scratch view.
    pub(crate) fn format_status<'a>(
        &self,
        plugin_rows: impl IntoIterator<Item = (&'a PluginId, &'static str)>,
        lazy_cmds: &[(String, EntryId, bool)],
    ) -> String {
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

        // `plugin_rows` are plugins that may have no entry row (not installed,
        // or a manifest that raised before declaring anything); one that does
        // have entries is already listed above.
        rows.extend(
            plugin_rows
                .into_iter()
                .filter(|(plugin, _)| !self.declares_plugin(plugin))
                .map(|(plugin, state)| (plugin.to_string(), state, NO_ACTIVATIONS.to_string())),
        );
        if rows.is_empty() {
            return String::new();
        }

        rows.sort_by(|a, b| a.0.cmp(&b.0));

        // Padded by display width, not `str::len`/Rust's own `{:<w$}` (both
        // count chars/bytes, not terminal cells). This table becomes buffer
        // content in a read-only view, rendered through the normal
        // grapheme-width-aware pipeline, so its own padding must agree with
        // that pipeline's unit.
        let id_width = rows
            .iter()
            .map(|(id, _, _)| {
                hume_rope::width::str_width(id, 0, hume_rope::width::CHROME_TAB_WIDTH)
            })
            .max()
            .expect("rows non-empty")
            .max(6);

        let mut out = format!(
            "{}  {:<8}  {}\n",
            pad_to_display_width("plugin", id_width),
            "state",
            "activations",
        );
        for (id, state, activations) in &rows {
            out.push_str(&format!(
                "{}  {:<8}  {}\n",
                pad_to_display_width(id, id_width),
                state,
                activations,
            ));
        }
        out
    }

    /// Invert the live activation maps (plus the caller-supplied `Lazy`-stub
    /// list) to collect the pending entries for `id`.
    ///
    /// Only meaningful for `Declared` plugins: on load/fail
    /// `finish_lazy_activation` drops the plugin's entries, so a non-`Declared`
    /// id yields nothing.
    fn pending_activations(&self, id: &EntryId, lazy_cmds: &[(String, EntryId, bool)]) -> String {
        let mut parts = Vec::new();

        // Split by kind: `cmd:` names are reachable once bound to a key,
        // `:cmd:` names only from `:`; a flat list couldn't tell the user
        // which a still-`Declared` plugin's pending name would turn out to be.
        let mut cmds: Vec<&str> = lazy_cmds
            .iter()
            .filter(|(_, p, is_typed)| p == id && !is_typed)
            .map(|(c, ..)| c.as_str())
            .collect();
        cmds.sort_unstable();
        if !cmds.is_empty() {
            parts.push(format!("cmd:{}", cmds.join(",")));
        }

        let mut typed_cmds: Vec<&str> = lazy_cmds
            .iter()
            .filter(|(_, p, is_typed)| p == id && *is_typed)
            .map(|(c, ..)| c.as_str())
            .collect();
        typed_cmds.sort_unstable();
        if !typed_cmds.is_empty() {
            parts.push(format!(":cmd:{}", typed_cmds.join(",")));
        }

        let mut evts: Vec<&str> = self
            .activation_events
            .iter()
            .filter(|(_, ps)| ps.contains(id))
            .map(|(name, _)| name.as_str())
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
            NO_ACTIVATIONS.to_string()
        } else {
            parts.join("  ")
        }
    }
}

/// Right-pad `s` with spaces to `width` display columns: [`format_status`]'s
/// own padding, since Rust's `{:<w$}` measures by char count, not
/// `hume_rope::width`, the unit the buffer view this table becomes will
/// actually render it in.
///
/// [`format_status`]: LazyRegistry::format_status
fn pad_to_display_width(s: &str, width: usize) -> String {
    let w = hume_rope::width::str_width(s, 0, hume_rope::width::CHROME_TAB_WIDTH);
    format!("{s}{}", " ".repeat(width.saturating_sub(w)))
}

#[cfg(test)]
mod tests;
