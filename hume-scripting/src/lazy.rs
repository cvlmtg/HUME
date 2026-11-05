//! Lazy plugin loading data model.
//!
//! [`LazyRegistry`] is held on [`super::ScriptingHost`] and borrowed into
//! [`super::SteelCtx`] during every eval.  It tracks each plugin's lifecycle
//! state, the event activation map consulted by event firing, and each
//! entry's declared languages, consulted by language-set to activate lazy
//! plugins on demand and kept as the entry's language scope.  Command activation
//! routing is *not* tracked here: the editor's `CommandRegistry` is the sole
//! owner of `Lazy` command stubs (see `CommandHost::register_lazy_commands`),
//! reached through the host rather than a parallel map.

use rustc_hash::FxHashMap;
use std::path::PathBuf;
use std::rc::Rc;

use super::attribution::{EntryId, Owner, PluginId};
use super::visibility::BufferVisibility;

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

impl PluginState {
    pub(crate) fn status(&self) -> PluginStatus {
        match self {
            Self::Declared { .. } => PluginStatus::Declared,
            Self::Loading => PluginStatus::Loading,
            Self::Loaded => PluginStatus::Loaded,
            Self::Failed => PluginStatus::Failed,
        }
    }
}

/// A plugin's lifecycle state without its `Declared` path: the public view
/// the editor reads to tell a declared lazy plugin from a loaded one,
/// without reaching into `ScriptingHost` fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginStatus {
    /// Declared but not yet loaded.
    Declared,
    /// Currently being loaded (re-entrancy guard).
    Loading,
    /// Loaded successfully.
    Loaded,
    /// Load failed (will not retry until `:reload-config`).
    Failed,
}

/// Persistent plugin state, activation map and declared languages.
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
    /// 1:many map: event name → the entries declared with it in
    /// `#:events`, for the whole session. Only a `Declared` one activates.
    pub(crate) activation_events: FxHashMap<String, Vec<EntryId>>,
    /// Each entry's `#:languages`, in declaration order, for the whole
    /// session: what activates the entry while it is `Declared`, and its
    /// language scope once its code runs. An entry declared with none has
    /// no row.
    pub(crate) languages: Vec<(EntryId, Rc<[String]>)>,
}

impl LazyRegistry {
    /// Record a plugin from a `%declare-plugin!` call (always lazy).
    ///
    /// Command activation entries are registered separately, directly in the
    /// editor's `CommandRegistry` via `CommandHost::register_lazy_commands`
    /// (see `declare_plugin` in `builtins/plugins.rs`); this method only
    /// records plugin lifecycle state, the event activation map and the
    /// entry's declared languages.
    ///
    /// - Each `id` (case-insensitive) is declared once: `declare_plugin`
    ///   raises for a second declaration before it gets here.
    /// - All plugins are inserted as `Declared`; they activate when an entry is exercised.
    pub(crate) fn declare(
        &mut self,
        id: EntryId,
        path: PathBuf,
        events: Vec<String>,
        languages: Vec<String>,
    ) {
        let previous = self
            .plugins
            .insert(id.clone(), PluginState::Declared { path });
        assert!(previous.is_none(), "entry '{id}' declared twice");

        for hook in events {
            self.activation_events
                .entry(hook)
                .or_default()
                .push(id.clone());
        }
        if !languages.is_empty() {
            self.languages.push((id, languages.into()));
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

    /// `Declared` entries to activate when the event named `name` fires.
    pub(crate) fn event_plugins(&self, name: &str) -> Vec<EntryId> {
        self.activation_events
            .get(name)
            .into_iter()
            .flatten()
            .filter(|id| self.is_declared(id))
            .cloned()
            .collect()
    }

    /// Whether `id` is waiting to activate: once it loads or fails, its
    /// events and languages activate it no more.
    fn is_declared(&self, id: &EntryId) -> bool {
        matches!(self.plugins.get(id), Some(PluginState::Declared { .. }))
    }

    /// `Declared` entries to activate when `language` is set on a buffer:
    /// those listing `language`, then those listing the wildcard `"*"` (a
    /// manifest that can't enumerate every language it might support), each
    /// once, in declaration order.
    pub(crate) fn language_plugins(&self, language: &str) -> Vec<EntryId> {
        let mut plugins: Vec<EntryId> = self.declared_listing(language).cloned().collect();
        for id in self.declared_listing("*") {
            if !plugins.contains(id) {
                plugins.push(id.clone());
            }
        }
        plugins
    }

    /// The `Declared` entries whose `#:languages` names `lang`.
    fn declared_listing<'a>(&'a self, lang: &'a str) -> impl Iterator<Item = &'a EntryId> {
        self.languages
            .iter()
            .filter(move |(id, langs)| self.is_declared(id) && langs.iter().any(|l| l == lang))
            .map(|(id, _)| id)
    }

    /// The `#:languages` `id` was declared with, or `None` for an entry
    /// declared with none.
    pub(crate) fn declared_languages(&self, id: &EntryId) -> Option<&Rc<[String]>> {
        self.languages
            .iter()
            .find(|(entry, _)| entry == id)
            .map(|(_, langs)| langs)
    }

    /// The buffers code `owner` wrote acts on: a plugin entry's declared
    /// languages, or every buffer for an entry declared with none or with
    /// `"*"`, and for `init.scm` and HUME's own code.
    pub(crate) fn visibility_of(&self, owner: &Owner) -> BufferVisibility {
        let Owner::Plugin(entry) = owner else {
            return BufferVisibility::default();
        };
        self.declared_languages(entry)
            .map(|langs| BufferVisibility::declared(Rc::clone(langs)))
            .unwrap_or_default()
    }

    /// Build a human-readable status table for `:plugin-status`.
    ///
    /// One row per plugin entry, sorted by id for stable output: a secondary
    /// entry reads `plugin (file)` and sorts right after its plugin's main
    /// entry.  For entries still in the `Declared` state (not yet loaded), the
    /// pending activation entries are the triggers the entry is still
    /// waiting on.  A `Loaded` row shows the languages it acts on, if it was
    /// declared with any; a `Failed` row shows nothing.
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
        use std::fmt::Write;

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
                let activations = match state {
                    PluginState::Declared { .. } => self.pending_activations(id, lazy_cmds),
                    PluginState::Loaded => self.scope_column(id).unwrap_or_default(),
                    PluginState::Loading | PluginState::Failed => String::new(),
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
            .map(|(id, _, _)| hume_rope::width::chrome_width(id))
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
            writeln!(
                out,
                "{}  {:<8}  {}",
                pad_to_display_width(id, id_width),
                state,
                activations,
            )
            .expect("writing to a String cannot fail");
        }
        out
    }

    /// Collect the pending activation entries for `id` from the event map,
    /// its declared languages and the caller-supplied `Lazy`-stub list.
    /// Called for a `Declared` entry only: once an entry loads or fails,
    /// nothing is pending.
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

        parts.extend(self.languages_column(id));

        if parts.is_empty() {
            // Defensive fallback: policy in declare_plugin rejects zero-activation-entry
            // declarations, but the data layer does not enforce this invariant.
            NO_ACTIVATIONS.to_string()
        } else {
            parts.join("  ")
        }
    }

    /// The buffers loaded entry `id` acts on: `lang:*` when its
    /// `#:languages` does not limit it, else its languages as for
    /// [`Self::languages_column`]. `None` for an entry declared with none.
    fn scope_column(&self, id: &EntryId) -> Option<String> {
        let langs = self.declared_languages(id)?;
        if BufferVisibility::limits(langs) {
            self.languages_column(id)
        } else {
            Some("lang:*".to_string())
        }
    }

    /// `id`'s declared languages as `lang:a,b`, sorted, or `None` for an
    /// entry declared with none.
    fn languages_column(&self, id: &EntryId) -> Option<String> {
        let mut langs: Vec<&str> = self
            .declared_languages(id)?
            .iter()
            .map(String::as_str)
            .collect();
        langs.sort_unstable();
        Some(format!("lang:{}", langs.join(",")))
    }
}

/// Right-pad `s` with spaces to `width` display columns: [`format_status`]'s
/// own padding, since Rust's `{:<w$}` measures by char count, not
/// `hume_rope::width`, the unit the buffer view this table becomes will
/// actually render it in.
///
/// [`format_status`]: LazyRegistry::format_status
fn pad_to_display_width(s: &str, width: usize) -> String {
    let w = hume_rope::width::chrome_width(s);
    format!("{s}{}", " ".repeat(width.saturating_sub(w)))
}

#[cfg(test)]
mod tests;
