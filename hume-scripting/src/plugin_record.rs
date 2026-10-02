//! Plugin-level state: what `init.scm` has said about each plugin and what
//! looking it up found.
//!
//! [`PluginRecords`] answers every question about a plugin as a whole (its
//! config, whether `load-plugin!` ran, whether it was found, whether PLUM
//! should list it). [`crate::lazy::LazyRegistry`] holds the state of each
//! entry file of a plugin, which can differ from entry to entry.

use steel::rvals::SteelVal;

use super::attribution::PluginId;

/// How `init.scm` has named a plugin so far.
pub(crate) enum Request {
    /// Only `declare-plugin!` has named it.
    Declared,
    /// `load-plugin!` has run. The config is the latest one passed.
    Loaded { config: SteelVal },
}

/// What looking the plugin up on disk found. Absent until the first lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// Not installed. Reported when it is first set.
    Absent,
    /// Installed, with its entries in the lazy registry.
    Declared,
    /// Its `manifest.scm` raised.
    ManifestFailed,
}

pub(crate) struct PluginRecord {
    id: PluginId,
    request: Request,
    resolution: Option<Resolution>,
}

/// One record per plugin, in the order `init.scm` first named them.
#[derive(Default)]
pub(crate) struct PluginRecords(Vec<PluginRecord>);

impl PluginRecords {
    fn find(&self, id: &PluginId) -> Option<&PluginRecord> {
        self.0.iter().find(|r| r.id == *id)
    }

    /// Records that `init.scm` named `id`. The first spelling is kept, a
    /// `Loaded` request replaces any earlier one, and a `Declared` request
    /// never downgrades a `Loaded` one.
    pub(crate) fn note(&mut self, id: PluginId, request: Request) {
        match self.0.iter_mut().find(|r| r.id == id) {
            Some(record) => {
                if matches!(request, Request::Loaded { .. }) {
                    record.request = request;
                }
            }
            None => self.0.push(PluginRecord {
                id,
                request,
                resolution: None,
            }),
        }
    }

    /// The config `load-plugin!` last passed for `id`.
    pub(crate) fn config(&self, id: &PluginId) -> Option<&SteelVal> {
        match &self.find(id)?.request {
            Request::Loaded { config } => Some(config),
            Request::Declared => None,
        }
    }

    pub(crate) fn was_loaded(&self, id: &PluginId) -> bool {
        self.find(id)
            .is_some_and(|r| matches!(r.request, Request::Loaded { .. }))
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
        let record = self.0.iter_mut().find(|r| r.id == *id).ok_or_else(|| {
            format!("plugin '{id}' was never named by load-plugin! or declare-plugin!")
        })?;
        Ok(record.resolution.replace(resolution))
    }

    /// Plugins that are absent or whose manifest failed. Neither has an entry
    /// row, so `:plugin-status` lists each on a row of its own.
    pub(crate) fn unresolved_rows(&self) -> impl Iterator<Item = (&PluginId, Resolution)> {
        self.0.iter().filter_map(|r| match r.resolution {
            Some(res @ (Resolution::Absent | Resolution::ManifestFailed)) => Some((&r.id, res)),
            _ => None,
        })
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
