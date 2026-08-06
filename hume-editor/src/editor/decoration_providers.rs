//! Per-frame sync of highlight/sign/inlay-hint/virtual-line/EOL-text
//! decoration data from editor-authoritative stores to the shared `Arc`
//! buffers the engine's providers read during rendering. Driven by
//! `prepare_frame`'s step 5/7.

use std::sync::Arc;

use hume_engine::pipeline::{BufferId, PaneId};
use hume_engine::types::EditorMode;

use super::Editor;
use crate::editor::lsp::diagnostics::DiagSeverity;
use crate::lock_ext::LockExt;
use hume_editing::lines::line_end_exclusive;
use hume_ops::pair::find_bracket_pair;

impl Editor {
    /// Interned scope ids for the four diagnostic severities, in
    /// `DiagSeverity` discriminant order (`[error, warning, info, hint]`) —
    /// resolved once and cached, since interning needs `&mut
    /// self.view.registry` but `DiagSeverity` itself lives in `self.state`.
    fn diagnostic_scopes(&mut self) -> [hume_engine::types::ScopeId; 4] {
        if let Some(scopes) = self.state.diagnostic_scopes {
            return scopes;
        }
        let scopes = [
            self.view.registry.intern("diagnostic.error"),
            self.view.registry.intern("diagnostic.warning"),
            self.view.registry.intern("diagnostic.info"),
            self.view.registry.intern("diagnostic.hint"),
        ];
        self.state.diagnostic_scopes = Some(scopes);
        scopes
    }

    /// Interned `ScopeId` for a plugin-supplied runtime scope name (extra
    /// highlights, signs, virtual lines), cached across frames so the same
    /// name string is never re-interned.
    fn runtime_scope(&mut self, name: &str) -> hume_engine::types::ScopeId {
        if let Some(&id) = self.state.runtime_scope_cache.get(name) {
            return id;
        }
        let id = self.view.registry.intern_runtime(name);
        self.state.runtime_scope_cache.insert(name.to_string(), id);
        id
    }

    /// Write per-frame highlight data to every pane's own `Arc<RwLock<...>>`
    /// buffers, read by that pane's `SharedHighlighter` providers.
    ///
    /// Called once per frame, after scroll is resolved and before `term.draw`.
    /// Bracket matching is suppressed in Insert mode. Each pane's search
    /// highlights are computed from **that pane's own buffer and viewport** —
    /// panes never share highlight data (see [`crate::ui::highlight_providers::PaneHighlights`]),
    /// so a pane viewing a different buffer, or the same buffer scrolled
    /// elsewhere, never inherits another pane's matches.
    pub(super) fn update_highlight_providers(&mut self) {
        let in_insert = self.state.mode() == EditorMode::Insert;

        // Snapshot (pane, buffer) pairs up front: the loop body mutates
        // `self.state.buffers` (refreshing the search-match cache), which would
        // otherwise conflict with an active borrow of `self.view.panes`.
        let panes: Vec<(PaneId, BufferId)> = self
            .view
            .panes
            .iter()
            .map(|(pid, pane)| (pid, pane.buffer_id))
            .collect();

        // ── Search match highlights — one pane at a time ─────────────────────
        for &(pid, bid) in &panes {
            // Clone the Arc (not the data) so the write lock and the buffer
            // refresh below don't hold a borrow of `self.state.panes`.
            let Some(search_arc) = self
                .state
                .panes
                .render
                .get(pid)
                .map(|r| Arc::clone(&r.highlights.search))
            else {
                continue;
            };
            let mut data = search_arc.write_or_panic();
            data.clear();
            // Hidden in Insert mode — matches aren't actionable while typing and
            // clutter the view. Same pattern as bracket match highlights below.
            if in_insert {
                continue;
            }

            // Keep this buffer's match cache current regardless of focus — a
            // non-focused pane's buffer may carry its own active search
            // pattern that the focused-pane-only `sync_search_cache` never
            // refreshes. No-op when the cache already matches this revision.
            super::search::ops::update_buffer_matches(&mut self.state.buffers, bid);

            let buf = self.state.buffers.get(bid);
            let text = buf.text();
            let vp = &self.view.panes[pid].viewport;
            let top_line = vp.top_line;
            let bot_line = top_line + vp.height as usize;

            // Matches are sorted by document order. Binary-search to the first
            // match that starts at or after this pane's `top_line`.
            let top_char = text.line_to_char(top_line.min(text.len_lines().saturating_sub(1)));
            let matches = &buf.search_matches.matches;
            let first = matches.partition_point(|&(start, _)| start < top_char);
            for &(start, end_incl) in &matches[first..] {
                let start_line = text.char_to_line(start);
                if start_line > bot_line {
                    break;
                }
                // end_incl is inclusive char offset; +1 makes it exclusive.
                let end_char = (end_incl + 1).min(text.len_chars());
                push_match_highlight_lines(text, start, end_char, &mut data);
            }
        }

        // ── Bracket match highlight — cursor concept, focused pane only ──────
        // Clear every pane first: a bracket match lingers only on whichever
        // pane last had focus, so moving focus away must blank the old one.
        for &(pid, _) in &panes {
            if let Some(r) = self.state.panes.render.get(pid) {
                r.highlights.bracket.write_or_panic().clear();
            }
        }
        if !in_insert {
            let focused = self.state.focused_pane_id;
            if let Some(bracket_arc) = self
                .state
                .panes
                .render
                .get(focused)
                .map(|r| Arc::clone(&r.highlights.bracket))
            {
                let buf = self.doc().text();
                let head = self.state.panes.state[focused][self.focused_buffer_id()]
                    .selections
                    .primary()
                    .head();
                if let Some(ch) = buf.char_at(head) {
                    let pair = match ch {
                        '(' | ')' => Some(('(', ')')),
                        '[' | ']' => Some(('[', ']')),
                        '{' | '}' => Some(('{', '}')),
                        '<' | '>' => Some(('<', '>')),
                        _ => None,
                    };
                    if let Some((open, close)) = pair
                        && let Some((op, cp)) = find_bracket_pair(buf, head, open, close)
                    {
                        let match_pos = if head == op { cp } else { op };
                        let (line, byte) = char_to_line_byte(buf, match_pos);
                        // Single-char match: byte_end = byte + utf8 length of the char.
                        let ch_len = buf.char_at(match_pos).map(|c| c.len_utf8()).unwrap_or(1);
                        bracket_arc
                            .write_or_panic()
                            .push((line, byte, byte + ch_len));
                    }
                }
            }
        }

        // ── Diagnostic + extra highlights — every pane ───────────────────────
        // Unlike search/bracket-match highlights, these stay visible in
        // Insert mode: an error squiggle is exactly as relevant while you're
        // editing the line it's on (most editors keep them showing).
        {
            let floor = self.state.settings.lsp_diagnostics_severity_floor;
            let diag_scopes = self.diagnostic_scopes();
            for &(pid, bid) in &panes {
                let Some((diag_arc, extra_arc)) = self.state.panes.render.get(pid).map(|r| {
                    (
                        Arc::clone(&r.highlights.diagnostics),
                        Arc::clone(&r.highlights.extra),
                    )
                }) else {
                    continue;
                };

                let visible = self.visible_char_range(pid, bid);

                // Collect raw diagnostic ranges first — this ends the
                // immutable borrow of `self.lsp` before `runtime_scope`
                // (called below for extra highlights) needs `&mut self`.
                let diags: Vec<(usize, usize, DiagSeverity)> = self
                    .lsp
                    .diagnostics_for_range(bid, visible.clone(), floor)
                    .map(|d| {
                        (
                            d.start.max(visible.start),
                            d.end.min(visible.end),
                            d.severity,
                        )
                    })
                    .collect();

                // Same for extra highlights: collect owned data before
                // resolving each source's scope name to a `ScopeId`.
                let extra_raw: Vec<(usize, usize, String)> = self
                    .state
                    .config
                    .decorations
                    .extra_highlights_for_buffer(bid)
                    .filter(|e| e.start < visible.end && e.end > visible.start)
                    .map(|e| {
                        (
                            e.start.max(visible.start),
                            e.end.min(visible.end),
                            e.scope.clone(),
                        )
                    })
                    .collect();
                let extra: Vec<(usize, usize, hume_engine::types::ScopeId)> = extra_raw
                    .into_iter()
                    .map(|(start, end, name)| (start, end, self.runtime_scope(&name)))
                    .collect();

                let buf = self.state.buffers.get(bid);
                let text = buf.text();

                {
                    let mut raw = Vec::new();
                    for (start, end, severity) in diags {
                        // Priority = severity discriminant: Error(0) beats
                        // Warning(1) beats Info(2) beats Hint(3) in overlaps.
                        push_priority_highlight_lines(
                            text,
                            start,
                            end,
                            severity as u8,
                            diag_scopes[severity as usize],
                            &mut raw,
                        );
                    }
                    let mut data = diag_arc.write_or_panic();
                    data.clear();
                    flatten_priority_overlaps(&mut raw, &mut data);
                }

                {
                    let mut raw = Vec::new();
                    for (start, end, scope) in extra {
                        // No severity concept for plugin-supplied spans —
                        // uniform priority; overlap ties resolve by push
                        // order (first source registered wins).
                        push_priority_highlight_lines(text, start, end, 0, scope, &mut raw);
                    }
                    let mut data = extra_arc.write_or_panic();
                    data.clear();
                    flatten_priority_overlaps(&mut raw, &mut data);
                }
            }
        }
    }

    /// Write per-frame gutter sign data (diagnostics + plugin signs) to every
    /// pane's own `Arc<RwLock<FxHashMap<line, Vec<Sign>>>>` buffers, read by
    /// that pane's `SharedSignSource`s. Stays visible in Insert mode — same
    /// reasoning as [`Self::update_highlight_providers`]'s diagnostics
    /// section. Called from `prepare_frame`'s step 5, *before* scrolling: the
    /// sign column's width feeds `Pane::content_width`, which decides the
    /// wrap column the scroll step's `RowMap` resolves against.
    pub(super) fn update_sign_providers(&mut self) {
        use hume_engine::builtins::sign_column::Sign;

        let panes: Vec<(PaneId, BufferId)> = self
            .view
            .panes
            .iter()
            .map(|(pid, pane)| (pid, pane.buffer_id))
            .collect();

        let floor = self.state.settings.lsp_diagnostics_severity_floor;
        let diag_scopes = self.diagnostic_scopes();
        for &(pid, bid) in &panes {
            let Some((diag_map, plugin_map)) = self.state.panes.render.get(pid).map(|r| {
                (
                    Arc::clone(&r.signs.diagnostics),
                    Arc::clone(&r.signs.plugin),
                )
            }) else {
                continue;
            };

            let visible = self.visible_char_range(pid, bid);
            let visible_lines = self.visible_line_range(pid, bid);

            // Compute the buffer's `signcolumn` setting up front — the
            // configured column count decides how many signs per line the
            // plugin merge keeps (the rest is dropped before the map write).
            let signcolumn = self
                .state
                .buffers
                .get(bid)
                .overrides
                .signcolumn(&self.state.settings);
            let max_plugin_signs = signcolumn.columns as usize;

            // Diagnostics: every line a diagnostic touches gets a marker;
            // the most severe diagnostic wins when several touch one line.
            // Clamped to the buffer's last valid char (same defense the
            // highlight path above takes against a stored diagnostic whose
            // offsets have drifted past the current text) — `char_to_line`
            // panics on an out-of-bounds char index.
            let diag_raw: Vec<(usize, usize, DiagSeverity)> = {
                let text = self.state.buffers.get(bid).text();
                let last_char = text.len_chars().saturating_sub(1);
                self.lsp
                    .diagnostics_for_range(bid, visible.clone(), floor)
                    .map(|d| {
                        (
                            text.char_to_line(d.start.min(last_char)),
                            text.char_to_line(d.end.saturating_sub(1).min(last_char)),
                            d.severity,
                        )
                    })
                    .collect()
            };
            let mut diag_best: rustc_hash::FxHashMap<usize, DiagSeverity> =
                rustc_hash::FxHashMap::default();
            for (start_line, end_line, severity) in diag_raw {
                for line in start_line..=end_line {
                    if !visible_lines.contains(&line) {
                        continue;
                    }
                    diag_best
                        .entry(line)
                        .and_modify(|best| {
                            if severity < *best {
                                *best = severity;
                            }
                        })
                        .or_insert(severity);
                }
            }
            {
                let mut guard = diag_map.write_or_panic();
                guard.clear();
                for (line, severity) in diag_best {
                    guard.insert(
                        line,
                        vec![Sign {
                            text: std::borrow::Cow::Borrowed("●"),
                            scope: diag_scopes[severity as usize],
                            priority: 10,
                        }],
                    );
                }
            }

            // Plugin signs (`set-signs!`): top N signs per line by priority,
            // where N = the buffer's configured `signcolumn` columns.
            // Pre-truncating to N here (rather than passing everything
            // through downstream) bounds memory — an unbounded per-line Vec
            // would get cloned every frame by
            // `SharedSignSource::signs_for_line`. Safe only because the sort
            // below is priority-only: same-priority ties resolve by the
            // input order `plugin_raw.sort_by` set just above (source name,
            // ascending), not a second tie-break rule invented here. The
            // only other explicit priority-tie decision in the sign
            // pipeline is `SignColumn::render_row_cells`'s own sort
            // (hume-engine/src/builtins/sign_column.rs, arbitrates plugin vs
            // diagnostics map by source-registration order) — this sort
            // must stay priority-only so it never overrides that.
            let mut plugin_raw: Vec<(String, usize, String, String, i64)> = self
                .state
                .config
                .decorations
                .signs_for_buffer(bid)
                .filter(|(_, e)| visible_lines.contains(&e.line))
                .map(|(source, e)| {
                    (
                        source.to_string(),
                        e.line,
                        e.text.clone(),
                        e.scope.clone(),
                        e.priority,
                    )
                })
                .collect();
            plugin_raw.sort_by(|a, b| a.0.cmp(&b.0));

            let mut plugin_all: rustc_hash::FxHashMap<usize, Vec<(String, String, i64)>> =
                rustc_hash::FxHashMap::default();
            for (_, line, text, scope, priority) in plugin_raw {
                plugin_all
                    .entry(line)
                    .or_default()
                    .push((text, scope, priority));
            }
            {
                let mut guard = plugin_map.write_or_panic();
                guard.clear();
                for (line, mut entries) in plugin_all {
                    entries.sort_by_key(|e| std::cmp::Reverse(e.2));
                    entries.truncate(max_plugin_signs);
                    let signs: Vec<Sign> = entries
                        .into_iter()
                        .map(|(text, scope_name, priority)| {
                            let scope = self.runtime_scope(&scope_name);
                            Sign {
                                text: std::borrow::Cow::Owned(text),
                                scope,
                                priority: priority.clamp(i16::MIN as i64, i16::MAX as i64) as i16,
                            }
                        })
                        .collect();
                    guard.insert(line, signs);
                }
            }

            // Compute sign column width from the buffer's `signcolumn` setting:
            // `always` keeps it visible at the configured width; `auto` collapses
            // to zero when no signs are visible in the current viewport (diag_map/
            // plugin_map above only hold visible-line entries — a sign elsewhere
            // in the buffer, scrolled out of view, does not keep the column open).
            let has_signs = {
                let diag_empty = diag_map.read_or_panic().is_empty();
                let plugin_empty = plugin_map.read_or_panic().is_empty();
                !(diag_empty && plugin_empty)
            };
            let width = match signcolumn.mode {
                crate::settings::SignColumnMode::Always => signcolumn.width(),
                crate::settings::SignColumnMode::Auto => {
                    if has_signs {
                        signcolumn.width()
                    } else {
                        0
                    }
                }
            };
            self.view.panes[pid].providers.sync_sign_column_width(width);
        }
    }

    /// Interned `ScopeId` for `ui.virtual.inlay-hint`, cached across frames —
    /// every inlay hint shares this one scope (locked decision: no per-hint
    /// styling in v1), unlike `runtime_scope`'s plugin-name-keyed cache.
    fn inlay_hint_scope(&mut self) -> hume_engine::types::ScopeId {
        if let Some(id) = self.state.inlay_hint_scope {
            return id;
        }
        let id = self.view.registry.intern("ui.virtual.inlay-hint");
        self.state.inlay_hint_scope = Some(id);
        id
    }

    /// Sync per-pane inlay-hint decorations from the
    /// `decorations.inlay_hints` store to each pane's `InlayHintProvider`
    /// Arc. Gated on `lsp.inlay-hints`: when off, every pane's map is
    /// cleared so a mid-session toggle takes effect immediately rather than
    /// waiting for the store to next change.
    pub(super) fn update_inlay_hint_providers(&mut self) {
        use hume_engine::providers::InlineInsert;

        let panes: Vec<(PaneId, BufferId)> = self
            .view
            .panes
            .iter()
            .map(|(pid, pane)| (pid, pane.buffer_id))
            .collect();

        if !self.state.settings.lsp_inlay_hints {
            for &(pid, _) in &panes {
                if let Some(r) = self.state.panes.render.get(pid) {
                    r.inlay_hints.write_or_panic().clear();
                }
            }
            return;
        }

        let scope = self.inlay_hint_scope();
        for &(pid, bid) in &panes {
            let Some(map) = self
                .state
                .panes
                .render
                .get(pid)
                .map(|r| Arc::clone(&r.inlay_hints))
            else {
                continue;
            };
            let visible = self.visible_char_range(pid, bid);
            let text = self.state.buffers.get(bid).text();

            let mut by_line: rustc_hash::FxHashMap<usize, Vec<InlineInsert>> =
                rustc_hash::FxHashMap::default();
            for entry in self.state.config.decorations.inlay_hints_for_buffer(bid) {
                if !visible.contains(&entry.pos) {
                    continue;
                }
                // `before`: byte offset of the char at `pos` itself, so the
                // hint text is spliced in immediately before it. `after`:
                // the next char boundary, so it's spliced in immediately
                // after — `char_to_line_byte` resolves a trailing `\n`'s
                // own position to the *same* line (ropey's line boundaries
                // include their `\n`), so a hint at end-of-line-content
                // never bleeds onto the following line.
                let (line, byte_offset) = if entry.before {
                    char_to_line_byte(text, entry.pos)
                } else {
                    char_to_line_byte(text, entry.pos + 1)
                };
                by_line.entry(line).or_default().push(InlineInsert {
                    byte_offset,
                    text: entry.text.clone(),
                    scope,
                });
            }

            *map.write_or_panic() = by_line;
        }
    }

    /// Sync per-pane EOL-text decorations from the `decorations.eol_text`
    /// store to each pane's second `InlayHintProvider` Arc
    /// (`PaneRenderHandles::eol_text`). Unconditional per-frame rebuild, same
    /// as `update_inlay_hint_providers` — cheap enough that, unlike
    /// `virtual_lines`, it doesn't need a dirty-tracking generation gate to
    /// skip needless work. Both write into a pane's `inline_decorations`
    /// providers, which `RowMap::format_line` reads, so this feeds wrap row
    /// counts and columns exactly like inlay hints do — called from
    /// `prepare_frame`'s step 5, *before* scrolling.
    pub(super) fn update_eol_text_providers(&mut self) {
        use hume_engine::providers::InlineInsert;

        let panes: Vec<(PaneId, BufferId)> = self
            .view
            .panes
            .iter()
            .map(|(pid, pane)| (pid, pane.buffer_id))
            .collect();

        for &(pid, bid) in &panes {
            let Some(map) = self
                .state
                .panes
                .render
                .get(pid)
                .map(|r| Arc::clone(&r.eol_text))
            else {
                continue;
            };

            // Collected into an owned Vec, and every scope name resolved,
            // *before* borrowing buffer text below: `self.runtime_scope`
            // needs `&mut self`, which can't overlap with either the
            // immutable borrow `eol_text_for_buffer` holds on
            // `self.state.config.decorations` or the one `text` will hold on
            // `self.state.buffers`.
            let entries: Vec<(usize, String, String)> = self
                .state
                .config
                .decorations
                .eol_text_for_buffer(bid)
                .map(|e| (e.line, e.text.clone(), e.scope.clone()))
                .collect();
            let resolved: Vec<(usize, String, hume_engine::types::ScopeId)> = entries
                .into_iter()
                .map(|(line, text, scope_name)| (line, text, self.runtime_scope(&scope_name)))
                .collect();

            let text = self.state.buffers.get(bid).text();
            let mut by_line: rustc_hash::FxHashMap<usize, Vec<InlineInsert>> =
                rustc_hash::FxHashMap::default();
            for (line, entry_text, scope) in resolved {
                // End-of-line placement: the line's own trailing '\n' char
                // resolves to a byte offset within `line` (never the next
                // line — see `char_to_line_byte`'s doc comment on the same
                // pattern used for inlay hints' `'after` anchor).
                let line_newline = line_end_exclusive(text, line) - 1;
                let (_, byte_offset) = char_to_line_byte(text, line_newline);
                by_line.entry(line).or_default().push(InlineInsert {
                    byte_offset,
                    text: entry_text,
                    scope,
                });
            }

            *map.write_or_panic() = by_line;
        }
    }

    /// Interned `ScopeId` for `ui.virtual` — the same theme key
    /// `Theme::ui.virtual_text` (the struct field) resolves from — used as
    /// the fallback scope for a virtual-line entry with no explicit
    /// `scope`. Cached the same way as [`Self::inlay_hint_scope`].
    fn virtual_text_fallback_scope(&mut self) -> hume_engine::types::ScopeId {
        if let Some(id) = self.state.virtual_text_fallback_scope {
            return id;
        }
        let id = self.view.registry.intern("ui.virtual");
        self.state.virtual_text_fallback_scope = Some(id);
        id
    }

    /// Sync per-pane virtual-line decorations from the
    /// `decorations.virtual_lines` store to each pane's `PaneVirtualLines`
    /// Arc — a `RowMap::block` provider, so this feeds row *counts* the same
    /// way inlay hints/EOL text feed wrap columns. Unlike those two, this
    /// only rebuilds when `decorations.generation()` changed since the
    /// pane's last sync, or the pane's buffer changed, since resolving each
    /// entry's scope (`runtime_scope`) is costlier to redo unconditionally
    /// every frame. Called from `prepare_frame`'s step 5, *before*
    /// scrolling, no viewport dependency to make stale.
    ///
    /// Each entry becomes `Before(line)` or `After(line)` per its `before`
    /// flag, and its `segments` are gap-filled with its base scope so the
    /// engine always sees full byte coverage — see `gap_fill_segments`.
    pub(super) fn update_virtual_line_providers(&mut self) {
        use hume_engine::providers::{VirtualLine, VirtualLineAnchor};

        let current_gen = self.state.config.decorations.generation();
        let panes: Vec<(PaneId, BufferId)> = self
            .view
            .panes
            .iter()
            .map(|(pid, pane)| (pid, pane.buffer_id))
            .collect();

        let fallback_scope = self.virtual_text_fallback_scope();
        for &(pid, bid) in &panes {
            if self.virtual_lines_synced.get(&pid) == Some(&(bid, current_gen)) {
                continue;
            }
            let Some(map) = self
                .state
                .panes
                .render
                .get(pid)
                .map(|r| Arc::clone(&r.virtual_lines))
            else {
                continue;
            };

            // Collected into an owned Vec first: `self.runtime_scope` needs
            // `&mut self`, which can't overlap with the immutable borrow
            // `virtual_lines_for_buffer` holds on `self.state.config.decorations`.
            let entries: Vec<crate::editor::decorations::VirtualLineEntry> = self
                .state
                .config
                .decorations
                .virtual_lines_for_buffer(bid)
                .cloned()
                .collect();

            let mut by_line: rustc_hash::FxHashMap<usize, Vec<VirtualLine>> =
                rustc_hash::FxHashMap::default();
            for entry in entries {
                let base = match &entry.scope {
                    Some(name) => self.runtime_scope(name),
                    None => fallback_scope,
                };
                let segments = self.gap_fill_segments(&entry.segments, entry.text.len(), base);
                let anchor = if entry.before {
                    VirtualLineAnchor::Before(entry.line)
                } else {
                    VirtualLineAnchor::After(entry.line)
                };
                by_line.entry(entry.line).or_default().push(VirtualLine {
                    anchor,
                    // Overwritten by the engine at collection time with the
                    // registration-assigned id (see `ProviderSet::add_virtual_line_source`).
                    provider_id: 0,
                    text: entry.text,
                    segments,
                });
            }

            *map.write_or_panic() = by_line;
            self.virtual_lines_synced.insert(pid, (bid, current_gen));
        }
    }

    /// Fills the gaps `segments` (already sorted, non-overlapping, in-bounds —
    /// guaranteed by the host boundary, `virtual_line_segments_to_bytes` in
    /// `host_impl.rs`) leaves in `0..text_len`
    /// with `base`, so the engine always receives full byte coverage instead
    /// of falling back to `ui.virtual_text` per uncovered byte. No segments →
    /// exactly one segment spanning the whole text, matching the
    /// pre-segments behavior byte-for-byte.
    ///
    /// A non-sorted or overlapping `segments` here is a caller bug, not a
    /// runtime condition to tolerate: `RowMap::block` only *sorts* the
    /// output by `start` (it does not merge or reject overlaps), so
    /// silently emitting overlapping ranges here would still reach
    /// `IntervalCursor`, whose non-overlap precondition it violates —
    /// producing a wrong-scope render rather than an error.
    fn gap_fill_segments(
        &mut self,
        segments: &[(usize, usize, String)],
        text_len: usize,
        base: hume_engine::types::ScopeId,
    ) -> Vec<(usize, usize, hume_engine::types::ScopeId)> {
        let mut out = Vec::with_capacity(segments.len() * 2 + 1);
        let mut cursor = 0usize;
        for (start, end, name) in segments {
            debug_assert!(
                *start >= cursor,
                "gap_fill_segments: caller-guaranteed sorted/non-overlapping segments violated"
            );
            let scope = self.runtime_scope(name);
            if *start > cursor {
                out.push((cursor, *start, base));
            }
            out.push((*start, *end, scope));
            cursor = *end;
        }
        if cursor < text_len {
            out.push((cursor, text_len, base));
        }
        out
    }
}

/// Convert a char-offset position to a line-relative byte offset.
///
/// Returns `(line_idx, byte_in_line)` where `byte_in_line` is the byte offset
/// from the start of the line — suitable for building highlight spans that the
/// engine expects in line-relative byte coordinates.
fn char_to_line_byte(buf: &hume_editing::text::Text, char_pos: usize) -> (usize, usize) {
    let line = buf.char_to_line(char_pos);
    let line_start_byte = buf.char_to_byte(buf.line_to_char(line));
    let byte = buf.char_to_byte(char_pos).saturating_sub(line_start_byte);
    (line, byte)
}

/// Yield `(line, byte_start, byte_end)` for each line the *non-empty*
/// `[start, end_char_excl)` char range touches, clipped to that line's own
/// content (up to but excluding its trailing `\n`). Caller must check
/// `start < end_char_excl` first.
///
/// A single-line range yields one triple, byte-identical to converting
/// `start`/`end_char_excl` directly with [`char_to_line_byte`]. A multi-line
/// range yields one triple per touched line. The clip point is deliberately
/// the `\n` char's own position, not `line_end_exclusive` — the latter is
/// the *next* line's start, which `char_to_line_byte` would resolve to the
/// wrong line (byte 0 of the line after).
///
/// Shared by [`push_match_highlight_lines`] (search/bracket matches, one
/// scope per provider) and [`push_priority_highlight_lines`]
/// (diagnostics/extra highlights, one scope + priority per range) — same
/// per-line splitting math, only the tuple shape differs.
fn line_segments(
    buf: &hume_editing::text::Text,
    start: usize,
    end_char_excl: usize,
) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
    let last_char = end_char_excl - 1;
    let start_line = buf.char_to_line(start);
    let end_line = buf.char_to_line(last_char);
    (start_line..=end_line).map(move |line| {
        // Every content line ends with a '\n' — HUME buffers always end with
        // a structural trailing '\n', so this position always exists and
        // still belongs to `line` in ropey's line model.
        let line_newline = line_end_exclusive(buf, line) - 1;
        let seg_start = start.max(buf.line_to_char(line));
        let seg_end = end_char_excl.min(line_newline);
        let (_, byte_start) = char_to_line_byte(buf, seg_start);
        let (_, byte_end) = char_to_line_byte(buf, seg_end);
        (line, byte_start, byte_end)
    })
}

/// Push one `(line, byte_start, byte_end)` triple per line the
/// `[start, end_char_excl)` char range touches. See [`line_segments`].
fn push_match_highlight_lines(
    buf: &hume_editing::text::Text,
    start: usize,
    end_char_excl: usize,
    data: &mut Vec<(usize, usize, usize)>,
) {
    if start >= end_char_excl {
        return;
    }
    data.extend(line_segments(buf, start, end_char_excl));
}

/// Push one `(line, byte_start, byte_end, priority, scope)` quintuple per
/// line the `[start, end_char_excl)` char range touches. See
/// [`line_segments`]; `priority` and `scope` are carried through unchanged
/// for [`flatten_priority_overlaps`] to resolve same-line overlaps from
/// (lower `priority` wins — see that function).
fn push_priority_highlight_lines(
    buf: &hume_editing::text::Text,
    start: usize,
    end_char_excl: usize,
    priority: u8,
    scope: hume_engine::types::ScopeId,
    data: &mut Vec<(usize, usize, usize, u8, hume_engine::types::ScopeId)>,
) {
    if start >= end_char_excl {
        return;
    }
    data.extend(
        line_segments(buf, start, end_char_excl).map(|(l, s, e)| (l, s, e, priority, scope)),
    );
}

/// Flattens overlapping same-line `(start, end, priority, scope)` spans
/// (already split per-line by [`push_priority_highlight_lines`]) into the
/// sorted, non-overlapping sequence the engine's `HighlightSource` contract
/// requires — a single `HighlightSource`'s own output must not overlap
/// itself (cross-tier layering, e.g. diagnostics vs. search matches, is
/// handled automatically by the engine's per-tier `HighlightStack`; this
/// only resolves overlaps *within* one tier, e.g. two diagnostics on the
/// same line). Lower `priority` wins overlapping regions (ties keep
/// whichever was pushed first) — same event-sweep shape as
/// `flatten_overlaps` in `hume-treesitter/src/highlight.rs`
/// (nested tree-sitter injection layers), adapted for scope-carrying
/// diagnostic/extra-highlight spans instead of syntax layers. `raw` need
/// not be pre-sorted; drained (left empty) on return.
fn flatten_priority_overlaps(
    raw: &mut Vec<(usize, usize, usize, u8, hume_engine::types::ScopeId)>,
    out: &mut Vec<(usize, usize, usize, hume_engine::types::ScopeId)>,
) {
    if raw.is_empty() {
        return;
    }
    raw.sort_by_key(|&(line, start, _, _, _)| (line, start));

    let mut i = 0;
    while i < raw.len() {
        let line = raw[i].0;
        let mut j = i;
        while j < raw.len() && raw[j].0 == line {
            j += 1;
        }
        flatten_one_line(&raw[i..j], line, out);
        i = j;
    }
    raw.clear();
}

/// One line's worth of `(_, start, end, priority, scope)` spans (the `line`
/// field is ignored — the caller already grouped by it) → flattened,
/// non-overlapping `(line, start, end, scope)` output. See
/// [`flatten_priority_overlaps`].
fn flatten_one_line(
    group: &[(usize, usize, usize, u8, hume_engine::types::ScopeId)],
    line: usize,
    out: &mut Vec<(usize, usize, usize, hume_engine::types::ScopeId)>,
) {
    if group.len() == 1 {
        let (_, start, end, _, scope) = group[0];
        out.push((line, start, end, scope));
        return;
    }

    // Event sweep: (pos, is_end, seq, priority, scope). `seq` is the span's
    // index within `group`, used to pop the exact matching stack entry.
    // End events sort before start events at the same position so a
    // closing span is popped before a new one at the same byte is pushed.
    let mut events: Vec<(usize, bool, u32, u8, hume_engine::types::ScopeId)> =
        Vec::with_capacity(group.len() * 2);
    for (seq, &(_, start, end, priority, scope)) in group.iter().enumerate() {
        let seq = seq as u32;
        events.push((start, false, seq, priority, scope));
        events.push((end, true, seq, priority, scope));
    }
    events.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    // Sorted ascending by (priority, seq) — the lowest-priority (highest-
    // severity) active span is always at `stack[0]`.
    let mut stack: Vec<(u8, u32, hume_engine::types::ScopeId)> = Vec::new();
    let mut pos = 0usize;
    for &(event_pos, is_end, seq, priority, scope) in &events {
        if let Some(&(_, _, active_scope)) = stack.first()
            && pos < event_pos
        {
            out.push((line, pos, event_pos, active_scope));
        }
        pos = event_pos;

        if is_end {
            if let Some(idx) = stack
                .iter()
                .position(|&(p, s, _)| p == priority && s == seq)
            {
                stack.remove(idx);
            }
        } else {
            let insert_at = stack.partition_point(|&(p, s, _)| (p, s) < (priority, seq));
            stack.insert(insert_at, (priority, seq, scope));
        }
    }
}
