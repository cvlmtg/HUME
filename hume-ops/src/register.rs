use std::collections::VecDeque;

use rustc_hash::FxHashMap;
use termina::event::KeyEvent;

use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;

// ── Register name constants ────────────────────────────────────────────────────
//
// HUME uses mnemonic single-char register names — 10 named registers (0-9)
// cover real workflows, freeing letters for intuitive special names.
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
// '0'–'9' are durable storage; 'k' + `[`/`]` address the kill ring, whose
// head shifts with every d/c/y.
//
/// The kill-ring register (`k`) — see the module doc above for how it
/// differs from the durable `0`–`9` registers.
pub const KILL_RING_REGISTER: char = 'k';

/// The black-hole register (`b`) — writes are silently discarded, reads return `None`.
/// Use `"by` to yank without touching the default register.
pub const BLACK_HOLE_REGISTER: char = 'b';

/// The search register (`s`) — holds the last search pattern.
/// Written by the search command; readable for paste into the command line.
pub const SEARCH_REGISTER: char = 's';

/// The default macro register (`q`).
/// `QQ` starts/stops recording into this register; `qq` replays from it.
/// Can also hold yanked text if the user explicitly writes to it.
pub const MACRO_REGISTER: char = 'q';

/// The system clipboard register (`c`).
/// Reads and writes the OS clipboard via `arboard`. Falls back to in-memory
/// storage with a warning when the clipboard is unavailable (headless CI/SSH).
pub const CLIPBOARD_REGISTER: char = 'c';

/// Returns `true` if `ch` is a valid register name for macro recording/replay.
///
/// Accepts the default macro register (`q`) and the numbered storage registers
/// (`0`–`9`). Special registers (`b`, `c`, `s`) are not valid macro targets.
pub fn is_valid_macro_register(ch: char) -> bool {
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
pub enum RegisterContent {
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
pub struct Register {
    content: RegisterContent,
}

impl Register {
    fn new(content: RegisterContent) -> Self {
        Self { content }
    }

    /// If this register holds text, borrow the string slice. Returns `None` for macro registers.
    ///
    /// Callers that try to paste a macro register get `None` and treat it as a no-op.
    pub fn as_text(&self) -> Option<&[String]> {
        match &self.content {
            RegisterContent::Text(v) => Some(v),
            RegisterContent::Macro(_) => None,
        }
    }

    /// If this register holds a recorded macro, borrow the key slice. Returns `None` for text registers.
    pub fn as_macro(&self) -> Option<&[KeyEvent]> {
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
pub struct RegisterSet {
    registers: FxHashMap<char, Register>,
    /// Snapshot of the blob last written to the OS clipboard.
    /// Compared on read to detect external modifications: when the clipboard
    /// content matches this blob the in-memory `'c'` register is in sync and
    /// its structured `Vec<String>` (preserving multi-selection boundaries) is
    /// preferred over the flattened single-string OS clipboard representation.
    clipboard_blob: Option<String>,
}

impl RegisterSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a register by name.
    ///
    /// Returns `None` for the black-hole register and for any register that has
    /// not been written yet.
    pub fn read(&self, name: char) -> Option<&Register> {
        if name == BLACK_HOLE_REGISTER {
            return None;
        }
        self.registers.get(&name)
    }

    /// Write text to a register, replacing its previous contents.
    ///
    /// Writes to the black-hole register (`'b'`) are silently discarded.
    pub fn write_text(&mut self, name: char, values: Vec<String>) {
        self.write(name, RegisterContent::Text(values));
    }

    /// Write a recorded macro to a register, replacing its previous contents.
    ///
    /// Writes to the black-hole register (`'b'`) are silently discarded.
    pub fn write_macro(&mut self, name: char, keys: Vec<KeyEvent>) {
        self.write(name, RegisterContent::Macro(keys));
    }

    fn write(&mut self, name: char, content: RegisterContent) {
        if name == BLACK_HOLE_REGISTER {
            return;
        }
        self.registers.insert(name, Register::new(content));
    }

    pub fn clipboard_blob(&self) -> Option<&str> {
        self.clipboard_blob.as_deref()
    }

    pub fn set_clipboard_blob(&mut self, blob: String) {
        self.clipboard_blob = Some(blob);
    }
}

// ── KillRing ──────────────────────────────────────────────────────────────────

/// Bounded ring buffer of deleted / yanked text entries.
///
/// Newest entry is always at index 0 (the "head"). Entries are accessed via
/// `"kp` (head), by cycling with `[`/`]`, or by slot (a bare smart paste
/// resuming from wherever a prior cycle left off). The digit registers
/// `"0`–`"9` are independent in-memory storage, not aliases for ring slots.
/// The ring holds no two equal entries — [`KillRing::push`] moves a
/// re-captured entry to the head instead of duplicating it.
///
/// `cycle` is seeded by the paste command based on origin and persists until
/// the next paste re-seeds it; a lingering value between sessions is harmless
/// because every fresh paste re-seeds before reading the cursor.
#[derive(Debug, Clone, Default)]
pub struct KillRing {
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
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a new entry to the head of the ring, evicting the oldest if full.
    ///
    /// Dedupe: if an equal entry already exists elsewhere in the ring, it is
    /// removed before insertion — re-capturing the same text moves it to the
    /// head (a recency refresh) rather than taking a second slot. Checked
    /// before the whitespace collapse below: collapsing first would write
    /// `values` into the whitespace head while an equal older entry survived
    /// deeper in the ring, defeating the dedupe.
    ///
    /// Whitespace collapse: when the current head is a *pure whitespace* entry
    /// (every string, every char `is_whitespace`), the new entry overwrites
    /// it in place instead of taking a fresh slot. This keeps from filling the
    /// ring with entries you never want to cycle back to.
    pub fn push(&mut self, values: Vec<String>) {
        if let Some(pos) = self.entries.iter().position(|entry| *entry == values) {
            if pos == 0 {
                return;
            }
            self.entries.remove(pos);
        }
        let head_is_ws = self
            .entries
            .front()
            .is_some_and(|head| entry_is_whitespace(head));
        if head_is_ws && let Some(slot) = self.entries.front_mut() {
            *slot = values;
            return;
        }
        self.entries.push_front(values);
        if self.entries.len() > KILL_RING_DEPTH {
            self.entries.pop_back();
        }
    }

    /// Seed the cycle cursor based on paste origin.
    ///
    /// Call once per completed paste:
    /// - ring-sourced paste (`"kp`, or smart-p reading slot `n`) → `Some(n)`
    /// - clipboard / named-register (`"0`–`"9`) paste → `None`
    pub fn seed_cycle(&mut self, pos: Option<usize>) {
        self.cycle = pos;
    }

    /// Borrow the head entry (most recently pushed), if any.
    pub fn head(&self) -> Option<&[String]> {
        self.entries.front().map(Vec::as_slice)
    }

    /// Borrow ring slot `n` (0-based), where 0 = head.
    pub fn slot(&self, n: usize) -> Option<&[String]> {
        self.entries.get(n).map(Vec::as_slice)
    }

    /// Current `[`/`]` cycle position, if a session is active.
    pub fn cycle_position(&self) -> Option<usize> {
        self.cycle
    }

    /// Advance the cycle cursor one step older and return that entry.
    ///
    /// `None → 0`, `Some(n) → n+1`. Noop (returns `None`, leaves `cycle` unchanged)
    /// when the next slot would be out of bounds or the ring is empty (rule 27:
    /// every subsequent `[` past the oldest entry is a noop).
    pub fn cycle_older(&mut self) -> Option<&[String]> {
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
    pub fn cycle_newer(&mut self) -> Option<&[String]> {
        let prev = match self.cycle {
            None | Some(0) => return None,
            Some(n) => n - 1,
        };
        self.cycle = Some(prev);
        self.entries.get(prev).map(Vec::as_slice)
    }

    /// Number of entries currently in the ring. Used in tests, including
    /// `hume-editor`'s (a downstream crate) — see the `test-util` feature.
    ///
    /// No `is_empty` companion: every test caller checks a specific count
    /// (e.g. depth-capping), never emptiness — `head()` is the actual
    /// production-code check for "is there anything to paste".
    #[cfg(any(test, feature = "test-util"))]
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Whether a kill-ring entry is pure whitespace — every string in the entry,
/// every char `char::is_whitespace`. Used by [`KillRing::push`] to decide
/// whether to overwrite the head in place or take a fresh slot.
fn entry_is_whitespace(entry: &[String]) -> bool {
    entry.iter().all(|s| s.chars().all(char::is_whitespace))
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
pub fn yank_selections(buf: &Text, sels: &SelectionSet) -> Vec<String> {
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

/// Returns `true` if `text` represents linewise register content.
///
/// Linewise content always ends with `\n` because each selected line includes
/// its trailing newline, and the buffer invariant ensures even the last line
/// has one. Charwise/wordwise content does not.
///
/// This operates on *register/clipboard text* (paste time), not on a
/// selection. For the selection-geometry predicate see
/// `hume_editing::selection::is_selection_linewise`.
pub fn is_register_linewise(text: &str) -> bool {
    text.ends_with('\n')
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
