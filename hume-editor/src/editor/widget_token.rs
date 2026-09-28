//! One process-unique token counter, shared by every overlay widget that
//! hands Steel an opaque handle scoping later mutation to the instance that
//! minted it (`PopupLayer`, `MenuLayer`, `DrawerLayer`, `PickerSession`)
//! and by every completion `Invocation` (a handle scoping a source's answer
//! to the one call that asked for it): a late async callback racing a widget
//! the user already closed or replaced, or a source answering a call a
//! later keystroke superseded, reads as a silent no-op rather than reaching
//! the wrong instance. A single global counter is a superset of several private
//! ones: it still guarantees uniqueness (no two tokens minted anywhere, of
//! any kind, are ever equal), so a value one widget mints can never alias
//! another kind's live token either.

use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Mints a fresh token. Starts at `1`, never `0`, so a caller that needs a
/// token-shaped value with nothing behind it can use `0` knowing no live
/// instance holds it. Every widget's/invocation's own constructor calls
/// this once, at construction, as its token field's only initializer.
pub(in crate::editor) fn next() -> u64 {
    NEXT_TOKEN.fetch_add(1, Ordering::Relaxed)
}
