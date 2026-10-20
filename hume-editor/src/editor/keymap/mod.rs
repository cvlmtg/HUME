//! Trie-based keymaps for Normal, Extend, Insert and Command modes.
//!
//! Each mode has a [`KeyTrie`] mapping [`KeyEvent`] sequences to
//! [`KeymapCommand`]s: single keys and multi-key sequences (`m i <char>`,
//! `g <key>`). The global [`Keymap`] holds the default and user bindings, and
//! [`LocalKeymaps`] the named keymaps attached to buffers. Key handlers look
//! keys up through `Editor::resolve_keys`; the Normal-mode handler in
//! `input_stack/base.rs` accumulates a count prefix and runs commands through
//! [`crate::editor::registry::CommandRegistry`]. A command that takes a
//! character (f/t/F/T/r) reads it from the next key itself.
//!
//! Only base command names are stored; Extend mode is a `MotionMode` parameter
//! chosen at dispatch time. Each keymap's sparse `extend` trie holds overrides
//! checked before its own normal trie in Extend mode. The global one ships
//! empty; plugins fill it. So does the `command` trie, the keys bound in the
//! command mode prompt.

mod canonical;
#[macro_use]
mod defaults;
mod local;
use defaults::{default_extend_keymap, default_insert_keymap, default_normal_keymap};

pub(in crate::editor) use canonical::CanonicalKey;
pub(in crate::editor) use local::{LocalKeymaps, Resolved, UnknownKeymap};

use rustc_hash::FxHashMap;
use std::borrow::Cow;

use termina::event::KeyEvent;

use crate::editor::registry::CommandRegistry;

/// What a key binding resolves to after trie lookup.
///
/// Every binding, including composite editor operations, is expressed as
/// a command name referencing an entry in the [`CommandRegistry`]. Extend-mode
/// pairing is stored in the registry, not here.
///
/// [`CommandRegistry`]: super::registry::CommandRegistry
#[derive(Debug, Clone)]
pub(in crate::editor) struct KeymapCommand {
    /// The command name to look up in the registry.
    pub name: Cow<'static, str>,
    /// When `true`, the dispatcher always dispatches this command with
    /// `extend = true`, regardless of kitty mode. Set on default Ctrl
    /// bindings whose extend-line semantics are inherent (e.g. `Ctrl-x` →
    /// `select-line`), and on Normal-mode bindings made with Steel's
    /// `bind-key-extend!`.
    pub force_extend: bool,
}

/// The outcome of walking a key sequence through a [`KeyTrie`].
pub(super) enum WalkResult {
    /// The sequence matches a leaf command: execute it.
    Leaf(KeymapCommand),
    /// At an interior trie node: more keys are needed.
    Interior,
    /// The sequence has no match in this trie.
    NoMatch,
}

/// A single level of the keymap trie.
///
/// Maps [`KeyEvent`] values to either a sub-trie (interior node) or a leaf
/// command.
#[derive(Clone)]
pub(super) struct KeyTrie {
    map: FxHashMap<CanonicalKey, KeyTrieNode>,
}

#[derive(Clone)]
enum KeyTrieNode {
    /// Terminal node: execute this command.
    Leaf(KeymapCommand),
    /// Interior node: more keys needed.
    Node(KeyTrie),
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

    fn bind_leaf(&mut self, key: KeyEvent, cmd: KeymapCommand) {
        self.bind(key, KeyTrieNode::Leaf(cmd));
    }

    /// Bind a multi-key sequence to a leaf command, creating interior nodes as
    /// needed. Single-key sequences insert directly as a `Leaf`.
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
        // If the slot already holds a Leaf, replace with a Node
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
    fn walk(&self, keys: &[KeyEvent]) -> WalkResult {
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
                    // A leaf was reached before consuming all keys: the extra
                    // keys have no match.
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

    /// The command name of every binding under this trie.
    fn collect_command_names(&self, out: &mut Vec<String>) {
        for node in self.map.values() {
            match node {
                KeyTrieNode::Leaf(cmd) => out.push(cmd.name.to_string()),
                KeyTrieNode::Node(sub) => sub.collect_command_names(out),
            }
        }
    }
}

/// Which keymap to apply a user-supplied binding to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum BindMode {
    Normal,
    /// Sparse extend-mode overrides. In extend mode a keymap's overrides are
    /// checked before its normal trie, whose match runs with `extend = true`.
    Extend,
    Insert,
    /// The command mode prompt: the `:` line and `prompt!` prompts.
    Command,
}

/// Per-mode keymap container: the editor's global keymap, and each named
/// keymap in [`LocalKeymaps`].
#[derive(Clone)]
pub(in crate::editor) struct Keymap {
    normal: KeyTrie,
    /// Sparse extend-mode overrides. Empty by default; plugins populate it
    /// (e.g. `core:vim-keybind`'s `o → flip-selections`).
    ///
    /// Checked before this keymap's normal trie when the editor is in Extend
    /// mode. A match dispatches directly with `extend = false`: these are
    /// different commands, not extend variants of normal commands.
    /// A miss falls through to the normal trie with `extend = true`.
    extend: KeyTrie,
    insert: KeyTrie,
    /// Keys bound in the command mode prompt, walked one key at a time
    /// before the prompt's own keys. Empty by default.
    command: KeyTrie,
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            normal: default_normal_keymap(),
            extend: default_extend_keymap(),
            insert: default_insert_keymap(),
            command: KeyTrie::new(),
        }
    }
}

impl Keymap {
    /// A keymap with nothing bound, unlike [`Keymap::default`], which holds
    /// the default bindings.
    pub(in crate::editor) fn empty() -> Self {
        Self {
            normal: KeyTrie::new(),
            extend: KeyTrie::new(),
            insert: KeyTrie::new(),
            command: KeyTrie::new(),
        }
    }

    /// What `keys` resolve to in this keymap alone, or `None` when they match
    /// nothing here. See [`LocalKeymaps::resolve`] for what each mode walks.
    fn resolve(&self, mode: BindMode, keys: &[KeyEvent]) -> Option<Resolved> {
        let found = |walk: WalkResult, extend_override| {
            (!matches!(walk, WalkResult::NoMatch)).then_some(Resolved {
                walk,
                extend_override,
            })
        };
        match mode {
            BindMode::Extend => {
                found(self.extend.walk(keys), true).or_else(|| found(self.normal.walk(keys), false))
            }
            mode => found(self.trie(mode).walk(keys), false),
        }
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
            BindMode::Command => &mut self.command,
        }
    }

    /// The trie for `mode`.
    fn trie(&self, mode: BindMode) -> &KeyTrie {
        match mode {
            BindMode::Normal => &self.normal,
            BindMode::Extend => &self.extend,
            BindMode::Insert => &self.insert,
            BindMode::Command => &self.command,
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
        self.command.collect_command_names(&mut out);
        out
    }

    /// The bound command names `registry` has no key-bindable command for,
    /// sorted and without repeats.
    pub(super) fn unknown_command_names(&self, registry: &CommandRegistry) -> Vec<String> {
        let mut names = self.all_command_names();
        names.retain(|name| registry.get_mappable(name).is_none());
        names.sort_unstable();
        names.dedup();
        names
    }
}

#[cfg(test)]
mod tests;
