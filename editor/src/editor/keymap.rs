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
//! The keymap stores only base command names. Extend-variant pairing lives in
//! the [`CommandRegistry`] — when extend mode is active, the dispatcher
//! resolves the extend variant automatically via
//! [`CommandRegistry::extend_variant`].
//!
//! [`CommandRegistry`]: super::registry::CommandRegistry
//!
//! # Wait-char bindings
//!
//! Keys like f/t/F/T/r produce a [`WaitCharPending`] that stores the command
//! name to dispatch. When the next character arrives, the dispatcher stores it
//! in `Editor.pending_char` and dispatches the named command. Extend-mode
//! resolution happens at char-consumption time via the registry.

use std::borrow::Cow;
use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
}

// ── WalkResult ────────────────────────────────────────────────────────────────

/// The outcome of walking a key sequence through a [`KeyTrie`].
pub(super) enum WalkResult {
    /// The sequence matches a leaf command — execute it.
    Leaf(KeymapCommand),
    /// At an interior trie node — more keys are needed.
    /// The `name` field names this node (e.g. `"match"`, `"goto"`) and will
    /// be shown in the statusline while the user completes the sequence.
    #[allow(dead_code)]
    Interior { name: &'static str },
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
pub(super) struct KeyTrie {
    /// Human-readable name shown in the statusline when the user is mid-sequence
    /// at this node (e.g. `"match"` after pressing `m`, `"goto"` after `g`).
    pub(super) name: &'static str,
    map: HashMap<KeyEvent, KeyTrieNode>,
}

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
        Self { name, map: HashMap::new() }
    }

    fn bind(&mut self, key: KeyEvent, node: KeyTrieNode) {
        self.map.insert(key, node);
    }

    fn bind_leaf(&mut self, key: KeyEvent, cmd: KeymapCommand) {
        self.bind(key, KeyTrieNode::Leaf(cmd));
    }

    /// Bind a multi-key sequence to a leaf command, creating interior nodes as
    /// needed. Single-key sequences insert directly as a `Leaf`.
    ///
    /// Called by [`Keymap::bind_user`] at runtime (e.g. from Steel config).
    #[allow(dead_code)]
    pub(crate) fn bind_sequence(&mut self, keys: &[KeyEvent], cmd: KeymapCommand) {
        debug_assert!(!keys.is_empty());
        if keys.len() == 1 {
            self.bind_leaf(keys[0], cmd);
            return;
        }
        let entry = self.map.entry(keys[0]).or_insert_with(|| {
            KeyTrieNode::Node(KeyTrie::new("user"))
        });
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
    #[allow(dead_code)]
    pub(crate) fn remove_sequence(&mut self, keys: &[KeyEvent]) {
        match keys {
            [] => {}
            [only] => { self.map.remove(only); }
            [first, rest @ ..] => {
                if let Some(KeyTrieNode::Node(sub)) = self.map.get_mut(first) {
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
            match current.map.get(key) {
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
}

// ── BindMode ─────────────────────────────────────────────────────────────────

/// Which keymap to apply a user-supplied binding to.
///
/// Used by [`Keymap::bind_user`] and [`Keymap::unbind_user`].
#[allow(dead_code)]
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
pub(crate) struct Keymap {
    pub(super) normal: KeyTrie,
    /// Sparse extend-mode overrides (e.g. `o → flip-selections`).
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
    /// Bind a key sequence to a command name in the given mode.
    ///
    /// Overwrites any existing binding for the same sequence. Single-key
    /// sequences are inserted as a `Leaf`; multi-key sequences create
    /// interior nodes as needed.
    ///
    /// `keys` must not be empty.
    #[allow(dead_code)]
    pub(crate) fn bind_user(&mut self, mode: BindMode, keys: &[KeyEvent], command: Cow<'static, str>) {
        debug_assert!(!keys.is_empty(), "bind_user called with empty key sequence");
        let trie = match mode {
            BindMode::Normal => &mut self.normal,
            BindMode::Extend => &mut self.extend,
            BindMode::Insert => &mut self.insert,
        };
        trie.bind_sequence(keys, KeymapCommand { name: command });
    }

    /// Remove a binding for a key sequence in the given mode.
    ///
    /// No-op if the sequence is not bound or any intermediate node is missing.
    #[allow(dead_code)]
    pub(crate) fn unbind_user(&mut self, mode: BindMode, keys: &[KeyEvent]) {
        let trie = match mode {
            BindMode::Normal => &mut self.normal,
            BindMode::Extend => &mut self.extend,
            BindMode::Insert => &mut self.insert,
        };
        trie.remove_sequence(keys);
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Construct a [`KeymapCommand`] from a command name string literal.
macro_rules! cmd {
    ($name:expr) => {
        KeymapCommand { name: Cow::Borrowed($name) }
    };
}

/// Construct a wait-char trie node from a command name string literal.
macro_rules! wait_char {
    ($cmd_name:expr) => {
        KeyTrieNode::WaitChar(WaitCharPending { cmd_name: Cow::Borrowed($cmd_name), ctrl_extend: false })
    };
}

// ── key! macro ────────────────────────────────────────────────────────────────

/// Construct a [`KeyEvent`] value concisely for use in keymap builders and tests.
///
/// ```rust,ignore
/// key!('w')           // Char('w'), no modifiers
/// key!(Ctrl + 'h')    // Char('h'), CONTROL modifier
/// key!(Esc)           // Esc, no modifiers
/// key!(Left)          // Left arrow, no modifiers
/// ```
macro_rules! key {
    // Ctrl+char — must come first so `Ctrl + 'h'` is not mistakenly parsed
    // by a later arm.
    (Ctrl + $ch:literal) => {
        KeyEvent::new(KeyCode::Char($ch), KeyModifiers::CONTROL)
    };
    // Named KeyCode variant: `key!(Esc)`, `key!(Left)`, `key!(Backspace)`, …
    // Rust macros dispatch by syntactic category: `Esc` is an *identifier*
    // (`$variant:ident`), while `'w'` is a *literal* (`$ch:literal`), so these
    // two arms never overlap even though they look similar.
    ($variant:ident) => {
        KeyEvent::new(KeyCode::$variant, KeyModifiers::NONE)
    };
    // Plain character literal
    ($ch:literal) => {
        KeyEvent::new(KeyCode::Char($ch), KeyModifiers::NONE)
    };
}

// ── Match / text-object trie ──────────────────────────────────────────────────

/// Build the sub-trie rooted at `m` (match commands).
///
/// Bindings:
///
/// ```text
/// m ─┬─ i ─┬─ w  → inner-word
///    │      ├─ (  → inner-paren
///    │      └─ …
///    ├─ a ─┬─ w  → around-word
///    │      ├─ (  → around-paren
///    │      └─ …
///    ├─ s ─┬─ (  → surround-paren
///    │      └─ …
///    └─ /       → select-all-matches
/// ```
fn build_text_object_trie() -> KeyTrie {
    // Table: (object chars, inner name, around name).
    // Extend-variant pairing lives in the registry, not here.
    #[rustfmt::skip]
    let objects: &[(&[char], &str, &str)] = &[
        // ── Word / WORD ───────────────────────────────────────────────────
        (&['w'],             "inner-word",         "around-word"),
        (&['W'],             "inner-WORD",         "around-WORD"),
        // ── Brackets ─────────────────────────────────────────────────────
        (&['(', ')'],        "inner-paren",        "around-paren"),
        (&['[', ']'],        "inner-bracket",      "around-bracket"),
        (&['{', '}'],        "inner-brace",        "around-brace"),
        (&['<', '>'],        "inner-angle",        "around-angle"),
        // ── Quotes ───────────────────────────────────────────────────────
        (&['"'],             "inner-double-quote", "around-double-quote"),
        (&['\''],            "inner-single-quote", "around-single-quote"),
        (&['`'],             "inner-backtick",     "around-backtick"),
        // ── Arguments ────────────────────────────────────────────────────
        (&['a'],             "inner-argument",     "around-argument"),
        // ── Line ─────────────────────────────────────────────────────────
        (&['l'],             "inner-line",         "around-line"),
    ];

    let mut inner_trie = KeyTrie::new("inner");
    let mut around_trie = KeyTrie::new("around");

    for (chars, inner_name, around_name) in objects {
        for &ch in *chars {
            let k = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
            inner_trie.bind_leaf(k, cmd!(inner_name));
            around_trie.bind_leaf(k, cmd!(around_name));
        }
    }

    // ── Surround sub-trie ────────────────────────────────────────────────
    // `ms` + char selects the surrounding delimiters as two cursor
    // selections, enabling select-then-act composition (e.g. `ms(` → `d`
    // to delete parens, `ms(` → `r[` to replace with brackets).
    #[rustfmt::skip]
    let surround_objects: &[(&[char], &str)] = &[
        (&['(', ')'], "surround-paren"),
        (&['[', ']'], "surround-bracket"),
        (&['{', '}'], "surround-brace"),
        (&['<', '>'], "surround-angle"),
        (&['"'],      "surround-double-quote"),
        (&['\''],     "surround-single-quote"),
        (&['`'],      "surround-backtick"),
    ];

    let mut surround_trie = KeyTrie::new("surround");
    for (chars, name) in surround_objects {
        for &ch in *chars {
            let k = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
            surround_trie.bind_leaf(k, cmd!(name));
        }
    }

    let mut match_trie = KeyTrie::new("match");
    match_trie.bind(key!('i'), KeyTrieNode::Node(inner_trie));
    match_trie.bind(key!('a'), KeyTrieNode::Node(around_trie));
    match_trie.bind(key!('s'), KeyTrieNode::Node(surround_trie));
    match_trie.bind_leaf(key!('/'), cmd!("select-all-matches"));
    match_trie
}

// ── Goto trie ─────────────────────────────────────────────────────────────────

/// Build the `g` sub-trie for goto commands.
///
/// ```text
/// g ─┬─ g  → goto-first-line
///    ├─ e  → goto-last-line
///    ├─ h  → goto-line-start
///    ├─ l  → goto-line-end
///    └─ s  → goto-first-nonblank
/// ```
fn build_goto_trie() -> KeyTrie {
    let mut t = KeyTrie::new("goto");
    t.bind_leaf(key!('g'), cmd!("goto-first-line"));
    t.bind_leaf(key!('e'), cmd!("goto-last-line"));
    t.bind_leaf(key!('h'), cmd!("goto-line-start"));
    t.bind_leaf(key!('l'), cmd!("goto-line-end"));
    t.bind_leaf(key!('s'), cmd!("goto-first-nonblank"));
    t
}

// ── Default Normal keymap ─────────────────────────────────────────────────────

fn default_normal_keymap() -> KeyTrie {
    let mut t = KeyTrie::new("normal");

    // ── Basic motion ─────────────────────────────────────────────────────────
    // The keymap stores only the base command name. Extend-variant pairing
    // lives in the registry — the dispatcher resolves it at execution time.
    t.bind_leaf(key!('h'),    cmd!("move-left"));
    t.bind_leaf(key!(Left),   cmd!("move-left"));
    t.bind_leaf(key!('l'),    cmd!("move-right"));
    t.bind_leaf(key!(Right),  cmd!("move-right"));
    t.bind_leaf(key!('j'),    cmd!("move-down"));
    t.bind_leaf(key!(Down),   cmd!("move-down"));
    t.bind_leaf(key!('k'),    cmd!("move-up"));
    t.bind_leaf(key!(Up),     cmd!("move-up"));

    // NOTE: Ctrl+h/j/k/l/w/b (kitty one-shot extend) are NOT bound in the trie.
    // The dispatcher normalises them: strips CONTROL and passes extend=true to
    // execute_keymap_command when kitty_enabled is true. Commands without an
    // extend variant in the registry are suppressed (no-op).
    // See `handle_normal` in mappings.rs for the normalisation logic.

    // ── Word motion ───────────────────────────────────────────────────────────
    t.bind_leaf(key!('w'), cmd!("select-next-word"));
    t.bind_leaf(key!('W'), cmd!("select-next-WORD"));
    t.bind_leaf(key!('b'), cmd!("select-prev-word"));
    t.bind_leaf(key!('B'), cmd!("select-prev-WORD"));

    // ── Line start / end ──────────────────────────────────────────────────────
    t.bind_leaf(key!('0'),   cmd!("goto-line-start"));
    t.bind_leaf(key!(Home),  cmd!("goto-line-start"));
    t.bind_leaf(key!('$'),   cmd!("goto-line-end"));
    t.bind_leaf(key!(End),   cmd!("goto-line-end"));
    t.bind_leaf(key!('^'),   cmd!("goto-first-nonblank"));

    // ── Paragraph motion ──────────────────────────────────────────────────────
    t.bind_leaf(key!('{'), cmd!("prev-paragraph"));
    t.bind_leaf(key!('}'), cmd!("next-paragraph"));

    // ── Line selection ────────────────────────────────────────────────────────
    t.bind_leaf(key!('x'), cmd!("select-line"));
    t.bind_leaf(key!('X'), cmd!("select-line-backward"));
    // Ctrl+x/X extend the selection to cover additional lines — works in both
    // kitty and legacy mode (unlike the basic-motion Ctrl keys, these are not
    // kitty-only; they were explicitly gated on CONTROL in the old code).
    t.bind_leaf(key!(Ctrl + 'x'), cmd!("extend-select-line"));
    t.bind_leaf(key!(Ctrl + 'X'), cmd!("extend-select-line-backward"));

    // ── Page scroll ───────────────────────────────────────────────────────────
    // PageUp/PageDown use view.height as count — handled by EditorCmd, not a
    // raw motion count. Extend duality is expressed in the normal way.
    t.bind_leaf(key!(PageDown), cmd!("page-down"));
    t.bind_leaf(key!(PageUp),   cmd!("page-up"));
    t.bind_leaf(key!(Ctrl + 'd'), cmd!("half-page-down"));
    t.bind_leaf(key!(Ctrl + 'u'), cmd!("half-page-up"));

    // ── Jump list ────────────────────────────────────────────────────────────
    t.bind_leaf(key!(Ctrl + 'o'), cmd!("jump-backward"));
    // Ctrl-i is traditionally Tab (0x09). Even with kitty keyboard protocol,
    // some terminals still report Ctrl-i as Tab rather than Char('i')+CONTROL.
    // Bind both so jump-forward works everywhere.
    t.bind_leaf(key!(Ctrl + 'i'), cmd!("jump-forward"));
    t.bind_leaf(key!(Tab), cmd!("jump-forward"));

    // ── Whole-buffer selection ────────────────────────────────────────────────
    t.bind_leaf(key!('%'), cmd!("select-all"));

    // ── Selection manipulation ────────────────────────────────────────────────
    t.bind_leaf(key!(';'), cmd!("collapse-and-exit-extend"));
    t.bind_leaf(key!(','), cmd!("keep-primary-selection"));
    // Ctrl+, removes primary; only transmitted with kitty keyboard protocol but
    // binding it here is harmless — legacy terminals never send it.
    t.bind_leaf(key!(Ctrl + ','), cmd!("remove-primary-selection"));
    t.bind_leaf(key!('S'), cmd!("split-selection-on-newlines"));
    t.bind_leaf(key!('('), cmd!("cycle-primary-backward"));
    t.bind_leaf(key!(')'), cmd!("cycle-primary-forward"));
    t.bind_leaf(key!('C'), cmd!("copy-selection-on-next-line"));
    t.bind_leaf(key!('_'), cmd!("trim-selection-whitespace"));

    // ── Extend mode ───────────────────────────────────────────────────────────
    t.bind_leaf(key!('e'), cmd!("toggle-extend"));

    // ── Edit ──────────────────────────────────────────────────────────────────
    t.bind_leaf(key!('d'), cmd!("delete"));
    t.bind_leaf(key!('c'), cmd!("change"));
    t.bind_leaf(key!('y'), cmd!("yank"));
    t.bind_leaf(key!('p'), cmd!("paste-after"));
    t.bind_leaf(key!('P'), cmd!("paste-before"));
    t.bind_leaf(key!('u'), cmd!("undo"));
    t.bind_leaf(key!('U'), cmd!("redo"));
    // `r` (no Ctrl) → wait for replacement char; `Ctrl+r` → redo.
    t.bind(key!('r'), wait_char!("replace"));
    t.bind_leaf(key!(Ctrl + 'r'), cmd!("redo"));

    // ── Find / till character ─────────────────────────────────────────────────
    // Each key waits for the next character, then dispatches the named command.
    // Extend duality is resolved at char-consumption time.
    t.bind(key!('f'), wait_char!("find-forward"));
    t.bind(key!('F'), wait_char!("find-backward"));
    t.bind(key!('t'), wait_char!("till-forward"));
    t.bind(key!('T'), wait_char!("till-backward"));

    // Repeat last find in absolute direction.
    t.bind_leaf(key!('='), cmd!("repeat-find-forward"));
    t.bind_leaf(key!('-'), cmd!("repeat-find-backward"));

    // Repeat last editing action.
    t.bind_leaf(key!('.'), cmd!("repeat-last-action"));

    // ── Search ────────────────────────────────────────────────────────────────
    // `/` opens forward search; `?` opens backward search.
    // `n` repeats in the original direction; `N` repeats in the opposite direction.
    // Both `n` and `N` have extend duality (keep anchor, move head).
    t.bind_leaf(key!('/'), cmd!("search-forward"));
    t.bind_leaf(key!('?'), cmd!("search-backward"));
    t.bind_leaf(key!('n'), cmd!("search-next"));
    t.bind_leaf(key!('N'), cmd!("search-prev"));
    t.bind_leaf(key!('s'), cmd!("select-within"));
    t.bind_leaf(key!('*'), cmd!("use-selection-as-search"));

    // ── Goto prefix ───────────────────────────────────────────────────────────
    // `g` → second key (goto commands, 2-key sequence).
    t.bind(key!('g'), KeyTrieNode::Node(build_goto_trie()));

    // ── Match prefix (`m`) ────────────────────────────────────────────────────
    // `m` → text objects (`mi`/`ma`), surround (`ms`), and `m/` (select-all-matches).
    t.bind(key!('m'), KeyTrieNode::Node(build_text_object_trie()));

    // ── Mode transitions ──────────────────────────────────────────────────────
    t.bind_leaf(key!(':'), cmd!("command-mode"));
    t.bind_leaf(key!('i'), cmd!("insert-before"));
    t.bind_leaf(key!('a'), cmd!("insert-after"));
    t.bind_leaf(key!('I'), cmd!("insert-at-line-start"));
    t.bind_leaf(key!('A'), cmd!("insert-at-line-end"));
    // `o` in normal mode: open line below.
    // `o` in extend mode: flip selections (extend pairing in the registry).
    t.bind_leaf(key!('o'), cmd!("open-line-below"));
    t.bind_leaf(key!('O'), cmd!("open-line-above"));

    // Ctrl+c quits from normal mode.
    t.bind_leaf(key!(Ctrl + 'c'), cmd!("force-quit"));

    t
}

// ── Default Extend keymap ─────────────────────────────────────────────────────

/// Sparse overrides active when the editor is in Extend mode.
///
/// Keys bound here dispatch their command directly (with `extend = false`),
/// bypassing the normal trie entirely. Keys *not* bound here fall through to
/// the normal trie with `extend = true` — the extend-variant resolution in
/// `execute_keymap_command` then applies as usual.
///
/// Default: `o → flip-selections` (mirrors Helix/Kakoune: `Alt+o` in extend
/// mode flips the selection direction).
fn default_extend_keymap() -> KeyTrie {
    let mut t = KeyTrie::new("extend");
    t.bind_leaf(key!('o'), cmd!("flip-selections"));
    t
}

// ── Default Insert keymap ─────────────────────────────────────────────────────

fn default_insert_keymap() -> KeyTrie {
    let mut t = KeyTrie::new("insert");

    // Return to Normal mode.
    t.bind_leaf(key!(Esc),        cmd!("exit-insert"));
    t.bind_leaf(key!(Ctrl + 'c'), cmd!("exit-insert"));

    // Navigation (no extend in insert mode).
    t.bind_leaf(key!(Left),  cmd!("move-left"));
    t.bind_leaf(key!(Right), cmd!("move-right"));
    t.bind_leaf(key!(Down),  cmd!("move-down"));
    t.bind_leaf(key!(Up),    cmd!("move-up"));
    t.bind_leaf(key!(Home),  cmd!("goto-line-start"));
    t.bind_leaf(key!(End),   cmd!("goto-line-end"));

    // Special insert-mode keys (Backspace, Delete, Enter) are handled directly
    // in handle_insert because they interact with auto-pairs logic.
    // Characters that are NOT in the trie fall through to char-insertion.

    t
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── bind_sequence / remove_sequence / bind_user / unbind_user ────────────

    #[test]
    fn bind_sequence_single_key() {
        let mut trie = KeyTrie::new("test");
        trie.bind_sequence(&[key!('z')], KeymapCommand { name: Cow::Borrowed("my-cmd") });
        assert!(matches!(trie.walk(&[key!('z')]), WalkResult::Leaf(ref c) if c.name == "my-cmd"));
    }

    #[test]
    fn bind_sequence_multi_key() {
        let mut trie = KeyTrie::new("test");
        trie.bind_sequence(
            &[key!('g'), key!('g')],
            KeymapCommand { name: Cow::Borrowed("goto-first-line") },
        );
        assert!(matches!(trie.walk(&[key!('g')]), WalkResult::Interior { .. }));
        assert!(matches!(
            trie.walk(&[key!('g'), key!('g')]),
            WalkResult::Leaf(ref c) if c.name == "goto-first-line"
        ));
    }

    #[test]
    fn bind_sequence_shadows_existing_leaf() {
        let mut trie = KeyTrie::new("test");
        // Bind `g` as a leaf first.
        trie.bind_sequence(&[key!('g')], KeymapCommand { name: Cow::Borrowed("old-cmd") });
        // Now bind `gg` — should convert `g` from Leaf to Node.
        trie.bind_sequence(
            &[key!('g'), key!('g')],
            KeymapCommand { name: Cow::Borrowed("new-cmd") },
        );
        assert!(matches!(trie.walk(&[key!('g')]), WalkResult::Interior { .. }));
        assert!(matches!(
            trie.walk(&[key!('g'), key!('g')]),
            WalkResult::Leaf(ref c) if c.name == "new-cmd"
        ));
    }

    #[test]
    fn remove_sequence_single_key() {
        let mut trie = KeyTrie::new("test");
        trie.bind_sequence(&[key!('z')], KeymapCommand { name: Cow::Borrowed("my-cmd") });
        trie.remove_sequence(&[key!('z')]);
        assert!(matches!(trie.walk(&[key!('z')]), WalkResult::NoMatch));
    }

    #[test]
    fn remove_sequence_multi_key() {
        let mut trie = KeyTrie::new("test");
        trie.bind_sequence(
            &[key!('g'), key!('g')],
            KeymapCommand { name: Cow::Borrowed("goto-first-line") },
        );
        trie.remove_sequence(&[key!('g'), key!('g')]);
        // Interior node for `g` remains; leaf `gg` is gone.
        assert!(matches!(trie.walk(&[key!('g')]), WalkResult::Interior { .. }));
        assert!(matches!(trie.walk(&[key!('g'), key!('g')]), WalkResult::NoMatch));
    }

    #[test]
    fn remove_sequence_nonexistent_is_noop() {
        let mut trie = KeyTrie::new("test");
        trie.bind_sequence(&[key!('z')], KeymapCommand { name: Cow::Borrowed("my-cmd") });
        trie.remove_sequence(&[key!('q')]); // q not bound — no-op
        trie.remove_sequence(&[key!('z'), key!('z')]); // path doesn't exist — no-op
        // `z` leaf is untouched.
        assert!(matches!(trie.walk(&[key!('z')]), WalkResult::Leaf(ref c) if c.name == "my-cmd"));
    }

    #[test]
    fn bind_user_normal_mode() {
        let mut km = Keymap::default();
        km.bind_user(BindMode::Normal, &[key!('z')], Cow::Borrowed("my-cmd"));
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
        km.bind_user(BindMode::Normal, &[key!('z')], Cow::Borrowed("my-cmd"));
        km.unbind_user(BindMode::Normal, &[key!('z')]);
        assert!(matches!(km.normal.walk(&[key!('z')]), WalkResult::NoMatch));
    }

    // ── Walk ─────────────────────────────────────────────────────────────────

    #[test]
    fn single_key_leaf() {
        let trie = default_normal_keymap();
        let result = trie.walk(&[key!('h')]);
        assert!(matches!(result, WalkResult::Leaf(ref cmd) if cmd.name == "move-left"));
    }

    #[test]
    fn single_key_editor_cmd() {
        let trie = default_normal_keymap();
        assert!(matches!(trie.walk(&[key!('d')]), WalkResult::Leaf(ref cmd) if cmd.name == "delete"));
        assert!(matches!(trie.walk(&[key!('u')]), WalkResult::Leaf(ref cmd) if cmd.name == "undo"));
        assert!(matches!(trie.walk(&[key!('i')]), WalkResult::Leaf(ref cmd) if cmd.name == "insert-before"));
    }

    #[test]
    fn wait_char_bindings() {
        let trie = default_normal_keymap();
        assert!(matches!(trie.walk(&[key!('f')]), WalkResult::WaitChar(_)));
        assert!(matches!(trie.walk(&[key!('t')]), WalkResult::WaitChar(_)));
        assert!(matches!(trie.walk(&[key!('F')]), WalkResult::WaitChar(_)));
        assert!(matches!(trie.walk(&[key!('T')]), WalkResult::WaitChar(_)));
        assert!(matches!(trie.walk(&[key!('r')]), WalkResult::WaitChar(_)));
    }

    #[test]
    fn wait_char_has_correct_names() {
        let trie = default_normal_keymap();

        let WalkResult::WaitChar(wc) = trie.walk(&[key!('f')]) else { panic!("expected WaitChar") };
        assert_eq!(wc.cmd_name, "find-forward");

        let WalkResult::WaitChar(wc) = trie.walk(&[key!('t')]) else { panic!("expected WaitChar") };
        assert_eq!(wc.cmd_name, "till-forward");

        let WalkResult::WaitChar(wc) = trie.walk(&[key!('F')]) else { panic!("expected WaitChar") };
        assert_eq!(wc.cmd_name, "find-backward");

        let WalkResult::WaitChar(wc) = trie.walk(&[key!('T')]) else { panic!("expected WaitChar") };
        assert_eq!(wc.cmd_name, "till-backward");

        let WalkResult::WaitChar(wc) = trie.walk(&[key!('r')]) else { panic!("expected WaitChar") };
        assert_eq!(wc.cmd_name, "replace");
    }

    #[test]
    fn multi_key_text_object_interior() {
        let trie = default_normal_keymap();
        // `m` alone → Interior at the match node.
        assert!(matches!(trie.walk(&[key!('m')]), WalkResult::Interior { name: "match" }));
        // `m`, `i` → Interior at the inner node.
        assert!(matches!(
            trie.walk(&[key!('m'), key!('i')]),
            WalkResult::Interior { name: "inner" }
        ));
        // `m`, `a` → Interior at the around node.
        assert!(matches!(
            trie.walk(&[key!('m'), key!('a')]),
            WalkResult::Interior { name: "around" }
        ));
    }

    #[test]
    fn multi_key_text_object_leaf() {
        let trie = default_normal_keymap();

        // inner-word
        let result = trie.walk(&[key!('m'), key!('i'), key!('w')]);
        let WalkResult::Leaf(KeymapCommand { name }) = result else {
            panic!("expected Cmd leaf, got something else");
        };
        assert_eq!(name, "inner-word");

        // around-paren (both `(` and `)` map to the same text object)
        let result = trie.walk(&[key!('m'), key!('a'), key!('(')]);
        let WalkResult::Leaf(KeymapCommand { name, .. }) = result else {
            panic!("expected Cmd leaf");
        };
        assert_eq!(name, "around-paren");

        let result = trie.walk(&[key!('m'), key!('a'), key!(')')]);
        let WalkResult::Leaf(KeymapCommand { name, .. }) = result else {
            panic!("expected Cmd leaf");
        };
        assert_eq!(name, "around-paren");
    }

    #[test]
    fn surround_trie_interior() {
        let trie = default_normal_keymap();
        // `m`, `s` → Interior at the surround node.
        assert!(matches!(
            trie.walk(&[key!('m'), key!('s')]),
            WalkResult::Interior { name: "surround" }
        ));
    }

    #[test]
    fn surround_trie_leaf() {
        let trie = default_normal_keymap();

        // surround-paren via `(`
        let result = trie.walk(&[key!('m'), key!('s'), key!('(')]);
        let WalkResult::Leaf(KeymapCommand { name, .. }) = result else {
            panic!("expected Cmd leaf");
        };
        assert_eq!(name, "surround-paren");

        // surround-paren via `)` (same command)
        let result = trie.walk(&[key!('m'), key!('s'), key!(')')]);
        let WalkResult::Leaf(KeymapCommand { name, .. }) = result else {
            panic!("expected Cmd leaf");
        };
        assert_eq!(name, "surround-paren");

        // surround-double-quote
        let result = trie.walk(&[key!('m'), key!('s'), key!('"')]);
        let WalkResult::Leaf(KeymapCommand { name, .. }) = result else {
            panic!("expected Cmd leaf");
        };
        assert_eq!(name, "surround-double-quote");
    }

    #[test]
    fn no_match() {
        let trie = default_normal_keymap();
        // `~` is not bound.
        assert!(matches!(trie.walk(&[key!('~')]), WalkResult::NoMatch));
        // `m` + `z` is not a valid text object sequence.
        assert!(matches!(
            trie.walk(&[key!('m'), key!('z')]),
            WalkResult::NoMatch
        ));
        // Too many keys for a leaf binding.
        assert!(matches!(
            trie.walk(&[key!('h'), key!('j')]),
            WalkResult::NoMatch
        ));
    }

    #[test]
    fn w_maps_to_select_next_word() {
        let trie = default_normal_keymap();
        let WalkResult::Leaf(cmd) = trie.walk(&[key!('w')]) else {
            panic!("expected Cmd leaf");
        };
        assert_eq!(cmd.name, "select-next-word");
    }

    #[test]
    fn comma_maps_to_keep_primary_selection() {
        let trie = default_normal_keymap();
        let WalkResult::Leaf(cmd) = trie.walk(&[key!(',')]) else {
            panic!("expected Cmd leaf");
        };
        assert_eq!(cmd.name, "keep-primary-selection");
    }

    #[test]
    fn o_maps_to_open_line_below() {
        let trie = default_normal_keymap();
        let WalkResult::Leaf(cmd) = trie.walk(&[key!('o')]) else {
            panic!("expected Cmd leaf");
        };
        assert_eq!(cmd.name, "open-line-below");
    }

    // ── Insert keymap ─────────────────────────────────────────────────────────

    #[test]
    fn insert_esc_exits() {
        let trie = default_insert_keymap();
        assert!(matches!(trie.walk(&[key!(Esc)]), WalkResult::Leaf(ref cmd) if cmd.name == "exit-insert"));
    }

    #[test]
    fn insert_arrows_are_motions() {
        let trie = default_insert_keymap();
        assert!(matches!(trie.walk(&[key!(Left)]), WalkResult::Leaf(ref cmd) if cmd.name == "move-left"));
    }

    #[test]
    fn insert_char_is_no_match() {
        // Regular characters are NOT in the insert trie — they fall through
        // to the char-insertion handler in the dispatcher.
        let trie = default_insert_keymap();
        assert!(matches!(trie.walk(&[key!('a')]), WalkResult::NoMatch));
        assert!(matches!(trie.walk(&[key!('z')]), WalkResult::NoMatch));
    }

    #[test]
    fn insert_ctrl_c_exits() {
        // Ctrl+c is an alternative exit key in insert mode (same as Esc).
        let trie = default_insert_keymap();
        assert!(matches!(trie.walk(&[key!(Ctrl + 'c')]), WalkResult::Leaf(ref cmd) if cmd.name == "exit-insert"));
    }

    #[test]
    fn ctrl_bindings_in_normal_keymap() {
        let trie = default_normal_keymap();
        assert!(matches!(trie.walk(&[key!(Ctrl + 'c')]), WalkResult::Leaf(ref cmd) if cmd.name == "force-quit"),
            "Ctrl+c should map to force-quit");
        assert!(matches!(trie.walk(&[key!(Ctrl + 'r')]), WalkResult::Leaf(ref cmd) if cmd.name == "redo"),
            "Ctrl+r should map to redo");
        assert!(matches!(trie.walk(&[key!(Ctrl + 'x')]), WalkResult::Leaf(ref cmd) if cmd.name == "extend-select-line"),
            "Ctrl+x should map to extend-select-line");
    }

    #[test]
    fn essential_keys_are_bound() {
        let trie = default_normal_keymap();
        // Spot-check a set of keys that would be ambiguous if duplicated.
        let must_be_bound = [
            key!('h'), key!('j'), key!('k'), key!('l'),
            key!('w'), key!('W'), key!('b'), key!('B'),
            key!('d'), key!('c'), key!('y'), key!('u'),
            key!('i'), key!('a'), key!('o'), key!('O'),
            key!('x'), key!('X'), key!('p'), key!('P'),
            key!('f'), key!('t'), key!('F'), key!('T'), key!('r'),
            key!('e'), key!(';'), key!(','),
            key!(Ctrl + 'c'), key!(Ctrl + 'r'), key!(Ctrl + 'x'),
            key!(Ctrl + 'o'), key!(Ctrl + 'i'), key!(Tab),
        ];
        for k in must_be_bound {
            assert!(
                !matches!(trie.walk(&[k]), WalkResult::NoMatch),
                "key {:?} unexpectedly unbound in normal keymap",
                k
            );
        }
    }
}
