//! Whose code made a registration, and the one rule that ranks two writes to
//! the same slot.
//!
//! A store keeps a value per key (a key binding, an option, a language, a
//! server registration) and the [`ConfigOrigin`] of the write behind it. Every
//! later write is ranked against that origin by [`ConfigOrigin::rank`]; no
//! store compares origins itself. [`ConfigOrigin::claim`] and
//! [`Ranked::write`] record the winner, and [`ByOrigin`] keeps one value per
//! origin for a store whose lower layers stay visible when a higher one is
//! cleared.

/// Whose code made a registration, lowest first: the user's beats a plugin's,
/// and either beats HUME's bundled data, whichever arrives first.
///
/// An origin comes from the code that is running: `init.scm` and the procs it
/// defines are the user's, a plugin's own body is a plugin's, and a command or
/// hook runs as whoever it was started for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfigOrigin {
    /// HUME's runtime files: `languages.scm`, the per-language data files,
    /// and the rest of `runtime/scheme`.
    Bundled,
    Plugin,
    /// `init.scm`, and the procs it defines.
    User,
}

/// Which of two writes from one origin a store keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tie {
    /// Detection data and a configured field: a second write from the same
    /// origin is [`WriteOutcome::Ignored`].
    FirstWins,
    /// Everything else a script sets: a second write from the same origin
    /// replaces the first.
    LaterWins,
}

/// What a write did to the slot it was ranked against.
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteOutcome {
    Applied,
    /// The slot holds a value from the same origin and [`Tie::FirstWins`]
    /// keeps it.
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
    /// What a write from `self` does to a slot whose value `held` set (`None`:
    /// empty).
    pub fn rank(self, held: Option<Self>, tie: Tie) -> WriteOutcome {
        match held.map(|held| held.cmp(&self)) {
            None | Some(std::cmp::Ordering::Less) => WriteOutcome::Applied,
            Some(std::cmp::Ordering::Greater) => WriteOutcome::Shadowed,
            Some(std::cmp::Ordering::Equal) => match tie {
                Tie::FirstWins => WriteOutcome::Ignored,
                Tie::LaterWins => WriteOutcome::Applied,
            },
        }
    }

    /// [`Self::rank`] against the origin `held` records, which becomes `self`
    /// when the write applies. For a store whose value lives in a field of
    /// its own.
    pub fn claim(self, held: &mut Option<Self>, tie: Tie) -> WriteOutcome {
        let outcome = self.rank(*held, tie);
        if outcome.is_applied() {
            *held = Some(self);
        }
        outcome
    }
}

/// A value and the origin of the write that set it.
#[derive(Clone, Debug)]
pub struct Ranked<T> {
    pub value: T,
    pub by: ConfigOrigin,
}

impl<T> Ranked<T> {
    /// Stores `value` from `by` in `slot` unless the slot's origin outranks
    /// the write.
    pub fn write(slot: &mut Option<Self>, value: T, by: ConfigOrigin, tie: Tie) -> WriteOutcome {
        let outcome = by.rank(slot.as_ref().map(|held| held.by), tie);
        if outcome.is_applied() {
            *slot = Some(Self { value, by });
        }
        outcome
    }
}

/// One `T` per origin.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ByOrigin<T> {
    bundled: T,
    plugin: T,
    user: T,
}

impl<T> ByOrigin<T> {
    pub fn get_mut(&mut self, origin: ConfigOrigin) -> &mut T {
        match origin {
            ConfigOrigin::Bundled => &mut self.bundled,
            ConfigOrigin::Plugin => &mut self.plugin,
            ConfigOrigin::User => &mut self.user,
        }
    }

    /// Every origin's value, highest origin first.
    pub fn iter(&self) -> impl Iterator<Item = (ConfigOrigin, &T)> {
        [
            (ConfigOrigin::User, &self.user),
            (ConfigOrigin::Plugin, &self.plugin),
            (ConfigOrigin::Bundled, &self.bundled),
        ]
        .into_iter()
    }

    /// The values of every origin above `origin`, highest first.
    pub fn above(&self, origin: ConfigOrigin) -> impl Iterator<Item = &T> {
        self.iter()
            .take_while(move |(layer, _)| *layer > origin)
            .map(|(_, value)| value)
    }
}

#[cfg(test)]
mod tests;
