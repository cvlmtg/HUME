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
//! The dispatcher in `mappings.rs` walks the trie on each keypress, accumulates
//! a numeric count prefix, and executes [`KeymapCommand`] values via the
//! [`CommandRegistry`].
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

use std::borrow::Cow;
use std::collections::HashMap;
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
    /// The `name` field names this node (e.g. `"match"`, `"goto"`) and will
    /// be shown in the statusline while the user completes the sequence.
    Interior {
        #[allow(dead_code)]
        name: &'static str,
    },
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
    /// Human-readable name shown in the statusline when the user is mid-sequence
    /// at this node (e.g. `"match"` after pressing `m`, `"goto"` after `g`).
    pub(super) name: &'static str,
    map: HashMap<TrieKey, KeyTrieNode>,
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
    fn new(name: &'static str) -> Self {
        Self {
            name,
            map: HashMap::new(),
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
            .or_insert_with(|| KeyTrieNode::Node(KeyTrie::new("user")));
        if !matches!(entry, KeyTrieNode::Node(_)) {
            *entry = KeyTrieNode::Node(KeyTrie::new("user"));
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
            .or_insert_with(|| KeyTrieNode::Node(KeyTrie::new("user")));
        // If the slot already holds a Leaf or WaitChar, replace with a Node
        // so the prefix can be extended. This may shadow an existing binding.
        if !matches!(entry, KeyTrieNode::Node(_)) {
            *entry = KeyTrieNode::Node(KeyTrie::new("user"));
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
                Some(KeyTrieNode::Node(subtrie)) if i == last => {
                    return WalkResult::Interior { name: subtrie.name };
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
pub enum BindMode {
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
pub struct Keymap {
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
    pub fn bind_wait_char_user(
        &mut self,
        mode: BindMode,
        keys: &[KeyEvent],
        command: Cow<'static, str>,
    ) {
        debug_assert!(
            !keys.is_empty(),
            "bind_wait_char_user called with empty key sequence"
        );
        let trie = match mode {
            BindMode::Normal => &mut self.normal,
            BindMode::Extend => &mut self.extend,
            BindMode::Insert => &mut self.insert,
        };
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
    pub fn bind_user_with_extend(
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
        let trie = match mode {
            BindMode::Normal => &mut self.normal,
            BindMode::Extend => &mut self.extend,
            BindMode::Insert => &mut self.insert,
        };
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
    pub fn unbind_user(&mut self, mode: BindMode, keys: &[KeyEvent]) {
        let trie = match mode {
            BindMode::Normal => &mut self.normal,
            BindMode::Extend => &mut self.extend,
            BindMode::Insert => &mut self.insert,
        };
        trie.remove_sequence(keys);
    }

    /// Return the command name and `force_extend` flag for `keys` in `mode`,
    /// or `None` if the sequence is unbound.
    pub fn lookup_command(&self, mode: BindMode, keys: &[KeyEvent]) -> Option<(String, bool)> {
        let trie = match mode {
            BindMode::Normal => &self.normal,
            BindMode::Extend => &self.extend,
            BindMode::Insert => &self.insert,
        };
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
mod tests {
    use super::*;
    use termina::event::{KeyCode, KeyEvent, Modifiers};

    // ── bind_sequence / remove_sequence / bind_user_with_extend / unbind_user ─

    #[test]
    fn bind_sequence_single_key() {
        let mut trie = KeyTrie::new("test");
        trie.bind_sequence(
            &[key!('z')],
            KeymapCommand {
                name: Cow::Borrowed("my-cmd"),
                force_extend: false,
            },
        );
        assert!(matches!(trie.walk(&[key!('z')]), WalkResult::Leaf(ref c) if c.name == "my-cmd"));
    }

    #[test]
    fn bind_sequence_multi_key() {
        let mut trie = KeyTrie::new("test");
        trie.bind_sequence(
            &[key!('g'), key!('g')],
            KeymapCommand {
                name: Cow::Borrowed("goto-first-line"),
                force_extend: false,
            },
        );
        assert!(matches!(
            trie.walk(&[key!('g')]),
            WalkResult::Interior { .. }
        ));
        assert!(matches!(
            trie.walk(&[key!('g'), key!('g')]),
            WalkResult::Leaf(ref c) if c.name == "goto-first-line"
        ));
    }

    #[test]
    fn bind_sequence_shadows_existing_leaf() {
        let mut trie = KeyTrie::new("test");
        // Bind `g` as a leaf first.
        trie.bind_sequence(
            &[key!('g')],
            KeymapCommand {
                name: Cow::Borrowed("old-cmd"),
                force_extend: false,
            },
        );
        // Now bind `gg` — should convert `g` from Leaf to Node.
        trie.bind_sequence(
            &[key!('g'), key!('g')],
            KeymapCommand {
                name: Cow::Borrowed("new-cmd"),
                force_extend: false,
            },
        );
        assert!(matches!(
            trie.walk(&[key!('g')]),
            WalkResult::Interior { .. }
        ));
        assert!(matches!(
            trie.walk(&[key!('g'), key!('g')]),
            WalkResult::Leaf(ref c) if c.name == "new-cmd"
        ));
    }

    #[test]
    fn remove_sequence_single_key() {
        let mut trie = KeyTrie::new("test");
        trie.bind_sequence(
            &[key!('z')],
            KeymapCommand {
                name: Cow::Borrowed("my-cmd"),
                force_extend: false,
            },
        );
        trie.remove_sequence(&[key!('z')]);
        assert!(matches!(trie.walk(&[key!('z')]), WalkResult::NoMatch));
    }

    #[test]
    fn remove_sequence_multi_key() {
        let mut trie = KeyTrie::new("test");
        trie.bind_sequence(
            &[key!('g'), key!('g')],
            KeymapCommand {
                name: Cow::Borrowed("goto-first-line"),
                force_extend: false,
            },
        );
        trie.remove_sequence(&[key!('g'), key!('g')]);
        // Interior node for `g` remains; leaf `gg` is gone.
        assert!(matches!(
            trie.walk(&[key!('g')]),
            WalkResult::Interior { .. }
        ));
        assert!(matches!(
            trie.walk(&[key!('g'), key!('g')]),
            WalkResult::NoMatch
        ));
    }

    #[test]
    fn remove_sequence_nonexistent_is_noop() {
        let mut trie = KeyTrie::new("test");
        trie.bind_sequence(
            &[key!('z')],
            KeymapCommand {
                name: Cow::Borrowed("my-cmd"),
                force_extend: false,
            },
        );
        trie.remove_sequence(&[key!('q')]); // q not bound — no-op
        trie.remove_sequence(&[key!('z'), key!('z')]); // path doesn't exist — no-op
        // `z` leaf is untouched.
        assert!(matches!(trie.walk(&[key!('z')]), WalkResult::Leaf(ref c) if c.name == "my-cmd"));
    }

    #[test]
    fn bind_user_normal_mode() {
        let mut km = Keymap::default();
        km.bind_user_with_extend(
            BindMode::Normal,
            &[key!('z')],
            Cow::Borrowed("my-cmd"),
            false,
        );
        assert!(matches!(
            km.normal.walk(&[key!('z')]),
            WalkResult::Leaf(ref c) if c.name == "my-cmd"
        ));
        // Insert mode unchanged.
        assert!(matches!(km.insert.walk(&[key!('z')]), WalkResult::NoMatch));
    }

    #[test]
    fn unbind_user_normal_mode() {
        let mut km = Keymap::default();
        km.bind_user_with_extend(
            BindMode::Normal,
            &[key!('z')],
            Cow::Borrowed("my-cmd"),
            false,
        );
        km.unbind_user(BindMode::Normal, &[key!('z')]);
        assert!(matches!(km.normal.walk(&[key!('z')]), WalkResult::NoMatch));
    }

    // ── collect_command_names ─────────────────────────────────────────────────

    #[test]
    fn collect_command_names_includes_leaves_and_waitchars() {
        let mut trie = KeyTrie::new("test");
        trie.bind_sequence(
            &[key!('x')],
            KeymapCommand {
                name: Cow::Borrowed("delete-char-forward"),
                force_extend: false,
            },
        );
        trie.bind_sequence(
            &[key!('g'), key!('g')],
            KeymapCommand {
                name: Cow::Borrowed("goto-first-line"),
                force_extend: false,
            },
        );
        trie.bind_wait_char_sequence(
            &[key!('f')],
            WaitCharPending {
                cmd_name: Cow::Borrowed("find-char"),
                ctrl_extend: false,
            },
        );

        let mut names: Vec<String> = Vec::new();
        trie.collect_command_names(&mut names);
        names.sort();

        // Independent oracle: exact expected set.
        assert!(
            names.contains(&"delete-char-forward".to_string()),
            "leaf must appear"
        );
        assert!(
            names.contains(&"goto-first-line".to_string()),
            "nested leaf must appear"
        );
        assert!(
            names.contains(&"find-char".to_string()),
            "wait-char must appear"
        );
    }

    #[test]
    fn all_command_names_covers_all_three_modes() {
        let mut km = Keymap::default();
        // Bind sentinel names in each mode to verify the sweep is complete.
        km.bind_user_with_extend(
            BindMode::Normal,
            &[key!('Q')],
            Cow::Borrowed("normal-sentinel"),
            false,
        );
        km.bind_user_with_extend(
            BindMode::Insert,
            &[key!('Q')],
            Cow::Borrowed("insert-sentinel"),
            false,
        );
        let names = km.all_command_names();
        assert!(
            names.contains(&"normal-sentinel".to_string()),
            "normal mode must be swept"
        );
        assert!(
            names.contains(&"insert-sentinel".to_string()),
            "insert mode must be swept"
        );
    }

    // ── canonical() ────────────────────────────────────────────────────────────

    #[test]
    fn canonical_uppercase_char_gains_shift() {
        let k = canonical(KeyEvent::new(KeyCode::Char('G'), Modifiers::NONE));
        assert_eq!(k, KeyEvent::new(KeyCode::Char('G'), Modifiers::SHIFT));
    }

    #[test]
    fn canonical_shift_lowercase_char_becomes_uppercase() {
        let k = canonical(KeyEvent::new(KeyCode::Char('g'), Modifiers::SHIFT));
        assert_eq!(k, KeyEvent::new(KeyCode::Char('G'), Modifiers::SHIFT));
    }

    #[test]
    fn canonical_shift_punctuation_is_unchanged() {
        // Punctuation has no case to normalize — SHIFT+':' stays distinct from
        // plain ':'. (The lone-SHIFT strip in `handle_normal` handles the
        // partially-compliant-terminal gap for punctuation; that's a separate
        // mechanism from this trie-identity normalization.)
        let k = canonical(KeyEvent::new(KeyCode::Char(':'), Modifiers::SHIFT));
        assert_eq!(k, KeyEvent::new(KeyCode::Char(':'), Modifiers::SHIFT));
    }

    #[test]
    fn canonical_scrubs_lock_bits() {
        let k = canonical(KeyEvent::new(
            KeyCode::Char('h'),
            Modifiers::CAPS_LOCK | Modifiers::NUM_LOCK,
        ));
        assert_eq!(k, KeyEvent::new(KeyCode::Char('h'), Modifiers::NONE));
    }

    #[test]
    fn canonical_scrubs_protocol_state() {
        let mut k = KeyEvent::new(KeyCode::Char('h'), Modifiers::NONE);
        k.state = KeyEventState::KEYPAD | KeyEventState::CAPS_LOCK;
        assert_eq!(canonical(k).state, KeyEventState::NONE);
    }

    #[test]
    fn canonical_repeat_becomes_press() {
        let mut k = KeyEvent::new(KeyCode::Char('j'), Modifiers::NONE);
        k.kind = KeyEventKind::Repeat;
        assert_eq!(canonical(k).kind, KeyEventKind::Press);
    }

    #[test]
    fn canonical_is_idempotent() {
        let k = KeyEvent::new(KeyCode::Char('g'), Modifiers::SHIFT);
        assert_eq!(canonical(canonical(k)), canonical(k));
    }

    // ── Trie resolution equivalence ───────────────────────────────────────────

    #[test]
    fn walk_resolves_uppercase_leaf_regardless_of_incoming_shift_bit() {
        let mut trie = KeyTrie::new("test");
        trie.bind_leaf(
            key!('I'),
            KeymapCommand {
                name: Cow::Borrowed("insert-at-line-start"),
                force_extend: false,
            },
        );
        // A non-conformant kitty terminal delivers the uppercase codepoint
        // with SHIFT still set (see `handle_normal`'s doc comment); a clean
        // delivery omits it. Both must resolve to the same leaf.
        let non_conformant = KeyEvent::new(KeyCode::Char('I'), Modifiers::SHIFT);
        let clean = KeyEvent::new(KeyCode::Char('I'), Modifiers::NONE);
        assert!(matches!(
            trie.walk(&[non_conformant]),
            WalkResult::Leaf(ref c) if c.name == "insert-at-line-start"
        ));
        assert!(matches!(
            trie.walk(&[clean]),
            WalkResult::Leaf(ref c) if c.name == "insert-at-line-start"
        ));
    }

    #[test]
    fn walk_resolves_repeat_kind_key_like_press() {
        // A kitty terminal with REPORT_EVENT_TYPES sends autorepeat as
        // `KeyEventKind::Repeat`, not `Press`. A held key must keep matching
        // its binding.
        let mut trie = KeyTrie::new("test");
        trie.bind_leaf(
            key!('j'),
            KeymapCommand {
                name: Cow::Borrowed("move-down"),
                force_extend: false,
            },
        );
        let mut repeated = KeyEvent::new(KeyCode::Char('j'), Modifiers::NONE);
        repeated.kind = KeyEventKind::Repeat;
        assert!(matches!(
            trie.walk(&[repeated]),
            WalkResult::Leaf(ref c) if c.name == "move-down"
        ));
    }

    #[test]
    fn bind_user_with_extend_shift_lowercase_and_uppercase_are_the_same_binding() {
        // Steel's `bind-key!` "shift-a" / "S-a" spelling parses to
        // Char('a')+SHIFT (see hume-scripting's `keys.rs`); a plain "A" bind
        // string and live uppercase keypresses both resolve to Char('A')+SHIFT
        // once canonicalized. All three must be the same trie entry.
        let mut km = Keymap::default();
        km.bind_user_with_extend(
            BindMode::Normal,
            &[KeyEvent::new(KeyCode::Char('a'), Modifiers::SHIFT)],
            Cow::Borrowed("my-cmd"),
            false,
        );
        assert!(matches!(
            km.normal.walk(&[key!('A')]),
            WalkResult::Leaf(ref c) if c.name == "my-cmd"
        ));
    }
}
