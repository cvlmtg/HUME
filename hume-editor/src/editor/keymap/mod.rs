//! Trie-based keymap for Normal and Insert modes.
//!
//! # Architecture
//!
//! Each mode has a [`KeyTrie`] that maps [`KeyEvent`] sequences to
//! [`KeymapCommand`] values. The trie supports:
//!
//! - **Single-key bindings**: most keys (h/j/k/l, d, y, etc.)
//! - **Multi-key sequences**: `m` → `i`/`a` → object char (text objects);
//!   future `g` → second key (goto commands).
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

#[macro_use]
mod defaults;
use defaults::{default_extend_keymap, default_insert_keymap, default_normal_keymap};

use rustc_hash::FxHashMap;
use std::borrow::Cow;
use std::hash::{Hash, Hasher};

use termina::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, Modifiers};

// ── WaitCharPending ───────────────────────────────────────────────────────────

/// State stored on the editor after a wait-char key (f/t/F/T/r).
///
/// On the next keypress the dispatcher stores the character in
/// `Editor.pending_char` and dispatches `cmd_name`. Extend-mode resolution
/// happens at char-consumption time via the registry.
#[derive(Debug, Clone)]
pub(crate) struct WaitCharPending {
    pub cmd_name: Cow<'static, str>,
    /// Set to `true` when this wait-char was triggered via Ctrl+key (kitty
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
pub(crate) struct KeymapCommand {
    /// The command name to look up in the registry.
    pub name: Cow<'static, str>,
    /// When `true`, the dispatcher always dispatches this command with
    /// `extend = true`, regardless of kitty mode. Only set on explicit Ctrl
    /// bindings whose extend-line semantics are inherent (e.g. `Ctrl+x` →
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

// ── Key binding identity ─────────────────────────────────────────────────────

/// Canonical binding identity for a key event.
///
/// `KeyEvent`'s `PartialEq`/`Hash` impls perform no case normalization, so
/// the trie normalizes uppercase char ⇔ `SHIFT` explicitly at the binding
/// boundary. Also scrubs fields that never participate in binding identity:
/// `kind` (a kitty autorepeat is a `Repeat` event, not `Press` — held keys
/// must keep matching the same binding under `REPORT_EVENT_TYPES`), protocol
/// `state`, and the Caps/Num Lock modifier bits.
fn canonical(mut key: KeyEvent) -> KeyEvent {
    key.kind = KeyEventKind::Press;
    key.state = KeyEventState::NONE;
    key.modifiers -= Modifiers::CAPS_LOCK | Modifiers::NUM_LOCK;
    if let KeyCode::Char(c) = key.code {
        if c.is_ascii_uppercase() {
            key.modifiers |= Modifiers::SHIFT;
        } else if key.modifiers.contains(Modifiers::SHIFT) {
            // No-op for punctuation and other non-alphabetic chars: shifted
            // punctuation (e.g. `:`) stays distinct from its unshifted form,
            // matching what terminals actually deliver.
            key.code = KeyCode::Char(c.to_ascii_uppercase());
        }
    }
    key
}

/// Tags each [`KeyCode`] variant with a small integer plus payload so
/// [`encode`] can pack it into a `u64`. `Media`/`Modifier` are fieldless enums,
/// so casting to `u32` is a plain discriminant read.
fn encode_key_code(code: KeyCode) -> (u8, u32) {
    match code {
        KeyCode::Char(c) => (0, c as u32),
        KeyCode::Function(n) => (1, n as u32),
        KeyCode::Media(m) => (2, m as u32),
        KeyCode::Modifier(m) => (3, m as u32),
        KeyCode::Enter => (4, 0),
        KeyCode::Backspace => (5, 0),
        KeyCode::Tab => (6, 0),
        KeyCode::Escape => (7, 0),
        KeyCode::Left => (8, 0),
        KeyCode::Right => (9, 0),
        KeyCode::Up => (10, 0),
        KeyCode::Down => (11, 0),
        KeyCode::Home => (12, 0),
        KeyCode::End => (13, 0),
        KeyCode::BackTab => (14, 0),
        KeyCode::PageUp => (15, 0),
        KeyCode::PageDown => (16, 0),
        KeyCode::Insert => (17, 0),
        KeyCode::Delete => (18, 0),
        KeyCode::KeypadBegin => (19, 0),
        KeyCode::CapsLock => (20, 0),
        KeyCode::ScrollLock => (21, 0),
        KeyCode::NumLock => (22, 0),
        KeyCode::PrintScreen => (23, 0),
        KeyCode::Pause => (24, 0),
        KeyCode::Menu => (25, 0),
        KeyCode::Null => (26, 0),
    }
}

/// Injective encoding of a canonical key event's `(code, modifiers)` pair,
/// used as the trie's hash. `kind`/`state` are excluded because [`canonical`]
/// already collapses them to fixed values.
fn encode(key: &KeyEvent) -> u64 {
    let (tag, payload) = encode_key_code(key.code);
    ((tag as u64) << 40) | ((payload as u64) << 8) | key.modifiers.bits() as u64
}

/// Hashable, case-normalized wrapper around [`KeyEvent`] used as the trie's
/// map key. termina's `KeyEvent` derives `PartialEq` but not `Hash`, and its
/// equality has no case-normalization — both are needed for binding lookup,
/// so this type is the only place a raw `KeyEvent` becomes a map key.
#[derive(Clone, Copy, PartialEq, Eq)]
struct TrieKey(KeyEvent);

impl From<KeyEvent> for TrieKey {
    fn from(key: KeyEvent) -> Self {
        Self(canonical(key))
    }
}

impl Hash for TrieKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(encode(&self.0));
    }
}

// ── KeyTrie ───────────────────────────────────────────────────────────────────

/// A single level of the keymap trie.
///
/// Maps [`KeyEvent`] values to either a sub-trie (interior node) or a leaf
/// command. The trie is built once at startup and never mutated during editing
/// (the Steel config layer will support user overrides).
#[derive(Clone)]
pub(super) struct KeyTrie {
    map: FxHashMap<TrieKey, KeyTrieNode>,
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
        self.map.insert(TrieKey::from(key), node);
    }

    fn bind_leaf(&mut self, key: KeyEvent, cmd: KeymapCommand) {
        self.bind(key, KeyTrieNode::Leaf(cmd));
    }

    /// Bind a multi-key sequence prefix to a WaitChar node, creating interior
    /// nodes as needed. The next character the user presses after the sequence
    /// will be stored in `pending_char` and `wc.cmd_name` will be dispatched.
    ///
    /// Called by [`Keymap::bind_wait_char_user`] at runtime (e.g. from Steel config).
    pub(crate) fn bind_wait_char_sequence(&mut self, keys: &[KeyEvent], wc: WaitCharPending) {
        debug_assert!(!keys.is_empty());
        if keys.len() == 1 {
            self.bind(keys[0], KeyTrieNode::WaitChar(wc));
            return;
        }
        let entry = self
            .map
            .entry(TrieKey::from(keys[0]))
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
    pub(crate) fn bind_sequence(&mut self, keys: &[KeyEvent], cmd: KeymapCommand) {
        debug_assert!(!keys.is_empty());
        if keys.len() == 1 {
            self.bind_leaf(keys[0], cmd);
            return;
        }
        let entry = self
            .map
            .entry(TrieKey::from(keys[0]))
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
    pub(crate) fn remove_sequence(&mut self, keys: &[KeyEvent]) {
        match keys {
            [] => {}
            [only] => {
                self.map.remove(&TrieKey::from(*only));
            }
            [first, rest @ ..] => {
                if let Some(KeyTrieNode::Node(sub)) = self.map.get_mut(&TrieKey::from(*first)) {
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
            match current.map.get(&TrieKey::from(*key)) {
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

    pub(super) fn collect_command_names(&self, out: &mut Vec<String>) {
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
pub(crate) enum BindMode {
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
pub(crate) struct Keymap {
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
    pub(crate) fn bind_wait_char_user(
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
    pub(crate) fn bind_user_with_extend(
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
    pub(crate) fn unbind_user(&mut self, mode: BindMode, keys: &[KeyEvent]) {
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
    pub(crate) fn lookup_command(
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
