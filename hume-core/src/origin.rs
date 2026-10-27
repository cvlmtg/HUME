//! Whose code made a registration, and the one rule that ranks two writes to
//! the same slot.
//!
//! A store keeps a value per key (a key binding, an option, a language, a
//! server registration) in a [`Ranked`] slot, or the origin alone in a
//! [`Claim`] when the value lives elsewhere. The slot's type carries its
//! [`TiePolicy`], and every write goes through [`Ranked::write`],
//! [`Ranked::clear`] or [`Claim::claim`], which rank it against the held
//! origin. [`ConfigOrigin`] has no ordering, so no store compares origins
//! itself. [`ByOrigin`] keeps one value per origin for a store whose lower
//! layers stay visible when a higher one is cleared.

use std::cmp::Ordering;
use std::fmt;
use std::marker::PhantomData;

/// Whose code made a registration, lowest first: the user's beats a plugin's,
/// and either beats HUME's bundled data, whichever arrives first.
///
/// An origin comes from the code that is running: `init.scm` and the procs it
/// defines are the user's, a plugin's own body is a plugin's, and a command or
/// hook runs as whoever it was started for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigOrigin {
    /// HUME's runtime files: `languages.scm`, the per-language data files,
    /// and the rest of `runtime/scheme`.
    Bundled,
    Plugin,
    /// `init.scm`, and the procs it defines.
    User,
}

/// What a write did to the slot it was ranked against.
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteOutcome {
    Applied,
    /// The slot holds a value from the same origin and its policy is
    /// [`FirstWins`].
    Ignored,
    /// The slot holds a value from a higher origin.
    Shadowed,
}

impl WriteOutcome {
    pub fn is_applied(self) -> bool {
        self == Self::Applied
    }
}

impl ConfigOrigin {
    /// Every origin, lowest first: the declaration order, which
    /// [`Self::level`] numbers.
    const LOWEST_FIRST: [Self; 3] = [Self::Bundled, Self::Plugin, Self::User];

    const fn level(self) -> u8 {
        self as u8
    }
}

// `LOWEST_FIRST` is the declaration order, ending at `User`.
const _: () = {
    let mut level = 0;
    while level < ConfigOrigin::LOWEST_FIRST.len() {
        assert!(ConfigOrigin::LOWEST_FIRST[level] as usize == level);
        level += 1;
    }
    assert!(ConfigOrigin::LOWEST_FIRST.len() == ConfigOrigin::User as usize + 1);
};

mod sealed {
    pub trait Sealed {}
}

/// Which of two writes from one origin a slot keeps.
pub trait TiePolicy: sealed::Sealed {
    /// What a write does to a slot whose value the same origin set.
    const SAME_ORIGIN: WriteOutcome;
}

/// Detection data and a configured field: a second write from the same
/// origin is ignored.
pub struct FirstWins;

/// Everything else a script sets: a second write from the same origin
/// replaces the first.
pub struct LaterWins;

impl sealed::Sealed for FirstWins {}
impl sealed::Sealed for LaterWins {}

impl TiePolicy for FirstWins {
    const SAME_ORIGIN: WriteOutcome = WriteOutcome::Ignored;
}

impl TiePolicy for LaterWins {
    const SAME_ORIGIN: WriteOutcome = WriteOutcome::Applied;
}

/// A slot: the value it holds, if any, and the origin of the write that set
/// it, ranked under the tie policy `P`.
///
/// A write to an empty slot always applies, so
/// `map.entry(key).or_default().write(..)` never leaves an empty entry.
pub struct Ranked<T, P> {
    held: Option<(T, ConfigOrigin)>,
    tie: PhantomData<P>,
}

/// The origin of a value the store keeps elsewhere.
pub type Claim<P> = Ranked<(), P>;

/// What a write or a clear did, and the value it took out of the slot.
#[must_use]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written<T> {
    pub outcome: WriteOutcome,
    /// The value the slot held before an applied write; `None` when the
    /// write did not apply or the slot was empty.
    pub displaced: Option<T>,
}

impl<T, P: TiePolicy> Ranked<T, P> {
    /// What a write from `by` would do to this slot.
    pub fn rank(&self, by: ConfigOrigin) -> WriteOutcome {
        let Some((_, held)) = &self.held else {
            return WriteOutcome::Applied;
        };
        match held.level().cmp(&by.level()) {
            Ordering::Less => WriteOutcome::Applied,
            Ordering::Greater => WriteOutcome::Shadowed,
            Ordering::Equal => P::SAME_ORIGIN,
        }
    }

    /// Stores `value` from `by` unless the slot's origin outranks the write.
    pub fn write(&mut self, value: T, by: ConfigOrigin) -> Written<T> {
        let outcome = self.rank(by);
        let displaced = match outcome {
            WriteOutcome::Applied => self.held.replace((value, by)).map(|(old, _)| old),
            WriteOutcome::Ignored | WriteOutcome::Shadowed => None,
        };
        Written { outcome, displaced }
    }

    /// Empties the slot unless its origin outranks `by`.
    pub fn clear(&mut self, by: ConfigOrigin) -> Written<T> {
        let outcome = self.rank(by);
        let displaced = match outcome {
            WriteOutcome::Applied => self.take(),
            WriteOutcome::Ignored | WriteOutcome::Shadowed => None,
        };
        Written { outcome, displaced }
    }

    pub fn get(&self) -> Option<&T> {
        self.held.as_ref().map(|(value, _)| value)
    }

    /// The origin of the write that set the held value.
    pub fn by(&self) -> Option<ConfigOrigin> {
        self.held.as_ref().map(|&(_, by)| by)
    }

    /// Empties the slot whatever its origin: a reset, not a ranked write.
    pub fn take(&mut self) -> Option<T> {
        self.held.take().map(|(value, _)| value)
    }

    /// This slot's origin, without its value.
    pub fn as_claim(&self) -> Claim<P> {
        Ranked {
            held: self.by().map(|by| ((), by)),
            tie: PhantomData,
        }
    }
}

impl<P: TiePolicy> Claim<P> {
    /// Records `by` as the slot's origin when a write from it applies.
    pub fn claim(&mut self, by: ConfigOrigin) -> WriteOutcome {
        self.write((), by).outcome
    }
}

impl<T, P> Default for Ranked<T, P> {
    fn default() -> Self {
        Self {
            held: None,
            tie: PhantomData,
        }
    }
}

impl<T: Clone, P> Clone for Ranked<T, P> {
    fn clone(&self) -> Self {
        Self {
            held: self.held.clone(),
            tie: PhantomData,
        }
    }
}

impl<T: Copy, P> Copy for Ranked<T, P> {}

impl<T: fmt::Debug, P> fmt::Debug for Ranked<T, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ranked").field("held", &self.held).finish()
    }
}

impl<T: PartialEq, P> PartialEq for Ranked<T, P> {
    fn eq(&self, other: &Self) -> bool {
        self.held == other.held
    }
}

impl<T: Eq, P> Eq for Ranked<T, P> {}

/// One `T` per origin, indexed by its level.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ByOrigin<T>([T; ConfigOrigin::LOWEST_FIRST.len()]);

impl<T> ByOrigin<T> {
    pub fn get_mut(&mut self, origin: ConfigOrigin) -> &mut T {
        &mut self.0[usize::from(origin.level())]
    }

    /// Every origin's value, highest origin first.
    pub fn iter(&self) -> impl Iterator<Item = (ConfigOrigin, &T)> {
        ConfigOrigin::LOWEST_FIRST
            .into_iter()
            .rev()
            .map(|origin| (origin, &self.0[usize::from(origin.level())]))
    }

    /// The values of every origin above `origin`, highest first.
    pub fn above(&self, origin: ConfigOrigin) -> impl Iterator<Item = &T> {
        self.iter()
            .take_while(move |(layer, _)| *layer != origin)
            .map(|(_, value)| value)
    }
}

#[cfg(test)]
mod tests;
