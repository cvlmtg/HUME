//! `EditorHostImpl`'s inlay hints, signs, virtual lines, extra highlights,
//! EOL text, statusline text, and the diagnostic pull/count reads.
//!
//! The five position-validation free functions below are `pub(super)` where
//! `host_impl/tests.rs` (a sibling of this module, not a descendant) needs
//! them — the same reachable set (`host_impl` and its descendants) they had
//! as private items directly in `host_impl.rs`, just spelled differently now
//! that a caller of them lives outside this module.

use hume_engine::pipeline::BufferId;
use hume_rope::column::ByteCol;
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::editor::EditorState;
use crate::statusline::StatusElement;

use super::EditorHostImpl;
use hume_scripting::host::DecorationHost;

impl<'a> DecorationHost for EditorHostImpl<'a> {
    fn set_inlay_hints(
        &mut self,
        source: String,
        bid: BufferId,
        hints: Vec<(usize, String, bool)>,
    ) -> Result<(), String> {
        let text = buffer_text(self.state, bid, "set-inlay-hints!")?;
        let entries = hints
            .into_iter()
            .map(|(pos, hint_text, before)| {
                let pos = validate_offset(text, pos, before, "set-inlay-hints!")?;
                Ok(hume_decorations::InlayHintEntry {
                    pos,
                    text: hint_text,
                    before,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.state
            .config
            .decorations
            .set_inlay_hints(source, bid, entries);
        Ok(())
    }

    fn register_sign_source(
        &mut self,
        name: String,
        bid: BufferId,
        priority: i64,
    ) -> Result<(), String> {
        self.state
            .config
            .decorations
            .register_sign_source(name, bid, priority);
        Ok(())
    }

    fn set_signs(
        &mut self,
        source: String,
        bid: BufferId,
        signs: Vec<(usize, String, String)>,
    ) -> Result<(), String> {
        if self
            .state
            .config
            .decorations
            .sign_slot(bid, &source)
            .is_none()
        {
            return Err(format!(
                "set-signs!: unregistered sign source {source:?} — call \
                 (register-sign-source! …) first"
            ));
        }
        let text = buffer_text(self.state, bid, "set-signs!")?;
        let entries = signs
            .into_iter()
            .map(|(line, sign_text, scope)| {
                Ok(hume_decorations::SignEntry {
                    pos: line_start_offset(text, line, "set-signs!")?,
                    text: sign_text.into(),
                    scope: self.view.registry.intern_runtime(&scope),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
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
        lines: Vec<hume_scripting::VirtualLineSpec>,
    ) -> Result<(), String> {
        let text = buffer_text(self.state, bid, "set-virtual-lines!")?;
        let entries = lines
            .into_iter()
            .map(|spec| {
                let pos = line_start_offset(text, spec.line, "set-virtual-lines!")?;
                let scope = match spec.scope.as_deref() {
                    Some(name) => self.view.registry.intern_runtime(name),
                    None => self
                        .view
                        .registry
                        .intern(hume_engine::theme::ui_scopes::VIRTUAL_TEXT),
                };
                let segments = virtual_line_segments_to_bytes(&spec.text, spec.segments)?
                    .into_iter()
                    .map(|(start, end, name)| {
                        (
                            ByteCol::new(start),
                            ByteCol::new(end),
                            self.view.registry.intern_runtime(&name),
                        )
                    })
                    .collect();
                Ok(hume_decorations::VirtualLineEntry {
                    pos,
                    text: spec.text,
                    before: spec.before,
                    scope,
                    segments,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
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
    ) -> Result<(), String> {
        let text = buffer_text(self.state, bid, "set-extra-highlights!")?;
        let entries = spans
            .into_iter()
            .map(|(start, end, scope)| {
                let range = validate_range(text, start, end, "set-extra-highlights!")?;
                Ok(hume_decorations::ExtraHighlightEntry {
                    start: range.start,
                    end: range.end,
                    scope: self.view.registry.intern_runtime(&scope),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
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
        lines: Vec<(usize, String, String)>,
    ) -> Result<(), String> {
        let text = buffer_text(self.state, bid, "set-eol-text!")?;
        let entries = lines
            .into_iter()
            .map(|(line, eol_text, scope)| {
                Ok(hume_decorations::EolTextEntry {
                    pos: line_start_offset(text, line, "set-eol-text!")?,
                    text: eol_text,
                    scope: self.view.registry.intern_runtime(&scope),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
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
        entries: Vec<(usize, String)>,
    ) -> Result<(), String> {
        let text = buffer_text(self.state, bid, "set-line-backgrounds!")?;
        let entries = entries
            .into_iter()
            .map(|(line, scope)| {
                Ok(hume_decorations::LineBgEntry {
                    pos: line_start_offset(text, line, "set-line-backgrounds!")?,
                    scope: self.view.registry.intern_runtime(&scope),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
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
        // Return value discarded — called purely to reuse the SSOT "unknown
        // buffer" wording every sibling setter raises for a stale `bid`.
        buffer_text(self.state, bid, "set-statusline-text!")?;
        // A name `StatusElement::from_str` would reject can never be placed
        // — reject it here too, or the push silently stores an entry no
        // `steel:<name>` element can ever render.
        StatusElement::custom(&source).map_err(|e| format!("set-statusline-text!: {e}"))?;
        // Wholesale replace, same as every sibling decoration setter
        // (`SourceStore::set`) — an empty `text` is stored as-is rather than
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
        &self,
        bid: BufferId,
        severity_floor: Option<&str>,
        range: Option<(usize, usize)>,
    ) -> Result<Vec<serde_json::Value>, String> {
        let Some(lsp) = self.lsp.as_deref() else {
            return Ok(Vec::new());
        };
        // Converted immediately at the Steel/LSP host seam — `range` arrives
        // as the raw `(usize, usize)` tuple the FFI boundary decodes Steel's
        // `#:range` argument into (see `CharOffset`'s doc on this one carve-out)
        // and must not travel any further as one.
        let range = range.map(|(start, end)| {
            hume_rope::offset::ExclusiveRange::new(CharOffset::new(start), CharOffset::new(end))
        });
        crate::editor::lsp::introspect::diagnostics_for_buffer(
            self.state,
            lsp,
            bid,
            severity_floor,
            range,
        )
    }

    fn diagnostic_counts(&self, bid: BufferId) -> (usize, usize) {
        let Some(lsp) = self.lsp.as_deref() else {
            return (0, 0);
        };
        crate::editor::lsp::introspect::diagnostic_counts(lsp, bid)
    }
}

/// Converts `segments`' char offsets into `text` to byte offsets, sorting by
/// `start` and validating in the process — the sole enforcement point for
/// `set-virtual-lines!`'s segment contract (bounds, ordering, non-overlap,
/// grapheme-cluster alignment), now that the Steel boundary
/// (`virtual_line_specs` in `hume-scripting`'s `builtins/decorations.rs`)
/// only decodes shape. See `VirtualLineSpec::segments`'s doc.
///
/// Grapheme boundaries, not merely char boundaries: the engine
/// (`hume-engine/src/display_lines.rs`'s `segment_virtual_line`) resolves each virtual
/// grapheme's scope once per cluster, at the cluster's start byte. A segment
/// edge that splits a multi-codepoint cluster (e.g. `e` + combining acute)
/// would still pass a char-boundary check, but the engine's per-cluster
/// lookup would either paint the whole cluster with a segment that only
/// claimed part of it, or miss a segment that only claimed part of it — both
/// silent.
pub(super) fn virtual_line_segments_to_bytes(
    text: &str,
    mut segments: Vec<(usize, usize, String)>,
) -> Result<Vec<(usize, usize, String)>, String> {
    use unicode_segmentation::UnicodeSegmentation;

    segments.sort_by_key(|(start, _, _)| *start);

    // Char index -> byte offset, plus a sentinel one past the last char so
    // `end == char_count` resolves to `text.len()`.
    let char_to_byte: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    let char_count = char_to_byte.len() - 1;

    // Every grapheme-cluster start byte offset, plus end-of-text — sorted,
    // since `grapheme_indices` yields ascending byte offsets. Built once per
    // entry rather than re-walking `text` on every boundary check below.
    let grapheme_boundaries: Vec<usize> = text
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    let is_grapheme_boundary =
        |byte_offset: usize| grapheme_boundaries.binary_search(&byte_offset).is_ok();

    let mut prev_end = 0usize;
    let mut out = Vec::with_capacity(segments.len());
    for (start, end, scope) in segments {
        if start >= end {
            return Err(format!(
                "set-virtual-lines! segments: segment ({start}, {end}) must have start < end"
            ));
        }
        if end > char_count {
            return Err(format!(
                "set-virtual-lines! segments: segment end {end} is past text's char length {char_count}"
            ));
        }
        let start_byte = char_to_byte[start];
        let end_byte = char_to_byte[end];
        if !is_grapheme_boundary(start_byte) || !is_grapheme_boundary(end_byte) {
            return Err(format!(
                "set-virtual-lines! segments: segment ({start}, {end}) is not aligned to a \
                 grapheme-cluster boundary in text"
            ));
        }
        if start < prev_end {
            return Err(format!(
                "set-virtual-lines! segments: segments must not overlap (segment starting at \
                 {start} overlaps the previous one ending at {prev_end})"
            ));
        }
        prev_end = end;
        out.push((start_byte, end_byte, scope));
    }

    Ok(out)
}

/// The live text for `bid`, or `Err` naming `builtin` if `bid` doesn't name
/// an open buffer. Every decoration setter needs this to validate/convert
/// its Steel-facing positions, so a bogus `bid` fails loudly here rather
/// than silently storing data no pane will ever render.
fn buffer_text<'s>(
    state: &'s EditorState,
    bid: BufferId,
    builtin: &str,
) -> Result<&'s hume_editing::text::BufferText, String> {
    state
        .buffers
        .try_get(bid)
        .map(|b| b.text())
        .ok_or_else(|| format!("{builtin}: unknown buffer"))
}

/// `line`'s line-start char offset, or `Err` naming `builtin` if `line` is
/// out of range. Signs/virtual-lines/EOL-text/line-backgrounds keep their
/// Steel-facing `line` unit — this is the one place — already holding the
/// rope — where that converts to the internal char-offset position model.
///
/// Rejects the buffer's last *ropey* line, not just any out-of-range line:
/// the buffer invariant (every buffer ends with a structural `\n`) means
/// that last line is always the empty phantom line the trailing `\n`
/// produces — zero-width, at `pos == len_chars()`, nothing to decorate.
/// `DisplayLineMap::last_line()` never lays it out, so admitting it would hand a
/// caller a position no render pass can resolve to a real line.
pub(super) fn line_start_offset(
    text: &hume_editing::text::BufferText,
    line: usize,
    builtin: &str,
) -> Result<CharOffset, String> {
    let Some(line) = hume_rope::line::ContentLine::checked(text.rope(), line) else {
        return Err(format!(
            "{builtin}: line {line} is out of range (buffer has {} content lines)",
            text.content_line_count().get()
        ));
    };
    Ok(text.line_to_char(line.into()))
}

/// `pos` must address a real char in `text` (`<` its length) — `Err` naming
/// `builtin` otherwise. One past the last char looks tempting for an
/// `'after` hint at end-of-buffer, but there's no char there to anchor to:
/// `visible_char_range` is half-open, so `pos == len_chars()` can never pass
/// its `contains` check and the hint would silently never render — reject it
/// here instead, same as every other position-taking decoration kind.
///
/// `before == false` ('after') gets a second check: the render bridge
/// (`decoration_providers.rs`'s `update_inlay_hint_providers`) anchors an
/// 'after' hint at `pos + 1`, so a hint on the buffer's last content char
/// (its trailing structural `\n`) would resolve to the trailing phantom
/// line — same unresolvable position `line_start_offset` already refuses
/// for the line-anchored kinds, but reachable here through a char offset
/// instead of a line number, so that check alone doesn't catch it.
pub(super) fn validate_offset(
    text: &hume_editing::text::BufferText,
    pos: usize,
    before: bool,
    builtin: &str,
) -> Result<CharOffset, String> {
    if pos >= text.len_chars() {
        return Err(format!(
            "{builtin}: offset {pos} is out of range (buffer has {} chars)",
            text.len_chars()
        ));
    }
    if !before {
        // `pos < text.len_chars()` is checked above, so `pos` shifted by one
        // codepoint stays `<= len_chars()` — the one past-the-end position
        // `ropey_char_to_line` (ropey domain) accepts, needed to find which
        // line an 'after' hint at `pos + 1` actually lands on.
        let landing_line = text.ropey_char_to_line(CharOffset::new(pos).shift(1));
        if landing_line.to_content(text.rope()).is_none() {
            return Err(format!(
                "{builtin}: offset {pos} anchored 'after would land on the buffer's trailing \
                 empty line"
            ));
        }
    }
    // Trusted mint: both checks above already proved `pos` a valid char
    // position in `text`.
    Ok(CharOffset::new(pos))
}

/// `(start, end)` must be a valid, non-empty char range into `text` — `Err`
/// naming `builtin` otherwise.
fn validate_range(
    text: &hume_editing::text::BufferText,
    start: usize,
    end: usize,
    builtin: &str,
) -> Result<ExclusiveRange<CharOffset>, String> {
    if start >= end {
        return Err(format!(
            "{builtin}: range ({start}, {end}) must have start < end"
        ));
    }
    if end > text.len_chars() {
        return Err(format!(
            "{builtin}: range end {end} is past the buffer's char length {}",
            text.len_chars()
        ));
    }
    // Trusted mint: both checks above already proved `start`/`end` a valid,
    // non-empty char range in `text`.
    Ok(ExclusiveRange::new(
        CharOffset::new(start),
        CharOffset::new(end),
    ))
}
