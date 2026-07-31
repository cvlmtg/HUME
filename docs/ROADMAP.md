# HUME — Roadmap

## Roadmap

### Editor — new features

- [ ] Tabline UI — engine-rendered buffer/tab bar; `TabBarProvider` slot already exists.
- [ ] Tree-sitter text objects — `textobjects.scm` / `locals.scm` (structural select).
- [ ] Tree-sitter structural navigation — jump to next/prev function, argument, class, etc. (structural move, distinct from text-object select).
- [ ] Scope-aware local rename — tree-sitter locals, LSP fallback via `core:lsp`.
- [ ] Code folding — tree-sitter-powered collapse/expand.
- [ ] Class A docked panes — fixed-row-count `LayoutTree` variant alongside ratio-based splits. Clients: quickfix list, LSP references/diagnostics, embedded terminal/REPL, build/test runner, `:help` pager, DAP debugger views. Deferred until the first concrete client is scoped.
- [ ] Wire remaining Class B drawer clients — `:ls`, `:messages`, notifications, command/search history pagers onto the existing bottom-drawer primitive.
- [ ] Wrap indicator — configurable char prepended to continuation rows in soft-wrap mode.
- [ ] `on-buffer-switch` hook + per-buffer keymaps (Steel).
- [ ] `:e <new-path>` touch-or-open — create empty buffer bound to path when file doesn't exist; first `:w` writes it.
- [ ] `:e` binary / huge-file y/n confirm — binary-sniff + size threshold. The reusable confirm-overlay primitive this needs (`ui::confirm`) already exists, built for the disk-change reload prompt.
- [ ] Streaming load for huge files — chunked read replacing single blocking full-file read.
- [ ] File-size statusline element + cached size metadata.
- [ ] Unified decoration system — single trait replacing the separate gutter/highlight/virtual-line/overlay provider traits; post-LSP, once the surface is stable.
- [ ] Scriptable minibuffer completers — Steel builtin to register plugin completers; core does prefix matching only, fuzzy scoring is a plugin concern.
- [ ] Scriptable insert-mode completion sources — see `docs/COMPLETION-PICKER.md` (additive, nothing blocks on current work).
- [ ] Auto-generated command reference + in-editor `:help` expansion.

### Editor — fixes & optimizations

- [ ] Byte-string parsing in settings — `"10MB"` / `"512KB"` strings; companion to the size-threshold setting.
- [ ] Native directory-walker fallback for the file picker — for bare directories without `fd`; build only if the fallback posture proves inadequate in practice (see `docs/FUZZY-FINDERS.md`).
- [ ] `RowMap::block`'s provider queries are one line at a time (`virtual_lines(line..line+1, ..)`) — the `VirtualLineSource` API already takes a `Range`, but no caller ever queries wider than one line, so each cache miss pays a per-provider lock+lookup+clone per line. Batching would need the render path to query its whole visible range up front.
- [ ] `:sort --lexicographic` override — for when numeric auto-detection guesses wrong (e.g. `1.10` vs `1.9`). Not worth shipping until it actually bites.

### Plugins

- [ ] PLUM: pin plugins to commit / tag / branch.
- [ ] `core:lsp` `cargo-git` install flavor — installs from a pinned git tag instead of crates.io semver; unblocks `nil`.
- [ ] `core:lsp` install support for `pkg:golang` (gopls) and `pkg:pypi` source kinds — currently fail loudly as unsupported (see `docs/LSP-INSTALL.md`'s "v1 scope and limitations").
- [ ] `:lsp-install` argument completion — Steel commands have no argument-completion path today.
- [ ] `git-diff` plugin — gutter signs + inline diff, layered (signs first, then virtual deleted lines, then background tint); plugin candidate, keep out of core. Design: `docs/GIT-DIFF.md`.

## Open questions

- Multiline quote text objects — currently line-bounded (parity scan gives wrong results across unmatched quotes on earlier lines); use tree-sitter to resolve when a grammar is loaded, fall back to line-bounded parity otherwise.
- Common-prefix auto-extend on Tab — extend input to the shared prefix of candidates before opening the popup, like readline/bash? Decide after more usage; risks surprising input mutation mid-type.
- Plugin-defined languages × lazy loading — a plugin that defines its **own** language must register that identity eagerly; it can't be the sole provider of its own lazy-activation trigger. Deferred to a dedicated brainstorm.
- `llvm-mir` / `llvm-mir-yaml` grammar mismatch — inherited from Helix; no HUME-specific fix until upstream resolves it.
- Undercurl blocked on ratatui underline-shape support — engine model and theme loader are already correct; revisit once ratatui exposes underline-shape bits.
- `ProviderSet::remove` unwired — implemented and tested, but no editor call site yet; wire it when the first real consumer lands (plugin-registered columns/overlays, or a gutter-visibility toggle).
- Search-state clobbering from Steel — no risk today (`search-next`/`select-all-matches` take no pattern arg, can't inject a pattern). Decide guard policy before any future pattern-taking search builtin is added.
- Mark/bookmark drift across edits — a stored char offset goes stale as edits land before it's used. Real marks need positions mapped through edit history; decide where that mapping lives and which stored positions opt in.
- Snap-vs-error policy for future selection setters — a future selection-setting builtin must handle out-of-range or mid-cluster input from Steel: snap to the nearest valid boundary (forgiving, hides bugs) or raise (fail-fast, breaks legitimately-drifted positions). Decide per call site before the first setter lands.
