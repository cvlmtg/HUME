use crate::editor::registry::CommandRegistry;

mod builder;
mod editor_cmds;
mod edits;
mod motions;
mod selections;
mod surround;
mod text_objects;
mod typed;

// Local macros to cut down on struct-literal boilerplate — shared by every
// family registrar below (invoked as `super::motion!(self, ...)` etc.). Each
// invoking file needs its own `use std::borrow::Cow;` and `MappableCommand`
// import — macro_rules resolves plain paths written in the macro body against
// the *invocation* site, not this definition site. The registry is taken as
// an explicit `$reg` argument rather than a bare `self` for the same reason:
// a literal `self` in the macro body has no receiver to bind to here.
macro_rules! motion {
    ($reg:expr, $name:literal, $doc:literal, $fun:expr, jump) => {
        $reg.register(MappableCommand::Motion {
            name: Cow::Borrowed($name),
            doc: Cow::Borrowed($doc),
            fun: $fun,
            around_fun: None,
            jump: true,
            reaching: false,
        })
    };
    ($reg:expr, $name:literal, $doc:literal, $fun:expr, reaching) => {
        $reg.register(MappableCommand::Motion {
            name: Cow::Borrowed($name),
            doc: Cow::Borrowed($doc),
            fun: $fun,
            around_fun: None,
            jump: false,
            reaching: true,
        })
    };
    // Word motions: `around_fun` swaps in for `fun` when
    // `word-selects-whitespace` is on (see `run_native_body`). All
    // four are `reaching` — see the plain `reaching` arm above.
    ($reg:expr, $name:literal, $doc:literal, $fun:expr, $around_fun:expr, reaching) => {
        $reg.register(MappableCommand::Motion {
            name: Cow::Borrowed($name),
            doc: Cow::Borrowed($doc),
            fun: $fun,
            around_fun: Some($around_fun),
            jump: false,
            reaching: true,
        })
    };
    ($reg:expr, $name:literal, $doc:literal, $fun:expr) => {
        $reg.register(MappableCommand::Motion {
            name: Cow::Borrowed($name),
            doc: Cow::Borrowed($doc),
            fun: $fun,
            around_fun: None,
            jump: false,
            reaching: false,
        })
    };
}
macro_rules! selection {
    ($reg:expr, $name:literal, $doc:literal, $fun:expr) => {
        $reg.register(MappableCommand::Selection {
            name: Cow::Borrowed($name),
            doc: Cow::Borrowed($doc),
            fun: $fun,
            around_fun: None,
            jump: false,
        })
    };
    ($reg:expr, $name:literal, $doc:literal, $fun:expr, jump) => {
        $reg.register(MappableCommand::Selection {
            name: Cow::Borrowed($name),
            doc: Cow::Borrowed($doc),
            fun: $fun,
            around_fun: None,
            jump: true,
        })
    };
    // `mm`/`MM` (select-word/select-uppercase-word): `around_fun` swaps in
    // for `fun` when `word-selects-whitespace` is on. Tried last — the
    // `jump` arm above must win when the 4th argument is literally the
    // identifier `jump`, since a bare identifier also parses as a valid
    // `expr` and macro_rules does not backtrack across arms once one
    // matches.
    ($reg:expr, $name:literal, $doc:literal, $fun:expr, $around_fun:expr) => {
        $reg.register(MappableCommand::Selection {
            name: Cow::Borrowed($name),
            doc: Cow::Borrowed($doc),
            fun: $fun,
            around_fun: Some($around_fun),
            jump: false,
        })
    };
}
macro_rules! edit {
    ($reg:expr, $name:literal, $doc:literal, $fun:expr) => {
        $reg.register(MappableCommand::Edit {
            name: Cow::Borrowed($name),
            doc: Cow::Borrowed($doc),
            fun: $fun,
            repeatable: false,
        })
    };
    ($reg:expr, $name:literal, $doc:literal, $fun:expr, repeatable) => {
        $reg.register(MappableCommand::Edit {
            name: Cow::Borrowed($name),
            doc: Cow::Borrowed($doc),
            fun: $fun,
            repeatable: true,
        })
    };
}
// macro_rules items need an explicit `use` to be path-addressable
// (`super::motion!`) from a child module — bare declaration only gives
// textual scope within this module itself.
pub(super) use edit;
pub(super) use motion;
pub(super) use selection;

impl CommandRegistry {
    pub(super) fn register_defaults(&mut self) {
        self.register_motions();
        self.register_selections();
        self.register_text_objects();
        self.register_surround();
        self.register_edits();
        self.register_editor_cmds();
        self.register_typed_commands();
    }
}
