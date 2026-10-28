use hume_scripting::command_name::fold;
use rustc_hash::FxHashSet;

use crate::editor::registry::{CommandRegistry, MappableCommand, TypedCommand};

mod builder;
mod editor_cmds;
mod edits;
mod motions;
mod selections;
pub(in crate::editor) mod structural;
mod surround;
mod text_objects;
mod typed;

// Local macros to cut down on struct-literal boilerplate, shared by every
// family registrar below (invoked as `super::motion!(self, ...)` etc.). Each
// invoking file needs its own `use std::borrow::Cow;` and `MappableCommand`/`NativeCommand`
// imports: macro_rules resolves plain paths written in the macro body against
// the *invocation* site, not this definition site. The registry is taken as
// an explicit `$reg` argument rather than a bare `self` for the same reason:
// a literal `self` in the macro body has no receiver to bind to here. Each
// public arm forwards to its macro's `@build` arm by full path for the same
// reason: a bare `motion!` is not in scope at the invoking file.
macro_rules! motion {
    (@build $reg:expr, $name:literal, $doc:literal, $body:expr,
     jump: $jump:literal, aligns_view: $aligns:literal) => {
        $reg.register_builtin(MappableCommand::Native(NativeCommand::Motion {
            name: $name.into(),
            doc: Cow::Borrowed($doc),
            fun: crate::editor::commands::NativeBody::new($body),
            jump: $jump,
            aligns_view: $aligns,
        }))
    };
    // Forward object-jump family (`}`, `goto-next-<kind>`); see
    // `CmdMeta::aligns_view`'s doc for why only this direction opts in.
    ($reg:expr, $name:literal, $doc:literal, $fun:expr, jump, aligns_view) => {
        $crate::editor::registry::defaults::motion!(@build $reg, $name, $doc,
            SelectionBody::Plain($fun), jump: true, aligns_view: true)
    };
    ($reg:expr, $name:literal, $doc:literal, $fun:expr, jump) => {
        $crate::editor::registry::defaults::motion!(@build $reg, $name, $doc,
            SelectionBody::Plain($fun), jump: true, aligns_view: false)
    };
    // Word-family motions (`w`/`W`/`b`/`B`); see [`SelectionBody::Word`].
    ($reg:expr, $name:literal, $doc:literal, $fun:expr, word) => {
        $crate::editor::registry::defaults::motion!(@build $reg, $name, $doc,
            SelectionBody::Word($fun), jump: false, aligns_view: false)
    };
    // `goto-line-end`; see [`SelectionBody::LineEnd`].
    ($reg:expr, $name:literal, $doc:literal, $fun:expr, line_end) => {
        $crate::editor::registry::defaults::motion!(@build $reg, $name, $doc,
            SelectionBody::LineEnd($fun), jump: false, aligns_view: false)
    };
    ($reg:expr, $name:literal, $doc:literal, $fun:expr) => {
        $crate::editor::registry::defaults::motion!(@build $reg, $name, $doc,
            SelectionBody::Plain($fun), jump: false, aligns_view: false)
    };
}
macro_rules! selection {
    (@build $reg:expr, $name:literal, $doc:literal, $body:expr,
     jump: $jump:literal, tracking: $tracking:ident) => {
        $reg.register_builtin(MappableCommand::Native(NativeCommand::Selection {
            name: $name.into(),
            doc: Cow::Borrowed($doc),
            fun: crate::editor::commands::NativeBody::new($body),
            jump: $jump,
            selection_tracking: SelectionTracking::$tracking,
        }))
    };
    ($reg:expr, $name:literal, $doc:literal, $fun:expr) => {
        $crate::editor::registry::defaults::selection!(@build $reg, $name, $doc,
            SelectionBody::Plain($fun), jump: false, tracking: Establishes)
    };
    ($reg:expr, $name:literal, $doc:literal, $fun:expr, jump) => {
        $crate::editor::registry::defaults::selection!(@build $reg, $name, $doc,
            SelectionBody::Plain($fun), jump: true, tracking: Establishes)
    };
    // Transforms whatever extent is already staged instead of establishing
    // one; see `SelectionTracking::Composes`.
    ($reg:expr, $name:literal, $doc:literal, $fun:expr, composes) => {
        $crate::editor::registry::defaults::selection!(@build $reg, $name, $doc,
            SelectionBody::Plain($fun), jump: false, tracking: Composes)
    };
    // Word-family selections/text objects (`mm`/`MM`, `miw`/`maw`); see
    // [`SelectionBody::Word`].
    ($reg:expr, $name:literal, $doc:literal, $fun:expr, word) => {
        $crate::editor::registry::defaults::selection!(@build $reg, $name, $doc,
            SelectionBody::Word($fun), jump: false, tracking: Establishes)
    };
}
macro_rules! edit {
    ($reg:expr, $name:literal, $doc:literal, $fun:expr) => {
        $reg.register_builtin(MappableCommand::Native(NativeCommand::Edit {
            name: $name.into(),
            doc: Cow::Borrowed($doc),
            fun: crate::editor::commands::NativeBody::new($fun),
            repeatable: false,
        }))
    };
    ($reg:expr, $name:literal, $doc:literal, $fun:expr, repeatable) => {
        $reg.register_builtin(MappableCommand::Native(NativeCommand::Edit {
            name: $name.into(),
            doc: Cow::Borrowed($doc),
            fun: crate::editor::commands::NativeBody::new($fun),
            repeatable: true,
        }))
    };
}
// macro_rules items need an explicit `use` to be path-addressable
// (`super::motion!`) from a child module; bare declaration only gives
// textual scope within this module itself.
pub(super) use edit;
pub(super) use motion;
pub(super) use selection;

impl CommandRegistry {
    pub(super) fn register_defaults(&mut self) {
        self.register_motions();
        self.register_selections();
        self.register_text_objects();
        self.register_structural();
        self.register_surround();
        self.register_edits();
        self.register_editor_cmds();
        self.register_typed_commands();
    }

    fn register_builtin(&mut self, cmd: MappableCommand) {
        self.assert_unclaimed(cmd.name());
        self.register(cmd)
            .expect("a built-in editor command's name is lowercase");
    }

    /// # Panics
    /// Panics if the name or an alias is already registered, or if one of
    /// them repeats another.
    fn register_builtin_typed(&mut self, cmd: TypedCommand) {
        let mut own = FxHashSet::default();
        for name in std::iter::once(cmd.name.as_ref()).chain(cmd.aliases.iter().copied()) {
            self.assert_unclaimed(name);
            assert!(
                own.insert(fold(name)),
                "built-in command name {name:?} is listed twice in one command"
            );
        }
        self.register_typed(cmd)
            .expect("a built-in typed command's name and aliases are valid");
    }

    /// Registration replaces whatever a name already holds, so two built-ins
    /// sharing a name would leave only the last one registered.
    ///
    /// # Panics
    /// Panics if `name` is already a command's name or a typed alias.
    fn assert_unclaimed(&self, name: &str) {
        assert!(
            !self.contains(name) && self.get_typed(name).is_none(),
            "built-in command name {name:?} is already registered"
        );
    }
}

#[cfg(test)]
mod tests;
