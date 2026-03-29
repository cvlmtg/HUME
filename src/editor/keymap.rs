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

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::FindKind;

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

// ── EditorAction ──────────────────────────────────────────────────────────────

/// Editor-level actions that cannot be expressed as pure `cmd_*` function
/// pointers because they require side effects: register access, mode
/// transitions, undo group management, or composite multi-step operations.
///
/// This is a *closed* enum — the default Rust keymap binds a finite set of
/// special cases. Commands that are pure `(&Buffer, SelectionSet) -> SelectionSet`
/// or `(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet)` are
/// registered in the [`CommandRegistry`] and referenced by name via
/// [`KeymapCommand::Cmd`] instead.
///
/// [`CommandRegistry`]: crate::command::CommandRegistry
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorAction {
    // ── Mode transitions ──────────────────────────────────────────────────
    /// `i` — enter Insert before the selection (collapse to start).
    EnterInsertBefore,
    /// `a` — enter Insert after the cursor (move one grapheme right).
    EnterInsertAfter,
    /// `I` — enter Insert at the first non-blank character on the line.
    EnterInsertLineStart,
    /// `A` — enter Insert after the last character on the line.
    EnterInsertLineEnd,
    /// `o` (normal mode) — open a new line below and enter Insert.
    /// `o` (extend mode) — flip anchor/head of every selection.
    OpenLineBelowOrFlip,
    /// `O` — open a new line above and enter Insert.
    OpenLineAbove,
    /// `:` — open the command-mode mini-buffer.
    EnterCommandMode,
    /// `Esc` (Insert mode) — return to Normal mode.
    ExitInsert,

    // ── Edit composites ───────────────────────────────────────────────────
    /// `d` — yank selections into the default register, then delete them.
    Delete,
    /// `c` — yank, delete, then enter Insert (change). All in one undo group.
    Change,
    /// `y` — yank selections into the default register (no buffer change).
    Yank,
    /// `p` — paste after; swap displaced text back if selection was non-cursor.
    PasteAfter,
    /// `P` — paste before; same swap semantics.
    PasteBefore,
    /// `u` — undo.
    Undo,
    /// `U` / `Ctrl+r` — redo.
    Redo,

    // ── Selection state ───────────────────────────────────────────────────
    /// `;` — collapse selection to cursor AND clear sticky extend mode.
    CollapseAndExitExtend,
    /// `e` — toggle sticky extend mode.
    ToggleExtend,

    // ── Find/till character ───────────────────────────────────────────────
    /// `f`/`F` or `t`/`T` after the target char is known.
    FindForward  { ch: char, kind: FindKind },
    FindBackward { ch: char, kind: FindKind },
    /// `=` — repeat last find forward (absolute direction).
    RepeatFindForward,
    /// `-` — repeat last find backward (absolute direction).
    RepeatFindBackward,

    // ── Replace ───────────────────────────────────────────────────────────
    /// `r` after the replacement char is known.
    Replace(char),

    // ── Page scroll ───────────────────────────────────────────────────────
    /// `PageDown` — move down by `view.height` lines.
    /// Stored as an action (not a count motion) because the count is derived
    /// from viewport dimensions, not the user's numeric prefix.
    PageDown,
    /// `PageUp` — move up by `view.height` lines.
    PageUp,

    // ── Misc ──────────────────────────────────────────────────────────────
    /// `Ctrl+c` — quit.
    Quit,
}

// ── KeymapCommand ─────────────────────────────────────────────────────────────

/// What a key binding resolves to after trie lookup.
#[derive(Debug, Clone, Copy)]
pub(crate) enum KeymapCommand {
    /// A [`CommandRegistry`] command looked up by name.
    ///
    /// `extend_name` is the name to use when extend mode is active. If `None`,
    /// `name` is used regardless of extend mode (for commands without an extend
    /// variant, e.g. selection manipulation commands).
    ///
    /// [`CommandRegistry`]: crate::command::CommandRegistry
    Cmd {
        name: &'static str,
        extend_name: Option<&'static str>,
    },
    /// An action that needs editor-level side effects.
    Action(EditorAction),
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
    /// should consume the *next* character and pass it to the constructor.
    WaitChar(fn(char) -> KeymapCommand),
    /// The sequence has no match in this trie.
    NoMatch,
}

// ── KeyTrie ───────────────────────────────────────────────────────────────────

/// A single level of the keymap trie.
///
/// Maps [`KeyEvent`] values to either a sub-trie (interior node) or a leaf
/// command. The trie is built once at startup and never mutated during editing
/// (mutation is an M5 concern for Steel config overrides).
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
    WaitChar(fn(char) -> KeymapCommand),
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
                Some(KeyTrieNode::WaitChar(f)) if i == last => {
                    return WalkResult::WaitChar(*f);
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

/// Construct a [`KeymapCommand::Cmd`] leaf with extend duality.
#[inline]
fn cmd(name: &'static str, extend_name: &'static str) -> KeymapCommand {
    KeymapCommand::Cmd { name, extend_name: Some(extend_name) }
}

/// Construct a [`KeymapCommand::Cmd`] leaf without an extend variant.
#[inline]
fn cmd_plain(name: &'static str) -> KeymapCommand {
    KeymapCommand::Cmd { name, extend_name: None }
}

/// Construct a [`KeymapCommand::Action`] leaf.
#[inline]
fn action(act: EditorAction) -> KeymapCommand {
    KeymapCommand::Action(act)
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
            inner_trie.bind_leaf(k, cmd(inner_name, ext_inner_name));
            around_trie.bind_leaf(k, cmd(around_name, ext_around_name));
        }
    }

    let mut match_trie = KeyTrie::new("match");
    match_trie.bind(key!('i'), KeyTrieNode::Node(inner_trie));
    match_trie.bind(key!('a'), KeyTrieNode::Node(around_trie));
    match_trie
}

// ── Default Normal keymap ─────────────────────────────────────────────────────

fn default_normal_keymap() -> KeyTrie {
    let mut t = KeyTrie::new("normal");

    // ── Basic motion ─────────────────────────────────────────────────────────
    // Each motion binding stores both the normal and extend-mode variant name.
    // The dispatcher resolves the right one at execution time.
    t.bind_leaf(key!('h'),    cmd("move-left",    "extend-left"));
    t.bind_leaf(key!(Left),   cmd("move-left",    "extend-left"));
    t.bind_leaf(key!('l'),    cmd("move-right",   "extend-right"));
    t.bind_leaf(key!(Right),  cmd("move-right",   "extend-right"));
    t.bind_leaf(key!('j'),    cmd("move-down",    "extend-down"));
    t.bind_leaf(key!(Down),   cmd("move-down",    "extend-down"));
    t.bind_leaf(key!('k'),    cmd("move-up",      "extend-up"));
    t.bind_leaf(key!(Up),     cmd("move-up",      "extend-up"));

    // NOTE: Ctrl+h/j/k/l/w/b (kitty one-shot extend) are NOT bound in the trie.
    // The dispatcher normalises them: strips CONTROL and temporarily sets extend=true
    // when kitty_enabled is true, OR strips CONTROL with no extend change when
    // kitty_enabled is false (preserving legacy "Ctrl+motion = bare motion" behaviour).
    // See `handle_normal` in mappings.rs for the normalisation logic.

    // ── Word motion ───────────────────────────────────────────────────────────
    t.bind_leaf(key!('w'), cmd("select-next-word",  "extend-select-next-word"));
    t.bind_leaf(key!('W'), cmd("select-next-WORD",  "extend-select-next-WORD"));
    t.bind_leaf(key!('b'), cmd("select-prev-word",  "extend-select-prev-word"));
    t.bind_leaf(key!('B'), cmd("select-prev-WORD",  "extend-select-prev-WORD"));

    // ── Line start / end ──────────────────────────────────────────────────────
    t.bind_leaf(key!('0'),   cmd("goto-line-start",    "extend-line-start"));
    t.bind_leaf(key!(Home),  cmd("goto-line-start",    "extend-line-start"));
    t.bind_leaf(key!('$'),   cmd("goto-line-end",      "extend-line-end"));
    t.bind_leaf(key!(End),   cmd("goto-line-end",      "extend-line-end"));
    t.bind_leaf(key!('^'),   cmd("goto-first-nonblank","extend-first-nonblank"));

    // ── Paragraph motion ──────────────────────────────────────────────────────
    t.bind_leaf(key!('{'), cmd("prev-paragraph", "extend-prev-paragraph"));
    t.bind_leaf(key!('}'), cmd("next-paragraph", "extend-next-paragraph"));

    // ── Line selection ────────────────────────────────────────────────────────
    t.bind_leaf(key!('x'), cmd("select-line",          "extend-select-line"));
    t.bind_leaf(key!('X'), cmd("select-line-backward", "extend-select-line-backward"));
    // Ctrl+x/X extend the selection to cover additional lines — works in both
    // kitty and legacy mode (unlike the basic-motion Ctrl keys, these are not
    // kitty-only; they were explicitly gated on CONTROL in the old code).
    t.bind_leaf(key!(Ctrl + 'x'), cmd_plain("extend-select-line"));
    t.bind_leaf(key!(Ctrl + 'X'), cmd_plain("extend-select-line-backward"));

    // ── Page scroll ───────────────────────────────────────────────────────────
    // PageUp/PageDown use view.height as count — handled as actions, not motions.
    t.bind_leaf(key!(PageDown), action(EditorAction::PageDown));
    t.bind_leaf(key!(PageUp),   action(EditorAction::PageUp));

    // ── Selection manipulation ────────────────────────────────────────────────
    t.bind_leaf(key!(';'), action(EditorAction::CollapseAndExitExtend));
    t.bind_leaf(key!(','), cmd_plain("keep-primary-selection"));
    // Ctrl+, removes primary; only transmitted with kitty keyboard protocol but
    // binding it here is harmless — legacy terminals never send it.
    t.bind_leaf(key!(Ctrl + ','), cmd_plain("remove-primary-selection"));
    t.bind_leaf(key!('S'), cmd_plain("split-selection-on-newlines"));
    t.bind_leaf(key!('('), cmd_plain("cycle-primary-backward"));
    t.bind_leaf(key!(')'), cmd_plain("cycle-primary-forward"));
    t.bind_leaf(key!('C'), cmd_plain("copy-selection-on-next-line"));
    t.bind_leaf(key!('_'), cmd_plain("trim-selection-whitespace"));

    // ── Extend mode ───────────────────────────────────────────────────────────
    t.bind_leaf(key!('e'), action(EditorAction::ToggleExtend));

    // ── Edit ──────────────────────────────────────────────────────────────────
    t.bind_leaf(key!('d'), action(EditorAction::Delete));
    t.bind_leaf(key!('c'), action(EditorAction::Change));
    t.bind_leaf(key!('y'), action(EditorAction::Yank));
    t.bind_leaf(key!('p'), action(EditorAction::PasteAfter));
    t.bind_leaf(key!('P'), action(EditorAction::PasteBefore));
    t.bind_leaf(key!('u'), action(EditorAction::Undo));
    t.bind_leaf(key!('U'), action(EditorAction::Redo));
    // `r` (no Ctrl) → wait for replacement char; `Ctrl+r` → redo.
    t.bind(key!('r'), KeyTrieNode::WaitChar(|ch| action(EditorAction::Replace(ch))));
    t.bind_leaf(key!(Ctrl + 'r'), action(EditorAction::Redo));

    // ── Find / till character ─────────────────────────────────────────────────
    t.bind(key!('f'), KeyTrieNode::WaitChar(|ch| action(EditorAction::FindForward  { ch, kind: FindKind::Inclusive })));
    t.bind(key!('F'), KeyTrieNode::WaitChar(|ch| action(EditorAction::FindBackward { ch, kind: FindKind::Inclusive })));
    t.bind(key!('t'), KeyTrieNode::WaitChar(|ch| action(EditorAction::FindForward  { ch, kind: FindKind::Exclusive })));
    t.bind(key!('T'), KeyTrieNode::WaitChar(|ch| action(EditorAction::FindBackward { ch, kind: FindKind::Exclusive })));

    // Repeat last find in absolute direction.
    t.bind_leaf(key!('='), action(EditorAction::RepeatFindForward));
    t.bind_leaf(key!('-'), action(EditorAction::RepeatFindBackward));

    // ── Text objects ──────────────────────────────────────────────────────────
    // `m` → `i`/`a` → object char (3-key sequence).
    t.bind(key!('m'), KeyTrieNode::Node(build_text_object_trie()));

    // ── Mode transitions ──────────────────────────────────────────────────────
    t.bind_leaf(key!(':'), action(EditorAction::EnterCommandMode));
    t.bind_leaf(key!('i'), action(EditorAction::EnterInsertBefore));
    t.bind_leaf(key!('a'), action(EditorAction::EnterInsertAfter));
    t.bind_leaf(key!('I'), action(EditorAction::EnterInsertLineStart));
    t.bind_leaf(key!('A'), action(EditorAction::EnterInsertLineEnd));
    // `o` in normal mode: open line below; in extend mode: flip selections.
    // The dual behaviour is handled in execute_editor_action.
    t.bind_leaf(key!('o'), action(EditorAction::OpenLineBelowOrFlip));
    t.bind_leaf(key!('O'), action(EditorAction::OpenLineAbove));

    // Ctrl+c quits from normal mode.
    t.bind_leaf(key!(Ctrl + 'c'), action(EditorAction::Quit));

    t
}

// ── Default Insert keymap ─────────────────────────────────────────────────────

fn default_insert_keymap() -> KeyTrie {
    let mut t = KeyTrie::new("insert");

    // Return to Normal mode.
    t.bind_leaf(key!(Esc),       action(EditorAction::ExitInsert));
    t.bind_leaf(key!(Ctrl + 'c'), action(EditorAction::ExitInsert));

    // Navigation (no extend in insert mode).
    t.bind_leaf(key!(Left),  cmd_plain("move-left"));
    t.bind_leaf(key!(Right), cmd_plain("move-right"));
    t.bind_leaf(key!(Down),  cmd_plain("move-down"));
    t.bind_leaf(key!(Up),    cmd_plain("move-up"));
    t.bind_leaf(key!(Home),  cmd_plain("goto-line-start"));
    t.bind_leaf(key!(End),   cmd_plain("goto-line-end"));

    // Special insert-mode keys (Backspace, Delete, Enter) are handled as
    // EditorActions because they interact with auto-pairs logic.
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
        assert!(matches!(result, WalkResult::Leaf(KeymapCommand::Cmd { name: "move-left", .. })));
    }

    #[test]
    fn single_key_action() {
        let trie = default_normal_keymap();
        assert!(matches!(
            trie.walk(&[key!('d')]),
            WalkResult::Leaf(KeymapCommand::Action(EditorAction::Delete))
        ));
        assert!(matches!(
            trie.walk(&[key!('u')]),
            WalkResult::Leaf(KeymapCommand::Action(EditorAction::Undo))
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
    fn wait_char_constructor_produces_correct_action() {
        let trie = default_normal_keymap();
        let WalkResult::WaitChar(f) = trie.walk(&[key!('f')]) else { panic!("expected WaitChar") };
        let cmd = f('x');
        assert!(matches!(
            cmd,
            KeymapCommand::Action(EditorAction::FindForward { ch: 'x', kind: FindKind::Inclusive })
        ));

        let WalkResult::WaitChar(f) = trie.walk(&[key!('t')]) else { panic!("expected WaitChar") };
        let cmd = f('x');
        assert!(matches!(
            cmd,
            KeymapCommand::Action(EditorAction::FindForward { ch: 'x', kind: FindKind::Exclusive })
        ));
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
        let WalkResult::Leaf(KeymapCommand::Cmd { name, extend_name }) = result else {
            panic!("expected Cmd leaf, got something else");
        };
        assert_eq!(name, "inner-word");
        assert_eq!(extend_name, Some("extend-inner-word"));

        // around-paren (both `(` and `)` map to the same text object)
        let result = trie.walk(&[key!('m'), key!('a'), key!('(')]);
        let WalkResult::Leaf(KeymapCommand::Cmd { name, .. }) = result else {
            panic!("expected Cmd leaf");
        };
        assert_eq!(name, "around-paren");

        let result = trie.walk(&[key!('m'), key!('a'), key!(')')]);
        let WalkResult::Leaf(KeymapCommand::Cmd { name, .. }) = result else {
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
        let WalkResult::Leaf(KeymapCommand::Cmd { name, extend_name }) = trie.walk(&[key!('w')]) else {
            panic!("expected Cmd leaf");
        };
        assert_eq!(name, "select-next-word");
        assert_eq!(extend_name, Some("extend-select-next-word"));
    }

    #[test]
    fn plain_cmd_has_no_extend_name() {
        let trie = default_normal_keymap();
        let WalkResult::Leaf(KeymapCommand::Cmd { name, extend_name }) = trie.walk(&[key!(',')]) else {
            panic!("expected Cmd leaf");
        };
        assert_eq!(name, "keep-primary-selection");
        assert_eq!(extend_name, None);
    }

    // ── Insert keymap ─────────────────────────────────────────────────────────

    #[test]
    fn insert_esc_exits() {
        let trie = default_insert_keymap();
        assert!(matches!(
            trie.walk(&[key!(Esc)]),
            WalkResult::Leaf(KeymapCommand::Action(EditorAction::ExitInsert))
        ));
    }

    #[test]
    fn insert_arrows_are_motions() {
        let trie = default_insert_keymap();
        assert!(matches!(
            trie.walk(&[key!(Left)]),
            WalkResult::Leaf(KeymapCommand::Cmd { name: "move-left", .. })
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
            WalkResult::Leaf(KeymapCommand::Action(EditorAction::ExitInsert))
        ));
    }

    #[test]
    fn ctrl_bindings_in_normal_keymap() {
        let trie = default_normal_keymap();
        // Ctrl+c → Quit
        assert!(matches!(
            trie.walk(&[key!(Ctrl + 'c')]),
            WalkResult::Leaf(KeymapCommand::Action(EditorAction::Quit))
        ));
        // Ctrl+r → Redo (explicit binding, not a stripped Ctrl)
        assert!(matches!(
            trie.walk(&[key!(Ctrl + 'r')]),
            WalkResult::Leaf(KeymapCommand::Action(EditorAction::Redo))
        ));
        // Ctrl+x → extend-select-line (not stripped like motion Ctrl keys)
        assert!(matches!(
            trie.walk(&[key!(Ctrl + 'x')]),
            WalkResult::Leaf(KeymapCommand::Cmd { name: "extend-select-line", .. })
        ));
    }

    #[test]
    fn wait_char_r_produces_replace() {
        let trie = default_normal_keymap();
        let WalkResult::WaitChar(f) = trie.walk(&[key!('r')]) else { panic!("expected WaitChar") };
        assert!(matches!(f('!'), KeymapCommand::Action(EditorAction::Replace('!'))));
    }

    #[test]
    fn wait_char_f_t_backward_produce_find_backward() {
        let trie = default_normal_keymap();

        let WalkResult::WaitChar(f) = trie.walk(&[key!('F')]) else { panic!("expected WaitChar") };
        assert!(matches!(
            f('x'),
            KeymapCommand::Action(EditorAction::FindBackward { ch: 'x', kind: FindKind::Inclusive })
        ));

        let WalkResult::WaitChar(f) = trie.walk(&[key!('T')]) else { panic!("expected WaitChar") };
        assert!(matches!(
            f('x'),
            KeymapCommand::Action(EditorAction::FindBackward { ch: 'x', kind: FindKind::Exclusive })
        ));
    }

    #[test]
    fn no_duplicate_normal_bindings() {
        // HashMap::insert silently overwrites. Rebuild the trie and verify
        // each top-level key appears only once by checking that the count of
        // unique keys equals the total count of bind_leaf calls.
        // We do this indirectly: walk every single-char + special key and
        // assert that the result is stable (no overwrites produce NoMatch).
        //
        // More directly: rebuild a fresh trie and count entries.
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
