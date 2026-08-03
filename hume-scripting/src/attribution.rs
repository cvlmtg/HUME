//! Plugin attribution types: `PluginId`, `Owner`, and `PluginStack`.
//!
//! `PluginStack` tracks which plugin is currently executing; `current_owner()`
//! returns the owner to record at command-registration time. The result is
//! stored in `cmd_owners` and exposed to Steel via `(command-plugin …)`.

use std::fmt;
use std::hash::{Hash, Hasher};

// ── PluginId ──────────────────────────────────────────────────────────────────

/// A validated plugin identity: case-preserving for display and disk paths,
/// case-insensitive for equality and hashing.
///
/// Two valid forms:
/// - `Core(name)` — a bundled core plugin: `core:<name>`
/// - `User { user, repo }` — a third-party plugin: `<user>/<repo>`
///
/// `"SomeUser/CoolPlugin"` and `"someuser/coolplugin"` are equal on
/// case-insensitive filesystems (APFS, NTFS) while the original casing is
/// preserved for display and path construction.
#[derive(Debug, Clone)]
pub enum PluginId {
    Core(String),
    User { user: String, repo: String },
}

impl PluginId {
    /// Parse and validate a plugin name string.
    ///
    /// Valid forms:
    /// - `core:<name>` — bundled core plugin
    /// - `<user>/<repo>` — third-party plugin (exactly one `/`)
    ///
    /// Segments must be non-empty, must not be `.` or `..`, and must not
    /// contain `/`, `\`, `"`, `:`, or NUL — ensuring the components are safe
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
            "invalid plugin name '{name}': expected 'core:<name>' or '<user>/<repo>'"
        ))
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginId::Core(name) => write!(f, "core:{name}"),
            PluginId::User { user, repo } => write!(f, "{user}/{repo}"),
        }
    }
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Owner::Core => f.write_str("hume"),
            Owner::User => f.write_str("user"),
            Owner::Plugin(pid) => fmt::Display::fmt(pid, f),
        }
    }
}

/// Case-insensitive equality (ASCII fold — plugin names are ASCII by design).
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
            _ => false,
        }
    }
}

impl Eq for PluginId {}

/// Hash must be consistent with `PartialEq`: equal IDs → equal hashes.
impl Hash for PluginId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Discriminant is hashed implicitly via the match — different variants
        // hash differently even if the inner strings happen to be the same.
        match self {
            PluginId::Core(name) => {
                0u8.hash(state);
                for c in name.chars() {
                    c.to_ascii_lowercase().hash(state);
                }
            }
            PluginId::User { user, repo } => {
                1u8.hash(state);
                for c in user.chars() {
                    c.to_ascii_lowercase().hash(state);
                }
                // Separator so ("ab","c") and ("a","bc") hash differently.
                // '/' cannot appear in a segment, so no legal id collides.
                '/'.hash(state);
                for c in repo.chars() {
                    c.to_ascii_lowercase().hash(state);
                }
            }
        }
    }
}

// ── Owner ─────────────────────────────────────────────────────────────────────

/// The entity credited with a command registration.
///
/// - Stack empty → [`Owner::User`] (top-level `init.scm`)
/// - `stack.last()` → [`Owner::Plugin`] (inside a `(load-plugin …)` / plugin body)
/// - [`Owner::Core`] is the fallback returned by `(command-plugin …)` for
///   built-in Rust commands that were never registered through Steel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Owner {
    Core,
    User,
    Plugin(PluginId),
}

// ── PluginStack ───────────────────────────────────────────────────────────────

/// The `CURRENT_PLUGIN` attribution stack.
///
/// Every Steel mutation is attributed to `stack.last()`: `Some(id)` means a
/// plugin body is executing; `None` means top-level `init.scm` (→ [`Owner::User`]).
/// Core state is never mutated through the scripting layer — [`Owner::Core`] is
/// only ever a *prior*, never the active attribution.
#[derive(Debug, Default, Clone)]
pub(crate) struct PluginStack {
    stack: Vec<PluginId>,
}

impl PluginStack {
    /// Push `id` onto the stack when entering a plugin body (via `begin_lazy_activation`).
    pub(crate) fn push(&mut self, id: PluginId) {
        self.stack.push(id);
    }

    /// Pop the top attribution when leaving a plugin body.
    ///
    /// Gracefully no-ops on an empty stack — avoids panics on error-path
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

    /// The [`PluginId`] whose body is currently executing, if any.
    ///
    /// Used by `(plugin-config)` to look up the caller's own `#:config` value —
    /// valid during both eager (`load-plugin`) and lazy (`declare-plugin`,
    /// activated later) bodies, since both push here for the duration of the eval.
    pub(crate) fn current(&self) -> Option<&PluginId> {
        self.stack.last()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
