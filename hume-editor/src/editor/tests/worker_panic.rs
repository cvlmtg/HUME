//! Reporting panics from background threads.

use super::*;
use hume_platform::worker_panic::WorkerPanics;
use pretty_assertions::assert_eq;

#[test]
fn a_worker_panic_is_reported_as_one_error() {
    let mut ed = editor_from("-[a]>bc\n");
    let panics = WorkerPanics::default();
    ed.attach_worker_panics(panics.clone());
    panics.record(lsp_reader_panic());

    ed.report_worker_panics();

    let errors = error_log(&ed);
    assert_eq!(errors.len(), 1);
    insta::assert_snapshot!(
        errors[0],
        @"thread 'hume-lsp-reader' panicked at hume-lsp/src/transport.rs:150:9: bad frame"
    );
    assert_eq!(ed.state.status_msg.as_deref(), Some(errors[0].as_str()));
}

#[test]
fn a_worker_panic_is_reported_once() {
    let mut ed = editor_from("-[a]>bc\n");
    let panics = WorkerPanics::default();
    ed.attach_worker_panics(panics.clone());
    panics.record(lsp_reader_panic());

    ed.report_worker_panics();
    ed.report_worker_panics();

    assert_eq!(error_log(&ed).len(), 1);
}

#[test]
fn an_editor_with_no_worker_panics_reports_none() {
    let mut ed = editor_from("-[a]>bc\n");

    ed.report_worker_panics();

    assert!(error_log(&ed).is_empty());
}
