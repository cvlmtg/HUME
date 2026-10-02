//! The panic hook is process-wide, so these tests run in their own binary: a
//! deliberate panic in an unrelated test would otherwise be queued as a
//! worker panic.

use hume_platform::worker_panic::WorkerPanics;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

/// Tests in this binary run in parallel threads and share the one hook.
static HOOK_LOCK: Mutex<()> = Mutex::new(());

/// Puts the hook that was current before the test back when dropped, so a
/// failing assertion cannot leave a test's hook installed.
struct RestoreHook(Option<Box<dyn Fn(&panic::PanicHookInfo<'_>) + Send + Sync + 'static>>);

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

/// Installs a counting hook as the "previous" one, then wraps it with
/// `WorkerPanics::install`. Returns the queue, the previous hook's call count
/// and the wake call count.
fn install_counting() -> (WorkerPanics, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let (previous_calls, previous_seen) = counter();
    let (wake_calls, wake_seen) = counter();
    panic::set_hook(Box::new(move |_| {
        previous_calls.fetch_add(1, Ordering::SeqCst);
    }));
    let panics = WorkerPanics::install(Arc::new(move || {
        wake_calls.fetch_add(1, Ordering::SeqCst);
    }));
    (panics, previous_seen, wake_seen)
}

#[test]
fn a_panic_on_another_thread_is_recorded_and_skips_the_previous_hook() {
    let _lock = HOOK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = RestoreHook::take_current();
    let (panics, previous_seen, wake_seen) = install_counting();

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
            .is_some_and(|l| l.contains("worker_panic_hook.rs")),
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
    let (panics, _, _) = install_counting();

    let _ = thread::spawn(|| panic!("static message")).join();

    assert_eq!(panics.take_unreported()[0].message, "static message");
}

#[test]
fn a_panic_on_the_main_thread_goes_to_the_previous_hook_only() {
    let _lock = HOOK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = RestoreHook::take_current();
    let (panics, previous_seen, wake_seen) = install_counting();

    let caught = panic::catch_unwind(AssertUnwindSafe(|| panic!("main thread")));

    assert!(caught.is_err());
    assert_eq!(previous_seen.load(Ordering::SeqCst), 1);
    assert_eq!(wake_seen.load(Ordering::SeqCst), 0);
    assert!(panics.all().is_empty());
}
