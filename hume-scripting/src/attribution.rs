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
    /// Push `id` onto the stack when entering a plugin body (via `activate_plugin`).
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
mod tests {
    use super::*;

    /// Convenience: parse a known-valid plugin name in tests.
    fn pid(s: &str) -> PluginId {
        PluginId::parse(s).expect("valid plugin id in test")
    }

    // ── PluginId::parse ───────────────────────────────────────────────────────

    #[test]
    fn parse_core_plugin() {
        let id = PluginId::parse("core:plum").unwrap();
        assert!(matches!(id, PluginId::Core(ref n) if n == "plum"));
        assert_eq!(id.to_string(), "core:plum");
    }

    #[test]
    fn parse_user_plugin() {
        let id = PluginId::parse("alice/bar").unwrap();
        assert!(matches!(&id, PluginId::User { user, repo } if user == "alice" && repo == "bar"));
        assert_eq!(id.to_string(), "alice/bar");
    }

    #[test]
    fn parse_just_a_name_errors() {
        assert!(PluginId::parse("just-a-name").is_err());
    }

    #[test]
    fn parse_dotdot_segment_errors() {
        assert!(PluginId::parse("../evil").is_err());
        assert!(PluginId::parse("core:../evil").is_err()); // dotdot core name
        assert!(PluginId::parse("core:..").is_err());
        assert!(PluginId::parse("../evil").is_err());
    }

    #[test]
    fn parse_empty_segment_errors() {
        assert!(PluginId::parse("core:").is_err());
        assert!(PluginId::parse("/repo").is_err());
        assert!(PluginId::parse("user/").is_err());
    }

    #[test]
    fn parse_too_many_slashes_errors() {
        assert!(PluginId::parse("a/b/c").is_err());
    }

    #[test]
    fn parse_quote_in_segment_errors() {
        assert!(PluginId::parse("core:a\"b").is_err());
        assert!(PluginId::parse("a\"b/repo").is_err());
        assert!(PluginId::parse("user/a\"b").is_err());
    }

    // ── PluginId equality and hashing ─────────────────────────────────────────

    #[test]
    fn plugin_id_case_insensitive_equality() {
        assert_eq!(pid("foo/bar"), pid("FOO/BAR"));
        assert_eq!(pid("core:plum"), pid("core:PLUM"));
        assert_ne!(pid("foo/bar"), pid("foo/baz"));
        // Different variants are never equal.
        assert_ne!(pid("core:bar"), pid("foo/bar"));
    }

    #[test]
    fn plugin_id_preserves_case_in_display() {
        assert_eq!(
            pid("SomeUser/CoolPlugin").to_string(),
            "SomeUser/CoolPlugin"
        );
        assert_eq!(
            pid("core:helix-surround").to_string(),
            "core:helix-surround"
        );
    }

    #[test]
    fn plugin_id_equal_ids_have_equal_hashes() {
        use std::collections::hash_map::DefaultHasher;
        let hash_of = |id: &PluginId| {
            let mut h = DefaultHasher::new();
            id.hash(&mut h);
            h.finish()
        };
        assert_eq!(hash_of(&pid("Foo/Bar")), hash_of(&pid("foo/bar")));
        assert_eq!(hash_of(&pid("core:PLUM")), hash_of(&pid("core:plum")));
    }

    // ── PluginStack ──────────────────────────────────────────────────────────

    #[test]
    fn plugin_stack_empty_is_user() {
        let stack = PluginStack::default();
        assert_eq!(stack.current_owner(), Owner::User);
    }

    #[test]
    fn plugin_stack_push_makes_plugin_owner() {
        let mut stack = PluginStack::default();
        let x = pid("user/x");
        stack.push(x.clone());
        assert_eq!(stack.current_owner(), Owner::Plugin(x));
    }

    #[test]
    fn plugin_stack_pop_returns_to_user() {
        let mut stack = PluginStack::default();
        stack.push(pid("user/x"));
        stack.pop();
        assert_eq!(stack.current_owner(), Owner::User);
    }

    #[test]
    fn plugin_stack_nested_plugins() {
        let mut stack = PluginStack::default();
        let x = pid("user/x");
        let y = pid("user/y");
        stack.push(x);
        stack.push(y.clone());
        assert_eq!(stack.current_owner(), Owner::Plugin(y));
        stack.pop();
        assert_eq!(stack.current_owner(), Owner::Plugin(pid("user/x")));
    }

    #[test]
    fn plugin_stack_pop_on_empty_is_noop() {
        let mut stack = PluginStack::default();
        stack.pop(); // must not panic
        assert_eq!(stack.current_owner(), Owner::User);
    }
}
