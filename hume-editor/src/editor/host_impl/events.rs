//! `EditorHostImpl`'s event-name introspection.

use super::EditorHostImpl;
use hume_core::HostToken;
use hume_scripting::host::{EventHost, EventShape};

impl<'a> EventHost for EditorHostImpl<'a> {
    fn known_event_names(&self) -> &'static [&'static str] {
        crate::editor::event::known_event_names()
    }

    fn event_shape(&self, name: &str) -> Option<EventShape> {
        crate::editor::event::event_shape(name)
    }

    fn finish_hold(&mut self, hold: HostToken) {
        self.state.pending_writes.finish(hold);
    }
}
