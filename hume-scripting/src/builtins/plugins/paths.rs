//! Where a plugin's files are on disk, and the `require` form that loads
//! one.

use steel::rerrs::SteelErr;

use crate::attribution::{EntryId, PluginId};
use crate::builtins::dirs::ScriptDirs;

/// The directory a plugin's files live in, given its id: `core:` plugins
/// under `runtime_dir`, `user/repo` plugins under `data_dir`, a local file's
/// own directory under `init_dir`. `None` when the relevant root is unset
/// (`HOME`/`APPDATA` unset for user plugins, no `init.scm` for a local file).
pub(super) fn plugin_dir_for_id(
    plugin_id: &PluginId,
    dirs: &ScriptDirs,
) -> Option<std::path::PathBuf> {
    match plugin_id {
        PluginId::Core(core_name) => dirs
            .runtime_dir
            .as_deref()
            .map(|rt| rt.join("plugins").join("core").join(core_name)),
        // When data_dir is None (HOME/APPDATA unset), user plugins cannot be
        // resolved, so return None rather than panicking.
        PluginId::User { user, repo } => dirs
            .data_dir
            .as_deref()
            .map(|d| d.join("plugins").join(user).join(repo)),
        PluginId::Local(path) => dirs
            .init_dir
            .as_deref()
            .and_then(|d| d.join(path).parent().map(std::path::Path::to_path_buf)),
    }
}

/// [`plugin_dir_for_id`] when that directory exists on disk.
pub(super) fn installed_plugin_dir(
    plugin_id: &PluginId,
    dirs: &ScriptDirs,
) -> Result<Option<std::path::PathBuf>, String> {
    match plugin_dir_for_id(plugin_id, dirs) {
        Some(dir) => Ok(path_exists(&dir)?.then_some(dir)),
        None => Ok(None),
    }
}

/// Probe a path's existence without a pre-flight `.exists()` (avoids TOCTOU).
/// `NotFound` → `Ok(false)`; other errors propagate.
pub(super) fn path_exists(path: &std::path::Path) -> Result<bool, String> {
    match std::fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("cannot stat path '{}': {e}", path.display())),
    }
}

/// The file `entry` loads, if it exists on disk. A local plugin's one entry is
/// the file its id names; an installed plugin's entry is a file in its
/// directory. Errors when a local plugin is named with no `init.scm` to
/// resolve it against.
pub(super) fn entry_path(
    dirs: &ScriptDirs,
    entry: &EntryId,
) -> Result<Option<std::path::PathBuf>, String> {
    let path = match &entry.plugin {
        PluginId::Local(rel) => match dirs.init_dir.as_deref() {
            Some(init_dir) => init_dir.join(rel),
            None => {
                return Err(format!(
                    "local plugin '{}' can only be declared from init.scm",
                    entry.plugin
                ));
            }
        },
        installed => match plugin_dir_for_id(installed, dirs) {
            Some(dir) => dir.join(entry.file.as_str()),
            None => return Ok(None),
        },
    };
    exact_file_exists(&path).map(|found| found.then_some(path))
}

/// Whether `path`'s directory holds an entry named exactly like `path`'s file
/// name. A plain stat would match a differently-cased name on case-insensitive
/// filesystems and not on others.
fn exact_file_exists(path: &std::path::Path) -> Result<bool, String> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(format!(
            "plugin entry '{}' has no file name",
            path.display()
        ));
    };
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("cannot read directory '{}': {e}", dir.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read directory '{}': {e}", dir.display()))?;
        if entry.file_name() == name {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Builds a quoted Steel string literal for `path` (`"…"`, backslashes
/// doubled so Windows paths like `C:\Users\…` survive embedding, since `\U` etc.
/// are invalid Steel escapes), or `None` if `path` contains `"`, which no
/// amount of escaping can embed in a Scheme string literal.
///
/// Also exposed as `crate::steel_path_literal` in test builds, for call sites (any
/// `.scm` source built by string interpolation from a real filesystem path:
/// `open-buffer!`, `picker!`, `require`, …) that would otherwise hand-roll
/// this same escaping per site.
pub fn steel_path_literal(path: &std::path::Path) -> Option<String> {
    let raw = path.to_string_lossy();
    if raw.contains('"') {
        return None;
    }
    let escaped = raw.replace('\\', "\\\\");
    Some(format!("\"{escaped}\""))
}

/// Builds `(require "<abs path>")`, rejecting a path containing `"` (which
/// can't be embedded in a Steel string literal) via [`steel_path_literal`].
///
/// `kind` names what `path` is, for the error message (`"plugin"` /
/// `"plugin manifest"`). `on_unquotable` runs (for its side effect only)
/// before the shared error is raised. `begin_lazy_activation` uses it to
/// mark the plugin `Failed` first; `load_plugin` has no plugin-stack state to
/// unwind yet, so passes a no-op.
pub(super) fn require_program_for_path(
    path: &std::path::Path,
    kind: &str,
    on_unquotable: impl FnOnce(),
) -> Result<String, SteelErr> {
    let Some(literal) = steel_path_literal(path) else {
        on_unquotable();
        steel::stop!(Generic =>
            "{} path contains '\"', so it cannot be embedded in require: {}", kind, path.display());
    };
    Ok(format!("(require {literal})"))
}
