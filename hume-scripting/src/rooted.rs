//! A Steel value Rust keeps between evaluations.
//!
//! Steel's collector traces only its own stack, globals and the values
//! registered as roots. A value held in a Rust struct is invisible to it, so a
//! heap cell reachable only through that value (a captured `(box …)` or
//! `set!`-mutated variable) is freed by the next full collection: later reads
//! see whatever reused the slot, or panic once the slot is compacted away.

use std::rc::Rc;

use steel::RootedSteelVal;
use steel::rvals::SteelVal;

use crate::attribution::Provenance;

/// A Steel value that stays valid across collections for as long as Rust
/// holds it.
///
/// Mint one before any later Steel evaluation on this thread, and never in a
/// `static` or `thread_local`: the root lives in a thread-local that is gone
/// during thread teardown.
#[derive(Clone)]
pub struct Rooted(Repr);

#[derive(Clone)]
enum Repr {
    /// Holds nothing the collector manages, so it needs no root.
    Atom(SteelVal),
    /// The root lasts until the last clone drops.
    Root(Rc<RootedSteelVal>),
}

impl Rooted {
    pub fn new(value: SteelVal) -> Self {
        if is_atom(&value) {
            Self(Repr::Atom(value))
        } else {
            Self(Repr::Root(Rc::new(value.as_rooted())))
        }
    }

    pub fn value(&self) -> &SteelVal {
        match &self.0 {
            Repr::Atom(value) => value,
            Repr::Root(rooted) => rooted.value(),
        }
    }

    #[cfg(test)]
    fn is_rooted(&self) -> bool {
        !matches!(self.0, Repr::Atom(_))
    }
}

/// Variants that hold no `SteelVal` and no heap reference. Anything else,
/// including a variant added by a later steel-core, is rooted.
fn is_atom(value: &SteelVal) -> bool {
    matches!(
        value,
        SteelVal::BoolV(_)
            | SteelVal::NumV(_)
            | SteelVal::IntV(_)
            | SteelVal::Rational(_)
            | SteelVal::BigNum(_)
            | SteelVal::BigRational(_)
            | SteelVal::CharV(_)
            | SteelVal::Void
            | SteelVal::StringV(_)
            | SteelVal::SymbolV(_)
            | SteelVal::ByteVector(_)
            | SteelVal::FuncV(_)
            | SteelVal::MutFunc(_)
            | SteelVal::BuiltIn(_)
    )
}

impl std::fmt::Debug for Rooted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Rooted").field(self.value()).finish()
    }
}

/// A Steel procedure a script handed HUME to call later, credited as the
/// code that handed it over was (its `Provenance`): the call runs as that
/// code's author, so `(plugin-config)` reads that plugin's, and on the same
/// behalf, so a registration it makes ranks as one the handing code made.
#[derive(Clone, Debug)]
pub struct Callback {
    proc: Rooted,
    provenance: Provenance,
}

impl Callback {
    pub(crate) fn new(proc: Rooted, provenance: Provenance) -> Self {
        Self { proc, provenance }
    }

    /// `proc`, handed over by the code `ctx` is running.
    pub(crate) fn capture(ctx: &crate::SteelCtx, proc: Rooted) -> Self {
        Self::new(proc, ctx.provenance())
    }

    /// `proc`, as the user's own, for a test that builds a callback no
    /// script handed over.
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_test(proc: SteelVal) -> Self {
        Self::new(
            Rooted::new(proc),
            Provenance::own(crate::attribution::Owner::User),
        )
    }

    pub fn proc(&self) -> &Rooted {
        &self.proc
    }

    pub(crate) fn provenance(&self) -> &Provenance {
        &self.provenance
    }

    /// A call of `proc` credited as this callback, for a helper that calls
    /// this callback's proc in turn (`%call-held-handler`).
    pub(crate) fn wrapped_by(&self, proc: Rooted) -> Self {
        Self::new(proc, self.provenance.clone())
    }
}

/// A [`Callback`] and the arguments a later session calls it with.
/// Clones share one argument list, as every handler of one hook does.
#[derive(Clone, Debug)]
pub struct SteelCall {
    callback: Callback,
    args: Rc<[Rooted]>,
}

impl SteelCall {
    /// Roots each of `args`.
    pub fn new(callback: Callback, args: Vec<SteelVal>) -> Self {
        Self::with_args(callback, args.into_iter().map(Rooted::new).collect())
    }

    pub fn with_args(callback: Callback, args: Rc<[Rooted]>) -> Self {
        Self { callback, args }
    }

    pub fn proc(&self) -> &Rooted {
        self.callback.proc()
    }

    pub(crate) fn provenance(&self) -> &Provenance {
        self.callback.provenance()
    }

    pub fn args(&self) -> &[Rooted] {
        &self.args
    }
}

#[cfg(test)]
mod tests;
