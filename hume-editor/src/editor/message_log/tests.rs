use super::*;

fn make_log(entries: &[(Severity, &str)]) -> MessageLog {
    let mut log = MessageLog::new();
    for (sev, text) in entries {
        log.push(*sev, text.to_string());
    }
    log
}

#[test]
fn push_and_entries() {
    let log = make_log(&[(Severity::Warning, "first"), (Severity::Error, "second")]);
    let entries: Vec<_> = log.entries().collect();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].text, "first");
    assert_eq!(entries[1].severity, Severity::Error);
}

#[test]
fn unseen_counts_initial() {
    let log = make_log(&[
        (Severity::Error, "e1"),
        (Severity::Warning, "w1"),
        (Severity::Warning, "w2"),
        (Severity::Trace, "t1"),
    ]);
    let (errors, warnings) = log.unseen_counts();
    assert_eq!(errors, 1);
    assert_eq!(warnings, 2);
    // Trace entries are not counted in the summary counts.
}

#[test]
fn has_unseen_empty_log() {
    let log = MessageLog::new();
    assert!(!log.has_unseen());
}

#[test]
fn mark_all_seen_clears_unseen() {
    let mut log = make_log(&[(Severity::Error, "e1"), (Severity::Warning, "w1")]);
    assert!(log.has_unseen());
    log.mark_all_seen();
    assert!(!log.has_unseen());
    let (e, w) = log.unseen_counts();
    assert_eq!((e, w), (0, 0));
}

#[test]
fn new_entries_after_mark_seen_become_unseen() {
    let mut log = make_log(&[(Severity::Error, "old")]);
    log.mark_all_seen();
    assert!(!log.has_unseen());

    log.push(Severity::Warning, "new".to_string());
    assert!(log.has_unseen());
    let (e, w) = log.unseen_counts();
    assert_eq!((e, w), (0, 1)); // only the new warning
}

#[test]
fn summary_text_none_when_all_seen() {
    let mut log = make_log(&[(Severity::Error, "e")]);
    log.mark_all_seen();
    assert!(log.summary_text().is_none());
}

#[test]
fn summary_text_errors_only() {
    let log = make_log(&[(Severity::Error, "e1"), (Severity::Error, "e2")]);
    assert_eq!(
        log.summary_text().unwrap(),
        "2 errors — :messages for details"
    );
}

#[test]
fn summary_text_single_error() {
    let log = make_log(&[(Severity::Error, "e")]);
    assert_eq!(
        log.summary_text().unwrap(),
        "1 error — :messages for details"
    );
}

#[test]
fn summary_text_warnings_only() {
    let log = make_log(&[(Severity::Warning, "w")]);
    assert_eq!(
        log.summary_text().unwrap(),
        "1 warning — :messages for details"
    );
}

#[test]
fn summary_text_mixed() {
    let log = make_log(&[
        (Severity::Error, "e"),
        (Severity::Warning, "w1"),
        (Severity::Warning, "w2"),
    ]);
    assert_eq!(
        log.summary_text().unwrap(),
        "1 error, 2 warnings — :messages for details"
    );
}

#[test]
fn summary_text_trace_only() {
    let log = make_log(&[(Severity::Trace, "t1"), (Severity::Trace, "t2")]);
    assert!(log.summary_text().is_none());
}

#[test]
fn format_for_display_empty() {
    let log = MessageLog::new();
    assert_eq!(log.format_for_display(), "");
}

#[test]
fn format_for_display_prefixes() {
    let log = make_log(&[
        (Severity::Warning, "bad key"),
        (Severity::Error, "crash"),
        (Severity::Trace, "stack trace here"),
    ]);
    let out = log.format_for_display();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "[warning] bad key");
    assert_eq!(lines[1], "[error] crash");
    assert_eq!(lines[2], "[trace] stack trace here");
}

#[test]
fn push_respects_cap() {
    let mut log = MessageLog::new();
    // Push MAX_ENTRIES + 1 entries; the oldest should be evicted.
    for i in 0..=MAX_ENTRIES {
        log.push(Severity::Warning, format!("msg {i}"));
    }
    assert_eq!(log.entries().len(), MAX_ENTRIES);
    // The first entry should now be "msg 1" (msg 0 was evicted).
    let entries: Vec<_> = log.entries().collect();
    assert_eq!(entries[0].text, "msg 1");
    assert_eq!(entries[MAX_ENTRIES - 1].text, format!("msg {MAX_ENTRIES}"));
}

#[test]
fn push_cap_adjusts_seen_up_to() {
    let mut log = MessageLog::new();
    // Push MAX_ENTRIES entries and mark them all seen.
    for i in 0..MAX_ENTRIES {
        log.push(Severity::Warning, format!("msg {i}"));
    }
    log.mark_all_seen();
    assert!(!log.has_unseen());

    // Pushing one more evicts the oldest and shifts seen_up_to.
    log.push(Severity::Error, "overflow".to_string());
    // The new entry is unseen.
    assert!(log.has_unseen());
    let (e, w) = log.unseen_counts();
    assert_eq!((e, w), (1, 0));
}

/// `totals()` must keep counting past `MAX_ENTRIES` — the whole reason it
/// exists over `unseen_counts()`, which reads the live (evicting) deque and
/// so cannot answer "how many errors/warnings ever landed" once eviction
/// starts. Fail oracle: change `total_errors`/`total_warnings` to derive
/// from `entries.len()` instead of their own monotonic counters, and this
/// must start failing once eviction kicks in.
#[test]
fn totals_survive_eviction_past_max_entries() {
    let mut log = MessageLog::new();
    for i in 0..MAX_ENTRIES + 5 {
        log.push(Severity::Error, format!("e{i}"));
    }
    assert_eq!(
        log.entries().len(),
        MAX_ENTRIES,
        "sanity: the live deque itself must be capped"
    );
    assert_eq!(
        log.totals(),
        ((MAX_ENTRIES + 5) as u64, 0),
        "totals() must count every push ever made, not just what's still \
         in the (capped) deque"
    );
}

/// `Info`/`Trace` entries must not move either total — only `Error`/
/// `Warning` are the "did this reload go badly" signal `typed_reload_config`
/// diffs against.
#[test]
fn totals_ignore_info_and_trace() {
    let log = make_log(&[
        (Severity::Info, "i1"),
        (Severity::Trace, "t1"),
        (Severity::Trace, "t2"),
    ]);
    assert_eq!(log.totals(), (0, 0));
}
