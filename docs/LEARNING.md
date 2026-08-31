# HUME — Learning Notes

Concepts that come up while building HUME, explained in enough depth to be
useful later. Each topic lives in its own file under `docs/learning/`.

---

## Index

### Core text model

How text is stored and mutated, and the invariants that must always hold.

| Topic | File |
|-------|------|
| Unicode Position Model: Bytes, Chars, and Grapheme Clusters | [learning/unicode-position-model.md](learning/unicode-position-model.md) |
| Changesets: Describing Edits as Data | [learning/changesets.md](learning/changesets.md) |
| The Undo Tree: Branches, Not a Stack | [learning/undo-tree.md](learning/undo-tree.md) |
| Buffer Invariants and Plugin Safety | [learning/buffer-invariants.md](learning/buffer-invariants.md) |

### Selection & motion

How the cursor model works, how movements are classified, and how text objects
produce selections.

| Topic | File |
|-------|------|
| Edit Operations: Acting on Selections | [learning/edit-operations.md](learning/edit-operations.md) |
| Motions vs Text Objects | [learning/motions-vs-text-objects.md](learning/motions-vs-text-objects.md) |
| Move vs Extend: Separating Position from Anchor Semantics | [learning/motion-mode.md](learning/motion-mode.md) |
| Word Motions: Selecting the Whole Word | [learning/word-motions.md](learning/word-motions.md) |
| CharClass: Word Boundaries and the Eol Split | [learning/charclass.md](learning/charclass.md) |
| Inner vs Around: The Text Object Convention | [learning/inner-vs-around.md](learning/inner-vs-around.md) |
| Quote Scanning: Parity Instead of Depth | [learning/quote-scanning.md](learning/quote-scanning.md) |
| Matching Pairs: Depth Tracking and Two Accepted Limitations | [learning/matching-pairs.md](learning/matching-pairs.md) |

### Architecture

How rendering, dispatch, and the engine/editor boundary are structured.

| Topic | File |
|-------|------|
| The Command/Keymap/Dispatch Architecture | [learning/command-keymap-dispatch.md](learning/command-keymap-dispatch.md) |
| The Rendering Pipeline: Engine, Providers, and the 4-Stage Pipeline | [learning/rendering-pipeline.md](learning/rendering-pipeline.md) |
| Display Lines and Soft Wrap | [learning/display-lines-and-soft-wrap.md](learning/display-lines-and-soft-wrap.md) |
| Splits and Panes: One Buffer, Many Views | [learning/splits-and-panes.md](learning/splits-and-panes.md) |

### Languages & syntax

How buffers learn their language, how tree-sitter grammars produce syntax
highlighting — including embedded languages via injections — and how
language servers add semantic features like diagnostics and completion.

| Topic | File |
|-------|------|
| Language Identity and Detection | [learning/language-identity.md](learning/language-identity.md) |
| Tree-sitter: Grammars, Queries, and Plum | [learning/tree-sitter-pipeline.md](learning/tree-sitter-pipeline.md) |
| LSP: One Protocol Between Editors and Languages | [learning/lsp.md](learning/lsp.md) |

### Plugins & registers

How plugins interact with editor state, and how HUME thinks about captured
text and paste.

| Topic | File |
|-------|------|
| Plugin Architecture: Loading, Activation, and Isolation | [learning/plugin-architecture.md](learning/plugin-architecture.md) |
| Plugin Attribution: Who Owns What | [learning/plugin-attribution.md](learning/plugin-attribution.md) |
| Runaway-Script Protection: The Watchdog Timer | [learning/runaway-script-protection.md](learning/runaway-script-protection.md) |
| Kill Ring and Smart-p: Two Sources of Paste | [learning/kill-ring-and-smart-p.md](learning/kill-ring-and-smart-p.md) |

### Vimgolf

How HUME's editing model compares on real challenges, and what the idioms look like in practice.

| Topic | File |
|-------|------|
| HUME as a Golf Club | [learning/vimgolf.md](learning/vimgolf.md) |
