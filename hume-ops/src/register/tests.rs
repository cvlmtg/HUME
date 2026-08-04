use super::*;
use hume_test_fixtures::testing::parse_state;

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
    use termina::event::{KeyCode, Modifiers};
    let mut regs = RegisterSet::new();
    let keys = vec![KeyEvent::new(KeyCode::Char('j'), Modifiers::NONE)];
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
    use termina::event::{KeyCode, Modifiers};
    let mut regs = RegisterSet::new();
    let keys = vec![KeyEvent::new(KeyCode::Char('j'), Modifiers::NONE)];
    regs.write_macro('q', keys.clone());
    assert_eq!(regs.read('q').unwrap().as_macro(), Some(keys.as_slice()));
    // as_text() returns None for a macro register
    assert!(regs.read('q').unwrap().as_text().is_none());
}

#[test]
fn macro_overwrites_text_last_write_wins() {
    use termina::event::{KeyCode, Modifiers};
    let mut regs = RegisterSet::new();
    regs.write_text('0', vec!["hello".to_string()]);
    let keys = vec![KeyEvent::new(KeyCode::Char('j'), Modifiers::NONE)];
    regs.write_macro('0', keys.clone());
    // now holds a macro, not text
    assert!(regs.read('0').unwrap().as_text().is_none());
    assert_eq!(regs.read('0').unwrap().as_macro(), Some(keys.as_slice()));
}

#[test]
fn text_overwrites_macro_last_write_wins() {
    use termina::event::{KeyCode, Modifiers};
    let mut regs = RegisterSet::new();
    let keys = vec![KeyEvent::new(KeyCode::Char('j'), Modifiers::NONE)];
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
fn search_register_round_trip() {
    let mut regs = RegisterSet::new();
    regs.set_search_register("hello".to_string());
    assert_eq!(regs.search_register(), Some("hello"));
}

#[test]
fn search_register_none_when_unset() {
    let regs = RegisterSet::new();
    assert_eq!(regs.search_register(), None);
}

#[test]
fn search_register_none_when_slot_holds_macro() {
    use termina::event::{KeyCode, Modifiers};
    let mut regs = RegisterSet::new();
    let keys = vec![KeyEvent::new(KeyCode::Char('j'), Modifiers::NONE)];
    regs.write_macro(SEARCH_REGISTER, keys);
    assert_eq!(regs.search_register(), None);
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

// -- push: whitespace collapse ------------------------------------------------

#[test]
fn push_overwrites_whitespace_head() {
    let mut ring = KillRing::new();
    ring.push(vs(" "));
    ring.push(vs("x"));
    assert_eq!(ring.head(), Some(vs("x").as_slice()));
    assert_eq!(ring.len(), 1);
}

#[test]
fn push_overwrites_single_newline_head() {
    let mut ring = KillRing::new();
    ring.push(vs("\n"));
    ring.push(vs("x"));
    assert_eq!(ring.head(), Some(vs("x").as_slice()));
    assert_eq!(ring.len(), 1);
}

#[test]
fn push_overwrites_all_whitespace_multi_string_entry() {
    let mut ring = KillRing::new();
    ring.push(vec![" ".to_string(), "\t\n".to_string()]);
    ring.push(vs("x"));
    assert_eq!(ring.head(), Some(vs("x").as_slice()));
    assert_eq!(ring.len(), 1);
}

#[test]
fn push_keeps_non_whitespace_head_when_new_is_whitespace() {
    let mut ring = KillRing::new();
    ring.push(vs("hello"));
    ring.push(vs(" "));
    assert_eq!(ring.head(), Some(vs(" ").as_slice()));
    assert_eq!(ring.slot(1), Some(vs("hello").as_slice()));
    assert_eq!(ring.len(), 2);
}

#[test]
fn push_on_empty_ring_just_inserts() {
    let mut ring = KillRing::new();
    ring.push(vs(" "));
    assert_eq!(ring.head(), Some(vs(" ").as_slice()));
    assert_eq!(ring.len(), 1);
}

#[test]
fn push_whitespace_head_when_ring_full_no_eviction() {
    // Fill to depth, then make the head whitespace. Overwriting the head
    // must not evict the oldest real entry (no push_front, no overflow).
    let mut ring = KillRing::new();
    for i in 0..KILL_RING_DEPTH {
        ring.push(vs(&format!("entry{i}")));
    }
    ring.push(vs(" ")); // normal push: evicts entry0, head=" ", oldest=entry1
    ring.push(vs("new")); // whitespace collapse: reclaims " " slot, no eviction
    assert_eq!(ring.head(), Some(vs("new").as_slice()));
    assert_eq!(ring.len(), KILL_RING_DEPTH);
    assert_eq!(
        ring.slot(KILL_RING_DEPTH - 1),
        Some(vs("entry1").as_slice())
    );
}

#[test]
fn push_mixed_entry_not_overwritten() {
    let mut ring = KillRing::new();
    ring.push(vec![" ".to_string(), "x".to_string()]);
    ring.push(vs("y"));
    assert_eq!(ring.head(), Some(vs("y").as_slice()));
    assert_eq!(
        ring.slot(1),
        Some(vec![" ".to_string(), "x".to_string()].as_slice())
    );
    assert_eq!(ring.len(), 2);
}

#[test]
fn push_consecutive_whitespace_kills_collapse() {
    // d<space>, d<tab>, d<x>: each whitespace head is overwritten in turn,
    // so all three share a single slot — no whitespace junk lingers.
    let mut ring = KillRing::new();
    ring.push(vs(" "));
    ring.push(vs("\t"));
    ring.push(vs("x"));
    assert_eq!(ring.len(), 1);
    assert_eq!(ring.head(), Some(vs("x").as_slice()));
}

// -- push: content dedupe ------------------------------------------------------

#[test]
fn push_duplicate_head_is_noop() {
    let mut ring = KillRing::new();
    ring.push(vs("a"));
    ring.push(vs("a"));
    assert_eq!(ring.len(), 1);
    assert_eq!(ring.head(), Some(vs("a").as_slice()));
}

#[test]
fn push_duplicate_deeper_moves_to_front() {
    let mut ring = KillRing::new();
    ring.push(vs("a"));
    ring.push(vs("b"));
    ring.push(vs("c")); // head = slot 0
    ring.push(vs("a")); // re-capture: moves to head, no new slot
    assert_eq!(ring.len(), 3);
    assert_eq!(ring.slot(0), Some(vs("a").as_slice()));
    assert_eq!(ring.slot(1), Some(vs("c").as_slice()));
    assert_eq!(ring.slot(2), Some(vs("b").as_slice()));
}

#[test]
fn push_duplicate_at_capacity_does_not_evict() {
    let mut ring = KillRing::new();
    for i in 0..KILL_RING_DEPTH {
        ring.push(vs(&format!("entry{i}")));
    }
    ring.push(vs("entry5")); // dedupe is net-zero on length: nothing should fall off
    assert_eq!(ring.len(), KILL_RING_DEPTH);
    assert_eq!(ring.head(), Some(vs("entry5").as_slice()));
    assert_eq!(
        ring.slot(KILL_RING_DEPTH - 1),
        Some(vs("entry0").as_slice())
    );
}

#[test]
fn push_duplicate_reclaims_whitespace_head() {
    // Dedupe must run before the whitespace collapse: removing "a" from slot 2
    // first, then overwriting the whitespace head with it, leaves one "a" — not
    // an "a" head plus a surviving older "a" deeper in the ring.
    let mut ring = KillRing::new();
    ring.push(vs("a"));
    ring.push(vs("b"));
    ring.push(vs(" ")); // head = slot 0, whitespace
    ring.push(vs("a")); // dedupe removes the old "a", then collapses into " "'s slot
    assert_eq!(ring.len(), 2);
    assert_eq!(ring.head(), Some(vs("a").as_slice()));
    assert_eq!(ring.slot(1), Some(vs("b").as_slice()));
}

#[test]
fn push_dedupe_compares_whole_entry() {
    let mut ring = KillRing::new();
    ring.push(vec!["a".to_string(), "b".to_string()]);
    ring.push(vs("a"));
    ring.push(vec!["a".to_string(), "b".to_string()]); // equal to slot 1, not slot 0
    assert_eq!(ring.len(), 2);
    assert_eq!(
        ring.head(),
        Some(vec!["a".to_string(), "b".to_string()].as_slice())
    );
    assert_eq!(ring.slot(1), Some(vs("a").as_slice()));
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
    assert!(
        is_valid_register_name(CLIPBOARD_REGISTER),
        "'c' must be valid"
    );
    assert!(
        is_valid_register_name(BLACK_HOLE_REGISTER),
        "'b' must be valid"
    );
    assert!(
        is_valid_register_name(KILL_RING_REGISTER),
        "'k' must be valid"
    );
}

#[test]
fn letter_a_is_not_a_valid_register_name() {
    // Regression guard: 'a' is not a valid register name —
    // is_valid_register_name must keep rejecting it.
    assert!(!is_valid_register_name('a'), "'a' must be invalid");
}

#[test]
fn macro_and_search_registers_not_valid_for_prefix() {
    assert!(
        !is_valid_register_name(MACRO_REGISTER),
        "'q' not prefix-accessible"
    );
    assert!(
        !is_valid_register_name(SEARCH_REGISTER),
        "'s' not prefix-accessible"
    );
}
