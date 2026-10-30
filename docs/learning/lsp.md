# LSP: One Protocol Between Editors and Languages

## The M×N problem

Every language has its own smarts: how to find a definition, what a symbol's
type is, where a variable is used, how to reformat a file. Before the
Language Server Protocol, an editor that wanted these features for ten
languages had to hand-roll ten integrations, and a language that wanted them
in ten editors had to write ten plugins: an M×N problem.

LSP turns this into M+N. A *language server* is a separate process that
understands one language deeply and exposes that understanding over a fixed
protocol. An editor implements the protocol once and gets every language that
has a server: rust-analyzer, gopls, pyright, and so on. HUME runs these
servers as ordinary child processes and talks to them over their standard
input and output.

## The wire format

Messages are JSON-RPC 2.0, each framed by a `Content-Length` header, a blank
line, and that many bytes of JSON. There are three kinds of message:

| Kind | Has an id? | Expects a reply? |
|------|-----------|-------------------|
| Request | yes | yes, a matching response |
| Response | matches a request's id | no |
| Notification | no | no |

Either side can send any of the three. Most traffic is the editor asking the
server things (`textDocument/hover`, `textDocument/completion`), but the
server also sends notifications unprompted (diagnostics, indexing progress)
and occasionally asks the editor something, such as "apply this edit".

## The handshake

The first exchange is `initialize`. The editor sends a request describing
what it can do (can it show markdown in hover popups? does it support
workspace-wide renames?), and the server replies with what *it* supports. A
feature is used only if both sides advertised it. Only after the editor sends
the `initialized` notification is the connection live; anything queued before
that is held and sent right after.

One capability negotiated here is *position encoding*. LSP was designed with
JavaScript in mind, so its default measures positions in UTF-16 code units.
HUME asks for UTF-8 and falls back to UTF-16 only if a server doesn't support
it. See [Unicode Position Model](unicode-position-model.md) for why the unit a
position is measured in matters.

In shape, the opening exchange looks like this (abridged; real payloads carry
far more):

```
→ initialize    { capabilities: { hover: [plaintext, markdown],
                                   rename: true,
                                   positionEncodings: [utf-8, utf-16] } }
← response      { capabilities: { hoverProvider: true,
                                   renameProvider: true,
                                   positionEncoding: utf-8 } }
→ initialized   {}
                 # connection now live: anything queued during the
                 # handshake is sent right after this
```

The server's reply grants only what it supports. Had it left out
`renameProvider`, HUME would treat rename as unavailable for that server
rather than send a request nobody answers.

## Keeping the document in sync

For a file the editor has open, the server never reads the disk: the
editor's copy is the source of truth, and the editor reports every change.
(The rest of the project the server still reads from disk while indexing.)

`textDocument/didOpen` sends the full text once. After that,
`textDocument/didChange` follows the sync form the server asked for during
the handshake. Most servers ask for *incremental* sync: small
range-and-replacement events rather than the whole file again. Each event in
a batch is addressed against the document as it stands after the previous
events in that batch, not against the original text (worked through below).
A server that asks for *full* sync gets no ranges at all: every edit since
the last flush is collapsed into one event carrying the whole document,
because a ranged event would put its copy out of step from the first edit. A
server that asks for no sync gets no `didChange` at all. Every message
carries a version number, so an answer computed for an older version can be
recognized and discarded.

## Translating edits, in both directions

HUME's edits and the server's edits are described differently, so each
direction needs a translation.

**Outbound**, a HUME edit is a [changeset](changesets.md): a sequence of
keep, delete and insert steps. To become `didChange` events, the steps are
replayed one by one against a working copy of the *pre-edit* text, and each
step's range is measured against the working copy as the previous steps left
it. Measuring every step against the original text instead is the classic
bug: the second edit in a batch lands at the wrong offset, because the first
one already shifted everything after it.

```
working = copy of buffer before this edit
for each step in changeset:
    if step is delete/insert:
        emit didChange range = step's position in `working`
        apply step to `working`     # so the next step measures correctly
```

**Inbound**, a server hands back edits as a list of ranges and replacement
texts, from a rename, a format request or a code action. HUME first resolves
every position against its own text model (see
[Unicode Position Model](unicode-position-model.md)), sorts the edits, and
rejects any that overlap. Then it composes them into a single changeset and
applies it as one step: one undo entry, one pass of selection adjustment and
reparsing, and one outbound `didChange`. The usual technique of applying
edits back to front, so earlier offsets don't shift under later ones, is not
needed, because every position is resolved before anything is applied.

A server describes a file that may not end with a newline, while a HUME
buffer always does. An edit that would drop the final newline keeps it under
the rule every other edit follows: text inserted at the end gets a newline of
its own, and when nothing is inserted there, deletions stop before the
structural newline. See [Buffer Invariants](buffer-invariants.md).

A rename that touches several files follows the same idea at a larger scale:
every file's edits are validated before any file is touched, so a bad edit in
one file doesn't leave the rest half-applied.

## Push and pull

Diagnostics (errors, warnings, lints) are normally *pushed*: the server sends
them whenever it has them, and the editor shows the latest set from each
server. Some servers only answer when asked. HUME asks a server that offers
such answers and has not pushed anything yet: once per version of the text,
again after a save, and again when the server says its diagnostics changed.
HUME
shows diagnostics as gutter signs, underlines, an end-of-line summary and an
error and warning count in the statusline; `g n` / `g p` jump between them
and `:diagnostics` lists them all.

Everything else is *pulled*: the editor asks, the server answers once.

| Feature | Key |
|---------|-----|
| Hover info | `K` |
| Goto definition / declaration / type / implementation | `g d` / `g D` / `g y` / `g i` |
| Find references | `z r` |
| Rename symbol | `G R` |
| Code actions | `z a` |
| Completion | `ctrl-space`, or automatically on trigger characters (insert mode) |
| Signature help | automatic, as you type a call (insert mode) |
| Format buffer | `:format-source` |
| Inlay hints | off by default; `:toggle-inlay-hints` per buffer |

The keys follow what a request asks for, not what it returns. A goto jumps to
a place, so it lives with the other jump keys. References and code actions
open a panel, so they live with the other panel keys. Hover is looked up
often enough to earn a bare key.

## When servers misbehave

Every request carries a deadline, so a server that never answers doesn't
hang the editor. Some requests can be superseded: as the user types,
completion may need a fresh request (when the server said its list was
incomplete), and each new request cancels the one still in flight. If a
server process dies, HUME marks it dead and stops sending it requests. It
does not restart it automatically, so a crash loop caused by a bad project
config doesn't spin unnoticed in the background. `:lsp-restart` restarts it
by hand.

## Who does what in HUME

HUME's core doesn't know what "hover" or "rename" *is*. It provides a general
LSP platform: spawn servers, speak the wire protocol, keep documents in sync,
store and remap diagnostics, and filter completion candidates as the user
types. These are fast primitives that run on every keystroke or every
message. Every feature the user sees (what hover shows, how goto results are
presented, what rename's prompt looks like, how code actions are offered) is
ordinary plugin code, following the same activation and hook model as any
other plugin (see [Plugin Architecture](plugin-architecture.md)). A feature
plugin usually has the same small shape: send a request, transform the
response, call a UI primitive.

Keeping the core feature-blind means a new feature, or a variation on an
existing one, is a plugin change rather than a recompile. It also means the
built-in features have no access that a user's config or a third-party
plugin lacks. Only the work that runs on every keystroke (parsing the wire,
keeping the server's copy current, filtering completion candidates) lives in
the core; everything else lives where it is cheap to change.

A server is registered by hand or through the bundled install catalog
(`:lsp-install`), and registering it only says how to start it. Each language
has a list of the servers it uses, and a buffer attaches to every server on
its language's list. HUME runs one instance per server and workspace root,
finding the root by walking up from the file to the nearest directory holding
a marker the *language* names (`Cargo.toml` for Rust, say), or using the
working directory when there is none. Two languages that share a server, such
as TypeScript and TSX, share its instance whenever their files resolve to the
same root.

Putting the language's list in charge, rather than the registration, is what
lets one language use several servers: a server for the code itself and a
linter beside it, for example. Diagnostics from all of them show together,
completion and code actions combine their answers, and a request like hover
goes to the first server on the list that can answer it.

## Three parties: server, editor core, plugin

A plugin never touches the wire. To ask the server something, it hands the
core a request (a method name, its parameters and a callback), and the core
does the rest: it sends any pending document edits first (so a request never
races ahead of the change it depends on), tags the request with an id, tracks
its deadline, sends it, and later runs the callback with the answer. The
plugin never blocks waiting. Two hooks, `on-lsp-attach` and `on-lsp-detach`,
fire when a server starts or stops serving a buffer.

Three options keep stale answers from doing damage. If the document has moved
on by the time a response arrives, the callback can be skipped instead of
acting on positions that point at other text. A plugin can ask the core to
track a position through edits, so the request asks about where the symbol is
now and the callback acts on the right place. And a plugin can supersede its
own previous request instead of piling both up, which is how completion
avoids a backlog when further typing forces a fresh request. (The common case
needs no request at all: the core re-filters the candidates it already has as
the user types.)

Traffic the server starts reaches plugins in different ways:

| Comes from the server | Handled by | Reaches plugins? |
|---|---|---|
| Diagnostics | core stores and remaps them | yes: a hook fires so plugins can react; rendering reads the store directly |
| Progress, log and status messages | core only | no |
| "Apply this edit" | core applies it through the same edit path as everything else | no |
| A request for settings, or to register a capability | core answers it | no |
| Any other notification | core forwards it through the `on-lsp-notification` hook | yes, to every handler on that hook, or only to one registered for specific methods |
| Any other request | core answers "not supported" | no: every request gets one response |

Diagnostics show the split most clearly. The core owns storage and position
remapping, so a diagnostic keeps pointing at the right text as the buffer
changes, and rendering (gutter signs, underlines, the statusline count) reads
that store directly with no plugin involved. A plugin's job is reacting to
the change: deciding what `:diagnostics` lists or where `g n` / `g p` jump.

A results list is plugin work too. The list a find-references or multi-match
goto opens shows the positions the server gave, and edits can make them
wrong. When an edit in a listed file could move a listed line (it changes the
line count, touches a listed line, or adds or removes lines above one), the
LSP plugin marks the list stale and, once typing pauses, asks the server
again. The request's own position was tracked through the edits, so the new
question is about the symbol where it is now. An open completion menu, by
contrast, needs no new request: it stays open when text elsewhere in the
buffer changes, and still accepts at the cursor, because the position it is
anchored to is carried through the change.

## End-to-end: opening a Rust file

1. Open `main.rs`. HUME detects that its language is Rust (see
   [Language Identity and Detection](language-identity.md)).
2. Rust's list names a server, so HUME resolves the workspace root by walking
   up to the nearest directory holding one of Rust's root markers, such as
   `Cargo.toml`.
3. No instance of that server is running for that root yet, so HUME spawns
   one and starts the `initialize` handshake.
4. Once `initialized` is sent, HUME sends `didOpen` for `main.rs` with its
   full text.
5. The server starts indexing the project; the statusline shows a spinner fed
   by its progress notifications.
6. As indexing finds problems, `publishDiagnostics` notifications arrive
   unprompted, and the gutter and statusline update.
7. From here, hover, goto-definition and completion are one request away,
   answered from whatever the server has indexed so far.

---

*See also: [Changesets](changesets.md) for the edits HUME sends and applies;
[Plugin Architecture](plugin-architecture.md) for how the LSP features load as
a plugin.*
