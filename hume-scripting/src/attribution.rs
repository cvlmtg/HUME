//! Plugin attribution types: `PluginId`, `Owner`, and `PluginStack`.
//!
//! `PluginStack` tracks which plugin is currently executing; `current_owner()`
//! returns the owner to record at command-registration time. The result is
//! stored in `cmd_owners` and exposed to Steel via `(command-plugin …)`.

use std::fmt;
use std::hash::{Hash, Hasher};

/// A validated plugin identity: case-preserving for display and disk paths,
/// case-insensitive for equality and hashing.
///
/// Three valid forms:
/// - `Core(name)`: a bundled core plugin: `core:<name>`
/// - `User { user, repo }`: a third-party plugin: `<user>/<repo>`
/// - `Local(path)`: one `.scm` file beside `init.scm`: `./<path>.scm`, holding `<path>.scm`
///
/// `"SomeUser/CoolPlugin"` and `"someuser/coolplugin"` are equal on
/// case-insensitive filesystems (APFS, NTFS) while the original casing is
/// preserved for display and path construction.
#[derive(Debug, Clone)]
pub enum PluginId {
    Core(String),
    User { user: String, repo: String },
    Local(String),
}

impl PluginId {
    /// Parse and validate a plugin name string.
    ///
    /// Valid forms:
    /// - `core:<name>`: bundled core plugin
    /// - `<user>/<repo>`: third-party plugin (exactly one `/`)
    /// - `./<path>.scm`: a local file, relative to `init.scm`'s directory
    ///
    /// Segments must be non-empty, must not be `.` or `..`, and must not
    /// contain `/`, `\`, `"`, `:`, or NUL, ensuring the components are safe
    /// to use as filesystem path segments.  Validated by
    /// [`hume_platform::path::is_safe_segment`].
    ///
    /// Returns `Err(message)` for any other form.
    pub fn parse(name: &str) -> Result<Self, String> {
        if let Some(core_name) = name.strip_prefix("core:") {
            if !hume_platform::path::is_safe_segment(core_name) {
                return Err(format!(
                    "invalid plugin name '{name}': core name must be a non-empty path segment"
                ));
            }
            return Ok(PluginId::Core(core_name.to_string()));
        }
        if let Some(rel) = name.strip_prefix("./") {
            let mut segments = rel.rsplit('/');
            let last = segments.next().unwrap_or_default();
            let safe = segments.all(hume_platform::path::is_safe_segment);
            if !safe || EntryFile::parse(last).is_err() {
                return Err(format!(
                    "invalid local plugin path '{name}': expected './<file>.scm' relative to \
                     init.scm's directory, with no '.' or '..' segments"
                ));
            }
            return Ok(PluginId::Local(rel.to_string()));
        }
        if let Some((user, repo)) = name.split_once('/') {
            if repo.contains('/') {
                return Err(format!(
                    "invalid plugin name '{name}': expected user/repo with exactly one slash"
                ));
            }
            if !hume_platform::path::is_safe_segment(user)
                || !hume_platform::path::is_safe_segment(repo)
            {
                return Err(format!(
                    "invalid plugin name '{name}': user and repo must be non-empty valid path segments"
                ));
            }
            return Ok(PluginId::User {
                user: user.to_string(),
                repo: repo.to_string(),
            });
        }
        Err(format!(
            "invalid plugin name '{name}': expected 'core:<name>', '<user>/<repo>' or './<file>.scm'"
        ))
    }

    /// [`PluginId::parse`] for a plugin that lives in a plugin directory:
    /// a local file is an error.
    pub fn parse_installed(name: &str) -> Result<Self, String> {
        match Self::parse(name)? {
            PluginId::Local(_) => Err(format!(
                "'{name}' is a local file, not an installed plugin; declare it with \
                 (declare-plugin! \"{name}\" #:commands …)"
            )),
            id => Ok(id),
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, PluginId::Local(_))
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginId::Core(name) => write!(f, "core:{name}"),
            PluginId::User { user, repo } => write!(f, "{user}/{repo}"),
            PluginId::Local(path) => write!(f, "./{path}"),
        }
    }
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Owner::Core => f.write_str("hume"),
            Owner::User => f.write_str("user"),
            Owner::Plugin(entry) => fmt::Display::fmt(&entry.plugin, f),
        }
    }
}

/// Case-insensitive equality (ASCII fold; plugin names are ASCII by design).
///
/// `Core("PLUM") == Core("plum")`, `User { "Alice", "Bar" } == User { "alice", "bar" }`.
/// Different variants are never equal.
impl PartialEq for PluginId {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (PluginId::Core(a), PluginId::Core(b)) => a.eq_ignore_ascii_case(b),
            (PluginId::User { user: ua, repo: ra }, PluginId::User { user: ub, repo: rb }) => {
                ua.eq_ignore_ascii_case(ub) && ra.eq_ignore_ascii_case(rb)
            }
            (PluginId::Local(a), PluginId::Local(b)) => a.eq_ignore_ascii_case(b),
            _ => false,
        }
    }
}

impl Eq for PluginId {}

/// Hashes `s` with ASCII case folded, consistent with `eq_ignore_ascii_case`.
fn hash_folded<H: Hasher>(s: &str, state: &mut H) {
    for c in s.chars() {
        c.to_ascii_lowercase().hash(state);
    }
}

/// Hash must be consistent with `PartialEq`: equal IDs → equal hashes.
impl Hash for PluginId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Discriminant is hashed implicitly via the match: different variants
        // hash differently even if the inner strings happen to be the same.
        match self {
            PluginId::Core(name) => {
                0u8.hash(state);
                hash_folded(name, state);
            }
            PluginId::User { user, repo } => {
                1u8.hash(state);
                hash_folded(user, state);
                // Separator so ("ab","c") and ("a","bc") hash differently.
                // '/' cannot appear in a segment, so no legal id collides.
                '/'.hash(state);
                hash_folded(repo, state);
            }
            PluginId::Local(path) => {
                2u8.hash(state);
                hash_folded(path, state);
            }
        }
    }
}

/// A validated file name inside a plugin directory that a plugin entry loads:
/// one path segment ending in `.scm`. Compared exactly, case included, because
/// lookup requires the on-disk spelling on every filesystem.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntryFile(String);

impl EntryFile {
    const MAIN: &'static str = "plugin.scm";

    /// The entry every plugin has: `plugin.scm`.
    pub fn main() -> Self {
        Self(Self::MAIN.to_string())
    }

    /// Validate `name` as an entry file: a safe path segment with a non-empty
    /// stem and a `.scm` extension.
    pub fn parse(name: &str) -> Result<Self, String> {
        let stem_ok = name
            .strip_suffix(".scm")
            .is_some_and(|stem| !stem.is_empty());
        if !stem_ok || !hume_platform::path::is_safe_segment(name) {
            return Err(format!(
                "invalid plugin entry '{name}': expected a file name ending in .scm \
                 with no path separators"
            ));
        }
        Ok(Self(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_main(&self) -> bool {
        self.0 == Self::MAIN
    }
}

/// One loadable entry of a plugin: the unit that has its own activation
/// triggers, lifecycle state, and rollback scope.
///
/// Every plugin has a [`EntryFile::main`] entry; `#:entry` on
/// `declare-plugin!` adds others that share the plugin's directory, config
/// and identity. A local plugin has only its main entry, and that entry is
/// the file the id names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntryId {
    pub plugin: PluginId,
    pub file: EntryFile,
}

impl EntryId {
    pub fn new(plugin: PluginId, file: EntryFile) -> Self {
        Self { plugin, file }
    }

    /// The `plugin.scm` entry of `plugin`.
    pub fn main(plugin: PluginId) -> Self {
        Self::new(plugin, EntryFile::main())
    }
}

/// `core:p` for the main entry, `core:p (x.scm)` for a secondary one.
impl fmt::Display for EntryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.file.is_main() {
            fmt::Display::fmt(&self.plugin, f)
        } else {
            write!(f, "{} ({})", self.plugin, self.file.as_str())
        }
    }
}

/// The entity credited with a command registration.
///
/// - Stack empty → [`Owner::User`] (top-level `init.scm`)
/// - `stack.last()` → [`Owner::Plugin`] (inside a `(load-plugin! …)` / plugin body)
/// - [`Owner::Core`] is the fallback returned by `(command-plugin …)` for
///   built-in Rust commands that were never registered through Steel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Owner {
    Core,
    User,
    Plugin(EntryId),
}

/// The `CURRENT_PLUGIN` attribution stack.
///
/// Every Steel mutation is attributed to `stack.last()`: `Some(id)` means a
/// plugin body is executing; `None` means top-level `init.scm` (→ [`Owner::User`]).
/// Core state is never mutated through the scripting layer. [`Owner::Core`] is
/// only ever a *prior*, never the active attribution.
#[derive(Debug, Default, Clone)]
pub(crate) struct PluginStack {
    stack: Vec<EntryId>,
}

impl PluginStack {
    /// Push `id` onto the stack when entering a plugin body (via `begin_lazy_activation`).
    pub(crate) fn push(&mut self, id: EntryId) {
        self.stack.push(id);
    }

    /// Pop the top attribution when leaving a plugin body.
    ///
    /// Gracefully no-ops on an empty stack. Avoids panics on error-path
    /// cleanup where the stack may already be empty.
    pub(crate) fn pop(&mut self) {
        self.stack.pop();
    }

    /// Returns `true` if no plugin is currently executing.
    pub(crate) fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Current nesting depth: number of plugin bodies on the call stack.
    pub(crate) fn len(&self) -> usize {
        self.stack.len()
    }

    /// The [`Owner`] to attribute to the next mutation.
    pub(crate) fn current_owner(&self) -> Owner {
        match self.stack.last() {
            Some(id) => Owner::Plugin(id.clone()),
            None => Owner::User,
        }
    }

    /// The [`EntryId`] whose body is currently executing, if any.
    ///
    /// Valid during both eager (`load-plugin!`) and lazy (`declare-plugin!`,
    /// activated later) bodies, since both push here for the duration of the eval.
    pub(crate) fn current(&self) -> Option<&EntryId> {
        self.stack.last()
    }
}

#[cfg(test)]
mod tests;
