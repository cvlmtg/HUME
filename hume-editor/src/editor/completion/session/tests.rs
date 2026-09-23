//! The store on its own: slots, invocations, spans through edits, ranking
//! — no editor, no Steel. The orchestration around it is covered by
//! `editor/tests/completion/`.

use super::*;
use crate::editor::completion::registry::{BufferSourceEntry, MinibufBody, MinibufSourceEntry};
use hume_editing::changeset::ChangeSetBuilder;
use steel::rvals::SteelVal;

fn item(label: &str) -> CompletionItem {
    CompletionItem::plain(label.into(), label.into())
}

fn items(labels: &[&str]) -> Vec<CompletionItem> {
    labels.iter().map(|l| item(l)).collect()
}

/// A registry with one Steel `Buffer` source per `(name, priority)`, all
/// `MatchKind::Fuzzy` — the proc is never called here.
fn registry(sources: &[(&str, i64)]) -> SourceRegistry {
    let mut reg = SourceRegistry::with_defaults();
    for (name, priority) in sources {
        reg.register_buffer(BufferSourceEntry {
            name: (*name).into(),
            match_kind: MatchKind::Fuzzy,
            priority: *priority,
            proc: SteelVal::Void,
            resolve: false,
            trigger_chars: rustc_hash::FxHashMap::default(),
        });
    }
    reg
}

fn id_of(reg: &SourceRegistry, name: &str) -> BufferSourceId {
    reg.buffer_id_of(name).expect("registered")
}

/// [`registry`]'s `Minibuf` counterpart, for the one test below that opens
/// a `Minibuf` session.
fn minibuf_registry(sources: &[(&str, i64)]) -> SourceRegistry {
    let mut reg = SourceRegistry::with_defaults();
    for (name, priority) in sources {
        reg.register_minibuf(MinibufSourceEntry {
            name: (*name).into(),
            match_kind: MatchKind::Fuzzy,
            priority: *priority,
            body: MinibufBody::Steel(SteelVal::Void),
        });
    }
    reg
}

fn minibuf_id_of(reg: &SourceRegistry, name: &str) -> MinibufSourceId {
    reg.minibuf_id_of(name).expect("registered")
}

fn text(s: &str) -> BufferText {
    BufferText::from(s)
}

/// A `Buffer` session over `content` — the store never consults the
/// buffer/pane ids except through `still_valid`, unused here, so the null
/// keys do.
fn buffer_session(content: &str) -> (CompletionSession, BufferText) {
    let text = text(content);
    let session =
        CompletionSession::open_buffer(BufferId::default(), PaneId::default(), 0, text.len_chars());
    (session, text)
}

/// One `Word`-rule invocation with token `start..head`, answered with
/// `labels` at once.
fn invoke_and_answer(
    session: &mut CompletionSession,
    reg: &SourceRegistry,
    source: BufferSourceId,
    text: &BufferText,
    start: usize,
    head: usize,
    labels: &[&str],
) {
    let inv = Invocation::buffer(
        text.rope().clone(),
        CharOffset::new(head),
        CharOffset::new(start)..CharOffset::new(head),
    );
    let id = session.invoke_buffer(source, inv).expect("buffer session");
    session.contribute(reg, id, items(labels), false);
}

fn live(text: &BufferText, head: usize) -> Option<LiveDoc<'_>> {
    Some(LiveDoc {
        text,
        head: CharOffset::new(head),
    })
}

fn ranked_labels(session: &CompletionSession, reg: &SourceRegistry) -> Vec<String> {
    session
        .top(10, reg)
        .iter()
        .map(|v| v["label"].as_str().unwrap().to_string())
        .collect()
}

/// `text` with `[from, to)` replaced by `with`, as a `ChangeSet` plus the
/// text it produces.
fn edit(text: &BufferText, from: usize, to: usize, with: &str) -> (ChangeSet, BufferText) {
    let mut b = ChangeSetBuilder::new(text.end());
    b.retain(from);
    b.delete(to - from);
    b.insert(with);
    b.retain_rest();
    let cs = b.finish();
    let mut s = text.to_string();
    s.replace_range(
        text.char_to_byte(CharOffset::new(from))..text.char_to_byte(CharOffset::new(to)),
        with,
    );
    (cs, BufferText::from(s.as_str()))
}

// ── Per-source tokens ────────────────────────────────────────────────────────

/// Two sources with different token starts each rank against their own
/// text: the path-shaped one sees "./fo", the word one "fo".
#[test]
fn each_slot_ranks_against_its_own_token() {
    let reg = registry(&[("word", 0), ("dir", 0)]);
    let (mut session, text) = buffer_session("./fo\n");
    invoke_and_answer(
        &mut session,
        &reg,
        id_of(&reg, "word"),
        &text,
        2,
        4,
        &["foobar", "./x"],
    );
    invoke_and_answer(
        &mut session,
        &reg,
        id_of(&reg, "dir"),
        &text,
        0,
        4,
        &["./foo.txt", "bar"],
    );
    session.rank(&reg, live(&text, 4));
    let mut ranked = ranked_labels(&session, &reg);
    ranked.sort();
    assert_eq!(
        ranked,
        vec!["./foo.txt", "foobar"],
        "\"./x\" fails the word slot's \"fo\"; \"bar\" fails the dir slot's \"./fo\""
    );
}

/// The menu anchors at the leftmost token start among the slots with a
/// ranked candidate — it moves only when ranking changes which sources
/// contribute.
#[test]
fn the_menu_anchors_at_the_leftmost_ranked_slots_token_start() {
    let reg = registry(&[("word", 0), ("dir", 0)]);
    let (mut session, text) = buffer_session("./fo\n");
    invoke_and_answer(
        &mut session,
        &reg,
        id_of(&reg, "word"),
        &text,
        2,
        4,
        &["foobar"],
    );
    invoke_and_answer(
        &mut session,
        &reg,
        id_of(&reg, "dir"),
        &text,
        0,
        4,
        &["./foo.txt"],
    );
    session.rank(&reg, live(&text, 4));
    assert_eq!(session.menu_anchor_char(), Some(CharOffset::new(0)));

    // "b" narrows the dir slot out ("./foo.txt" has no 'b') but not the
    // word slot — the anchor moves to the word token's start.
    let (cs, text) = edit(&text, 4, 4, "b");
    assert!(session.observe_edit(
        &reg,
        &cs,
        1,
        CharOffset::new(5),
        &text,
        WordChars::default()
    ));
    session.rank(&reg, live(&text, 5));
    assert_eq!(ranked_labels(&session, &reg), vec!["foobar"]);
    assert_eq!(session.menu_anchor_char(), Some(CharOffset::new(2)));
}

#[test]
fn priority_breaks_a_score_tie_before_sort_text() {
    let reg = registry(&[("lo", 0), ("hi", 10)]);
    let (mut session, text) = buffer_session("\n");
    invoke_and_answer(&mut session, &reg, id_of(&reg, "lo"), &text, 0, 0, &["aaa"]);
    invoke_and_answer(&mut session, &reg, id_of(&reg, "hi"), &text, 0, 0, &["zzz"]);
    session.rank(&reg, live(&text, 0));
    assert_eq!(ranked_labels(&session, &reg), vec!["zzz", "aaa"]);
}

// ── Cross-source dedup ───────────────────────────────────────────────────────

fn ranked_sources(session: &CompletionSession, reg: &SourceRegistry) -> Vec<String> {
    session
        .top(10, reg)
        .iter()
        .map(|v| v["source"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_higher_priority_duplicate_hides_the_lower_priority_plain_item() {
    let reg = registry(&[("lo", 0), ("hi", 10)]);
    let (mut session, text) = buffer_session("\n");
    invoke_and_answer(&mut session, &reg, id_of(&reg, "lo"), &text, 0, 0, &["dup"]);
    invoke_and_answer(&mut session, &reg, id_of(&reg, "hi"), &text, 0, 0, &["dup"]);
    session.rank(&reg, live(&text, 0));
    assert_eq!(
        ranked_labels(&session, &reg),
        vec!["dup"],
        "shown once, not twice"
    );
    assert_eq!(
        ranked_sources(&session, &reg),
        vec!["hi"],
        "the higher-priority source's own item is the one that survives"
    );
}

#[test]
fn equal_priority_duplicates_are_not_deduplicated() {
    let reg = registry(&[("a", 0), ("b", 0)]);
    let (mut session, text) = buffer_session("\n");
    invoke_and_answer(&mut session, &reg, id_of(&reg, "a"), &text, 0, 0, &["dup"]);
    invoke_and_answer(&mut session, &reg, id_of(&reg, "b"), &text, 0, 0, &["dup"]);
    session.rank(&reg, live(&text, 0));
    assert_eq!(
        ranked_labels(&session, &reg),
        vec!["dup", "dup"],
        "no priority signal to prefer one over the other, so dedup doesn't apply"
    );
}

/// An item carrying edits is never hidden as someone else's duplicate —
/// accepting it does something a duplicate-looking plain item wouldn't —
/// regardless of which side of the priority comparison it's on: the
/// lower-priority source's edit-carrying item here survives against a
/// higher-priority plain duplicate, and the higher-priority source's own
/// plain item survives too, since nothing outranks *it*.
#[test]
fn an_item_with_edits_is_never_hidden_as_a_duplicate() {
    let reg = registry(&[("lo", 0), ("hi", 10)]);
    let (mut session, text) = buffer_session("\n");
    let head = CharOffset::new(0);
    let inv = Invocation::buffer(text.rope().clone(), head, head..head);
    let id = session
        .invoke_buffer(id_of(&reg, "lo"), inv)
        .expect("buffer session");
    assert!(session.contribute(
        &reg,
        id,
        vec![item_with_edits(
            "dup",
            None,
            vec![some_edit("import dup;\n")],
        )],
        false,
    ));
    invoke_and_answer(&mut session, &reg, id_of(&reg, "hi"), &text, 0, 0, &["dup"]);
    session.rank(&reg, live(&text, 0));
    let mut ranked = ranked_labels(&session, &reg);
    ranked.sort();
    assert_eq!(
        ranked,
        vec!["dup", "dup"],
        "the edit-carrying item is exempt from being hidden; the higher-priority \
         plain item has nothing above it to be hidden by either"
    );
}

// ── No-op items ──────────────────────────────────────────────────────────────

/// An item carrying a `text_edit`/`additional_text_edits` — the shape a
/// server sends for a case-correction or an auto-import — built directly
/// rather than through `item()`/`CompletionItem::plain`, which leaves both
/// empty.
fn item_with_edits(
    label: &str,
    text_edit: Option<lsp_types::TextEdit>,
    additional_text_edits: Vec<lsp_types::TextEdit>,
) -> CompletionItem {
    let has_additional_text_edits = !additional_text_edits.is_empty();
    CompletionItem {
        label: label.into(),
        kind: None,
        detail: None,
        sort_text: label.into(),
        filter_text: label.into(),
        insert_text: label.into(),
        text_edit,
        additional_text_edits,
        has_additional_text_edits,
        raw: serde_json::Value::Null,
    }
}

fn some_edit(new_text: &str) -> lsp_types::TextEdit {
    lsp_types::TextEdit {
        range: lsp_types::Range::default(),
        new_text: new_text.into(),
    }
}

#[test]
fn rank_drops_an_item_that_exactly_matches_what_was_typed() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("cat\n");
    invoke_and_answer(
        &mut session,
        &reg,
        id_of(&reg, "s"),
        &text,
        0,
        3,
        &["cat", "category"],
    );
    session.rank(&reg, live(&text, 3));
    assert_eq!(
        ranked_labels(&session, &reg),
        vec!["category"],
        "the typed word itself is a no-op to accept"
    );
}

#[test]
fn backspacing_past_the_dropped_word_brings_it_back() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("cat\n");
    invoke_and_answer(
        &mut session,
        &reg,
        id_of(&reg, "s"),
        &text,
        0,
        3,
        &["cat", "category"],
    );
    session.rank(&reg, live(&text, 3));
    assert_eq!(ranked_labels(&session, &reg), vec!["category"], "sanity");

    // Backspace: "cat" -> "ca". The item list is unchanged (the source
    // wasn't re-invoked) — only the live token narrows, so "cat" is no
    // longer an exact match and reappears.
    let (cs, text) = edit(&text, 2, 3, "");
    assert!(session.observe_edit(
        &reg,
        &cs,
        1,
        CharOffset::new(2),
        &text,
        WordChars::default()
    ));
    session.rank(&reg, live(&text, 2));
    let mut ranked = ranked_labels(&session, &reg);
    ranked.sort();
    assert_eq!(ranked, vec!["cat", "category"]);
}

#[test]
fn an_item_with_a_text_edit_is_kept_even_if_its_insert_text_matches() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("cat\n");
    let inv = Invocation::buffer(
        text.rope().clone(),
        CharOffset::new(3),
        CharOffset::new(0)..CharOffset::new(3),
    );
    let id = session
        .invoke_buffer(id_of(&reg, "s"), inv)
        .expect("buffer session");
    assert!(session.contribute(
        &reg,
        id,
        vec![item_with_edits("cat", Some(some_edit("cat")), Vec::new())],
        false,
    ));
    session.rank(&reg, live(&text, 3));
    assert_eq!(
        ranked_labels(&session, &reg),
        vec!["cat"],
        "a textEdit's range may cover more than the typed token — never a no-op by inspection alone"
    );
}

#[test]
fn an_item_with_additional_text_edits_is_kept_even_if_its_insert_text_matches() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("cat\n");
    let inv = Invocation::buffer(
        text.rope().clone(),
        CharOffset::new(3),
        CharOffset::new(0)..CharOffset::new(3),
    );
    let id = session
        .invoke_buffer(id_of(&reg, "s"), inv)
        .expect("buffer session");
    assert!(session.contribute(
        &reg,
        id,
        vec![item_with_edits(
            "cat",
            None,
            vec![some_edit("use std::cat;\n")],
        )],
        false,
    ));
    session.rank(&reg, live(&text, 3));
    assert_eq!(
        ranked_labels(&session, &reg),
        vec!["cat"],
        "additionalTextEdits (e.g. an auto-import) make accepting a real edit"
    );
}

// ── Invocations and answers ──────────────────────────────────────────────────

#[test]
fn an_answer_to_a_superseded_invocation_is_dropped() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("\n");
    let src = id_of(&reg, "s");
    let head = CharOffset::new(0);
    let first = session
        .invoke_buffer(
            src,
            Invocation::buffer(text.rope().clone(), head, head..head),
        )
        .expect("buffer session");
    let second = session
        .invoke_buffer(
            src,
            Invocation::buffer(text.rope().clone(), head, head..head),
        )
        .expect("buffer session");
    assert!(!session.contribute(&reg, first, items(&["stale"]), false));
    assert!(session.contribute(&reg, second, items(&["fresh"]), false));
    session.rank(&reg, live(&text, 0));
    assert_eq!(ranked_labels(&session, &reg), vec!["fresh"]);
}

#[test]
fn a_repeated_answer_for_the_latest_invocation_replaces_it() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("\n");
    let src = id_of(&reg, "s");
    let head = CharOffset::new(0);
    let id = session
        .invoke_buffer(
            src,
            Invocation::buffer(text.rope().clone(), head, head..head),
        )
        .expect("buffer session");
    assert!(session.contribute(&reg, id, items(&["x", "y"]), false));
    assert!(session.contribute(&reg, id, items(&["x", "z"]), false));
    session.rank(&reg, live(&text, 0));
    assert_eq!(ranked_labels(&session, &reg), vec!["x", "z"]);
}

#[test]
fn pending_and_live_track_each_slots_latest_call() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("\n");
    let src = id_of(&reg, "s");
    let head = CharOffset::new(0);
    assert!(!session.is_pending());
    assert!(!session.has_live_sources());
    let id = session
        .invoke_buffer(
            src,
            Invocation::buffer(text.rope().clone(), head, head..head),
        )
        .expect("buffer session");
    assert!(session.is_pending());
    assert!(session.contribute(&reg, id, items(&["x"]), true));
    assert!(!session.is_pending());
    assert!(session.has_live_sources());
    assert_eq!(
        session.sources_to_reinvoke(),
        vec![src],
        "flagged incomplete"
    );
    assert!(session.contribute(&reg, id, items(&[]), false));
    assert!(!session.has_live_sources(), "an empty answer is not live");
    assert!(session.sources_to_reinvoke().is_empty());
}

// ── Spans through edits ──────────────────────────────────────────────────────

/// Typing at the token's end extends it (`Assoc::After` on the end);
/// the filter follows.
#[test]
fn typing_at_the_tokens_end_extends_it() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("fo\n");
    invoke_and_answer(
        &mut session,
        &reg,
        id_of(&reg, "s"),
        &text,
        0,
        2,
        &["foobar", "fox", "bar"],
    );
    let (cs, text) = edit(&text, 2, 2, "o");
    assert!(session.observe_edit(
        &reg,
        &cs,
        1,
        CharOffset::new(3),
        &text,
        WordChars::default()
    ));
    session.rank(&reg, live(&text, 3));
    assert_eq!(ranked_labels(&session, &reg), vec!["foobar"]);
}

/// A non-word char landing exactly at the token's end (an auto-paired `(`,
/// say) must *not* extend the tracked span the way a continued-typing word
/// char does above — the token's own definition ("the word before the
/// cursor") never includes one. `head` moving past it then sits outside
/// `[start, end]`, so the ordinary "cursor left the token" containment
/// check drops the slot, same as leaving the token any other way.
#[test]
fn a_non_word_char_at_the_tokens_end_does_not_extend_it() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("fo\n");
    invoke_and_answer(
        &mut session,
        &reg,
        id_of(&reg, "s"),
        &text,
        0,
        2,
        &["foobar"],
    );
    let (cs, text) = edit(&text, 2, 2, "(");
    assert!(session.observe_edit(
        &reg,
        &cs,
        1,
        CharOffset::new(3),
        &text,
        WordChars::default()
    ));
    assert!(
        !session.has_live_sources(),
        "the '(' must not have joined the token — the cursor past it is outside \
         [start, end], so the slot is dropped like any other token exit"
    );
}

/// Deleting the token's own first char stays inside it; deleting the char
/// *before* the token crosses it — the slot's answer is dropped, and the
/// two are told apart by whether `start` and `start - 1` collapse to the
/// same live position.
#[test]
fn deleting_before_the_token_drops_the_slot_but_deleting_its_first_char_does_not() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("x fo\n");
    invoke_and_answer(&mut session, &reg, id_of(&reg, "s"), &text, 2, 4, &["foo"]);

    // Backspace twice: "o", then "f" — the token's own chars.
    let (cs, text) = edit(&text, 3, 4, "");
    assert!(session.observe_edit(
        &reg,
        &cs,
        1,
        CharOffset::new(3),
        &text,
        WordChars::default()
    ));
    let (cs, text) = edit(&text, 2, 3, "");
    assert!(session.observe_edit(
        &reg,
        &cs,
        2,
        CharOffset::new(2),
        &text,
        WordChars::default()
    ));
    assert!(
        session.has_live_sources(),
        "an empty token is still a token"
    );
    session.rank(&reg, live(&text, 2));
    assert_eq!(ranked_labels(&session, &reg), vec!["foo"]);

    // A third Backspace deletes the space before the token.
    let (cs, text) = edit(&text, 1, 2, "");
    assert!(session.observe_edit(
        &reg,
        &cs,
        3,
        CharOffset::new(1),
        &text,
        WordChars::default()
    ));
    assert!(!session.has_live_sources(), "crossed the token's start");
}

/// An edit elsewhere in the buffer — a second cursor deleting text far
/// before the token — shifts the token without crossing it.
#[test]
fn a_deletion_elsewhere_shifts_the_token_without_dropping_it() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("abc fo\n");
    invoke_and_answer(&mut session, &reg, id_of(&reg, "s"), &text, 4, 6, &["foo"]);
    let (cs, text) = edit(&text, 0, 1, "");
    assert!(session.observe_edit(
        &reg,
        &cs,
        1,
        CharOffset::new(5),
        &text,
        WordChars::default()
    ));
    assert!(session.has_live_sources());
    session.rank(&reg, live(&text, 5));
    assert_eq!(ranked_labels(&session, &reg), vec!["foo"]);
    assert_eq!(session.menu_anchor_char(), Some(CharOffset::new(3)));
}

#[test]
fn a_cursor_outside_the_token_drops_the_slot() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("fo bar\n");
    invoke_and_answer(&mut session, &reg, id_of(&reg, "s"), &text, 0, 2, &["foo"]);
    // No edit — the cursor just moved (an out-of-band motion).
    let cs = ChangeSet::identity(text.len_chars());
    assert!(session.observe_edit(
        &reg,
        &cs,
        1,
        CharOffset::new(5),
        &text,
        WordChars::default()
    ));
    assert!(!session.has_live_sources());
}

#[test]
fn an_edit_the_session_never_saw_is_refused() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("fo\n");
    invoke_and_answer(&mut session, &reg, id_of(&reg, "s"), &text, 0, 2, &["foo"]);
    let longer = BufferText::from("fooooo\n");
    let cs = ChangeSet::identity(longer.len_chars());
    assert!(
        !session.observe_edit(
            &reg,
            &cs,
            1,
            CharOffset::new(2),
            &text,
            WordChars::default()
        ),
        "a changeset built against a different length is an unseen edit"
    );
}

/// Each invocation composes the edits observed since *its own* call: a
/// second call minted after a keystroke starts from a fresh snapshot.
#[test]
fn a_later_invocation_starts_from_its_own_snapshot() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("fo\n");
    let src = id_of(&reg, "s");
    invoke_and_answer(&mut session, &reg, src, &text, 0, 2, &["foo"]);
    let (cs, text) = edit(&text, 2, 2, "o");
    assert!(session.observe_edit(
        &reg,
        &cs,
        1,
        CharOffset::new(3),
        &text,
        WordChars::default()
    ));
    // Re-invoked against the post-edit document.
    invoke_and_answer(&mut session, &reg, src, &text, 0, 3, &["foobar"]);
    let (cs, text) = edit(&text, 3, 3, "b");
    assert!(session.observe_edit(
        &reg,
        &cs,
        2,
        CharOffset::new(4),
        &text,
        WordChars::default()
    ));
    session.rank(&reg, live(&text, 4));
    assert_eq!(ranked_labels(&session, &reg), vec!["foobar"]);
    assert_eq!(session.menu_anchor_char(), Some(CharOffset::new(0)));
}

// ── Selection stepping ───────────────────────────────────────────────────────

#[test]
fn step_selection_on_an_empty_ranking_is_none() {
    let session = CompletionSession::open_minibuf(String::new(), 0);
    assert_eq!(session.step_selection(0, true), None);
    assert_eq!(session.step_selection(0, false), None);
}

#[test]
fn step_selection_wraps_at_either_end() {
    let reg = minibuf_registry(&[("s", 0)]);
    let mut session = CompletionSession::open_minibuf("w".into(), 1);
    let id = session
        .invoke_minibuf(minibuf_id_of(&reg, "s"), Invocation::minibuf(0..1))
        .expect("minibuf session");
    assert!(session.contribute(&reg, id, items(&["wa", "wb"]), false));
    session.rank(&reg, None);
    assert_eq!(session.step_selection(1, true), Some(0), "wraps forward");
    assert_eq!(session.step_selection(0, false), Some(1), "wraps backward");
}

// ── prefix_matches ───────────────────────────────────────────────────────────

#[test]
fn prefix_matches_case_sensitive() {
    assert!(prefix_matches("write-quit", "write", true));
    assert!(!prefix_matches("write-quit", "Write", true));
}

#[test]
fn prefix_matches_case_insensitive() {
    assert!(prefix_matches("write-quit", "Write", false));
    assert!(!prefix_matches("write-quit", "quit", false));
}

/// `haystack.get(..prefix.len())` (the case-insensitive branch) must not
/// panic when `prefix.len()` lands mid-codepoint in a non-ASCII haystack —
/// "ï" (U+00EF) occupies bytes 2-3 of "naïve-cmd", so a 3-byte prefix lands
/// inside it; `.get()` returns `None` there instead of panicking.
#[test]
fn prefix_matches_non_ascii_boundary_does_not_panic() {
    assert!(!prefix_matches("naïve-cmd", "xyz", false));
    assert!(prefix_matches("naïve-cmd", "na", false));
}

#[test]
fn prefix_matches_prefix_longer_than_haystack_does_not_panic() {
    assert!(!prefix_matches("q", "quit", false));
    assert!(!prefix_matches("q", "quit", true));
}
