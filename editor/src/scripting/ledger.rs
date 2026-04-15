//! Ownership ledger and `CURRENT_PLUGIN` attribution stack.
//!
//! Every Steel mutation is attributed to an [`Owner`] derived from the
//! [`PluginStack`]. The [`LedgerStack`] records those mutations so that
//! plugins can be cleanly unloaded in any order, with correct prior-chaining
//! when multiple plugins have touched the same key.
//!
//! ## The rewrite algorithm (STEEL.md §Ledgers)
//!
//! When plugin X is unloaded, for each key X touched:
//! - If a later plugin Y also touched the key (Y is the live owner):
//!   rewrite Y's ledger entry so its `prior_value`/`prior_owner` point to
//!   what existed *before X* — effectively splicing X out of the chain.
//!   The live value is unchanged (Y still owns it).
//! - If no later plugin touched the key (X is the live owner):
//!   return the entry in `unload`'s result; the caller restores the prior
//!   value to the live registry/keymap/settings.

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
pub(crate) enum PluginId {
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
    /// contain `/`, `\`, or NUL — ensuring the components are safe to use as
    /// filesystem path segments.
    ///
    /// Returns `Err(message)` for any other form.
    pub(crate) fn parse(name: &str) -> Result<Self, String> {
        if let Some(core_name) = name.strip_prefix("core:") {
            if !is_valid_segment(core_name) {
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
            if !is_valid_segment(user) || !is_valid_segment(repo) {
                return Err(format!(
                    "invalid plugin name '{name}': user and repo must be non-empty valid path segments"
                ));
            }
            return Ok(PluginId::User { user: user.to_string(), repo: repo.to_string() });
        }
        Err(format!("invalid plugin name '{name}': expected 'core:<name>' or '<user>/<repo>'"))
    }
}

/// A valid path segment is non-empty, not `.` or `..`, and contains no
/// `/`, `\`, or NUL.  Dots elsewhere are permitted (e.g. `v1.2`).
///
/// Used by [`PluginId::parse`] to validate each name component before storing
/// it — ensures all stored strings are safe to use as filesystem path segments.
fn is_valid_segment(s: &str) -> bool {
    if s.is_empty() || s == "." || s == ".." {
        return false;
    }
    s.chars().all(|c| c != '/' && c != '\\' && c != '\0')
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginId::Core(name)          => write!(f, "core:{name}"),
            PluginId::User { user, repo } => write!(f, "{user}/{repo}"),
        }
    }
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Owner::Core        => f.write_str("hume"),
            Owner::User        => f.write_str("user"),
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
            (PluginId::Core(a), PluginId::Core(b)) =>
                a.eq_ignore_ascii_case(b),
            (PluginId::User { user: ua, repo: ra }, PluginId::User { user: ub, repo: rb }) =>
                ua.eq_ignore_ascii_case(ub) && ra.eq_ignore_ascii_case(rb),
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
                for c in name.chars() { c.to_ascii_lowercase().hash(state); }
            }
            PluginId::User { user, repo } => {
                1u8.hash(state);
                for c in user.chars() { c.to_ascii_lowercase().hash(state); }
                for c in repo.chars() { c.to_ascii_lowercase().hash(state); }
            }
        }
    }
}

// ── Owner ─────────────────────────────────────────────────────────────────────

/// The entity that owns a HUME binding at any given moment.
///
/// Derived from the [`PluginStack`] at mutation time:
/// - Stack empty → [`Owner::User`] (top-level `init.scm` mutation)
/// - `stack.last()` → [`Owner::Plugin`] (inside a `(load-plugin …)` body)
/// - [`Owner::Core`] appears only as a `prior_owner` in ledger entries —
///   core defaults are never mutated through the scripting layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Owner {
    Core,
    User,
    Plugin(PluginId),
}

// ── LedgerEntry ───────────────────────────────────────────────────────────────

/// One mutation recorded by a plugin, storing only what is needed to undo it.
///
/// The *new* value lives in the live registry/keymap/settings — the ledger
/// never duplicates it. Only the prior state is recorded here.
///
/// Note: `prior_value` is a serialized `String` in Phase 2. Phase 3 replaces
/// this with a typed `PriorValue` enum (one variant per mutable subsystem).
#[derive(Debug, Clone)]
pub(crate) struct LedgerEntry {
    /// The key that was changed (e.g. `"f"` for a keybind, `"tab-width"` for a
    /// setting). Typed as `String` for Phase 2; will become a `LedgerKey` enum
    /// in Phase 3 when builtins wire into live registries.
    pub(crate) key: String,
    /// The serialized value that was live before this mutation.
    pub(crate) prior_value: String,
    /// The owner of the binding before this mutation.
    pub(crate) prior_owner: Owner,
}

// ── Ledger ────────────────────────────────────────────────────────────────────

/// All mutations made by a single plugin, in the order they were recorded.
///
/// At most one entry exists per key — subsequent mutations of the same key by
/// the same plugin are silently deduplicated because the *first* entry already
/// captures "what existed before this plugin touched it."
#[derive(Debug, Clone)]
pub(crate) struct Ledger {
    pub(crate) plugin: PluginId,
    pub(crate) entries: Vec<LedgerEntry>,
}

// ── LedgerStack ───────────────────────────────────────────────────────────────

/// The global ordered list of plugin ledgers, oldest first.
///
/// Each plugin gets exactly one [`Ledger`], created on its first mutation and
/// removed when the plugin is unloaded. Activation order is preserved: the
/// ledger for the first-activated plugin is always at index 0.
#[derive(Debug, Default, Clone)]
pub(crate) struct LedgerStack {
    /// Ordered by plugin activation time (oldest first).
    pub(crate) ledgers: Vec<Ledger>,
}

impl LedgerStack {
    /// Record a mutation by `plugin` on `key`.
    ///
    /// Creates a new [`Ledger`] for `plugin` if this is its first mutation.
    /// **Deduplicates by key within a plugin's ledger:** if `plugin` has already
    /// recorded a mutation for `key`, this call is a no-op — the first entry
    /// already captures the pre-plugin state and that is all we need to restore.
    pub(crate) fn record(
        &mut self,
        plugin: &PluginId,
        key: String,
        prior_owner: Owner,
        prior_value: String,
    ) {
        if let Some(ledger) = self.ledgers.iter_mut().find(|l| l.plugin == *plugin) {
            // Only record the first mutation per key for this plugin.
            if !ledger.entries.iter().any(|e| e.key == key) {
                ledger.entries.push(LedgerEntry { key, prior_value, prior_owner });
            }
        } else {
            self.ledgers.push(Ledger {
                plugin: plugin.clone(),
                entries: vec![LedgerEntry { key, prior_value, prior_owner }],
            });
        }
    }

    /// Return the current live owner of `key` (last-writer-wins).
    ///
    /// Scans from newest to oldest. Returns [`Owner::Core`] if no plugin has
    /// ever touched `key`.
    pub(crate) fn owner_of(&self, key: &str) -> Owner {
        for ledger in self.ledgers.iter().rev() {
            if ledger.entries.iter().any(|e| e.key == key) {
                return Owner::Plugin(ledger.plugin.clone());
            }
        }
        Owner::Core
    }

    /// Unload `plugin`, applying the rewrite-prior algorithm from STEEL.md §Ledgers.
    ///
    /// For each entry in `plugin`'s ledger:
    /// - **Later writer exists** (another plugin loaded after `plugin` also
    ///   touched the same key): rewrite that later entry's `prior_value` /
    ///   `prior_owner` to this entry's values, splicing `plugin` out of the
    ///   chain. The live value is unchanged.
    /// - **No later writer** (`plugin` is the live owner): add the entry to the
    ///   returned `Vec`. The caller must restore `prior_value` to the live
    ///   registry / keymap / settings.
    ///
    /// "Later" means a ledger at a higher index than `plugin`'s — i.e., a plugin
    /// that was activated after `plugin`. The first such entry for a given key is
    /// the one rewritten (closest successor gets the correct prior chain).
    ///
    /// Does nothing and returns an empty `Vec` if `plugin` has no ledger.
    pub(crate) fn unload(&mut self, plugin: &PluginId) -> Vec<LedgerEntry> {
        let Some(pos) = self.ledgers.iter().position(|l| l.plugin == *plugin) else {
            return Vec::new();
        };

        let removed = self.ledgers.remove(pos);
        let mut to_restore = Vec::new();

        for entry in removed.entries {
            // After removing at `pos`, ledgers that were after `plugin` are
            // now at indices `pos..` — those are the "later" plugins.
            let later = self.ledgers[pos..]
                .iter_mut()
                .flat_map(|l| l.entries.iter_mut())
                .find(|e| e.key == entry.key);

            if let Some(later_entry) = later {
                // Splice `plugin` out: Y's prior now points to what existed
                // before plugin (X's prior), so when Y is later unloaded it
                // restores the correct baseline value.
                later_entry.prior_owner = entry.prior_owner;
                later_entry.prior_value = entry.prior_value;
            } else {
                // `plugin` was the live owner — caller must restore.
                to_restore.push(entry);
            }
        }

        to_restore
    }
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
    /// Push `id` onto the stack when entering a `(load-plugin …)` body.
    pub(crate) fn push(&mut self, id: PluginId) {
        self.stack.push(id);
    }

    /// Pop the top attribution when leaving a `(load-plugin …)` body.
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

    /// The [`Owner`] to attribute to the next mutation.
    pub(crate) fn current_owner(&self) -> Owner {
        match self.stack.last() {
            Some(id) => Owner::Plugin(id.clone()),
            None => Owner::User,
        }
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
        assert!(PluginId::parse("core:../evil").is_err());  // dotdot core name
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
        assert_eq!(pid("SomeUser/CoolPlugin").to_string(), "SomeUser/CoolPlugin");
        assert_eq!(pid("core:helix-surround").to_string(), "core:helix-surround");
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

    // ── LedgerStack — basic ───────────────────────────────────────────────────

    #[test]
    fn owner_of_returns_core_for_untouched_key() {
        let stack = LedgerStack::default();
        assert_eq!(stack.owner_of("f"), Owner::Core);
    }

    #[test]
    fn owner_of_returns_last_writer() {
        let mut stack = LedgerStack::default();
        let x = pid("user/x");
        let y = pid("user/y");
        stack.record(&x, "f".into(), Owner::Core, "find-char".into());
        stack.record(&y, "f".into(), Owner::Plugin(x.clone()), "foo".into());
        assert_eq!(stack.owner_of("f"), Owner::Plugin(y));
    }

    #[test]
    fn record_deduplicates_key_within_plugin() {
        let mut stack = LedgerStack::default();
        let x = pid("user/x");
        // First record — should be stored.
        stack.record(&x, "f".into(), Owner::Core, "find-char".into());
        // Second record for the same key by the same plugin — should be ignored.
        stack.record(&x, "f".into(), Owner::Plugin(x.clone()), "foo".into());
        let ledger = stack.ledgers.iter().find(|l| l.plugin == x).unwrap();
        assert_eq!(ledger.entries.len(), 1);
        assert_eq!(ledger.entries[0].prior_value, "find-char");
    }

    // ── LedgerStack — unload ─────────────────────────────────────────────────

    #[test]
    fn unload_no_later_writer_returns_entry_to_restore() {
        let mut stack = LedgerStack::default();
        let x = pid("user/x");
        stack.record(&x, "tab-size".into(), Owner::Core, "2".into());

        let to_restore = stack.unload(&x);

        assert_eq!(to_restore.len(), 1);
        assert_eq!(to_restore[0].key, "tab-size");
        assert_eq!(to_restore[0].prior_value, "2");
        assert_eq!(to_restore[0].prior_owner, Owner::Core);
        assert!(stack.ledgers.is_empty(), "X's ledger must be removed");
    }

    /// The canonical scenario from STEEL.md §Ledgers:
    ///
    /// X binds `f`, Y rebinds `f`. Unloading X must *not* restore `f` to
    /// `find-char` (Y still owns it). Instead, X's entry is spliced into Y's
    /// prior chain so that when Y is eventually unloaded, it will restore
    /// `find-char` directly.
    #[test]
    fn unload_later_writer_rewrites_prior_not_live() {
        let mut stack = LedgerStack::default();
        let x = pid("user/x");
        let y = pid("user/y");
        // X: f was find-char / Core
        stack.record(&x, "f".into(), Owner::Core, "find-char".into());
        // Y: f was foo / X
        stack.record(&y, "f".into(), Owner::Plugin(x.clone()), "foo".into());

        let to_restore = stack.unload(&x);

        assert!(to_restore.is_empty(), "f is still live under Y");
        let y_entry = stack.ledgers.iter()
            .find(|l| l.plugin == y).unwrap()
            .entries.iter()
            .find(|e| e.key == "f").unwrap();
        assert_eq!(y_entry.prior_owner, Owner::Core);
        assert_eq!(y_entry.prior_value, "find-char");
    }

    #[test]
    fn unload_mixed_keys_separates_restore_from_rewrite() {
        let mut stack = LedgerStack::default();
        let x = pid("user/x");
        let y = pid("user/y");
        // X owns both; Y later takes `f` but never touches `tab-size`.
        stack.record(&x, "f".into(), Owner::Core, "find-char".into());
        stack.record(&x, "tab-size".into(), Owner::Core, "2".into());
        stack.record(&y, "f".into(), Owner::Plugin(x.clone()), "foo".into());

        let to_restore = stack.unload(&x);

        // `f` is owned by Y — not in to_restore.
        // `tab-size` is owned by X — must be restored.
        assert_eq!(to_restore.len(), 1);
        assert_eq!(to_restore[0].key, "tab-size");
    }

    #[test]
    fn unload_unknown_plugin_is_noop() {
        let mut stack = LedgerStack::default();
        let x = pid("user/x");
        let y = pid("user/y");
        stack.record(&x, "f".into(), Owner::Core, "find-char".into());

        let to_restore = stack.unload(&y); // y has no ledger

        assert!(to_restore.is_empty());
        assert_eq!(stack.ledgers.len(), 1, "X's ledger must be untouched");
    }

    /// Three-plugin chain: X → Y → Z all bind `f`.
    /// Unloading X rewrites Y's prior to Core.
    /// Then unloading Y rewrites Z's prior to Core.
    #[test]
    fn three_plugin_chain_rewrites_correctly() {
        let mut stack = LedgerStack::default();
        let x = pid("user/x");
        let y = pid("user/y");
        let z = pid("user/z");
        stack.record(&x, "f".into(), Owner::Core, "find-char".into());
        stack.record(&y, "f".into(), Owner::Plugin(x.clone()), "foo".into());
        stack.record(&z, "f".into(), Owner::Plugin(y.clone()), "bar".into());

        // Unload X — Y still owns f; Y's prior must now point to Core.
        assert!(stack.unload(&x).is_empty());
        let y_entry = stack.ledgers.iter()
            .find(|l| l.plugin == y).unwrap()
            .entries.iter().find(|e| e.key == "f").unwrap();
        assert_eq!(y_entry.prior_owner, Owner::Core);
        assert_eq!(y_entry.prior_value, "find-char");

        // Unload Y — Z still owns f; Z's prior must now point to Core.
        assert!(stack.unload(&y).is_empty());
        let z_entry = stack.ledgers.iter()
            .find(|l| l.plugin == z).unwrap()
            .entries.iter().find(|e| e.key == "f").unwrap();
        assert_eq!(z_entry.prior_owner, Owner::Core);
        assert_eq!(z_entry.prior_value, "find-char");
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
