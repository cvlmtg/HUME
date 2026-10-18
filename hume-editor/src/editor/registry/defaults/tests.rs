use std::borrow::Cow;

use crate::editor::commands::typed_quit;
use crate::editor::registry::{ArgExpansion, CommandRegistry, TypedBody, TypedCommand};

fn typed(name: &'static str, aliases: &'static [&'static str]) -> TypedCommand {
    TypedCommand {
        name: Cow::Borrowed(name),
        doc: Cow::Borrowed(""),
        aliases,
        body: TypedBody::Native(typed_quit),
        completer: None,
        arg_expansion: ArgExpansion::Paths,
    }
}

#[test]
#[should_panic(expected = "\"move-right\" is already registered")]
fn a_built_in_registered_twice_panics() {
    let mut reg = CommandRegistry::with_defaults();
    let cmd = reg
        .get_mappable("move-right")
        .cloned()
        .expect("move-right is a built-in");
    reg.register_builtin(cmd);
}

#[test]
#[should_panic(expected = "\"quit\" is already registered")]
fn a_typed_built_in_registered_twice_panics() {
    CommandRegistry::with_defaults().register_builtin_typed(typed("quit", &[]));
}

#[test]
#[should_panic(expected = "\"q\" is already registered")]
fn a_typed_built_in_alias_already_taken_panics() {
    CommandRegistry::with_defaults().register_builtin_typed(typed("quit-again", &["q"]));
}

#[test]
#[should_panic(expected = "\"move-right\" is already registered")]
fn a_typed_built_in_named_like_a_key_command_panics() {
    CommandRegistry::with_defaults().register_builtin_typed(typed("move-right", &[]));
}
