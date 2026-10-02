//! Plugin-level state: what `init.scm` has said about each plugin and what
//! looking it up found.
//!
//! [`PluginRecords`] answers every question about a plugin as a whole (its
//! config, whether `load-plugin!` ran, whether it was found, whether PLUM
//! should list it). [`crate::lazy::LazyRegistry`] holds the state of each
//! entry file of a plugin, which can differ from entry to entry.

use steel::rvals::SteelVal;

use super::attribution::PluginId;

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

pub(crate) struct PluginRecord {
    id: PluginId,
    /// Set once `load-plugin!` has run; the latest config it passed.
    config: Option<SteelVal>,
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
    /// `Some` config (from `load-plugin!`) replaces any earlier one; `None`
    /// (from `declare-plugin!`) never removes one.
    pub(crate) fn note(&mut self, id: PluginId, config: Option<SteelVal>) {
        match self.find_mut(&id) {
            Some(record) => {
                if config.is_some() {
                    record.config = config;
                }
            }
            None => self.0.push(PluginRecord {
                id,
                config,
                resolution: None,
            }),
        }
    }

    /// The config `load-plugin!` last passed for `id`.
    pub(crate) fn config(&self, id: &PluginId) -> Option<&SteelVal> {
        self.find(id)?.config.as_ref()
    }

    pub(crate) fn was_loaded(&self, id: &PluginId) -> bool {
        self.config(id).is_some()
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
