//! Who a set of trigger characters fires.

use std::sync::Arc;

/// Who a trigger set fires: the kind is the variant, so a hook listener and a
/// completion source with the same name are different listeners.
///
/// Orders hooks before completion sources, then by name; that is the order a
/// trigger table yields its listeners in.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TriggerListener {
    /// A name `on-trigger-char` handlers filter on.
    Hook(Arc<str>),
    /// A `'buffer` completion source, by the name it is registered under.
    Completion(Arc<str>),
}
