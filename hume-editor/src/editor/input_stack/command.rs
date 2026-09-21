//! The `Command` layer — the `:`-prompt minibuffer mode.

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;

use super::super::completion;
use super::super::error::CommandError;
use super::super::input_stack::CompletionLayer;
use super::super::minibuf::history::{HistoryDir, HistoryKind};
use super::super::minibuf::{self, MiniBuffer, MiniBufferEvent};
use super::super::registry::TypedBody;
use super::super::{Editor, EditorState, Severity, commands};
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, Removal};

pub(in crate::editor) struct CommandLayer {
    pub(in crate::editor) minibuf: MiniBuffer,
}

impl Layer for CommandLayer {
    fn handler(&self) -> LayerHandler {
        command_input
    }
    fn mode(&self) -> Option<EditorMode> {
        Some(EditorMode::Command)
    }
    fn tear_down(&mut self, state: &mut EditorState, _view: &EngineView, _why: Removal) {
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
    /// Write the open minibuffer completion session into the shared
    /// `PopupState` Arc — same widget as [`Self::sync_menu_view`]/
    /// [`Self::sync_completion_menu_view`] (unwrapped rows, selected-row
    /// styling), but anchored at the pane's bottom edge above the
    /// statusline rather than a buffer cursor. `Minibuf`-target only — a
    /// `Buffer`-target session renders through
    /// [`Self::sync_completion_menu_view`] instead, into its own slot.
    ///
    /// Called from `prepare_frame`'s overlay-sync step. Needs only
    /// `last_pane_area` (settled in step 0), unlike its cursor-anchored
    /// siblings, which need the current frame's scroll result too.
    /// `&mut self`, not `&self` — `menu_rows()` lazily populates a cache.
    pub(in crate::editor) fn sync_minibuf_completion_view(&mut self) {
        let is_open = self
            .state
            .input
            .completion()
            .is_some_and(|s| s.minibuf_span().is_some());
        // Skip the write-lock when both sides are already None — common case
        // while no popup is open.
        if !is_open && self.state.views.minibuf_completion.read().is_none() {
            return;
        }
        let pane_rect = self.view.last_pane_area;
        // Sequential borrows, same reasoning as `sync_completion_menu_view`:
        // the shared reads (span, selection) have to end before `menu_rows`
        // takes `&mut self`.
        let view = (|| -> Option<hume_ui::popup::PopupState> {
            let session = self.state.input.completion()?;
            let span = session.minibuf_span()?;
            let selected = self.state.input.completion_ui().map_or(0, |ui| ui.selected);
            let anchor_x = self
                .state
                .input
                .minibuf()
                .map(|mb| mb.cursor_x_at(span.start))
                .unwrap_or(0);
            let session = self.state.input.completion_mut()?;
            let rows = session.menu_rows();
            // Anchoring at the pane's bottom edge is what drives
            // `resolve_popup_geometry` into its flip-above branch
            // (`space_below` saturates to 0 there), landing the box on the
            // rows just above the statusline. The -1 pulls the frame left so
            // the first label column sits under the token in the input.
            Some(hume_ui::popup::resolve_menu(
                rows,
                selected,
                hume_ui::popup::PopupPlacement {
                    anchor: (anchor_x.saturating_sub(1), pane_rect.bottom()),
                    pane_rect,
                    content_width: pane_rect.width,
                },
                self.state.settings.popup_border,
            ))
        })();
        self.state.views.minibuf_completion.set(view);
    }
}

pub(in crate::editor) fn command_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    if let Some(event) = minibuf::minibuf_input(ed, r, ev) {
        handle_command_event(ed, r, event);
    }
}

fn handle_command_event(ed: &mut Editor, r: LayerRef, event: MiniBufferEvent) {
    // No completion-dismiss calls in this function, on any arm: a
    // `CompletionLayer` sits *above* `Command` whenever a popup is open, and
    // `completion_input_minibuf` (`input_stack/completion.rs`) dismisses it
    // before falling through to whatever runs here — by the time any of
    // these arms sees an event, either no popup was open, or one already
    // was and just closed. One place enforces "any non-Tab key dismisses",
    // not one check repeated at every event site here.
    match event {
        MiniBufferEvent::Cancel
        | MiniBufferEvent::ConfirmEmpty
        | MiniBufferEvent::BackspaceOnEmpty => {
            ed.state.truncate_layers(&ed.view, r);
        }
        MiniBufferEvent::Confirm(_) => {
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
        // An edit, cursor move, or Backspace that clears to empty demotes
        // any active history recall back to scratch. EmptiedByBackspace
        // keeps the minibuffer open (showing just the prompt) so a second
        // Backspace is needed to dismiss — avoids accidental closure when
        // the user deletes a one-char typo.
        MiniBufferEvent::EmptiedByBackspace
        | MiniBufferEvent::Edited
        | MiniBufferEvent::CursorMoved => {
            ed.state
                .history
                .get_mut(HistoryKind::Command)
                .demote_to_scratch();
        }
        MiniBufferEvent::CompleteRequested { reverse } => {
            complete_minibuf(ed, reverse);
        }
        MiniBufferEvent::HistoryPrev => {
            minibuf::recall_history(ed, HistoryKind::Command, HistoryDir::Prev);
        }
        MiniBufferEvent::HistoryNext => {
            minibuf::recall_history(ed, HistoryKind::Command, HistoryDir::Next);
        }
        MiniBufferEvent::Ignored => {}
    }
}

/// Drive one Tab / Shift-Tab cycle in the completion popup.
///
/// Runs only for the *first* Tab, when no completion session is open yet:
/// once one opens (a `CompletionLayer` pushed above this one), every
/// subsequent Tab/Shift-Tab — and the directory-descend-on-Enter check — is
/// handled by that layer's own key handler
/// (`completion_input_minibuf`, `input_stack/completion.rs`) before it ever
/// reaches here again. Resolves the applicable source for the current input
/// shape, runs it, and either applies the sole candidate silently, or opens
/// the popup with the first candidate already applied — a `Minibuf`-target
/// session's own cycle-and-apply behavior (see `CompletionTarget`'s doc,
/// `completion/session.rs`).
pub(in crate::editor::input_stack) fn complete_minibuf(ed: &mut Editor, reverse: bool) {
    // Shift-Tab with no open popup is a no-op.
    if reverse {
        return;
    }

    // Extract input context without holding &mut ed.state.input.
    let (input, cursor) = match ed.state.input.minibuf() {
        Some(mb) => (mb.input.clone(), mb.cursor),
        None => return,
    };

    // Only complete command-mode minibuffers.
    if ed.state.input.minibuf().map(|mb| mb.prompt.as_str()) != Some(":") {
        return;
    }

    let Some((source_name, universe_span_start)) = resolve_minibuf_source(ed, &input, cursor)
    else {
        return;
    };

    let ctx = completion::CompletionCtx {
        registry: &ed.state.config.registry,
        buffers: &ed.state.buffers,
        cwd: &ed.state.cwd,
        languages: &ed.state.config.languages,
    };

    let Some((match_kind, source_result)) =
        ed.state
            .config
            .completion_sources
            .run(source_name, &input, cursor, &ctx)
    else {
        // `TypedCommand.completer` naming no registered source — a stale
        // name after a rename. Silent to the user, same as `:bd` declaring
        // no completer at all; loud enough to find in the log.
        ed.report(
            Severity::Trace,
            format!("no completion source named {source_name:?}"),
        );
        return;
    };

    use completion::SourceResult;
    let (span, items) = match source_result {
        SourceResult::Universe(items) => {
            // The `Universe` shape doesn't carry a span of its own — every
            // source of this kind shares the same forward boundary
            // (whitespace), unlike `Delegated`'s per-source rules (`:set`'s
            // `'='` stop, for one).
            let span_end = completion::token_end_at(&input, cursor, &[' ']);
            (universe_span_start..span_end, items)
        }
        SourceResult::Delegated { span, items } => (span, items),
    };

    if items.is_empty() {
        return;
    }

    // A `Universe`-kind source (`String`/`Fuzzy`) returns its whole stable
    // candidate set unfiltered — narrowing against what's actually been
    // typed is the session's own job (`MatchKind::String`'s boundary-safe
    // prefix gate), not this function's. Building the session and filtering
    // it before the single-vs-multi decision below means that decision
    // reads real match counts, not the source's raw universe size. A
    // `Delegated` source already returned its final, filtered order — its
    // own `MatchKind::Delegated` ignores filter text entirely, so applying
    // it here is a no-op for that case.
    let source = Box::<str>::from(source_name);
    // The filter narrows on the cursor-bounded prefix (what the user has
    // actually typed), never the full `span` — a `MatchKind::String`
    // source's boundary-safe prefix gate would otherwise filter on the
    // whole word the cursor sits inside, showing only the exact match the
    // user is standing in the middle of.
    let prefix_text = input[span.start.min(input.len())..cursor.min(input.len())].to_owned();
    let mut session =
        completion::CompletionSession::begin_minibuf(span.clone(), source, match_kind, items);
    session.update_filter(prefix_text);

    if session.is_empty() {
        return;
    }

    if session.len() == 1 {
        // Single match: apply silently without opening a popup.
        let insert_text = session
            .selected_item(0)
            .expect("len() == 1 just above")
            .insert_text()
            .to_owned();
        if let Some(mb) = ed.state.input.minibuf_mut() {
            mb.splice(span, &insert_text);
        }
        return;
    }

    // Two or more: open the session and apply the first candidate.
    let new_r = ed
        .state
        .push_layer(&ed.view, CompletionLayer { session, ui: None });
    super::completion::apply_selected_minibuf_candidate(ed, new_r);
}

/// Resolves which registered source applies to the current `(input,
/// cursor)` shape, and — for a `String`/`Fuzzy` source, whose function no
/// longer sees the input at all — the span its completed token starts at
/// (a `Delegated` source computes its own span instead, from the live
/// input it's handed directly).
fn resolve_minibuf_source(
    ed: &Editor,
    input: &str,
    cursor: usize,
) -> Option<(&'static str, usize)> {
    match input.split_once(' ') {
        // No space yet, or the cursor sits within the command name (moved
        // left past the space) — complete the command name itself.
        None => Some((completion::COMMAND_SOURCE, 0)),
        Some((cmd_raw, _)) if cursor <= cmd_raw.len() => Some((completion::COMMAND_SOURCE, 0)),
        Some((cmd_raw, _)) => {
            // Resolve alias → command, and its declared argument completer.
            let cmd = cmd_raw.strip_suffix('!').unwrap_or(cmd_raw);
            let name = ed.state.config.registry.get_typed(cmd)?.completer?;
            let (arg_start, _) = completion::arg_prefix(input, cursor);
            Some((name, arg_start))
        }
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
