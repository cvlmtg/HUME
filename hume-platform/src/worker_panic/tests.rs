use super::*;

fn panic_named(message: &str) -> WorkerPanic {
    WorkerPanic {
        thread: "w".into(),
        location: None,
        message: message.into(),
        backtrace: None,
    }
}

#[test]
fn take_unreported_hands_out_each_panic_once_and_all_keeps_them() {
    let panics = WorkerPanics::default();
    panics.record(panic_named("one"));
    panics.record(panic_named("two"));

    let first: Vec<_> = panics
        .take_unreported()
        .into_iter()
        .map(|p| p.message)
        .collect();
    assert_eq!(first, ["one", "two"]);
    assert!(panics.take_unreported().is_empty());

    panics.record(panic_named("three"));
    let second: Vec<_> = panics
        .take_unreported()
        .into_iter()
        .map(|p| p.message)
        .collect();
    assert_eq!(second, ["three"]);
    let all: Vec<_> = panics.all().into_iter().map(|p| p.message).collect();
    assert_eq!(all, ["one", "two", "three"]);
}

#[test]
fn take_unprinted_counts_apart_from_take_unreported() {
    let panics = WorkerPanics::default();
    panics.record(panic_named("one"));
    assert_eq!(panics.take_unreported().len(), 1);

    let printed: Vec<_> = panics
        .take_unprinted()
        .into_iter()
        .map(|p| p.message)
        .collect();
    assert_eq!(printed, ["one"]);
    assert!(panics.take_unprinted().is_empty());

    panics.record(panic_named("two"));
    let printed: Vec<_> = panics
        .take_unprinted()
        .into_iter()
        .map(|p| p.message)
        .collect();
    assert_eq!(printed, ["two"]);
}

#[test]
fn payload_message_reads_str_and_string_payloads() {
    let from_str: Box<dyn std::any::Any + Send> = Box::new("literal");
    let from_string: Box<dyn std::any::Any + Send> = Box::new(String::from("formatted"));
    let other: Box<dyn std::any::Any + Send> = Box::new(5_u8);

    assert_eq!(payload_message(&*from_str), "literal");
    assert_eq!(payload_message(&*from_string), "formatted");
    assert_eq!(payload_message(&*other), "non-string panic payload");
}
