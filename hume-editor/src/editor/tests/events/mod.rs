use super::*;
use crate::editor::commands::open_pane_in_layout;
use hume_platform::dirs::Dirs;
use hume_scripting::host::NativeCall;

fn trace_count(ed: &Editor, line: &str) -> usize {
    ed.state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace && e.text == line)
        .count()
}

mod buffer_enter_focus;
mod dispatch_hygiene;
mod mode_and_startup;
mod settle_queue;
mod text_changed;
mod text_changed_edge_cases;
