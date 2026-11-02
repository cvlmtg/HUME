//! `EditorHostImpl`'s inlay hints, signs, virtual lines, extra highlights,
//! EOL text, statusline text, and the diagnostic pull/count reads.
//!
//! `validate_offset` is `pub(super)` because `host_impl/tests.rs`, a
//! sibling of this module, calls it.

use hume_core::{BufferId, DiagSeverity, Side};
use hume_rope::line::ContentLine;
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::editor::EditorState;
use crate::statusline::StatusElement;

use super::EditorHostImpl;
use hume_scripting::host::{DecorationHost, DiagnosticCounts, MarkedLines, SetError, each_entry};

impl<'a> DecorationHost for EditorHostImpl<'a> {
    fn set_inlay_hints(
        &mut self,
        source: String,
        bid: BufferId,
        hints: Vec<(usize, String, Side)>,
    ) -> Result<(), SetError> {
        let text = buffer_text(self.state, bid).map_err(SetError::Call)?;
        let entries = each_entry(hints, |(pos, hint_text, side)| {
            let pos = validate_offset(text, pos, side)?;
            Ok(hume_decorations::InlayHintEntry {
                pos,
                text: hint_text,
                side,
            })
        })?;
        self.state
            .config
            .decorations
            .set_inlay_hints(source, bid, entries);
        Ok(())
    }

    fn register_sign_source(
        &mut self,
        source: String,
        bid: BufferId,
        priority: i64,
    ) -> Result<(), String> {
        self.state
            .config
            .decorations
            .register_sign_source(source, bid, priority);
        Ok(())
    }

    fn set_signs(
        &mut self,
        source: String,
        bid: BufferId,
        signs: Vec<(MarkedLines, String, String)>,
    ) -> Result<(), SetError> {
        if self
            .state
            .config
            .decorations
            .sign_slot(bid, &source)
            .is_none()
        {
            return Err(SetError::Call(format!(
                "unregistered sign source {source:?}, call \
                 (register-sign-source! …) first"
            )));
        }
        let signs = signs.into_iter().map(|(lines, sign_text, scope)| {
            let scope = self.view.registry.intern_runtime(&scope);
            (lines, (std::sync::Arc::<str>::from(sign_text), scope))
        });
        let entries = each_line(signs)
            .map(|(line, (text, scope))| hume_decorations::SignEntry { line, text, scope })
            .collect();
        self.state
            .config
            .decorations
            .set_signs(source, bid, entries);
        Ok(())
    }

    fn set_virtual_lines(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<(ContentLine, hume_scripting::VirtualLineSpec)>,
    ) -> Result<(), SetError> {
        let entries = lines
            .into_iter()
            .map(|(line, spec)| {
                let scope = match spec.scope.as_deref() {
                    Some(name) => self.view.registry.intern_runtime(name),
                    None => self
                        .view
                        .registry
                        .intern(hume_engine::theme::ui_scopes::VIRTUAL_TEXT),
                };
                hume_decorations::VirtualLineEntry {
                    line,
                    content: spec
                        .content
                        .map_payload(|name| self.view.registry.intern_runtime(&name)),
                    side: spec.side,
                    scope,
                }
            })
            .collect();
        self.state
            .config
            .decorations
            .set_virtual_lines(source, bid, entries);
        Ok(())
    }

    fn set_extra_highlights(
        &mut self,
        source: String,
        bid: BufferId,
        spans: Vec<(usize, usize, String)>,
    ) -> Result<(), SetError> {
        let text = buffer_text(self.state, bid).map_err(SetError::Call)?;
        let entries = each_entry(spans, |(start, end, scope)| {
            let fail = |e: String| format!("range {e}");
            let range = crate::editor::buffer::offsets::checked_char_range(text, start, end)
                .map_err(fail)?;
            if range.is_empty() {
                return Err(fail(format!("{start}..{end} is empty")));
            }
            // Widened to whole clusters: the renderer paints a cluster at a time.
            let range = crate::editor::buffer::offsets::widened_to_clusters(text, range);
            Ok(hume_decorations::ExtraHighlightEntry {
                start: range.start,
                end: range.end,
                scope: self.view.registry.intern_runtime(&scope),
            })
        })?;
        self.state
            .config
            .decorations
            .set_extra_highlights(source, bid, entries);
        Ok(())
    }

    fn set_eol_text(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<(ContentLine, String, String)>,
        hide_on_insert_line: bool,
    ) -> Result<(), SetError> {
        let entries = lines
            .into_iter()
            .map(|(line, eol_text, scope)| hume_decorations::EolTextEntry {
                line,
                text: eol_text,
                scope: self.view.registry.intern_runtime(&scope),
                hide_on_insert_line,
            })
            .collect();
        self.state
            .config
            .decorations
            .set_eol_text(source, bid, entries);
        Ok(())
    }

    fn set_line_backgrounds(
        &mut self,
        source: String,
        bid: BufferId,
        entries: Vec<(MarkedLines, String)>,
    ) -> Result<(), SetError> {
        let entries = entries
            .into_iter()
            .map(|(lines, scope)| (lines, self.view.registry.intern_runtime(&scope)));
        let entries = each_line(entries)
            .map(|(line, scope)| hume_decorations::LineBgEntry { line, scope })
            .collect();
        self.state
            .config
            .decorations
            .set_line_backgrounds(source, bid, entries);
        Ok(())
    }

    fn set_statusline_text(
        &mut self,
        source: String,
        bid: BufferId,
        text: String,
    ) -> Result<(), String> {
        // Return value discarded: called purely to reuse the SSOT "unknown
        // buffer" wording every sibling setter raises for a stale `bid`.
        buffer_text(self.state, bid)?;
        // A name `StatusElement::from_str` would reject can never be placed,
        // so reject it here too, or the push silently stores an entry no
        // `steel:<name>` element can ever render.
        StatusElement::custom(&source).map_err(|e| e.to_string())?;
        // Wholesale replace, same as every sibling decoration setter
        // (`SourceStore::set`): an empty `text` is stored as-is rather than
        // pruned; `render_element`'s `Custom` arm and `render_section` both
        // already treat empty and absent identically.
        self.state
            .config
            .statusline_text
            .entry(bid)
            .or_default()
            .insert(source.into_boxed_str(), text.into_boxed_str());
        Ok(())
    }

    fn diagnostics_for_buffer(
        &mut self,
        bid: BufferId,
        severity_floor: Option<DiagSeverity>,
        range: Option<hume_scripting::host::DiagnosticRange>,
    ) -> Result<Vec<hume_scripting::host::DiagnosticEntry>, String> {
        crate::editor::lsp::diagnostics::diagnostics_for_buffer(
            self.state,
            bid,
            severity_floor,
            range,
        )
    }

    fn diagnostic_counts(&self, bid: BufferId, floor: Option<DiagSeverity>) -> DiagnosticCounts {
        crate::editor::lsp::introspect::diagnostic_counts(self.state, bid, floor)
    }
}

/// The live text for `bid`, or `Err` if `bid` doesn't name
/// an open buffer, for the setters that validate char offsets against it
/// and the statusline setter's "unknown buffer" wording. `bid`'s liveness is already checked before
/// any of these setters' bodies run (`args::LiveBuffer`'s `BuiltinArg::resolve`
/// in the `builtins!`-registered closure), so the `Err` arm here is a
/// defensive fallback, not the primary check. It is kept because `try_get` could
/// in principle diverge from the liveness check's own `buffer_exists`.
fn buffer_text(
    state: &EditorState,
    bid: BufferId,
) -> Result<&hume_editing::text::BufferText, String> {
    state
        .buffers
        .try_get(bid)
        .map(|b| b.text())
        .ok_or_else(|| "unknown buffer".to_owned())
}

/// Each line of each entry in `entries`, paired with its payload, in entry
/// order: the store keeps the first entry for a line.
fn each_line<T: Clone>(
    entries: impl IntoIterator<Item = (MarkedLines, T)>,
) -> impl Iterator<Item = (ContentLine, T)> {
    entries.into_iter().flat_map(|(marked, payload)| {
        marked
            .lines()
            .iter()
            .map(move |line| (line, payload.clone()))
    })
}

/// `pos` must address a real char in `text` (`<` its length), `Err`
/// otherwise. A `pos` inside a grapheme cluster snaps to it: a
/// `'before` hint to the cluster's first char, an `'after` one to its last,
/// so the hint sits before or after the whole character.
/// `pos == len_chars()` is refused: `visible_char_range` is half-open, so
/// that position never passes its `contains` check and the hint would not
/// render.
///
/// An 'after' hint gets a second check: the render bridge
/// (`decoration_providers.rs`'s `update_inlay_hint_providers`) anchors an
/// 'after' hint at `pos + 1`, so a hint on the buffer's last content char
/// (its trailing structural `\n`) would resolve to the trailing phantom
/// line, the same unresolvable position the line-anchored kinds' line check
/// refuses (`hume_scripting`'s `content_line_arg`), but reachable here
/// through a char offset instead of a line number, so that check alone
/// doesn't catch it.
pub(super) fn validate_offset(
    text: &hume_editing::text::BufferText,
    pos: usize,
    side: Side,
) -> Result<CharOffset, String> {
    if pos >= text.len_chars() {
        return Err(format!(
            "offset {pos} is out of range (buffer has {} chars)",
            text.len_chars()
        ));
    }
    // Trusted mint: `pos < len_chars()` is checked above.
    let at = CharOffset::new(pos);
    let cluster = text
        .covering(ExclusiveRange::new(at, at.shift(1)))
        .expect("a char below the text's length lies in a cluster");
    let pos = match side {
        Side::Before => cluster.start().offset(),
        Side::After => cluster.end().offset().retreat(1),
    };
    if side == Side::After {
        // `pos` is a char below `len_chars()`, so `pos` shifted by one
        // codepoint stays `<= len_chars()`: the one past-the-end position
        // `ropey_char_to_line` (ropey domain) accepts, needed to find which
        // line an 'after' hint at `pos + 1` actually lands on.
        let landing_line = text.ropey_char_to_line(pos.shift(1));
        if landing_line.to_content(text.rope()).is_none() {
            return Err(format!(
                "offset {} anchored 'after would land on the buffer's trailing \
                 empty line",
                pos.index()
            ));
        }
    }
    Ok(pos)
}
