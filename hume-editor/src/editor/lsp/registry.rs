//! Server registration: `register-lsp-server!` configs by name, each
//! language's server lists, and workspace root resolution. Pure policy: no
//! process state. A registration says how to launch a server; only a
//! language's list makes a server serve that language.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hume_core::{
    Claim, ConfigOrigin, LanguageLists, ListEntry, ListTier, LspServerConfig, Ranked, ServerName,
    ServerRegistrationTie, Written,
};
use rustc_hash::FxHashMap;

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

/// Every server registration and per-language list: pure policy, no
/// process state. [`Registry::plan`] is the one derivation of which servers
/// a language's buffers attach to, in what order, with what filter.
#[derive(Debug, Default)]
pub(in crate::editor::lsp) struct Registry {
    registrations: FxHashMap<ServerName, Ranked<Arc<LspServerConfig>, ServerRegistrationTie>>,
    lists: FxHashMap<String, LanguageLists>,
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
    /// ([`ServerRegistrationTie`]).
    Shadowed,
}

/// What a ranked write or clear of a registration found under its name.
fn registry_write<T>(written: &Written<T>) -> RegistryWrite {
    if !written.outcome.is_applied() {
        RegistryWrite::Shadowed
    } else if written.displaced.is_some() {
        RegistryWrite::Replaced
    } else {
        RegistryWrite::Unregistered
    }
}

impl Registry {
    /// Registers `name` as `origin`'s, replacing an earlier registration of
    /// the same name unless a higher origin made it.
    pub(in crate::editor::lsp) fn register(
        &mut self,
        name: ServerName,
        config: LspServerConfig,
        origin: ConfigOrigin,
    ) -> RegistryWrite {
        let written = self
            .registrations
            .entry(name)
            .or_default()
            .write(Arc::new(config), origin);
        registry_write(&written)
    }

    /// Removes `name`'s registration unless a higher origin than `origin`
    /// made it. Lists that name it keep the entry, which applies again if
    /// the name registers again.
    pub(in crate::editor::lsp) fn unregister(
        &mut self,
        name: &ServerName,
        origin: ConfigOrigin,
    ) -> RegistryWrite {
        let Some(slot) = self.registrations.get_mut(name) else {
            return RegistryWrite::Unregistered;
        };
        let written = slot.clear(origin);
        if slot.get().is_none() {
            self.registrations.remove(name);
        }
        registry_write(&written)
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
        lists.set(tier, origin, entries)
    }

    /// `language`'s server lists, empty when nothing set one.
    pub(in crate::editor::lsp) fn lists(&self, language: &str) -> LanguageLists {
        self.lists.get(language).cloned().unwrap_or_default()
    }

    pub(in crate::editor::lsp) fn contains(&self, name: &ServerName) -> bool {
        self.registrations.contains_key(name)
    }

    pub(in crate::editor::lsp) fn get(&self, name: &ServerName) -> Option<&Arc<LspServerConfig>> {
        self.registrations.get(name).and_then(Ranked::get)
    }

    /// The origin of `name`'s registration, empty when it has none.
    pub(in crate::editor::lsp) fn claim(&self, name: &ServerName) -> Claim<ServerRegistrationTie> {
        self.registrations
            .get(name)
            .map_or_else(Claim::default, Ranked::as_claim)
    }

    /// The languages whose servers include `name`, sorted.
    pub(in crate::editor::lsp) fn languages_of(&self, name: &ServerName) -> Vec<&str> {
        let mut languages: Vec<&str> = self
            .lists
            .keys()
            .map(String::as_str)
            .filter(|language| self.plan(language).iter().any(|e| &e.name == name))
            .collect();
        languages.sort_unstable();
        languages
    }

    /// The servers `language`'s buffers attach to, in order
    /// ([`LanguageLists::plan`] over the registered servers); a registered
    /// server no list names serves nothing.
    pub(in crate::editor::lsp) fn plan(&self, language: &str) -> Vec<&ListEntry> {
        self.lists.get(language).map_or_else(Vec::new, |lists| {
            lists.plan(|name| self.registrations.contains_key(name))
        })
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
