//! Per-frame sync of highlight/sign/inlay-hint/virtual-line/EOL-text/
//! line-background decoration data from editor-authoritative stores to the
//! shared `Arc` buffers the engine's providers read during rendering. Driven
//! by `prepare_frame`'s step 3/5.

use std::sync::Arc;

use hume_engine::pipeline::{BufferId, PaneId};
use hume_engine::types::EditorMode;

use super::Editor;
use crate::editor::lsp::diagnostics::DiagSeverity;
use crate::lock_ext::LockExt;
use hume_editing::lines::{char_to_line_byte, line_break_char, line_segments};
use hume_ops::pair::find_bracket_pair;

impl Editor {
    /// Snapshot of every pane's `(PaneId, BufferId)` — the entry point every
    /// render bridge below starts with, so its loop body can freely mutate
    /// `self.state` (e.g. `update_highlight_providers` refreshing a
    /// buffer's search-match cache) without conflicting with a live borrow
    /// of `self.view.panes`.
    fn decorated_panes(&self) -> Vec<(PaneId, BufferId)> {
        self.view
            .panes
            .iter()
            .map(|(pid, pane)| (pid, pane.buffer_id))
            .collect()
    }

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

    /// Interned `ScopeId` for `ui.cursor.match` (bracket match highlight),
    /// cached the same way as [`Self::diagnostic_scopes`] — every pane's
    /// bracket-match `ScopedHighlighter` writes this into each span it
    /// pushes rather than carrying it fixed on the provider.
    fn bracket_match_scope(&mut self) -> hume_engine::types::ScopeId {
        if let Some(id) = self.state.bracket_match_scope {
            return id;
        }
        let id = self.view.registry.intern("ui.cursor.match");
        self.state.bracket_match_scope = Some(id);
        id
    }

    /// Interned `ScopeId` for `ui.selection.search` (search match
    /// highlight) — see [`Self::bracket_match_scope`].
    fn search_match_scope(&mut self) -> hume_engine::types::ScopeId {
        if let Some(id) = self.state.search_match_scope {
            return id;
        }
        let id = self.view.registry.intern("ui.selection.search");
        self.state.search_match_scope = Some(id);
        id
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
    /// buffers, read by that pane's `ScopedHighlighter` providers.
    ///
    /// Called once per frame, after scroll is resolved and before `term.draw`.
    /// Bracket matching is suppressed in Insert mode. Each pane's search
    /// highlights are computed from **that pane's own buffer and viewport** —
    /// panes never share highlight data (see [`crate::ui::highlight_providers::PaneHighlights`]),
    /// so a pane viewing a different buffer, or the same buffer scrolled
    /// elsewhere, never inherits another pane's matches.
    pub(super) fn update_highlight_providers(&mut self) {
        let in_insert = self.state.mode() == EditorMode::Insert;

        let panes = self.decorated_panes();
        let search_scope = self.search_match_scope();
        let bracket_scope = self.bracket_match_scope();

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

            let visible = self.visible_line_range(pid, bid);
            let buf = self.state.buffers.get(bid);
            let text = buf.text();

            // Matches are sorted by document order. Binary-search to the first
            // match that starts at or after this pane's `top_line`.
            let top_char = text.line_to_char(visible.start);
            let matches = &buf.search_matches.matches;
            let first = matches.partition_point(|&(start, _)| start < top_char);
            for &(start, end_incl) in &matches[first..] {
                let start_line = text.char_to_line(start);
                if start_line >= visible.end {
                    break;
                }
                // end_incl is inclusive char offset; +1 makes it exclusive.
                let end_char = (end_incl + 1).min(text.len_chars());
                push_match_highlight_lines(text, start, end_char, search_scope, &mut data);
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
                let text = self.doc().text();
                let head = self.state.panes.state[focused][self.focused_buffer_id()]
                    .selections
                    .primary()
                    .head();
                if let Some(ch) = text.char_at(head) {
                    let pair = match ch {
                        '(' | ')' => Some(('(', ')')),
                        '[' | ']' => Some(('[', ']')),
                        '{' | '}' => Some(('{', '}')),
                        '<' | '>' => Some(('<', '>')),
                        _ => None,
                    };
                    if let Some((open, close)) = pair
                        && let Some((op, cp)) = find_bracket_pair(text, head, open, close)
                    {
                        let match_pos = if head == op { cp } else { op };
                        let (line, byte) = char_to_line_byte(text, match_pos);
                        // Single-char match: byte_end = byte + utf8 length of the char.
                        let ch_len = text.char_at(match_pos).map(|c| c.len_utf8()).unwrap_or(1);
                        bracket_arc.write_or_panic().push((
                            line,
                            byte,
                            byte + ch_len,
                            bracket_scope,
                        ));
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
                        // order, which `extra_highlights_for_buffer`
                        // (`SourceStore::for_buffer`) yields ascending by
                        // source name — the alphabetically-first source wins,
                        // deterministic across sessions rather than whichever
                        // source happened to call `set-extra-highlights!` first.
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
    /// section. Called from `prepare_frame`'s step 3, *before* scrolling: the
    /// sign column's width feeds `Pane::content_width`, which decides the
    /// wrap column the scroll step's `RowMap` resolves against.
    pub(super) fn update_sign_providers(&mut self) {
        use hume_engine::builtins::sign_column::Sign;

        let panes = self.decorated_panes();

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
            // configured slot count decides how many signs per line the
            // plugin merge keeps (the rest is dropped before the map write).
            let signcolumn = self
                .state
                .buffers
                .get(bid)
                .overrides
                .signcolumn(&self.state.settings);
            let max_plugin_signs = signcolumn.slots as usize;

            // Both the diagnostics and plugin-sign passes below need to turn
            // a char offset back into a line — diagnostics store char ranges
            // directly (live server output, never validated against
            // `line_start_offset`, so can legitimately be EOF-anchored), and
            // plugin signs store their line's line-start char offset
            // (`SignEntry::pos`, remapped through edits like every other
            // decoration kind).
            let text = self.state.buffers.get(bid).text();
            // Diagnostics only: clamped to the last content char so an
            // EOF-anchored diagnostic (server points one past the buffer's
            // last char) still marks the buffer's last real line rather than
            // resolving to the trailing phantom line and vanishing.
            let last_content_char = text.len_chars().saturating_sub(1);

            // Diagnostics: every line a diagnostic touches gets a marker;
            // the most severe diagnostic wins when several touch one line.
            let diag_raw: Vec<(usize, usize, DiagSeverity)> = self
                .lsp
                .diagnostics_for_range(bid, visible.clone(), floor)
                .map(|d| {
                    (
                        text.char_to_line(d.start.min(last_content_char)),
                        text.char_to_line(d.end.saturating_sub(1).min(last_content_char)),
                        d.severity,
                    )
                })
                .collect();
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
            // input order, which `signs_for_buffer` (`SourceStore::for_buffer`)
            // already yields ascending by source name — no local re-sort
            // needed, not a second tie-break rule invented here. The only
            // other explicit priority-tie decision in the sign pipeline is
            // `SignColumn::render_row_cells`'s own sort
            // (hume-engine/src/builtins/sign_column.rs, arbitrates plugin vs
            // diagnostics map by source-registration order) — this sort
            // must stay priority-only so it never overrides that.
            let plugin_raw = visible_line_anchored(
                text,
                &visible_lines,
                self.state.config.decorations.signs_for_buffer(bid),
                |e| e.pos,
            );

            let mut plugin_all: rustc_hash::FxHashMap<usize, Vec<(String, String, i64)>> =
                rustc_hash::FxHashMap::default();
            for (_, line, e) in plugin_raw {
                plugin_all
                    .entry(line)
                    .or_default()
                    .push((e.text, e.scope, e.priority));
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
                // display-width-safe: SignColumnConfig::width is a slot count, not display width.
                crate::settings::SignColumnMode::Always => signcolumn.width(),
                crate::settings::SignColumnMode::Auto => {
                    if has_signs {
                        signcolumn.width() // display-width-safe: slot count, not display width
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
    /// `decorations.inlay_hints` store to each pane's `InlineDecorationProvider`
    /// Arc. Not gated on `lsp.inlay-hints` here: the store is per-source
    /// (`set-inlay-hints!` takes a `source` arg precisely so unrelated
    /// plugins can coexist), and `lsp.inlay-hints` is the LSP inlay-hints
    /// plugin's own setting — it owns clearing *its* source on toggle-off,
    /// via the `on-option-change` hook (`inlay.scm`), rather than this
    /// bridge wiping every source wholesale on a setting it doesn't own.
    pub(super) fn update_inlay_hint_providers(&mut self) {
        use hume_engine::providers::InlineInsert;

        let panes = self.decorated_panes();

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
            for entry in self
                .state
                .config
                .decorations
                .inlay_hints_in_range(bid, visible.clone())
            {
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
    /// store to each pane's second `InlineDecorationProvider` Arc
    /// (`PaneRenderHandles::eol_text`). Unconditional per-frame rebuild, same
    /// as `update_inlay_hint_providers` — cheap enough that, unlike
    /// `virtual_lines`, it doesn't need a dirty-tracking generation gate to
    /// skip needless work; filtered to the viewport before any per-entry
    /// clone or scope resolution runs, same as the sign/line-bg bridges
    /// above, so the per-frame cost is one entry per *visible* EOL line, not
    /// per EOL line in the whole buffer. Both write into a pane's
    /// `inline_decorations` providers, which `RowMap::format_line` reads, so
    /// this feeds wrap row counts and columns exactly like inlay hints do —
    /// called from `prepare_frame`'s step 3, *before* scrolling, so (like
    /// `update_sign_providers`) the viewport it filters against is still the
    /// previous frame's.
    pub(super) fn update_eol_text_providers(&mut self) {
        use hume_engine::providers::InlineInsert;

        let panes = self.decorated_panes();

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

            // `visible_line_anchored` ends the immutable `eol_text_for_buffer`
            // borrow (on `self.state.config.decorations`) before
            // `self.runtime_scope` below, which needs `&mut self`. Each
            // entry's `pos` is its line's line-start char offset
            // (`EolTextEntry::pos`); resolved to its *current* line here
            // (once), reused below rather than re-resolved.
            let visible_lines = self.visible_line_range(pid, bid);
            let text = self.state.buffers.get(bid).text();
            let filtered: Vec<(String, usize, crate::editor::decorations::EolTextEntry)> =
                visible_line_anchored(
                    text,
                    &visible_lines,
                    self.state.config.decorations.eol_text_for_buffer(bid),
                    |e| e.pos,
                );
            let resolved: Vec<(String, usize, String, hume_engine::types::ScopeId)> = filtered
                .into_iter()
                .map(|(source, line, e)| (source, line, e.text, self.runtime_scope(&e.scope)))
                .collect();

            let text = self.state.buffers.get(bid).text();
            let per_line: Vec<(String, usize, InlineInsert)> = resolved
                .into_iter()
                .map(|(source, line, entry_text, scope)| {
                    // End-of-line placement: the line's own trailing '\n'
                    // char resolves to a byte offset within `line` (never
                    // the next line — see `char_to_line_byte`'s doc comment
                    // on the same pattern used for inlay hints' `'after`
                    // anchor).
                    let line_newline = line_break_char(text, line);
                    let (_, byte_offset) = char_to_line_byte(text, line_newline);
                    (
                        source,
                        line,
                        InlineInsert {
                            byte_offset,
                            text: entry_text,
                            scope,
                        },
                    )
                })
                .collect();
            let by_line: rustc_hash::FxHashMap<usize, Vec<InlineInsert>> =
                last_writer_per_line(per_line)
                    .into_iter()
                    .map(|(line, insert)| (line, vec![insert]))
                    .collect();

            *map.write_or_panic() = by_line;
        }
    }

    /// Interned `ScopeId` for `ui.virtual` — the fallback `base_scope` for a
    /// virtual-line entry with no explicit `scope`, so a theme that gives
    /// `ui.virtual` a `bg` tints a scope-less virtual line's gutter and
    /// trailing cells the same as its text, not just the text (the row-fill
    /// site and the per-grapheme site must resolve the same scope for a
    /// line — see `update_virtual_line_providers`). Cached the same way as
    /// [`Self::inlay_hint_scope`].
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
    /// only rebuilds when `decorations.generation(bid)` changed since the
    /// pane's last sync, or the pane's buffer changed, since resolving each
    /// entry's scope (`runtime_scope`) is costlier to redo unconditionally
    /// every frame. The stamp is per-buffer (not a single store-wide
    /// counter): an edit only bumps the buffer it edited, so typing in one
    /// buffer no longer forces every pane on every *other* buffer to
    /// resync too. Called from `prepare_frame`'s step 3, *before* scrolling,
    /// no viewport dependency to make stale. Two sources anchored to the same
    /// line stack rather than collapse (unlike the four line-anchored kinds
    /// `last_writer_per_line` folds) — `virtual_lines_for_buffer`
    /// (`SourceStore::for_buffer`) yields sources ascending by name, and
    /// `RowMap::block`'s anchor sort is stable, so they render in
    /// alphabetical-by-source order, not registration order.
    ///
    /// Each entry becomes `Before(line)` or `After(line)` per its `before`
    /// flag. `entry.scope` (or `ui.virtual` when absent) resolves to
    /// `VirtualLine::base_scope` — the engine falls back to it for bytes
    /// `segments` doesn't cover, and reads its `bg` to fill the row past the
    /// last grapheme (see `segment_virtual_row`/`pane_render.rs`'s
    /// virtual-row `row_bg`). Always `Some`, never left as the engine's own
    /// `None` fallback, so a theme that puts a `bg` on `ui.virtual` reaches
    /// the row fill exactly the same way an explicit `scope` would.
    pub(super) fn update_virtual_line_providers(&mut self) {
        use hume_engine::providers::{VirtualLine, VirtualLineAnchor};

        let panes = self.decorated_panes();
        let fallback_scope = self.virtual_text_fallback_scope();

        for &(pid, bid) in &panes {
            let current_gen = self.state.config.decorations.generation(bid);
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
                let base_scope = Some(match &entry.scope {
                    Some(name) => self.runtime_scope(name),
                    None => fallback_scope,
                });
                let segments = entry
                    .segments
                    .iter()
                    .map(|(start, end, name)| (*start, *end, self.runtime_scope(name)))
                    .collect();
                // `entry.pos` is the anchor line's line-start char offset
                // (`VirtualLineEntry::pos`) — fetched fresh per entry since
                // `runtime_scope` above needs `&mut self`, which can't
                // overlap with a borrow of `self.state.buffers`.
                let text = self.state.buffers.get(bid).text();
                let Some(line) = resolve_decoration_line(text, entry.pos) else {
                    continue;
                };
                let anchor = if entry.before {
                    VirtualLineAnchor::Before(line)
                } else {
                    VirtualLineAnchor::After(line)
                };
                by_line.entry(line).or_default().push(VirtualLine {
                    anchor,
                    // Overwritten by the engine at collection time with the
                    // registration-assigned id (see `ProviderSet::add_decoration_source`).
                    provider_id: 0,
                    text: entry.text,
                    segments,
                    base_scope,
                });
            }

            *map.write_or_panic() = by_line;
            self.virtual_lines_synced.insert(pid, (bid, current_gen));
        }
    }

    /// Write per-frame line-background data to every pane's own
    /// `Arc<RwLock<FxHashMap<usize, ScopeId>>>` buffer, read by that pane's
    /// `PaneLineBackgrounds` provider. Rebuilds unconditionally each frame —
    /// unlike `virtual_lines`, the payload is filtered to the viewport
    /// before any per-entry clone or scope resolution runs (mirrors
    /// `update_sign_providers`), so the per-frame cost is one `ScopeId` per
    /// *visible* tinted line, not per tinted line in the whole buffer —
    /// cheap enough that a dedicated generation-gated sync buys nothing.
    pub(super) fn update_line_bg_providers(&mut self) {
        let panes = self.decorated_panes();

        for &(pid, bid) in &panes {
            let Some(map) = self
                .state
                .panes
                .render
                .get(pid)
                .map(|r| Arc::clone(&r.line_backgrounds))
            else {
                continue;
            };

            // `visible_line_anchored` ends the immutable
            // `line_backgrounds_for_buffer` borrow before `self.runtime_scope`
            // below, which needs `&mut self` — like the sign path above
            // (`update_sign_providers`): a tinted line scrolled out of view
            // costs nothing but the filter check.
            let visible_lines = self.visible_line_range(pid, bid);
            let text = self.state.buffers.get(bid).text();
            let filtered: Vec<(String, usize, crate::editor::decorations::LineBgEntry)> =
                visible_line_anchored(
                    text,
                    &visible_lines,
                    self.state
                        .config
                        .decorations
                        .line_backgrounds_for_buffer(bid),
                    |e| e.pos,
                );
            let per_line: Vec<(String, usize, hume_engine::types::ScopeId)> = filtered
                .into_iter()
                .map(|(source, line, e)| (source, line, self.runtime_scope(&e.scope)))
                .collect();
            let by_line = last_writer_per_line(per_line);

            *map.write_or_panic() = by_line;
        }
    }
}

/// Folds per-source, line-anchored decoration entries into one winner per
/// line: within one source, a later entry beats an earlier
/// one that a remap collapsed onto the same line (ties resolve by store
/// order — `SourceStore::set` sorts by position, so "later" means originally
/// further along the buffer); across sources, tie-break by source name,
/// mirroring the sign pipeline (`SourceStore::set` keeps sources ascending by
/// name; `update_sign_providers`'s *stable* priority sort over that input
/// leaves same-priority ties in source order — i.e. the alphabetically first
/// source wins).
///
/// A single stable sort **descending** by source name gets both properties
/// from one `FxHashMap::insert`-per-entry fold: one source's own entries
/// keep their relative (store) order, so its later entry overwrites its
/// earlier one when folded left-to-right; and the alphabetically-first
/// source sorts last, so its entries are folded last and win the
/// cross-source overwrite.
fn last_writer_per_line<T>(
    mut entries: Vec<(String, usize, T)>,
) -> rustc_hash::FxHashMap<usize, T> {
    entries.sort_by(|a, b| b.0.cmp(&a.0));
    entries.into_iter().map(|(_, line, v)| (line, v)).collect()
}

/// Resolves a stored line-anchored decoration's position to its current
/// line, or `None` if a remap drifted it onto the buffer's trailing phantom
/// line — always empty (every buffer ends with a structural `\n`,
/// `text.last_ropey_line()`), the same line `host_impl.rs`'s
/// `line_start_offset` refuses to hand out a position on in the first place.
/// A fresh `set-*!` call can never produce this; a `remap_points` result can,
/// when an edit deletes everything after the entry's anchor up to
/// end-of-buffer. The entry disappears rather than getting relocated onto
/// whatever line precedes it (four callers: signs, EOL text, virtual lines,
/// line backgrounds — all four line-anchored decoration kinds).
fn resolve_decoration_line(text: &hume_editing::text::BufferText, pos: usize) -> Option<usize> {
    let line = text.char_to_line(pos);
    text.content_lines_range().contains(&line).then_some(line)
}

/// Filters `entries`' `(source, entry)` pairs to `visible_lines`, resolving
/// each entry's anchor position (via `pos_of`) to its current line and
/// cloning past the borrow — shared by `update_eol_text_providers` and
/// `update_line_bg_providers`, whose bodies are otherwise identical up to
/// this filter step: resolve line → drop if scrolled out of view or drifted
/// onto the phantom trailing line (`resolve_decoration_line`) → own the
/// result. Returns fully-owned `(source, line, entry)` triples, not
/// `&entry`, so the immutable borrow on the decoration store `entries` came
/// from ends here — every caller needs that borrow gone before its next
/// step (`self.runtime_scope`), which needs `&mut self`.
fn visible_line_anchored<'a, E: Clone + 'a>(
    text: &hume_editing::text::BufferText,
    visible_lines: &std::ops::Range<usize>,
    entries: impl Iterator<Item = (&'a str, &'a E)>,
    pos_of: impl Fn(&E) -> usize,
) -> Vec<(String, usize, E)> {
    entries
        .filter_map(|(source, e)| {
            let line = resolve_decoration_line(text, pos_of(e))?;
            visible_lines
                .contains(&line)
                .then(|| (source.to_string(), line, e.clone()))
        })
        .collect()
}

/// Push one `(line, byte_start, byte_end, scope)` quadruple per line the
/// `[start, end_char_excl)` char range touches, all sharing `scope` — search
/// matches are the one caller, always one fixed scope per call. See
/// [`line_segments`].
fn push_match_highlight_lines(
    text: &hume_editing::text::BufferText,
    start: usize,
    end_char_excl: usize,
    scope: hume_engine::types::ScopeId,
    data: &mut Vec<(usize, usize, usize, hume_engine::types::ScopeId)>,
) {
    if start >= end_char_excl {
        return;
    }
    data.extend(line_segments(text, start, end_char_excl).map(|(l, s, e)| (l, s, e, scope)));
}

/// Push one `(line, byte_start, byte_end, priority, scope)` quintuple per
/// line the `[start, end_char_excl)` char range touches. See
/// [`line_segments`]; `priority` and `scope` are carried through unchanged
/// for [`flatten_priority_overlaps`] to resolve same-line overlaps from
/// (lower `priority` wins — see that function).
fn push_priority_highlight_lines(
    text: &hume_editing::text::BufferText,
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
        line_segments(text, start, end_char_excl).map(|(l, s, e)| (l, s, e, priority, scope)),
    );
}

/// Flattens overlapping same-line `(start, end, priority, scope)` spans
/// (already split per-line by [`push_priority_highlight_lines`]) into the
/// sorted, non-overlapping sequence the engine's `Decoration::Highlight`
/// contract requires — a single source's own output must not overlap itself
/// (cross-tier layering, e.g. diagnostics vs. search matches, is
/// handled automatically by the engine's per-tier `HighlightStack`; this
/// only resolves overlaps *within* one tier, e.g. two diagnostics on the
/// same line). One line's worth of spans at a time through
/// [`hume_engine::interval_sweep::flatten_overlapping_spans`] — the same
/// event-sweep `hume-treesitter/src/highlight.rs` uses for nested injection
/// layers, generic over both crates now instead of a second hand-rolled
/// copy. `Reverse<priority>` makes "lower priority number wins" read as
/// "highest rank wins" with no inversion arithmetic;
/// `TieBreak::FirstPushed` matches this function's original contract —
/// same-priority ties keep whichever span was pushed to `raw` first, pinned
/// by `overlapping_extra_highlights_from_two_sources_resolve_alphabetically`
/// (`raw`'s push order comes from `SourceStore::for_buffer`'s ascending
/// source-name order). `raw` need not be pre-sorted; drained (left empty)
/// on return.
fn flatten_priority_overlaps(
    raw: &mut Vec<(usize, usize, usize, u8, hume_engine::types::ScopeId)>,
    out: &mut Vec<(usize, usize, usize, hume_engine::types::ScopeId)>,
) {
    use hume_engine::interval_sweep::{TieBreak, flatten_overlapping_spans};
    use std::cmp::Reverse;

    if raw.is_empty() {
        return;
    }
    raw.sort_by_key(|&(line, start, _, _, _)| (line, start));

    let mut group: Vec<(usize, usize, Reverse<u8>, hume_engine::types::ScopeId)> = Vec::new();
    let mut stack = Vec::new();
    let mut events = Vec::new();
    let mut line_out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let line = raw[i].0;
        let mut j = i;
        group.clear();
        while j < raw.len() && raw[j].0 == line {
            let (_, start, end, priority, scope) = raw[j];
            group.push((start, end, Reverse(priority), scope));
            j += 1;
        }
        flatten_overlapping_spans(
            &mut group,
            &mut stack,
            &mut events,
            &mut line_out,
            TieBreak::FirstPushed,
        );
        out.extend(
            line_out
                .drain(..)
                .map(|(start, end, scope)| (line, start, end, scope)),
        );
        i = j;
    }
    raw.clear();
}

#[cfg(test)]
mod tests;
