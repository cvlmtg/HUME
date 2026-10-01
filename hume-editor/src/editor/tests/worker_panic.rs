//! Reporting panics from background threads.

use super::*;
use hume_platform::worker_panic::{WorkerPanic, WorkerPanics};
use pretty_assertions::assert_eq;

fn lsp_reader_panic() -> WorkerPanic {
    WorkerPanic {
        thread: "hume-lsp-reader".into(),
        location: Some("hume-lsp/src/transport.rs:150:9".into()),
        message: "bad frame".into(),
    }
}

fn error_texts(ed: &Editor) -> Vec<String> {
    ed.state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect()
}

#[test]
fn a_worker_panic_is_reported_as_one_error() {
    let mut ed = editor_from("-[a]>bc\n");
    let panics = WorkerPanics::default();
    ed.attach_worker_panics(panics.clone());
    panics.record(lsp_reader_panic());

    ed.report_worker_panics();

    let errors = error_texts(&ed);
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

    assert_eq!(error_texts(&ed).len(), 1);
}

#[test]
fn an_editor_with_no_worker_panics_reports_none() {
    let mut ed = editor_from("-[a]>bc\n");

    ed.report_worker_panics();

    assert!(error_texts(&ed).is_empty());
}
