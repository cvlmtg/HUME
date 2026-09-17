//! The `Command` layer — the `:`-prompt minibuffer mode.

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;

use super::super::completion::MinibufCompletionState;
use super::super::error::CommandError;
use super::super::minibuf::history::{HistoryDir, HistoryKind};
use super::super::minibuf::{self, MiniBuffer, MiniBufferEvent};
use super::super::registry::TypedBody;
use super::super::{Editor, EditorState, commands};
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef};

pub(in crate::editor) struct CommandLayer {
    pub(in crate::editor) minibuf: MiniBuffer,
    pub(in crate::editor) completion: Option<MinibufCompletionState>,
}

impl Layer for CommandLayer {
    fn handler(&self) -> LayerHandler {
        command_input
    }
    fn mode(&self) -> Option<EditorMode> {
        Some(EditorMode::Command)
    }
    fn tear_down(&mut self, state: &mut EditorState, _view: &EngineView) {
        state.history.begin_session_all();
    }
    fn minibuf(&self) -> Option<&MiniBuffer> {
        Some(&self.minibuf)
    }
    fn minibuf_mut(&mut self) -> Option<&mut MiniBuffer> {
        Some(&mut self.minibuf)
    }
}

impl Editor {
    /// Write the current completion state into the shared `MinibufCompletionView`
    /// so `MinibufCompletionOverlay` can render it during this frame.
    ///
    /// Called from `prepare_frame` after highlight data is synced.
    pub(in crate::editor) fn sync_minibuf_completion_view(&self) {
        let completion = self.state.input.minibuf_completion();
        // Skip the write-lock when both sides are already None — common case
        // while no popup is open.
        if completion.is_none() && self.state.views.minibuf_completion.read().is_none() {
            return;
        }
        let view = completion.map(|state| {
            let anchor_x = self
                .state
                .input
                .minibuf()
                .map(|mb| mb.cursor_x_at(state.span_start))
                .unwrap_or(0);
            hume_ui::completion_overlay::MinibufCompletionView {
                rows: state.rows.clone(),
                selected: state.selected,
                anchor_x,
                border: self.state.settings.popup_border,
            }
        });
        self.state.views.minibuf_completion.set(view);
    }
}

impl super::stack::InputStack {
    /// The active completion session, flattened — `None` both when no
    /// `Command` layer is open and when one is open with no session. Reads
    /// only; see [`Self::minibuf_completion_mut`] to replace or clear it.
    pub(in crate::editor) fn minibuf_completion(&self) -> Option<&MinibufCompletionState> {
        self.find::<CommandLayer>()
            .and_then(|l| l.completion.as_ref())
    }

    /// The `Command` layer's completion slot itself (not its content) —
    /// `Some(&mut Option<..>)` whenever a `Command` layer is open, letting a
    /// caller assign a fresh session or clear one (`*slot = None`), as
    /// opposed to [`Self::minibuf_completion`]'s flattened read.
    pub(in crate::editor) fn minibuf_completion_mut(
        &mut self,
    ) -> Option<&mut Option<MinibufCompletionState>> {
        self.find_mut::<CommandLayer>().map(|l| &mut l.completion)
    }

    /// Clears the `Command` layer's completion slot, if one is open —
    /// [`Self::minibuf_completion_mut`]'s common case, shared by every
    /// event that dismisses the popup without closing the minibuffer
    /// itself (an edit, a cursor move, a history recall).
    pub(in crate::editor) fn clear_minibuf_completion(&mut self) {
        if let Some(l) = self.find_mut::<CommandLayer>() {
            l.completion = None;
        }
    }
}

pub(in crate::editor) fn command_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    if let Some(event) = minibuf::minibuf_input(ed, r, ev) {
        handle_command_event(ed, r, event);
    }
}

fn handle_command_event(ed: &mut Editor, r: LayerRef, event: MiniBufferEvent) {
    match event {
        MiniBufferEvent::Cancel
        | MiniBufferEvent::ConfirmEmpty
        | MiniBufferEvent::BackspaceOnEmpty => {
            ed.state.truncate_layers(&ed.view, r);
        }
        MiniBufferEvent::Confirm(_) => {
            // If the selected completion candidate is a directory
            // (trailing `/`), Enter descends into it instead of executing:
            // the candidate is already in the input (Tab applied it), so
            // we just dismiss the popup and restart completion for the
            // directory's children.
            if ed
                .state
                .input
                .minibuf_completion()
                .and_then(|s| s.candidates.get(s.selected))
                .is_some_and(|c| c.replacement.ends_with('/'))
            {
                ed.state.input.clear_minibuf_completion();
                complete_minibuf(ed, false);
                return;
            }
            // Record into history and extract the input before
            // truncating — the `Command` layer (and the minibuf it
            // owns) is gone by the time `execute_command` runs, so the
            // string has to be taken out first, per truncate-before-
            // execute: the body below runs with the mode layer already
            // back at `Base`, so a `:cmd` that enters Insert stays in
            // Insert instead of being stomped back to Normal, and a
            // body calling `(prompt! …)` pushes `Prompt` on a clean
            // stack with no special case needed.
            let raw = ed
                .state
                .input
                .minibuf()
                .map(|mb| mb.input.clone())
                .unwrap_or_default();
            ed.state
                .history
                .get_mut(HistoryKind::Command)
                .push(raw.clone());
            let input = raw.trim().to_owned();
            ed.state.truncate_layers(&ed.view, r);
            execute_command(ed, &input);
        }
        // Any edit, cursor move, or Backspace that clears to empty dismisses the
        // completion popup and demotes any active history recall back to scratch.
        // EmptiedByBackspace keeps the minibuffer open (showing just the prompt)
        // so a second Backspace is needed to dismiss — avoids accidental closure
        // when the user deletes a one-char typo.
        MiniBufferEvent::EmptiedByBackspace
        | MiniBufferEvent::Edited
        | MiniBufferEvent::CursorMoved => {
            ed.state.input.clear_minibuf_completion();
            ed.state
                .history
                .get_mut(HistoryKind::Command)
                .demote_to_scratch();
        }
        MiniBufferEvent::CompleteRequested { reverse } => {
            complete_minibuf(ed, reverse);
        }
        MiniBufferEvent::HistoryPrev => {
            ed.state.input.clear_minibuf_completion();
            minibuf::recall_history(ed, HistoryKind::Command, HistoryDir::Prev);
        }
        MiniBufferEvent::HistoryNext => {
            ed.state.input.clear_minibuf_completion();
            minibuf::recall_history(ed, HistoryKind::Command, HistoryDir::Next);
        }
        MiniBufferEvent::Ignored => {}
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
fn complete_minibuf(ed: &mut Editor, reverse: bool) {
    // If completion is already open, cycle to the next candidate.
    if let Some(slot) = ed.state.input.minibuf_completion_mut()
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
        if let Some(mb) = ed.state.input.minibuf_mut() {
            mb.input.replace_range(span.clone(), &replacement);
            mb.cursor = span.start + replacement.len();
        }
        return;
    }

    // Shift-Tab with no open popup is a no-op.
    if reverse {
        return;
    }

    // First Tab: extract input context without holding &mut ed.state.input.
    let (input, cursor) = match ed.state.input.minibuf() {
        Some(mb) => (mb.input.clone(), mb.cursor),
        None => return,
    };

    // Only complete command-mode minibuffers.
    if ed.state.input.minibuf().map(|mb| mb.prompt.as_str()) != Some(":") {
        return;
    }

    let ctx = crate::editor::completion::CompletionCtx {
        registry: &ed.state.config.registry,
        buffers: &ed.state.buffers,
        cwd: &ed.state.cwd,
        languages: &ed.state.config.languages,
    };

    // Dispatch to the right completer based on command + input shape.
    use crate::editor::completion::{
        BufferNameCompleter, CommandCompleter, Completer, CompletionResult, MinibufCompletionState,
        PathCompleter, SetCompleter, ThemeCompleter,
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
                let completer = ed
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
        if let Some(mb) = ed.state.input.minibuf_mut() {
            mb.input.replace_range(span_start..cursor, &replacement);
            mb.cursor = span_start + replacement.len();
        }
        return;
    }

    // Two or more: open popup with the first candidate selected.
    let replacement = candidates[0].replacement.clone();
    if let Some(mb) = ed.state.input.minibuf_mut() {
        mb.input.replace_range(span_start..cursor, &replacement);
        mb.cursor = span_start + replacement.len();
    }
    let rows = hume_ui::popup::MenuRows::measure(std::sync::Arc::new(
        candidates.iter().map(|c| c.display.clone()).collect(),
    ));
    if let Some(slot) = ed.state.input.minibuf_completion_mut() {
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
/// before-execute.
fn execute_command(ed: &mut Editor, input: &str) {
    let (cmd, force, arg) = parse_typed_command(input);

    // Bare line number `:42` — shorthand for `:goto 42`.
    // The parser leaves `cmd = ""` for digit-only input (digits are excluded
    // from the command-name alphabet so `:42` → cmd="" arg=Some("42")).
    if cmd.is_empty()
        && !force
        && arg.is_some_and(|a| !a.is_empty() && a.bytes().all(|b| b.is_ascii_digit()))
    {
        if let Err(e) = commands::typed_goto_line(ed, arg, false) {
            ed.report(e.severity(), e.message().to_owned());
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
            match expand_command_arg(ed, a) {
                Ok(s) => Some(s),
                Err(e) => {
                    ed.report(e.severity(), e.message().to_owned());
                    return;
                }
            }
        }
        Some(a) => Some(a.to_owned()),
        None => None,
    };

    // `:` resolves only typed commands — an editor (key-bindable) command's
    // name is unreachable here; see `registry/mod.rs`'s module doc.
    match ed.state.config.registry.get_typed(cmd) {
        Some(tc) => match &tc.body {
            TypedBody::Native(fun) => {
                let fun = *fun;
                if let Err(e) = fun(ed, expanded.as_deref(), force) {
                    ed.report(e.severity(), e.message().to_owned());
                }
            }
            // Lazy-stub activation and Steel arg marshalling both happen
            // inside `Editor::run_typed_steel_command`, so `:bar arg`
            // cannot silently drop `arg` on the plugin's first call.
            TypedBody::Steel { .. } => {
                ed.run_typed_steel_command(cmd, None, expanded, force);
            }
            TypedBody::Lazy(plugin) => {
                let plugin = plugin.clone();
                ed.run_typed_steel_command(cmd, Some(plugin), expanded, force);
            }
        },
        None => {
            ed.report_unknown_command(cmd, format!("Unknown command: {cmd}"));
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
pub(in crate::editor) fn parse_typed_command(input: &str) -> (&str, bool, Option<&str>) {
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
