use super::super::Editor;
use super::super::commands::typed_goto_line;
use super::super::input_stack::LayerRef;
use super::super::minibuf::MiniBufferEvent;
use super::super::minibuf::history::{HistoryDir, HistoryKind};
use super::super::registry::TypedBody;
use crate::editor::error::CommandError;

impl Editor {
    // ── Command mode ──────────────────────────────────────────────────────────

    pub(super) fn handle_command_event(&mut self, r: LayerRef, event: MiniBufferEvent) {
        match event {
            MiniBufferEvent::Cancel
            | MiniBufferEvent::ConfirmEmpty
            | MiniBufferEvent::BackspaceOnEmpty => {
                self.state.truncate_layers(&self.view, r);
            }
            MiniBufferEvent::Confirm(_) => {
                // If the selected completion candidate is a directory
                // (trailing `/`), Enter descends into it instead of executing:
                // the candidate is already in the input (Tab applied it), so
                // we just dismiss the popup and restart completion for the
                // directory's children.
                if self
                    .state
                    .input
                    .minibuf_completion()
                    .and_then(|s| s.candidates.get(s.selected))
                    .is_some_and(|c| c.replacement.ends_with('/'))
                {
                    if let Some(slot) = self.state.input.minibuf_completion_mut() {
                        *slot = None;
                    }
                    self.complete_minibuf(false);
                    return;
                }
                // Record into history and extract the input before
                // truncating — the `Command` layer (and the minibuf it
                // owns) is gone by the time `execute_command` runs, so the
                // string has to be taken out first, per truncate-before-
                // execute (§2.7): the body below runs with the mode layer
                // already back at `Base`, so a `:cmd` that enters Insert
                // stays in Insert instead of being stomped back to Normal,
                // and a body calling `(prompt! …)` pushes `Prompt` on a
                // clean stack with no special case needed.
                let raw = self
                    .state
                    .input
                    .minibuf()
                    .map(|mb| mb.input.clone())
                    .unwrap_or_default();
                self.state
                    .history
                    .get_mut(HistoryKind::Command)
                    .push(raw.clone());
                let input = raw.trim().to_owned();
                self.state.truncate_layers(&self.view, r);
                self.execute_command(&input);
            }
            // Any edit, cursor move, or Backspace that clears to empty dismisses the
            // completion popup and demotes any active history recall back to scratch.
            // EmptiedByBackspace keeps the minibuffer open (showing just the prompt)
            // so a second Backspace is needed to dismiss — avoids accidental closure
            // when the user deletes a one-char typo.
            MiniBufferEvent::EmptiedByBackspace
            | MiniBufferEvent::Edited
            | MiniBufferEvent::CursorMoved => {
                if let Some(slot) = self.state.input.minibuf_completion_mut() {
                    *slot = None;
                }
                self.state
                    .history
                    .get_mut(HistoryKind::Command)
                    .demote_to_scratch();
            }
            MiniBufferEvent::CompleteRequested { reverse } => {
                self.complete_minibuf(reverse);
            }
            MiniBufferEvent::HistoryPrev => {
                if let Some(slot) = self.state.input.minibuf_completion_mut() {
                    *slot = None;
                }
                self.recall_history(HistoryKind::Command, HistoryDir::Prev);
            }
            MiniBufferEvent::HistoryNext => {
                if let Some(slot) = self.state.input.minibuf_completion_mut() {
                    *slot = None;
                }
                self.recall_history(HistoryKind::Command, HistoryDir::Next);
            }
            MiniBufferEvent::Ignored => {}
        }
    }

    /// Routes a `Prompt` layer's key for a Steel `(prompt! …)` session
    /// rather than a `:` command line — no history, no completion, no
    /// directory-descend special case. Exactly one `(callback text-or-#f)`
    /// call fires, on Confirm or on any of the cancel paths.
    pub(super) fn handle_steel_prompt_event(&mut self, r: LayerRef, event: MiniBufferEvent) {
        match event {
            MiniBufferEvent::Cancel
            | MiniBufferEvent::ConfirmEmpty
            | MiniBufferEvent::BackspaceOnEmpty => self.finish_steel_prompt(r, None),
            MiniBufferEvent::Confirm(text) => self.finish_steel_prompt(r, Some(text)),
            // Plain editing (char typed/deleted, cursor moved) is already
            // applied by `MiniBuffer::handle_key` — nothing further to do.
            // Tab/Up/Down are no-ops here (no completion, no history for a
            // one-shot prompt).
            MiniBufferEvent::Edited
            | MiniBufferEvent::CursorMoved
            | MiniBufferEvent::EmptiedByBackspace
            | MiniBufferEvent::CompleteRequested { .. }
            | MiniBufferEvent::HistoryPrev
            | MiniBufferEvent::HistoryNext
            | MiniBufferEvent::Ignored => {}
        }
    }

    /// Queues exactly one `(callback text-or-#f)` call and truncates the
    /// `Prompt` layer (`r`) — the callback is cloned out (cheap: `SteelVal`
    /// is reference-counted) before truncating, since teardown never fires
    /// a Steel callback itself (D9).
    fn finish_steel_prompt(&mut self, r: LayerRef, text: Option<String>) {
        let Some(callback) = self.state.input.prompt_callback().cloned() else {
            return;
        };
        let arg = match text {
            Some(s) => steel::rvals::SteelVal::StringV(s.into()),
            None => steel::rvals::SteelVal::BoolV(false),
        };
        self.state.queue_steel_call(callback, vec![arg]);
        self.state.truncate_layers(&self.view, r);
    }

    /// Recall the previous (`Prev`) or next (`Next`) entry from `kind`'s history
    /// ring and install it in the minibuffer. No-op when there is no active
    /// minibuffer or when the ring has nowhere to go.
    pub(super) fn recall_history(&mut self, kind: HistoryKind, dir: HistoryDir) {
        let current = self
            .state
            .input
            .minibuf()
            .map(|m| m.input.as_str())
            .unwrap_or("");
        let text = match dir {
            HistoryDir::Prev => self.state.history.get_mut(kind).prev(current),
            HistoryDir::Next => self.state.history.get_mut(kind).next(),
        };
        if let Some(text) = text
            && let Some(mb) = self.state.input.minibuf_mut()
        {
            mb.input = text;
            mb.cursor = mb.input.len();
        }
    }

    /// Drive one Tab / Shift-Tab cycle in the completion popup.
    ///
    /// On the first Tab: queries the appropriate completer for the current
    /// minibuffer input.  If zero candidates → no-op.  If one → apply
    /// silently.  If two or more → open the popup and apply the first
    /// candidate.
    ///
    /// On subsequent Tab presses (state already Some): rotate `selected`
    /// forward (or backward when `reverse`) and apply the new candidate.
    fn complete_minibuf(&mut self, reverse: bool) {
        // If completion is already open, cycle to the next candidate.
        if let Some(slot) = self.state.input.minibuf_completion_mut()
            && let Some(completion) = slot.as_mut()
        {
            let n = completion.candidates.len();
            // current_span() reflects what's currently in the input (based on the
            // previously-selected candidate), so it must be read before advancing
            // completion.selected — after the update, candidates[selected] is the new one.
            let span = completion.current_span();
            completion.selected = if reverse {
                completion.selected.checked_sub(1).unwrap_or(n - 1)
            } else {
                (completion.selected + 1) % n
            };
            let replacement = completion.candidates[completion.selected]
                .replacement
                .clone();
            if let Some(mb) = self.state.input.minibuf_mut() {
                mb.input.replace_range(span.clone(), &replacement);
                mb.cursor = span.start + replacement.len();
            }
            return;
        }

        // Shift-Tab with no open popup is a no-op.
        if reverse {
            return;
        }

        // First Tab: extract input context without holding &mut self.state.input.
        let (input, cursor) = match self.state.input.minibuf() {
            Some(mb) => (mb.input.clone(), mb.cursor),
            None => return,
        };

        // Only complete command-mode minibuffers.
        if self.state.input.minibuf().map(|mb| mb.prompt.as_str()) != Some(":") {
            return;
        }

        let ctx = crate::editor::completion::CompletionCtx {
            registry: &self.state.config.registry,
            buffers: &self.state.buffers,
            cwd: &self.state.cwd,
            languages: &self.state.config.languages,
        };

        // Dispatch to the right completer based on command + input shape.
        use crate::editor::completion::{
            BufferNameCompleter, CommandCompleter, Completer, CompletionResult,
            MinibufCompletionState, PathCompleter, SetCompleter, ThemeCompleter,
        };
        use crate::editor::registry::ArgCompleter;

        let result: CompletionResult = {
            // Split input into (cmd_raw, arg) to determine the completer.
            match input.split_once(' ') {
                None => {
                    // No space yet — complete the command name.
                    CommandCompleter.complete(&input, cursor, &ctx)
                }
                Some((cmd_raw, _)) if cursor <= cmd_raw.len() => {
                    // Cursor is within the command name (user moved left past the
                    // space) — complete the command name, not the argument.
                    CommandCompleter.complete(&input, cursor, &ctx)
                }
                Some((cmd_raw, _)) => {
                    // Resolve alias → command, and its declared argument completer.
                    let cmd = cmd_raw.strip_suffix('!').unwrap_or(cmd_raw);
                    let completer = self
                        .state
                        .config
                        .registry
                        .get_typed(cmd)
                        .and_then(|tc| tc.completer.as_ref());
                    match completer {
                        Some(ArgCompleter::Path { dirs_only }) => PathCompleter {
                            dirs_only: *dirs_only,
                        }
                        .complete(&input, cursor, &ctx),
                        Some(ArgCompleter::Buffer) => {
                            BufferNameCompleter.complete(&input, cursor, &ctx)
                        }
                        Some(ArgCompleter::Theme) => ThemeCompleter.complete(&input, cursor, &ctx),
                        Some(ArgCompleter::Set) => SetCompleter.complete(&input, cursor, &ctx),
                        // No completer declared — e.g. `:bd` ignores its argument;
                        // skip completion to avoid a misleading
                        // pick-then-close-current-buffer UX.
                        None => return,
                    }
                }
            }
        };

        if result.candidates.is_empty() {
            return;
        }

        let span_start = result.span_start;
        let mut candidates = result.candidates;

        if candidates.len() == 1 {
            // Single match: apply silently without opening a popup.
            let replacement = candidates.remove(0).replacement;
            if let Some(mb) = self.state.input.minibuf_mut() {
                mb.input.replace_range(span_start..cursor, &replacement);
                mb.cursor = span_start + replacement.len();
            }
            return;
        }

        // Two or more: open popup with the first candidate selected.
        let replacement = candidates[0].replacement.clone();
        if let Some(mb) = self.state.input.minibuf_mut() {
            mb.input.replace_range(span_start..cursor, &replacement);
            mb.cursor = span_start + replacement.len();
        }
        let rows = hume_ui::popup::MenuRows::measure(std::sync::Arc::new(
            candidates.iter().map(|c| c.display.clone()).collect(),
        ));
        if let Some(slot) = self.state.input.minibuf_completion_mut() {
            *slot = Some(MinibufCompletionState {
                candidates,
                selected: 0,
                span_start,
                rows,
            });
        }
    }

    /// Execute a typed command line. `input` is the already-trimmed text
    /// the `Command` layer's minibuf held at Confirm — the layer (and its
    /// minibuf) is already gone by the time this runs, per truncate-
    /// before-execute (§2.7).
    fn execute_command(&mut self, input: &str) {
        let (cmd, force, arg) = parse_typed_command(input);

        // Bare line number `:42` — shorthand for `:goto 42`.
        // The parser leaves `cmd = ""` for digit-only input (digits are excluded
        // from the command-name alphabet so `:42` → cmd="" arg=Some("42")).
        if cmd.is_empty()
            && !force
            && arg.is_some_and(|a| !a.is_empty() && a.bytes().all(|b| b.is_ascii_digit()))
        {
            if let Err(e) = typed_goto_line(self, arg, false) {
                self.report(e.severity(), e.message().to_owned());
            }
            return;
        }

        // Expand `%`/`#` tokens in the arg. Gate on the fast-path check so the
        // common case (no expansion) stays allocation-free. Skip expansion for
        // `:b`/`:buffer`: their `#` is the alternate-buffer specifier itself,
        // not a filename token — expanding it to a path loses pathless
        // alternates (scratch, [messages], the [buffers] view from :ls).
        let needs_expansion = !matches!(cmd, "b" | "buffer");
        let expanded: Option<String> = match arg {
            Some(a) if needs_expansion && (a.contains('%') || a.contains('#')) => {
                match expand_command_arg(self, a) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        self.report(e.severity(), e.message().to_owned());
                        return;
                    }
                }
            }
            Some(a) => Some(a.to_owned()),
            None => None,
        };

        // `:` resolves only typed commands — an editor (key-bindable) command's
        // name is unreachable here; see `registry/mod.rs`'s module doc.
        match self.state.config.registry.get_typed(cmd) {
            Some(tc) => match &tc.body {
                TypedBody::Native(fun) => {
                    let fun = *fun;
                    if let Err(e) = fun(self, expanded.as_deref(), force) {
                        self.report(e.severity(), e.message().to_owned());
                    }
                }
                // Lazy-stub activation and Steel arg marshalling both happen
                // inside `Editor::run_typed_steel_command`, so `:bar arg`
                // cannot silently drop `arg` on the plugin's first call.
                TypedBody::Steel { .. } => {
                    self.run_typed_steel_command(cmd, None, expanded, force);
                }
                TypedBody::Lazy(plugin) => {
                    let plugin = plugin.clone();
                    self.run_typed_steel_command(cmd, Some(plugin), expanded, force);
                }
            },
            None => {
                self.report_unknown_command(cmd, format!("Unknown command: {cmd}"));
            }
        }
    }
}

// ── Typed-command helpers ─────────────────────────────────────────────────────

/// Parse a typed-command string into `(cmd, force, arg)`.
///
/// Command name = longest `[A-Za-z_-]` prefix. Digits are deliberately
/// excluded (Vim convention) so `:b1` parses as `cmd="b"` `arg="1"` — see
/// `:help :command-name`. One optional trailing `!` is consumed as
/// `force = true`. Everything after is the argument (whitespace-trimmed).
/// Matches Vim's ex-parser so `:b#`, `:e!/path`, `:list-buffers`, and
/// `:w foo.txt` all parse correctly.
pub(in super::super) fn parse_typed_command(input: &str) -> (&str, bool, Option<&str>) {
    let name_end = input
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_alphabetic() || *c == '-' || *c == '_'))
        .map(|(i, _)| i)
        .unwrap_or(input.len());
    let force = input[name_end..].starts_with('!');
    let cmd_end = name_end + usize::from(force);
    let cmd = &input[..name_end];
    let rest = input[cmd_end..].trim();
    let arg = if rest.is_empty() { None } else { Some(rest) };
    (cmd, force, arg)
}

/// Expand `%` → focused buffer's path and `#` → alternate buffer's path in a
/// typed-command argument.
///
/// Only whole tokens (separated by ASCII spaces) are substituted, so filenames
/// containing `%` or `#` as part of a longer word pass through unchanged.
/// Spacing is preserved; returns a user-facing error on the first unresolved token.
///
/// Expands to the canonical `path`, not `display_path`: the result feeds
/// straight back into path resolution, which doesn't `~`-expand.
fn expand_command_arg(ed: &Editor, arg: &str) -> Result<String, CommandError> {
    let mut out = String::with_capacity(arg.len());
    for (i, token) in arg.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        match token {
            "%" => {
                let path = ed
                    .doc()
                    .path()
                    .ok_or_else(|| CommandError::transient("No file name"))?;
                out.push_str(&path.display().to_string());
            }
            "#" => {
                let alt_id = ed
                    .alternate_buffer()
                    .ok_or_else(|| CommandError::transient("No alternate buffer"))?;
                let alt_path =
                    ed.state.buffers.get(alt_id).path().ok_or_else(|| {
                        CommandError::transient("Alternate buffer has no file name")
                    })?;
                out.push_str(&alt_path.display().to_string());
            }
            other => out.push_str(other),
        }
    }
    Ok(out)
}
