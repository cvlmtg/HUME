//! The Steel procs behind `define-command!` and `define-typed-command!`,
//! each carrying the owner it was defined under.
//!
//! The editor's `CommandRegistry` stores them on the entry that names the
//! command, so a command's name, kind, proc and owner live in one place.
//! Fields stay private: the editor holds and hands back a proc but can read
//! neither its value nor its owner.

use std::rc::Rc;

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;
use steel::steel_vm::engine::Engine;

use crate::attribution::{EntryId, Owner};

/// `owner` is shared so that cloning a proc on every dispatch copies no
/// strings.
#[derive(Clone, Debug)]
struct Defined {
    proc: SteelVal,
    owner: Rc<Owner>,
}

/// A `define-command!` proc. `call!` hands it to the VM as a value.
#[derive(Clone, Debug)]
pub struct MappableProc(Defined);

impl MappableProc {
    pub(crate) fn new(proc: SteelVal, owner: Owner) -> Self {
        Self(Defined {
            proc,
            owner: Rc::new(owner),
        })
    }

    /// A placeholder proc owned by the user, for a test that builds a
    /// registry entry it never calls.
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_test() -> Self {
        Self::new(SteelVal::Void, Owner::User)
    }

    /// A placeholder proc that `plugin`'s entry file defined.
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_test_defined_by(plugin: EntryId) -> Self {
        Self::new(SteelVal::Void, Owner::Plugin(plugin))
    }

    /// The proc as a value `%dispatch-command!` applies inside the VM.
    pub(crate) fn steel_val(&self) -> SteelVal {
        self.0.proc.clone()
    }
}

/// A `define-typed-command!` proc. It has no `steel_val`: its value leaves
/// only through `CommandProc::call`, which takes the `Engine` a builtin never
/// holds, so `call!` (which resolves a name inside the VM) cannot reach a
/// `:`-only command.
#[derive(Clone, Debug)]
pub struct TypedProc(Defined);

impl TypedProc {
    pub(crate) fn new(proc: SteelVal, owner: Owner) -> Self {
        Self(Defined {
            proc,
            owner: Rc::new(owner),
        })
    }

    /// A placeholder proc owned by the user, for a test that builds a
    /// registry entry it never calls.
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_test() -> Self {
        Self::new(SteelVal::Void, Owner::User)
    }

    /// A placeholder proc that `plugin`'s entry file defined.
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_test_defined_by(plugin: EntryId) -> Self {
        Self::new(SteelVal::Void, Owner::Plugin(plugin))
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
        steel.call_function_with_args(self.defined().proc.clone(), args)
    }

    pub(crate) fn owner(&self) -> &Owner {
        &self.defined().owner
    }

    /// Whether `plugin`'s entry file defined this command, the filter a
    /// failed activation's rollback removes by.
    pub fn defined_by(&self, plugin: &EntryId) -> bool {
        matches!(&*self.defined().owner, Owner::Plugin(id) if id == plugin)
    }
}
