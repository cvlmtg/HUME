//! Per-frame sync of highlight/sign/inlay-hint/virtual-line/EOL-text/
//! line-background decoration data from editor-authoritative stores to the
//! shared `Arc` buffers the engine's providers read during rendering. Driven
//! by `prepare_frame`'s step 3/5.

use std::ops::Range;
use std::sync::Arc;

use hume_engine::pipeline::{BufferId, PaneId};
use hume_engine::types::EditorMode;

use super::Editor;
use crate::lock_ext::LockExt;
use hume_editing::lines::{char_to_line_byte, line_break_char, line_segments};
use hume_ops::pair::find_bracket_pair;

/// One pane's identity plus its on-screen slice, as of the moment
/// [`Editor::decorated_panes`] was called — every render bridge below reads
/// `lines`/`chars` instead of asking the viewport directly, so a bridge can
/// only ever see the viewport its group was handed (see `decorated_panes`'s
/// doc for why that matters across the scroll step).
pub(super) struct DecoratedPane {
    pub(super) pid: PaneId,
    pub(super) bid: BufferId,
    /// Line range, end-exclusive — used by line-indexed stores (gutter
    /// signs, EOL text, line backgrounds) instead of `chars`.
    pub(super) lines: Range<usize>,
    /// Char range of `bid`'s content currently visible in `pid` — shared by
    /// every per-frame write side that pulls a bounded slice from a
    /// Rust-side store (diagnostics, decorations) instead of the whole
    /// buffer.
    pub(super) chars: Range<usize>,
}

impl Editor {
    /// Snapshot of every pane's identity and visible slice — the entry point
    /// every render bridge below starts with, so its loop body can freely
    /// mutate `self.state` (e.g. `update_highlight_providers` refreshing a
    /// buffer's search-match cache) without conflicting with a live borrow
    /// of `self.view.panes`, and so a bridge's `lines`/`chars` can never
    /// drift from what every other bridge in the same call sees.
    ///
    /// `prepare_frame` calls this **twice** — once before the scroll step,
    /// once after — rather than sharing one snapshot across both groups: the
    /// sign/inlay-hint/virtual-line/EOL-text bridges deliberately read the
    /// *previous* frame's viewport (their output decides row counts/columns
    /// the scroll step's `RowMap` resolves against, so it must already be
    /// visible before that math runs), while the highlight/line-background
    /// bridges are paint-only and want the *current* one. Two snapshots make
    /// that straddle a property of *where* each is taken, not something each
    /// of the six bridges has to individually get right.
    ///
    /// `lines`/`chars` are a one-row *superset* of the viewport (its end plus
    /// one more row) — cheap over-fetch beats a wrap-aware exact bound for a
    /// bulk store slice. Both are end-exclusive; `lsp::introspect::pane_visible_range`
    /// is also end-exclusive, so the two conventions differ only in that
    /// one-row slack, not in inclusive-vs-exclusive — they still don't share
    /// an implementation, since one clamps to `content_lines` and the other
    /// to the ropey-domain line count.
    pub(super) fn decorated_panes(&self) -> Vec<DecoratedPane> {
        self.view
            .panes
            .iter()
            .map(|(pid, pane)| {
                let bid = pane.buffer_id;
                let vp = &pane.viewport;
                let bottom = vp.top_line + vp.height as usize;
                let text = self.state.buffers.get(bid).text();
                let top_line = vp.top_line.min(text.last_ropey_line());
                DecoratedPane {
                    pid,
                    bid,
                    lines: top_line..(bottom + 1).min(text.ropey_line_count()),
                    chars: text.line_to_char(top_line)
                        ..hume_editing::lines::line_end_exclusive(text, bottom),
                }
            })
            .collect()
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
    pub(super) fn update_highlight_providers(&mut self, panes: &[DecoratedPane]) {
        let in_insert = self.state.mode() == EditorMode::Insert;

        let search_scope = self.view.registry.intern("ui.selection.search");
        let bracket_scope = self.view.registry.intern("ui.cursor.match");

        // ── Search match highlights — one pane at a time ─────────────────────
        for p in panes {
            let (pid, bid) = (p.pid, p.bid);
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

            let visible = p.lines.clone();
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
        for p in panes {
            if let Some(r) = self.state.panes.render.get(p.pid) {
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
            // Editing-area scope per diagnostic severity, in `DiagSeverity`
            // discriminant order (`[error, warning, info, hint]`) — indexed
            // below by `d.severity as usize`. The gutter counterpart (the
            // bare `error`/`warning`/`info`/`hint` name) is interned by
            // `core:lsp`'s `set-signs!` call, at the Steel boundary in
            // `host_impl.rs`, the same as any other plugin sign source's
            // scope.
            let diag_scopes = [
                "diagnostic.error",
                "diagnostic.warning",
                "diagnostic.info",
                "diagnostic.hint",
            ]
            .map(|name| self.view.registry.intern(name));
            for p in panes {
                let (pid, bid) = (p.pid, p.bid);
                let Some((diag_arc, extra_arc)) = self.state.panes.render.get(pid).map(|r| {
                    (
                        Arc::clone(&r.highlights.diagnostics),
                        Arc::clone(&r.highlights.extra),
                    )
                }) else {
                    continue;
                };

                let visible = p.chars.clone();

                let buf = self.state.buffers.get(bid);
                let text = buf.text();

                {
                    let mut raw = Vec::new();
                    for d in self.lsp.diagnostics_for_range(bid, visible.clone(), floor) {
                        let start = d.start.max(visible.start);
                        let end = d.end.min(visible.end);
                        // Priority = severity discriminant: Error(0) beats
                        // Warning(1) beats Info(2) beats Hint(3) in overlaps.
                        push_priority_highlight_lines(
                            text,
                            start,
                            end,
                            d.severity as u8,
                            diag_scopes[d.severity as usize],
                            &mut raw,
                        );
                    }
                    let mut data = diag_arc.write_or_panic();
                    data.clear();
                    flatten_priority_overlaps(&mut raw, &mut data);
                }

                {
                    let mut raw = Vec::new();
                    for e in self
                        .state
                        .config
                        .decorations
                        .extra_highlights_for_buffer(bid)
                    {
                        if e.start >= visible.end || e.end <= visible.start {
                            continue;
                        }
                        let start = e.start.max(visible.start);
                        let end = e.end.min(visible.end);
                        // No severity concept for plugin-supplied spans —
                        // uniform priority; overlap ties resolve by push
                        // order, which `extra_highlights_for_buffer`
                        // (`SourceStore::for_buffer`) yields ascending by
                        // source name — the alphabetically-first source wins,
                        // deterministic across sessions rather than whichever
                        // source happened to call `set-extra-highlights!` first.
                        push_priority_highlight_lines(text, start, end, 0, e.scope, &mut raw);
                    }
                    let mut data = extra_arc.write_or_panic();
                    data.clear();
                    flatten_priority_overlaps(&mut raw, &mut data);
                }
            }
        }
    }

    /// Write per-frame gutter sign data (`set-signs!`, all sources
    /// pre-merged at write time — diagnostics included, via `core:lsp`'s own
    /// `"lsp-diagnostics"` source) to every pane's own
    /// `Arc<RwLock<FxHashMap<line, Vec<Sign>>>>` buffer, read by that pane's
    /// `SharedSignSource`. Stays visible in Insert mode — same reasoning as
    /// [`Self::update_highlight_providers`]'s diagnostics section. Called
    /// from `prepare_frame`'s step 3, against the pre-scroll snapshot (see
    /// [`Self::decorated_panes`]) because the sign column's width feeds
    /// `Pane::content_width`, which decides the wrap column the scroll
    /// step's `RowMap` resolves against.
    pub(super) fn update_sign_providers(&mut self, panes: &[DecoratedPane]) {
        use hume_engine::builtins::sign_column::{Sign, SignColumn};

        for p in panes {
            let (pid, bid) = (p.pid, p.bid);
            let Some(sign_map) = self
                .state
                .panes
                .render
                .get(pid)
                .map(|r| Arc::clone(&r.signs))
            else {
                continue;
            };

            let visible = p.chars.clone();
            let visible_lines = p.lines.clone();

            let signcolumn = self
                .state
                .buffers
                .get(bid)
                .overrides
                .signcolumn(&self.state.settings);

            // A registered source's slot is its rank in this buffer's
            // registry (`DecorationStores::sign_sources`) — fixed the moment
            // it registers for `bid`, independent of what's actually placed
            // this frame. `slots_for` never walks a single sign.
            let slots = signcolumn.slots_for(self.state.config.decorations.sign_source_count(bid));

            // Plugin signs store their line's line-start char offset
            // (`SignEntry::pos`, remapped through edits like every other
            // decoration kind) — this is what turns it back into a line.
            let text = self.state.buffers.get(bid).text();

            // `signs_in_range` pre-filters by char range so this pass never
            // touches a sign the viewport can't show, and already resolves
            // each entry's source to its registered slot
            // (`DecorationStores::signs_in_range`) — this loop places, it
            // never looks a source up. `visible_line_anchored` still does
            // the precise per-line check (a char range can straddle a line
            // the viewport itself excludes).
            let anchored = visible_line_anchored(
                text,
                visible_lines,
                self.state.config.decorations.signs_in_range(bid, visible),
                |e| e.pos,
            );

            let mut by_line: rustc_hash::FxHashMap<usize, Vec<Sign>> =
                rustc_hash::FxHashMap::default();
            for (slot, line, e) in anchored {
                if slot >= slots as usize {
                    continue;
                }
                let slot = slot as u8;
                let entries = by_line.entry(line).or_default();
                // Two *different* sources can never contend for one slot
                // now — a slot is a source's fixed registry rank. This
                // guards only a same-source duplicate on one line (a source
                // bug: nothing stops one `set-signs!` call from listing two
                // entries for the same line): `Err` is the insertion point,
                // `Ok` means this source's own earlier (pos-sorted) entry
                // for the line already claimed the slot.
                if let Err(i) = entries.binary_search_by_key(&slot, |s| s.slot) {
                    entries.insert(
                        i,
                        Sign {
                            text: Arc::clone(&e.text),
                            scope: e.scope,
                            slot,
                        },
                    );
                }
            }

            // Compute sign column width from the buffer's `signcolumn` setting:
            // `always` keeps it visible at the resolved width (`slots + 1`);
            // `auto` collapses to zero when no signs are visible in the current
            // viewport (`by_line` above only holds visible-line entries — a
            // sign elsewhere in the buffer, scrolled out of view, does not
            // keep the column open). Checked before the move below — `by_line`
            // already has the answer, no need to re-lock `sign_map` to ask it.
            let has_signs = !by_line.is_empty();
            *sign_map.write_or_panic() = by_line;

            let width = match signcolumn.mode {
                crate::settings::SignColumnMode::Auto if !has_signs => 0,
                _ => SignColumn::width_for_slots(slots),
            };
            self.view.panes[pid].providers.sync_sign_column_width(width);
        }
    }

    /// Sync per-pane inlay-hint decorations from the
    /// `decorations.inlay_hints` store to each pane's `InlineDecorationProvider`
    /// Arc. Not gated on `lsp.inlay-hints` here: the store is per-source
    /// (`set-inlay-hints!` takes a `source` arg precisely so unrelated
    /// plugins can coexist), and `lsp.inlay-hints` is the LSP inlay-hints
    /// plugin's own setting — it owns clearing *its* source on toggle-off,
    /// via the `on-option-change` hook (`inlay.scm`), rather than this
    /// bridge wiping every source wholesale on a setting it doesn't own.
    pub(super) fn update_inlay_hint_providers(&mut self, panes: &[DecoratedPane]) {
        use hume_engine::providers::InlineInsert;

        // Every inlay hint shares this one scope (locked decision: no
        // per-hint styling in v1).
        let scope = self.view.registry.intern("ui.virtual.inlay-hint");
        for p in panes {
            let (pid, bid) = (p.pid, p.bid);
            let Some(map) = self
                .state
                .panes
                .render
                .get(pid)
                .map(|r| Arc::clone(&r.inlay_hints))
            else {
                continue;
            };
            let visible = p.chars.clone();
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
    /// called from `prepare_frame`'s step 3, against the pre-scroll snapshot
    /// (see [`Self::decorated_panes`]).
    pub(super) fn update_eol_text_providers(&mut self, panes: &[DecoratedPane]) {
        use hume_engine::providers::InlineInsert;

        for p in panes {
            let (pid, bid) = (p.pid, p.bid);
            let Some(map) = self
                .state
                .panes
                .render
                .get(pid)
                .map(|r| Arc::clone(&r.eol_text))
            else {
                continue;
            };

            // Each entry's `pos` is its line's line-start char offset
            // (`EolTextEntry::pos`); resolved to its *current* line here.
            let visible_lines = p.lines.clone();
            let text = self.state.buffers.get(bid).text();
            let per_line: Vec<(&str, usize, InlineInsert)> = visible_line_anchored(
                text,
                visible_lines,
                self.state.config.decorations.eol_text_for_buffer(bid),
                |e| e.pos,
            )
            .map(|(source, line, e)| {
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
                        text: e.text.clone(),
                        scope: e.scope,
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

    /// Sync per-pane virtual-line decorations from the
    /// `decorations.virtual_lines` store to each pane's `PaneVirtualLines`
    /// Arc — a `RowMap::block` provider, so this feeds row *counts* the same
    /// way inlay hints/EOL text feed wrap columns. Unlike those two, this
    /// only rebuilds when `decorations.generation(bid)` changed since the
    /// pane's last sync, or the pane's buffer changed — a whole-buffer
    /// rebuild (not viewport-filtered, since `RowMap::block` needs every
    /// anchor regardless of scroll position) with a `text`/`segments` clone
    /// per entry is costlier to redo unconditionally every frame than the
    /// other bridges' viewport-filtered passes. The stamp is per-buffer (not
    /// a single store-wide counter): an edit only bumps the buffer it
    /// edited, so typing in one buffer no longer forces every pane on every
    /// *other* buffer to resync too. Called from `prepare_frame`'s step 3 —
    /// unlike the rest of that step, has no viewport dependency (so which
    /// [`Self::decorated_panes`] snapshot it reads is immaterial) and takes
    /// only `pid`/`bid` from it. Two sources
    /// anchored to the same line stack rather than collapse (unlike the
    /// four line-anchored kinds `last_writer_per_line` folds) —
    /// `virtual_lines_for_buffer` (`SourceStore::for_buffer`) yields sources
    /// ascending by name, and `RowMap::block`'s anchor sort is stable, so
    /// they render in alphabetical-by-source order, not registration order.
    ///
    /// Each entry becomes `Before(line)` or `After(line)` per its `before`
    /// flag. `entry.scope` — already resolved to the `ui.virtual` fallback
    /// at the `set-virtual-lines!` boundary when the Steel call passed none
    /// (`host_impl.rs`) — becomes `VirtualLine::base_scope`: the engine
    /// falls back to it for bytes `segments` doesn't cover, and reads its
    /// `bg` to fill the row past the last grapheme (see
    /// `segment_virtual_row`/`pane_render.rs`'s virtual-row `row_bg`).
    /// Always `Some`, never left as the engine's own `None` fallback, so a
    /// theme that puts a `bg` on `ui.virtual` reaches the row fill exactly
    /// the same way an explicit `scope` would.
    pub(super) fn update_virtual_line_providers(&mut self, panes: &[DecoratedPane]) {
        use hume_engine::providers::{VirtualLine, VirtualLineAnchor};

        for p in panes {
            let (pid, bid) = (p.pid, p.bid);
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

            let text = self.state.buffers.get(bid).text();
            let mut by_line: rustc_hash::FxHashMap<usize, Vec<VirtualLine>> =
                rustc_hash::FxHashMap::default();
            for entry in self.state.config.decorations.virtual_lines_for_buffer(bid) {
                // `entry.pos` is the anchor line's line-start char offset
                // (`VirtualLineEntry::pos`).
                let Some(line) = resolve_decoration_line(text, entry.pos) else {
                    continue;
                };
                let anchor = if entry.before {
                    VirtualLineAnchor::Before(line)
                } else {
                    VirtualLineAnchor::After(line)
                };
                let segments = entry
                    .segments
                    .iter()
                    .map(|(start, end, scope)| (*start, *end, *scope))
                    .collect();
                by_line.entry(line).or_default().push(VirtualLine {
                    anchor,
                    // Overwritten by the engine at collection time with the
                    // registration-assigned id (see `ProviderSet::add_decoration_source`).
                    provider_id: 0,
                    text: entry.text.clone(),
                    segments,
                    base_scope: Some(entry.scope),
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
    pub(super) fn update_line_bg_providers(&mut self, panes: &[DecoratedPane]) {
        for p in panes {
            let (pid, bid) = (p.pid, p.bid);
            let Some(map) = self
                .state
                .panes
                .render
                .get(pid)
                .map(|r| Arc::clone(&r.line_backgrounds))
            else {
                continue;
            };

            // Like the sign path above (`update_sign_providers`): a tinted
            // line scrolled out of view costs nothing but the filter check.
            let visible_lines = p.lines.clone();
            let text = self.state.buffers.get(bid).text();
            let per_line: Vec<(&str, usize, hume_engine::types::ScopeId)> = visible_line_anchored(
                text,
                visible_lines,
                self.state
                    .config
                    .decorations
                    .line_backgrounds_for_buffer(bid),
                |e| e.pos,
            )
            .map(|(source, line, e)| (source, line, e.scope))
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
/// further along the buffer); across sources, tie-break by source name —
/// the alphabetically first source wins, same convention `SourceStore::set`
/// itself keeps sources ascending by name. Signs use a different mechanism:
/// each registered source has its own fixed gutter slot, so two sign
/// sources never contend for one line's cell the way EOL text/line
/// backgrounds do here.
///
/// A single stable sort **descending** by source name gets both properties
/// from one `FxHashMap::insert`-per-entry fold: one source's own entries
/// keep their relative (store) order, so its later entry overwrites its
/// earlier one when folded left-to-right; and the alphabetically-first
/// source sorts last, so its entries are folded last and win the
/// cross-source overwrite.
fn last_writer_per_line<T>(mut entries: Vec<(&str, usize, T)>) -> rustc_hash::FxHashMap<usize, T> {
    entries.sort_by(|a, b| b.0.cmp(a.0));
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

/// Filters `entries`' `(tag, entry)` pairs to `visible_lines`, resolving
/// each entry's anchor position (via `pos_of`) to its current line — shared
/// by every per-line-anchored render bridge (signs, EOL text, line
/// backgrounds), whose bodies are otherwise identical up to this filter
/// step: resolve line → drop if scrolled out of view or drifted onto the
/// phantom trailing line (`resolve_decoration_line`) → keep. `tag` is
/// whatever a caller needs alongside the resolved line, generic because
/// each caller's `entries` iterator already decided its shape upstream: a
/// resolved gutter slot for signs (`DecorationStores::signs_in_range`
/// resolves it once per source, before this function ever sees an entry), a
/// source name for EOL text/line backgrounds' cross-source tie-break
/// (`last_writer_per_line`). This function only threads it through,
/// borrowed the whole way: every field on every decoration entry is already
/// either `Copy` or an already-interned `ScopeId`, so no caller needs an
/// owned clone to escape this iterator's borrow.
fn visible_line_anchored<'a, K, E: 'a>(
    text: &'a hume_editing::text::BufferText,
    visible_lines: std::ops::Range<usize>,
    entries: impl Iterator<Item = (K, &'a E)>,
    pos_of: impl Fn(&E) -> usize,
) -> impl Iterator<Item = (K, usize, &'a E)> {
    entries.filter_map(move |(tag, e)| {
        let line = resolve_decoration_line(text, pos_of(e))?;
        visible_lines.contains(&line).then_some((tag, line, e))
    })
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
