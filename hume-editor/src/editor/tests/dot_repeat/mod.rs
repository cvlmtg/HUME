use super::*;
use crate::editor::replay::{Answer, Input, Pick, Picked};
use crate::editor::settings::WordSelectsWhitespace;

/// `i` is the recorded unbound key `ch`.
fn is_key(i: &Input, ch: char) -> bool {
    matches!(i, Input::Key(k) if k.code == KeyCode::Char(ch))
}

/// `i` is the recorded binding of `name`.
fn is_binding(i: &Input, name: &str) -> bool {
    matches!(i, Input::Binding { name: n, .. } if &**n == name)
}

/// An editor over `initial_state` with the Steel commands `source` defines.
fn editor_with_steel(initial_state: &str, source: &str) -> Editor {
    let mut ed = editor_from(initial_state);
    attach_steel(&mut ed, source);
    ed
}

mod basics;
mod command_pickers;
mod insert_binding_replay;
mod outside_reads_and_dismissal;
mod reads_and_menus;
mod recipe_composition;
mod replay_failures_and_accepts;
mod selection_recipe;
mod steel_and_insert_keys;
