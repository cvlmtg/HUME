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
Capability is attached separately: every installed grammar is registered
automatically at startup, and the `plum` plugin is what installs a grammar
that isn't there yet.

This separation is what lets HUME recognize a `.zig` file as Zig (useful for
the statusline and hooks) even before the user has installed the Zig grammar.

## What a grammar bundle is

When a grammar is installed and registered, HUME loads two things together:

1. **A parser** — a native shared library (`.dylib` on macOS, `.so` on Linux,
   `.dll` on Windows) compiled from the grammar's C source. Loading it gives
   HUME a language handle that tree-sitter can use to parse source text into
   a concrete syntax tree.

2. **A compiled highlight query** — a query compiled against that specific
   language handle. The query is the bridge between the parse tree and
   semantic meaning: it matches patterns in the tree and assigns named
   captures to them.

A grammar whose language embeds others carries an optional third piece: an
**injection query**, which marks regions of the tree as belonging to a
different language (see [Injections](#injections-embedded-languages) below).

These pieces are kept together — a grammar bundle — because the queries are
compiled against the language handle inside the shared library. They share a
lifetime: you cannot use a query compiled for one version of a grammar with a
parser from a different version.

The compiled queries are shared across every buffer of that language. Whether
ten files or a hundred are open, there is exactly one compiled highlight query
for Rust, one for Python, and so on. Parsing produces distinct syntax trees
per buffer; the queries used to walk those trees are the same objects.

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

## Injections: embedded languages

One document often contains more than one language. A Markdown file has
fenced code blocks in Rust or Python; an HTML page has `<script>` tags full
of JavaScript; a Rust doc comment contains Markdown. The Markdown grammar
knows *where* a fenced block is, but it cannot parse Rust — its tree just
says "here is a code block whose info string reads `rust`."

**Injection queries** bridge that gap. Like a highlight query, an injection
query is a `.scm` file of patterns compiled against the host grammar — but
instead of assigning colors, its captures say "this node's text is another
language: parse it with that language's grammar." The embedded language is
sometimes named right in the query (a fixed choice like Markdown's inline
grammar) and sometimes read out of the document itself (the ` ```rust ` info
string of a fence).

### Layers

With injections, a buffer no longer holds one syntax tree. It holds a **root
tree** (the buffer's own language) plus a set of **injected layers**: each
layer is a full parse of another grammar, restricted to the byte ranges the
injection query marked. Layers can nest — a Markdown fence containing Rust
containing a doc comment containing Markdown again — so injection resolution
runs recursively, with a small depth cap to keep pathological documents from
recursing forever.

Two wrinkles make layers more interesting than "a tree per region":

- **Combined layers.** Some embedded languages are scattered across the
  document in many small spans that only make sense parsed *together*.
  Markdown's inline grammar (bold, italic, inline code) is the canonical
  case: every paragraph contributes a span, and they are parsed as one layer
  covering many disjoint ranges rather than one layer per paragraph.
- **Depth priority.** Where layers overlap, the *deeper* layer's highlighting
  wins. Inside a Rust fence, Rust's `@keyword` beats whatever Markdown would
  have said about those bytes — the most specific parse of a region is the
  one the user should see.

Missing grammars degrade gracefully: if a fence names a language whose
grammar isn't installed, that region simply keeps the host language's
highlighting. Nothing errors; installing the grammar later lights it up.

### Incremental root, full-parse layers

Edits update the root tree incrementally, as before. Injected layers are
re-resolved and re-parsed from scratch after each root parse instead. This is
a deliberate trade-off: matching up "which layer from the previous parse is
the same layer now" across arbitrary edits is complex and error-prone, while
the regions themselves (a code fence, a script tag) are typically small
enough that a full parse is cheap. The expensive parse — the whole buffer —
stays incremental; the small ones are recomputed.

## Text objects: selecting and navigating by structure

A highlight query answers "what is this node?" An injection query answers "what language is this
region?" A third kind of query — a text-object query — answers a different question: "where does the
function, class, comment, or argument around this cursor begin and end?" It is the same mechanism
(a `.scm` file of patterns compiled against the grammar) turned toward a different purpose: instead
of naming nodes for the theme, its captures name whole structural objects for selection and
navigation to use.

### One object, several captures

A structural object is not always one node. A function preceded by an attribute or decorator is,
conceptually, one selectable unit — the attribute plus the function it attaches to — even though the
grammar represents them as two adjacent nodes. A block comment is a run of several single-line
comment nodes read as one paragraph. A query pattern can tag more than one node with the same object
name in a single match, and the object those captures describe is the *span from the earliest node's
start to the latest node's end* — a hull drawn around every node the pattern singled out, not any one
node in isolation. This is what lets "select the function" include its attribute without the query
author having to special-case attributes at every call site.

### Selecting: smallest wins

When you ask to select "the enclosing function," there may be several functions whose span contains
the cursor — an inner closure inside an outer method, say. The rule is simply: the *smallest* span
that contains the cursor wins. This is exactly the intuition "select the closest enclosing thing," and
it composes naturally with nesting of any depth without needing a special case for how deep the
nesting goes.

### Navigating: nearest start wins

Jumping to the "next" or "previous" object is a different question — not "which object contains the
cursor" but "which object's boundary is closest in the given direction." Moving forward, that is the
object whose start comes soonest after the cursor; moving backward, the object whose start comes
soonest before it. Keying both directions off an object's *start* (rather than, say, its end) means a
backward jump from inside an object lands on that object's own beginning first, before walking further
back to an earlier one — matching how a reader would describe "go back to the start of this thing,"
one step at a time.

### Why layering needs no special handling

A document can hold more than one syntax tree at once (see [Layers](#layers) above) — a Markdown file
with a fenced Rust block has a Markdown tree and a Rust tree, each queried independently for
structural objects. A structural object found inside an injected layer's tree always sits at a
position inside the region that layer covers, which itself sits inside the host document's own
structure. Combined with "smallest span wins," an object from the more deeply nested layer is
automatically preferred over anything coarser the host layer might have offered for the same
position — no separate rule is needed to prefer "the more specific parse." A layer whose language
defines no structural objects at all simply contributes nothing, and the search continues in whichever
layer does.

### Why the tree must be current first

A structural query answers in terms of *positions* — where a span starts and ends. If the cursor has
moved because of an edit since the tree was last built, but the tree itself hasn't been rebuilt yet,
those positions describe where the code used to be, not where it is now. Ordinary highlighting can
tolerate a tree that lags an edit by a frame or two — the visual difference is invisible for that
long. A structural command cannot: it would select the wrong span, at best, and could ask for a
position that no longer exists in the buffer at all. So a structural command always brings the tree
up to date with the buffer's current text before it asks its query anything, even if that means doing
a small amount of parsing work synchronously rather than waiting for the background parser to catch up
on its own.

## Plum: the grammar manager

`plum` is HUME's built-in plugin manager. It is written in Scheme and ships
with the editor — not a separate CLI binary. The name covers two roles:
plugin management (install, update, list, cleanup for Scheme plugins) and
grammar management. For grammars specifically, plum provides:

- **`:plum-install-grammar`** — installs (or repairs) a named grammar, or the
  current buffer's language if no name is given, always re-cloning and
  recompiling from a clean slate at the currently pinned revision.
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

1. Installs any **dependency grammars** first. Some grammars are incomplete
   without a companion: Markdown's emphasis and inline code live in a separate
   inline grammar that Markdown's injection query expects to find. Plum knows
   these pairings and installs the companion transparently.
2. Clones the grammar repository at the pinned revision.
3. Fetches the matching highlight query from Helix's pinned runtime and writes
   it to the editor's data directory. The injection query is fetched the same
   way once the library is built, but best-effort: most languages embed
   nothing and have no injection query, so its absence is normal, not an
   error.
4. Compiles the C source to a shared library, also in the data directory.
5. Registers the shared library and the queries with the running editor. A
   grammar whose tree-sitter ABI version is incompatible with the editor is
   rejected here with a clear error rather than crashing the parse worker
   later.

On subsequent starts, HUME scans the data directory for compiled grammars and
registers every one whose library and queries are already on disk. No network
access on startup, and no catalog *scan* either — the walk works from what's
actually present on the filesystem, not from what's declared, and only
consults the catalog for a name it actually finds there. A fresh setup with
nothing installed never touches the catalog at all.

## Why pinned revisions

Grammar repositories and their highlight queries are pinned to specific
revisions for reproducibility. The coupling is tight: query patterns match
named nodes that the grammar defines; if the grammar changes, the query may
stop matching or match incorrectly. Pinning both to known-good revisions
ensures that every HUME installation using the same catalog produces the same
highlighting behavior.

A separate pinned-revision record tracks the Helix revision from which
queries are fetched. Updating grammars means moving the Helix pin forward and
re-deriving the catalog's grammar revisions from it — the two records move
together.

## Late grammar registration

Grammars can arrive after buffers are already open. A user might open a Rust
file, then install the Rust grammar (`:plum-install-grammar`) in the same
session. HUME handles this by sweeping open buffers when a grammar is attached.

After a grammar is registered, HUME walks every open buffer. Any buffer whose
language name matches a grammar that was just attached is re-run through
syntax setup: a parse request is queued, and once the worker responds the
buffer renders with highlighting — no restart required. Buffers whose own
grammar can inject other languages are swept too — installing the Rust
grammar lights up Rust code fences in a Markdown buffer that was already open
and highlighted.

The same mechanism handles batch registration at startup: because grammar
registration runs during initialization (potentially after some buffers have
already been opened by the startup sequence), the sweep guarantees that no
buffer is left without highlighting due to ordering.

## End-to-end: opening a `.rs` file

Putting it all together, here is what happens when you open `main.rs`:

1. **Path resolution** — the path is resolved and a buffer is created with
   the file's contents.

2. **Detection** — the detection logic runs: no glob matches, the `.rs`
   extension matches Rust, so the language is identified as `"rust"`.

3. **Funnel** — the funnel is called with `"rust"`. It writes the language
   name to the buffer, looks up Rust's registered grammar (if any), proceeds
   to syntax setup, and attaches the buffer to a Rust language server if one
   is configured (see the LSP doc).

4. **Syntax setup** — if a grammar bundle is attached to the Rust config, a
   parse request is queued to the background parse worker. Parsing happens
   off the main thread: the worker parses the root tree, then resolves any
   injections and parses those layers too, and the finished set is installed
   on the buffer with the highlighter wired up. A size gate prevents very large
   files from being parsed at all — the default cap is one mebibyte, and it is
   enforced not just at open but mid-session: a buffer that grows past the cap
   has its syntax detached, and one that later shrinks back under the cap is
   re-attached without a restart.

5. **Hook** — `on-language-set` fires with the buffer id and `"rust"`.
   Plugins can react (e.g. configuring indent width, setting options).

6. **`on-buffer-open`** — fires next. By this point the language is
   guaranteed to be set, so `on-buffer-open` handlers can safely branch on
   language.

7. **Render** — when the buffer is drawn, the renderer's style stage reads
   the stored syntax trees (once available) — the root tree plus any injected
   layers — and walks each with its language's shared query to produce
   per-grapheme style spans, deeper layers winning where they overlap. Those
   spans are resolved against the active theme to produce the final colors.

The parse trees live on the buffer: the root tree is updated incrementally on
each edit, while injected layers are re-parsed in full after each root parse.
The queries and the theme are shared resources read during every render pass.
Incremental updates are baked once per frame before the renderer runs — into
every layer's tree, not just the root — so the trees the renderer reads are
always coordinate-aligned with the buffer text on screen even while a fresh
reparse is still in flight on the background worker.
