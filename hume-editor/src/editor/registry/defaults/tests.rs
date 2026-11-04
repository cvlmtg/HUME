use std::borrow::Cow;

use super::builder::ecmd_pane;
use crate::editor::commands::{cmd_undo_selection_change, typed_quit};
use crate::editor::registry::{
    ArgExpansion, CommandRegistry, NativeTyped, SelectionEffect, TypedBody, TypedCommand,
};

fn typed(name: &'static str, aliases: &'static [&'static str]) -> TypedCommand {
    TypedCommand {
        name: Cow::Borrowed(name),
        doc: Cow::Borrowed(""),
        aliases,
        body: TypedBody::Native(NativeTyped::Force(typed_quit)),
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

#[test]
#[should_panic(expected = "\"quit-again\" is listed twice")]
fn a_typed_built_in_alias_repeating_its_name_panics() {
    CommandRegistry::with_defaults().register_builtin_typed(typed("quit-again", &["quit-again"]));
}

#[test]
#[should_panic(expected = "\"QA2\" is listed twice")]
fn a_typed_built_in_alias_listed_twice_panics() {
    CommandRegistry::with_defaults().register_builtin_typed(typed("quit-again", &["qa2", "QA2"]));
}

#[test]
#[should_panic(expected = "\"two-roles\" declares a selection role twice")]
fn a_command_with_two_selection_roles_panics() {
    let _ = ecmd_pane("two-roles", "", cmd_undo_selection_change)
        .establishes_selection()
        .selection_effect(SelectionEffect::Composes);
}

#[test]
#[should_panic(expected = "\"extends-twice\" is extendable through its Extends effect")]
fn an_extends_command_marked_extendable_panics() {
    let mut reg = CommandRegistry::with_defaults();
    ecmd_pane("extends-twice", "", cmd_undo_selection_change)
        .extendable()
        .selection_effect(SelectionEffect::Extends)
        .reg(&mut reg);
}
