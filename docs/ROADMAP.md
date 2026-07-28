# HUME — Roadmap

## Key decisions

- **Rust** — memory-safe, zero-cost abstractions, mature TUI ecosystem; ideal learning project.
- **Rope (`ropey`) text storage** — O(log n) edits anywhere, built-in line indexing.
- **Steel (Rust-native Scheme) for config + plugins** — Lisp, embeds in Rust.
- **Select-then-act, multiple selections from day one; selections always inclusive** — `anchor == head` is a 1-char selection, never a zero-width point.
- **Grapheme clusters from day one** — all motions/selections/edits over `unicode-segmentation`; retrofitting is expensive.
- **Command-based keymaps** — keys bind to named commands, never to other keys; no key-to-key remapping.
- **Keymap defaults hardcoded in Rust (SSOT), config overrides only** — editor always works with zero config.
- **Extend = `e` toggle** — Ctrl+motion rejected as universal modifier (fatal legacy-terminal collisions on 10/15 motion keys); Alt rejected (types accented chars on macOS). Kitty Ctrl+motion is a graceful bonus.
- **Word motions select the whole destination word (+ optional adjacent whitespace)**.
- **`*` = word-under-cursor (Vim-style)**, **`Ctrl+/` = search the current selection.**
- **Surround = `ms` + smart `r`** — rejects Helix's `md`/`mr`, which bake selection+action together and violate select-then-act.
- **`c` leaves the typed replacement selected** — divergence from Kakoune/Helix (both collapse to a cursor); `i`/`a`/`o`/`O` keep a collapsed cursor.
- **Tab handling = one knob (`tab-style`, reuses `tab-width`)** — rejects Vim's four-knob `shiftwidth`/`softtabstop` model as too complex.
- **`undo-levels`: `0` = unlimited** — diverges from Vim (no `-1` value); caps total revision count tree-wide, not path depth.
- **`set-option!`/`get-option` are callable from every eval mode** — `settings_ops::apply_global` is the validating write chokepoint, so an init-only/command-only split guarded nothing it didn't already.
- **Macros = register-based, Vim-style `Q`/`q` UX** — `Q` records (deliberate setup), `q` replays (hot path).
- **Syntax = tree-sitter** — incremental parsing, structural understanding beyond colors.
- **Theming = Helix-compatible hierarchical scopes + TOML** — `inherits` chains and `palette`, for theme reuse.
- **Full-trust plugins, no sandbox** — every plugin runs with Steel's full stdlib.
- **Terminal: require true color + synchronized output; prefer kitty keyboard, fall back** — no shims for ancient terminals.
- **Register paste count mismatch (N≠M)** — join full register content, apply to every selection.
- **Signals quit through the main loop; pty-hangup force-exits** — SIGINT/TERM/HUP/QUIT wake the loop so LSP servers get a graceful `shutdown` (3 s force-exit fallback). A hangup can't: termina's reader spins on an EOF tty and never sees the waker. Exit code is `128 + signo` on Unix; Windows is always 130, since `ctrlc` doesn't say which control event fired.
- **Long-lived children get their own process group and a process-wide registry** — force-exit `killpg`s each one, reaching grandchildren (rust-analyzer's `proc-macro-srv`, build scripts) that killing the tracked pid alone would orphan. The registry holds the unreaped `Child`, not a pid, so pids can't be recycled underneath it and Windows works the same way. Accepted trade-off: leaving the foreground group costs these children the kernel's SIGHUP on pty teardown — covered by the LSP `processId` convention and stdin EOF.
- **`:reload-config` resets config-owned state, then replays the buffer-open lifecycle** — everything config owns goes back to its compiled-in default before `init.scm` re-runs, discarding runtime `:set`/`:theme` changes too. Buffers, panes, undo history, registers, and running LSP servers are untouched. One exception: an explicit `language=` assertion survives, because detection alone can never reconstruct it. Config-derived hooks re-fire afterwards so state gated on a transition a bare reload never causes doesn't silently stay empty — deliberately not a literal close+reopen (no LSP `didClose`/`didOpen`, no `OnBufferClose`).
- **Startup grammar registration is core, not PLUM** — every already-compiled grammar registers unconditionally at startup, so highlighting survives `core:plum` being absent from `init.scm`. PLUM keeps only the install pipeline.
- **Language identity and grammar attachment are independent facts** — re-running either must not silently undo the other, since grammars attach before `init.scm` gets a chance to override an identity.
- **The whole statusline row tints with the active mode's color** — replaces the Helix mode-pill, so the mode is legible at a glance rather than in a 3-character corner. Opt out with `statusline.mode-colors = false`.
- **Scroll affordance is a proportional thumb, not arrows** — in menus as well as popups; an arrow glyph can't convey how much more there is to scroll.
- **`:sort` permutes whole rows, keyed by the selected text (`sort -k`), per contiguous run, numeric auto-detected** — rejects both Helix's `:sort` (permutes text *between* selection slots, rows never move — requires a manual `%` + split-on-newline step to sort a file) and Kakoune's `|sort` (pipes each selection through the shell, so N one-line selections is an N-way no-op). Non-adjacent selections form independent groups; equal keys keep document order (stable); `-r`/`-i` flip/fold the comparison, never the result. Deferred: a `--lexicographic` override for when auto-numeric guesses wrong (e.g. `1.10` vs `1.9`) — not worth shipping until it actually bites.
- **External file-change detection = stat-on-trigger (mtime + size), not a filesystem watcher** — Neovim's own design for the same problem, copied deliberately. inotify/FSEvents/kqueue/ReadDirectoryChangesW disagree on rename semantics and coalescing, and a watcher needs a thread + handle per watched directory; stating at a handful of trigger points (terminal focus, buffer-enter, return from an inline shell command, `:checktime`) costs nothing in the background and behaves identically on every platform. Size is compared alongside mtime because HFS+/FAT only report mtime to one-second resolution. `autoread` (default on) prompts via a reusable native confirm overlay; off just warns. A buffer flagged stale refuses `:w` until reloaded or forced with `!`.

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

### Plugins

- [ ] PLUM: pin plugins to commit / tag / branch.
- [ ] `core:lsp` `cargo-git` install flavor — installs from a pinned git tag instead of crates.io semver; unblocks `nil`.
- [ ] Git gutter signs — plugin candidate, keep out of core.

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
