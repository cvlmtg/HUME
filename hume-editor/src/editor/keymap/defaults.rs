use std::borrow::Cow;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{KeyTrie, KeyTrieNode, Keymap, KeymapCommand, WaitCharPending};

// ── Macros ────────────────────────────────────────────────────────────────────
// Lifted to `defaults.rs` (where they're primarily used).
// `#[macro_use] mod defaults` in mod.rs re-exports them into the keymap scope,
// making them available to mod.rs code and tests.

/// Construct a [`KeymapCommand`] from a command name string literal.
macro_rules! cmd {
    ($name:expr) => {
        KeymapCommand {
            name: Cow::Borrowed($name),
            force_extend: false,
        }
    };
}

/// Like `cmd!`, but marks the binding as always-extending (`force_extend =
/// true`). Use for explicit Ctrl+letter bindings whose extend semantics are
/// inherent regardless of kitty mode (e.g. `Ctrl+x` → `select-line`).
macro_rules! cmd_extend {
    ($name:expr) => {
        KeymapCommand {
            name: Cow::Borrowed($name),
            force_extend: true,
        }
    };
}

/// Construct a wait-char trie node from a command name string literal.
macro_rules! wait_char {
    ($cmd_name:expr) => {
        KeyTrieNode::WaitChar(WaitCharPending {
            cmd_name: Cow::Borrowed($cmd_name),
            ctrl_extend: false,
        })
    };
}

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
        (&['w'],             "inner-word",           "around-word"),
        (&['W'],             "inner-uppercase-word", "around-uppercase-word"),
        // ── Brackets ─────────────────────────────────────────────────────
        (&['(', ')'],        "inner-paren",          "around-paren"),
        (&['[', ']'],        "inner-bracket",        "around-bracket"),
        (&['{', '}'],        "inner-brace",          "around-brace"),
        (&['<', '>'],        "inner-angle",          "around-angle"),
        // ── Quotes ───────────────────────────────────────────────────────
        (&['"'],             "inner-double-quote",   "around-double-quote"),
        (&['\''],            "inner-single-quote",   "around-single-quote"),
        (&['`'],             "inner-backtick",       "around-backtick"),
        // ── Arguments ────────────────────────────────────────────────────
        (&['a'],             "inner-argument",       "around-argument"),
        // ── Line ─────────────────────────────────────────────────────────
        (&['l'],             "inner-line",           "around-line"),
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
    match_trie.bind(key!('w'), wait_char!("surround-add"));
    match_trie.bind_leaf(key!('/'), cmd!("select-all-matches"));
    // `mm` → `miw` — faster shortcut for selecting inner word.
    match_trie.bind_leaf(key!('m'), cmd!("inner-word"));
    match_trie
}

/// Build a standalone trie for the `M` prefix so that `MM` selects the
/// inner WORD without opening the full text-object trie.  This keeps `m`
/// and `M` as separate roots — `mM` and `Mm` are no-ops.
fn build_uppercase_match_trie() -> KeyTrie {
    let mut t = KeyTrie::new("match");
    t.bind_leaf(key!('M'), cmd!("inner-uppercase-word"));
    t
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

// ── Pane (Ctrl+p) sub-trie ───────────────────────────────────────────────────

fn build_pane_trie() -> KeyTrie {
    let mut t = KeyTrie::new("pane");
    t.bind_leaf(key!('p'), cmd!("pane-focus-next"));
    t.bind_leaf(key!('h'), cmd!("pane-focus-left"));
    t.bind_leaf(key!('j'), cmd!("pane-focus-down"));
    t.bind_leaf(key!('k'), cmd!("pane-focus-up"));
    t.bind_leaf(key!('l'), cmd!("pane-focus-right"));
    t.bind_leaf(key!('s'), cmd!("pane-split"));
    t.bind_leaf(key!('v'), cmd!("pane-vsplit"));
    t.bind_leaf(key!('c'), cmd!("pane-close"));
    t
}

// ── View (`z`) sub-trie ──────────────────────────────────────────────────────
//
// Vim-style viewport repositioning. `zz` centres the cursor row, `zt` puts it
// at the top, `zb` puts it at the bottom. Cursor position is unchanged.

fn build_view_trie() -> KeyTrie {
    let mut t = KeyTrie::new("view");
    t.bind_leaf(key!('z'), cmd!("center-view-on-cursor"));
    t.bind_leaf(key!('t'), cmd!("top-view-on-cursor"));
    t.bind_leaf(key!('b'), cmd!("bottom-view-on-cursor"));
    t
}

// ── Default Normal keymap ─────────────────────────────────────────────────────

pub(super) fn default_normal_keymap() -> KeyTrie {
    let mut t = KeyTrie::new("normal");

    // ── Basic motion ─────────────────────────────────────────────────────────
    // The keymap stores only the base command name. Extend-variant pairing
    // lives in the registry — the dispatcher resolves it at execution time.
    t.bind_leaf(key!('h'), cmd!("move-left"));
    t.bind_leaf(key!(Left), cmd!("move-left"));
    t.bind_leaf(key!('l'), cmd!("move-right"));
    t.bind_leaf(key!(Right), cmd!("move-right"));
    t.bind_leaf(key!('j'), cmd!("move-down"));
    t.bind_leaf(key!(Down), cmd!("move-down"));
    t.bind_leaf(key!('k'), cmd!("move-up"));
    t.bind_leaf(key!(Up), cmd!("move-up"));

    // NOTE: Ctrl+h/j/k/l/w/b (kitty one-shot extend) are NOT bound in the trie.
    // The dispatcher normalises them: strips CONTROL and passes extend=true to
    // execute_keymap_command when kitty_enabled is true. Commands without an
    // extend variant in the registry are suppressed (no-op). In legacy mode
    // these are a silent no-op.
    // See `handle_normal` in mappings.rs for the normalisation logic.
    // Ctrl+w is a kitty one-shot extend for `select-next-word`. The pane prefix is Ctrl+p.

    // ── Word motion ───────────────────────────────────────────────────────────
    t.bind_leaf(key!('w'), cmd!("select-next-word"));
    t.bind_leaf(key!('W'), cmd!("select-next-uppercase-word"));
    t.bind_leaf(key!('b'), cmd!("select-prev-word"));
    t.bind_leaf(key!('B'), cmd!("select-prev-uppercase-word"));

    // ── Line start / end ──────────────────────────────────────────────────────
    // `0`, `^`, and `$` are vim muscle memory, restored by the opt-in
    // `core:vim-keybind` plugin — native equivalents are `g h` / `g s` / `g l`.
    t.bind_leaf(key!(Home), cmd!("goto-line-start"));
    t.bind_leaf(key!(End), cmd!("goto-line-end"));

    // ── Paragraph motion ──────────────────────────────────────────────────────
    t.bind_leaf(key!('{'), cmd!("prev-paragraph"));
    t.bind_leaf(key!('}'), cmd!("next-paragraph"));

    // ── Line selection ────────────────────────────────────────────────────────
    t.bind_leaf(key!('x'), cmd!("select-line"));
    t.bind_leaf(key!('X'), cmd!("select-line-backward"));
    // Ctrl+x/X extend the selection to cover additional lines via force_extend,
    // so they work in both kitty and legacy mode.
    t.bind_leaf(key!(Ctrl + 'x'), cmd_extend!("select-line"));
    t.bind_leaf(key!(Ctrl + 'X'), cmd_extend!("select-line-backward"));

    // ── Page scroll ───────────────────────────────────────────────────────────
    // PageUp/PageDown use view.height as count — handled by EditorCmd, not a
    // raw motion count. Extend duality is expressed in the normal way.
    t.bind_leaf(key!(PageDown), cmd!("page-down"));
    t.bind_leaf(key!(PageUp), cmd!("page-up"));
    t.bind_leaf(key!(Ctrl + 'd'), cmd!("half-page-down"));
    t.bind_leaf(key!(Ctrl + 'u'), cmd!("half-page-up"));

    // ── Jump list ────────────────────────────────────────────────────────────
    t.bind_leaf(key!(Ctrl + 'o'), cmd!("jump-backward"));
    // Ctrl-i is traditionally Tab (0x09) on legacy terminals, which cannot
    // distinguish the two — so this default trie binds both to the same
    // command. Under the kitty keyboard protocol the two are always distinct
    // key events, and `apply_kitty_defaults` rebinds Tab to `pane-focus-next`
    // once the protocol is confirmed; jump-forward then stays reachable via
    // Ctrl-i alone (see that function's doc for why Ctrl-i can be trusted to
    // arrive as its own event under kitty).
    t.bind_leaf(key!(Ctrl + 'i'), cmd!("jump-forward"));
    t.bind_leaf(key!(Tab), cmd!("jump-forward"));

    // ── Whole-buffer selection ────────────────────────────────────────────────
    t.bind_leaf(key!('%'), cmd!("select-all"));

    // ── Selection manipulation ────────────────────────────────────────────────
    t.bind_leaf(key!(';'), cmd!("collapse-and-exit-extend"));
    t.bind_leaf(key!(','), cmd!("keep-primary-selection"));
    t.bind_leaf(key!('S'), cmd!("split-selection-on-newlines"));
    t.bind_leaf(key!('('), cmd!("cycle-primary-backward"));
    t.bind_leaf(key!(')'), cmd!("cycle-primary-forward"));
    t.bind_leaf(key!('C'), cmd!("copy-selection-on-next-line"));
    t.bind_leaf(key!('_'), cmd!("trim-selection-whitespace"));

    // ── Extend mode ───────────────────────────────────────────────────────────
    t.bind_leaf(key!('e'), cmd!("toggle-extend"));
    // Ctrl+e flips anchor↔head (same as `o` in Extend mode). Unlike Ctrl+;,
    // this emits a real control byte (0x05), so it works on legacy terminals
    // that don't support the kitty keyboard protocol. In Extend mode it falls
    // through to here with extend=true; flip-selections ignores MotionMode.
    t.bind_leaf(key!(Ctrl + 'e'), cmd!("flip-selections"));

    // ── Edit ──────────────────────────────────────────────────────────────────
    t.bind_leaf(key!('d'), cmd!("delete"));
    t.bind_leaf(key!('J'), cmd!("join-lines-select-spaces"));
    t.bind_leaf(key!('&'), cmd!("align-selections"));
    t.bind_leaf(key!('c'), cmd!("change"));
    t.bind_leaf(key!('y'), cmd!("yank"));
    t.bind_leaf(key!('p'), cmd!("paste-after"));
    t.bind_leaf(key!('P'), cmd!("paste-before"));
    // Kill-ring cycle: `[` walks older, `]` walks newer; each press also pastes.
    // These claim the bracket namespace (accepted design trade-off).
    t.bind_leaf(key!('['), cmd!("paste-ring-older"));
    t.bind_leaf(key!(']'), cmd!("paste-ring-newer"));
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

    // ── Pane prefix (Ctrl+p) ─────────────────────────────────────────────────
    // `Ctrl+p` → second key (pane navigation). Works in both kitty and legacy.
    // Ctrl+w is deliberately unbound here so that it falls through to the
    // kitty one-shot extend path (strip CONTROL → `w` → select-next-word).
    t.bind(key!(Ctrl + 'p'), KeyTrieNode::Node(build_pane_trie()));

    // ── Goto prefix ───────────────────────────────────────────────────────────
    // `g` → second key (goto commands, 2-key sequence).
    t.bind(key!('g'), KeyTrieNode::Node(build_goto_trie()));

    // ── View prefix ───────────────────────────────────────────────────────────
    // `z` → second key (zz/zt/zb viewport repositioning).
    t.bind(key!('z'), KeyTrieNode::Node(build_view_trie()));

    // ── Match prefix (`m` / `M`) ───────────────────────────────────────────────
    // `m` → text objects (`mi`/`ma`), surround (`ms`), and `m/` (select-all-matches).
    // `M` mirrors `m` and only contains `MM`, a shortcut for `miW` (inner WORD).
    t.bind(key!('m'), KeyTrieNode::Node(build_text_object_trie()));
    t.bind(key!('M'), KeyTrieNode::Node(build_uppercase_match_trie()));

    // ── Mode transitions ──────────────────────────────────────────────────────
    t.bind_leaf(key!(':'), cmd!("command-mode"));
    t.bind_leaf(key!('i'), cmd!("insert-at-selection-start"));
    t.bind_leaf(key!('a'), cmd!("insert-at-selection-end"));
    t.bind_leaf(key!('I'), cmd!("insert-at-line-start"));
    t.bind_leaf(key!('A'), cmd!("insert-at-line-end"));
    // `o` in normal mode: open line below.
    // `o` in extend mode: flip selections (extend pairing in the registry).
    t.bind_leaf(key!('o'), cmd!("open-line-below"));
    t.bind_leaf(key!('O'), cmd!("open-line-above"));

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
pub(super) fn default_extend_keymap() -> KeyTrie {
    let mut t = KeyTrie::new("extend");
    t.bind_leaf(key!('o'), cmd!("flip-selections"));
    t
}

// ── Default Insert keymap ─────────────────────────────────────────────────────

pub(super) fn default_insert_keymap() -> KeyTrie {
    let mut t = KeyTrie::new("insert");

    // Return to Normal mode.
    t.bind_leaf(key!(Esc), cmd!("exit-insert"));
    t.bind_leaf(key!(Ctrl + 'c'), cmd!("exit-insert"));

    // Navigation (no extend in insert mode).
    t.bind_leaf(key!(Left), cmd!("move-left"));
    t.bind_leaf(key!(Right), cmd!("move-right"));
    t.bind_leaf(key!(Down), cmd!("move-down"));
    t.bind_leaf(key!(Up), cmd!("move-up"));
    t.bind_leaf(key!(Home), cmd!("goto-line-start"));
    t.bind_leaf(key!(End), cmd!("goto-line-end"));

    t.bind_leaf(key!(Ctrl + 'w'), cmd!("delete-word-backward"));

    // Special insert-mode keys (Backspace, Delete, Enter) are handled directly
    // in handle_insert because they interact with auto-pairs logic.
    // Characters that are NOT in the trie fall through to char-insertion.

    t
}

// ── Kitty-only default binds ──────────────────────────────────────────────────

impl Keymap {
    /// Bind the default keys that are only delivered under the kitty keyboard
    /// protocol. Call this once after the kitty probe succeeds so the binds
    /// exist only when the terminal can actually produce them.
    pub fn apply_kitty_defaults(&mut self) {
        // Ctrl+; mirrors `;` but collapses to the anchor (the word's first char for
        // forward selections).
        self.normal
            .bind_leaf(key!(Ctrl + ';'), cmd!("collapse-to-anchor-and-exit-extend"));
        // Ctrl+, removes primary.
        self.normal
            .bind_leaf(key!(Ctrl + ','), cmd!("remove-primary-selection"));
        // Tab cycles panes. Disambiguating Ctrl-i from Tab (both 0x09 on
        // legacy terminals) is precisely what the kitty protocol exists to
        // do, so once the probe above has confirmed it, jump-forward stays
        // reachable via Ctrl-i — see the base keymap's jump-list binds.
        self.normal.bind_leaf(key!(Tab), cmd!("pane-focus-next"));
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::super::{Keymap, KeymapCommand, WalkResult};
    use super::{default_extend_keymap, default_insert_keymap, default_normal_keymap};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn single_key_leaf() {
        let trie = default_normal_keymap();
        let result = trie.walk(&[key!('h')]);
        assert!(matches!(result, WalkResult::Leaf(ref cmd) if cmd.name == "move-left"));
    }

    #[test]
    fn single_key_editor_cmd() {
        let trie = default_normal_keymap();
        assert!(
            matches!(trie.walk(&[key!('d')]), WalkResult::Leaf(ref cmd) if cmd.name == "delete")
        );
        assert!(matches!(trie.walk(&[key!('u')]), WalkResult::Leaf(ref cmd) if cmd.name == "undo"));
        assert!(
            matches!(trie.walk(&[key!('i')]), WalkResult::Leaf(ref cmd) if cmd.name == "insert-at-selection-start")
        );
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

        let WalkResult::WaitChar(wc) = trie.walk(&[key!('f')]) else {
            panic!("expected WaitChar")
        };
        assert_eq!(wc.cmd_name, "find-forward");

        let WalkResult::WaitChar(wc) = trie.walk(&[key!('t')]) else {
            panic!("expected WaitChar")
        };
        assert_eq!(wc.cmd_name, "till-forward");

        let WalkResult::WaitChar(wc) = trie.walk(&[key!('F')]) else {
            panic!("expected WaitChar")
        };
        assert_eq!(wc.cmd_name, "find-backward");

        let WalkResult::WaitChar(wc) = trie.walk(&[key!('T')]) else {
            panic!("expected WaitChar")
        };
        assert_eq!(wc.cmd_name, "till-backward");

        let WalkResult::WaitChar(wc) = trie.walk(&[key!('r')]) else {
            panic!("expected WaitChar")
        };
        assert_eq!(wc.cmd_name, "replace");
    }

    #[test]
    fn multi_key_text_object_interior() {
        let trie = default_normal_keymap();
        // `m` alone → Interior at the match node.
        assert!(matches!(
            trie.walk(&[key!('m')]),
            WalkResult::Interior { name: "match" }
        ));
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
        let WalkResult::Leaf(KeymapCommand { name, .. }) = result else {
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
        assert!(
            matches!(trie.walk(&[key!(Esc)]), WalkResult::Leaf(ref cmd) if cmd.name == "exit-insert")
        );
    }

    #[test]
    fn insert_arrows_are_motions() {
        let trie = default_insert_keymap();
        assert!(
            matches!(trie.walk(&[key!(Left)]), WalkResult::Leaf(ref cmd) if cmd.name == "move-left")
        );
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
        assert!(
            matches!(trie.walk(&[key!(Ctrl + 'c')]), WalkResult::Leaf(ref cmd) if cmd.name == "exit-insert")
        );
    }

    #[test]
    fn ctrl_bindings_in_normal_keymap() {
        let trie = default_normal_keymap();
        // Ctrl+c is intentionally unbound in normal mode — force-quit must be
        // invoked via :quit or :q to avoid accidental data loss.
        assert!(
            matches!(trie.walk(&[key!(Ctrl + 'c')]), WalkResult::NoMatch),
            "Ctrl+c must be unbound in normal mode"
        );
        assert!(
            matches!(trie.walk(&[key!(Ctrl + 'r')]), WalkResult::Leaf(ref cmd) if cmd.name == "redo"),
            "Ctrl+r should map to redo"
        );
        assert!(
            matches!(trie.walk(&[key!(Ctrl + 'x')]), WalkResult::Leaf(ref cmd) if cmd.name == "select-line"),
            "Ctrl+x should map to select-line"
        );
        // Ctrl+e flips anchor↔head and works on legacy terminals (0x05 control byte).
        assert!(
            matches!(trie.walk(&[key!(Ctrl + 'e')]), WalkResult::Leaf(ref cmd) if cmd.name == "flip-selections"),
            "Ctrl+e should map to flip-selections"
        );
        // Ctrl+w is deliberately unbound (kitty one-shot extend via strip-CONTROL).
        assert!(
            matches!(trie.walk(&[key!(Ctrl + 'w')]), WalkResult::NoMatch),
            "Ctrl+w must be unbound — pane prefix is Ctrl+p"
        );
        // Ctrl+p is the pane prefix (Interior node).
        assert!(
            matches!(trie.walk(&[key!(Ctrl + 'p')]), WalkResult::Interior { .. }),
            "Ctrl+p must be the pane prefix Interior node"
        );
    }

    /// Explicit Ctrl+x/X must carry force_extend=true; scroll and jump bindings must not.
    #[test]
    fn force_extend_flags_are_correct() {
        let trie = default_normal_keymap();

        let WalkResult::Leaf(cx) = trie.walk(&[key!(Ctrl + 'x')]) else {
            panic!()
        };
        assert!(
            cx.force_extend,
            "Ctrl+x (select-line) must have force_extend=true"
        );

        let WalkResult::Leaf(cx_upper) = trie.walk(&[key!(Ctrl + 'X')]) else {
            panic!()
        };
        assert!(
            cx_upper.force_extend,
            "Ctrl+X (select-line-backward) must have force_extend=true"
        );

        let WalkResult::Leaf(cd) = trie.walk(&[key!(Ctrl + 'd')]) else {
            panic!()
        };
        assert!(
            !cd.force_extend,
            "Ctrl+d (half-page-down) must have force_extend=false"
        );

        let WalkResult::Leaf(cu) = trie.walk(&[key!(Ctrl + 'u')]) else {
            panic!()
        };
        assert!(
            !cu.force_extend,
            "Ctrl+u (half-page-up) must have force_extend=false"
        );

        let WalkResult::Leaf(co) = trie.walk(&[key!(Ctrl + 'o')]) else {
            panic!()
        };
        assert!(
            !co.force_extend,
            "Ctrl+o (jump-backward) must have force_extend=false"
        );
    }

    #[test]
    fn essential_keys_are_bound() {
        let trie = default_normal_keymap();
        // Spot-check a set of keys that would be ambiguous if duplicated.
        let must_be_bound = [
            key!('h'),
            key!('j'),
            key!('k'),
            key!('l'),
            key!('w'),
            key!('W'),
            key!('b'),
            key!('B'),
            key!('d'),
            key!('c'),
            key!('y'),
            key!('u'),
            key!('i'),
            key!('a'),
            key!('o'),
            key!('O'),
            key!('x'),
            key!('X'),
            key!('p'),
            key!('P'),
            key!('f'),
            key!('t'),
            key!('F'),
            key!('T'),
            key!('r'),
            key!('e'),
            key!(';'),
            key!(','),
            key!(Ctrl + 'r'),
            key!(Ctrl + 'x'),
            key!(Ctrl + 'o'),
            key!(Ctrl + 'i'),
            key!(Tab),
            key!(Ctrl + 'p'), // pane prefix (Interior)
            key!(Ctrl + 'e'), // flip-selections
        ];
        for k in must_be_bound {
            assert!(
                !matches!(trie.walk(&[k]), WalkResult::NoMatch),
                "key {:?} unexpectedly unbound in normal keymap",
                k
            );
        }
    }

    #[test]
    fn extend_keymap_has_o_flip() {
        let trie = default_extend_keymap();
        assert!(
            matches!(trie.walk(&[key!('o')]), WalkResult::Leaf(ref cmd) if cmd.name == "flip-selections")
        );
    }

    // ── Kitty-only default binds ──────────────────────────────────────────────

    #[test]
    fn default_keymap_omits_kitty_only_binds() {
        let km = Keymap::default();
        // Ctrl+; and Ctrl+, must NOT be present in the legacy-accurate default
        // trie — they are installed by apply_kitty_defaults only when the kitty
        // probe succeeds.
        assert!(
            matches!(km.normal.walk(&[key!(Ctrl + ';')]), WalkResult::NoMatch),
            "Ctrl+; must be unbound in default keymap (legacy mode)"
        );
        assert!(
            matches!(km.normal.walk(&[key!(Ctrl + ',')]), WalkResult::NoMatch),
            "Ctrl+, must be unbound in default keymap (legacy mode)"
        );
        // The plain-key counterparts remain bound regardless of kitty mode.
        assert!(
            matches!(km.normal.walk(&[key!(';')]), WalkResult::Leaf(ref c) if c.name == "collapse-and-exit-extend")
        );
        assert!(
            matches!(km.normal.walk(&[key!(',')]), WalkResult::Leaf(ref c) if c.name == "keep-primary-selection")
        );
    }

    #[test]
    fn apply_kitty_defaults_binds_kitty_only_keys() {
        let mut km = Keymap::default();
        km.apply_kitty_defaults();
        let WalkResult::Leaf(c) = km.normal.walk(&[key!(Ctrl + ';')]) else {
            panic!("Ctrl+; should bind to collapse-to-anchor-and-exit-extend");
        };
        assert_eq!(c.name, "collapse-to-anchor-and-exit-extend");
        let WalkResult::Leaf(c) = km.normal.walk(&[key!(Ctrl + ',')]) else {
            panic!("Ctrl+, should bind to remove-primary-selection");
        };
        assert_eq!(c.name, "remove-primary-selection");
        let WalkResult::Leaf(c) = km.normal.walk(&[key!(Tab)]) else {
            panic!("Tab should bind to pane-focus-next under kitty defaults");
        };
        assert_eq!(c.name, "pane-focus-next");
    }
}
