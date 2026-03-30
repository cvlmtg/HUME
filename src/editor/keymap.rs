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
//! Every motion/text-object binding stores a `name` (normal) and an optional
//! `extend_name`. When extend mode is active the dispatcher resolves the extend
//! variant automatically — no binding duplication needed.
//!
//! # Wait-char bindings
//!
//! Keys like f/t/F/T/r produce a [`WaitCharPending`] that stores the command
//! name to dispatch and an optional extend-mode variant. When the next character
//! arrives, the dispatcher stores it in `Editor.pending_char` and dispatches
//! the named command. Extend-mode resolution happens at char-consumption time.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

// ── WaitCharPending ───────────────────────────────────────────────────────────

/// State stored on the editor after a wait-char key (f/t/F/T/r).
///
/// On the next keypress the dispatcher stores the character in
/// `Editor.pending_char` and dispatches `cmd_name` (or `extend_name` when
/// extend mode is active).
#[derive(Debug, Clone, Copy)]
pub(crate) struct WaitCharPending {
    pub cmd_name: &'static str,
    pub extend_name: Option<&'static str>,
}

// ── KeymapCommand ─────────────────────────────────────────────────────────────

/// What a key binding resolves to after trie lookup.
///
/// Every binding — including composite editor operations — is expressed as
/// a `Cmd` referencing a name in the [`CommandRegistry`]. There is no parallel
/// `Action` escape hatch.
///
/// [`CommandRegistry`]: super::registry::CommandRegistry
#[derive(Debug, Clone, Copy)]
pub(crate) struct KeymapCommand {
    /// The command name to look up in the registry (normal mode).
    pub name: &'static str,
    /// The command name to use instead when extend mode is active.
    /// `None` means the same `name` is used regardless of extend state.
    pub extend_name: Option<&'static str>,
}

// ── WalkResult ────────────────────────────────────────────────────────────────

/// The outcome of walking a key sequence through a [`KeyTrie`].
pub(super) enum WalkResult {
    /// The sequence matches a leaf command — execute it.
    Leaf(KeymapCommand),
    /// At an interior trie node — more keys are needed.
    /// The `name` field names this node (e.g. `"match"`, `"goto"`) and will
    /// be shown in the status bar while the user completes the sequence.
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
    /// Human-readable name shown in the status bar when the user is mid-sequence
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
                    return WalkResult::Leaf(*cmd);
                }
                Some(KeyTrieNode::Leaf(_)) => {
                    // A leaf was reached before consuming all keys — the extra
                    // keys have no match.
                    return WalkResult::NoMatch;
                }
                Some(KeyTrieNode::WaitChar(wc)) if i == last => {
                    return WalkResult::WaitChar(*wc);
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

// ── Keymap ────────────────────────────────────────────────────────────────────

/// Per-mode keymap container. One instance lives on the [`Editor`].
///
/// [`Editor`]: super::Editor
pub(crate) struct Keymap {
    pub(super) normal: KeyTrie,
    pub(super) insert: KeyTrie,
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            normal: default_normal_keymap(),
            insert: default_insert_keymap(),
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Construct a [`KeymapCommand`].
///
/// - `cmd!("name", "extend-name")` — with extend duality.
/// - `cmd!("name")` — no extend variant (`extend_name` is `None`).
macro_rules! cmd {
    ($name:expr, $extend:expr) => {
        KeymapCommand { name: $name, extend_name: Some($extend) }
    };
    ($name:expr) => {
        KeymapCommand { name: $name, extend_name: None }
    };
}

/// Construct a wait-char trie node.
///
/// - `wait_char!("name", "extend-name")` — with extend duality.
/// - `wait_char!("name")` — no extend variant.
macro_rules! wait_char {
    ($cmd_name:expr, $extend:expr) => {
        KeyTrieNode::WaitChar(WaitCharPending { cmd_name: $cmd_name, extend_name: Some($extend) })
    };
    ($cmd_name:expr) => {
        KeyTrieNode::WaitChar(WaitCharPending { cmd_name: $cmd_name, extend_name: None })
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

// ── Text object trie ──────────────────────────────────────────────────────────

/// Build the sub-trie rooted at `m` for text object sequences.
///
/// The full sequence is `m` → `i`/`a` → object char. The returned trie
/// sits under the `m` key in the normal-mode keymap:
///
/// ```text
/// m ─┬─ i ─┬─ w  → inner-word
///    │      ├─ (  → inner-paren
///    │      └─ …
///    └─ a ─┬─ w  → around-word
///           ├─ (  → around-paren
///           └─ …
/// ```
fn build_text_object_trie() -> KeyTrie {
    // Table: (object char, inner name, extend-inner name, around name, extend-around name)
    #[rustfmt::skip]
    let objects: &[(&[char], &str, &str, &str, &str)] = &[
        // ── Word / WORD ───────────────────────────────────────────────────
        (&['w'],             "inner-word",         "extend-inner-word",         "around-word",         "extend-around-word"),
        (&['W'],             "inner-WORD",         "extend-inner-WORD",         "around-WORD",         "extend-around-WORD"),
        // ── Brackets ─────────────────────────────────────────────────────
        (&['(', ')'],        "inner-paren",        "extend-inner-paren",        "around-paren",        "extend-around-paren"),
        (&['[', ']'],        "inner-bracket",      "extend-inner-bracket",      "around-bracket",      "extend-around-bracket"),
        (&['{', '}'],        "inner-brace",        "extend-inner-brace",        "around-brace",        "extend-around-brace"),
        (&['<', '>'],        "inner-angle",        "extend-inner-angle",        "around-angle",        "extend-around-angle"),
        // ── Quotes ───────────────────────────────────────────────────────
        (&['"'],             "inner-double-quote", "extend-inner-double-quote", "around-double-quote", "extend-around-double-quote"),
        (&['\''],            "inner-single-quote", "extend-inner-single-quote", "around-single-quote", "extend-around-single-quote"),
        (&['`'],             "inner-backtick",     "extend-inner-backtick",     "around-backtick",     "extend-around-backtick"),
        // ── Arguments ────────────────────────────────────────────────────
        (&['a'],             "inner-argument",     "extend-inner-argument",     "around-argument",     "extend-around-argument"),
        // ── Line ─────────────────────────────────────────────────────────
        (&['l'],             "inner-line",         "extend-inner-line",         "around-line",         "extend-around-line"),
    ];

    let mut inner_trie = KeyTrie::new("inner");
    let mut around_trie = KeyTrie::new("around");

    for (chars, inner_name, ext_inner_name, around_name, ext_around_name) in objects {
        for &ch in *chars {
            let k = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
            inner_trie.bind_leaf(k, cmd!(inner_name, ext_inner_name));
            around_trie.bind_leaf(k, cmd!(around_name, ext_around_name));
        }
    }

    let mut match_trie = KeyTrie::new("match");
    match_trie.bind(key!('i'), KeyTrieNode::Node(inner_trie));
    match_trie.bind(key!('a'), KeyTrieNode::Node(around_trie));
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
    t.bind_leaf(key!('g'), cmd!("goto-first-line",     "extend-first-line"));
    t.bind_leaf(key!('e'), cmd!("goto-last-line",      "extend-last-line"));
    t.bind_leaf(key!('h'), cmd!("goto-line-start",     "extend-line-start"));
    t.bind_leaf(key!('l'), cmd!("goto-line-end",       "extend-line-end"));
    t.bind_leaf(key!('s'), cmd!("goto-first-nonblank", "extend-first-nonblank"));
    t
}

// ── Default Normal keymap ─────────────────────────────────────────────────────

fn default_normal_keymap() -> KeyTrie {
    let mut t = KeyTrie::new("normal");

    // ── Basic motion ─────────────────────────────────────────────────────────
    // Each motion binding stores both the normal and extend-mode variant name.
    // The dispatcher resolves the right one at execution time.
    t.bind_leaf(key!('h'),    cmd!("move-left",    "extend-left"));
    t.bind_leaf(key!(Left),   cmd!("move-left",    "extend-left"));
    t.bind_leaf(key!('l'),    cmd!("move-right",   "extend-right"));
    t.bind_leaf(key!(Right),  cmd!("move-right",   "extend-right"));
    t.bind_leaf(key!('j'),    cmd!("move-down",    "extend-down"));
    t.bind_leaf(key!(Down),   cmd!("move-down",    "extend-down"));
    t.bind_leaf(key!('k'),    cmd!("move-up",      "extend-up"));
    t.bind_leaf(key!(Up),     cmd!("move-up",      "extend-up"));

    // NOTE: Ctrl+h/j/k/l/w/b (kitty one-shot extend) are NOT bound in the trie.
    // The dispatcher normalises them: strips CONTROL and temporarily sets extend=true
    // when kitty_enabled is true, OR strips CONTROL with no extend change when
    // kitty_enabled is false (preserving legacy "Ctrl+motion = bare motion" behaviour).
    // See `handle_normal` in mappings.rs for the normalisation logic.

    // ── Word motion ───────────────────────────────────────────────────────────
    t.bind_leaf(key!('w'), cmd!("select-next-word",  "extend-select-next-word"));
    t.bind_leaf(key!('W'), cmd!("select-next-WORD",  "extend-select-next-WORD"));
    t.bind_leaf(key!('b'), cmd!("select-prev-word",  "extend-select-prev-word"));
    t.bind_leaf(key!('B'), cmd!("select-prev-WORD",  "extend-select-prev-WORD"));

    // ── Line start / end ──────────────────────────────────────────────────────
    t.bind_leaf(key!('0'),   cmd!("goto-line-start",    "extend-line-start"));
    t.bind_leaf(key!(Home),  cmd!("goto-line-start",    "extend-line-start"));
    t.bind_leaf(key!('$'),   cmd!("goto-line-end",      "extend-line-end"));
    t.bind_leaf(key!(End),   cmd!("goto-line-end",      "extend-line-end"));
    t.bind_leaf(key!('^'),   cmd!("goto-first-nonblank","extend-first-nonblank"));

    // ── Paragraph motion ──────────────────────────────────────────────────────
    t.bind_leaf(key!('{'), cmd!("prev-paragraph", "extend-prev-paragraph"));
    t.bind_leaf(key!('}'), cmd!("next-paragraph", "extend-next-paragraph"));

    // ── Line selection ────────────────────────────────────────────────────────
    t.bind_leaf(key!('x'), cmd!("select-line",          "extend-select-line"));
    t.bind_leaf(key!('X'), cmd!("select-line-backward", "extend-select-line-backward"));
    // Ctrl+x/X extend the selection to cover additional lines — works in both
    // kitty and legacy mode (unlike the basic-motion Ctrl keys, these are not
    // kitty-only; they were explicitly gated on CONTROL in the old code).
    t.bind_leaf(key!(Ctrl + 'x'), cmd!("extend-select-line"));
    t.bind_leaf(key!(Ctrl + 'X'), cmd!("extend-select-line-backward"));

    // ── Page scroll ───────────────────────────────────────────────────────────
    // PageUp/PageDown use view.height as count — handled by EditorCmd, not a
    // raw motion count. Extend duality is expressed in the normal way.
    t.bind_leaf(key!(PageDown), cmd!("page-down", "extend-page-down"));
    t.bind_leaf(key!(PageUp),   cmd!("page-up",   "extend-page-up"));

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
    t.bind(key!('f'), wait_char!("find-forward",  "extend-find-forward"));
    t.bind(key!('F'), wait_char!("find-backward", "extend-find-backward"));
    t.bind(key!('t'), wait_char!("till-forward",  "extend-till-forward"));
    t.bind(key!('T'), wait_char!("till-backward", "extend-till-backward"));

    // Repeat last find in absolute direction.
    t.bind_leaf(key!('='), cmd!("repeat-find-forward",  "extend-repeat-find-forward"));
    t.bind_leaf(key!('-'), cmd!("repeat-find-backward", "extend-repeat-find-backward"));

    // Repeat last editing action.
    t.bind_leaf(key!('.'), cmd!("repeat-last-action"));

    // ── Search ────────────────────────────────────────────────────────────────
    // `/` opens forward search; `?` opens backward search.
    // `n` repeats in the original direction; `N` repeats in the opposite direction.
    // Both `n` and `N` have extend duality (keep anchor, move head).
    t.bind_leaf(key!('/'), cmd!("search-forward"));
    t.bind_leaf(key!('?'), cmd!("search-backward"));
    t.bind_leaf(key!('n'), cmd!("search-next", "extend-search-next"));
    t.bind_leaf(key!('N'), cmd!("search-prev", "extend-search-prev"));

    // ── Goto prefix ───────────────────────────────────────────────────────────
    // `g` → second key (goto commands, 2-key sequence).
    t.bind(key!('g'), KeyTrieNode::Node(build_goto_trie()));

    // ── Text objects ──────────────────────────────────────────────────────────
    // `m` → `i`/`a` → object char (3-key sequence).
    t.bind(key!('m'), KeyTrieNode::Node(build_text_object_trie()));

    // ── Mode transitions ──────────────────────────────────────────────────────
    t.bind_leaf(key!(':'), cmd!("command-mode"));
    t.bind_leaf(key!('i'), cmd!("insert-before"));
    t.bind_leaf(key!('a'), cmd!("insert-after"));
    t.bind_leaf(key!('I'), cmd!("insert-at-line-start"));
    t.bind_leaf(key!('A'), cmd!("insert-at-line-end"));
    // `o` in normal mode: open line below.
    // `o` in extend mode: flip selections (the extend-duality mechanism handles this).
    t.bind_leaf(key!('o'), cmd!("open-line-below", "flip-selections"));
    t.bind_leaf(key!('O'), cmd!("open-line-above"));

    // Ctrl+c quits from normal mode.
    t.bind_leaf(key!(Ctrl + 'c'), cmd!("quit"));

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

    // ── Walk ─────────────────────────────────────────────────────────────────

    #[test]
    fn single_key_leaf() {
        let trie = default_normal_keymap();
        let result = trie.walk(&[key!('h')]);
        assert!(matches!(result, WalkResult::Leaf(KeymapCommand { name: "move-left", .. })));
    }

    #[test]
    fn single_key_editor_cmd() {
        let trie = default_normal_keymap();
        assert!(matches!(
            trie.walk(&[key!('d')]),
            WalkResult::Leaf(KeymapCommand { name: "delete", .. })
        ));
        assert!(matches!(
            trie.walk(&[key!('u')]),
            WalkResult::Leaf(KeymapCommand { name: "undo", .. })
        ));
        assert!(matches!(
            trie.walk(&[key!('i')]),
            WalkResult::Leaf(KeymapCommand { name: "insert-before", .. })
        ));
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
        assert_eq!(wc.extend_name, Some("extend-find-forward"));

        let WalkResult::WaitChar(wc) = trie.walk(&[key!('t')]) else { panic!("expected WaitChar") };
        assert_eq!(wc.cmd_name, "till-forward");
        assert_eq!(wc.extend_name, Some("extend-till-forward"));

        let WalkResult::WaitChar(wc) = trie.walk(&[key!('F')]) else { panic!("expected WaitChar") };
        assert_eq!(wc.cmd_name, "find-backward");
        assert_eq!(wc.extend_name, Some("extend-find-backward"));

        let WalkResult::WaitChar(wc) = trie.walk(&[key!('T')]) else { panic!("expected WaitChar") };
        assert_eq!(wc.cmd_name, "till-backward");
        assert_eq!(wc.extend_name, Some("extend-till-backward"));

        let WalkResult::WaitChar(wc) = trie.walk(&[key!('r')]) else { panic!("expected WaitChar") };
        assert_eq!(wc.cmd_name, "replace");
        assert_eq!(wc.extend_name, None);
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
        let WalkResult::Leaf(KeymapCommand { name, extend_name }) = result else {
            panic!("expected Cmd leaf, got something else");
        };
        assert_eq!(name, "inner-word");
        assert_eq!(extend_name, Some("extend-inner-word"));

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
    fn extend_name_stored_on_motion() {
        let trie = default_normal_keymap();
        let WalkResult::Leaf(KeymapCommand { name, extend_name }) = trie.walk(&[key!('w')]) else {
            panic!("expected Cmd leaf");
        };
        assert_eq!(name, "select-next-word");
        assert_eq!(extend_name, Some("extend-select-next-word"));
    }

    #[test]
    fn plain_cmd_has_no_extend_name() {
        let trie = default_normal_keymap();
        let WalkResult::Leaf(KeymapCommand { name, extend_name }) = trie.walk(&[key!(',')]) else {
            panic!("expected Cmd leaf");
        };
        assert_eq!(name, "keep-primary-selection");
        assert_eq!(extend_name, None);
    }

    #[test]
    fn o_has_extend_duality_for_flip() {
        // `o` maps to open-line-below normally, flip-selections in extend mode.
        let trie = default_normal_keymap();
        let WalkResult::Leaf(KeymapCommand { name, extend_name }) = trie.walk(&[key!('o')]) else {
            panic!("expected Cmd leaf");
        };
        assert_eq!(name, "open-line-below");
        assert_eq!(extend_name, Some("flip-selections"));
    }

    // ── Insert keymap ─────────────────────────────────────────────────────────

    #[test]
    fn insert_esc_exits() {
        let trie = default_insert_keymap();
        assert!(matches!(
            trie.walk(&[key!(Esc)]),
            WalkResult::Leaf(KeymapCommand { name: "exit-insert", .. })
        ));
    }

    #[test]
    fn insert_arrows_are_motions() {
        let trie = default_insert_keymap();
        assert!(matches!(
            trie.walk(&[key!(Left)]),
            WalkResult::Leaf(KeymapCommand { name: "move-left", .. })
        ));
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
        assert!(matches!(
            trie.walk(&[key!(Ctrl + 'c')]),
            WalkResult::Leaf(KeymapCommand { name: "exit-insert", .. })
        ));
    }

    #[test]
    fn ctrl_bindings_in_normal_keymap() {
        let trie = default_normal_keymap();
        // Ctrl+c → quit
        assert!(matches!(
            trie.walk(&[key!(Ctrl + 'c')]),
            WalkResult::Leaf(KeymapCommand { name: "quit", .. })
        ));
        // Ctrl+r → redo (explicit binding, not a stripped Ctrl)
        assert!(matches!(
            trie.walk(&[key!(Ctrl + 'r')]),
            WalkResult::Leaf(KeymapCommand { name: "redo", .. })
        ));
        // Ctrl+x → extend-select-line (not stripped like motion Ctrl keys)
        assert!(matches!(
            trie.walk(&[key!(Ctrl + 'x')]),
            WalkResult::Leaf(KeymapCommand { name: "extend-select-line", .. })
        ));
    }

    #[test]
    fn no_duplicate_normal_bindings() {
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
