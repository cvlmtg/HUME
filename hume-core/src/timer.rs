//! Timer identity shared by the editor's timer wheel and the scripting host
//! that hands timers to Steel.

/// A scheduled timer: an opaque id only [`TimerIds`] mints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimerId(u64);

impl TimerId {
    /// The id its [`TimerIds`] minted.
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// Mints [`TimerId`]s for one timer wheel: each one no earlier call returned.
#[derive(Debug, Default)]
pub struct TimerIds(u64);

impl TimerIds {
    pub fn mint(&mut self) -> TimerId {
        let id = TimerId(self.0);
        self.0 += 1;
        id
    }
}
