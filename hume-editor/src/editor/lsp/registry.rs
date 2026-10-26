//! Server registration: `register-lsp-server!` configs by name, each
//! language's server lists, and workspace root resolution. Pure policy: no
//! process state. A registration says how to launch a server; only a
//! language's list makes a server serve that language.

use std::path::{Path, PathBuf};

use hume_origin::{ByOrigin, Ranked};
use hume_scripting::{
    ConfigOrigin, FeatureFilter, ListEntry, ListTier, SERVER_REGISTRATION_TIE, ServerName,
};
use rustc_hash::FxHashMap;

/// The process a `register-lsp-server!` call spawns and configures.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::editor::lsp) struct LspServerConfig {
    pub(in crate::editor) command: String,
    pub(in crate::editor) args: Vec<String>,
    /// Sent verbatim as `initializationOptions` in the `initialize` request
    /// (`Instances::spawn`, via `LspClient::set_init_options`).
    pub(in crate::editor) init_options: Option<serde_json::Value>,
    /// Pushed as `workspace/didChangeConfiguration` after `initialized`
    /// (`Instances::spawn`, via `LspClient::set_settings`), and resolved
    /// per-item to answer `workspace/configuration` pull requests from the
    /// copy the instance was spawned with.
    pub(in crate::editor) settings: Option<serde_json::Value>,
    /// `#:env`: applied additively to the spawned process's inherited
    /// environment (`Instances::spawn`, via `LspBackend::start`).
    pub(in crate::editor) env: Vec<(String, String)>,
}

/// Walks up from `file`'s directory to the first ancestor containing any of
/// `markers` (a file or a directory, `.git` included); falls back to `cwd`
/// if none match. `Path::ancestors()` yields `file`'s parent first, then
/// each successively shorter prefix up to (and including) the filesystem
/// root, so the nearest marker wins.
pub(in crate::editor::lsp::registry) fn resolve_root(
    file: &Path,
    markers: &[String],
    cwd: &Path,
) -> PathBuf {
    let start = file.parent().unwrap_or(cwd);
    for dir in start.ancestors() {
        if markers.iter().any(|m| dir.join(m).exists()) {
            return dir.to_path_buf();
        }
    }
    cwd.to_path_buf()
}

/// One tier of a language's server lists: the list each origin set, if any.
/// The highest origin's is the whole answer, whichever was set first, and
/// clearing it lets the next lower one apply.
type ListLayers = ByOrigin<Option<Vec<ListEntry>>>;

/// A language's server lists. A default list applies only while no explicit
/// list is set, whatever the origins of the two.
#[derive(Debug, Default)]
struct LanguageLists {
    explicit: ListLayers,
    defaults: ListLayers,
}

impl LanguageLists {
    fn layers_mut(&mut self, tier: ListTier) -> &mut ListLayers {
        match tier {
            ListTier::Explicit => &mut self.explicit,
            ListTier::Default => &mut self.defaults,
        }
    }

    /// The list that decides the language's servers.
    fn deciding(&self) -> Option<&[ListEntry]> {
        self.explicit
            .iter()
            .chain(self.defaults.iter())
            .find_map(|(_, list)| list.as_deref())
    }
}

/// Every server registration and per-language list: pure policy, no
/// process state. [`Registry::plan`] is the one derivation of which servers
/// a language's buffers attach to, in what order, with what filter.
#[derive(Debug, Default)]
pub(in crate::editor::lsp) struct Registry {
    registrations: FxHashMap<ServerName, Ranked<LspServerConfig>>,
    lists: FxHashMap<String, LanguageLists>,
}

/// One server a language's buffers attach to.
pub(in crate::editor::lsp) struct Planned<'a> {
    pub(in crate::editor::lsp) name: &'a ServerName,
    pub(in crate::editor::lsp) filter: FeatureFilter,
}

/// What a registration or unregistration from one origin found under its
/// name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor::lsp) enum RegistryWrite {
    /// No registration: the write applied to an unregistered name.
    Unregistered,
    /// A registration the write replaced or removed.
    Replaced,
    /// A registration a higher origin made, which stays
    /// ([`SERVER_REGISTRATION_TIE`]).
    Shadowed,
}

impl Registry {
    /// What a write from `origin` to `name` finds there.
    fn rank(&self, name: &ServerName, origin: ConfigOrigin) -> RegistryWrite {
        let held = self.origin(name);
        if !origin.rank(held, SERVER_REGISTRATION_TIE).is_applied() {
            RegistryWrite::Shadowed
        } else if held.is_some() {
            RegistryWrite::Replaced
        } else {
            RegistryWrite::Unregistered
        }
    }

    /// Registers `name` as `origin`'s, replacing an earlier registration of
    /// the same name unless a higher origin made it.
    pub(in crate::editor::lsp) fn register(
        &mut self,
        name: ServerName,
        config: LspServerConfig,
        origin: ConfigOrigin,
    ) -> RegistryWrite {
        let write = self.rank(&name, origin);
        if write != RegistryWrite::Shadowed {
            self.registrations.insert(
                name,
                Ranked {
                    value: config,
                    by: origin,
                },
            );
        }
        write
    }

    /// Removes `name`'s registration unless a higher origin than `origin`
    /// made it. Lists that name it keep the entry, which applies again if
    /// the name registers again.
    pub(in crate::editor::lsp) fn unregister(
        &mut self,
        name: &ServerName,
        origin: ConfigOrigin,
    ) -> RegistryWrite {
        let write = self.rank(name, origin);
        if write != RegistryWrite::Shadowed {
            self.registrations.remove(name);
        }
        write
    }

    /// Sets (`Some`) or clears (`None`) `language`'s `tier` list from
    /// `origin`, leaving the lists other origins set. Returns whether a
    /// higher origin's list in the same tier hides it.
    pub(in crate::editor::lsp) fn set_list(
        &mut self,
        language: String,
        tier: ListTier,
        origin: ConfigOrigin,
        entries: Option<Vec<ListEntry>>,
    ) -> bool {
        let lists = match entries {
            Some(_) => self.lists.entry(language).or_default(),
            None => match self.lists.get_mut(&language) {
                Some(lists) => lists,
                None => return false,
            },
        };
        let layers = lists.layers_mut(tier);
        *layers.get_mut(origin) = entries;
        layers.above(origin).any(Option::is_some)
    }

    pub(in crate::editor::lsp) fn contains(&self, name: &ServerName) -> bool {
        self.registrations.contains_key(name)
    }

    pub(in crate::editor::lsp) fn get(&self, name: &ServerName) -> Option<&LspServerConfig> {
        self.registrations.get(name).map(|reg| &reg.value)
    }

    /// The origin of `name`'s registration, `None` when it has none.
    pub(in crate::editor::lsp) fn origin(&self, name: &ServerName) -> Option<ConfigOrigin> {
        self.registrations.get(name).map(|reg| reg.by)
    }

    /// The languages whose servers include `name`, sorted.
    pub(in crate::editor::lsp) fn languages_of(&self, name: &ServerName) -> Vec<&str> {
        let mut languages: Vec<&str> = self
            .lists
            .keys()
            .map(String::as_str)
            .filter(|language| self.plan(language).iter().any(|p| p.name == name))
            .collect();
        languages.sort_unstable();
        languages
    }

    /// The servers `language`'s buffers attach to, in order: the entries of
    /// the explicit list the highest origin set, or of the default list while
    /// no explicit one is set, that name a registered server. An entry naming
    /// an unregistered server is skipped; a registered server no list names
    /// serves nothing.
    pub(in crate::editor::lsp) fn plan(&self, language: &str) -> Vec<Planned<'_>> {
        let Some(entries) = self.lists.get(language).and_then(LanguageLists::deciding) else {
            return Vec::new();
        };
        entries
            .iter()
            .filter_map(|entry| {
                let (name, _) = self.registrations.get_key_value(&entry.name)?;
                Some(Planned {
                    name,
                    filter: entry.filter,
                })
            })
            .collect()
    }
}

/// Workspace roots resolved during one reconcile pass, by the directory of
/// the file and the markers searched for: buffers sharing both share one
/// walk up the directory tree. A marker created while the pass runs is not
/// seen by the rest of it.
#[derive(Default)]
pub(in crate::editor::lsp) struct RootCache {
    roots: rustc_hash::FxHashMap<(PathBuf, Vec<String>), PathBuf>,
}

impl RootCache {
    /// The workspace root for a buffer at `file` whose language has the root
    /// markers `language_roots`.
    pub(in crate::editor::lsp) fn root_for(
        &mut self,
        language_roots: &[String],
        file: &Path,
        cwd: &Path,
    ) -> PathBuf {
        let dir = file.parent().unwrap_or(cwd).to_path_buf();
        self.roots
            .entry((dir, language_roots.to_vec()))
            .or_insert_with_key(|(_, markers)| resolve_root(file, markers, cwd))
            .clone()
    }
}

#[cfg(test)]
mod tests;
