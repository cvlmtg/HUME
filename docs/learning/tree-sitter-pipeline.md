# Tree-sitter: Grammars, Queries, and Plum

## The two halves of a language

Every language in HUME has an identity half and a capability half.

The **identity half** is covered in
[Language Identity and Detection](language-identity.md): a name, a set of
detection rules (globs, extensions, shebangs), and hooks. It is always present
and always cheap.

The **capability half** is what this document is about: a grammar that can
parse the language into a syntax tree, and queries that map tree nodes to
meanings. A language can exist with only its identity half. Detection and
hooks work, but there is no syntax highlighting. Capability is attached
separately: every installed grammar is registered at startup, and the `plum`
plugin installs a grammar that isn't there yet.

This separation is what lets HUME recognize a `.zig` file as Zig (useful for
the statusline and hooks) before the Zig grammar is installed.

## What a grammar bundle is

An installed grammar gives HUME two things that belong together:

1. **A parser**, a native shared library (`.dylib` on macOS, `.so` on Linux,
   `.dll` on Windows) compiled from the grammar's C source. Loading it gives
   HUME a language handle that tree-sitter uses to parse source text into a
   concrete syntax tree.

2. **A compiled highlight query**, compiled against that language handle. The
   query is the bridge between the parse tree and meaning: it matches
   patterns in the tree and assigns named captures to them.

Every other query is optional, and each answers one more question about the
tree:

| Query | Answers |
|-------|---------|
| injections | which regions belong to another language (see [Injections](#injections-embedded-languages)) |
| textobjects | where the function, class or argument around the cursor begins and ends (see [Text objects](#text-objects-selecting-and-navigating-by-structure)) |
| indents | how far a new line is indented |
| context | which scopes pin their first line at the top of the pane while you scroll inside them |
| folds | which regions a pane can fold |
| locals | which names are local, and which definition each use of a name refers to (see [Locals](#locals-coloring-a-name-by-its-definition)) |

Some fall back on others. Without a context query, the scopes are the
functions and classes the text-object query names. Without a folds query,
the foldable regions are those scopes. So a language with only text objects
still gets scope headers and folding.

These pieces form one bundle because the queries are compiled against the
language handle inside the shared library. A query compiled for one version
of a grammar cannot be used with a parser from another version.

The compiled queries are shared by every buffer of that language: each
buffer gets its own syntax tree, and one compiled query walks them all.

## Highlight queries

A highlight query is a tree-sitter `.scm` file. Its patterns match subtrees
and annotate them with named captures:

```scheme
(function_item name: (identifier) @function)
(string_literal) @string
["fn" "let" "mut"] @keyword
```

The capture names (`@function`, `@string`, `@keyword`) are the bridge to the
theme system. The theme defines colors for these names; the query says which
tree nodes get which name. HUME's highlight queries are Helix's, with Helix's
capture names; the context and fold queries come from the nvim-treesitter
projects.
A Helix query can inherit another language's patterns through an
`; inherits:` line; HUME does not follow that line itself, and the grammars it
installs come with the inherited patterns already merged in.

When a query is compiled, its capture names are added to the editor's scope
registry, the vocabulary of token types that themes speak. A theme maps scope
names to colors and a query maps tree patterns to scope names, so together
they map parse trees to styled text.

A query can also carry predicates and directives, small instructions attached
to a pattern (`#eq?` compares a node's text, `#make-range!` builds a region
from two captures). Each query kind accepts the ones its job needs. A pattern
using one HUME does not apply is disabled rather than matched as if it held,
since matching it anyway would color or select the wrong thing; the rest of
its file still loads, and the editor warns once, naming the file and line.

## Locals: coloring a name by its definition

A highlight pattern sees one node at a time, so it cannot tell that the `x`
on line 40 is the parameter declared on line 12. A **locals query** supplies
that: it marks scopes (a function body, a block), the definitions inside them
(a parameter, a `let`), and the uses of names. Each use then resolves to the
definition it can see: the latest one before it with the same name, searching
its own scope first and then each scope around it. The use takes that
definition's color, so a parameter keeps its parameter color wherever it is
used, and a highlight pattern can ask whether a node is local at all.

Resolving every name on every keystroke would be wasteful. After a reparse,
only the scopes an edit could have changed are rebuilt: the innermost scope
around each edited byte, since a definition is only visible inside its own
scope. An edit outside every scope rebuilds the whole layer.

## Injections: embedded languages

One document often contains more than one language. A Markdown file has
fenced code blocks in Rust or Python; an HTML page has `<script>` tags full of
JavaScript; a Rust doc comment contains Markdown. The Markdown grammar knows
*where* a fenced block is, but it cannot parse Rust. Its tree only says "here
is a code block whose info string reads `rust`."

**Injection queries** bridge that gap. Like a highlight query, an injection
query is a `.scm` file of patterns compiled against the host grammar, but its
captures say "this node's text is another language: parse it with that
language's grammar." The embedded language is sometimes named in the query
(a fixed choice, like Markdown's inline grammar) and sometimes read from the
document (the ` ```rust ` info string of a fence).

### Layers

With injections, a buffer holds a **root tree** (the buffer's own language)
plus a set of **injected layers**. Each layer is a full parse by another
grammar, restricted to the byte ranges the injection query marked. Layers can
nest (a Markdown fence containing Rust containing a doc comment containing
Markdown again), so injections resolve recursively, with a small depth cap so
a pathological document cannot recurse forever.

Two details make layers more than "a tree per region":

- **Combined layers.** Some embedded languages are scattered across the
  document in many small spans that only make sense parsed together.
  Markdown's inline grammar (bold, italic, inline code) is the common case:
  every paragraph contributes a span, and all of them are parsed as one layer
  covering many disjoint ranges rather than one layer per paragraph.
- **Depth priority.** Where layers overlap, the deeper layer's highlighting
  wins. Inside a Rust fence, Rust's `@keyword` beats whatever Markdown would
  say about those bytes, because the most specific parse of a region is the
  one the user should see.

Missing grammars degrade gracefully. If a fence names a language whose
grammar isn't installed, that region keeps the host language's highlighting,
and installing the grammar later lights it up.

### Incremental parsing, layer by layer

An edit does not reparse the whole document from scratch. The root tree is
updated incrementally: tree-sitter reuses every part of the old tree the edit
did not touch. Injected layers follow the same idea. After the root is
reparsed, each layer is matched against the layer of the previous parse with
the same grammar, the same nesting depth and the same ranges (shifted by the
edit). A matching layer the edit did not touch is kept as it is; one the edit
did touch is reparsed incrementally from its old tree. A layer with no match,
such as a fence the user just typed, is parsed fresh.

## Text objects: selecting and navigating by structure

A highlight query answers "what is this node?" An injection query answers
"what language is this region?" A third kind, a **text-object query**,
answers "where does the function, class, comment or argument around this
cursor begin and end?" It is the same mechanism (a `.scm` file of patterns
compiled against the grammar) put to a different use: its captures name whole
structural objects for selection and navigation.

### One object, several captures

A structural object is not always one node. A function preceded by an
attribute or decorator is one selectable unit, even though the grammar
represents the two as adjacent nodes. A block comment can be a run of several
single-line comment nodes. A query pattern can tag several nodes with the same
object name in one match, and the object is the span from the earliest node's
start to the latest node's end. This is what lets "select the function"
include its attribute.

A pattern can also build an object explicitly: one directive makes it run
from the start of one capture through the end of another, and another trims
bytes off its ends, so an object can leave out a block's braces.

That hull is only meaningful when the tagged nodes sit side by side. A loosely
written pattern ("one or more nodes of this kind") can match two nodes far
apart, and hulling those would produce an "object" stretching across most of
the file. So a multi-node match counts as an object only when its nodes are
adjacent; otherwise the match is discarded.

### Selecting: smallest wins

When you select "the enclosing function", several functions may contain the
cursor: an inner closure inside an outer method, say. The smallest span that
contains the cursor wins. This is the intuition "select the closest enclosing
thing", and it handles nesting of any depth without a special case.

### Navigating: nearest start wins

Jumping to the next or previous object asks a different question: not "which
object contains the cursor" but "which object's boundary is closest in this
direction". Moving forward, that is the object whose start comes soonest
after the cursor; moving backward, the one whose start comes soonest before
it. Keying both directions on the start means a backward jump from inside an
object lands on that object's own beginning first, then walks back to earlier
ones.

### Why layering needs no special handling

A Markdown file with a fenced Rust block has a Markdown tree and a Rust tree
(see [Layers](#layers) above), each queried for structural objects. An object
found in the Rust layer sits inside the fence, which sits inside the
Markdown structure, so "smallest span wins" already prefers it over anything
coarser Markdown offers at the same position. A layer whose language defines
no structural objects contributes nothing.

### Why the tree must be current first

A structural query answers in positions: where a span starts and ends. If the
buffer was edited since the tree was last built, those positions describe
where the code was, not where it is. Highlighting can tolerate a tree that
lags an edit by a frame or two, since nobody sees the difference for that
long. A structural command cannot: it would select the wrong span, or a
position past the end of the text. So a structural command brings the tree up
to date with the buffer before it queries it, parsing on the spot if the
background parser has not caught up.

## Plum: the grammar manager

`plum` is HUME's plugin manager. It is written in Scheme and ships with the
editor as a core plugin, not a separate program. It manages plugins, themes
and grammars. For grammars it provides:

- **`:plum-install-grammar`** installs (or repairs) a language's grammar, by
  default the current buffer's.
- **`plum/ensure-grammars`** installs the grammars of a list of languages
  that are not installed yet. It is called from `init.scm`, for example
  `(call! "plum/ensure-grammars" '("rust" "json"))`, not typed at the prompt.
- **`:plum-list-grammars`** shows the installed grammars and any leftovers,
  such as older installs a reinstall could not delete yet.
- **`:plum-cleanup-grammars`** deletes the leftovers, keeping only the
  current install of each language's grammar.

HUME compiles no grammar. A separate project builds every grammar for each
supported platform and publishes them together as a release: one archive per
platform, holding one small archive per language, plus an index of where each
language's archive sits and its checksum. Plum downloads only the slice for
the language it installs and checks it against the index.

A language's archive holds the shared library, its query files, and a
manifest naming the library's tree-sitter symbol, the ABI version and the
platform it was built for. Grammars are installed per language, so two
languages sharing one grammar (JavaScript and JSX) each get their own copy.

Grammars can also come from a folder on your own machine, the output of the
grammar builder run locally: one archive per language, no index. Plum looks
there first, language by language, and downloads only what the folder
doesn't have.

A loaded shared library stays loaded until the editor exits, and the
operating system will not load a different file from the same path again
(Windows won't even let it be deleted). So plum never overwrites a grammar in
place. Each install gets a new folder of its own, and the newest checked
folder is the grammar in use. That is what lets a reinstall take effect in
the session that ran it.

An install runs these steps in order:

1. **Companion grammars first.** Markdown's emphasis and inline code live in
   a separate inline grammar that Markdown's injection query expects to find,
   so installing Markdown installs that one too.
2. **Unpack.** Plum unpacks the archive, checks that the manifest names the
   right language and this platform, and moves it into a new install folder.
3. **Load before registering.** Plum loads the library and compiles the
   queries. A grammar the editor can't use, such as one built for a
   tree-sitter ABI version it can't load or one whose highlight query
   doesn't compile, is rejected here. An optional query that doesn't compile
   only leaves that query out, with a warning. Only a grammar
   that loads gets its folder marked as checked, and only a checked folder
   counts as an install, so a rejected grammar never replaces a working one.
4. **Register and clean up.** Plum registers the grammar with the running
   editor and deletes the language's older installs.

On later starts, HUME registers each installed grammar from its newest
checked folder. Startup needs no network and reads only what is present on
disk; a fresh setup with nothing installed reads nothing.

Registering a grammar opens nothing. HUME notes where its library and queries
are, and loads them the first time something needs the grammar: a buffer of
that language, or an embedded block that names it. Grammars a session never
uses cost nothing.

Only the main thread loads a grammar, because a loaded grammar joins the
editor's shared language table. When the background parser meets an
embedded block whose grammar is registered but not loaded, it skips the block
and reports the language. The main thread loads the grammar, and the buffer
resolves its embedded blocks again, keeping every layer that is still valid
and parsing only the new ones. A grammar that fails to load is reported once,
and counts as absent until it is registered again.

## Why pinned revisions

Grammars and their queries are pinned for reproducibility: query patterns
match named nodes the grammar defines, so a changed grammar can make a query
stop matching or match the wrong thing.

HUME pins one thing: a release of the project that builds its grammars. That
release fixes each grammar's revision and the Helix revision its queries come
from, and HUME's own language data is generated from the same Helix revision,
so grammars, queries and language data move forward together.

## Late grammar registration

Grammars can arrive after buffers are open. A user might open a Rust file,
then install the Rust grammar with `:plum-install-grammar` in the same
session.

When a grammar is registered, HUME walks every open buffer. Each buffer whose
language matches goes through syntax setup again and is highlighted once the
background parser answers, with no restart. Buffers whose grammar can inject
other languages resolve their embedded blocks again, so installing the Rust
grammar lights up Rust code fences in a Markdown buffer that was already
open. The same sweep covers buffers the startup sequence opened before the
installed grammars were registered.

## End-to-end: opening a `.rs` file

Here is what happens when you open `main.rs`:

1. **Path resolution.** The path is resolved and a buffer is created with the
   file's contents.

2. **Detection.** No glob matches and the `.rs` extension matches Rust, so
   the language is `"rust"`.

3. **The language funnel.** It writes the name to the buffer, sets up syntax
   parsing if Rust has a grammar, and attaches the buffer to the language
   servers Rust's list names, if any (see [LSP](lsp.md)).

4. **Syntax setup.** The Rust grammar loads now unless something already
   needed it, and the background parser builds the root tree and its
   injected layers off the main thread. A size limit (one mebibyte by
   default) keeps very large files from being parsed at all. It applies
   throughout the session: a buffer that grows past it loses its syntax tree,
   and one that shrinks back under it gets it back without a restart.

5. **`on-language-set`** fires with the buffer and `"rust"`. Plugins can
   react, for example by setting indent width.

6. **`on-buffer-open`** fires next. The language is already set, so its
   handlers can branch on it.

7. **Render.** The renderer's style stage reads the syntax trees once they
   are available (the root plus any injected layers) and walks each with its
   language's shared query to produce per-grapheme style spans, deeper layers
   winning where they overlap. Where the locals query resolved a name, the
   name takes its definition's color. The active theme turns those spans
   into colors.

Before each frame, pending edits are applied to every layer's tree, so the
trees the renderer reads line up with the text on screen even while a fresh
parse is still running in the background.

## Plugins reading the tree

The tree is not only for the editor's own features. A plugin can run a query
of its own over a buffer, across every layer parsed as a given language, and
get back the captured nodes and their positions. The compiled query is
cached, so asking again each time the buffer changes costs only the match. A
hook tells the plugin each time a fresh parse lands, so it can update what it
drew. The Markdown preview is built this way; see [Conceals and the Markdown
Preview](conceals-and-preview.md).

---

*See also: [Language Identity and Detection](language-identity.md) for how a
buffer gets its language; [The Rendering Pipeline](rendering-pipeline.md) for
where highlighting is applied.*
