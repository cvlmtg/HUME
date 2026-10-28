//! Plugin-level state: what `init.scm` has said about each plugin and what
//! looking it up found.
//!
//! [`PluginRecords`] answers every question about a plugin as a whole (its
//! config, whether `load-plugin!` ran, which entries `declare-plugin!` named,
//! whether it was found, whether PLUM should list it).
//! [`crate::lazy::LazyRegistry`] holds the state of each entry file of a
//! plugin that is installed, which can differ from entry to entry.

use steel::rvals::SteelVal;

use super::Rooted;
use super::attribution::{EntryFile, EntryId, PluginId};

/// What looking the plugin up found, beyond the entries the lazy registry
/// holds. Absent until the first lookup that found one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// Not installed. Reported when it is first set.
    Absent,
    /// Its `manifest.scm` raised.
    ManifestFailed,
}

impl Resolution {
    /// The `:plugin-status` state column for a plugin with no entry row.
    fn label(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::ManifestFailed => "failed",
        }
    }
}

/// What one `load-plugin!` call passed.
pub(crate) struct LoadCall {
    pub(crate) config: Rooted,
    /// The `#:ref` PLUM checks the plugin's clone out at.
    pub(crate) git_ref: Option<GitRef>,
}

/// A tag, a branch or a commit id, full or abbreviated: a name
/// `git check-ref-format --allow-onelevel` accepts, so a ref git would refuse
/// fails when `init.scm` loads rather than at install time.
pub(crate) struct GitRef(String);

impl GitRef {
    /// `None` for a name git refuses. That includes a leading `-`, which git
    /// refuses for a tag or branch name, so such a ref could never resolve.
    pub(crate) fn parse(name: String) -> Option<Self> {
        let forbidden = |c: char| c.is_ascii_control() || " ~^:?*[\\".contains(c);
        let valid = name != "@"
            && !name.starts_with('-')
            && !name.contains(forbidden)
            && !name.contains("..")
            && !name.contains("@{")
            && !name.ends_with('.')
            && name
                .split('/')
                .all(|part| !part.is_empty() && !part.starts_with('.') && !part.ends_with(".lock"));
        valid.then_some(Self(name))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

pub(crate) struct PluginRecord {
    id: PluginId,
    /// Set once `load-plugin!` has run; the latest call's arguments.
    load: Option<LoadCall>,
    /// The entries `declare-plugin!` declared, whether or not the plugin is
    /// installed.
    declared: Vec<EntryFile>,
    resolution: Option<Resolution>,
}

/// One record per plugin, in the order `init.scm` first named them.
#[derive(Default)]
pub(crate) struct PluginRecords(Vec<PluginRecord>);

impl PluginRecords {
    fn find(&self, id: &PluginId) -> Option<&PluginRecord> {
        self.0.iter().find(|r| r.id == *id)
    }

    fn find_mut(&mut self, id: &PluginId) -> Option<&mut PluginRecord> {
        self.0.iter_mut().find(|r| r.id == *id)
    }

    /// Records that `init.scm` named `id`. The first spelling is kept. A
    /// `Some` call (from `load-plugin!`) replaces any earlier one; `None`
    /// (from `declare-plugin!`) never removes one.
    pub(crate) fn note(&mut self, id: PluginId, load: Option<LoadCall>) {
        match self.find_mut(&id) {
            Some(record) => {
                if load.is_some() {
                    record.load = load;
                }
            }
            None => self.0.push(PluginRecord {
                id,
                load,
                declared: Vec::new(),
                resolution: None,
            }),
        }
    }

    /// Records that `declare-plugin!` declared `entry`, noting its plugin as
    /// [`Self::note`] does.
    pub(crate) fn note_declared(&mut self, entry: EntryId) {
        self.note(entry.plugin.clone(), None);
        self.find_mut(&entry.plugin)
            .expect("noted above")
            .declared
            .push(entry.file);
    }

    /// Whether `declare-plugin!` already declared `entry`.
    pub(crate) fn declared(&self, entry: &EntryId) -> bool {
        self.find(&entry.plugin)
            .is_some_and(|r| r.declared.contains(&entry.file))
    }

    fn load(&self, id: &PluginId) -> Option<&LoadCall> {
        self.find(id)?.load.as_ref()
    }

    /// The config `load-plugin!` last passed for `id`.
    pub(crate) fn config(&self, id: &PluginId) -> Option<&SteelVal> {
        Some(self.load(id)?.config.value())
    }

    /// The `#:ref` `load-plugin!` last passed for `id`.
    pub(crate) fn git_ref(&self, id: &PluginId) -> Option<&str> {
        self.load(id)?.git_ref.as_ref().map(GitRef::as_str)
    }

    pub(crate) fn was_loaded(&self, id: &PluginId) -> bool {
        self.load(id).is_some()
    }

    pub(crate) fn resolution(&self, id: &PluginId) -> Option<Resolution> {
        self.find(id)?.resolution
    }

    /// Sets the resolution of a plugin that was already [`noted`](Self::note)
    /// and returns the one it replaces. Errors for a plugin never noted.
    pub(crate) fn resolve(
        &mut self,
        id: &PluginId,
        resolution: Resolution,
    ) -> Result<Option<Resolution>, String> {
        let record = self.find_mut(id).ok_or_else(|| {
            format!("plugin '{id}' was never named by load-plugin! or declare-plugin!")
        })?;
        Ok(record.resolution.replace(resolution))
    }

    /// Plugins that are absent or whose manifest failed, with their
    /// `:plugin-status` state. Neither need have an entry row, so the status
    /// table lists each on a row of its own.
    pub(crate) fn entryless_rows(&self) -> impl Iterator<Item = (&PluginId, &'static str)> {
        self.0
            .iter()
            .filter_map(|r| Some((&r.id, r.resolution?.label())))
    }

    /// The names PLUM lists, in the order they were first typed. A local file
    /// is never installed, so it is not listed.
    pub(crate) fn plum_names(&self) -> impl Iterator<Item = String> {
        self.0
            .iter()
            .filter(|r| !r.id.is_local())
            .map(|r| r.id.to_string())
    }
}

#[cfg(test)]
mod tests;
