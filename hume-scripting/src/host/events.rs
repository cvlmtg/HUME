//! Event-name introspection — moved out of `host.rs`'s per-capability
//! split.

/// Event-name introspection — accessed through [`EditorHost::events`](super::EditorHost::events).
///
/// The name-based boundary this crate is built on: `hume-scripting` has no
/// compiled-in knowledge of which event names exist (that's the editor's
/// `EditorEvent`), so `register-hook!` and `declare-plugin`'s `#:events`
/// validate against this instead of a static match.
pub trait EventHost {
    /// Every Steel-visible event name this host can raise.
    fn known_event_names(&self) -> &'static [&'static str];
}
