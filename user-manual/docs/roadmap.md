# Roadmap

HUME is under active development. Here is what's coming.

## Implemented

**Syntax highlighting** — tree-sitter integration for accurate, incremental highlighting. Grammar installation is handled automatically by PLUM via `:plum-install-grammar`.

**Multiple selections** — `s` to select within selection, `S` to split on newlines, `C` to copy selections, and simultaneous multi-cursor editing.

**Split panes** — `:split` and `:vsplit` (or `Ctrl+p s`/`Ctrl+p v`) to view multiple buffers side by side, with directional pane focus (`Ctrl+p h/j/k/l`) and a visible divider between panes.

## Planned

**LSP support** — language server integration: completions, diagnostics, hover, go-to-definition, rename.

**Fuzzy file picker** — Helix-style picker for opening files and switching buffers quickly.

**Inline diagnostics** — error and warning indicators inline in the buffer, powered by LSP.

**Tabline** — visible buffer/tab bar at the top of the screen.

## Future ideas

**Git gutter** — diff markers in the gutter (plugin candidate).

**Embedded terminal / REPL** — run a Steel REPL or shell inside a docked pane.

**DAP debugger** — debugger adapter protocol support for interactive debugging.

**In-editor help (`:help`)** — browse the manual or per-command docs from inside HUME. Currently all documentation lives outside the editor (this manual and `:tutor`).

**Persistent undo tree** — persist the branching undo history to disk so it survives restart. Today the undo tree is in-memory only.

---

::: info
This roadmap reflects current plans and is subject to change. For a detailed technical breakdown, see the project's `ROADMAP.md` on GitHub.
:::
