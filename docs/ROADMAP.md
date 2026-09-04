# HUME — Roadmap

## Roadmap

### Editor — new features

- [ ] Tabline UI — engine-rendered buffer/tab bar; `TabBarProvider` slot already exists.
- [ ] Scope-aware local rename — tree-sitter locals, LSP fallback via `core:lsp`.
- [ ] Code folding — tree-sitter-powered collapse/expand.
- [ ] Docked panes — fixed-extent (along the split axis) `LayoutTree` variant alongside ratio-based splits, so a docked pane can be a column as well as a row. `equalize` (which currently rewrites every ratio in the tree on every split/close, see `LayoutTree::Split`'s doc comment) must be scoped to skip docked panes rather than resizing them. Clients: quickfix list, LSP references/diagnostics, embedded terminal/REPL, build/test runner, `:help` pager, DAP debugger views, undo-tree graph (`docs/UNDOTREE.md`). Deferred until the first concrete client is scoped.
- [ ] Wire remaining bottom-drawer clients — `:ls`, `:messages`, notifications, command/search history pagers onto the existing bottom-drawer primitive.
- [ ] Wrap indicator — configurable char prepended to continuation rows in soft-wrap mode.
- [ ] Per-buffer keymaps (Steel) — `on-buffer-enter` already exists to key off of.
- [ ] `:e <new-path>` touch-or-open — create empty buffer bound to path when file doesn't exist; first `:w` writes it.
- [ ] `:e` binary / huge-file y/n confirm — binary-sniff + size threshold. The reusable confirm-overlay primitive this needs (`ui::confirm`) already exists, built for the disk-change reload prompt.
- [ ] Streaming load for huge files — chunked read replacing single blocking full-file read.
- [ ] File-size statusline element + cached size metadata.
- [ ] Scriptable minibuffer completers — Steel builtin to register plugin completers; core does prefix matching only, fuzzy scoring is a plugin concern.
- [ ] Scriptable insert-mode completion sources — see `docs/COMPLETION-PICKER.md` (additive, nothing blocks on current work).
- [ ] Styled spans in `show-popup!` — the popup takes one flat string today, so signature help marks the active parameter as `⟨…⟩` on a second line instead of highlighting it in place. Wants a `(start end scope)` span list over the popup text, the shape `set-virtual-lines!`'s `'segments` already uses. Its input already arrives: HUME declares `labelOffsetSupport`, so a server sends each parameter's offsets into the signature label.
- [ ] Styled spans in the drawer — `lsp-locations->display-parts` shows an unopened target's column as the location's own wire unit rather than a measured grapheme column (see `docs/LSP.md`'s "User-facing column unit" decision row); once a drawer row can style part of itself, render that unmeasured column visually distinctly (e.g. italic) instead of identically to a measured one. Wants the same per-row span support as the `show-popup!` item above.
- [ ] Auto-generated command reference + in-editor `:help` expansion.
- [ ] `:earlier` / `:later` undo-tree time travel — the substrate already exists (`History::goto_revision`, `Revision::timestamp`); wants only the typed commands and, longer-term, a history-browsing UI.
- [ ] Steel-side picker row display formatter — `#:truncate 'head|'tail` only picks which end of an over-long row is clipped; see `docs/FUZZY-FINDERS.md`'s "Remaining work" for the general per-row formatter this doesn't cover.

### Editor — fixes & optimizations

- [ ] Byte-string parsing in settings — `"10MB"` / `"512KB"` strings; companion to the size-threshold setting.
- [ ] Native directory-walker fallback for the file picker — for bare directories without `fd`; build only if the fallback posture proves inadequate in practice (see `docs/FUZZY-FINDERS.md`).
- [ ] `RowMap::block`'s provider queries are one line at a time — `DecorationSource::decorations_for_line` takes a single `line_idx`, so each cache miss pays a per-provider lock+lookup+clone per line. Batching would need a range-taking query variant plus the render path querying its whole visible range up front.
- [ ] `:sort --lexicographic` override — for when numeric auto-detection guesses wrong (e.g. `1.10` vs `1.9`). Not worth shipping until it actually bites.
- [ ] Non-`file:` LSP location URIs (jdtls's `jdt://`, deno's `deno:`) — `goto-location!`/`lsp-locations->display-parts` reject them outright (`hume_lsp::uri::uri_to_path` only understands `file:`); a server sending one is conforming, HUME just has no reader for it yet. The error now names the URI and the unsupported scheme rather than printing `UriError`'s `Debug` form, but the destination is still unreachable.

### Performance — deferred

Structural work found during a cheap-wins sweep. Each is real but wants a design decision, an invalidation contract, or a wide signature change — none is a small edit. Nothing here is measured as dominant: the benchmark harness is itself the first item.

- [ ] Criterion benchmark harness over `Editor::render_to_buf` — the workspace has none, so every item below is reasoned from allocation and complexity counts rather than from a profile. Build this before betting on any of them.
- [ ] `Operation::Insert` carrying its own char length — five sites re-derive it with `chars().count()`, and `compose` re-counts a growing accumulator once per keystroke of an insert session (quadratic over the session). Touches every match site on the variant.
- [ ] Frame-level damage tracking — every frame re-formats, re-styles and re-composes every visible row; only the cell diff in `hume-grid` limits terminal writes. The largest single win and the largest correctness risk (several per-frame sync steps have side effects).
- [ ] Rows are formatted twice per frame under soft wrap — the scroll step and the render step each build a `RowMap` over their own `FormatScratch` (separate to avoid a borrow conflict), and counting a line's wrap rows means formatting it.
- [ ] One chunk-walking cursor for `display_col_in_line` / `char_pos_at_display_col` — both pay ~6 O(log n) rope descents per grapheme where a single resumable cursor would make them O(line). A grapheme-level sibling of `CharCursor` would do the same for the word motions, which interleave `char_at` with `next_grapheme_boundary`.
- [ ] `find_tightest_bracket_pair` runs three unbounded whole-buffer scans (`()`, `[]`, `{}`), each scanning to both ends when unmatched — one combined pass tracking three depths would replace six. Paragraph motions similarly re-descend per line where `lines_at` would traverse once.
- [ ] `Buffer::apply_edit` clones the whole `ChangeSet` per edit (once to record the revision, once into an open group's accumulator) — fine while typing, a full extra copy of the payload on a large paste. `Rc`/`Arc` on the propagated value fixes it structurally.
- [ ] Interning for names cloned per dispatch — `KeymapCommand`'s `Cow<'static, str>` allocates on every dispatch of a Steel-bound key (built-ins are borrowed and free).
- [ ] Statusline recomputes per frame — the `FilePath` two-pass sizing re-renders any section containing it (the default left section), and `DiagnosticsElement` scans the whole diagnostics store for its counts. Both want caching against a generation, not a mechanical fix.

### Plugins

- [ ] PLUM: pin plugins to commit / tag / branch.
- [ ] `core:lsp` `cargo-git` install flavor — installs from a pinned git tag instead of crates.io semver; unblocks `nil`.
- [ ] `core:lsp` install support for `pkg:golang` (gopls) and `pkg:pypi` source kinds — currently fail loudly as unsupported (see `docs/LSP-INSTALL.md`'s "v1 scope and limitations").
- [ ] `:lsp-install` argument completion — Steel commands have no argument-completion path today.

## Open questions

- Multiline quote text objects — currently line-bounded (parity scan gives wrong results across unmatched quotes on earlier lines); use tree-sitter to resolve when a grammar is loaded, fall back to line-bounded parity otherwise.
- `goto-matching-pair` tag matching (`#` on `<tag>`) is a lexical scan, not tree-sitter-backed (`hume-ops` can't depend on `hume-treesitter`). Decide whether it should resolve the enclosing element node once tree-sitter structural navigation lands — same lexical-vs-tree-sitter tension as the multiline-quote question above.
- Common-prefix auto-extend on Tab — extend input to the shared prefix of candidates before opening the popup, like readline/bash? Decide after more usage; risks surprising input mutation mid-type.
- Plugin-defined languages × lazy loading — a plugin that defines its **own** language must register that identity eagerly; it can't be the sole provider of its own lazy-activation trigger. Deferred to a dedicated brainstorm.
- `llvm-mir` / `llvm-mir-yaml` grammar mismatch — inherited from Helix; no HUME-specific fix until upstream resolves it.
- Search-state clobbering from Steel — no risk today (`search-next`/`select-all-matches` take no pattern arg, can't inject a pattern). Decide guard policy before any future pattern-taking search builtin is added.
- Mark/bookmark drift across edits — a stored char offset goes stale as edits land before it's used. Real marks need positions mapped through edit history; decide where that mapping lives and which stored positions opt in.
- Snap-vs-error policy for future selection setters — a future selection-setting builtin must handle out-of-range or mid-cluster input from Steel: snap to the nearest valid boundary (forgiving, hides bugs) or raise (fail-fast, breaks legitimately-drifted positions). Decide per call site before the first setter lands.
- `#:keys` activation entry for `declare-plugin` — would let a lazily declared plugin register its own key bindings as placeholder stubs, the way `#:commands` does today, instead of forcing `load-plugin` for any plugin whose only entry point is a key it adds or overrides (`core:pickers`, `core:vim-keybind`, `core:classic-paste`, `core:helix-surround`). Not pursued yet: the entry would have to restate the body's own `bind-key!` calls in the manifest, a second list to keep in sync, and still couldn't express `unbind-key!` (`core:helix-surround` drops `m w`). Decide based on real usage or user feedback rather than up front.
- Macro registers through `read-register`/`write-register!` — a macro is `Vec<KeyEvent>`, not text, so today both builtins treat a register holding one as empty (`read-register` answers `#f`; there is no write path for one at all). A real fix needs a wire format for a key-event sequence to/from Scheme — decide the shape (raw key strings? the same encoding `bind-key!` parses?) before adding it.
