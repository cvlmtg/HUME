//! `EditorHostImpl`'s event-name introspection.

use super::EditorHostImpl;
use hume_core::HostToken;
use hume_scripting::host::EventHost;

impl<'a> EventHost for EditorHostImpl<'a> {
    fn known_event_names(&self) -> &'static [&'static str] {
        crate::editor::event::known_event_names()
    }

    fn held_event_arg_count(&self, name: &str) -> Option<usize> {
        crate::editor::event::held_event_arg_count(name)
    }

    fn finish_hold(&mut self, hold: HostToken) {
        self.state.pending_writes.finish(hold);
    }
}
