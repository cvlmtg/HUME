//! `EditorHostImpl`'s timer scheduling. Steel sees a timer as the plain
//! integer inside its `TimerId`.

use super::EditorHostImpl;
use crate::editor::timer_bridge::TimerPayload;
use crate::editor::timers::TimerId;
use hume_scripting::host::TimerHost;

impl<'a> TimerHost for EditorHostImpl<'a> {
    fn schedule_timer(&mut self, ms: u64, thunk: hume_scripting::Rooted) -> Option<u64> {
        let id = self.timers.as_mut()?.schedule(
            std::time::Duration::from_millis(ms),
            TimerPayload::SteelThunk(thunk),
        );
        Some(id.0)
    }

    fn cancel_timer(&mut self, id: u64) {
        if let Some(timers) = self.timers.as_mut() {
            timers.cancel_thunk(TimerId(id));
        }
    }
}
