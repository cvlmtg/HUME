use std::collections::{HashMap, VecDeque};

use crossterm::event::KeyEvent;

use editing::selection::SelectionSet;
use editing::text::Text;

// ── Register name constants ────────────────────────────────────────────────────
//
// HUME uses mnemonic single-char register names rather than the cryptic Vim/
// Helix convention (`"`, `+`, `_`). The key insight: 10 named registers (0-9)
// are enough for real workflows, freeing letters for intuitive special names.
//
// User-facing register names:
//   '0'–'9'  Named storage — text or macros (last write wins). Symmetric:
//            `"5y` and `"5p` use the same in-memory slot. Not kill-ring slots.
//   'k'      Kill-ring head. Paste reads the most-recently-pushed entry;
//            yank/delete/change push onto the ring (ring-only, no clipboard).
//            Older ring entries reachable only via `[`/`]` cycling.
//   'q'      Default macro register. `QQ` records, `qq` replays.
//            `Q3` records into register '3', `q3` replays from it.
//   'c'      System clipboard (requires OS integration).
//   'b'      Black hole — writes discarded, reads return None.
//   's'      Search register — last search pattern.
//
// Design intent: '0'–'9' are the deterministic, durable storage namespace —
// scripted/macro/surgical use (write once, read back verbatim regardless of
// intervening edits). 'k' + `[`/`]` address the kill ring, whose head shifts
// with every d/c/y — interactive reach-back only, not durable storage.
//
/// The kill-ring register (`k`) — paste reads the ring head; yank/delete/change push onto
/// the ring without touching the clipboard. Older ring entries are reachable via `[`/`]`.
pub(crate) const KILL_RING_REGISTER: char = 'k';

/// The black-hole register (`b`) — writes are silently discarded, reads return `None`.
/// Use `"by` to yank without touching the default register.
pub(crate) const BLACK_HOLE_REGISTER: char = 'b';

/// The search register (`s`) — holds the last search pattern.
/// Written by the search command; readable for paste into the command line.
pub(crate) const SEARCH_REGISTER: char = 's';

/// The default macro register (`q`).
/// `QQ` starts/stops recording into this register; `qq` replays from it.
/// Can also hold yanked text if the user explicitly writes to it.
pub(crate) const MACRO_REGISTER: char = 'q';

/// The system clipboard register (`c`).
/// Reads and writes the OS clipboard via `arboard`. Falls back to in-memory
/// storage with a warning when the clipboard is unavailable (headless CI/SSH).
pub(crate) const CLIPBOARD_REGISTER: char = 'c';

/// Returns `true` if `ch` is a valid register name for macro recording/replay.
///
/// Accepts the default macro register (`q`) and the numbered storage registers
/// (`0`–`9`). Special registers (`b`, `c`, `s`) are not valid macro targets.
pub(crate) fn is_valid_macro_register(ch: char) -> bool {
    ch == MACRO_REGISTER || ch.is_ascii_digit()
}

/// Returns `true` if `ch` is a valid register name for the `"<reg>` prefix.
///
/// Accepts the numbered storage registers (`0`–`9`), kill-ring head (`k`),
/// black hole (`b`), and clipboard (`c`). The default register (`"`), search
/// register (`s`), and macro register (`q`) are intentionally excluded — `q`
/// is written via `Q` recording, not the prefix; the others cannot be named.
pub fn is_valid_register_name(ch: char) -> bool {
    ch.is_ascii_digit()
        || ch == KILL_RING_REGISTER
        || ch == CLIPBOARD_REGISTER
        || ch == BLACK_HOLE_REGISTER
}

/// The content of a register — either yanked text or a recorded macro.
///
/// Registers are single-slot: the last write wins. Writing a macro to a register
/// that previously held text replaces it (and vice-versa).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RegisterContent {
    /// Yanked text — one `String` per selection that was active at yank time,
    /// in document order. A single-cursor yank produces a `Vec` of length 1.
    ///
    /// The linewise-vs-charwise distinction is not tracked explicitly; at paste
    /// time, content that ends with `\n` is treated as linewise.
    Text(Vec<String>),
    /// A recorded macro — the raw sequence of key events captured during recording.
    Macro(Vec<KeyEvent>),
}

/// One named register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Register {
    content: RegisterContent,
}

impl Register {
    fn new(content: RegisterContent) -> Self {
        Self { content }
    }

    /// If this register holds text, borrow the string slice. Returns `None` for macro registers.
    ///
    /// Callers that try to paste a macro register get `None` and treat it as a no-op.
    pub(crate) fn as_text(&self) -> Option<&[String]> {
        match &self.content {
            RegisterContent::Text(v) => Some(v),
            RegisterContent::Macro(_) => None,
        }
    }

    /// If this register holds a recorded macro, borrow the key slice. Returns `None` for text registers.
    pub(crate) fn as_macro(&self) -> Option<&[KeyEvent]> {
        match &self.content {
            RegisterContent::Macro(keys) => Some(keys),
            RegisterContent::Text(_) => None,
        }
    }
}

/// The full collection of named registers.
///
/// Each register holds a [`RegisterContent`] — either yanked text or a recorded macro.
///
/// Special registers (enforced here):
/// - `BLACK_HOLE_REGISTER` (`'b'`): writes discarded silently; reads return `None`.
///
/// Named registers `'0'`–`'9'` are user storage. Special registers `'c'`
/// (clipboard), `'s'` (search), and `'q'` (macro) are reserved by constants
/// above; their behaviour is wired in the editor layer.
#[derive(Debug, Clone, Default)]
pub(crate) struct RegisterSet {
    registers: HashMap<char, Register>,
}

impl RegisterSet {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Look up a register by name.
    ///
    /// Returns `None` for the black-hole register and for any register that has
    /// not been written yet.
    pub(crate) fn read(&self, name: char) -> Option<&Register> {
        if name == BLACK_HOLE_REGISTER {
            return None;
        }
        self.registers.get(&name)
    }

    /// Write text to a register, replacing its previous contents.
    ///
    /// Writes to the black-hole register (`'b'`) are silently discarded.
    pub(crate) fn write_text(&mut self, name: char, values: Vec<String>) {
        self.write(name, RegisterContent::Text(values));
    }

    /// Write a recorded macro to a register, replacing its previous contents.
    ///
    /// Writes to the black-hole register (`'b'`) are silently discarded.
    pub(crate) fn write_macro(&mut self, name: char, keys: Vec<KeyEvent>) {
        self.write(name, RegisterContent::Macro(keys));
    }

    fn write(&mut self, name: char, content: RegisterContent) {
        if name == BLACK_HOLE_REGISTER {
            return;
        }
        self.registers.insert(name, Register::new(content));
    }
}

// ── KillRing ──────────────────────────────────────────────────────────────────

/// Bounded ring buffer of deleted / yanked text entries.
///
/// Newest entry is always at index 0 (the "head"). Entries are accessed via
/// `"kp` (head) or by cycling with `[`/`]`. The digit registers `"0`–`"9`
/// are independent in-memory storage, not aliases for ring slots.
///
/// `cycle` is seeded by the paste command based on origin and persists until
/// the next paste re-seeds it; a lingering value between sessions is harmless
/// because every fresh paste re-seeds before reading the cursor.
#[derive(Debug, Clone, Default)]
pub(crate) struct KillRing {
    /// Entries newest-first; max `KILL_RING_DEPTH` entries.
    entries: VecDeque<Vec<String>>,
    /// Active `[`/`]` cycle position.
    /// `None` = clipboard / named-register origin (conceptually "before slot 0").
    /// `Some(n)` = currently showing slot `n`.
    cycle: Option<usize>,
}

/// Maximum number of entries the kill ring retains.
pub(crate) const KILL_RING_DEPTH: usize = 10;

impl KillRing {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Push a new entry to the head of the ring, evicting the oldest if full.
    pub(crate) fn push(&mut self, values: Vec<String>) {
        self.entries.push_front(values);
        if self.entries.len() > KILL_RING_DEPTH {
            self.entries.pop_back();
        }
    }

    /// Seed the cycle cursor based on paste origin.
    ///
    /// Call once per fresh paste:
    /// - `"kp` / smart-p ring-head paste → `Some(0)`
    /// - clipboard / named-register (`"0`–`"9`) paste → `None`
    pub(crate) fn seed_cycle(&mut self, pos: Option<usize>) {
        self.cycle = pos;
    }

    /// Borrow the head entry (most recently pushed), if any.
    pub(crate) fn head(&self) -> Option<&[String]> {
        self.entries.front().map(Vec::as_slice)
    }

    /// Borrow ring slot `n` (0-based), where 0 = head.
    #[cfg(test)]
    pub(crate) fn slot(&self, n: usize) -> Option<&[String]> {
        self.entries.get(n).map(Vec::as_slice)
    }

    /// Advance the cycle cursor one step older and return that entry.
    ///
    /// `None → 0`, `Some(n) → n+1`. Noop (returns `None`, leaves `cycle` unchanged)
    /// when the next slot would be out of bounds or the ring is empty (rule 27:
    /// every subsequent `[` past the oldest entry is a noop).
    pub(crate) fn cycle_older(&mut self) -> Option<&[String]> {
        let next = match self.cycle {
            None => 0,
            Some(n) => n + 1,
        };
        if next >= self.entries.len() {
            return None;
        }
        self.cycle = Some(next);
        self.entries.get(next).map(Vec::as_slice)
    }

    /// Retreat the cycle cursor one step newer and return that entry.
    ///
    /// Noop (returns `None`, leaves `cycle` unchanged) when `cycle` is `None` or
    /// `Some(0)` — there is nowhere newer to go (rule 28: every subsequent `]`
    /// at the head entry is a noop). Otherwise `Some(n) → Some(n-1)`.
    pub(crate) fn cycle_newer(&mut self) -> Option<&[String]> {
        let prev = match self.cycle {
            None | Some(0) => return None,
            Some(n) => n - 1,
        };
        self.cycle = Some(prev);
        self.entries.get(prev).map(Vec::as_slice)
    }

    /// Number of entries currently in the ring. Used in tests.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Extract the text of each selection from the buffer, in document order.
///
/// Returns one `String` per selection. This is the content that gets stored in
/// a register on yank or captured before a delete:
///
/// ```text
/// let yanked = yank_selections(&buf, &sels);
/// let (new_buf, new_sels, _cs) = delete_selection(buf, sels);
/// kill_ring.push(yanked);
/// ```
///
/// Selections are always inclusive, so the text spans `start()..=end()` —
/// internally `buf.slice(start..end+1)`.
pub(crate) fn yank_selections(buf: &Text, sels: &SelectionSet) -> Vec<String> {
    sels.iter_sorted()
        .map(|sel| {
            // end_inclusive() gives the last codepoint of the final grapheme
            // (handles multi-codepoint clusters like e + \u{0301}); +1 converts
            // to an exclusive upper bound for the slice.
            buf.slice(sel.start()..sel.end_inclusive(buf) + 1)
                .to_string()
        })
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::parse_state;

    // ── RegisterSet ───────────────────────────────────────────────────────────

    #[test]
    fn write_and_read() {
        let mut regs = RegisterSet::new();
        regs.write_text('"', vec!["hello".to_string()]);
        assert_eq!(
            regs.read('"').unwrap().as_text(),
            Some(vec!["hello".to_string()].as_slice())
        );
    }

    #[test]
    fn overwrite_replaces_previous() {
        let mut regs = RegisterSet::new();
        regs.write_text('0', vec!["first".to_string()]);
        regs.write_text('0', vec!["second".to_string()]);
        assert_eq!(
            regs.read('0').unwrap().as_text(),
            Some(vec!["second".to_string()].as_slice())
        );
    }

    #[test]
    fn read_empty_register_returns_none() {
        let regs = RegisterSet::new();
        assert!(regs.read('0').is_none());
    }

    #[test]
    fn black_hole_write_text_is_discarded() {
        let mut regs = RegisterSet::new();
        regs.write_text(BLACK_HOLE_REGISTER, vec!["ignored".to_string()]);
        assert!(regs.read(BLACK_HOLE_REGISTER).is_none());
    }

    #[test]
    fn black_hole_write_macro_is_discarded() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mut regs = RegisterSet::new();
        let keys = vec![KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)];
        regs.write_macro(BLACK_HOLE_REGISTER, keys);
        // The black-hole guard must apply to macro writes too.
        assert!(regs.read(BLACK_HOLE_REGISTER).is_none());
    }

    #[test]
    fn named_registers_are_independent() {
        let mut regs = RegisterSet::new();
        regs.write_text('1', vec!["one".to_string()]);
        regs.write_text('2', vec!["two".to_string()]);
        assert_eq!(
            regs.read('1').unwrap().as_text(),
            Some(vec!["one".to_string()].as_slice())
        );
        assert_eq!(
            regs.read('2').unwrap().as_text(),
            Some(vec!["two".to_string()].as_slice())
        );
    }

    #[test]
    fn write_macro_and_read_back() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mut regs = RegisterSet::new();
        let keys = vec![KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)];
        regs.write_macro('q', keys.clone());
        assert_eq!(regs.read('q').unwrap().as_macro(), Some(keys.as_slice()));
        // as_text() returns None for a macro register
        assert!(regs.read('q').unwrap().as_text().is_none());
    }

    #[test]
    fn macro_overwrites_text_last_write_wins() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mut regs = RegisterSet::new();
        regs.write_text('0', vec!["hello".to_string()]);
        let keys = vec![KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)];
        regs.write_macro('0', keys.clone());
        // now holds a macro, not text
        assert!(regs.read('0').unwrap().as_text().is_none());
        assert_eq!(regs.read('0').unwrap().as_macro(), Some(keys.as_slice()));
    }

    #[test]
    fn text_overwrites_macro_last_write_wins() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mut regs = RegisterSet::new();
        let keys = vec![KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)];
        regs.write_macro('0', keys);
        regs.write_text('0', vec!["text".to_string()]);
        // now holds text, not a macro
        assert!(regs.read('0').unwrap().as_macro().is_none());
        assert_eq!(
            regs.read('0').unwrap().as_text(),
            Some(vec!["text".to_string()].as_slice())
        );
    }

    #[test]
    fn constants_have_expected_values() {
        // Document the register name choices so a future reader sees them tested.
        assert_eq!(BLACK_HOLE_REGISTER, 'b');
        assert_eq!(SEARCH_REGISTER, 's');
        assert_eq!(MACRO_REGISTER, 'q');
        assert_eq!(CLIPBOARD_REGISTER, 'c');
    }

    // ── yank_selections ───────────────────────────────────────────────────────

    #[test]
    fn yank_single_cursor() {
        // Cursor on 'h' — yank captures just 'h'.
        let (buf, sels) = parse_state("-[h]>ello\n");
        assert_eq!(yank_selections(&buf, &sels), vec!["h"]);
    }

    #[test]
    fn yank_multi_char_selection() {
        // Selection covers "hell".
        let (buf, sels) = parse_state("-[hell]>o\n");
        assert_eq!(yank_selections(&buf, &sels), vec!["hell"]);
    }

    #[test]
    fn yank_backward_selection_same_text() {
        // Direction doesn't change the yanked text — it's always start()..=end().
        let (buf, sels) = parse_state("<[hell]-o\n");
        assert_eq!(yank_selections(&buf, &sels), vec!["hell"]);
    }

    #[test]
    fn yank_multi_cursor_document_order() {
        // Two cursors — one on 'h', one on 'o'. Returned in document order.
        let (buf, sels) = parse_state("-[h]>ell-[o]>\n");
        let yanked = yank_selections(&buf, &sels);
        assert_eq!(yanked, vec!["h", "o"]);
    }

    #[test]
    fn yank_full_line_including_newline() {
        // Selection covers "hello\n" — result ends with '\n' (linewise heuristic).
        let (buf, sels) = parse_state("-[hello\n]>");
        assert_eq!(yank_selections(&buf, &sels), vec!["hello\n"]);
    }

    #[test]
    fn yank_grapheme_cluster() {
        // "e\u{0301}" is two chars (e + combining acute) but one grapheme cluster.
        // A cursor on 'e' (pos 0) covers that grapheme — yank must include the
        // combining mark so the yanked text is the complete grapheme "é".
        let (buf, sels) = parse_state("-[e]>\u{0301}x\n");
        assert_eq!(yank_selections(&buf, &sels), vec!["e\u{0301}"]);
    }

    #[test]
    fn yank_on_structural_newline() {
        // Cursor on the trailing '\n' — captures the newline itself.
        let (buf, sels) = parse_state("hello-[\n]>");
        assert_eq!(yank_selections(&buf, &sels), vec!["\n"]);
    }

    #[test]
    fn yank_empty_buffer() {
        // Empty buffer is just "\n"; cursor on it — yank captures the newline.
        let (buf, sels) = parse_state("-[\n]>");
        assert_eq!(yank_selections(&buf, &sels), vec!["\n"]);
    }

    // ── KillRing ──────────────────────────────────────────────────────────────

    fn vs(s: &str) -> Vec<String> {
        vec![s.to_string()]
    }

    // -- push / head / slot -------------------------------------------------------

    #[test]
    fn kill_ring_push_head_eviction() {
        let mut ring = KillRing::new();
        for i in 0..15usize {
            ring.push(vs(&i.to_string()));
        }
        assert_eq!(ring.len(), KILL_RING_DEPTH);
        assert_eq!(ring.head(), Some(vs("14").as_slice()));
        assert_eq!(ring.slot(KILL_RING_DEPTH - 1), Some(vs("5").as_slice()));
    }

    #[test]
    fn kill_ring_head_empty() {
        let ring = KillRing::new();
        assert!(ring.head().is_none());
    }

    #[test]
    fn kill_ring_slot_access() {
        let mut ring = KillRing::new();
        ring.push(vs("a"));
        ring.push(vs("b"));
        ring.push(vs("c")); // head = slot 0
        assert_eq!(ring.slot(0), Some(vs("c").as_slice()));
        assert_eq!(ring.slot(1), Some(vs("b").as_slice()));
        assert_eq!(ring.slot(2), Some(vs("a").as_slice()));
        assert!(ring.slot(3).is_none());
    }

    // -- seed_cycle ---------------------------------------------------------------

    #[test]
    fn seed_cycle_sets_position() {
        let mut ring = KillRing::new();
        ring.push(vs("a"));
        ring.push(vs("b"));
        ring.seed_cycle(Some(0));
        assert_eq!(ring.cycle, Some(0));
        ring.seed_cycle(None);
        assert_eq!(ring.cycle, None);
    }

    // -- cycle_older ([) ----------------------------------------------------------

    #[test]
    fn cycle_older_from_none_reads_slot0() {
        // Clipboard origin (None) → first [ goes to slot 0 (ring head).
        let mut ring = KillRing::new();
        ring.push(vs("a")); // slot 0 = head
        ring.seed_cycle(None);
        assert_eq!(ring.cycle_older(), Some(vs("a").as_slice()));
        assert_eq!(ring.cycle, Some(0));
    }

    #[test]
    fn cycle_older_from_head_reads_slot1() {
        // Ring-head origin (Some(0)) → first [ goes to slot 1 (one older).
        let mut ring = KillRing::new();
        ring.push(vs("a")); // slot 1
        ring.push(vs("b")); // slot 0 = head
        ring.seed_cycle(Some(0));
        assert_eq!(ring.cycle_older(), Some(vs("a").as_slice()));
        assert_eq!(ring.cycle, Some(1));
    }

    #[test]
    fn cycle_older_noop_at_last_entry() {
        // Rule 27: [ is a noop when already at the oldest entry.
        let mut ring = KillRing::new();
        ring.push(vs("a")); // slot 1
        ring.push(vs("b")); // slot 0 = head
        ring.seed_cycle(Some(1)); // at last
        assert!(ring.cycle_older().is_none());
        assert_eq!(ring.cycle, Some(1)); // unchanged
    }

    #[test]
    fn cycle_older_noop_on_empty_ring() {
        let mut ring = KillRing::new();
        assert!(ring.cycle_older().is_none());
        assert_eq!(ring.cycle, None); // unchanged
    }

    // -- cycle_newer (]) ----------------------------------------------------------

    #[test]
    fn cycle_newer_noop_from_none() {
        // Rule 28: ] is a noop when there is no active cycle.
        let mut ring = KillRing::new();
        ring.push(vs("a"));
        ring.seed_cycle(None);
        assert!(ring.cycle_newer().is_none());
        assert_eq!(ring.cycle, None); // unchanged
    }

    #[test]
    fn cycle_newer_noop_at_head() {
        // Rule 28: ] is a noop when already at the head (slot 0).
        let mut ring = KillRing::new();
        ring.push(vs("a"));
        ring.push(vs("b"));
        ring.seed_cycle(Some(0));
        assert!(ring.cycle_newer().is_none());
        assert_eq!(ring.cycle, Some(0)); // unchanged
    }

    #[test]
    fn cycle_newer_retreats_toward_head() {
        let mut ring = KillRing::new();
        ring.push(vs("a")); // slot 2
        ring.push(vs("b")); // slot 1
        ring.push(vs("c")); // slot 0 = head
        ring.seed_cycle(Some(2));
        assert_eq!(ring.cycle_newer(), Some(vs("b").as_slice()));
        assert_eq!(ring.cycle, Some(1));
    }

    // -- round-trips --------------------------------------------------------------

    #[test]
    fn cycle_round_trip_clipboard_origin() {
        // Simulates rule 6: clipboard paste → [ → [ → noop.
        let mut ring = KillRing::new();
        ring.push(vs("charwise")); // slot 1
        ring.push(vs("linewise\n")); // slot 0 = head
        ring.seed_cycle(None); // clipboard origin

        assert_eq!(ring.cycle_older(), Some(vs("linewise\n").as_slice())); // slot 0
        assert_eq!(ring.cycle_older(), Some(vs("charwise").as_slice())); // slot 1
        assert!(ring.cycle_older().is_none()); // noop (at oldest)
        assert_eq!(ring.cycle, Some(1)); // unchanged after noop
    }

    #[test]
    fn cycle_round_trip_older_then_newer() {
        let mut ring = KillRing::new();
        ring.push(vs("a")); // slot 2
        ring.push(vs("b")); // slot 1
        ring.push(vs("c")); // slot 0 = head
        ring.seed_cycle(None);

        ring.cycle_older(); // None→0: "c"
        ring.cycle_older(); // 0→1:   "b"
        assert_eq!(ring.cycle_newer(), Some(vs("c").as_slice())); // 1→0: "c"
        assert_eq!(ring.cycle, Some(0));
        assert!(ring.cycle_newer().is_none()); // noop at head
    }

    // ── is_valid_register_name ────────────────────────────────────────────────

    #[test]
    fn valid_register_names_accepted() {
        for d in '0'..='9' {
            assert!(is_valid_register_name(d), "digit '{d}' must be valid");
        }
        assert!(is_valid_register_name(CLIPBOARD_REGISTER), "'c' must be valid");
        assert!(is_valid_register_name(BLACK_HOLE_REGISTER), "'b' must be valid");
        assert!(is_valid_register_name(KILL_RING_REGISTER), "'k' must be valid");
    }

    #[test]
    fn letter_a_is_not_a_valid_register_name() {
        // Regression guard: 'a' is not a valid register name —
        // is_valid_register_name must keep rejecting it.
        assert!(!is_valid_register_name('a'), "'a' must be invalid");
    }

    #[test]
    fn macro_and_search_registers_not_valid_for_prefix() {
        assert!(!is_valid_register_name(MACRO_REGISTER), "'q' not prefix-accessible");
        assert!(!is_valid_register_name(SEARCH_REGISTER), "'s' not prefix-accessible");
    }
}
