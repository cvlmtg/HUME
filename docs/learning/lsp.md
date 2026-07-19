# LSP: One Protocol Between Editors and Languages

## The M×N problem

Every language has its own smarts: how to find a definition, what a symbol's
type is, where a variable is used, how to reformat a file. Before the
Language Server Protocol, an editor that wanted these features for ten
languages had to hand-roll ten integrations, and a language that wanted them
in ten editors had to write ten plugins — an M×N problem.

LSP flips this into M+N. A *language server* is a separate process that
understands one language deeply and exposes that understanding over a fixed
protocol. An editor implements the protocol once and gets every language that
has a server for it — rust-analyzer, gopls, pyright, and so on. HUME spawns
these servers as ordinary child processes and talks to them over their
stdin and stdout.

## The wire format

Messages are JSON-RPC 2.0, each one framed by a `Content-Length` header
followed by a blank line and exactly that many bytes of JSON. There are three
kinds of message:

| Kind | Has an id? | Expects a reply? |
|------|-----------|-------------------|
| Request | yes | yes — a matching response |
| Response | matches a request's id | — |
| Notification | no | no |

Either side can send any of the three. Most traffic is the editor asking
things of the server (`textDocument/hover`, `textDocument/completion`), but
the server also pushes notifications to the editor unprompted — diagnostics
are the main example — and occasionally asks something of the editor itself,
such as "apply this edit" or "here is my progress on indexing."

## The handshake

The very first exchange is `initialize`: the editor sends a request
describing everything it's capable of (can it show markdown in hover
popups? does it support workspace-wide renames?), and the server replies
with what *it* supports. Every feature after that is only used if both
sides advertised it — capabilities degrade gracefully rather than assuming.
Only after the editor sends the `initialized` notification back is the
connection considered live; anything queued before that point is held and
flushed right after.

One capability negotiated during the handshake is *position encoding*.
LSP was designed with JavaScript in mind, so its historical default measures
positions in UTF-16 code units. HUME asks for UTF-8 instead and only falls
back to UTF-16 if a server doesn't support it — see
[Unicode Position Model](unicode-position-model.md) for why the encoding a
position is measured in matters at all.

In shape, the opening exchange looks like this (abridged — real payloads
carry far more than shown):

```
→ initialize    { capabilities: { hover: [plaintext, markdown],
                                   rename: true,
                                   positionEncodings: [utf-8, utf-16] } }
← response      { capabilities: { hoverProvider: true,
                                   renameProvider: true,
                                   positionEncoding: utf-8 } }
→ initialized   {}
                 # connection now live — anything queued during the
                 # handshake flushes right after this
```

The server's reply only ever grants what it actually supports — if it had
left `renameProvider` out, HUME would treat rename as unavailable for that
server rather than sending a request into the void.

## Keeping the document in sync

The server never reads files off disk on its own — the editor is the source
of truth for anything open, and tells the server about every change.
`textDocument/didOpen` sends the full text once. After that,
`textDocument/didChange` sends *incremental* edits: small range-and-replacement
descriptions rather than the whole file again. Each event in a batch is
addressed against the document as it stood after the previous events in
that same batch, not against the original text — the same "replay changes in
order" idea covered in [Changesets](changesets.md), and worked through in
detail below. Every message also carries a version number, so a server
that's slow to respond can have its answer discarded if the document has
since moved on.

## Translating edits, in both directions

HUME's own edits and the server's edits are described differently, and
neither side speaks the other's language natively — something has to
translate.

**Outbound**, a HUME edit is a [changeset](changesets.md): a sequence of
retain/delete/insert steps describing how the buffer changed. To become
`didChange` events, that sequence is replayed step by step against a working
copy of the *pre-edit* text, and each step's range is measured against
whatever that working copy looks like after the previous steps — not
against the original text. Skipping this and measuring every step against
the original document is the classic bug: the second edit in a batch lands
at the wrong offset because the first edit already shifted everything after
it.

```
working = copy of buffer before this edit
for each step in changeset:
    if step is delete/insert:
        emit didChange range = step's position in `working`
        apply step to `working`     # so the *next* step measures correctly
```

**Inbound**, a server hands back edits as a list of ranges-and-replacement-text
— from a rename, a format request, a code action. HUME resolves every one of
those positions against its own text model first (see
[Unicode Position Model](unicode-position-model.md) for why that resolution
step matters), sorts them, and rejects any that overlap. Only once every
position is pinned down does it compose all the edits into a single
changeset and apply that as one atomic step — one undo entry, one pass
through selection adjustment and re-parsing, and one outbound `didChange`
describing the result. This sidesteps the older trick of applying edits
back-to-front so earlier offsets don't shift out from under later ones —
HUME never needs that, because it resolves positions before composing
rather than applying them one at a time.

A rename that touches several files follows the same idea at a larger
scale: every file's edits are validated before any file is touched, so a
bad edit in one file doesn't leave the rest half-applied.

## Push vs. pull

Diagnostics — errors, warnings, lints — are *pushed*: the server sends them
whenever it feels like it, unprompted, and the editor just displays whatever
arrived most recently. HUME shows them as gutter signs, inline underlines, an
end-of-line summary, and an error/warning count in the statusline; jump
between them with `g n` / `g p`, or list them all with `:diagnostics`.

Everything else is *pulled* — the editor asks, the server answers once:

| Feature | Key |
|---------|-----|
| Hover info | `g k` |
| Goto definition / declaration / type / implementation | `g d` / `g D` / `g y` / `g i` |
| Find references | `g R` |
| Rename symbol | `g r` |
| Code actions | `g a` |
| Completion | `ctrl-space` (insert mode) |
| Format buffer | `:lsp-fmt` |
| Inlay hints | off by default; `:set global lsp.inlay-hints=true` |

## When servers misbehave

Every request carries a deadline, so a server that never answers doesn't
hang the editor forever. Some requests can be superseded — completion
re-filters on every keystroke, and each new request cancels the one still in
flight rather than piling up. If a server process dies outright, HUME marks
it dead and stops routing buffers to it; it does **not** restart
automatically. That's a deliberate choice — a crash loop caused by a bad
project config shouldn't spin silently in the background. Restart by hand
with `:lsp-restart`.

## Who does what in HUME

HUME's core doesn't know what "hover" or "rename" *is*. It provides a general
LSP platform — spawn servers, speak the wire protocol, keep documents in
sync, store and remap diagnostics, filter completion candidates as the user
types — all as fast primitives that run on every keystroke or every message.
Every feature the user actually sees — what hover shows, how
goto-definition results are presented, what rename's prompt looks like, how
code actions are offered — is ordinary plugin code, following the same
activation and hook model as any other plugin (see
[Plugin Architecture](plugin-architecture.md)). A feature plugin is almost
always the same small shape: send a request, transform the response, call a
UI primitive.

This split is deliberate, not incidental. Keeping the core feature-blind
means a new feature, or a variation on an existing one, is a plugin change
rather than a recompile — and it means built-in features have no special
access that a user's own config or a third-party plugin lacks. Only the
things that must run on every keystroke (parsing the wire, keeping the
server's copy of the text current, filtering completion candidates) earn a
place in the fast core; everything else is free to live where it's cheap to
change.

A server is associated with a language by registering it — either by hand or
through the bundled install catalog (`:lsp-install`) — and HUME runs at most
one server instance per (language, workspace root) pair, finding the root by
walking up from the file toward a marker like `.git`.

## Three parties: server, editor core, plugin

A plugin never touches the wire directly. To ask the server something, it
hands the core a request — a method name, its parameters, and a callback —
and the core takes it from there: it flushes any pending document edits
first (so a request can never race ahead of the change it depends on),
tags the request with an id, tracks its deadline, sends it, and later runs
the callback with whatever came back. The plugin never blocks waiting for
an answer — the callback fires whenever the response arrives. Two related
hooks, `on-lsp-attach` and `on-lsp-detach`, fire when a server connects to
or drops a buffer.

Two safety valves keep stale answers from causing damage. If the document
has moved on by the time a response arrives, the callback can be skipped
instead of acting on positions that no longer mean anything. And a plugin
can supersede its own previous request instead of piling both up — this is
how completion re-filters on every keystroke without a backlog of stale
answers trailing behind.

Server-initiated traffic doesn't all reach plugins the same way:

| Comes from the server | Handled by | Reaches plugins? |
|---|---|---|
| Diagnostics | core stores and remaps them | yes — a hook fires so plugins can react; rendering reads the store directly |
| Progress, log/status messages | core only | no |
| "Apply this edit" | core applies it, using the same edit path as everything else | no |
| Anything else unrecognized | core forwards it | yes, to whatever plugin registered for it |

Diagnostics are the one case worth dwelling on: the core owns storage and
position remapping (so a diagnostic keeps pointing at the right line as the
buffer changes), and rendering — gutter signs, inline underlines, the
statusline count — reads that store directly, no plugin involved. What
*is* a plugin's job is reacting to the fact that diagnostics changed:
deciding what `:diagnostics` should list, or what `g n` / `g p` jump to.

## End-to-end: opening a Rust file

1. Open `main.rs`. HUME detects its language is Rust (see
   [Language Identity and Detection](language-identity.md)).
2. A server is registered for Rust, so HUME resolves the workspace root by
   walking up to the nearest `.git`.
3. No server is running yet for that (language, root) pair, so HUME spawns
   one and starts the `initialize` handshake.
4. Once `initialized` is sent, HUME flushes `didOpen` for `main.rs` with its
   full text.
5. The server starts indexing the project; the statusline shows a spinner
   fed by its progress notifications.
6. As indexing finds problems, `publishDiagnostics` notifications arrive
   unprompted — the gutter and statusline update to show them.
7. From here, hover, goto-definition, and completion are one request away,
   answered from whatever the server has indexed so far.
