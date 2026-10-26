//! Trie-based keymaps for Normal, Extend, Insert and Command modes.
//!
//! Each mode has a [`KeyTrie`] mapping [`KeyEvent`] sequences to
//! [`KeymapCommand`]s: single keys and multi-key sequences (`m i <char>`,
//! `g <key>`). Every keymap holds one layer of bindings per `ConfigOrigin`,
//! the compiled defaults in the bundled one, and the highest layer that knows
//! a sequence decides it. The global [`Keymap`] holds the editor-wide
//! bindings, and [`LocalKeymaps`] the named keymaps attached to buffers. Key
//! handlers look keys up through `Editor::resolve_keys`; the Normal-mode
//! handler in `input_stack/base.rs` accumulates a count prefix and runs
//! commands through [`crate::editor::registry::CommandRegistry`]. A command
//! that takes a character (f/t/F/T/r) reads it from the next key itself.
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

use hume_origin::ByOrigin;
use hume_scripting::ConfigOrigin;
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

/// The outcome of walking a key sequence through a keymap's layers, or
/// through one layer's [`KeyTrie`]: a layer answers `None` when it knows
/// nothing of the sequence, and [`WalkResult::NoMatch`] when it hides it.
pub(super) enum WalkResult {
    /// The sequence matches a leaf command: execute it.
    Leaf(KeymapCommand),
    /// At an interior trie node: more keys are needed.
    Interior,
    /// The sequence has no match. From one layer: it unbound the sequence or
    /// a prefix of it, or bound a shorter one as a leaf, so no lower layer's
    /// binding applies.
    NoMatch,
}

/// A single level of the keymap trie.
///
/// Maps [`KeyEvent`] values to a sub-trie (interior node), a leaf command,
/// or an unbinding.
#[derive(Clone, Default)]
pub(super) struct KeyTrie {
    map: FxHashMap<CanonicalKey, KeyTrieNode>,
    /// Built where this layer had unbound the prefix: a sequence under it
    /// that this layer does not bind stays unbound instead of reaching a
    /// lower layer.
    hides_lower: bool,
}

#[derive(Clone)]
enum KeyTrieNode {
    /// Terminal node: execute this command.
    Leaf(KeymapCommand),
    /// Interior node: more keys needed.
    Node(KeyTrie),
    /// `unbind-key!` from this layer: the sequence and everything under it
    /// is unbound here and below.
    Unbound,
}

impl KeyTrie {
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
        self.set_sequence(keys, KeyTrieNode::Leaf(cmd));
    }

    /// Unbind a key sequence in this trie, creating interior nodes as needed,
    /// so a lower layer's binding of it does not apply. Unbinding a
    /// prefix unbinds everything under it.
    pub(in crate::editor::keymap) fn unbind_sequence(&mut self, keys: &[KeyEvent]) {
        self.set_sequence(keys, KeyTrieNode::Unbound);
    }

    /// Put `node` at `keys`, turning a leaf or unbinding on the way into an
    /// interior node and replacing whatever `keys` held. An unbinding turned
    /// into an interior node keeps hiding the lower layers' sequences under
    /// it.
    fn set_sequence(&mut self, keys: &[KeyEvent], node: KeyTrieNode) {
        debug_assert!(!keys.is_empty());
        let [first, rest @ ..] = keys else {
            return;
        };
        if rest.is_empty() {
            self.bind(*first, node);
            return;
        }
        let entry = self
            .map
            .entry(CanonicalKey::from(*first))
            .or_insert_with(|| KeyTrieNode::Node(KeyTrie::default()));
        if !matches!(entry, KeyTrieNode::Node(_)) {
            let hides_lower = matches!(entry, KeyTrieNode::Unbound);
            *entry = KeyTrieNode::Node(KeyTrie {
                hides_lower,
                ..KeyTrie::default()
            });
        }
        if let KeyTrieNode::Node(sub) = entry {
            sub.set_sequence(rest, node);
        }
    }

    /// Walk a key sequence through the trie, returning the result after all
    /// keys. `None` when the trie knows nothing of the sequence, so a lower
    /// layer may.
    fn walk(&self, keys: &[KeyEvent]) -> Option<WalkResult> {
        let (last, prefix) = keys
            .split_last()
            .expect("walk called with empty key sequence");

        let mut current = self;
        let mut hiding = self.hides_lower;
        for key in prefix {
            match current.map.get(&CanonicalKey::from(*key)) {
                Some(KeyTrieNode::Node(subtrie)) => {
                    current = subtrie;
                    hiding |= subtrie.hides_lower;
                }
                Some(KeyTrieNode::Leaf(_) | KeyTrieNode::Unbound) => {
                    return Some(WalkResult::NoMatch);
                }
                None if hiding => return Some(WalkResult::NoMatch),
                None => return None,
            }
        }
        match current.map.get(&CanonicalKey::from(*last)) {
            Some(KeyTrieNode::Leaf(cmd)) => Some(WalkResult::Leaf(cmd.clone())),
            Some(KeyTrieNode::Node(_)) => Some(WalkResult::Interior),
            Some(KeyTrieNode::Unbound) => Some(WalkResult::NoMatch),
            None if hiding => Some(WalkResult::NoMatch),
            None => None,
        }
    }

    /// The command name of every binding under this trie.
    fn collect_command_names(&self, out: &mut Vec<String>) {
        for node in self.map.values() {
            match node {
                KeyTrieNode::Leaf(cmd) => out.push(cmd.name.to_string()),
                KeyTrieNode::Node(sub) => sub.collect_command_names(out),
                KeyTrieNode::Unbound => {}
            }
        }
    }
}

/// One mode's bindings: a trie per [`ConfigOrigin`]. The highest layer that
/// knows a sequence, as a binding, a prefix or an unbinding, decides it, so a
/// layer's bindings apply wherever no higher layer has its own.
#[derive(Clone, Default)]
struct Bindings(ByOrigin<KeyTrie>);

impl Bindings {
    /// `bundled` as the bundled layer, with no plugin or user bindings.
    fn bundled(bundled: KeyTrie) -> Self {
        let mut layers = ByOrigin::default();
        *layers.get_mut(ConfigOrigin::Bundled) = bundled;
        Self(layers)
    }

    fn layer_mut(&mut self, by: ConfigOrigin) -> &mut KeyTrie {
        self.0.get_mut(by)
    }

    fn walk(&self, keys: &[KeyEvent]) -> WalkResult {
        self.0
            .iter()
            .find_map(|(_, layer)| layer.walk(keys))
            .unwrap_or(WalkResult::NoMatch)
    }

    /// Whether a layer above `by` knows `keys`, so a write to `by`'s layer
    /// does not apply there.
    fn hidden_above(&self, by: ConfigOrigin, keys: &[KeyEvent]) -> bool {
        self.0.above(by).any(|layer| layer.walk(keys).is_some())
    }

    fn collect_command_names(&self, out: &mut Vec<String>) {
        for (_, layer) in self.0.iter() {
            layer.collect_command_names(out);
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
    normal: Bindings,
    /// Sparse extend-mode overrides. Empty by default; plugins populate it
    /// (e.g. `core:vim-keybind`'s `o → flip-selections`).
    ///
    /// Checked before this keymap's normal trie when the editor is in Extend
    /// mode. A match dispatches directly with `extend = false`: these are
    /// different commands, not extend variants of normal commands.
    /// A miss falls through to the normal trie with `extend = true`.
    extend: Bindings,
    insert: Bindings,
    /// Keys bound in the command mode prompt, walked one key at a time
    /// before the prompt's own keys. Empty by default.
    command: Bindings,
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            normal: Bindings::bundled(default_normal_keymap()),
            extend: Bindings::bundled(default_extend_keymap()),
            insert: Bindings::bundled(default_insert_keymap()),
            command: Bindings::default(),
        }
    }
}

impl Keymap {
    /// A keymap with nothing bound, unlike [`Keymap::default`], which holds
    /// the default bindings.
    pub(in crate::editor) fn empty() -> Self {
        Self {
            normal: Bindings::default(),
            extend: Bindings::default(),
            insert: Bindings::default(),
            command: Bindings::default(),
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
            mode => found(self.bindings(mode).walk(keys), false),
        }
    }

    /// Bind a key sequence to a command name in the given mode, in `by`'s
    /// layer, replacing that layer's binding of the same sequence.
    /// Single-key sequences are inserted as a `Leaf`; multi-key sequences
    /// create interior nodes as needed. Pass `force_extend = true` for
    /// bindings that should always extend (see `cmd_extend!`).
    ///
    /// Returns whether a higher origin's binding, prefix or unbinding hides
    /// this one. `keys` must not be empty.
    pub(in crate::editor) fn bind(
        &mut self,
        mode: BindMode,
        keys: &[KeyEvent],
        command: Cow<'static, str>,
        force_extend: bool,
        by: ConfigOrigin,
    ) -> bool {
        debug_assert!(!keys.is_empty(), "bind called with empty key sequence");
        let bindings = self.bindings_mut(mode);
        bindings.layer_mut(by).bind_sequence(
            keys,
            KeymapCommand {
                name: command,
                force_extend,
            },
        );
        bindings.hidden_above(by, keys)
    }

    /// Unbind a key sequence in the given mode, in `by`'s layer: no lower
    /// origin's binding of it applies. Returns whether a higher origin's
    /// layer decides the sequence anyway.
    pub(in crate::editor) fn unbind(
        &mut self,
        mode: BindMode,
        keys: &[KeyEvent],
        by: ConfigOrigin,
    ) -> bool {
        let bindings = self.bindings_mut(mode);
        bindings.layer_mut(by).unbind_sequence(keys);
        bindings.hidden_above(by, keys)
    }

    /// The bindings for `mode`, mutably.
    fn bindings_mut(&mut self, mode: BindMode) -> &mut Bindings {
        match mode {
            BindMode::Normal => &mut self.normal,
            BindMode::Extend => &mut self.extend,
            BindMode::Insert => &mut self.insert,
            BindMode::Command => &mut self.command,
        }
    }

    /// The bindings for `mode`.
    fn bindings(&self, mode: BindMode) -> &Bindings {
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
        match self.bindings(mode).walk(keys) {
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
