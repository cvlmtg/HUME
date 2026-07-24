# Roadmap

HUME is under active development. Here's what's missing and what's coming.

## Planned

**Tabline** — a visible buffer bar across the top of the screen.

**In-editor help (`:help`)** — browse the manual or per-command docs without leaving HUME.

**Persistent undo tree** — save the branching undo history to disk so it survives a restart. Today the tree is in-memory only, and `u` / `U` walk the newest branch.

## Future ideas

**Git gutter** — diff markers in the gutter (a good plugin candidate).

**Embedded REPL** — a Steel REPL running in a docked pane.

**DAP debugger** — Debug Adapter Protocol support for interactive debugging.

## Already here

Language servers and inline diagnostics have landed — see [Language Servers](lsp.md) for completions, hover, go-to-definition, rename, formatting, code actions, and the rest.

A fuzzy file/buffer picker has landed as `core:pickers` — see [Fuzzy Finder](pickers.md) (`g f` to open files, `g b` to switch buffers).

---

::: info
Plans change. This page tracks the user-visible shape of what's coming, not a schedule.
:::
