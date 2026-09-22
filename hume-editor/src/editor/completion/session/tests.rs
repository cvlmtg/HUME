//! The store on its own: slots, invocations, spans through edits, ranking
//! — no editor, no Steel. The orchestration around it is covered by
//! `editor/tests/completion/`.

use super::*;
use crate::editor::completion::registry::{BufferToken, SourceBody, SourceEntry, SourceTarget};
use hume_editing::changeset::ChangeSetBuilder;
use steel::rvals::SteelVal;

fn item(label: &str) -> CompletionItem {
    CompletionItem::plain(label.into(), label.into(), label.into())
}

fn items(labels: &[&str]) -> Vec<CompletionItem> {
    labels.iter().map(|l| item(l)).collect()
}

/// A registry with one Steel `Buffer` source per `(name, priority)`, all
/// `MatchKind::Fuzzy` — the proc is never called here.
fn registry(sources: &[(&str, i64)]) -> SourceRegistry {
    let mut reg = SourceRegistry::with_defaults();
    for (name, priority) in sources {
        reg.register(SourceEntry {
            name: (*name).into(),
            match_kind: MatchKind::Fuzzy,
            priority: *priority,
            body: SourceBody::Steel {
                proc: SteelVal::Void,
                target: SourceTarget::Buffer(BufferToken::Word),
            },
        });
    }
    reg
}

fn id_of(reg: &SourceRegistry, name: &str) -> SourceId {
    reg.id_of(name).expect("registered")
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
    source: SourceId,
    text: &BufferText,
    start: usize,
    head: usize,
    labels: &[&str],
) {
    let inv = Invocation::buffer(
        text.rope().clone(),
        CharOffset::new(head),
        Some(CharOffset::new(start)..CharOffset::new(head)),
    );
    let id = session.invoke(source, inv);
    session
        .contribute(id, items(labels), false, None)
        .expect("a resolved-span invocation takes no span");
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
        id_of(&reg, "word"),
        &text,
        2,
        4,
        &["foobar", "./x"],
    );
    invoke_and_answer(
        &mut session,
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
    invoke_and_answer(&mut session, id_of(&reg, "word"), &text, 2, 4, &["foobar"]);
    invoke_and_answer(
        &mut session,
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
    assert!(session.observe_edit(&cs, 1, CharOffset::new(5)));
    session.rank(&reg, live(&text, 5));
    assert_eq!(ranked_labels(&session, &reg), vec!["foobar"]);
    assert_eq!(session.menu_anchor_char(), Some(CharOffset::new(2)));
}

#[test]
fn priority_breaks_a_score_tie_before_sort_text() {
    let reg = registry(&[("lo", 0), ("hi", 10)]);
    let (mut session, text) = buffer_session("\n");
    invoke_and_answer(&mut session, id_of(&reg, "lo"), &text, 0, 0, &["aaa"]);
    invoke_and_answer(&mut session, id_of(&reg, "hi"), &text, 0, 0, &["zzz"]);
    session.rank(&reg, live(&text, 0));
    assert_eq!(ranked_labels(&session, &reg), vec!["zzz", "aaa"]);
}

// ── Invocations and answers ──────────────────────────────────────────────────

#[test]
fn an_answer_to_a_superseded_invocation_is_dropped() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("\n");
    let src = id_of(&reg, "s");
    let head = CharOffset::new(0);
    let first = session.invoke(
        src,
        Invocation::buffer(text.rope().clone(), head, Some(head..head)),
    );
    let second = session.invoke(
        src,
        Invocation::buffer(text.rope().clone(), head, Some(head..head)),
    );
    assert_eq!(
        session.contribute(first, items(&["stale"]), false, None),
        Ok(false)
    );
    assert_eq!(
        session.contribute(second, items(&["fresh"]), false, None),
        Ok(true)
    );
    session.rank(&reg, live(&text, 0));
    assert_eq!(ranked_labels(&session, &reg), vec!["fresh"]);
}

#[test]
fn a_repeated_answer_for_the_latest_invocation_replaces_it() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("\n");
    let src = id_of(&reg, "s");
    let head = CharOffset::new(0);
    let id = session.invoke(
        src,
        Invocation::buffer(text.rope().clone(), head, Some(head..head)),
    );
    assert_eq!(
        session.contribute(id, items(&["x", "y"]), false, None),
        Ok(true)
    );
    assert_eq!(
        session.contribute(id, items(&["x", "z"]), false, None),
        Ok(true)
    );
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
    let id = session.invoke(
        src,
        Invocation::buffer(text.rope().clone(), head, Some(head..head)),
    );
    assert!(session.is_pending());
    session.contribute(id, items(&["x"]), true, None).unwrap();
    assert!(!session.is_pending());
    assert!(session.has_live_sources());
    assert_eq!(
        session.sources_to_reinvoke(),
        vec![src],
        "flagged incomplete"
    );
    session.contribute(id, items(&[]), false, None).unwrap();
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
        id_of(&reg, "s"),
        &text,
        0,
        2,
        &["foobar", "fox", "bar"],
    );
    let (cs, text) = edit(&text, 2, 2, "o");
    assert!(session.observe_edit(&cs, 1, CharOffset::new(3)));
    session.rank(&reg, live(&text, 3));
    assert_eq!(ranked_labels(&session, &reg), vec!["foobar"]);
}

/// Deleting the token's own first char stays inside it; deleting the char
/// *before* the token crosses it — the slot's answer is dropped, and the
/// two are told apart by whether `start` and `start - 1` collapse to the
/// same live position.
#[test]
fn deleting_before_the_token_drops_the_slot_but_deleting_its_first_char_does_not() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("x fo\n");
    invoke_and_answer(&mut session, id_of(&reg, "s"), &text, 2, 4, &["foo"]);

    // Backspace twice: "o", then "f" — the token's own chars.
    let (cs, text) = edit(&text, 3, 4, "");
    assert!(session.observe_edit(&cs, 1, CharOffset::new(3)));
    let (cs, text) = edit(&text, 2, 3, "");
    assert!(session.observe_edit(&cs, 2, CharOffset::new(2)));
    assert!(
        session.has_live_sources(),
        "an empty token is still a token"
    );
    session.rank(&reg, live(&text, 2));
    assert_eq!(ranked_labels(&session, &reg), vec!["foo"]);

    // A third Backspace deletes the space before the token.
    let (cs, _) = edit(&text, 1, 2, "");
    assert!(session.observe_edit(&cs, 3, CharOffset::new(1)));
    assert!(!session.has_live_sources(), "crossed the token's start");
}

/// An edit elsewhere in the buffer — a second cursor deleting text far
/// before the token — shifts the token without crossing it.
#[test]
fn a_deletion_elsewhere_shifts_the_token_without_dropping_it() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("abc fo\n");
    invoke_and_answer(&mut session, id_of(&reg, "s"), &text, 4, 6, &["foo"]);
    let (cs, text) = edit(&text, 0, 1, "");
    assert!(session.observe_edit(&cs, 1, CharOffset::new(5)));
    assert!(session.has_live_sources());
    session.rank(&reg, live(&text, 5));
    assert_eq!(ranked_labels(&session, &reg), vec!["foo"]);
    assert_eq!(session.menu_anchor_char(), Some(CharOffset::new(3)));
}

#[test]
fn a_cursor_outside_the_token_drops_the_slot() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("fo bar\n");
    invoke_and_answer(&mut session, id_of(&reg, "s"), &text, 0, 2, &["foo"]);
    // No edit — the cursor just moved (an out-of-band motion).
    let cs = ChangeSet::identity(text.len_chars());
    assert!(session.observe_edit(&cs, 1, CharOffset::new(5)));
    assert!(!session.has_live_sources());
}

#[test]
fn an_edit_the_session_never_saw_is_refused() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("fo\n");
    invoke_and_answer(&mut session, id_of(&reg, "s"), &text, 0, 2, &["foo"]);
    let longer = BufferText::from("fooooo\n");
    let cs = ChangeSet::identity(longer.len_chars());
    assert!(
        !session.observe_edit(&cs, 1, CharOffset::new(2)),
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
    invoke_and_answer(&mut session, src, &text, 0, 2, &["foo"]);
    let (cs, text) = edit(&text, 2, 2, "o");
    assert!(session.observe_edit(&cs, 1, CharOffset::new(3)));
    // Re-invoked against the post-edit document.
    invoke_and_answer(&mut session, src, &text, 0, 3, &["foobar"]);
    let (cs, text) = edit(&text, 3, 3, "b");
    assert!(session.observe_edit(&cs, 2, CharOffset::new(4)));
    session.rank(&reg, live(&text, 4));
    assert_eq!(ranked_labels(&session, &reg), vec!["foobar"]);
    assert_eq!(session.menu_anchor_char(), Some(CharOffset::new(0)));
}

// ── Custom spans ─────────────────────────────────────────────────────────────

fn custom_invocation(text: &BufferText, head: usize) -> Invocation {
    Invocation::buffer(text.rope().clone(), CharOffset::new(head), None)
}

#[test]
fn a_custom_span_is_validated_against_the_snapshot_and_mapped_to_live() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("x fo\n");
    let id = session.invoke(id_of(&reg, "s"), custom_invocation(&text, 4));
    // A keystroke lands before the (still pending) answer.
    let (cs, text) = edit(&text, 4, 4, "o");
    assert!(session.observe_edit(&cs, 1, CharOffset::new(5)));
    // The answer names its span in the snapshot's own coordinates.
    assert_eq!(
        session.contribute(id, items(&["foobar"]), false, Some((2, 4))),
        Ok(true)
    );
    session.rank(&reg, live(&text, 5));
    assert_eq!(
        ranked_labels(&session, &reg),
        vec!["foobar"],
        "filtered on the live \"foo\""
    );
    assert_eq!(session.menu_anchor_char(), Some(CharOffset::new(2)));
}

#[test]
fn a_custom_span_is_required_and_checked() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("abc\ndef\n");
    let src = id_of(&reg, "s");
    let err = |session: &mut CompletionSession, span| {
        let id = session.invoke(src, custom_invocation(&text, 5));
        session
            .contribute(id, items(&["x"]), false, span)
            .expect_err("rejected")
    };
    assert!(err(&mut session, None).contains("#:span is required"));
    assert!(err(&mut session, Some((0, 99))).contains("out of range"));
    assert!(err(&mut session, Some((6, 7))).contains("does not contain the cursor"));
    assert!(err(&mut session, Some((0, 5))).contains("more than one line"));
    assert!(
        !session.is_pending() && !session.has_live_sources(),
        "a rejected answer leaves the slot with nothing — neither pending nor live"
    );
}

#[test]
fn a_minibuf_custom_span_is_checked_against_the_input() {
    let reg = registry(&[("s", 0)]);
    let mut session = CompletionSession::open_minibuf("e src/ma".into(), 8);
    let src = id_of(&reg, "s");
    let err = |session: &mut CompletionSession, span| {
        let id = session.invoke(src, Invocation::minibuf(None));
        session
            .contribute(id, items(&["x"]), false, span)
            .expect_err("rejected")
    };
    assert!(err(&mut session, Some((2, 4))).contains("must contain the cursor"));
    assert!(err(&mut session, Some((2, 99))).contains("must contain the cursor"));
    let id = session.invoke(src, Invocation::minibuf(None));
    assert_eq!(
        session.contribute(id, items(&["src/main.rs"]), false, Some((2, 8))),
        Ok(true)
    );
    session.rank(&reg, None);
    assert_eq!(session.minibuf_apply(0), Some((2..8, "src/main.rs")));
    assert_eq!(session.menu_anchor_byte(), Some(2));
}

/// A source may stream: a repeated answer for a still-latest id replaces
/// the earlier one wholesale, `#:span` included — the earlier span must
/// not stick once a later answer names a different one.
#[test]
fn a_second_answer_for_a_still_latest_custom_span_replaces_it() {
    let reg = registry(&[("s", 0)]);
    let (mut session, text) = buffer_session("ab foo\n");
    let id = session.invoke(id_of(&reg, "s"), custom_invocation(&text, 6));
    assert_eq!(
        session.contribute(id, items(&["ab foobar"]), false, Some((3, 6))),
        Ok(true)
    );
    session.rank(&reg, live(&text, 6));
    assert_eq!(
        session.menu_anchor_char(),
        Some(CharOffset::new(3)),
        "sanity"
    );

    assert_eq!(
        session.contribute(id, items(&["ab foobar"]), false, Some((0, 6))),
        Ok(true)
    );
    session.rank(&reg, live(&text, 6));
    assert_eq!(
        session.menu_anchor_char(),
        Some(CharOffset::new(0)),
        "a re-emitted #:span for a still-latest id must replace the earlier \
         span, not be dropped"
    );
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
    let reg = registry(&[("s", 0)]);
    let mut session = CompletionSession::open_minibuf("w".into(), 1);
    let id = session.invoke(id_of(&reg, "s"), Invocation::minibuf(Some(0..1)));
    session
        .contribute(id, items(&["wa", "wb"]), false, None)
        .unwrap();
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
