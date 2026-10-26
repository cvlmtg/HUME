//! The Steel procs behind `define-command!` and `define-typed-command!`,
//! each carrying the owner it was defined under and the plugin body it was
//! defined in.
//!
//! The editor's `CommandRegistry` stores them on the entry that names the
//! command, so a command's name, kind, proc and owner live in one place.
//! Fields stay private: the editor holds and hands back a proc but can read
//! neither its value nor its owner.

use std::rc::Rc;

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;
use steel::steel_vm::engine::Engine;

use crate::Rooted;
use crate::attribution::{EntryId, Owner};

/// `owner` and `activation` are shared so that cloning a proc on every
/// dispatch copies no strings. A command has an author but no requester:
/// whoever invokes it is. `activation` is the plugin body that was running
/// when it was defined (`None`: none was), whose failure unregisters it.
#[derive(Clone, Debug)]
struct Defined {
    proc: Rooted,
    owner: Rc<Owner>,
    activation: Option<Rc<EntryId>>,
}

impl Defined {
    fn new(proc: Rooted, owner: Rc<Owner>, activation: Option<Rc<EntryId>>) -> Self {
        Self {
            proc,
            owner,
            activation,
        }
    }

    #[cfg(any(test, feature = "test-util"))]
    fn placeholder_in(plugin: EntryId) -> Self {
        Self::new(
            Rooted::new(SteelVal::Void),
            Rc::new(Owner::Plugin(plugin.clone())),
            Some(Rc::new(plugin)),
        )
    }
}

/// A `define-command!` proc. `call!` hands it to the VM as a value.
#[derive(Clone, Debug)]
pub struct MappableProc(Defined);

impl MappableProc {
    pub(crate) fn new(proc: Rooted, owner: Rc<Owner>, activation: Option<Rc<EntryId>>) -> Self {
        Self(Defined::new(proc, owner, activation))
    }

    /// A placeholder proc owned by the user, for a test that builds a
    /// registry entry it never calls.
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_test() -> Self {
        Self::new(Rooted::new(SteelVal::Void), Rc::new(Owner::User), None)
    }

    /// A placeholder proc that `plugin`'s entry file defined in its body.
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_test_registered_in(plugin: EntryId) -> Self {
        Self(Defined::placeholder_in(plugin))
    }

    /// The proc as a value `%dispatch-command!` applies inside the VM.
    pub(crate) fn steel_val(&self) -> SteelVal {
        self.0.proc.value().clone()
    }
}

/// A `define-typed-command!` proc. It has no `steel_val`: its value leaves
/// only through `CommandProc::call`, which takes the `Engine` a builtin never
/// holds, so `call!` (which resolves a name inside the VM) cannot reach a
/// `:`-only command.
#[derive(Clone, Debug)]
pub struct TypedProc(Defined);

impl TypedProc {
    pub(crate) fn new(proc: Rooted, owner: Rc<Owner>, activation: Option<Rc<EntryId>>) -> Self {
        Self(Defined::new(proc, owner, activation))
    }

    /// A placeholder proc owned by the user, for a test that builds a
    /// registry entry it never calls.
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_test() -> Self {
        Self::new(Rooted::new(SteelVal::Void), Rc::new(Owner::User), None)
    }

    /// A placeholder proc that `plugin`'s entry file defined in its body.
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_test_registered_in(plugin: EntryId) -> Self {
        Self(Defined::placeholder_in(plugin))
    }
}

#[derive(Clone, Debug)]
pub enum CommandProc {
    Mappable(MappableProc),
    Typed(TypedProc),
}

impl CommandProc {
    fn defined(&self) -> &Defined {
        let (Self::Mappable(MappableProc(defined)) | Self::Typed(TypedProc(defined))) = self;
        defined
    }

    /// Runs the proc from Rust, for the editor's key and `:` dispatch, which
    /// takes it from the registry entry it resolved.
    pub(crate) fn call(
        &self,
        steel: &mut Engine,
        args: Vec<SteelVal>,
    ) -> Result<SteelVal, SteelErr> {
        steel.call_function_with_args(self.defined().proc.value().clone(), args)
    }

    pub(crate) fn owner(&self) -> &Rc<Owner> {
        &self.defined().owner
    }

    /// Whether this command was defined while `activation`'s body ran, the
    /// filter a failed activation's rollback removes by.
    pub fn registered_in(&self, activation: &EntryId) -> bool {
        self.defined().activation.as_deref() == Some(activation)
    }
}
