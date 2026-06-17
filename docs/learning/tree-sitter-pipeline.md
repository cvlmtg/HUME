# Tree-sitter: Grammars, Queries, and Plum

## The two halves of a language

Every language in HUME has an identity half and a capability half.

The **identity half** is covered in [Language Identity and Detection](language-identity.md):
a name, a set of detection rules (globs, extensions, shebangs), and hooks.
It is always present and always cheap.

The **capability half** is what this document is about: a grammar that can
parse the language into a syntax tree, and a query that can map tree nodes
to semantic names. A language can be registered with only the identity half —
the detection and hooks work fine, but there is no syntax highlighting.
Capability is attached separately, usually by the `plum` plugin.

This separation is what lets HUME recognize a `.zig` file as Zig (useful for
the statusline and hooks) even before the user has installed the Zig grammar.

## What a grammar bundle is

When a grammar is installed and registered, HUME loads two things together:

1. **A parser** — a native shared library (`.dylib` on macOS, `.so` on Linux,
   `.dll` on Windows) compiled from the grammar's C source. Loading it gives
   HUME a tree-sitter `Language` object that can parse source text into a
   concrete syntax tree.

2. **A compiled highlight query** — a tree-sitter `Query` compiled against
   that specific `Language`. The query is the bridge between the parse tree
   and semantic meaning: it matches patterns in the tree and assigns named
   captures to them.

These two are kept together — a grammar bundle — because the query is compiled
against the `Language` pointer inside the shared library. They share a
lifetime: you cannot use a query compiled for one version of a grammar with
a parser from a different version.

The compiled query is shared across every buffer of that language. Whether
ten files or a hundred are open, there is exactly one `Arc<Query>` for Rust,
one for Python, and so on. Parsing produces a distinct syntax tree per
buffer; the query used to walk those trees is the same object.

## Highlight queries

A highlight query is a tree-sitter `.scm` file. It contains patterns that
match subtrees and annotate them with named captures:

```scheme
(function_item name: (identifier) @function)
(string_literal) @string
["fn" "let" "mut"] @keyword
```

The capture names — `@function`, `@string`, `@keyword` — are the bridge to
the theme system. HUME's theme defines colors for these names; the query
says which tree nodes get which name. HUME uses Helix-compatible queries,
which means any Helix highlight query works without modification.

At query-compilation time, capture names are interned into the editor's scope
registry. The scope registry is the vocabulary of token types that themes
speak. A theme maps scope names to colors; a query maps AST patterns to scope
names; together they map parse trees to styled text.

## Plum: the grammar manager

`plum` is HUME's built-in plugin manager. It is written in Scheme and ships
with the editor — not a separate CLI binary. For grammars specifically, plum
provides:

- **`:plum-install-grammar`** — installs the grammar for the current buffer's
  language.
- **`:plum-update-grammar`** — re-clones and recompiles the grammar for the
  current buffer's language at the currently pinned revision.
- **`:plum-ensure-grammars`** — batch-installs a list of named grammars that
  are not yet compiled. Called programmatically from `init.scm`:
  `(call! "plum-ensure-grammars" '("rust" "json"))`.
- **`:plum-list-grammars`** — reports declared, installed, missing, and
  orphan grammars.
- **`:plum-cleanup-grammars`** — removes on-disk artifacts for grammars
  no longer in the catalog.

The grammar catalog is a Scheme data file that ships with HUME. It declares
each supported language as a 5-tuple: the language name, the upstream git
repository, the revision to pin, the tree-sitter symbol name inside the
library, and (optionally) a subdirectory within the repo. This is the only
place where the list of supported grammars is maintained.

When you run `:plum-install-grammar`, plum:

1. Clones the grammar repository at the pinned revision.
2. Fetches the matching highlight query from Helix's pinned runtime.
3. Compiles the C source to a shared library and writes it to the editor's
   data directory.
4. Writes the highlight query alongside it.
5. Calls `register-grammar!` to load the grammar into the running editor.

On subsequent starts, plum calls `register-installed-grammars!` during
initialization, which scans the data directory and registers every grammar
already on disk. No network access on startup; grammars are registered from
local files.

## Why pinned revisions

Grammar repositories and their highlight queries are pinned to specific
revisions for reproducibility. The coupling is tight: query patterns match
named nodes that the grammar defines; if the grammar changes, the query may
stop matching or match incorrectly. Pinning both to known-good revisions
ensures that every HUME installation using the same catalog produces the same
highlighting behavior.

The `helix-pin.scm` file records the Helix revision from which queries are
fetched. Updating to a new version of a grammar means updating both the
grammar pin and the Helix pin in the catalog.

## Late grammar registration

Grammars can arrive after buffers are already open. A user might open a Rust
file, then install the Rust grammar (`:plum-install-grammar`) in the same
session. HUME handles this by sweeping open buffers when a grammar is attached.

After `register-grammar!` completes, HUME walks every open buffer. Any buffer
whose language name matches a grammar that was just attached is re-run through
syntax setup: a parse request is queued, and once the worker responds the
buffer renders with highlighting — no restart required.

The same mechanism handles batch registration at startup: because `plum`
registers all installed grammars during initialization (potentially after some
buffers have already been opened by the startup sequence), the sweep guarantees
that no buffer is left without highlighting due to ordering.

## End-to-end: opening a `.rs` file

Putting it all together, here is what happens when you open `main.rs`:

1. **Path resolution** — the path is resolved and a buffer is created with
   the file's contents.

2. **Detection** — the detection logic runs: no glob matches, the `.rs`
   extension matches Rust, so the language is identified as `"rust"`.

3. **Funnel** — the funnel is called with `"rust"`. It writes the language
   name to the buffer, looks up the `LanguageConfig` for Rust in the registry,
   and proceeds to syntax setup.

4. **Syntax setup** — if a grammar bundle is attached to the Rust config, a
   parse request is queued to the background parse worker. Parsing happens
   off the main thread; when the worker responds, the buffer's syntax tree is
   installed and the highlighter is wired up. A size gate prevents very large
   files from being parsed at all.

5. **Hook** — `OnLanguageSet` fires with the buffer id and `"rust"`. Plugins
   can react (e.g. configuring indent width, setting options, enabling
   diagnostics).

6. **`OnBufferOpen`** — fires next. By this point the language is guaranteed
   to be set, so `on-buffer-open` handlers can safely branch on language.

7. **Render** — when the buffer is drawn, the renderer's style stage reads
   the stored syntax tree (once available) and walks it with the shared query
   to produce per-grapheme style spans. Those spans are resolved against the
   active theme to produce the final colors.

The parse tree lives on the buffer and is updated incrementally on each edit.
The query and the theme are shared resources read during every render pass.
