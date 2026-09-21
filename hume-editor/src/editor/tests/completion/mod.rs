//! Completion, end to end through the real seams: a Steel source registered
//! with `register-completion-source!`, invoked by `completion-trigger`
//! (Ctrl-Space) or a trigger char, answering with `completion-emit!`; the
//! `:` line's native sources on Tab. One helper set here, one file per
//! concern:
//!
//! - `sources.rs` — registration, invocation, token rules, stale answers,
//!   re-invocation, multi-source ranking.
//! - `menu_keys.rs` — the Insert-mode menu's key handling and lifetime.
//! - `accept.rs` — what accepting a candidate writes, single- and
//!   multi-cursor, with and without server edits.
//! - `minibuf.rs` — the `:` line's Tab completion.
//! - `render.rs` — the menu's geometry and the two render snapshots.
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

/// The ranked labels the open session would show, top 10.
fn labels(ed: &Editor) -> Vec<String> {
    ed.state
        .input
        .completion()
        .map(|s| {
            s.top(10, &ed.state.config.completion_sources)
                .iter()
                .map(|v| v["label"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The completion popup's selected row index — `0` when no `CompletionMenuUi`
/// has been allocated yet, matching `move_completion_selection`'s own
/// `map_or(0, ...)` convention (a session opens with `ui: None` until the
/// first Tab/Down/BackTab/Up moves the selection off its implicit default).
fn selected_row(ed: &Editor) -> usize {
    ed.state.input.completion_ui().map_or(0, |ui| ui.selected)
}

fn status(ed: &Editor) -> String {
    ed.state.status_msg.clone().unwrap_or_default()
}

/// Ctrl-Space, then settle so the queued Steel source answers.
fn trigger(ed: &mut Editor) {
    ed.feed_key(key_ctrl(' '));
    ed.settle();
}

/// Registers a `'word`-token source answering `items` (Scheme literals),
/// with a real `i` into Insert mode, then triggers it — the setup nearly
/// every Insert-mode test starts from.
fn insert_with_source(ed: &mut Editor, tmp: &std::path::Path, items: &str) {
    run(ed, tmp, &completion_source("test", items, ""));
    ed.feed_key(key('i'));
    trigger(ed);
}

/// [`insert_with_source`] on a raw `push_mode_layer(Insert)` rather than a
/// real `i` keypress — no edit group opened, no selection collapsed — for
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
