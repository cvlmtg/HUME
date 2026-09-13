//! `TimerHost` — moved out of `host_impl.rs`'s per-capability split.

use super::EditorHostImpl;
use hume_scripting::host::TimerHost;

impl<'a> TimerHost for EditorHostImpl<'a> {
    fn schedule_timer(&mut self, ms: u64, thunk: steel::rvals::SteelVal) -> Option<u64> {
        Some(
            self.timers
                .as_mut()?
                .schedule(std::time::Duration::from_millis(ms), thunk),
        )
    }

    fn cancel_timer(&mut self, id: u64) {
        if let Some(timers) = self.timers.as_mut() {
            timers.cancel(id);
        }
    }
}
