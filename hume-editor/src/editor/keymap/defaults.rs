use std::borrow::Cow;

use termina::event::{KeyCode, KeyEvent, Modifiers};

use super::{KeyTrie, KeyTrieNode, Keymap, KeymapCommand, WaitCharPending};
use crate::editor::registry::STRUCTURAL_OBJECTS;

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
/// key!(Escape)        // Escape, no modifiers
/// key!(Left)          // Left arrow, no modifiers
/// ```
macro_rules! key {
    // Ctrl+char — must come first so `Ctrl + 'h'` is not mistakenly parsed
    // by a later arm.
    (Ctrl + $ch:literal) => {
        KeyEvent::new(KeyCode::Char($ch), Modifiers::CONTROL)
    };
    // Named KeyCode variant: `key!(Escape)`, `key!(Left)`, `key!(Backspace)`, …
    // Rust macros dispatch by syntactic category: `Escape` is an *identifier*
    // (`$variant:ident`), while `'w'` is a *literal* (`$ch:literal`), so these
    // two arms never overlap even though they look similar.
    ($variant:ident) => {
        KeyEvent::new(KeyCode::$variant, Modifiers::NONE)
    };
    // Plain character literal
    ($ch:literal) => {
        KeyEvent::new(KeyCode::Char($ch), Modifiers::NONE)
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
///    │      ├─ f  → inner-function
///    │      ├─ i  → select-last-insertion
///    │      └─ …
///    ├─ a ─┬─ w  → around-word
///    │      ├─ (  → around-paren
///    │      ├─ f  → around-function
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
        // ── Line ─────────────────────────────────────────────────────────
        (&['l'],             "inner-line",           "around-line"),
        // ── Paragraph ────────────────────────────────────────────────────
        (&['p'],             "inner-paragraph",      "around-paragraph"),
    ];

    let mut inner_trie = KeyTrie::new();
    let mut around_trie = KeyTrie::new();

    for (chars, inner_name, around_name) in objects {
        for &ch in *chars {
            let k = KeyEvent::new(KeyCode::Char(ch), Modifiers::NONE);
            inner_trie.bind_leaf(k, cmd!(inner_name));
            around_trie.bind_leaf(k, cmd!(around_name));
        }
    }
    // Structural kinds (function/class/argument/comment/unit-test/value) — one
    // table shared with `register_structural` (`registry/defaults/
    // structural.rs`), so a kind added there needs no change here. `a`
    // (argument) reuses the same two names the lexical scan registered
    // before this feature — see `StructuralObject`'s doc.
    for obj in STRUCTURAL_OBJECTS {
        let k = KeyEvent::new(KeyCode::Char(obj.key), Modifiers::NONE);
        inner_trie.bind_leaf(k, cmd!(obj.inner));
        around_trie.bind_leaf(k, cmd!(obj.around));
    }
    // `mii` — select the text typed during the last completed insert
    // session. No `mai` counterpart: an insertion has no "around" — there
    // are no delimiters or adjacent structure to widen into, unlike every
    // other object in `objects` above.
    inner_trie.bind_leaf(key!('i'), cmd!("select-last-insertion"));

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

    let mut surround_trie = KeyTrie::new();
    for (chars, name) in surround_objects {
        for &ch in *chars {
            let k = KeyEvent::new(KeyCode::Char(ch), Modifiers::NONE);
            surround_trie.bind_leaf(k, cmd!(name));
        }
    }

    let mut match_trie = KeyTrie::new();
    match_trie.bind(key!('i'), KeyTrieNode::Node(inner_trie));
    match_trie.bind(key!('a'), KeyTrieNode::Node(around_trie));
    match_trie.bind(key!('s'), KeyTrieNode::Node(surround_trie));
    match_trie.bind(key!('w'), wait_char!("surround-add"));
    match_trie.bind_leaf(key!('/'), cmd!("select-all-matches"));
    // `mm` — select the word under the cursor. Follows
    // `word-selects-whitespace` (see `cmd_select_word`'s `ctx.around`
    // branch), unlike `miw`/`maw` above which are never flag-affected.
    match_trie.bind_leaf(key!('m'), cmd!("select-word"));
    match_trie
}

/// Build a standalone trie for the `M` prefix so that `MM` selects the
/// uppercase WORD without opening the full text-object trie.  This keeps `m`
/// and `M` as separate roots — `mM` and `Mm` are no-ops.
fn build_uppercase_match_trie() -> KeyTrie {
    let mut t = KeyTrie::new();
    t.bind_leaf(key!('M'), cmd!("select-uppercase-word"));
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
///    ├─ s  → goto-first-nonblank
///    └─ <STRUCTURAL_OBJECTS' key/KEY>  → goto-next-<kind> / goto-prev-<kind>
/// ```
fn build_goto_trie() -> KeyTrie {
    let mut t = KeyTrie::new();
    t.bind_leaf(key!('g'), cmd!("goto-first-line"));
    t.bind_leaf(key!('e'), cmd!("goto-last-line"));
    t.bind_leaf(key!('h'), cmd!("goto-line-start"));
    t.bind_leaf(key!('l'), cmd!("goto-line-end"));
    t.bind_leaf(key!('s'), cmd!("goto-first-nonblank"));

    // Structural navigation: lowercase key → next, uppercase → previous.
    // Reuses the same `key` STRUCTURAL_OBJECTS assigns for `m i`/`m a` (see
    // that table's doc comment for why each kind's letter is what it is).
    // Can't use the `key!` macro here — it needs a literal token, and `key`
    // is a runtime `char` — so `KeyEvent` is built directly, the same
    // pattern `build_text_object_trie` uses below. Uppercasing `key` to
    // derive "previous" requires every entry to be lowercase and no two
    // uppercased forms to collide; both are asserted so a future table edit
    // can't silently reintroduce a collision.
    for obj in STRUCTURAL_OBJECTS {
        debug_assert!(
            obj.key.is_ascii_lowercase(),
            "STRUCTURAL_OBJECTS key {:?} must be lowercase — build_goto_trie derives \
             the \"previous\" bind by uppercasing it",
            obj.key
        );
        let next = KeyEvent::new(KeyCode::Char(obj.key), Modifiers::NONE);
        let prev = KeyEvent::new(KeyCode::Char(obj.key.to_ascii_uppercase()), Modifiers::NONE);
        t.bind_leaf(next, cmd!(obj.next));
        t.bind_leaf(prev, cmd!(obj.prev));
    }
    t
}

/// Build the `G` sub-trie: the commands Vim files under `g` that are *not*
/// gotos. `G L`/`G U`/`G C` are Vim's `gu`/`gU`/`g~`; `core:lsp` adds `G R`
/// (rename) on the same reasoning, since nvim's own LSP-rename default
/// (`grn`) is no more a goto than a case transform is. `g` stays reserved
/// for commands that name a destination.
fn build_transform_trie() -> KeyTrie {
    let mut t = KeyTrie::new();
    t.bind_leaf(key!('L'), cmd!("make-text-lowercase"));
    t.bind_leaf(key!('U'), cmd!("make-text-uppercase"));
    t.bind_leaf(key!('C'), cmd!("make-text-capitalized"));
    t
}

// ── Pane (Ctrl+p) sub-trie ───────────────────────────────────────────────────

fn build_pane_trie() -> KeyTrie {
    let mut t = KeyTrie::new();
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
// Viewport repositioning; the cursor itself never moves. The three leaves are
// laid out directionally rather than by Vim's initials: `z k` (up/top),
// `z z` (centre), `z j` (down/bottom), reusing the j/k axis every motion in
// the editor already trains. Vim's `zt`/`zb` are deliberately not aliased —
// `t` and `b` stay free under `z` for plugins (`core:pickers` claims `z b`).

fn build_view_trie() -> KeyTrie {
    let mut t = KeyTrie::new();
    t.bind_leaf(key!('z'), cmd!("center-view-on-cursor"));
    t.bind_leaf(key!('k'), cmd!("top-view-on-cursor"));
    t.bind_leaf(key!('j'), cmd!("bottom-view-on-cursor"));
    t
}

// ── Default Normal keymap ─────────────────────────────────────────────────────

pub(super) fn default_normal_keymap() -> KeyTrie {
    let mut t = KeyTrie::new();

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

    // Extend mode itself is an `e` toggle, not a held modifier — Ctrl+motion was
    // rejected as the universal extend modifier (fatal legacy-terminal collisions
    // on 10 of 15 motion keys; Ctrl-i/Tab below is one instance), and Alt was
    // rejected because it types accented characters on macOS. Kitty Ctrl+motion
    // (below) is a graceful bonus on top of that model, not the model itself.
    //
    // NOTE: Ctrl+h/j/k/l/w/b (kitty one-shot extend) are NOT bound in the trie.
    // The dispatcher normalises them: strips CONTROL and passes extend=true to
    // execute_keymap_command when kitty_enabled is true. Commands without an
    // extend variant in the registry are suppressed (no-op). In legacy mode
    // these are a silent no-op.
    // See `handle_normal` in `mappings/normal.rs` for the normalisation logic.
    // Ctrl+w is a kitty one-shot extend for `select-next-word`. The pane prefix is Ctrl+p.

    // ── Word motion ───────────────────────────────────────────────────────────
    t.bind_leaf(key!('w'), cmd!("select-next-word"));
    t.bind_leaf(key!('W'), cmd!("select-next-uppercase-word"));
    t.bind_leaf(key!('b'), cmd!("select-prev-word"));
    t.bind_leaf(key!('B'), cmd!("select-prev-uppercase-word"));

    // ── Line start / end ──────────────────────────────────────────────────────
    t.bind_leaf(key!(Home), cmd!("goto-line-start"));
    t.bind_leaf(key!(End), cmd!("goto-line-end"));

    // ── Paragraph motion ──────────────────────────────────────────────────────
    t.bind_leaf(key!('{'), cmd!("goto-prev-paragraph"));
    t.bind_leaf(key!('}'), cmd!("goto-next-paragraph"));

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

    // ── Matching bracket/tag ───────────────────────────────────────────────────
    // Vim's own key for this is `%`, but that's already select-all here (see
    // above). A bare key rather than a `g`-prefixed pair: `g` names a
    // destination (first line, definition, next function) while a
    // matching-pair jump names a relationship to wherever the cursor already
    // is, and it's pressed at motion frequency — worth a single keystroke.
    // `#` is unbound everywhere.
    t.bind_leaf(key!('#'), cmd!("goto-matching-pair"));

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
    // Ctrl+e flips anchor↔head, in both Normal and Extend mode. Unlike Ctrl+;,
    // this emits a real control byte (0x05), so it works on legacy terminals
    // that don't support the kitty keyboard protocol. In Extend mode it falls
    // through to here with extend=true; flip-selections ignores MotionMode.
    t.bind_leaf(key!(Ctrl + 'e'), cmd!("flip-selections"));

    // ── Edit ──────────────────────────────────────────────────────────────────
    t.bind_leaf(key!('d'), cmd!("delete"));
    t.bind_leaf(key!('J'), cmd!("join-lines-select-spaces"));
    t.bind_leaf(key!('&'), cmd!("align-selections"));
    t.bind_leaf(key!('>'), cmd!("indent"));
    t.bind_leaf(key!('<'), cmd!("unindent"));
    t.bind_leaf(key!('c'), cmd!("change"));
    t.bind_leaf(key!('y'), cmd!("yank"));
    t.bind_leaf(key!('p'), cmd!("smart-paste-after"));
    t.bind_leaf(key!('P'), cmd!("smart-paste-before"));
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
    t.bind_leaf(key!('*'), cmd!("search-word-under-cursor"));
    // Select text, then Ctrl+/ turns it into the search pattern verbatim (Helix's
    // `search_selection`), so `n`/`N` cycle its other occurrences. Kitty-only:
    // legacy terminals encode Ctrl+/ as the control byte 0x1F, which decodes
    // as `Ctrl+'7'` — left unbound, so the key silently no-ops there.
    t.bind_leaf(key!(Ctrl + '/'), cmd!("search-selection"));

    // ── Pane prefix (Ctrl+p) ─────────────────────────────────────────────────
    // `Ctrl+p` → second key (pane navigation). Works in both kitty and legacy.
    // Ctrl+w is deliberately unbound here so that it falls through to the
    // kitty one-shot extend path (strip CONTROL → `w` → select-next-word).
    t.bind(key!(Ctrl + 'p'), KeyTrieNode::Node(build_pane_trie()));

    // ── Goto prefix ───────────────────────────────────────────────────────────
    // `g` → second key (goto + structural-navigation commands, 2-key sequence).
    t.bind(key!('g'), KeyTrieNode::Node(build_goto_trie()));

    // ── `G` prefix ────────────────────────────────────────────────────────────
    // `G` → second key. Must stay a Node: `bind`/`bind_leaf` is a plain map
    // insert, so binding bare `G` to anything drops this whole subtree (and
    // `core:lsp`'s `G R` with it). See `build_transform_trie` for what `G` means.
    t.bind(key!('G'), KeyTrieNode::Node(build_transform_trie()));

    // ── View prefix ───────────────────────────────────────────────────────────
    // `z` → second key (`z k`/`z z`/`z j` viewport repositioning).
    t.bind(key!('z'), KeyTrieNode::Node(build_view_trie()));

    // ── Match prefix (`m` / `M`) ───────────────────────────────────────────────
    // `m` → text objects (`mi`/`ma`), surround (`ms`), and `m/` (select-all-matches).
    // `M` mirrors `m` and only contains `MM` (select-uppercase-word): the WORD
    // under the cursor, with the around body swapped in while
    // `word-selects-whitespace` is on — same gating as `mm`.
    t.bind(key!('m'), KeyTrieNode::Node(build_text_object_trie()));
    t.bind(key!('M'), KeyTrieNode::Node(build_uppercase_match_trie()));

    // ── Mode transitions ──────────────────────────────────────────────────────
    t.bind_leaf(key!(':'), cmd!("command-mode"));
    t.bind_leaf(key!('i'), cmd!("insert-at-selection-start"));
    t.bind_leaf(key!('a'), cmd!("insert-at-selection-end"));
    t.bind_leaf(key!('I'), cmd!("insert-at-line-start"));
    t.bind_leaf(key!('A'), cmd!("insert-at-line-end"));
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
/// Empty by default: `Ctrl+e` already flips selections in both Normal and
/// Extend mode (see `default_normal_keymap`), so no Extend-only override is
/// needed. Plugins (e.g. `core:vim-keybind`'s vim-style `o`) may add entries.
pub(super) fn default_extend_keymap() -> KeyTrie {
    KeyTrie::new()
}

// ── Default Insert keymap ─────────────────────────────────────────────────────

pub(super) fn default_insert_keymap() -> KeyTrie {
    let mut t = KeyTrie::new();

    // Return to Normal mode.
    t.bind_leaf(key!(Escape), cmd!("exit-insert"));
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
    pub(crate) fn apply_kitty_defaults(&mut self) {
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
mod tests;
