//! `EditorHostImpl`'s timer scheduling. Steel sees a timer as the
//! `TimerHandle` wrapping its `TimerId`.

use super::EditorHostImpl;
use crate::editor::timer_bridge::TimerPayload;
use crate::editor::timers::TimerId;
use hume_scripting::host::{TimerHandle, TimerHost};

impl<'a> TimerHost for EditorHostImpl<'a> {
    fn schedule_timer(&mut self, ms: u64, thunk: hume_scripting::Callback) -> Option<TimerHandle> {
        let id = self.timers.as_mut()?.schedule(
            std::time::Duration::from_millis(ms),
            TimerPayload::SteelThunk(thunk),
        );
        Some(TimerHandle::from_raw(id.0))
    }

    fn cancel_timer(&mut self, timer: TimerHandle) {
        if let Some(timers) = self.timers.as_mut() {
            timers.cancel_thunk(TimerId(timer.raw()));
        }
    }
}
