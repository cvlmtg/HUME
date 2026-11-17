//! `EditorHostImpl`'s inlay hints, signs, virtual lines, extra highlights,
//! conceals, EOL text, statusline text, and the diagnostic pull/count reads.
//!
//! `validate_offset` is `pub(super)` because `host_impl/tests.rs`, a
//! sibling of this module, calls it.

use std::collections::BTreeMap;
use std::ops::Bound;

use hume_core::{BufferId, DiagSeverity, Side};
use hume_decorations::{ConcealEntry, ConcealEntrySpan};
use hume_editing::text::BufferText;
use hume_engine::providers::{ConcealText, conceal_ranges_overlap};
use hume_engine::theme::ScopeRegistry;
use hume_engine::types::ScopeId;
use hume_rope::line::ContentLine;
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::editor::EditorState;
use crate::editor::buffer::offsets::{checked_char_offset, checked_cluster_range};
use crate::statusline::StatusElement;

use super::EditorHostImpl;
use hume_scripting::host::{DecorationHost, DiagnosticCounts, MarkedLines, SetError, each_entry};
use hume_scripting::{ConcealOverlaps, ConcealSpec, ConcealSpecSpan, ConcealSpecText, ScopedSpans};

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
                text: hint_text.into(),
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

    fn unregister_sign_source(&mut self, source: &str, bid: BufferId) {
        self.state
            .config
            .decorations
            .unregister_sign_source(source, bid);
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
        let registry = &mut self.view.registry;
        let signs = each_entry(signs, |(lines, sign_text, scope)| {
            let scope = script_scope(registry, &scope)?;
            Ok((lines, (std::sync::Arc::<str>::from(sign_text), scope)))
        })?;
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
        let registry = &mut self.view.registry;
        let entries = each_entry(lines, |(line, spec)| {
            let scope = match spec.scope.as_deref() {
                Some(name) => script_scope(registry, name)?,
                None => registry.intern(hume_engine::theme::ui_scopes::VIRTUAL_TEXT),
            };
            Ok(hume_decorations::VirtualLineEntry {
                line,
                content: spec
                    .content
                    .try_map_payload(|name| script_scope(registry, &name))?,
                side: spec.side,
                scope,
            })
        })?;
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
        spans: ScopedSpans,
    ) -> Result<(), SetError> {
        let text = buffer_text(self.state, bid).map_err(SetError::Call)?;
        let entries = scoped_highlight_entries(text, spans, &mut self.view.registry)?;
        self.state
            .config
            .decorations
            .set_extra_highlights(source, bid, entries);
        Ok(())
    }

    fn set_conceals(
        &mut self,
        source: String,
        bid: BufferId,
        entries: Vec<ConcealSpec>,
        overlaps: ConcealOverlaps,
    ) -> Result<(), SetError> {
        let text = buffer_text(self.state, bid).map_err(SetError::Call)?;
        let entries = conceal_entries(text, &mut self.view.registry, entries, overlaps)?;
        self.state
            .config
            .decorations
            .set_conceals(source, bid, entries);
        Ok(())
    }

    fn set_eol_text(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<(ContentLine, String, String)>,
        hide_on_insert_line: bool,
    ) -> Result<(), SetError> {
        let registry = &mut self.view.registry;
        let entries = each_entry(lines, |(line, eol_text, scope)| {
            Ok(hume_decorations::EolTextEntry {
                line,
                text: eol_text.into(),
                scope: script_scope(registry, &scope)?,
                hide_on_insert_line,
            })
        })?;
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
        let entries = line_background_entries(&mut self.view.registry, entries)?;
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

impl EditorHostImpl<'_> {
    /// Adds `spans` to `source`'s extra highlights for `bid`, their ranges
    /// checked as [`DecorationHost::set_extra_highlights`] checks them. Their
    /// scopes are HUME's own names, which the registry never refuses.
    pub(in crate::editor) fn add_extra_highlights(
        &mut self,
        source: String,
        bid: BufferId,
        spans: Vec<(usize, usize, &'static str)>,
    ) -> Result<(), SetError> {
        let text = buffer_text(self.state, bid).map_err(SetError::Call)?;
        let registry = &mut self.view.registry;
        let entries = extra_highlight_entries(text, spans, |name| Ok(registry.intern(name)))?;
        self.state
            .config
            .decorations
            .add_extra_highlights(source, bid, entries);
        Ok(())
    }
}

/// The id of a scope a script or a text format names, interned in
/// `registry`: a new name is refused once the session holds
/// [`ScopeRegistry::NAME_LIMIT`] names.
pub(in crate::editor) fn script_scope(
    registry: &mut ScopeRegistry,
    name: &str,
) -> Result<ScopeId, String> {
    registry.intern_name(name).map_err(|e| e.to_string())
}

/// `specs` as conceals of `text`, in position order, their scopes interned
/// in `registry` through [`script_scope`]. Each entry's range is checked
/// and widened to whole clusters and must stay on one line without covering
/// its `\n`; an `Inserted` may sit before a line's `\n` but not at the
/// text's end. An entry overlapping another ([`conceal_ranges_overlap`]) is
/// refused or left out, as `overlaps` says.
pub(in crate::editor) fn conceal_entries(
    text: &BufferText,
    registry: &mut ScopeRegistry,
    specs: Vec<ConcealSpec>,
    overlaps: ConcealOverlaps,
) -> Result<Vec<ConcealEntry>, SetError> {
    let placed = each_entry(specs.into_iter().enumerate().collect(), |(index, spec)| {
        let mut shown = |t: ConcealSpecText| -> Result<ConcealText, String> {
            Ok(ConcealText {
                text: t.text.into(),
                scope: t
                    .scope
                    .map(|name| script_scope(registry, &name))
                    .transpose()?,
            })
        };
        let fail = |e: String| format!("range {e}");
        let span = match spec.span {
            ConcealSpecSpan::Hidden {
                start,
                end,
                text: replacement,
            } => ConcealEntrySpan::Hidden {
                chars: checked_cluster_range(text, start, end)
                    .map_err(fail)?
                    .chars(),
                text: replacement.map(&mut shown).transpose()?,
            },
            ConcealSpecSpan::Inserted { at, text: drawn } => {
                let at = checked_char_offset(text, at).map_err(fail)?;
                if at == text.end() {
                    return Err(format!("offset {} is past the last line break", at.index()));
                }
                ConcealEntrySpan::Inserted {
                    at: text.snap(at).offset(),
                    text: shown(drawn)?,
                }
            }
        };
        let reveal = spec.reveal.map(|marked| {
            let lines = marked.span();
            text.lines().span(lines.start, lines.end).chars()
        });
        let entry = ConcealEntry { span, reveal };
        if !entry.within_line(text) {
            let chars = entry.chars();
            return Err(fail(format!(
                "{}..{} must stay within one line and not cover its newline",
                chars.start.index(),
                chars.end.index()
            )));
        }
        Ok((index, entry))
    })?;
    let mut kept: BTreeMap<(CharOffset, CharOffset), (usize, ConcealEntry)> = BTreeMap::new();
    for (index, entry) in placed {
        let chars = entry.chars();
        let key = (chars.start, chars.end);
        // Kept entries are disjoint, so their order by start is their
        // order by end too, and an entry overlapping any kept one
        // overlaps the kept entry just before or just after its own
        // place in that order.
        let before = kept.range(..=key).next_back();
        let after = kept.range((Bound::Excluded(key), Bound::Unbounded)).next();
        let earlier = before
            .into_iter()
            .chain(after)
            .find(|(_, (_, k))| conceal_ranges_overlap(k.chars(), chars))
            .map(|(_, (earlier, _))| *earlier);
        match (earlier, overlaps) {
            (None, _) => {
                kept.insert(key, (index, entry));
            }
            (Some(_), ConcealOverlaps::DropLater) => {}
            (Some(earlier), ConcealOverlaps::Refuse) => {
                return Err(SetError::Entry {
                    index,
                    message: format!("overlaps entry {earlier}"),
                });
            }
        }
    }
    Ok(kept.into_values().map(|(_, entry)| entry).collect())
}

/// Each `(start, end, scope)` char range of `text` as an extra highlight,
/// its scope's id given by `scope_id`. Errors on a range outside the text,
/// an empty one, or a scope `scope_id` refuses.
fn extra_highlight_entries<S>(
    text: &BufferText,
    spans: Vec<(usize, usize, S)>,
    mut scope_id: impl FnMut(S) -> Result<ScopeId, String>,
) -> Result<Vec<hume_decorations::ExtraHighlightEntry>, SetError> {
    each_entry(spans, |(start, end, scope)| {
        let fail = |e: String| format!("range {e}");
        // Whole clusters: the renderer paints a cluster at a time.
        let range = checked_cluster_range(text, start, end)
            .map_err(fail)?
            .chars();
        Ok(hume_decorations::ExtraHighlightEntry {
            start: range.start,
            end: range.end,
            scope: scope_id(scope)?,
        })
    })
}

/// [`extra_highlight_entries`] of spans that name their scope by its place in
/// `spans.scopes`, each scope interned through [`script_scope`] when a span
/// first uses it, so a scope after a failing entry is never interned.
pub(in crate::editor) fn scoped_highlight_entries(
    text: &BufferText,
    spans: ScopedSpans,
    registry: &mut ScopeRegistry,
) -> Result<Vec<hume_decorations::ExtraHighlightEntry>, SetError> {
    let ScopedSpans { scopes, spans } = spans;
    let mut ids: Vec<Option<ScopeId>> = vec![None; scopes.len()];
    extra_highlight_entries(text, spans, |place| {
        if let Some(id) = ids[place] {
            return Ok(id);
        }
        let id = script_scope(registry, &scopes[place])?;
        ids[place] = Some(id);
        Ok(id)
    })
}

/// One line background per line each entry marks, its scope interned in
/// `registry` through [`script_scope`].
pub(in crate::editor) fn line_background_entries(
    registry: &mut ScopeRegistry,
    entries: Vec<(MarkedLines, String)>,
) -> Result<Vec<hume_decorations::LineBgEntry>, SetError> {
    let entries = each_entry(entries, |(lines, scope)| {
        Ok((lines, script_scope(registry, &scope)?))
    })?;
    Ok(each_line(entries)
        .map(|(line, scope)| hume_decorations::LineBgEntry { line, scope })
        .collect())
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
