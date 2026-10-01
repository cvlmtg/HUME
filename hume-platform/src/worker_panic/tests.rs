use super::*;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The panic hook is process-wide: tests that install one run one at a time.
static HOOK_LOCK: Mutex<()> = Mutex::new(());

/// Puts the previous panic hook back when dropped, so a failing assertion
/// cannot leave a test's hook installed.
struct RestoreHook(Option<Hook>);

impl RestoreHook {
    fn take_current() -> Self {
        Self(Some(panic::take_hook()))
    }
}

impl Drop for RestoreHook {
    fn drop(&mut self) {
        if let Some(hook) = self.0.take() {
            panic::set_hook(hook);
        }
    }
}

fn counter() -> (Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let n = Arc::new(AtomicUsize::new(0));
    (n.clone(), n)
}

#[test]
fn a_panic_on_another_thread_is_recorded_and_skips_the_previous_hook() {
    let _lock = HOOK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = RestoreHook::take_current();
    let (previous_calls, previous_seen) = counter();
    let (wake_calls, wake_seen) = counter();
    let panics = WorkerPanics::install_over(
        Box::new(move |_| {
            previous_calls.fetch_add(1, Ordering::SeqCst);
        }),
        thread::current().id(),
        Arc::new(move || {
            wake_calls.fetch_add(1, Ordering::SeqCst);
        }),
    );

    let joined = thread::Builder::new()
        .name("hume-test-worker".into())
        .spawn(|| panic!("boom {}", 7))
        .unwrap()
        .join();

    assert!(joined.is_err());
    let recorded = panics.take_unreported();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].thread, "hume-test-worker");
    assert_eq!(recorded[0].message, "boom 7");
    assert!(
        recorded[0]
            .location
            .as_deref()
            .is_some_and(|l| l.contains("tests.rs")),
        "got {:?}",
        recorded[0].location
    );
    assert_eq!(wake_seen.load(Ordering::SeqCst), 1);
    assert_eq!(previous_seen.load(Ordering::SeqCst), 0);
}

#[test]
fn a_static_str_panic_message_is_recorded() {
    let _lock = HOOK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = RestoreHook::take_current();
    let panics =
        WorkerPanics::install_over(Box::new(|_| {}), thread::current().id(), Arc::new(|| {}));

    let _ = thread::spawn(|| panic!("static message")).join();

    assert_eq!(panics.take_unreported()[0].message, "static message");
}

#[test]
fn a_panic_on_the_main_thread_goes_to_the_previous_hook_only() {
    let _lock = HOOK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = RestoreHook::take_current();
    let (previous_calls, previous_seen) = counter();
    let (wake_calls, wake_seen) = counter();
    let panics = WorkerPanics::install_over(
        Box::new(move |_| {
            previous_calls.fetch_add(1, Ordering::SeqCst);
        }),
        thread::current().id(),
        Arc::new(move || {
            wake_calls.fetch_add(1, Ordering::SeqCst);
        }),
    );

    let caught = panic::catch_unwind(AssertUnwindSafe(|| panic!("main thread")));

    assert!(caught.is_err());
    assert_eq!(previous_seen.load(Ordering::SeqCst), 1);
    assert_eq!(wake_seen.load(Ordering::SeqCst), 0);
    assert!(panics.all().is_empty());
}

fn panic_named(message: &str) -> WorkerPanic {
    WorkerPanic {
        thread: "w".into(),
        location: None,
        message: message.into(),
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
fn payload_message_reads_str_and_string_payloads() {
    let from_str: Box<dyn std::any::Any + Send> = Box::new("literal");
    let from_string: Box<dyn std::any::Any + Send> = Box::new(String::from("formatted"));
    let other: Box<dyn std::any::Any + Send> = Box::new(5_u8);

    assert_eq!(payload_message(&*from_str), "literal");
    assert_eq!(payload_message(&*from_string), "formatted");
    assert_eq!(payload_message(&*other), "non-string panic payload");
}
