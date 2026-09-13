//! `RegisterHost` — moved out of `host_impl.rs`'s per-capability split.

use crate::editor::Severity;
use crate::editor::register_ops;

use super::EditorHostImpl;
use hume_scripting::host::RegisterHost;

impl<'a> RegisterHost for EditorHostImpl<'a> {
    fn read_register(&mut self, name: char) -> Option<Vec<String>> {
        match name {
            hume_ops::register::KILL_RING_REGISTER => {
                self.state.kill_ring.head().map(<[String]>::to_vec)
            }
            // Black hole and macro registers fall out for free:
            // `RegisterSet::read` already returns `None` for the black hole,
            // and `Register::as_text` already returns `None` for
            // `RegisterContent::Macro`, so both read as `#f` — same as empty.
            c => {
                let (values, warn) = register_ops::read_register_text(
                    &self.state.registers,
                    &mut self.state.clipboard,
                    c,
                );
                let values = values.map(|v| v.to_vec()); // end the &state.registers borrow
                if let Some(w) = warn {
                    self.state.report(Severity::Warning, w);
                }
                values
            }
        }
    }

    fn write_register(&mut self, name: char, values: Vec<String>) {
        match name {
            hume_ops::register::KILL_RING_REGISTER => self.state.capture_to_ring(values),
            _ => self.state.write_register(name, values),
        }
    }
}
