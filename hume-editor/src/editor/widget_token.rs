//! One process-unique token counter, shared by every overlay widget that
//! hands Steel an opaque handle scoping later mutation to the instance that
//! minted it (`DrawerLayer`, `PickerSession`, `CompletionSession`) — a late
//! async callback racing a widget the user already closed or replaced reads
//! as a silent no-op rather than reaching the wrong instance. A single
//! global counter is a superset of three private ones: it still guarantees
//! per-widget uniqueness (no two tokens minted anywhere, of any kind, are
//! ever equal), so a value one widget mints can never alias another kind's
//! live token either.

use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Mints a fresh token. Starts at `1` (see [`DEAD`]) — every widget's own
/// constructor calls this once, at construction, as its token field's only
/// initializer.
pub(in crate::editor) fn next() -> u64 {
    NEXT_TOKEN.fetch_add(1, Ordering::Relaxed)
}

/// A token no live widget can ever hold, since [`next`] starts at `1` and
/// never wraps back to `0` in a process's lifetime — the named stand-in for
/// a caller that needs a token-shaped value with no widget behind it (an
/// opener that closed the request as stale before minting one).
pub(in crate::editor) const DEAD: u64 = 0;
