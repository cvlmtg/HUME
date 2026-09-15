//! Trie-based keymap for Normal and Insert modes.
//!
//! # Architecture
//!
//! Each mode has a [`KeyTrie`] that maps [`KeyEvent`] sequences to
//! [`KeymapCommand`] values. The trie supports:
//!
//! - **Single-key bindings**: most keys (h/j/k/l, d, y, etc.)
//! - **Multi-key sequences**: `m` → `i`/`a` → object char (text objects);
//!   `g` → second key (goto commands).
//! - **Wait-for-char bindings**: f/t/F/T/r consume the *next* character as
//!   an argument rather than a fixed trie branch.
//!
//! The dispatcher in `mappings/execute.rs` walks the trie on each keypress,
//! accumulates a numeric count prefix, and executes [`KeymapCommand`] values
//! via the [`crate::editor::registry::CommandRegistry`].
//!
//! # Extend-mode duality
//!
//! The keymap stores only base command names. Extend mode is resolved at
//! dispatch time via a `MotionMode` parameter — no separate extend-variant
//! command names are needed. The sparse `extend` trie in [`Keymap`] holds
//! per-key overrides that take priority in extend mode before falling
//! through to the normal trie with `extend = true`. It ships empty by
//! default — plugins (e.g. `core:vim-keybind`'s `o → flip-selections`) are
//! the usual source of entries.
//!
//! # Wait-char bindings
//!
//! Keys like f/t/F/T/r produce a [`WaitCharPending`] that stores the command
//! name to dispatch. When the next character arrives, the dispatcher stores it
//! in `Editor.pending_char` and dispatches the named command. Extend-mode
//! resolution happens at char-consumption time via the `ctrl_extend` flag.

mod canonical;
#[macro_use]
mod defaults;
use defaults::{default_extend_keymap, default_insert_keymap, default_normal_keymap};

pub(in crate::editor) use canonical::CanonicalKey;

use rustc_hash::FxHashMap;
use std::borrow::Cow;

use termina::event::KeyEvent;

// ── WaitCharPending ───────────────────────────────────────────────────────────

/// State stored on the editor after a wait-char key (f/t/F/T/r).
///
/// On the next keypress the dispatcher stores the character in
/// `Editor.pending_char` and dispatches `cmd_name`. Extend-mode resolution
/// happens at char-consumption time via the registry.
#[derive(Debug, Clone)]
pub(crate) struct WaitCharPending {
    pub cmd_name: Cow<'static, str>,
    /// Set to `true` when this wait-char was triggered via Ctrl-key (kitty
    /// protocol). The dispatcher uses this to force extend resolution at
    /// char-consumption time.
    pub ctrl_extend: bool,
}

// ── KeymapCommand ─────────────────────────────────────────────────────────────

/// What a key binding resolves to after trie lookup.
///
/// Every binding — including composite editor operations — is expressed as
/// a command name referencing an entry in the [`CommandRegistry`]. Extend-mode
/// pairing is stored in the registry, not here.
///
/// [`CommandRegistry`]: super::registry::CommandRegistry
#[derive(Debug, Clone)]
pub(in crate::editor) struct KeymapCommand {
    /// The command name to look up in the registry.
    pub name: Cow<'static, str>,
    /// When `true`, the dispatcher always dispatches this command with
    /// `extend = true`, regardless of kitty mode. Only set on explicit Ctrl
    /// bindings whose extend-line semantics are inherent (e.g. `Ctrl-x` →
    /// `select-line`). Not exposed to Steel's `bind-key!`.
    pub force_extend: bool,
}

// ── WalkResult ────────────────────────────────────────────────────────────────

/// The outcome of walking a key sequence through a [`KeyTrie`].
pub(super) enum WalkResult {
    /// The sequence matches a leaf command — execute it.
    Leaf(KeymapCommand),
    /// At an interior trie node — more keys are needed.
    Interior,
    /// The last key of the sequence matches a wait-char binding. The caller
    /// should consume the next character, store it in `pending_char`, and
    /// dispatch the named command.
    WaitChar(WaitCharPending),
    /// The sequence has no match in this trie.
    NoMatch,
}

// ── KeyTrie ───────────────────────────────────────────────────────────────────

/// A single level of the keymap trie.
///
/// Maps [`KeyEvent`] values to either a sub-trie (interior node) or a leaf
/// command. The trie is built once at startup and never mutated during editing
/// (the Steel config layer will support user overrides).
#[derive(Clone)]
pub(super) struct KeyTrie {
    map: FxHashMap<CanonicalKey, KeyTrieNode>,
}

#[derive(Clone)]
enum KeyTrieNode {
    /// Terminal node — execute this command.
    Leaf(KeymapCommand),
    /// Interior node — more keys needed.
    Node(KeyTrie),
    /// The next character is consumed as an argument (f/t/F/T/r).
    WaitChar(WaitCharPending),
}

impl KeyTrie {
    fn new() -> Self {
        Self {
            map: FxHashMap::default(),
        }
    }

    fn bind(&mut self, key: KeyEvent, node: KeyTrieNode) {
        self.map.insert(CanonicalKey::from(key), node);
    }

    /// True when nothing is bound — every [`Self::walk`] would return
    /// [`WalkResult::NoMatch`]. Lets a caller skip building the sequence to
    /// walk with, which for the Extend trie (empty until a plugin binds into
    /// it) is the usual case.
    pub(in crate::editor) fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn bind_leaf(&mut self, key: KeyEvent, cmd: KeymapCommand) {
        self.bind(key, KeyTrieNode::Leaf(cmd));
    }

    /// Bind a multi-key sequence prefix to a WaitChar node, creating interior
    /// nodes as needed. The next character the user presses after the sequence
    /// will be stored in `pending_char` and `wc.cmd_name` will be dispatched.
    ///
    /// Called by [`Keymap::bind_wait_char_user`] at runtime (e.g. from Steel config).
    pub(in crate::editor) fn bind_wait_char_sequence(
        &mut self,
        keys: &[KeyEvent],
        wc: WaitCharPending,
    ) {
        debug_assert!(!keys.is_empty());
        if keys.len() == 1 {
            self.bind(keys[0], KeyTrieNode::WaitChar(wc));
            return;
        }
        let entry = self
            .map
            .entry(CanonicalKey::from(keys[0]))
            .or_insert_with(|| KeyTrieNode::Node(KeyTrie::new()));
        if !matches!(entry, KeyTrieNode::Node(_)) {
            *entry = KeyTrieNode::Node(KeyTrie::new());
        }
        if let KeyTrieNode::Node(sub) = entry {
            sub.bind_wait_char_sequence(&keys[1..], wc);
        }
    }

    /// Bind a multi-key sequence to a leaf command, creating interior nodes as
    /// needed. Single-key sequences insert directly as a `Leaf`.
    ///
    /// Called by [`Keymap::bind_user_with_extend`] at runtime (e.g. from Steel config).
    pub(in crate::editor::keymap) fn bind_sequence(
        &mut self,
        keys: &[KeyEvent],
        cmd: KeymapCommand,
    ) {
        debug_assert!(!keys.is_empty());
        if keys.len() == 1 {
            self.bind_leaf(keys[0], cmd);
            return;
        }
        let entry = self
            .map
            .entry(CanonicalKey::from(keys[0]))
            .or_insert_with(|| KeyTrieNode::Node(KeyTrie::new()));
        // If the slot already holds a Leaf or WaitChar, replace with a Node
        // so the prefix can be extended. This may shadow an existing binding.
        if !matches!(entry, KeyTrieNode::Node(_)) {
            *entry = KeyTrieNode::Node(KeyTrie::new());
        }
        if let KeyTrieNode::Node(sub) = entry {
            sub.bind_sequence(&keys[1..], cmd);
        }
    }

    /// Remove the binding for a key sequence. Leaves interior nodes in place.
    ///
    /// No-op if the sequence is not bound or any intermediate node is absent.
    pub(in crate::editor::keymap) fn remove_sequence(&mut self, keys: &[KeyEvent]) {
        match keys {
            [] => {}
            [only] => {
                self.map.remove(&CanonicalKey::from(*only));
            }
            [first, rest @ ..] => {
                if let Some(KeyTrieNode::Node(sub)) = self.map.get_mut(&CanonicalKey::from(*first))
                {
                    sub.remove_sequence(rest);
                }
            }
        }
    }

    /// Walk a key sequence through the trie, returning the result after all keys.
    ///
    /// Called by the dispatcher with `self.pending_keys` on every keypress.
    pub(super) fn walk(&self, keys: &[KeyEvent]) -> WalkResult {
        debug_assert!(!keys.is_empty(), "walk called with empty key sequence");

        let mut current = self;
        let last = keys.len() - 1;

        for (i, key) in keys.iter().enumerate() {
            match current.map.get(&CanonicalKey::from(*key)) {
                None => return WalkResult::NoMatch,
                Some(KeyTrieNode::Leaf(cmd)) if i == last => {
                    return WalkResult::Leaf(cmd.clone());
                }
                Some(KeyTrieNode::Leaf(_)) => {
                    // A leaf was reached before consuming all keys — the extra
                    // keys have no match.
                    return WalkResult::NoMatch;
                }
                Some(KeyTrieNode::WaitChar(wc)) if i == last => {
                    return WalkResult::WaitChar(wc.clone());
                }
                Some(KeyTrieNode::WaitChar(_)) => {
                    // WaitChar is always a leaf — can't go deeper.
                    return WalkResult::NoMatch;
                }
                Some(KeyTrieNode::Node(_)) if i == last => {
                    return WalkResult::Interior;
                }
                Some(KeyTrieNode::Node(subtrie)) => {
                    current = subtrie;
                }
            }
        }

        // Unreachable: the loop above always returns before the iterator exhausts.
        WalkResult::NoMatch
    }

    pub(in crate::editor::keymap) fn collect_command_names(&self, out: &mut Vec<String>) {
        for node in self.map.values() {
            match node {
                KeyTrieNode::Leaf(cmd) => out.push(cmd.name.to_string()),
                KeyTrieNode::WaitChar(wc) => out.push(wc.cmd_name.to_string()),
                KeyTrieNode::Node(sub) => sub.collect_command_names(out),
            }
        }
    }
}

// ── BindMode ─────────────────────────────────────────────────────────────────

/// Which keymap to apply a user-supplied binding to.
///
/// Used by [`Keymap::bind_user_with_extend`] and [`Keymap::unbind_user`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum BindMode {
    Normal,
    /// Sparse extend-mode overrides. These are checked first in extend mode;
    /// a miss falls through to the normal trie with `extend = true`.
    Extend,
    Insert,
}

// ── Keymap ────────────────────────────────────────────────────────────────────

/// Per-mode keymap container. One instance lives on the [`Editor`].
///
/// [`Editor`]: super::Editor
#[derive(Clone)]
pub(in crate::editor) struct Keymap {
    pub(super) normal: KeyTrie,
    /// Sparse extend-mode overrides. Empty by default; plugins populate it
    /// (e.g. `core:vim-keybind`'s `o → flip-selections`).
    ///
    /// Checked before the normal trie when the editor is in Extend mode.
    /// A match dispatches directly with `extend = false` — these are
    /// different commands, not extend variants of normal commands.
    /// A miss falls through to the normal trie with `extend = true`.
    pub(super) extend: KeyTrie,
    pub(super) insert: KeyTrie,
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            normal: default_normal_keymap(),
            extend: default_extend_keymap(),
            insert: default_insert_keymap(),
        }
    }
}

impl Keymap {
    /// Bind a key sequence to a WaitChar node in the given mode.
    ///
    /// After the user completes `keys`, the next character is stored in
    /// `pending_char` and `command` is dispatched.  Interior nodes are created
    /// as needed.  `keys` must not be empty.
    pub(in crate::editor) fn bind_wait_char_user(
        &mut self,
        mode: BindMode,
        keys: &[KeyEvent],
        command: Cow<'static, str>,
    ) {
        debug_assert!(
            !keys.is_empty(),
            "bind_wait_char_user called with empty key sequence"
        );
        let trie = self.trie_mut(mode);
        trie.bind_wait_char_sequence(
            keys,
            WaitCharPending {
                cmd_name: command,
                ctrl_extend: false,
            },
        );
    }

    /// Bind a key sequence to a command name in the given mode.
    ///
    /// Overwrites any existing binding for the same sequence. Single-key
    /// sequences are inserted as a `Leaf`; multi-key sequences create
    /// interior nodes as needed. Pass `force_extend = true` for bindings
    /// that should always extend (see `cmd_extend!`).
    ///
    /// `keys` must not be empty.
    pub(in crate::editor) fn bind_user_with_extend(
        &mut self,
        mode: BindMode,
        keys: &[KeyEvent],
        command: Cow<'static, str>,
        force_extend: bool,
    ) {
        debug_assert!(
            !keys.is_empty(),
            "bind_user_with_extend called with empty key sequence"
        );
        let trie = self.trie_mut(mode);
        trie.bind_sequence(
            keys,
            KeymapCommand {
                name: command,
                force_extend,
            },
        );
    }

    /// Remove a binding for a key sequence in the given mode.
    ///
    /// No-op if the sequence is not bound or any intermediate node is missing.
    pub(in crate::editor) fn unbind_user(&mut self, mode: BindMode, keys: &[KeyEvent]) {
        let trie = self.trie_mut(mode);
        trie.remove_sequence(keys);
    }

    /// The trie for `mode`, mutably.
    fn trie_mut(&mut self, mode: BindMode) -> &mut KeyTrie {
        match mode {
            BindMode::Normal => &mut self.normal,
            BindMode::Extend => &mut self.extend,
            BindMode::Insert => &mut self.insert,
        }
    }

    /// The trie for `mode`. Test-only: production code only ever needs
    /// mutable access via [`Self::trie_mut`].
    #[cfg(test)]
    fn trie(&self, mode: BindMode) -> &KeyTrie {
        match mode {
            BindMode::Normal => &self.normal,
            BindMode::Extend => &self.extend,
            BindMode::Insert => &self.insert,
        }
    }

    /// Return the command name and `force_extend` flag for `keys` in `mode`,
    /// or `None` if the sequence is unbound.
    #[cfg(test)]
    pub(in crate::editor) fn lookup_command(
        &self,
        mode: BindMode,
        keys: &[KeyEvent],
    ) -> Option<(String, bool)> {
        let trie = self.trie(mode);
        match trie.walk(keys) {
            WalkResult::Leaf(cmd) => Some((cmd.name.into_owned(), cmd.force_extend)),
            _ => None,
        }
    }

    pub(super) fn all_command_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.normal.collect_command_names(&mut out);
        self.extend.collect_command_names(&mut out);
        self.insert.collect_command_names(&mut out);
        out
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
