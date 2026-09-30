//! The handle the editor gives Steel for an object it keeps on a script's
//! behalf.

use steel::rvals::SteelVal;

/// Opaque handle the editor returns to Steel for an object it keeps (an
/// open widget, a tracked position), scoping every later call to the
/// instance that minted it: a late callback naming an instance that is gone
/// names a token nothing holds, and reads as a no-op. Crosses the Steel
/// boundary as an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostToken(u64);

impl HostToken {
    /// Names nothing: what a `#f` token argument decodes to. The editor's
    /// mint starts at `1`, so no live object holds it.
    pub const NONE: Self = Self(0);

    /// Wraps a counter value the editor minted, or an integer Steel handed
    /// back. Any value is safe to wrap: a forged one matches at most a live
    /// object of the kind it is handed to, never a different kind.
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The token as the integer Steel sees.
    pub fn to_steel(self) -> SteelVal {
        SteelVal::IntV(self.0 as isize)
    }
}
