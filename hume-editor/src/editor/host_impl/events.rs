//! `EditorHostImpl`'s event-name introspection.

use super::EditorHostImpl;
use hume_scripting::host::EventHost;

impl<'a> EventHost for EditorHostImpl<'a> {
    fn known_event_names(&self) -> &'static [&'static str] {
        crate::editor::event::known_event_names()
    }
}
