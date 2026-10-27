//! Event-name introspection, and the holds a raiser keeps on its handlers.

use hume_core::HostToken;

/// Event-name introspection, accessed through [`EditorHost::events`](super::EditorHost::events).
///
/// The name-based boundary this crate is built on: `hume-scripting` has no
/// compiled-in knowledge of which event names exist (that's the editor's
/// `EditorEvent`), so `register-hook!` and `declare-plugin!`'s `#:events`
/// validate against this instead of a static match.
pub trait EventHost {
    /// Every Steel-visible event name this host can raise.
    fn known_event_names(&self) -> &'static [&'static str];

    /// For an event whose raiser waits for its handlers, the number of args
    /// the event passes; `None` for every other event. A handler of such an
    /// event takes those args and counts as finished when it returns, or
    /// takes one more, `done`, and holds the raiser until it calls `(done)`.
    fn held_event_arg_count(&self, name: &str) -> Option<usize>;

    /// Releases `hold`, one a handler of a held event took: its `done` was
    /// called, or the handler raised. A hold already released, or one that
    /// names nothing, is ignored.
    fn finish_hold(&mut self, hold: HostToken);
}
