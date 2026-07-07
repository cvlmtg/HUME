//! [`EditorHostImpl`] — the production implementation of the scripting crate's
//! [`EditorHost`] trait.
//!
//! Holds disjoint borrows of `EditorState` and `EngineView`, which enables the
//! Steel VM (`scripting.steel`) to take `&mut Engine` simultaneously without
//! aliasing editor data. Two construction sites create this:
//!
//! - **Command dispatch** (`editor/mod.rs`, `run_steel_command`): called with the
//!   live editor state and view from the focused pane.
//! - **Init dispatch** (`scripting_setup.rs`): called with the same fields
//!   during `init_scripting`; init-only builtins set settings/keymap.

use std::path::{Path, PathBuf};

use crossterm::event::KeyEvent;
use hume_engine::pipeline::{BufferId, EngineView, PaneId};

use crate::editor::lsp::LspState;
use crate::editor::registry::MappableCommand;
use crate::editor::timer_bridge::TimerHandle;
use crate::settings::{BufferOverrides, SettingScope, apply_setting};
use crate::ui::statusline::{StatusElement, StatusLineConfig};
use hume_scripting::host::{BindMode, EditorHost};

use super::EditorState;

pub(crate) struct EditorHostImpl<'a> {
    pub(crate) state: &'a mut EditorState,
    pub(crate) view: &'a mut EngineView,
    /// `Some` only at the three call sites that can reach a B3 introspection
    /// builtin (command dispatch, hook fire, queued-call drain) — `None`
    /// everywhere else (init evals, which `require_cmd_ctx!` already blocks
    /// LSP builtins from anyway), so those sites don't need to thread it in.
    pub(crate) lsp: Option<&'a LspState>,
    /// Same `Some`-at-three-sites shape as `lsp`, for B4's `(after …)` /
    /// `(cancel-timer! …)` — these mutate (schedule/cancel), so `&LspState`'s
    /// shared-borrow shape doesn't fit; `TimerHandle` bundles the two
    /// `&mut` pieces this needs.
    pub(crate) timers: Option<TimerHandle<'a>>,
}

impl<'a> EditorHostImpl<'a> {
    /// Convenience constructor for the (common) case with no LSP/timer
    /// access — init evals, and every non-LSP/non-timer test in the suite.
    pub(crate) fn new(state: &'a mut EditorState, view: &'a mut EngineView) -> Self {
        Self {
            state,
            view,
            lsp: None,
            timers: None,
        }
    }

    /// Look up a buffer by id.
    fn buffer(&self, id: BufferId) -> Option<&crate::editor::buffer::Buffer> {
        self.state.buffers.try_get(id)
    }

    /// Seeded pane-buffer state for the focused (pane, buffer), or `None` when
    /// unseeded (stale or never-focused ids) — the shared guard behind every
    /// live cursor/selection read.
    fn focused_pane_buffer_state(&self) -> Option<&crate::editor::pane_state::PaneBufferState> {
        let buf_id = crate::editor::commands::focused_buffer_id(self.state, self.view);
        self.state
            .panes
            .state
            .get(self.state.focused_pane_id)?
            .get(buf_id)
    }
}

impl<'a> EditorHost for EditorHostImpl<'a> {
    // ── Enumeration ──────────────────────────────────────────────────────────
    fn buffer_ids(&self) -> Vec<BufferId> {
        self.state.buffers.iter().map(|(id, _)| id).collect()
    }
    fn pane_ids(&self) -> Vec<PaneId> {
        self.view.panes.iter().map(|(id, _)| id).collect()
    }

    // ── Buffer reads ─────────────────────────────────────────────────────────
    fn buffer_exists(&self, id: BufferId) -> bool {
        self.buffer(id).is_some()
    }
    fn buffer_path(&self, id: BufferId) -> Option<PathBuf> {
        self.buffer(id)?.path().map(Path::to_path_buf)
    }
    fn buffer_display_name(&self, id: BufferId) -> Option<String> {
        self.buffer(id).map(|buf| buf.display_name().to_owned())
    }
    fn buffer_is_dirty(&self, id: BufferId) -> Option<bool> {
        self.buffer(id).map(|buf| buf.is_dirty())
    }
    fn buffer_stored_language(&self, id: BufferId) -> Option<String> {
        self.buffer(id)?.language.clone()
    }

    // ── Buffer lifecycle ─────────────────────────────────────────────────────
    fn open_buffer(&mut self, path: &Path) -> Result<BufferId, String> {
        let canonical = hume_platform::fs::canonicalize(path)
            .map_err(|e| format!("open-buffer!: {}: {e}", path.display()))?;
        let (bid, _) = crate::editor::buffer::lifecycle::open_or_dedup(
            self.view,
            &mut self.state.buffers,
            &mut self.state.panes.state,
            self.state.focused_pane_id,
            &canonical,
        )
        .map_err(|e| format!("open-buffer!: {}: {e}", canonical.display()))?;
        Ok(bid)
    }
    fn close_buffer(&mut self, id: BufferId) -> Result<BufferId, String> {
        if self.state.buffers.try_get(id).is_none() {
            return Err(format!("close-buffer!: buffer {id:?} does not exist"));
        }
        Ok(crate::editor::buffer::lifecycle::close_buffer(
            self.view,
            &mut self.state.buffers,
            &mut self.state.panes.state,
            &mut self.state.panes.jumps,
            self.state.focused_pane_id,
            id,
        ))
    }
    fn switch_to_buffer(&mut self, current: BufferId, target: BufferId) -> Result<(), String> {
        crate::editor::buffer::lifecycle::switch_to_buffer_with_jump(
            self.view,
            &self.state.buffers,
            &mut self.state.panes.state,
            &mut self.state.panes.jumps,
            self.state.focused_pane_id,
            current,
            target,
        );
        Ok(())
    }

    // ── Settings ─────────────────────────────────────────────────────────────
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String> {
        let mut dummy = BufferOverrides::default();
        apply_setting(
            SettingScope::Global,
            key,
            value,
            &mut self.state.settings,
            &mut dummy,
        )
    }

    // ── Statusline ────────────────────────────────────────────────────────────
    fn configure_statusline(
        &mut self,
        left: Vec<String>,
        center: Vec<String>,
        right: Vec<String>,
    ) -> Result<(), String> {
        let parse = |list: Vec<String>, section: &str| -> Result<Vec<StatusElement>, String> {
            list.iter()
                .map(|s| {
                    s.parse::<StatusElement>()
                        .map_err(|e| format!("configure-statusline! {section}: {e}"))
                })
                .collect()
        };
        let left = parse(left, "left")?;
        let center = parse(center, "center")?;
        let right = parse(right, "right")?;
        self.state.settings.statusline = StatusLineConfig {
            left,
            center,
            right,
        };
        Ok(())
    }

    // ── Keymap ────────────────────────────────────────────────────────────────
    fn bind_key(
        &mut self,
        mode: BindMode,
        keys: &[KeyEvent],
        cmd: &str,
        force_extend: bool,
    ) -> Result<(), String> {
        self.state.keymap.bind_user_with_extend(
            to_editor_bind_mode(mode),
            keys,
            std::borrow::Cow::Owned(cmd.to_owned()),
            force_extend,
        );
        Ok(())
    }
    fn bind_wait_char(
        &mut self,
        mode: BindMode,
        keys: &[KeyEvent],
        cmd: &str,
    ) -> Result<(), String> {
        self.state.keymap.bind_wait_char_user(
            to_editor_bind_mode(mode),
            keys,
            std::borrow::Cow::Owned(cmd.to_owned()),
        );
        Ok(())
    }
    fn unbind_key(&mut self, mode: BindMode, keys: &[KeyEvent]) -> Result<(), String> {
        self.state
            .keymap
            .unbind_user(to_editor_bind_mode(mode), keys);
        Ok(())
    }

    // ── Language / grammar ────────────────────────────────────────────────────
    fn attach_grammar(
        &mut self,
        name: &str,
        grammar_path: &Path,
        symbol: &str,
        highlights_path: &Path,
        injections_path: Option<&Path>,
    ) -> Result<(), String> {
        self.state
            .languages
            .attach_grammar(
                name,
                grammar_path,
                symbol,
                highlights_path,
                injections_path,
                &mut self.view.registry,
            )
            .map_err(|e| format!("register-grammar! '{name}': {e}"))?;
        Ok(())
    }
    fn has_grammar(&self, language: &str) -> bool {
        self.state.languages.has_grammar(language)
    }

    // ── Command registration (init-only) ────────────────────────────────────
    fn register_command(&mut self, def: hume_scripting::SteelCmdDef) -> Result<(), String> {
        match self.state.registry.get_mappable(&def.name) {
            Some(MappableCommand::Lazy { .. }) | None => {
                self.state.registry.register(MappableCommand::SteelBacked {
                    name: def.name.into(),
                    doc: def.doc.into(),
                    arity: def.arity,
                    is_variadic: def.is_variadic,
                    inline_output: def.inline_output,
                    repeatable: def.repeatable,
                });
                Ok(())
            }
            Some(_) => Err(format!(
                "define-command!: '{}' conflicts with existing command",
                def.name
            )),
        }
    }

    fn unregister_command(&mut self, name: &str) {
        self.state.registry.unregister(name);
    }

    // ── Register validation ───────────────────────────────────────────────────
    fn is_valid_register_name(&self, ch: char) -> bool {
        crate::ops::register::is_valid_register_name(ch)
    }

    // ── Budget ────────────────────────────────────────────────────────────────
    fn steel_command_budget_ms(&self) -> u64 {
        self.state.settings.steel_command_budget_ms as u64
    }

    // ── Terminal safety ──────────────────────────────────────────────────────
    fn is_inline_output_command(&self) -> bool {
        self.state.dispatch_inline_output
    }

    // ── Synchronous command dispatch ─────────────────────────────────────────
    fn command_is_native(&self, name: &str) -> Result<bool, String> {
        self.state
            .registry
            .get_mappable(name)
            .map(MappableCommand::is_native)
            .ok_or_else(|| format!("unknown command: {name}"))
    }

    fn run_command_sync(
        &mut self,
        name: &str,
        count: Option<usize>,
        extend: bool,
        register: Option<char>,
    ) -> Result<(), String> {
        let Some(cmd) = self.state.registry.get_mappable(name).cloned() else {
            return Err(format!("unknown command: {name}"));
        };
        if !cmd.is_native() {
            return Err(format!(
                "{name} is not a native command — use call! instead of call-native!"
            ));
        }
        // Arm the register prefix so register-aware commands (yank, delete,
        // paste-after, …) route to the right destination.
        if let Some(r) = register {
            self.state.register_prefix = Some(crate::editor::RegisterPrefix::Selected(r));
        }
        // Delegate to the shared pipeline — all bookkeeping (paste session, jump
        // list, dot-repeat, last_command) lives there so the sync path is
        // identical to the keypress path.
        crate::editor::commands::run_dispatch_pipeline(
            self.state,
            self.view,
            cmd,
            crate::editor::CmdCtx {
                // `count` came from `parse_count_extend`, which decodes a
                // Steel-side count of 0 to `None` — the script's way of asking
                // for "as if no count was typed" (move-down/move-up read this
                // as visual-row movement instead of buffer-line movement).
                count,
                extend,
                steel_args: vec![],
            },
        );
        // Clear the prefix when we armed it, so it does not bleed into the
        // next interactive command.
        if register.is_some() {
            self.state.register_prefix = None;
        }
        Ok(())
    }

    // ── Live cursor read ─────────────────────────────────────────────────────
    fn current_line_number(&self) -> Option<usize> {
        let pbs = self.focused_pane_buffer_state()?;
        self.char_index_to_line(pbs.selections.primary().head())
    }

    fn current_selections(&self) -> Option<Vec<(usize, usize, bool)>> {
        let pbs = self.focused_pane_buffer_state()?;
        let primary_index = pbs.selections.primary_index();
        Some(
            pbs.selections
                .iter_sorted()
                .enumerate()
                .map(|(i, sel)| (sel.anchor(), sel.head(), i == primary_index))
                .collect(),
        )
    }

    fn char_index_to_line(&self, idx: usize) -> Option<usize> {
        let buf_id = crate::editor::commands::focused_buffer_id(self.state, self.view);
        let text = self.buffer(buf_id)?.text();
        if idx > text.len_chars() {
            return None;
        }
        Some(text.char_to_line(idx) + 1)
    }

    fn buffer_generation(&self, id: BufferId) -> Option<u64> {
        Some(self.buffer(id)?.text_gen)
    }

    // ── LSP introspection (B3) ────────────────────────────────────────────────
    fn lsp_capabilities(&self, server: Option<&str>) -> Option<serde_json::Value> {
        let lsp = self.lsp?;
        let bid = crate::editor::commands::focused_buffer_id(self.state, self.view);
        crate::editor::lsp::introspect::capabilities(self.state, lsp, bid, server)
    }

    fn lsp_server_status(&self) -> Vec<hume_scripting::LspServerStatusEntry> {
        self.lsp
            .map(crate::editor::lsp::introspect::server_status)
            .unwrap_or_default()
    }

    fn lsp_server_for_buffer(&self, id: BufferId) -> Option<String> {
        crate::editor::lsp::introspect::server_for_buffer(self.state, self.lsp?, id)
    }

    fn lsp_position_params(&self, id: BufferId) -> Option<serde_json::Value> {
        crate::editor::lsp::introspect::position_params(self.state, self.lsp?, id)
    }

    fn lsp_range_params(&self, id: BufferId) -> Option<serde_json::Value> {
        crate::editor::lsp::introspect::range_params(self.state, self.lsp?, id)
    }

    // ── Timers (B4) ──────────────────────────────────────────────────────────
    fn schedule_timer(&mut self, ms: u64, thunk: steel::rvals::SteelVal) -> Option<u64> {
        Some(
            self.timers
                .as_mut()?
                .schedule(std::time::Duration::from_millis(ms), thunk),
        )
    }

    fn cancel_timer(&mut self, id: u64) {
        if let Some(timers) = self.timers.as_mut() {
            timers.cancel(id);
        }
    }

    // ── Trigger chars (B7) ───────────────────────────────────────────────────
    fn register_trigger_chars(&mut self, source: String, chars: Vec<char>) {
        self.state.trigger_chars.insert(source, chars);
    }

    // ── Decoration stores (B5) ───────────────────────────────────────────────
    fn set_inlay_hints(&mut self, bid: BufferId, hints: Vec<(serde_json::Value, String, bool)>) {
        let Some(lsp) = self.lsp else {
            return;
        };
        let encoding = crate::editor::lsp::introspect::encoding_for_buffer(self.state, lsp, bid);
        let Some(rope) = self
            .state
            .buffers
            .try_get(bid)
            .map(|b| b.text().rope().clone())
        else {
            return;
        };
        let entries: Vec<crate::editor::decorations::InlayHintEntry> = hints
            .into_iter()
            .filter_map(|(wire_pos, text, before)| {
                let line = wire_pos.get("line")?.as_u64()? as usize;
                let character = wire_pos.get("character")?.as_u64()? as usize;
                let pos =
                    hume_editing::position_encoding::wire_to_char(&rope, line, character, encoding);
                Some(crate::editor::decorations::InlayHintEntry { pos, text, before })
            })
            .collect();
        self.state.decorations.set_inlay_hints(bid, entries);
    }

    fn set_signs(
        &mut self,
        source: String,
        bid: BufferId,
        signs: Vec<(usize, String, String, i64)>,
    ) {
        let entries = signs
            .into_iter()
            .map(
                |(line, text, scope, priority)| crate::editor::decorations::SignEntry {
                    line,
                    text,
                    scope,
                    priority,
                },
            )
            .collect();
        self.state.decorations.set_signs(source, bid, entries);
    }

    fn set_virtual_lines(&mut self, source: String, bid: BufferId, lines: Vec<(usize, String)>) {
        let entries = lines
            .into_iter()
            .map(|(line, text)| crate::editor::decorations::VirtualLineEntry { line, text })
            .collect();
        self.state
            .decorations
            .set_virtual_lines(source, bid, entries);
    }

    fn set_extra_highlights(
        &mut self,
        source: String,
        bid: BufferId,
        spans: Vec<(usize, usize, String)>,
    ) {
        let entries = spans
            .into_iter()
            .map(
                |(start, end, scope)| crate::editor::decorations::ExtraHighlightEntry {
                    start,
                    end,
                    scope,
                },
            )
            .collect();
        self.state
            .decorations
            .set_extra_highlights(source, bid, entries);
    }

    fn diagnostics_for_buffer(
        &self,
        bid: BufferId,
        severity_floor: Option<&str>,
        range: Option<(usize, usize)>,
    ) -> Vec<serde_json::Value> {
        let Some(lsp) = self.lsp else {
            return Vec::new();
        };
        crate::editor::lsp::introspect::diagnostics_for_buffer(
            self.state,
            lsp,
            bid,
            severity_floor,
            range,
        )
    }

    fn diagnostic_counts(&self, bid: BufferId) -> (usize, usize) {
        let Some(lsp) = self.lsp else {
            return (0, 0);
        };
        crate::editor::lsp::introspect::diagnostic_counts(lsp, bid)
    }

    // ── Edit + navigation primitives (B6) ────────────────────────────────────
    fn apply_text_edits(
        &mut self,
        bid: BufferId,
        edits: Vec<(usize, usize, usize, usize, String)>,
        expect_gen: Option<u64>,
    ) -> Result<(), String> {
        let Some(lsp) = self.lsp else {
            return Err("apply-text-edits!: no LSP state available".to_string());
        };
        let wire_edits = edits
            .into_iter()
            .map(|(start_line, start_char, end_line, end_char, new_text)| {
                crate::editor::lsp::edits::WireEdit {
                    start_line,
                    start_char,
                    end_line,
                    end_char,
                    new_text,
                }
            })
            .collect();
        crate::editor::lsp::edits::apply_text_edits(self.state, lsp, bid, wire_edits, expect_gen)
    }

    fn apply_workspace_edit(&mut self, edit: serde_json::Value) -> Result<usize, String> {
        let Some(lsp) = self.lsp else {
            return Err("apply-workspace-edit!: no LSP state available".to_string());
        };
        let we: lsp_types::WorkspaceEdit =
            serde_json::from_value(edit).map_err(|e| format!("malformed WorkspaceEdit: {e}"))?;
        let summary =
            crate::editor::lsp::edits::apply_workspace_edit(self.state, self.view, lsp, we)?;
        Ok(summary.buffers_modified)
    }

    fn goto_location_wire(
        &mut self,
        uri: String,
        line: usize,
        character: usize,
    ) -> Result<(), String> {
        let Some(lsp) = self.lsp else {
            return Err("goto-location!: no LSP state available".to_string());
        };
        let uri: lsp_types::Uri = uri
            .parse()
            .map_err(|_| format!("goto-location!: bad uri {uri:?}"))?;
        let target = crate::editor::lsp::edits::GotoTarget::Wire {
            uri,
            line,
            character,
        };
        crate::editor::lsp::edits::goto_location(self.state, self.view, lsp, target)
    }

    fn goto_location_path(
        &mut self,
        path_or_uri: String,
        line: usize,
        col: usize,
    ) -> Result<(), String> {
        let Some(lsp) = self.lsp else {
            return Err("goto-location!: no LSP state available".to_string());
        };
        let target = crate::editor::lsp::edits::GotoTarget::Path {
            path_or_uri,
            line,
            col,
        };
        crate::editor::lsp::edits::goto_location(self.state, self.view, lsp, target)
    }

    fn goto_location_buffer(
        &mut self,
        bid: BufferId,
        line: usize,
        col: usize,
    ) -> Result<(), String> {
        let Some(lsp) = self.lsp else {
            return Err("goto-location!: no LSP state available".to_string());
        };
        let target = crate::editor::lsp::edits::GotoTarget::Buffer { bid, line, col };
        crate::editor::lsp::edits::goto_location(self.state, self.view, lsp, target)
    }

    fn selection_spans_full_line(&self, bid: BufferId) -> bool {
        crate::editor::lsp::edits::selection_spans_full_line(self.state, bid)
    }

    // ── Minibuffer prompt (B9) ────────────────────────────────────────────────
    fn prompt(
        &mut self,
        label: String,
        prefill: String,
        callback: steel::rvals::SteelVal,
    ) -> Result<(), String> {
        // Not `self.state.minibuf.is_some()` — a `prompt!` called from a
        // `:command`'s body runs while that command line's own minibuffer
        // session is still open (it closes only after the command
        // returns). `steel_prompt_callback` is only `Some` once a *prior*
        // `prompt!` call has actually taken over the session.
        if self.state.steel_prompt_callback.is_some() {
            return Err("prompt!: a minibuffer session is already open".to_string());
        }
        let cursor = prefill.len();
        self.state.minibuf = Some(crate::editor::MiniBuffer {
            prompt: label,
            input: prefill,
            cursor,
        });
        self.state.steel_prompt_callback = Some(callback);
        self.state.history.begin_session_all();
        self.state.set_mode(crate::editor::Mode::Command);
        Ok(())
    }

    fn symbol_under_cursor(&self, bid: BufferId) -> String {
        let Some(buf) = self.state.buffers.try_get(bid) else {
            return String::new();
        };
        let pid = self.state.focused_pane_id;
        let Some(pbs) = self
            .state
            .panes
            .state
            .get(pid)
            .and_then(|by_buf| by_buf.get(bid))
        else {
            return String::new();
        };
        let text = buf.text();
        let head = pbs.selections.primary().head();
        let Some(ch) = text.char_at(head) else {
            return String::new();
        };
        if hume_editing::word::classify_char(ch) != hume_editing::word::CharClass::Word {
            return String::new();
        }
        let Some((start, end)) = crate::ops::text_object::inner_word_impl(
            text,
            head,
            hume_editing::word::is_word_boundary,
        ) else {
            return String::new();
        };
        text.slice(start..end + 1).to_string()
    }

    // ── Completion orchestration (B8) ────────────────────────────────────────
    fn completion_begin(
        &mut self,
        bid: BufferId,
        items: Vec<serde_json::Value>,
        incomplete: bool,
    ) -> Result<(), String> {
        if self.state.buffers.try_get(bid).is_none() {
            return Err("completion-begin!: no such buffer".to_string());
        }
        let session = crate::editor::lsp::completion::CompletionSession::begin(
            self.state, bid, &items, incomplete,
        );
        self.state.lsp_completion = Some(session);
        Ok(())
    }

    fn completion_update_filter(&mut self, text: String) -> Result<(), String> {
        let Some(mut session) = self.state.lsp_completion.take() else {
            return Err("completion-update-filter!: no active completion session".to_string());
        };
        session.update_filter(self.state, text);
        self.state.lsp_completion = Some(session);
        Ok(())
    }

    fn completion_top(&self, n: usize) -> Vec<serde_json::Value> {
        self.state
            .lsp_completion
            .as_ref()
            .map(|s| s.top(n))
            .unwrap_or_default()
    }

    fn completion_accept(&mut self, idx: usize) -> Result<(), String> {
        let Some(session) = self.state.lsp_completion.take() else {
            return Err("completion-accept!: no active completion session".to_string());
        };
        let Some(lsp) = self.lsp else {
            return Err("completion-accept!: no LSP state available".to_string());
        };
        // Ends the session either way — success or failure — so a rejected
        // accept never leaves a stale session lingering.
        session.accept(self.state, lsp, idx)
    }

    fn completion_dismiss(&mut self) {
        self.state.lsp_completion = None;
    }
}

/// Map scripting `BindMode` → editor `keymap::BindMode`.
pub(crate) fn to_editor_bind_mode(mode: BindMode) -> crate::editor::keymap::BindMode {
    match mode {
        BindMode::Normal => crate::editor::keymap::BindMode::Normal,
        BindMode::Extend => crate::editor::keymap::BindMode::Extend,
        BindMode::Insert => crate::editor::keymap::BindMode::Insert,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::Path;

    use hume_engine::pipeline::BufferId;
    use hume_scripting::host::EditorHost;

    use crate::editor::Editor;
    use crate::editor::scripting_setup::make_init_host;

    #[test]
    fn close_buffer_errs_when_id_unknown() {
        let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
            hume_editing::text::Text::empty(),
            hume_editing::selection::SelectionSet::default(),
        ));
        let mut host = make_init_host(&mut ed.state, &mut ed.view);
        // BufferId::default() is a zeroed key — not present in any live store.
        let err = host.close_buffer(BufferId::default()).unwrap_err();
        assert!(!err.is_empty(), "expected an error message");
    }

    #[test]
    fn switch_to_buffer_noop_when_same() {
        let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
            hume_editing::text::Text::empty(),
            hume_editing::selection::SelectionSet::default(),
        ));
        let bid = ed.focused_buffer_id();
        let mut host = make_init_host(&mut ed.state, &mut ed.view);
        // Switching to the same buffer should not error.
        host.switch_to_buffer(bid, bid).expect("same-buffer switch");
    }

    #[test]
    fn attach_grammar_errs_for_bad_path() {
        let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
            hume_editing::text::Text::empty(),
            hume_editing::selection::SelectionSet::default(),
        ));
        let mut host = make_init_host(&mut ed.state, &mut ed.view);
        let err = host
            .attach_grammar(
                "rust",
                Path::new("/no/such/lib.dylib"),
                "rust_language",
                Path::new("/no/such/highlights.scm"),
                None,
            )
            .unwrap_err();
        assert!(
            err.contains("register-grammar!"),
            "unexpected message: {err}"
        );
    }
}
