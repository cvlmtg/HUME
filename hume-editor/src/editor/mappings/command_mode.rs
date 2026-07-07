use crossterm::event::KeyEvent;

use super::super::commands::typed_goto_line;
use super::super::minibuf::MiniBufferEvent;
use super::super::minibuf::history::{HistoryDir, HistoryKind};
use super::super::registry::MappableCommand;
use super::super::{Editor, Mode, Severity};
use crate::editor::error::CommandError;

impl Editor {
    // ── Command mode ──────────────────────────────────────────────────────────

    pub(super) fn handle_command(&mut self, key: KeyEvent) {
        let event = match self.state.minibuf.as_mut() {
            Some(mb) => mb.handle_key(key),
            None => return,
        };
        if self.state.steel_prompt_callback.is_some() {
            self.handle_steel_prompt_event(event);
            return;
        }
        match event {
            MiniBufferEvent::Cancel => {
                self.set_mode(Mode::Normal);
                self.close_minibuf();
            }
            // Empty Enter: dismiss silently without dispatching.
            MiniBufferEvent::ConfirmEmpty => {
                self.set_mode(Mode::Normal);
                self.close_minibuf();
            }
            MiniBufferEvent::Confirm(_) => {
                // If the selected completion candidate is a directory
                // (trailing `/`), Enter descends into it instead of executing:
                // the candidate is already in the input (Tab applied it), so
                // we just dismiss the popup and restart completion for the
                // directory's children.
                if self
                    .state
                    .completion
                    .as_ref()
                    .and_then(|s| s.candidates.get(s.selected))
                    .is_some_and(|c| c.replacement.ends_with('/'))
                {
                    self.state.completion = None;
                    self.complete_minibuf(false);
                    return;
                }
                // Record before dispatch so failed/unknown commands are recallable.
                if let Some(mb) = self.state.minibuf.as_ref() {
                    let raw = mb.input.clone();
                    self.state.history.get_mut(HistoryKind::Command).push(raw);
                }
                self.execute_command();
                // A `:command` whose body calls `(prompt! …)` (B9) leaves a
                // new minibuffer session open — closing it here would stomp
                // that session before the user ever sees it.
                if self.state.steel_prompt_callback.is_none() {
                    self.set_mode(Mode::Normal);
                    self.close_minibuf();
                }
            }
            // Backspace on already-empty input: dismiss.
            MiniBufferEvent::BackspaceOnEmpty => {
                self.set_mode(Mode::Normal);
                self.close_minibuf();
            }
            // Any edit, cursor move, or Backspace that clears to empty dismisses the
            // completion popup and demotes any active history recall back to scratch.
            // EmptiedByBackspace keeps the minibuffer open (showing just the prompt)
            // so a second Backspace is needed to dismiss — avoids accidental closure
            // when the user deletes a one-char typo.
            MiniBufferEvent::EmptiedByBackspace
            | MiniBufferEvent::Edited
            | MiniBufferEvent::CursorMoved => {
                self.state.completion = None;
                self.state
                    .history
                    .get_mut(HistoryKind::Command)
                    .demote_to_scratch();
            }
            MiniBufferEvent::CompleteRequested { reverse } => {
                self.complete_minibuf(reverse);
            }
            MiniBufferEvent::HistoryPrev => {
                self.state.completion = None;
                self.recall_history(HistoryKind::Command, HistoryDir::Prev);
            }
            MiniBufferEvent::HistoryNext => {
                self.state.completion = None;
                self.recall_history(HistoryKind::Command, HistoryDir::Next);
            }
            MiniBufferEvent::Ignored => {}
        }
    }

    /// B9: routes a Command-mode minibuffer event for a Steel `(prompt! …)`
    /// session rather than a `:` command line — no history, no completion,
    /// no directory-descend special case. Exactly one `(callback
    /// text-or-#f)` call fires, on Confirm or on any of the cancel paths.
    fn handle_steel_prompt_event(&mut self, event: MiniBufferEvent) {
        match event {
            MiniBufferEvent::Cancel
            | MiniBufferEvent::ConfirmEmpty
            | MiniBufferEvent::BackspaceOnEmpty => self.finish_steel_prompt(None),
            MiniBufferEvent::Confirm(text) => self.finish_steel_prompt(Some(text)),
            // Plain editing (char typed/deleted, cursor moved) is already
            // applied by `MiniBuffer::handle_key` — nothing further to do.
            // Tab/Up/Down are no-ops here (no completion, no history for a
            // one-shot prompt, per the card).
            MiniBufferEvent::Edited
            | MiniBufferEvent::CursorMoved
            | MiniBufferEvent::EmptiedByBackspace
            | MiniBufferEvent::CompleteRequested { .. }
            | MiniBufferEvent::HistoryPrev
            | MiniBufferEvent::HistoryNext
            | MiniBufferEvent::Ignored => {}
        }
    }

    /// Queues exactly one `(callback text-or-#f)` call and closes the prompt.
    fn finish_steel_prompt(&mut self, text: Option<String>) {
        let Some(callback) = self.state.steel_prompt_callback.take() else {
            return;
        };
        let arg = match text {
            Some(s) => steel::rvals::SteelVal::StringV(s.into()),
            None => steel::rvals::SteelVal::BoolV(false),
        };
        self.queue_steel_call(callback, vec![arg]);
        self.set_mode(Mode::Normal);
        self.close_minibuf();
    }

    /// Close the minibuffer and clear any active completion session.
    pub(super) fn close_minibuf(&mut self) {
        self.state.minibuf = None;
        self.state.completion = None;
        self.state.history.begin_session_all();
    }

    /// Recall the previous (`Prev`) or next (`Next`) entry from `kind`'s history
    /// ring and install it in the minibuffer. No-op when there is no active
    /// minibuffer or when the ring has nowhere to go.
    pub(super) fn recall_history(&mut self, kind: HistoryKind, dir: HistoryDir) {
        let current = self
            .state
            .minibuf
            .as_ref()
            .map(|m| m.input.as_str())
            .unwrap_or("");
        let text = match dir {
            HistoryDir::Prev => self.state.history.get_mut(kind).prev(current),
            HistoryDir::Next => self.state.history.get_mut(kind).next(),
        };
        if let Some(text) = text
            && let Some(mb) = self.state.minibuf.as_mut()
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
        if let Some(ref mut state) = self.state.completion {
            let n = state.candidates.len();
            // current_span() reflects what's currently in the input (based on the
            // previously-selected candidate), so it must be read before advancing
            // state.selected — after the update, candidates[selected] is the new one.
            let span = state.current_span();
            state.selected = if reverse {
                state.selected.checked_sub(1).unwrap_or(n - 1)
            } else {
                (state.selected + 1) % n
            };
            let replacement = state.candidates[state.selected].replacement.clone();
            if let Some(mb) = &mut self.state.minibuf {
                mb.input.replace_range(span.clone(), &replacement);
                mb.cursor = span.start + replacement.len();
            }
            return;
        }

        // Shift-Tab with no open popup is a no-op.
        if reverse {
            return;
        }

        // First Tab: extract input context without holding &mut self.state.minibuf.
        let (input, cursor) = match &self.state.minibuf {
            Some(mb) => (mb.input.clone(), mb.cursor),
            None => return,
        };

        // Only complete command-mode minibuffers.
        if self.state.minibuf.as_ref().map(|mb| mb.prompt.as_str()) != Some(":") {
            return;
        }

        let ctx = crate::editor::completion::CompletionCtx {
            registry: &self.state.registry,
            buffers: &self.state.buffers,
            cwd: &self.state.cwd,
            languages: &self.state.languages,
        };

        // Dispatch to the right completer based on command + input shape.
        use crate::editor::completion::{
            BufferNameCompleter, CommandCompleter, Completer, CompletionResult, CompletionState,
            PathCompleter, SetCompleter, ThemeCompleter,
        };

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
                    // Resolve alias → canonical name.
                    let cmd = cmd_raw.strip_suffix('!').unwrap_or(cmd_raw);
                    let canonical = self
                        .state
                        .registry
                        .get_typed(cmd)
                        .map(|tc| tc.name.as_ref());
                    match canonical {
                        Some("edit" | "write" | "write-quit") => {
                            PathCompleter { dirs_only: false }.complete(&input, cursor, &ctx)
                        }
                        Some("change-directory") => {
                            PathCompleter { dirs_only: true }.complete(&input, cursor, &ctx)
                        }
                        Some("buffer") => BufferNameCompleter.complete(&input, cursor, &ctx),
                        Some("theme") => ThemeCompleter.complete(&input, cursor, &ctx),
                        Some("set") => SetCompleter.complete(&input, cursor, &ctx),
                        // `:bd` ignores its argument; skip completion to
                        // avoid a misleading pick-then-close-current-buffer UX.
                        _ => return,
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
            if let Some(mb) = &mut self.state.minibuf {
                mb.input.replace_range(span_start..cursor, &replacement);
                mb.cursor = span_start + replacement.len();
            }
            return;
        }

        // Two or more: open popup with the first candidate selected.
        let replacement = candidates[0].replacement.clone();
        if let Some(mb) = &mut self.state.minibuf {
            mb.input.replace_range(span_start..cursor, &replacement);
            mb.cursor = span_start + replacement.len();
        }
        self.state.completion = Some(CompletionState {
            candidates,
            selected: 0,
            span_start,
        });
    }

    /// Execute the command currently in the mini-buffer.
    ///
    /// Called just before the mini-buffer is cleared and mode returns to Normal.
    fn execute_command(&mut self) {
        let input = self
            .state
            .minibuf
            .as_ref()
            .map(|m| m.input.trim().to_owned())
            .unwrap_or_default();

        let (cmd, force, arg) = parse_typed_command(&input);

        // Bare line number `:42` — shorthand for `:goto 42`.
        // The parser leaves `cmd = ""` for digit-only input (digits are excluded
        // from the command-name alphabet so `:42` → cmd="" arg=Some("42")).
        if cmd.is_empty()
            && !force
            && arg.is_some_and(|a| !a.is_empty() && a.bytes().all(|b| b.is_ascii_digit()))
        {
            if let Err(e) = typed_goto_line(self, arg, false) {
                self.report(Severity::Error, e.message().to_owned());
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
                        self.report(Severity::Error, e.message().to_owned());
                        return;
                    }
                }
            }
            Some(a) => Some(a.to_owned()),
            None => None,
        };

        if let Some(tc) = self.state.registry.get_typed(cmd) {
            let fun = tc.fun;
            if let Err(e) = fun(self, expanded.as_deref(), force) {
                self.report(Severity::Error, e.message().to_owned());
            }
        } else if let Some(mut mappable) = self.state.registry.get_mappable(cmd).cloned() {
            // Any mappable command can be invoked from the command line with
            // an implicit count of 1. This means `:clear-search`, `:undo`, etc.
            // all work without needing typed-command wrappers.
            //
            // Lazy stubs are activated before arity marshalling so `:bar arg`
            // does not silently drop `arg` on the first call.
            if let MappableCommand::Lazy { plugin, .. } = &mappable {
                let plugin = plugin.clone();
                if !self.activate_lazy_plugin(&plugin, cmd) {
                    self.report(Severity::Warning, format!("Unknown command: {cmd}"));
                    return;
                }
                match self.state.registry.get_mappable(cmd).cloned() {
                    Some(m) => mappable = m,
                    None => {
                        self.report(Severity::Warning, format!("Unknown command: {cmd}"));
                        return;
                    }
                }
            }
            let steel_args = if let MappableCommand::SteelBacked {
                arity, is_variadic, ..
            } = &mappable
            {
                use steel::rvals::SteelVal;
                if *arity == 0 && !*is_variadic {
                    vec![]
                } else if *arity == 1 || *is_variadic {
                    match expanded {
                        Some(ref s) => vec![SteelVal::StringV(s.clone().into())],
                        // No arg typed: default count=1 for count-type lambdas; string-type
                        // lambdas reject IntV(1) via their own (string? x) guard.
                        None => vec![SteelVal::IntV(1)],
                    }
                } else {
                    self.report(
                        Severity::Error,
                        format!(":{cmd} requires {arity} args; the minibuffer can only supply 1"),
                    );
                    return;
                }
            } else {
                vec![]
            };
            self.execute_keymap_command(cmd.to_owned().into(), Some(1), false, steel_args);
        } else {
            self.report(Severity::Warning, format!("Unknown command: {cmd}"));
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
                    .ok_or_else(|| CommandError::new("No file name"))?;
                out.push_str(&path.display().to_string());
            }
            "#" => {
                let alt_id = ed
                    .alternate_buffer()
                    .ok_or_else(|| CommandError::new("No alternate buffer"))?;
                let alt_path = ed
                    .state
                    .buffers
                    .get(alt_id)
                    .path()
                    .ok_or_else(|| CommandError::new("Alternate buffer has no file name"))?;
                out.push_str(&alt_path.display().to_string());
            }
            other => out.push_str(other),
        }
    }
    Ok(out)
}
