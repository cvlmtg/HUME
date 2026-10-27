//! `EditorHostImpl`'s timer scheduling.

use super::EditorHostImpl;
use crate::editor::timer_bridge::TimerPayload;
use hume_core::TimerId;
use hume_scripting::host::TimerHost;

impl<'a> TimerHost for EditorHostImpl<'a> {
    fn schedule_timer(&mut self, ms: u64, thunk: hume_scripting::Callback) -> Option<TimerId> {
        let id = self.timers.as_mut()?.schedule(
            std::time::Duration::from_millis(ms),
            TimerPayload::SteelThunk(thunk),
        );
        Some(id)
    }

    fn cancel_timer(&mut self, timer: TimerId) {
        if let Some(timers) = self.timers.as_mut() {
            timers.cancel_thunk(timer);
        }
    }
}
