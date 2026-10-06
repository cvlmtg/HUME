//! The store on its own: slots, invocations, spans through edits, ranking
//! (no editor, no Steel). The orchestration around it is covered by
//! `editor/tests/completion/`.

use super::super::item::CompletionItem;
use super::super::registry::{
    BufferSourceEntry, BufferSourceId, MinibufBody, MinibufSourceEntry, MinibufSourceId,
    SourceRegistry,
};
use super::*;
use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::edit::TextChange;
use hume_editing::text::BufferText;
use hume_engine::pipeline::{BufferId, PaneId};
use hume_rope::cluster::ClusterStart;
use hume_rope::offset::CharOffset;
use steel::rvals::SteelVal;

fn item(label: &str) -> CompletionItem {
    CompletionItem::plain(label.into(), label.into())
}

fn items(labels: &[&str]) -> Vec<CompletionItem> {
    labels.iter().map(|l| item(l)).collect()
}

/// A registry with one Steel `Buffer` source per `(name, priority)`, all
/// `MatchKind::Fuzzy`; the proc is never called here.
fn registry(sources: &[(&str, i64)]) -> SourceRegistry {
    let mut reg = SourceRegistry::with_defaults();
    for (name, priority) in sources {
        reg.register_buffer(BufferSourceEntry {
            name: (*name).into(),
            match_kind: MatchKind::Fuzzy,
            priority: *priority,
            proc: SteelVal::Void,
            resolve: false,
            token_chars: "".into(),
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

/// A `Buffer` session over `content` with the cursor at its start. The
/// store never consults the buffer/pane ids beyond matching a carried
/// change to its own buffer, so the null keys do.
fn buffer_session(content: &str) -> (BufferSession, BufferText) {
    let text = text(content);
    let session = BufferSession::open(
        BufferId::default(),
        PaneId::default(),
        text.version(),
        CharOffset::new(0),
    );
    (session, text)
}

/// One invocation whose token starts at `start`, answered with `labels` at
/// once.
fn invoke_and_answer(
    session: &mut BufferSession,
    reg: &SourceRegistry,
    source: BufferSourceId,
    text: &BufferText,
    start: usize,
    labels: &[&str],
) {
    let inv = Invocation::buffer(text.rope().clone(), CharOffset::new(start));
    let id = session.invoke(source, inv);
    session.contribute(reg, id, items(labels), false);
}

fn live(text: &BufferText, head: usize) -> Option<LiveDoc<'_>> {
    Some(LiveDoc {
        text,
        head: CharOffset::new(head),
    })
}

fn ranked_labels(session: &BufferSession, reg: &SourceRegistry) -> Vec<String> {
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
    b.retain_to(CharOffset::new(from))
        .delete_to(CharOffset::new(to))
        .insert(with);
    let cs = b.finish();
    let mut s = text.to_string();
    s.replace_range(
        text.char_to_byte(CharOffset::new(from))..text.char_to_byte(CharOffset::new(to)),
        with,
    );
    (cs, BufferText::from(s.as_str()))
}

/// One edit on the session's buffer: `from..to` of `text` replaced by
/// `with`, carried into the session the way `PositionStores::carry` does,
/// then reconciled with the cursor at `head`. Returns the new text and what
/// the reconcile found.
fn change(
    session: &mut BufferSession,
    reg: &SourceRegistry,
    text: BufferText,
    (from, to, with): (usize, usize, &str),
    head: usize,
) -> (BufferText, Reconciled) {
    let (cs, after) = edit(&text, from, to, with);
    session.carry(BufferId::default(), &TextChange::new(&text, &after, &cs));
    let outcome = session.reconcile(reg, &after, CharOffset::new(head), "");
    (after, outcome)
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
        &["foobar", "./x"],
    );
    invoke_and_answer(
        &mut session,
        &reg,
        id_of(&reg, "dir"),
        &text,
        0,
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
/// ranked candidate: it moves only when ranking changes which sources
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
        &["foobar"],
    );
    invoke_and_answer(
        &mut session,
        &reg,
        id_of(&reg, "dir"),
        &text,
        0,
        &["./foo.txt"],
    );
    session.rank(&reg, live(&text, 4));
    assert_eq!(
        session.menu_anchor(&text).map(ClusterStart::offset),
        Some(CharOffset::new(0))
    );

    // "b" narrows the dir slot out ("./foo.txt" has no 'b') but not the
    // word slot, so the anchor moves to the word token's start.
    let (text, _) = change(&mut session, &reg, text, (4, 4, "b"), 5);
    session.rank(&reg, live(&text, 5));
    assert_eq!(ranked_labels(&session, &reg), vec!["foobar"]);
    assert_eq!(
        session.menu_anchor(&text).map(ClusterStart::offset),
        Some(CharOffset::new(2))
    );
}

#[test]
fn priority_breaks_a_score_tie_before_sort_text() {
    let reg = registry(&[("lo", 0), ("hi", 10)]);
    let (mut session, text) = buffer_session("\n");
    invoke_and_answer(&mut session, &reg, id_of(&reg, "lo"), &text, 0, &["aaa"]);
    invoke_and_answer(&mut session, &reg, id_of(&reg, "hi"), &text, 0, &["zzz"]);
    session.rank(&reg, live(&text, 0));
    assert_eq!(ranked_labels(&session, &reg), vec!["zzz", "aaa"]);
}

// ── Cross-source dedup ───────────────────────────────────────────────────────

fn ranked_sources(session: &BufferSession, reg: &SourceRegistry) -> Vec<String> {
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
    invoke_and_answer(&mut session, &reg, id_of(&reg, "lo"), &text, 0, &["dup"]);
    invoke_and_answer(&mut session, &reg, id_of(&reg, "hi"), &text, 0, &["dup"]);
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
    invoke_and_answer(&mut session, &reg, id_of(&reg, "a"), &text, 0, &["dup"]);
    invoke_and_answer(&mut session, &reg, id_of(&reg, "b"), &text, 0, &["dup"]);
    session.rank(&reg, live(&text, 0));
    assert_eq!(
        ranked_labels(&session, &reg),
        vec!["dup", "dup"],
        "no priority signal to prefer one over the other, so dedup doesn't apply"
    );
}

/// An item carrying edits is never hidden as someone else's duplicate
/// (accepting it does something a duplicate-looking plain item wouldn't),
/// regardless of which side of the priority comparison it's on: the
/// lower-priority source's edit-carrying item here survives against a
/// higher-priority plain duplicate, and the higher-priority source's own
/// plain item survives too, since nothing outranks *it*.
#[test]
fn an_item_with_edits_is_never_hidden_as_a_duplicate() {
    let reg = registry(&[("lo", 0), ("hi", 10)]);
    let (mut session, text) = buffer_session("\n");
    let head = CharOffset::new(0);
    let inv = Invocation::buffer(text.rope().clone(), head);
    let id = session.invoke(id_of(&reg, "lo"), inv);
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
    invoke_and_answer(&mut session, &reg, id_of(&reg, "hi"), &text, 0, &["dup"]);
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

/// An item carrying a `text_edit`/`additional_text_edits` (the shape a
/// server sends for a case-correction or an auto-import), built directly
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
        token_group: None,
        raw: None,
    }
}

fn some_edit(new_text: &str) -> lsp_types::TextEdit {
    lsp_types::TextEdit {
        range: lsp_types::Range::default(),
        new_text: new_text.into(),
    }
}

fn edit_over(start: (u32, u32), end: (u32, u32), new_text: &str) -> lsp_types::TextEdit {
    lsp_types::TextEdit {
        range: lsp_types::Range {
            start: lsp_types::Position::new(start.0, start.1),
            end: lsp_types::Position::new(end.0, end.1),
        },
        new_text: new_text.into(),
    }
}

/// A `MatchKind::String` source over "> foo.ba" whose token (the word before
/// the cursor) is "ba", answering `foo.bar` with an edit range that starts
/// before it and a plain `bar`.
fn dotted_session() -> (BufferSession, SourceRegistry, BufferText) {
    let mut reg = SourceRegistry::with_defaults();
    reg.register_buffer(BufferSourceEntry {
        name: "s".into(),
        match_kind: MatchKind::String {
            case_sensitive: true,
        },
        priority: 0,
        proc: SteelVal::Void,
        resolve: false,
        token_chars: "".into(),
    });
    let (mut session, text) = buffer_session("> foo.ba\n");
    let id = session.invoke(
        id_of(&reg, "s"),
        Invocation::buffer(text.rope().clone(), CharOffset::new(6)),
    );
    assert!(session.contribute(
        &reg,
        id,
        vec![
            item_with_edits(
                "foo.bar",
                Some(edit_over((0, 2), (0, 8), "foo.bar")),
                Vec::new()
            ),
            item("bar"),
        ],
        false,
    ));
    (session, reg, text)
}

/// The server's edit range, not the editor's word token, says what the
/// user has typed of an item: `foo.ba` for the dotted item, `ba` for the
/// plain one.
#[test]
fn an_item_is_filtered_against_the_text_from_its_own_edit_range_start() {
    let (mut session, reg, text) = dotted_session();
    session.rank(&reg, live(&text, 8));
    let mut ranked = ranked_labels(&session, &reg);
    ranked.sort();
    assert_eq!(ranked, vec!["bar", "foo.bar"]);
    assert_eq!(
        session.menu_anchor(&text).map(ClusterStart::offset),
        Some(CharOffset::new(2)),
        "the menu anchors at the start of the text the dotted item replaces"
    );
}

/// An edit range starting after the cursor cannot be what the user has
/// typed of the item: it is not shown, while its neighbours still are.
#[test]
fn an_item_whose_edit_range_starts_after_the_cursor_is_not_shown() {
    let (mut session, reg, text) = dotted_session();
    let mut items = vec![item_with_edits(
        "ahead",
        Some(edit_over((0, 2), (0, 8), "ahead")),
        Vec::new(),
    )];
    items.push(item("bar"));
    let id = session.invoke(
        id_of(&reg, "s"),
        Invocation::buffer(text.rope().clone(), CharOffset::new(0)),
    );
    assert!(session.contribute(&reg, id, items, false));
    session.rank(&reg, live(&text, 0));
    assert_eq!(ranked_labels(&session, &reg), vec!["bar"]);
}

/// A change before the edit range moves its start with it.
#[test]
fn an_items_edit_range_start_follows_a_change_before_it() {
    let (mut session, reg, text) = dotted_session();
    let (text, _) = change(&mut session, &reg, text, (0, 0, "x"), 9);
    session.rank(&reg, live(&text, 9));
    let mut ranked = ranked_labels(&session, &reg);
    ranked.sort();
    assert_eq!(ranked, vec!["bar", "foo.bar"]);
    assert_eq!(
        session.menu_anchor(&text).map(ClusterStart::offset),
        Some(CharOffset::new(3))
    );
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
        &["cat", "category"],
    );
    session.rank(&reg, live(&text, 3));
    assert_eq!(ranked_labels(&session, &reg), vec!["category"], "sanity");

    // Backspace: "cat" -> "ca". The item list is unchanged (the source
    // wasn't re-invoked); only the live token narrows, so "cat" is no
    // longer an exact match and reappears.
    let (text, _) = change(&mut session, &reg, text, (2, 3, ""), 2);
    session.rank(&reg, live(&text, 2));
    let mut ranked = ranked_labels(&session, &reg);
    ranked.sort();
    assert_eq!(ranked, vec!["cat", "category"]);
}

#[test]
fn an_item_with_a_text_edit_is_kept_even_if_its_insert_text_matches() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("cat\n");
    let inv = Invocation::buffer(text.rope().clone(), CharOffset::new(0));
    let id = session.invoke(id_of(&reg, "s"), inv);
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
        "a textEdit's range may cover more than the typed token, never a no-op by inspection alone"
    );
}

#[test]
fn an_item_with_additional_text_edits_is_kept_even_if_its_insert_text_matches() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("cat\n");
    let inv = Invocation::buffer(text.rope().clone(), CharOffset::new(0));
    let id = session.invoke(id_of(&reg, "s"), inv);
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
    let first = session.invoke(src, Invocation::buffer(text.rope().clone(), head));
    let second = session.invoke(src, Invocation::buffer(text.rope().clone(), head));
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
    let id = session.invoke(src, Invocation::buffer(text.rope().clone(), head));
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
    let id = session.invoke(src, Invocation::buffer(text.rope().clone(), head));
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
        &["foobar", "fox", "bar"],
    );
    let (text, _) = change(&mut session, &reg, text, (2, 2, "o"), 3);
    session.rank(&reg, live(&text, 3));
    assert_eq!(ranked_labels(&session, &reg), vec!["foobar"]);
}

/// A non-word char landing exactly at the token's end (an auto-paired `(`,
/// say) must *not* extend the tracked span the way a continued-typing word
/// char does above: the token's own definition ("the word before the
/// cursor") never includes one. `head` moving past it then sits outside
/// `[start, end]`, so the ordinary "cursor left the token" containment
/// check drops the slot, same as leaving the token any other way.
#[test]
fn a_non_word_char_at_the_tokens_end_does_not_extend_it() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("fo\n");
    invoke_and_answer(&mut session, &reg, id_of(&reg, "s"), &text, 0, &["foobar"]);
    change(&mut session, &reg, text, (2, 2, "("), 3);
    assert!(
        !session.has_live_sources(),
        "the '(' must not have joined the token; the cursor past it is outside \
         [start, end], so the slot is dropped like any other token exit"
    );
}

/// Deleting the token's own first char stays inside it; deleting the char
/// *before* the token crosses it. The slot's answer is dropped, and the
/// two are told apart by whether `start` and `start - 1` collapse to the
/// same live position.
#[test]
fn deleting_before_the_token_drops_the_slot_but_deleting_its_first_char_does_not() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("x fo\n");
    invoke_and_answer(&mut session, &reg, id_of(&reg, "s"), &text, 2, &["foo"]);

    // Backspace twice: "o", then "f", the token's own chars.
    let (text, _) = change(&mut session, &reg, text, (3, 4, ""), 3);
    let (text, _) = change(&mut session, &reg, text, (2, 3, ""), 2);
    assert!(
        session.has_live_sources(),
        "an empty token is still a token"
    );
    session.rank(&reg, live(&text, 2));
    assert_eq!(ranked_labels(&session, &reg), vec!["foo"]);

    // A third Backspace deletes the space before the token.
    change(&mut session, &reg, text, (1, 2, ""), 1);
    assert!(!session.has_live_sources(), "crossed the token's start");
}

/// An edit elsewhere in the buffer (a second cursor deleting text far
/// before the token) shifts the token without crossing it.
#[test]
fn a_deletion_elsewhere_shifts_the_token_without_dropping_it() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("abc fo\n");
    invoke_and_answer(&mut session, &reg, id_of(&reg, "s"), &text, 4, &["foo"]);
    let (text, _) = change(&mut session, &reg, text, (0, 1, ""), 5);
    assert!(session.has_live_sources());
    session.rank(&reg, live(&text, 5));
    assert_eq!(ranked_labels(&session, &reg), vec!["foo"]);
    assert_eq!(
        session.menu_anchor(&text).map(ClusterStart::offset),
        Some(CharOffset::new(3))
    );
}

#[test]
fn a_cursor_outside_the_token_drops_the_slot() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("fo bar\n");
    invoke_and_answer(&mut session, &reg, id_of(&reg, "s"), &text, 0, &["foo"]);
    // No edit: the cursor just moved (an out-of-band motion).
    let outcome = session.reconcile(&reg, &text, CharOffset::new(5), "");
    assert!(matches!(
        outcome,
        Reconciled::Changed {
            text_changed: false
        }
    ));
    assert!(!session.has_live_sources());
}

/// The same text and cursor reconciled twice: the second finds nothing to
/// do.
#[test]
fn reconcile_is_idempotent() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("fo\n");
    invoke_and_answer(&mut session, &reg, id_of(&reg, "s"), &text, 0, &["foo"]);
    let (text, first) = change(&mut session, &reg, text, (2, 2, "o"), 3);
    assert!(matches!(first, Reconciled::Changed { text_changed: true }));
    let second = session.reconcile(&reg, &text, CharOffset::new(3), "");
    assert!(matches!(second, Reconciled::Unchanged));
}

/// A buffer whose text was replaced has no change to carry positions
/// through, so nothing in the session means anything.
#[test]
fn a_replaced_text_dismisses_the_session() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("fo\n");
    invoke_and_answer(&mut session, &reg, id_of(&reg, "s"), &text, 0, &["foo"]);
    session.forget(BufferId::default());
    let outcome = session.reconcile(&reg, &text, CharOffset::new(2), "");
    assert!(matches!(outcome, Reconciled::Dismiss));
}

/// A change to another buffer is none of the session's business.
#[test]
fn a_change_to_another_buffer_is_not_carried() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("fo\n");
    invoke_and_answer(&mut session, &reg, id_of(&reg, "s"), &text, 0, &["foo"]);
    session.reconcile(&reg, &text, CharOffset::new(2), "");
    let mut buffers = slotmap::SlotMap::<BufferId, ()>::with_key();
    let other = buffers.insert(());
    let (cs, after) = edit(&text, 0, 0, "zzz");
    session.carry(other, &TextChange::new(&text, &after, &cs));
    let outcome = session.reconcile(&reg, &text, CharOffset::new(2), "");
    assert!(matches!(outcome, Reconciled::Unchanged));
    assert!(session.has_live_sources());
}

/// A source's own token characters extend the token: `-` typed at its end
/// belongs to the token of a source that declares it, and leaves the token
/// of one that doesn't.
#[test]
fn a_sources_token_chars_decide_what_typing_at_the_end_does() {
    let mut reg = registry(&[("plain", 0)]);
    reg.register_buffer(BufferSourceEntry {
        name: "dashed".into(),
        match_kind: MatchKind::Fuzzy,
        priority: 0,
        proc: SteelVal::Void,
        resolve: false,
        token_chars: "-".into(),
    });
    let (mut session, text) = buffer_session("fo\n");
    invoke_and_answer(&mut session, &reg, id_of(&reg, "plain"), &text, 0, &["foo"]);
    invoke_and_answer(
        &mut session,
        &reg,
        id_of(&reg, "dashed"),
        &text,
        0,
        &["foo-bar"],
    );
    let (text, _) = change(&mut session, &reg, text, (2, 2, "-"), 3);
    session.rank(&reg, live(&text, 3));
    assert_eq!(ranked_labels(&session, &reg), vec!["foo-bar"]);
}

/// Each invocation composes the changes carried since *its own* call: a
/// second call minted after a keystroke starts from a fresh snapshot.
#[test]
fn a_later_invocation_starts_from_its_own_snapshot() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("fo\n");
    let src = id_of(&reg, "s");
    invoke_and_answer(&mut session, &reg, src, &text, 0, &["foo"]);
    let (text, _) = change(&mut session, &reg, text, (2, 2, "o"), 3);
    // Re-invoked against the post-edit document.
    invoke_and_answer(&mut session, &reg, src, &text, 0, &["foobar"]);
    let (text, _) = change(&mut session, &reg, text, (3, 3, "b"), 4);
    session.rank(&reg, live(&text, 4));
    assert_eq!(ranked_labels(&session, &reg), vec!["foobar"]);
    assert_eq!(
        session.menu_anchor(&text).map(ClusterStart::offset),
        Some(CharOffset::new(0))
    );
}

// ── Selection stepping ───────────────────────────────────────────────────────

#[test]
fn step_selection_on_an_empty_ranking_does_not_move() {
    let mut session = MinibufSession::open(String::new(), 0);
    assert!(!session.step_selection(true));
    assert!(!session.step_selection(false));
    assert_eq!(session.selected(), 0);
}

#[test]
fn step_selection_wraps_at_either_end() {
    let reg = minibuf_registry(&[("s", 0)]);
    let mut session = MinibufSession::open("w".into(), 1);
    let id = session.invoke(minibuf_id_of(&reg, "s"), Invocation::minibuf(0..1));
    assert!(session.contribute(id, items(&["wa", "wb"]), false));
    session.rank(&reg);
    assert_eq!(session.selected(), 0, "rank resets to row 0");
    assert!(session.step_selection(true));
    assert_eq!(session.selected(), 1);
    assert!(session.step_selection(true));
    assert_eq!(session.selected(), 0, "wraps forward");
    assert!(session.step_selection(false));
    assert_eq!(session.selected(), 1, "wraps backward");
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
/// panic when `prefix.len()` lands mid-codepoint in a non-ASCII haystack.
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
