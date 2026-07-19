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

## Keeping the document in sync

The server never reads files off disk on its own — the editor is the source
of truth for anything open, and tells the server about every change.
`textDocument/didOpen` sends the full text once. After that,
`textDocument/didChange` sends *incremental* edits: small range-and-replacement
descriptions rather than the whole file again. Each event in a batch is
addressed against the document as it stood after the previous events in
that same batch, not against the original text — the same "replay changes in
order" idea covered in [Changesets](changesets.md). Every message also
carries a version number, so a server that's slow to respond can have its
answer discarded if the document has since moved on.

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

HUME splits LSP support along a frequency line: the parts that run on every
keystroke or every message (framing the wire protocol, syncing document
edits, storing and remapping diagnostics, filtering completion candidates)
are built in as fast primitives, while the actual *features* — what hover
shows, how goto-definition results are presented, what rename's prompt looks
like — are ordinary plugin code, following the same activation and hook
model as any other plugin (see
[Plugin Architecture](plugin-architecture.md), including the
`on-lsp-attach` / `on-lsp-detach` hooks fired when a server connects to or
drops a buffer).

A server is associated with a language by registering it — either by hand or
through the bundled install catalog (`:lsp-install`) — and HUME runs at most
one server instance per (language, workspace root) pair, finding the root by
walking up from the file toward a marker like `.git`.

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
