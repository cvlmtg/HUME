//! Server registration: `register-lsp-server!` configs by name, each
//! language's server lists, and workspace root resolution. Pure policy: no
//! process state. A registration says how to launch a server; only a
//! language's list makes a server serve that language.

use std::path::{Path, PathBuf};

use hume_scripting::{FeatureFilter, ListEntry, ListLayer, ServerName};
use rustc_hash::FxHashMap;

/// The process a `register-lsp-server!` call spawns and configures.
#[derive(Debug, Clone)]
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

impl LspServerConfig {
    /// Whether a server spawned with `other` is the process this config
    /// describes: the same command, arguments, environment and the two
    /// blobs it is told at startup.
    pub(in crate::editor::lsp) fn same_process(&self, other: &Self) -> bool {
        let Self {
            command,
            args,
            init_options,
            settings,
            env,
        } = self;
        *command == other.command
            && *args == other.args
            && *init_options == other.init_options
            && *settings == other.settings
            && *env == other.env
    }
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

/// A language's two ordered server lists. A `user` list, when set, is the
/// whole answer; `default` is a plugin's preferred order, used only while
/// there is no user list.
#[derive(Debug, Default)]
struct LanguageLists {
    user: Option<Vec<ListEntry>>,
    default: Option<Vec<ListEntry>>,
}

/// Every server registration and per-language list: pure policy, no
/// process state. [`Registry::plan`] is the one derivation of which servers
/// a language's buffers attach to, in what order, with what filter.
#[derive(Debug, Default)]
pub(in crate::editor::lsp) struct Registry {
    registrations: FxHashMap<ServerName, LspServerConfig>,
    lists: FxHashMap<String, LanguageLists>,
}

/// One server a language's buffers attach to.
pub(in crate::editor::lsp) struct Planned<'a> {
    pub(in crate::editor::lsp) name: &'a ServerName,
    pub(in crate::editor::lsp) filter: FeatureFilter,
}

impl Registry {
    /// Registers `name`, replacing an earlier registration of the same name.
    /// `true` when it replaced one.
    pub(in crate::editor::lsp) fn register(
        &mut self,
        name: ServerName,
        config: LspServerConfig,
    ) -> bool {
        self.registrations.insert(name, config).is_some()
    }

    /// Removes `name`'s registration. Lists that name it keep the entry,
    /// which applies again if the name registers again. `false` when it
    /// was not registered.
    pub(in crate::editor::lsp) fn unregister(&mut self, name: &ServerName) -> bool {
        self.registrations.remove(name).is_some()
    }

    /// Sets (`Some`) or clears (`None`) one of `language`'s lists.
    pub(in crate::editor::lsp) fn set_list(
        &mut self,
        language: String,
        layer: ListLayer,
        entries: Option<Vec<ListEntry>>,
    ) {
        let lists = self.lists.entry(language).or_default();
        match layer {
            ListLayer::User => lists.user = entries,
            ListLayer::Default => lists.default = entries,
        }
    }

    pub(in crate::editor::lsp) fn contains(&self, name: &ServerName) -> bool {
        self.registrations.contains_key(name)
    }

    pub(in crate::editor::lsp) fn get(&self, name: &ServerName) -> Option<&LspServerConfig> {
        self.registrations.get(name)
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
    /// its user list, or of its default list while it has no user list,
    /// that name a registered server. An entry naming an unregistered
    /// server is skipped; a registered server no list names serves nothing.
    pub(in crate::editor::lsp) fn plan(&self, language: &str) -> Vec<Planned<'_>> {
        let Some(lists) = self.lists.get(language) else {
            return Vec::new();
        };
        let Some(entries) = lists.user.as_deref().or(lists.default.as_deref()) else {
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
