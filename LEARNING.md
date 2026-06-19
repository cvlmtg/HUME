# HUME — Learning Notes

Concepts that come up while building HUME, explained in enough depth to be
useful later. Each topic lives in its own file under `docs/learning/`.

---

## Index

### Core text model

How text is stored and mutated, and the invariants that must always hold.

| Topic | File |
|-------|------|
| Unicode Position Model: Bytes, Chars, and Grapheme Clusters | [docs/learning/unicode-position-model.md](docs/learning/unicode-position-model.md) |
| Changesets: Describing Edits as Data | [docs/learning/changesets.md](docs/learning/changesets.md) |
| The Undo Tree: Branches, Not a Stack | [docs/learning/undo-tree.md](docs/learning/undo-tree.md) |
| Buffer Invariants and Plugin Safety | [docs/learning/buffer-invariants.md](docs/learning/buffer-invariants.md) |

### Selection & motion

How the cursor model works, how movements are classified, and how text objects
produce selections.

| Topic | File |
|-------|------|
| Edit Operations: Acting on Selections | [docs/learning/edit-operations.md](docs/learning/edit-operations.md) |
| Motions vs Text Objects | [docs/learning/motions-vs-text-objects.md](docs/learning/motions-vs-text-objects.md) |
| MotionMode: Separating Position from Anchor Semantics | [docs/learning/motion-mode.md](docs/learning/motion-mode.md) |
| Word Motions: Selecting the Whole Word | [docs/learning/word-motions.md](docs/learning/word-motions.md) |
| CharClass: Word Boundaries and the Eol Split | [docs/learning/charclass.md](docs/learning/charclass.md) |
| Inner vs Around: The Text Object Convention | [docs/learning/inner-vs-around.md](docs/learning/inner-vs-around.md) |
| Quote Scanning: Parity Instead of Depth | [docs/learning/quote-scanning.md](docs/learning/quote-scanning.md) |

### Architecture

How rendering, dispatch, and the engine/editor boundary are structured.

| Topic | File |
|-------|------|
| The Command/Keymap/Dispatch Architecture | [docs/learning/command-keymap-dispatch.md](docs/learning/command-keymap-dispatch.md) |
| The Rendering Pipeline: Engine, Providers, and the 4-Stage Pipeline | [docs/learning/rendering-pipeline.md](docs/learning/rendering-pipeline.md) |
| Display Lines and Soft Wrap | [docs/learning/display-lines-and-soft-wrap.md](docs/learning/display-lines-and-soft-wrap.md) |

### Languages & syntax

How buffers learn their language, and how tree-sitter grammars produce
syntax highlighting.

| Topic | File |
|-------|------|
| Language Identity and Detection | [docs/learning/language-identity.md](docs/learning/language-identity.md) |
| Tree-sitter: Grammars, Queries, and Plum | [docs/learning/tree-sitter-pipeline.md](docs/learning/tree-sitter-pipeline.md) |

### Plugins & registers

How plugins interact with editor state, and how HUME thinks about captured
text and paste.

| Topic | File |
|-------|------|
| Plugin Architecture: Loading, Activation, and Isolation | [docs/learning/plugin-architecture.md](docs/learning/plugin-architecture.md) |
| Plugin Attribution: Who Owns What | [docs/learning/plugin-attribution.md](docs/learning/plugin-attribution.md) |
| Runaway-Script Protection: The Watchdog Timer | [docs/learning/runaway-script-protection.md](docs/learning/runaway-script-protection.md) |
| Kill Ring and Smart-p: Two Sources of Paste | [docs/learning/kill-ring-and-smart-p.md](docs/learning/kill-ring-and-smart-p.md) |

### Vimgolf

How HUME's editing model compares on real challenges, and what the idioms look like in practice.

| Topic | File |
|-------|------|
| HUME as a Golf Club | [docs/learning/vimgolf.md](docs/learning/vimgolf.md) |
