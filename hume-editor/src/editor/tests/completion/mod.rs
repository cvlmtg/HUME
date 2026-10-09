//! Completion, end to end through the real seams: a Steel source registered
//! with `register-completion-source!`, invoked by `completion-trigger`
//! (Ctrl-Space) or a trigger char, answering with `completion-emit!`; the
//! `:` line's native sources on Tab. One helper set here, one file per
//! concern:
//!
//! - `sources.rs`: registration, invocation, token rules, stale answers,
//!   re-invocation, multi-source ranking.
//! - `menu_keys.rs`: the Insert-mode menu's key handling and lifetime.
//! - `accept.rs`: what accepting a candidate writes, single- and
//!   multi-cursor, with and without server edits.
//! - `minibuf.rs`: the `:` line's Tab completion.
//! - `render.rs`: the menu's geometry and the two render snapshots.
//!
//! The real-server end-to-end flow (a fake `rust-analyzer` answering
//! `textDocument/completion`, resolve, `additionalTextEdits`) lives in
//! `tests/unix/lsp_completion_feature.rs`; the store's own unit tests in
//! `completion/session/tests.rs`.

use super::*;
use crate::editor::input_stack::InsertLayer;

mod accept;
mod menu_keys;
mod minibuf;
mod render;
mod sources;

/// The ranked labels the open Insert-mode session would show, top 10.
fn labels(ed: &Editor) -> Vec<String> {
    ed.state
        .input
        .buffer_completion()
        .map(|s| {
            s.top(10, &ed.state.config.completion_sources)
                .iter()
                .map(|v| v["label"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The completion popup's picked row, `None` while a `:` line popup has
/// nothing picked.
fn picked_row(ed: &Editor) -> Option<usize> {
    ed.state.input.completion_selected()
}

/// The completion popup's selected row index; panics when no row is picked.
fn selected_row(ed: &Editor) -> usize {
    picked_row(ed).expect("a row is picked")
}

/// Ctrl-Space, then settle so the queued Steel source answers.
fn trigger(ed: &mut Editor) {
    ed.feed_key(key_ctrl(' '));
    ed.settle();
}

/// `run`s `script`, enters Insert with a real `i`, triggers: the setup
/// every Insert-mode test starts from, for a test whose own registration
/// isn't [`insert_with_source`]'s default single-word-source shape.
fn insert_with_script(ed: &mut Editor, tmp: &std::path::Path, script: &str) {
    run(ed, tmp, script);
    ed.feed_key(key('i'));
    trigger(ed);
}

/// [`insert_with_script`] over a `'word`-token source answering `items`
/// (Scheme literals): the single-source case, and the more common of the
/// two.
fn insert_with_source(ed: &mut Editor, tmp: &std::path::Path, items: &str) {
    insert_with_script(ed, tmp, &completion_source("test", items, ""));
}

/// [`insert_with_source`] on a raw `push_mode_layer(Insert)` rather than a
/// real `i` keypress (no edit group opened, no selection collapsed), for
/// the tests pinning `accept`'s *own* group-opening/collapsed-selection
/// logic, the path a Steel-triggered accept outside Insert mode takes.
/// `script` runs beside the source registration (a `define-command!` the
/// test then dispatches).
fn raw_insert_with_source(ed: &mut Editor, tmp: &std::path::Path, items: &str, script: &str) {
    run(
        ed,
        tmp,
        &format!("{}\n{script}", completion_source("test", items, "")),
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("completion-trigger".into(), None, false);
    ed.settle();
}
