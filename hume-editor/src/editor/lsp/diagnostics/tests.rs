use super::*;
use hume_editing::changeset::ChangeSetBuilder;
use hume_engine::pipeline::EngineView;
use hume_engine::theme::Theme;

fn make_bid() -> BufferId {
    let mut ev = EngineView::new(Theme::default());
    ev.buffers.insert(())
}

/// Two guaranteed-distinct `BufferId`s — `make_bid()` calls each start a
/// fresh `EngineView` with its own slotmap, so two separate calls are
/// *not* guaranteed distinct (both can land on the same first-insert
/// key). Needed by tests that must tell "this buffer" from "some other
/// buffer" apart.
fn make_two_bids() -> (BufferId, BufferId) {
    let mut ev = EngineView::new(Theme::default());
    let a = ev.buffers.insert(());
    let b = ev.buffers.insert(());
    (a, b)
}

fn diag(start: usize, end: usize, severity: DiagSeverity) -> StoredDiag {
    StoredDiag {
        start,
        end,
        severity,
        message: "boom".to_string(),
        code: None,
        source: None,
        raw: serde_json::Value::Null,
    }
}

#[test]
fn counts_tallies_errors_and_warnings_only() {
    let mut store = DiagnosticsStore::default();
    let bid = make_bid();
    store.replace(
        ServerId(0),
        bid,
        vec![
            diag(0, 1, DiagSeverity::Error),
            diag(2, 3, DiagSeverity::Error),
            diag(4, 5, DiagSeverity::Warning),
            diag(6, 7, DiagSeverity::Info),
            diag(8, 9, DiagSeverity::Hint),
        ],
    );
    assert_eq!(store.counts(bid), (2, 1));
}

#[test]
fn counts_is_zero_for_an_unknown_buffer() {
    let store = DiagnosticsStore::default();
    assert_eq!(store.counts(make_bid()), (0, 0));
}

#[test]
fn remove_server_clears_that_servers_diagnostics_only() {
    let mut store = DiagnosticsStore::default();
    let bid = make_bid();
    store.replace(ServerId(0), bid, vec![diag(0, 1, DiagSeverity::Error)]);
    store.replace(ServerId(1), bid, vec![diag(2, 3, DiagSeverity::Warning)]);

    let touched = store.remove_server(ServerId(0));
    assert_eq!(touched, vec![bid]);
    assert_eq!(
        store.counts(bid),
        (0, 1),
        "server 1's diagnostic must survive"
    );
}

#[test]
fn remove_server_drops_the_buffer_entry_once_no_server_remains() {
    let mut store = DiagnosticsStore::default();
    let bid = make_bid();
    store.replace(ServerId(0), bid, vec![diag(0, 1, DiagSeverity::Error)]);

    store.remove_server(ServerId(0));
    assert_eq!(store.counts(bid), (0, 0));
    assert!(
        store
            .for_range(bid, 0..100, DiagSeverity::Hint)
            .next()
            .is_none(),
        "no entry should remain for a buffer with no servers left"
    );
}

#[test]
fn for_range_is_globally_sorted_across_multiple_servers() {
    let mut store = DiagnosticsStore::default();
    let bid = make_bid();
    // Server 0 (inserted first) publishes a diagnostic starting later;
    // server 1 (inserted after) publishes one starting earlier —
    // concatenating in insertion order would put the later one first.
    store.replace(ServerId(0), bid, vec![diag(10, 12, DiagSeverity::Error)]);
    store.replace(ServerId(1), bid, vec![diag(0, 2, DiagSeverity::Warning)]);

    let starts: Vec<usize> = store
        .for_range(bid, 0..100, DiagSeverity::Hint)
        .map(|d| d.start)
        .collect();
    assert_eq!(
        starts,
        vec![0, 10],
        "results must be globally start-ascending regardless of server insertion order"
    );
}

#[test]
fn remove_server_is_a_no_op_for_a_server_with_nothing_stored() {
    let mut store = DiagnosticsStore::default();
    let bid = make_bid();
    store.replace(ServerId(0), bid, vec![diag(0, 1, DiagSeverity::Error)]);
    let gen_before = store.generation;

    let touched = store.remove_server(ServerId(99));
    assert!(touched.is_empty());
    assert_eq!(
        store.generation, gen_before,
        "no change must not bump generation"
    );
    assert_eq!(
        store.counts(bid),
        (1, 0),
        "unrelated server's diagnostics must survive"
    );
}

#[test]
fn remove_buffer_clears_every_servers_diagnostics_for_that_buffer() {
    let mut store = DiagnosticsStore::default();
    let (bid, other_bid) = make_two_bids();
    store.replace(ServerId(0), bid, vec![diag(0, 1, DiagSeverity::Error)]);
    store.replace(ServerId(1), bid, vec![diag(2, 3, DiagSeverity::Warning)]);
    store.replace(
        ServerId(0),
        other_bid,
        vec![diag(0, 1, DiagSeverity::Error)],
    );

    store.remove_buffer(bid);

    assert_eq!(
        store.counts(bid),
        (0, 0),
        "every server's entry for bid must be gone"
    );
    assert_eq!(
        store.counts(other_bid),
        (1, 0),
        "an unrelated buffer's diagnostics must survive"
    );
}

#[test]
fn for_range_respects_severity_floor() {
    let mut store = DiagnosticsStore::default();
    let bid = make_bid();
    store.replace(
        ServerId(0),
        bid,
        vec![
            diag(0, 1, DiagSeverity::Error),
            diag(2, 3, DiagSeverity::Warning),
            diag(4, 5, DiagSeverity::Info),
        ],
    );
    let kept: Vec<DiagSeverity> = store
        .for_range(bid, 0..100, DiagSeverity::Warning)
        .map(|d| d.severity)
        .collect();
    assert_eq!(kept, vec![DiagSeverity::Error, DiagSeverity::Warning]);
}

#[test]
fn for_range_respects_range_bounds() {
    let mut store = DiagnosticsStore::default();
    let bid = make_bid();
    store.replace(
        ServerId(0),
        bid,
        vec![
            diag(0, 5, DiagSeverity::Error),
            diag(10, 15, DiagSeverity::Error),
            diag(20, 25, DiagSeverity::Error),
        ],
    );
    let kept: Vec<(usize, usize)> = store
        .for_range(bid, 8..18, DiagSeverity::Hint)
        .map(|d| (d.start, d.end))
        .collect();
    assert_eq!(kept, vec![(10, 15)]);
}

#[test]
fn for_range_keeps_a_diagnostic_that_starts_before_the_range_but_overlaps_it() {
    // Regression test for the partition_point optimization in `for_range`:
    // the inner Vec is sorted by `start`, not `end`, so a diagnostic that
    // starts before the queried range can still overlap it and must not
    // be dropped by the upper-bound cut.
    let mut store = DiagnosticsStore::default();
    let bid = make_bid();
    store.replace(ServerId(0), bid, vec![diag(0, 10, DiagSeverity::Error)]);

    let kept: Vec<(usize, usize)> = store
        .for_range(bid, 8..18, DiagSeverity::Hint)
        .map(|d| (d.start, d.end))
        .collect();
    assert_eq!(
        kept,
        vec![(0, 10)],
        "a diagnostic starting before the range must survive if it overlaps"
    );
}

#[test]
fn remap_insert_before_shifts_the_range() {
    let mut store = DiagnosticsStore::default();
    let bid = make_bid();
    store.replace(ServerId(0), bid, vec![diag(10, 15, DiagSeverity::Error)]);

    let mut b = ChangeSetBuilder::new(20);
    b.retain(0).insert("XXX").retain_rest();
    store.remap_through(bid, &b.finish());

    let kept: Vec<(usize, usize)> = store
        .for_range(bid, 0..100, DiagSeverity::Hint)
        .map(|d| (d.start, d.end))
        .collect();
    assert_eq!(
        kept,
        vec![(13, 18)],
        "an insert before the range shifts it forward"
    );
}

#[test]
fn remap_insert_inside_grows_the_range() {
    let mut store = DiagnosticsStore::default();
    let bid = make_bid();
    store.replace(ServerId(0), bid, vec![diag(10, 15, DiagSeverity::Error)]);

    let mut b = ChangeSetBuilder::new(20);
    b.retain(12).insert("XX").retain_rest();
    store.remap_through(bid, &b.finish());

    let kept: Vec<(usize, usize)> = store
        .for_range(bid, 0..100, DiagSeverity::Hint)
        .map(|d| (d.start, d.end))
        .collect();
    assert_eq!(kept, vec![(10, 17)], "an insert inside the range grows it");
}

#[test]
fn remap_insert_after_leaves_the_range_unchanged() {
    let mut store = DiagnosticsStore::default();
    let bid = make_bid();
    store.replace(ServerId(0), bid, vec![diag(10, 15, DiagSeverity::Error)]);

    let mut b = ChangeSetBuilder::new(20);
    b.retain(18).insert("XX").retain_rest();
    store.remap_through(bid, &b.finish());

    let kept: Vec<(usize, usize)> = store
        .for_range(bid, 0..100, DiagSeverity::Hint)
        .map(|d| (d.start, d.end))
        .collect();
    assert_eq!(
        kept,
        vec![(10, 15)],
        "an insert after the range must not move it"
    );
}

#[test]
fn remap_deletion_covering_the_range_drops_it() {
    let mut store = DiagnosticsStore::default();
    let bid = make_bid();
    store.replace(ServerId(0), bid, vec![diag(10, 15, DiagSeverity::Error)]);

    let mut b = ChangeSetBuilder::new(20);
    b.retain(5).delete(15).retain_rest();
    store.remap_through(bid, &b.finish());

    let kept: Vec<(usize, usize)> = store
        .for_range(bid, 0..100, DiagSeverity::Hint)
        .map(|d| (d.start, d.end))
        .collect();
    assert!(
        kept.is_empty(),
        "a deletion covering the range must drop it, not zero it"
    );
}

#[test]
fn remap_bumps_generation_only_when_the_buffer_has_stored_diagnostics() {
    let mut store = DiagnosticsStore::default();
    let bid = make_bid();
    let mut b = ChangeSetBuilder::new(5);
    b.retain(0).insert("X").retain_rest();
    let cs = b.finish();

    let gen_before = store.generation;
    store.remap_through(bid, &cs); // no entry for bid — no-op
    assert_eq!(store.generation, gen_before);

    store.replace(ServerId(0), bid, vec![diag(0, 1, DiagSeverity::Error)]);
    let gen_after_replace = store.generation;
    store.remap_through(bid, &cs);
    assert_eq!(store.generation, gen_after_replace + 1);
}

/// `DiagnosticsStore::remap_through` now goes through the same
/// `SourceStore::remap_ranges` `ExtraHighlightEntry` uses
/// (`decorations.rs`) — this pins that shared policy for the diagnostics
/// instantiation: a diagnostic a covering deletion collapses to zero width
/// is dropped, not kept as a zero-width entry.
#[test]
fn remap_through_drops_a_diagnostic_a_covering_deletion_collapses() {
    let mut store = DiagnosticsStore::default();
    let bid = make_bid();
    store.replace(ServerId(0), bid, vec![diag(2, 5, DiagSeverity::Error)]);

    // Delete chars 0..8 of a 10-char document — fully covers [2, 5).
    let mut b = ChangeSetBuilder::new(10);
    b.delete(8).retain_rest();
    let cs = b.finish();
    store.remap_through(bid, &cs);

    // `counts` iterates every stored entry with no range/severity filter —
    // unlike `for_range`, it can't coincidentally exclude a surviving
    // zero-width entry the way a `d.end > lo` check with `lo == 0` would.
    assert_eq!(
        store.counts(bid),
        (0, 0),
        "a diagnostic fully covered by a deletion must be dropped, not kept \
         as a zero-width entry"
    );
}

#[test]
fn map_severity_absent_defaults_to_error() {
    assert_eq!(map_severity(None), DiagSeverity::Error);
}

#[test]
fn map_severity_maps_the_known_wire_values() {
    assert_eq!(
        map_severity(Some(lsp_types::DiagnosticSeverity::ERROR)),
        DiagSeverity::Error
    );
    assert_eq!(
        map_severity(Some(lsp_types::DiagnosticSeverity::WARNING)),
        DiagSeverity::Warning
    );
    assert_eq!(
        map_severity(Some(lsp_types::DiagnosticSeverity::INFORMATION)),
        DiagSeverity::Info
    );
    assert_eq!(
        map_severity(Some(lsp_types::DiagnosticSeverity::HINT)),
        DiagSeverity::Hint
    );
}

#[test]
fn widen_zero_length_widens_forward_mid_line() {
    let rope = Rope::from_str("hello\n");
    assert_eq!(widen_zero_length(&rope, 2), (2, 3));
}

#[test]
fn widen_zero_length_widens_backward_at_end_of_line() {
    let rope = Rope::from_str("hello\n");
    // Position 5 is the '\n' — widening forward would cross the line
    // boundary, so it must widen backward instead.
    assert_eq!(widen_zero_length(&rope, 5), (4, 5));
}

#[test]
fn widen_zero_length_widens_backward_at_end_of_buffer() {
    let rope = Rope::from_str("hi");
    assert_eq!(widen_zero_length(&rope, 2), (1, 2));
}

/// On the minimal 1-char "\n" buffer, `pos = 0` has no char to widen
/// onto in either direction under the general rule — it must widen onto
/// the structural newline itself rather than staying `(0, 0)`.
#[test]
fn widen_zero_length_widens_onto_the_newline_on_the_minimal_buffer() {
    let rope = Rope::from_str("\n");
    assert_eq!(widen_zero_length(&rope, 0), (0, 1));
}
