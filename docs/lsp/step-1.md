# LSP Step 1 — LSP core, Rust data plane (task cards)

The `hume-lsp` crate (transport + client) plus the editor glue that makes a server real: spawn, handshake, document sync, diagnostics in memory. After this step nothing is user-visible except `:lsp-status` and the message log — but the data plane is complete.

Read `docs/LSP.md` (hub) first. Layering rule for the whole step: **`hume-lsp` never names `Editor`, `Buffer`, or anything in `hume-editor`/`hume-engine`** — it speaks `BufferId`-free protocol types plus opaque metadata the editor glue attaches. Editor-side glue lives in a new `hume-editor/src/editor/lsp/` module.

---

### C1 — Crate scaffold

**Goal** — `hume-lsp/` workspace member with the module skeleton and the dependency fence.

**Depends** — P1. **Unlocks** — P5, P6, C2.

**Files** — `hume-lsp/Cargo.toml`, `hume-lsp/src/lib.rs` (module decls + crate docs); root `Cargo.toml` (`workspace.members`).

**Read first** — `hume-treesitter/Cargo.toml` (the precedent, including the `test-util` feature pattern — don't add the feature yet, just know it exists); hub *Decisions* rows "Crate boundary" and "Protocol types".

**Shape**
```toml
[package]
name = "hume-lsp"
version = "0.1.0"
edition = "2024"

[dependencies]
hume-editing = { path = "../hume-editing" }
lsp-types = "<latest 0.9x — check crates.io; 0.96+ has the new Uri type>"
serde.workspace = true
serde_json.workspace = true
ropey.workspace = true
```
`lib.rs` declares (empty for now, filled by later cards): `pub mod uri; pub mod codec; pub mod transport; pub mod client; pub mod sync;`.

**Tests** — `cargo tree -p hume-lsp` output contains neither `hume-editor` nor `hume-engine` (record the check in the commit message; there is no automated fence — review enforces it, same as `hume-treesitter`).

**Done when** — crate builds empty; workspace green; dependency fence verified.

**Traps** — don't pre-create speculative modules beyond the five listed; each card adds its own.

**Size** — ~30 lines.

---

### C2 — JSON-RPC codec

**Goal** — `Content-Length` framing (read + write), the three-message enum, request-id allocation. Malformed input fails loudly; nothing ever silently skips bytes.

**Depends** — C1. **Unlocks** — C3, C4.

**Files** — `hume-lsp/src/codec.rs`.

**Read first** — hub primer *Wire format*; `serde_json::value::RawValue` is **not** needed — plain `Value` params are fine at these rates.

**Shape**
```rust
/// Ids: we allocate integers; servers may use strings for their own requests.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId { Int(i64), Str(String) }

#[derive(Debug)]
pub enum Message {
    Request { id: RequestId, method: String, params: serde_json::Value },
    Response { id: RequestId, result: Result<serde_json::Value, ResponseError> },
    Notification { method: String, params: serde_json::Value },
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ResponseError { pub code: i64, pub message: String, pub data: Option<serde_json::Value> }

pub enum CodecError { Io(std::io::Error), MissingLength, BadHeader(String), Json(serde_json::Error), Ambiguous }

/// Blocks until one full frame is read. Any error is fatal for the
/// connection (C3 logs + treats as crash) — never resynchronize.
pub fn read_message(r: &mut impl BufRead) -> Result<Message, CodecError>;
pub fn write_message(w: &mut impl Write, msg: &Message) -> std::io::Result<()>;

pub struct IdAllocator(i64);
impl IdAllocator { pub fn next(&mut self) -> RequestId; }
```
Parsing strategy: headers = lines until the empty `\r\n` line; only `Content-Length` matters (tolerate and ignore others, e.g. `Content-Type`). Body → an intermediate raw struct `{ id, method, params, result, error }` (all `Option`), then classify: method+id = Request, method = Notification, id+result/error = Response, anything else = `Ambiguous`. Hand-rolling the classification beats fighting serde untagged enums on overlapping shapes.

**Tests** — tier 1. Round-trip every variant (int and string ids); a response with `error`; missing `Content-Length` → error; garbage header line → error; body shorter than declared length → Io error (read_exact); two messages back-to-back in one reader read cleanly; unknown extra JSON fields ignored; `params` absent (legal for notifications) → `Value::Null`.

**Done when** — round-trips green; every malformed case returns `Err` (assert no partial reads leave the stream "resynced").

**Traps**
- `Content-Length` counts **bytes**, not chars — UTF-8 body with multi-byte content in a test.
- Write side must emit `\r\n\r\n` exactly — `\n\n` breaks picky servers.
- Don't buffer-then-parse whole stdout; `read_message` pulls exactly one frame (the reader thread loops it).

**Size** — ~150 source + ~150 test lines.

---

### C3 — Server process management

**Goal** — spawn a server over piped stdio; reader/writer/stderr threads bridging to mpsc channels; deterministic teardown.

**Depends** — C2. **Unlocks** — C4 (threaded impl), C5.

**Files** — `hume-lsp/src/transport.rs`.

**Read first** — `hume-treesitter/src/parse_worker.rs` `ThreadedParseBackend` — thread + channel ownership, the `Option<Sender>` close-to-signal pattern, `Drop` ordering. This card is that pattern with a child process attached.

**Shape**
```rust
pub enum InboundEvent {
    Message(Message),
    Stderr(String),          // one line, already utf8-lossy
    /// Reader hit EOF or a codec error — the connection is dead.
    Eof { error: Option<String> },
}

pub struct ServerHandle {
    tx: Option<mpsc::Sender<Message>>,     // writer thread input; None after close
    rx: mpsc::Receiver<InboundEvent>,      // reader + stderr threads output
    child: std::process::Child,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl ServerHandle {
    /// Spawns the process (cwd = workspace root) + three threads.
    pub fn spawn(cmd: &str, args: &[String], root: &Path) -> std::io::Result<ServerHandle>;
    pub fn send(&self, msg: Message);              // ignore send-after-death
    pub fn try_recv_all(&mut self) -> Vec<InboundEvent>;
}
// Drop: close tx (writer exits) -> child.kill() -> child.wait() -> join threads.
```
Reader thread: `loop { read_message → send(Message) }`, on any error send `Eof` and exit. Writer thread: `for msg in rx { write_message; flush }`. Stderr thread: `BufRead::lines → Stderr` events.

**Tests** — tier 1 where possible: the thread loops factored to take `impl BufRead`/`impl Write` so they're testable with in-memory pipes (`io::Cursor`, `Vec<u8>`) without a child. One `#[cfg(unix)]` smoke test spawning `/bin/cat` (frames echo back verbatim; drop leaves no zombie — `child.wait` returns). Everything protocol-level is C4-double territory, not here.

**Done when** — echo smoke green on unix; drop kills + reaps + joins (no leaked threads under `cargo test`); send-after-death is a silent no-op (the crash path already reported via `Eof`).

**Traps**
- `child.wait()` after `kill()` or the process zombifies.
- Writer must `flush()` per message — servers block on partial frames.
- Don't read stdout and stderr from one thread — that deadlocks on full pipes; three threads is the design, not an option.
- stderr is unstructured text — never parse it, just forward lines (C10 logs them).

**Size** — ~150 source + ~100 test lines.

---

### C4 — `LspBackend` trait + `InlineLspBackend` double

**Goal** — the seam the editor holds (`Box<dyn LspBackend>`, mirroring `parse_worker: Box<dyn ParseBackend>`) and the synchronous scripted double that powers every editor/Steel test from here to F11.

**Depends** — C2 (types); C3 (threaded impl). **Unlocks** — C5…C10, all Step 2/4 tests.

**Files** — `hume-lsp/src/backend.rs` (trait + threaded impl over `ServerHandle`s), `hume-lsp/src/inline.rs` (double).

**Read first** — `ParseBackend` trait + `InlineParseBackend` (`hume-treesitter/src/parse_worker.rs`); how the editor constructs and holds `parse_worker` (`editor/mod.rs`, search `parse_worker:`).

**Shape**
```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ServerId(pub u32);

pub trait LspBackend {
    /// Spawn (threaded) or register (inline) a server. Handshake is the
    /// client layer's job (C5) — this is transport-level only.
    fn start(&mut self, cmd: &str, args: &[String], root: &Path) -> std::io::Result<ServerId>;
    fn send(&mut self, server: ServerId, msg: Message);
    /// All events that arrived since the last drain, in arrival order.
    fn drain(&mut self) -> Vec<(ServerId, InboundEvent)>;
    /// Any in-flight request or undrained event? Feeds P3's wake predicate.
    fn has_pending(&self) -> bool;
    fn shutdown(&mut self, server: ServerId);   // transport teardown
}

pub struct ThreadedLspBackend { servers: HashMap<ServerId, ServerHandle>, next: u32 }

/// Deterministic test double: scripted responses, no process, no threads.
pub struct InlineLspBackend {
    /// method -> FIFO of canned results; a request pops one and enqueues
    /// the Response event immediately.
    responses: HashMap<String, VecDeque<Result<serde_json::Value, ResponseError>>>,
    /// Everything the editor sent, for assertions.
    pub sent: Vec<(ServerId, Message)>,
    queue: VecDeque<(ServerId, InboundEvent)>,
}
impl InlineLspBackend {
    pub fn respond_to(&mut self, method: &str, result: serde_json::Value);
    pub fn fail_with(&mut self, method: &str, code: i64, msg: &str);
    /// Server-initiated traffic (publishDiagnostics, server->client requests).
    pub fn push_from_server(&mut self, server: ServerId, msg: Message);
    /// Convenience: canned successful `initialize` result with the standard
    /// v1 ServerCapabilities; tests override single capabilities as needed.
    pub fn with_default_handshake() -> Self;
}
```
`has_pending` for the inline double: `!queue.is_empty()`.

**Tests** — tier 1 for the double's own bookkeeping (respond_to FIFO order, sent log). The double's real test is *being used* by C5+ tests.

**Done when** — editor field `lsp: Box<dyn LspBackend>` exists (default `InlineLspBackend` in tests, `ThreadedLspBackend` in `run`) wired into P3's `AsyncSource` enumeration; a scripted `initialize` round-trip test passes end-to-end through the editor.

**Traps**
- Invest in the double's ergonomics **now** — 30+ later tests sit on it; a clunky double taxes every one.
- The double must deliver responses on `drain`, not inline inside `send` — callers depend on the drain boundary (same discipline as `InlineParseBackend`).
- Keep the trait transport-flavored: no capabilities, no text_gen, no buffer knowledge (that's C5/C6 client state above it).

**Size** — ~200 source + ~80 test lines.

---

### C5 — Lifecycle

**Goal** — per-server client state machine: `initialize` handshake with capability + position-encoding negotiation, graceful shutdown, crash detection.

**Depends** — C4. **Unlocks** — C6, C7, C8.

**Files** — `hume-lsp/src/client.rs` (state machine, capability storage); editor glue touchpoints land in C8.

**Read first** — hub primer *Lifecycle*; `lsp_types::{InitializeParams, ClientCapabilities, ServerCapabilities, InitializedParams}` docs for the chosen crate version.

**Shape**
```rust
pub enum ServerState { Starting, Running, ShuttingDown, Crashed, Dead }

pub struct LspClient {
    pub id: ServerId,
    pub state: ServerState,
    pub caps: Option<lsp_types::ServerCapabilities>,
    pub encoding: PositionEncoding,      // negotiated; Utf16 until proven Utf8
    pub root: PathBuf,
    queued: Vec<Message>,                // didOpens etc. arriving while Starting
}

impl LspClient {
    /// Builds InitializeParams and sends the request. Advertised client caps:
    /// general.positionEncodings = ["utf-8", "utf-16"]
    /// textDocument.synchronization (incremental)
    /// textDocument.{publishDiagnostics, hover, completion, signatureHelp,
    ///   rename, references, definition/declaration/typeDefinition/
    ///   implementation, formatting, rangeFormatting, codeAction, inlayHint}
    /// completion.completionItem.snippetSupport = false   // v1 strips snippets
    /// hover.contentFormat = ["plaintext", "markdown"]     // plaintext preferred, v1
    /// workspace.{applyEdit, configuration}
    /// workspaceFolders + rootUri both set (compat: rootUri is deprecated but
    /// older servers still read it).
    pub fn start_handshake(&mut self, backend: &mut dyn LspBackend);
    /// Feed one inbound event; returns actions for the glue (e.g. "now
    /// Running — flush queue, fire on-lsp-attach").
    pub fn on_event(&mut self, ev: InboundEvent) -> Vec<ClientAction>;
    pub fn begin_shutdown(&mut self, backend: &mut dyn LspBackend);
}
```
On the `initialize` response: store caps; `encoding = Utf8` iff `caps.position_encoding == Some("utf-8")`; send `initialized`; state → Running; drain `queued`. Reader `Eof` in any state → Crashed (glue reports to message log; restart stays manual per the OQ default). Editor quit: `begin_shutdown` for every Running server, drain replies for ≤500 ms, then transport `shutdown` regardless (Drop kills stragglers).

**Tests** — tier 2 against the double: full handshake (assert exact `initialize` params snapshot — capabilities are load-bearing config, snapshot-test them with `insta` or a golden JSON); messages sent while Starting are queued and flushed after `initialized`; utf-8 negotiated when offered, utf-16 otherwise; Eof → Crashed + no panic on subsequent sends; shutdown sequence order (`shutdown` request → `exit` notification).

**Done when** — handshake round-trips against the double and against rust-analyzer manually (record in commit message); capabilities readable (B3 will expose them); crash detection reports once, not per-frame.

**Traps**
- Buffers can open before Running — the `queued` vec is not optional; test it.
- `positionEncoding` absent from caps = UTF-16. Never default to utf-8.
- Don't advertise capabilities v1 doesn't implement (e.g. `workspace/didChangeWatchedFiles`, snippets) — servers will use whatever you advertise.

**Size** — ~220 source + ~180 test lines.

---

### C6 — Request bookkeeping + server→client dispatch

**Goal** — pending-request map with drain-time deadlines, `text_gen` staleness, `$/cancelRequest`, and the server→client request dispatcher (hub decision: answered in Rust, never Steel).

**Depends** — C5. **Unlocks** — C7 (drain plumbing), B2.

**Files** — `hume-lsp/src/client.rs` (pending map, correlation); editor glue `hume-editor/src/editor/lsp/mod.rs` (drain step inside P3's phase, callback dispatch, server-request answers).

**Read first** — hub decision *Server→client requests*; parse-worker staleness discipline (`editor/syntax/parse.rs` — how stale `ParseDone`s are dropped); P3's drain phase.

**Shape** — split across the crate boundary:
```rust
// hume-lsp (no editor knowledge): correlation + metadata round-trip.
pub struct RequestMeta {
    pub method: String,
    /// Editor-attached staleness token: (buffer, text_gen at send).
    /// None => request is position-independent (exempt from staleness).
    pub gen_tag: Option<(u64 /*opaque buffer key*/, u64)>,
    pub allow_stale: bool,
    pub deadline: Instant,
    pub token: CallbackToken,   // opaque u64 the editor maps to a callback
}
impl LspClient {
    pub fn send_request(&mut self, backend: &mut dyn LspBackend,
                        method: &str, params: serde_json::Value,
                        meta: RequestMeta) -> RequestId;
    pub fn cancel(&mut self, backend: &mut dyn LspBackend, id: RequestId);
    /// Called at drain: correlated responses + timed-out entries.
    pub fn take_completed(&mut self, now: Instant) -> Vec<(RequestMeta, Outcome)>;
}
pub enum Outcome { Ok(serde_json::Value), Err(ResponseError), TimedOut }
```
Editor glue at drain, for each completed entry: timed-out → log (Trace) + drop + send `$/cancelRequest`; gen-tagged and buffer moved past the tag and `!allow_stale` → drop silently (parse-worker discipline); otherwise dispatch by `token` (C-tasks use Rust closures in a `HashMap<CallbackToken, Box<dyn FnOnce…>>` on the editor; B2 adds Steel callbacks in the same table). In-flight requests count as `has_pending` (C4 trait) so drains run at poll cadence — deadline checks piggyback on that; no timer thread.

Server→client **requests** (dispatch table in the glue):
| Method | Answer |
|--------|--------|
| `workspace/configuration` | per requested item, the section-matching slice of the C8 `settings` blob, else `null` |
| `workspace/applyEdit` | v1-before-B6: `{ applied: false, failureReason: "workspace edits not supported yet" }`; B6 swaps in the real engine |
| `client/registerCapability` / `unregisterCapability` | empty success (acknowledge, ignore) |
| `window/workDoneProgress/create` | empty success |
| anything else | error `-32601` MethodNotFound |

Server→client notifications: `window/logMessage` / `window/showMessage` / `$/progress` → message log (C10 formats); `publishDiagnostics` → C9; the rest → B2's `on-lsp-notification` (until B2 exists: drop silently is **not** ok — log at Trace).

**Tests** — tier 2 (double): response dispatched to the right callback; timeout fires at drain after deadline (mock time by constructing `RequestMeta` with a past deadline); stale response dropped (edit the buffer between send and drain), `allow_stale` opt-out delivers; `$/cancelRequest` emitted on timeout and explicit cancel; every server→client request in the table gets exactly one response (assert on `sent`); unknown server request → MethodNotFound.

**Done when** — the table above is exhaustive in code (a `match` with an error arm, no `_ => {}` silent drop); timeout knob reads `lsp.request-timeout-ms` from settings (default 10 000 — rust-analyzer's first requests during indexing are slow).

**Traps**
- **Every** server request gets a response, even the unhandled ones — a server awaiting a reply can stall its whole pipeline.
- Deadline checks happen at drain; if nothing else is pending the loop blocks on input and time stops — that's fine (no request can complete while blocked either), don't add a wake-up just for timeouts.
- `CallbackToken` indirection is deliberate: `hume-lsp` must not hold editor closures (crate fence).

**Size** — ~250 source + ~220 test lines.

---

### C7 — Document sync glue (hume-editor)

**Goal** — buffers mirror to servers: `didOpen` on attach, incremental `didChange` on every text mutation (undo/redo included), `didSave` on write, `didClose` on `:bd`. Version = `text_gen`, no second counter. Pure protocol — zero Steel involvement.

**Depends** — C5, C6, P6. **Unlocks** — C9 correctness, all features.

**Files** — `hume-editor/src/editor/lsp/sync.rs` (+ calls from the mutation/save/close paths).

**Read first** — `editor/buffer/mod.rs`: every method that mutates text or bumps `text_gen` (search `text_gen`); how edits flow — find where command execution applies a `ChangeSet` to the focused buffer (start from an edit command in `ops/edit/` and trace upward to the editor apply site); `fire_hook_buffer_save` (`editor/scripting_setup.rs`) marks the save path; `:bd` handler in `editor/commands/typed_buffer.rs`.

**Mimic** — how `reparse_stale_buffers` reacts to `text_gen` drift (`editor/syntax/parse.rs`) — LSP sync is the same "text changed, notify the machinery" shape, but *eager per mutation* (LSP needs the ChangeSet itself, not just staleness).

**Shape**
```rust
impl Editor {
    /// Call wherever a ChangeSet has just been applied to `bid`
    /// (the single mutation chokepoint — see Read first; undo/redo
    /// return ChangeSets too and MUST route through here).
    fn lsp_did_change(&mut self, bid: BufferId, cs: &ChangeSet, before: &Rope);
    fn lsp_did_open(&mut self, bid: BufferId);    // full text, languageId = Buffer.language
    fn lsp_did_save(&mut self, bid: BufferId);
    fn lsp_did_close(&mut self, bid: BufferId);
}
```
`lsp_did_change` = P6 convert (with the server's negotiated encoding) + `didChange` notification with `version = text_gen as i32`. Buffer reload / `set_text` paths (no ChangeSet) send the whole-document change event form (`TextDocumentContentChangeEvent` without `range` — legal per spec). If no server is attached to the buffer's language/root: all four are no-ops.

**Tests** — tier 2 (double). The load-bearing one: **version-sync invariant** — after any scripted editing session (typed keys covering insert, delete, paste, undo, redo), replaying the recorded `didOpen` + `didChange` stream against a plain string mirror reproduces the buffer text exactly, and the last version equals `text_gen`. Plus: didOpen on attach carries full text + language id; didSave/didClose fire once on `:w`/`:bd`; no notifications for buffers without a server.

**Done when** — the invariant test is green across the whole edit-command matrix (reuse an existing command-coverage test list if one exists); manual: rust-analyzer stops flagging a fixed error as you type (proof didChange streams correctly).

**Traps**
- **Undo/redo**: they apply ChangeSets through their own path — if the chokepoint you found doesn't cover them, hook their apply site too; the invariant test catches the miss.
- The remap order with C9 matters: remap stored diagnostics through the ChangeSet (C9) *and* send didChange with the same ChangeSet — same source, both consumers.
- `text_gen` is `u64`, protocol version is `i32` — cast, don't invent a counter; wraparound is theoretical (2³¹ edits) and staleness only compares locally.
- `didSave` may want `includeText` if the server registered for it — v1: never include text (don't advertise `didSave.includeText`).

**Size** — ~120 source + ~200 test lines.

---

### C8 — Server registration

**Goal** — `register-lsp-server!` from Steel (init-only, queued like language regs); config side-table keyed by language; workspace-root resolution; spawn-on-first-open per the (language, root) decision.

**Depends** — C5. **Unlocks** — C7 attach flow, F11 user config.

**Files** — `hume-scripting/src/builtins/lsp.rs` (new; primitive `%register-lsp-server!`), `hume-scripting/src/builtins/mod.rs` (register + bootstrap wrapper with keyword args), `hume-scripting/src/types.rs` (or wherever `PendingLanguageReg` lives — put `PendingLspServerReg` beside it), `hume-scripting/src/lib.rs` (queue field + take method), `hume-editor/src/editor/lsp/registry.rs` (config table, root walk, spawn trigger).

**Read first** — the **whole** `PendingLanguageReg` chain: `%define-language!` in `hume-scripting/src/builtins/syntax.rs` → queue on `ScriptingHost` → `flush_pending_language_regs` (`editor/syntax/mod.rs`) → applied after init eval (`editor/scripting_setup.rs`). C8 is a clone of that chain. Also the bootstrap-wrapper pattern in `builtins/mod.rs` (how `declare-plugin` converts `#:keyword` args to a positional `%primitive!` call).

**Shape**
```rust
// hume-scripting side:
pub struct PendingLspServerReg {
    pub language: String,
    pub command: String,
    pub args: Vec<String>,
    pub root_markers: Vec<String>,       // e.g. ["Cargo.toml", ".git"]
    pub init_options: Option<serde_json::Value>,   // decoded from Steel via B1… 
    pub settings: Option<serde_json::Value>,       // …until B1 exists: accept
}                                                  // a JSON *string*, parse here
// hume-editor side:
pub struct LspServerConfig { /* same fields minus language */ }
// Editor fields (the cmd_owners pattern — a plain side table, NOT a field
// on LanguageConfig, which lives in hume-treesitter):
//   lsp_configs: HashMap<String /*language*/, LspServerConfig>
//   lsp_servers: HashMap<(String, PathBuf) /*(language, root)*/, ServerId>

/// Walk up from `file`'s directory to the first ancestor containing any
/// root marker; fall back to cwd.
fn resolve_root(file: &Path, markers: &[String], cwd: &Path) -> PathBuf;
```
Steel surface (bootstrap wrapper → positional primitive):
```scheme
(register-lsp-server! "rust"
  #:command "rust-analyzer"
  #:args '()
  #:root-markers '("Cargo.toml")
  #:init-options #f      ; JSON string or #f until B1; B1 upgrades to data
  #:settings #f)
```
Spawn trigger (per the hub decision row): on buffer open **and** on language-set, if `lsp_configs` has the language and `lsp_servers` lacks the `(language, resolve_root(file))` key → `backend.start` + C5 handshake, insert; then `lsp_did_open` (C7) either way. Second registration for the same language → loud error (OQ default).

**Tests** — tier 2/3. Root walk: marker in parent / in grandparent / nowhere (→ cwd) / marker is a directory (`.git`); init-only enforcement (calling at command time errors like `%define-language!` does); duplicate language registration errors; scripted end-to-end: registration in an init snippet + opening a matching file starts exactly one server, second file same root attaches (no second `start`), file in a different root starts a second instance.

**Done when** — the end-to-end scripted test passes; `:lsp-status` (C10) shows (language, root) pairs.

**Traps**
- Init-only means **queued**, applied after the init eval completes — copy the flush timing exactly; a half-registered server visible mid-init is the bug the pattern prevents.
- `resolve_root` must handle a file with no parent (buffer with no path → cwd, no server? v1: unnamed buffers never attach — document that).
- Don't store configs in `hume-treesitter`'s `LanguageConfig` — the hub decision says side table, and the crate doesn't know LSP exists.

**Size** — ~220 source + ~180 test lines.

---

### C9 — Diagnostics store (Rust-ingested)

**Goal** — `publishDiagnostics` lands in a per-(server, buffer) store: converted to char offsets at ingest, coalesced per frame, remapped through every subsequent edit. **Bulk never reaches Steel** — Steel gets a signal + bounded pulls (B5).

**Depends** — C6, C7, P2, P4, P5. **Unlocks** — U1/U2/U3/U8, B5, F4.

**Files** — `hume-editor/src/editor/lsp/diagnostics.rs`.

**Read first** — hub decision *Bulk-data guardrail*; P2's `map_ranges`; how `update_highlight_providers` materializes per-frame data (`editor/lifecycle.rs` or `ui/highlight_providers.rs` docs) — U1 will read this store the same way.

**Shape**
```rust
pub(crate) struct StoredDiag {
    pub start: usize,            // char offsets, kept remapped through edits
    pub end: usize,
    pub severity: DiagSeverity,  // Error | Warning | Info | Hint (map protocol ints)
    pub message: String,
    pub code: Option<String>,
    pub source: Option<String>,  // "rustc", "clippy", …
}

#[derive(Default)]
pub(crate) struct DiagnosticsStore {
    by_buffer: HashMap<BufferId, Vec<(ServerId, Vec<StoredDiag>)>>,  // sorted by start
    /// bumped on every change; cheap "did anything change" for U-consumers
    pub generation: u64,
}

impl DiagnosticsStore {
    /// Ingest one publishDiagnostics (already coalesced — see drain note).
    fn replace(&mut self, server: ServerId, bid: BufferId, diags: Vec<StoredDiag>);
    /// P2 remap on every edit to `bid` (called next to lsp_did_change).
    fn remap_through(&mut self, bid: BufferId, cs: &ChangeSet);
    pub fn counts(&self, bid: BufferId) -> (usize /*errors*/, usize /*warnings*/);
    pub fn for_range(&self, bid: BufferId, range: Range<usize>, floor: DiagSeverity)
        -> impl Iterator<Item = &StoredDiag>;
}
```
Drain-time coalescing: within one drain batch, keep only the **last** `publishDiagnostics` per (server, uri) — servers burst-publish and only the newest matters. Ingest conversion: URI → path (P5) → canonicalize → buffer lookup (no open buffer → drop, v1); positions via P4 with that server's negotiated encoding. Zero-length protocol ranges (start == end) widen to one char at ingest (HUME selections/decorations are never empty; clamp at line end).

**Tests** — tier 2 (double). Ingest converts positions per encoding (utf-16 emoji case); coalescing (two publishes same drain → last wins); remap: insert before / inside / after a diagnostic range moves/grows/keeps it, deletion covering it collapses and the collapsed entry is dropped; counts; `for_range` respects severity floor and range bounds; unknown-URI publish is dropped without error spam (one Trace line max).

**Done when** — with rust-analyzer manually: introduce an error, `:lsp-status` (C10) shows a nonzero count; fix it, count returns to zero; edit *above* the error line and the stored range stays glued to the text (verified via U1 once it lands; until then via the remap tests).

**Traps**
- Remap must run for **every** ChangeSet including undo/redo — hook the same chokepoint as C7 (one caller, two consumers).
- Store order (sorted by start) is an invariant `for_range` and U1 rely on — re-sort after remap (ranges can't reorder under mapping, but a debug_assert is cheap insurance).
- Don't fire per-diagnostic Steel hooks — one `on-diagnostics-changed` signal per drain batch (B7), payload-free.

**Size** — ~200 source + ~220 test lines.

---

### C10 — Observability + lifecycle commands

**Goal** — `:lsp-status`, `:lsp-stop`, `:lsp-restart` typed commands; server stderr and `window/logMessage` into the message log with a server-name prefix.

**Depends** — C5, C8. **Unlocks** — the Step 1 milestone checkpoint.

**Files** — `hume-editor/src/editor/commands/typed_lsp.rs` (new), `editor/registry/defaults.rs` (`typed_cmd!` entries), `editor/lsp/mod.rs` (log routing in the drain).

**Read first** — `typed_messages` in `editor/commands/typed_misc.rs` (the read-only-view pattern: `format → open_read_only_view`); `typed_cmd!` registration block in `editor/registry/defaults.rs`; `Editor::report` severities.

**Shape**
```rust
/// :lsp-status — read-only view "[lsp-status]": one block per server:
///   rust @ ~/code/hume — Running (pid …), 2 in flight, caps: utf-8, root: …
/// plus per-buffer attach lines and diagnostic counts (C9).
pub fn typed_lsp_status(ed: &mut Editor, _arg: Option<&str>, _force: bool) -> Result<(), CommandError>;
/// :lsp-stop [language] — graceful shutdown (C5); no arg = focused buffer's server.
pub fn typed_lsp_stop(…);
/// :lsp-restart [language] — stop + clear (language, root) entry + respawn via C8.
pub fn typed_lsp_restart(…);
```
Log routing at drain: `Stderr(line)` → `report(Trace, "rust-analyzer: {line}")`; `window/logMessage` type Error/Warning → matching severity, Info/Log → Trace; `window/showMessage` → Info (user-facing by protocol intent); `$/progress` → begin/end at Trace, per-report messages dropped (OQ default).

**Tests** — tier 2 (double): status text lists a Running server with root and pending count (snapshot the format); stop transitions to Dead and sends shutdown/exit in order; restart yields a fresh ServerId and a fresh handshake; stderr lines appear in the message log prefixed.

**Done when** — Step 1 milestone checkpoint passes end-to-end (hub: *Milestone checkpoints*).

**Traps**
- rust-analyzer logs a *lot* — Trace severity keeps `:messages` usable; don't promote stderr to Info.
- `:lsp-restart` must reuse C8's spawn path (config lookup + handshake), not duplicate it.

**Size** — ~140 source + ~120 test lines.
