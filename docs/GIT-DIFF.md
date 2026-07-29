# `git-diff` for HUME — implementation plan

> **Status.** This document is the agreed design. The plugin itself (Phase 5) is *not*
> to be built yet — everything below it is prerequisite work, tracked here as it lands.

| Phase | State |
|---|---|
| 1 — async subprocess execution | ✅ shipped (`2451d3d6`…`8bcb4606`) |
| theme `diff.*` scopes | ⚠️ partial — all 4 themes, **fg only**, no `bg` |
| 2 — native diff builtins | ⬜ **next** |
| 3 — engine: full-row background | ⬜ |
| 4.1 — `on-text-changed` hook | ⬜ |
| 4.2 — buffer-text reads | ⬜ |
| 4.3 — async git | ✅ subsumed by Phase 1 |
| 4.4 — line-bg decoration kind | ⬜ |
| 4.5 — `set-virtual-lines!` `Before` anchor + per-segment scopes | ⬜ **new — needed by 5b, see "Two contradictions" below** |
| 5a — the plugin: state/fetch/debounce/init + gutter signs | ⬜ not to be built yet |
| 5b — virtual deleted lines (+ word-del highlights inside them) | ⬜ not to be built yet |
| 5c — full-row background tint | ⬜ not to be built yet |

## Context

[`inline-diff.nvim`](https://github.com/cvlmtg/inline-diff.nvim) (local copy at
`/Users/matteo/dev/neovim-inline-diff`) is a live, VSCode-style inline git diff. As you
type it compares the buffer against a git ref (default `HEAD`) and renders:

- **Deleted lines** as virtual rows (struck / red) — content that is no longer in the buffer.
- **Added & changed lines** with a full-width background tint.
- **Word-level highlights** inside changed lines (Myers diff on tokens).
- **Gutter +/- signs** — a first-class rendering of the same hunk data as the three above,
  not an afterthought. *(Not actually in the nvim reference plugin — verified zero
  `sign_text`/`sign_hl` calls across its ~870 Lua lines. This is HUME-side scope beyond
  parity; see `docs/ROADMAP.md`'s git-gutter decision row.)*

The nvim version is ~700 lines of Lua plus the native primitives it leans on: `vim.diff`
(line diff, native C), `vim.system` (async git), `vim.uv` timers (debounce), and extmarks
(virtual lines, inline highlights). Architecturally: **only the line-diff is native; all
logic is Lua.**

**Goal:** reproduce this in HUME, written **as much as possible in Steel**. Only the diff
*algorithm* goes native (mirroring nvim). Everything else — state, debounce, git
orchestration, render/decoration construction, word-level diff — lives in Steel.

The feature is secondary. The real objective is to **stress-test how complete HUME's
plugin system is for a real-world plugin**, and to build the *general, reusable*
primitives that this exercise reveals are missing — primitives that future features
(LSP, file watchers, auto-save, blame, diagnostics) will reuse.

### Why gutter signs are merged into this plugin, not a separate one

Signs and inline rendering are two renderings of the *same* underlying hunk data, produced
by the same `git show`/`diff-buffer-lines` pipeline. Shared: repo probe, ref fetch via
`spawn-async!`, line diff, debounce, ref-cache invalidation, hunk-equality check to skip
no-op refreshes. Differing: a `set-signs!` call versus the virtual-line / tint / word-span
construction — roughly 20% of the plugin, not enough to justify splitting the other 80%.

A separate plugin would also spawn its own `git show` per buffer and diff independently,
opening a window where the gutter and the inline view disagree about the same file. Steel
has no cross-plugin state-sharing mechanism beyond `call!` into another plugin's registered
commands (`runtime/plugins/core/stdlib/plugin.scm` is the only precedent) — sharing a hunk
store across two plugins means inventing a protocol for it, which is more complexity than
the merge it would be avoiding.

### What HUME already has (verified by exploration)

- **Rendering substrate** in `hume-engine/src/providers.rs`: `VirtualLineSource`,
  `HighlightSource` (tiered byte-range spans), `GutterColumn`, `OverlayProvider`, all
  aggregated per-pane in a `ProviderSet`.
- **Theme scopes** `diff.plus` / `diff.minus` / `diff.delta` (+ `.gutter` variants) are now
  defined in all four bundled themes — `sand.toml:58-63`, `dark.toml:40-45`,
  `light.toml:41-46`, `gruvbox.toml:40-45` — but every entry is **`fg`-only**. Phase 5c's
  full-row tint needs a `bg`; see "What is missing" below. The `.gutter` variants are
  currently byte-identical to their base scope in every theme (redundant until a sign
  wants a different color — the dot-fallback chain in `theme/mod.rs:284` already covers
  that case for free). No Rust code resolves any `diff.*` scope by name today — the six
  entries are reachable only via the tree-sitter `diff` grammar's own `highlights.scm`
  when a `.patch`/`.rej` file is open. No test covers any `diff.*` scope
  (`hume-editor/src/ui/theme/tests.rs:40`'s allowlist and
  `editor/tests/commands.rs:2818` both omit them) — worth closing when Phase 5c lands.
- **`text_gen: u64`** on `Buffer` (`hume-editor/src/editor/buffer/mod.rs:102`) — bumped on
  every mutation inside `set_text` (`buffer/mod.rs:261`); the canonical "buffer changed"
  signal (the syntax cache already keys off it).
- **A background-worker precedent**: the tree-sitter parse worker
  (`hume-treesitter/src/parse_worker.rs`) runs parsing on a `std` thread + `mpsc` channel.
  Draining is ad hoc to that one worker — there's no unified job-completion mechanism.
- **`similar` crate is a real workspace dependency** (`Cargo.toml:49`), consumed today only
  by `hume-editing` (`hume-editing/Cargo.toml:9`). `hume-editing/src/diff.rs` already exposes
  both `diff_lines` (`:149`, Histogram → Myers fallback, 250ms deadline) and `diff_words`
  (`:254`, Myers, 50ms deadline, tokenized via `unicode-segmentation::split_word_bounds()` —
  grapheme-safe). `diff_lines` has one production caller today (`:e!` reload undo history,
  via `changesets_from_line_diff` in `changeset/diff_cs.rs:25,64`, `Buffer::reload_from_text`
  in `hume-editor/src/editor/buffer/mod.rs:369`); `diff_words` has zero production callers —
  implemented and tested, unused. Both are directly reusable — see Phase 2.
- **A timer/debounce subsystem** — `hume-editor/src/editor/timers.rs` (`TimerId`,
  schedule/cancel/take_due) + `hume-editor/src/editor/timer_bridge.rs` (fires due timers
  into Steel thunks or native actions). Steel already has `(after ms thunk)`,
  `(cancel-timer! id)`, and a `debounce` helper (`hume-scripting/src/builtins/timers.rs`,
  registered `builtins/mod.rs:467-468`), already used for debouncing in
  `runtime/plugins/core/lsp/inlay.scm` (`debounce-by`). Only background-job execution
  (async git) is missing — timers themselves are not.
- **A generic, Steel-writable decoration API**: `DecorationHost`
  (`hume-scripting/src/host.rs:409-465`) exposes `set-inlay-hints!`, `set-signs!`,
  `set-virtual-lines!`, `set-extra-highlights!`, `set-inline-diagnostics!`, backed by
  `DecorationStores` (`hume-editor/src/editor/decorations.rs`), source-namespaced. See
  Phase 4.4/4.5 for the remaining gaps (line-background tint; virtual-line anchor/segments).
- **`VirtualLine` already carries per-segment styling — engine-side only.** `VirtualLine.segments:
  Vec<(Range<usize>, ScopeId)>` (`hume-engine/src/providers.rs:228`) and `Grapheme.scope:
  Option<ScopeId>` (`hume-engine/src/types.rs:160`), consumed in `emit_virtual_row`
  (`hume-engine/src/pipeline/pane_render.rs:356-437`). The engine type supports styled,
  segmented, `Before`-or-`After`-anchored virtual lines. **The Steel-facing bridge does not
  yet expose this** — see "Two verified plan-vs-code contradictions" below.
- **A generic plugin highlight tier**: `HighlightTier::Extra`
  (`hume-engine/src/providers.rs:37`) is documented as "generic plugin-supplied spans",
  driven by `set-extra-highlights!`. **Resolved** (Phase 3.3): ranked below
  `SearchMatch`/`Diagnostic` but above `Syntax` — sufficient for word-diff highlights, no
  new tier needed.
- **Full-trust plugin model, no sandbox**: `git-clone`/`curl-fetch`/`npm-install!` Rust
  builtins don't exist; PLUM calls `git clone`/`git pull` directly via Steel's own
  `spawn-process` (`runtime/plugins/core/plum/lib.scm:33`, `plugins.scm:69,91`). There is no
  Rust-enforced sandbox to any directory — any plugin can already `spawn-process` `git` with
  arbitrary args/cwd (see Phase 4.3).
- **The `SharedHighlighter` pattern** (`hume-editor/src/ui/highlight_providers.rs:40,82`,
  `SharedHighlighter`/`ScopedHighlighter`): an `Arc<RwLock<…>>`-backed provider the editor
  refreshes each frame. Relevant for native-only providers; anything Steel-facing goes
  through `DecorationHost` instead.
- **`set-signs!` has no Steel client today.** `grep -rn "set-signs!" runtime/` returns
  nothing — diagnostics signs are produced Rust-side, not through the Steel builtin.
  `git-diff` will be the *first* Steel caller of `set-signs!`, which is exactly what the
  `"vcs"` source-name fixtures already pre-validate:
  `hume-editor/src/editor/tests/lsp_signs.rs:287,320,338,384` and
  `lsp_decorations.rs:241,263,301` use `"vcs"` as their canonical non-LSP sign source.

### What is missing

| nvim primitive the plugin uses | HUME today | must build |
|---|---|---|
| `vim.diff` (line Myers) | ✅ `diff_lines` exists (`hume-editing/src/diff.rs:149`), not exposed to Steel | Steel builtin wrapper, plus a buffer-native `(diff-buffer-lines bid ref-text)` variant so the plugin never has to round-trip the live buffer through Steel just to diff it (Phase 2) |
| word-diff Myers (in *Lua*) | ✅ `diff_words` exists (`hume-editing/src/diff.rs:254`), grapheme-safe, zero production callers today | Steel builtin wrapper (Phase 2) |
| `nvim_buf_get_lines` (live text) | ❌ no buffer-text read — the biggest gap for general-purpose Steel scripts (the diff plugin itself sidesteps this via `diff-buffer-lines`, but other consumers still need it) | buffer-text builtins (Phase 4.2) |
| `autocmd TextChanged` | ❌ no on-edit hook (12-entry `HOOKS` table, none fire on edit) | `on-text-changed` hook (Phase 4.1) |
| `autocmd BufWritePost` | ✅ `on-buffer-save` (`hooks.rs:78`) | reuse |
| `vim.uv` timer (debounce) | ✅ `(after ms thunk)` / `(cancel-timer! id)` / `debounce`, timer wheel in `editor/timers.rs` | reuse — nothing to build |
| `vim.system` (async git) | ✅ shipped — `(spawn-async! cmd args cwd callback)` / `(cancel-async! id)`, one-shot capture, exactly-once callback, never inline (`hume-scripting/src/builtins/process.rs`) | reuse — nothing to build |
| `nvim_buf_set_extmark` (virt_lines_above / inline hl / hl_eol) | ⚠️ **partial.** Engine-side `VirtualLineAnchor::Before/After` and per-segment `ScopeId` styling both exist (`hume-engine/src/providers.rs:202-228`), but the Steel bridge (`Editor::update_virtual_line_providers`, `hume-editor/src/editor/lifecycle.rs:1613-1671`) hardcodes `VirtualLineAnchor::After` and a single whole-line segment — **`Before` and per-segment scopes are unreachable from Steel today.** `set-extra-highlights!`/`set-signs!` are unaffected. | extend `set-virtual-lines!` (or add a variant) to accept an anchor + segment list (Phase 4.5); plus a **line-background** (full-row tint) decoration kind (Phase 3.2 + 4.4) |
| highlight groups | ⚠️ `diff.*` scopes now exist in **all four** bundled themes, but **`fg`-only** — no `bg` for the line/word tint | add `bg` (and `.word` variants) to all four themes |

## Goals

1. ✅ **Shipped.** A general, reusable **async subprocess execution** primitive
   (`spawn-async!`) for async git — not a one-off for diff, and not new scope: it was a
   named, deferred gap in `docs/FUZZY-FINDERS.md`/`docs/COMPLETION-PICKER.md` waiting on a
   second client. See Phase 1 below for the shipped shape.
2. The minimal **native** surface: line-diff and word-diff builtins, both wrapping existing
   `hume-editing` code — no new diff algorithm to write.
3. The **Steel-facing plugin API** filled out: edit hook, live buffer-text reads, and
   background-job-backed async git — each general-purpose. (Decoration API already exists,
   see #4.)
4. A small **engine** change so decorations can add a **line background (full-row tint)**
   kind; virtual-line segment styling and a generic plugin highlight tier (`Extra`) already
   exist at the engine layer (the Steel bridge needs extending — Phase 4.5). `Extra`'s tier
   ranking is confirmed sufficient for word-diff highlights — no new tier needed (Phase 3.3).
5. The `git-diff` plugin itself, written almost entirely in **Steel**, mirroring the
   nvim five-module layout, plus gutter signs as a first-class fourth rendering.

## Decisions (locked)

- **One plugin, `git-diff`, not two.** Gutter signs and inline rendering are both pure
  functions over the same hunk set (`(old-start old-count new-start new-count old-lines
  new-lines)` tuples from `diff-buffer-lines`), fed by one git fetch and one debounce
  cycle. Splitting them would duplicate the shared 80% (state, fetch, debounce,
  invalidation) to isolate the 20% that differs, and would risk the gutter and the inline
  view disagreeing about the same file if each plugin diffed independently.
- **Name: `git-diff`, non-core.** `inline-diff` was too narrow once the plugin also owns
  gutter signs. Ships as `runtime/plugins/git-diff/`, outside `core:` — it keeps
  stress-testing the *public* plugin API (the stated real objective of this exercise, see
  Context above), the same posture `core:pickers` demonstrates for the fuzzy-finder API.
- **Full VSCode parity, plus gutter signs beyond the nvim reference**: virtual deleted lines
  + line backgrounds + word highlights (all three match the nvim plugin) + gutter signs (the
  nvim plugin has none — see the Context section note above).
- **Defaults: signs on, inline rendering off.** Signs are cheap (one `set-signs!` call,
  no line-shifting side effects) and stay on whenever the plugin is active. Inline
  rendering moves virtual rows into the buffer's visual flow, so it is opt-in per buffer.
  Two independent `#:config` keys (e.g. `signs` default `#t`, `inline` default `#f`) and
  two commands (`toggle-git-signs`, `toggle-inline-diff`) rather than one combined toggle.
- **Maximize Steel, native diff algorithms only**: line-diff *and* word-diff are both native —
  `hume-editing::diff::{diff_lines, diff_words}` already implement both (Myers, grapheme-safe
  tokenization, deadline-guarded), so Steel wraps them rather than reimplementing nvim's
  `diff.lua` tokenizer/Myers pass. Steel owns everything else: state, debounce, git
  orchestration, decoration construction. Performance guard is the existing per-call deadline
  (`diff_words`: 50ms) plus its `deadline_hit()` flag, not a token-count threshold — Steel
  checks the flag and falls back to a coarse (line-only) highlight when hit.
- **Async/timer is a reusable subsystem**, not duplicated per feature. No tokio, no async
  runtime, no timer crate. Timers already ship (`editor/timers.rs` + `timer_bridge.rs`);
  `spawn-async!` **shipped**, reusing the file picker's proven wake-based transport (now
  extracted to `hume-platform/src/process/child.rs`) in a one-shot-callback shape
  (`hume-platform/src/process/job.rs`) modeled on LSP's request/callback template — not a
  poll loop, not a rebuild of the timer wheel. See Phase 1 for the shipped module layout,
  builtin surface, and the completed picker migration.
- **Diff crate: `similar`**, indirectly — Steel builtins reach `hume_editing::diff::{diff_lines,
  diff_words}` through a new `DiffHost` capability trait on `EditorHost`, implemented by
  `hume-editor` (which already depends on `hume-editing`), not by adding `hume-editing` as a
  dependency of `hume-scripting` (see Phase 2).
- **Toggle is a Steel-defined command** (`define-command!`), bound via existing `bind-key!`.
  No native command, no hardcoded default keymap entry.
- **Phase 5 is layered for internal testing only — the plugin does not ship until every
  layer is done.** The layers are 5a (state, fetch, debounce, init, plus gutter signs), 5b
  (virtual deleted lines + word-del highlights), 5c (full-row background tint). Layering is
  conditional on strict additivity, below — if a layer can't be added without rewriting an
  earlier one, the remaining work reverts to all-at-once.
- **Additivity constraint on how 5a is written.** 5a's state stores the full hunk tuples
  from `diff-buffer-lines` verbatim — not a signs-shaped derivative of them. Each rendering
  (signs, virtual lines, background tint) is then a pure function `hunks → decoration
  records`, calling exactly one setter. Written this way, 5b and 5c each add one renderer
  function and one setter call; they change nothing in state, fetch, debounce, or
  invalidation. If 5a instead stored a per-line sign map, 5b would need to rebuild the core
  to recover full hunk context, which is the condition under which layering is abandoned.

---

## Phase 1 — Background-job execution (Rust) ⭐ foundation — ✅ SHIPPED

Landed `2451d3d6`…`8bcb4606`. Was the named, deferred `spawn-async` gap from
`docs/FUZZY-FINDERS.md`/`docs/COMPLETION-PICKER.md` — this plugin's `git show`/`git status`
need was the anticipated second client.

### What shipped

- **Shared transport**, extracted from the file picker's proven spawn/reader-thread/wake
  model: `hume-platform/src/process/child.rs` (`spawn_piped`, `read_bounded`/`read_capped`,
  `WakeOnDrop`, `WakeCallback`) — used by both `line_source.rs` (streaming, `g f`) and the
  new `job.rs` (one-shot, below).
- **One-shot capture module**: `hume-platform/src/process/job.rs` —
  `spawn_job(cmd, args, cwd, wake) -> io::Result<SpawnedJob>` (`:80`), two reader threads
  (stdout/stderr, avoiding the pipe-fill deadlock), `try_take_result` (`:189`) fires at most
  once and synthesizes an empty result on channel disconnect so the callback contract holds
  even on a torn-down job; `Drop` kills and reaps the child (`:211`).
- **`AsyncProcessHost` capability trait**: `hume-scripting/src/host.rs:521-570`
  (`spawn_async`, `cancel_async`), accessor `EditorHost::async_process()` at `:131`.
  Implemented in `hume-editor/src/editor/host_impl.rs:661-719`. Job state lives on
  `ConfigState` (`editor/mod.rs:177,181`) so `:reload-config` kills in-flight children for
  free.
- **Steel builtins**, both `cmd`-level (not `open` — unavailable during config init):
  ```scheme
  (spawn-async! cmd args cwd callback)  ; → job id (int); callback (stdout stderr exit-code)
  (cancel-async! id)                    ; idempotent
  ```
  Registered `hume-scripting/src/builtins/mod.rs:498-499`, implemented
  `builtins/process.rs:30-61`.
- **Completion path**: `Editor::drain_async_jobs` (`editor/async_job.rs:34-63`), called from
  `drain_async_sources` (`async_source.rs:61`) each `prepare_frame`
  (`editor/lifecycle.rs:763-764`) → `queue_steel_call(callback, [stdout, stderr, exit_code])`
  → evaluated under the existing watchdog/step-budget path
  (`scripting_setup.rs:295-306`), same as hooks and timers.
- **The git-modified picker (`g m`) is migrated** — `runtime/plugins/core/pickers/plugin.scm:135-164`
  now calls `spawn-async!` directly (commit `1c179d4d`), opening the picker with `#:pending #t`
  and cancelling via `cancel-async!` on dismissal. The proposed migration is done; no
  further work needed here.

### Contracts the plugin must rely on

- `callback` fires **exactly once**, never inline, and never for a job that was cancelled
  after completion raced the cancel.
- A spawn failure (missing binary) does **not** raise — it arrives as a normal callback call
  with `exit-code -1` (`host_impl.rs:689-707`). One outcome path, not two.
- No dedicated `git-show-async`/`git-rev-parse-async` builtins exist or are needed — the
  plugin's `init.scm` calls `spawn-async!` directly for git, the same way
  `runtime/plugins/core/plum/lib.scm:33` wraps sync `spawn-process` for clone/pull today.
  This also resolves Phase 4.3 below.
- For a result that may arrive after its target (buffer, picker) is gone, follow the
  `#:token`/`#:pending` idiom already shipped for pickers
  (`user-manual/docs/plugins.md:363-393`): pass the id `spawn-async!`/`picker!` returned back
  in on dismissal so a late callback becomes a no-op instead of clobbering newer state.
- The tree-sitter parse worker (`hume-treesitter/src/parse_worker.rs`) remains a candidate
  future client of the same transport; migrating it is optional/out of scope here.

---

## Phase 2 — Native line-diff and word-diff builtins (Rust) [`diff-lines`, `diff-buffer-lines`, `diff-words`]

Thin Steel wrappers over `hume-editing`'s existing diff code — no new diff algorithm, no new
`similar` usage. Mirrors both `vim.diff` and nvim's Lua word-diff, natively.

**Routed through the `Host` trait, not a new crate dependency.** Verified dependency graph:
`hume-scripting` depends only on `hume-engine` + `hume-platform` (`hume-scripting/Cargo.toml`),
and the only thing it takes from `hume-engine` is the `BufferId`/`PaneId` newtypes — every
real capability (buffer reads, decorations, LSP info, timers) is reached through
`EditorHost`'s capability sub-traits (`hume-scripting/src/host.rs:97+`), implemented by
`hume-editor`, the only crate that links `hume-editing`. Adding `hume-editing` as a direct
`hume-scripting` dependency would be a first-of-its-kind edge bypassing that boundary.
Instead:

- Add a `DiffHost` capability trait to `hume-scripting/src/host.rs`, alongside
  `BufferHost`/`DecorationHost`/etc., accessed the same way:
  `EditorHost::diff(&mut self) -> Option<&mut dyn DiffHost>`. Read-only methods are already
  precedented on other capability traits (`DecorationHost::diagnostic_counts`,
  `LspHost::lsp_capabilities`) — `Host` mediates any reach outside `hume-scripting`'s own two
  dependencies, not just mutation.
- `hume-editor`'s implementation forwards straight to `hume_editing::diff::diff_lines`
  (`hume-editing/src/diff.rs:149`) and `diff_words` (`:254`) — it already depends on
  `hume-editing`, no new dependency needed there either. Re-verified: `hume-scripting/Cargo.toml`
  still has no `hume-editing` dependency, and `AsyncProcessHost` (Phase 1) is now a same-week
  precedent for routing a capability through a new host trait instead of a new crate edge.
- **Signatures to translate, not to assume flat.** `diff_lines(old: &[&str], new: &[&str]) ->
  LineDiff` (`:149`, plus a `_with_deadline` variant at `:118`) takes **line slices, not a
  joined string** — the builtin (or the `DiffHost` impl) splits. Its output is
  `LineDiff { algo_used, hunks: Vec<LineHunk>, .. }` (`:100`) where `LineHunk { old:
  Range<usize>, new: Range<usize>, kind: LineHunkKind }` (`:84`) and `LineHunkKind = Equal |
  Delete(String) | Insert(String) | Replace { old, new }` (`:67`) — **not** the flat
  `(old-start old-count new-start new-count old-lines new-lines)` tuple this doc originally
  assumed. Keep that flat tuple as the *Steel-facing* shape (so the render port in Phase 5
  stays verbatim against nvim's `_diff_lines`), but the builtin computes it from `LineHunk`
  ranges and drops `Equal` hunks — this is a translation step, not a passthrough.
- `diff_words(old: &str, new: &str) -> WordDiff` (`:254`, `_with_deadline` at `:266`) returns
  `WordHunk` ranges that are **char offsets, explicitly not byte offsets** (`:204`) — matching
  `ExtraHighlightEntry { start, end, scope }`, which is also a char range
  (`hume-editor/src/editor/decorations.rs:66-70`). Any byte-range description elsewhere in
  this doc (Phase 3's `(byte_start, byte_end, ScopeId)`) is a different, later layer — the
  char→byte conversion happens editor-side, never in Steel.
- Builtin `(diff-lines old-text new-text)` calls `ctx.host()?.diff()?.diff_lines(old, new)` →
  list of hunks, each `(old-start old-count new-start new-count old-lines new-lines)`, using
  the **same anchor convention** as nvim's `_diff_lines` (`diff.lua:155`) so the Steel render
  code ports directly: pure deletions anchor at the line after which the deletion appears
  (0 = before the first line); additions/changes use the 1-based new start.
- Builtin `(diff-words old-text new-text)` calls `ctx.host()?.diff()?.diff_words(old, new)` →
  list of word-hunks (char-offset ranges) plus a `deadline-hit?` flag (from
  `WordDiff::deadline_hit()`, `:231` — the underlying `250ms`/`50ms` deadlines at `:39`/`:46`
  are `pub(crate)`/private, so the flag is the only signal Steel ever gets) so Steel can
  degrade gracefully (skip word highlights, fall back to line-level tint only) on a timeout
  instead of guessing a token-count threshold.
- **Builtin `(diff-buffer-lines bid ref-text)`** — the plugin's actual hot-path call, added to
  avoid materializing the whole live buffer as a Steel string on every debounced edit just to
  hand it back to `diff-lines`. `Text` wraps a `ropey::Rope` (`hume-editing/src/text.rs:65`);
  `(buffer-text bid)` would force a full `rope.to_string()` copy every call, while a
  Host-mediated implementation can pull lines via `Rope::lines()` +
  `RopeSlice::as_str()` — zero-copy whenever a line sits inside one chunk (the common case),
  falling back to `.to_string()` only for a line that straddles a chunk boundary. `ref-text`
  stays a plain Steel string since it comes from `git show` and is cached per save/ref-change
  in Steel state, not re-fetched per debounce tick, so it isn't hot.
  - **No line-range parameters** (`range-start`/`range-end`) on any of these three builtins.
    Considered and rejected: a ranged diff can't correctly serve *this* plugin (hunk anchors
    depend on the full line sequence on both sides — diffing a sub-range in isolation against
    a git ref gives wrong anchors unless paired with real incremental re-diffing, a materially
    bigger, riskier feature nvim's own plugin doesn't do either), and a repo-wide check
    (`docs/ROADMAP.md`, `docs/LSP.md`) found no planned feature — no git blame, no merge/
    three-way diff view, no rename preview, no undo-tree visualizer — that wants a sub-buffer
    diff; LSP's incremental sync builds change events straight from the `ChangeSet` that
    already exists (`hume-lsp/src/sync.rs`), never by diffing two snapshots. Range and diff are
    separable concerns: if a future feature ever needs "diff lines X–Y", it composes a ranged
    buffer-read (an optional range on `(buffer-lines bid start end)`, Phase 4.2) with the
    already-generic `(diff-lines old new)` — no new diff-side parameter required.
- Register all three via a new `hume-scripting/src/builtins/diff.rs`, wired into `register_all`
  (`hume-scripting/src/builtins/mod.rs`).
- **No word-diff logic in Steel** — the plugin's `diff` module only calls these builtins and
  builds decoration records from their output; a tokenizer/Myers port from nvim's `diff.lua`
  is unnecessary.

---

## Phase 3 — Engine rendering changes (Rust, `hume-engine`)

Styled virtual lines and a generic plugin highlight tier already exist at the engine layer.
Only the full-row background is an actual engine-side gap (the Steel bridge for virtual
lines has its own gap — see Phase 4.5).

1. **Styled virtual lines — already done, engine-side.** `VirtualLine.segments: Vec<(Range<usize>,
   ScopeId)>` (`hume-engine/src/providers.rs:228`) and `Grapheme.scope: Option<ScopeId>`
   (`hume-engine/src/types.rs:160`) already exist and are consumed in `emit_virtual_row`
   (`hume-engine/src/pipeline/pane_render.rs:356-437`, resolved via
   `compose_ctx.theme.resolve(id)`). Red, struck deleted lines + word-del highlighting inside
   them are representable at this layer with the current API — no engine change needed here.
   (Steel cannot reach this yet — Phase 4.5.)

2. **Provider-driven full-row background.** A real gap, still the one piece of Phase 3 to
   build. Full-width tint has exactly **one** hardcoded producer:
   `hume-engine/src/pipeline/pane_render.rs:326-330` sets `row_bg = is_head_line ?
   theme.ui.cursorline.bg : None`, consumed in `render.rs` at `:122, 174, 231, 276, 294`
   plus the fill helpers at `:96-97` (method) and `:559` (free fn). Generalize the `row_bg`
   decision so a provider can request an edge-to-edge background for a line
   (added/changed lines). Depends on the theme prereq above: `diff.plus`/`diff.minus` are
   currently `fg`-only in all four themes, so `bg` values (plus `.word` variants for the
   word-level boost) need adding before this has a color to render.

3. **A highlight tier for plugin spans — done, no new tier needed.** `HighlightTier`
   (`hume-engine/src/providers.rs:37-45`) is `Syntax=0, Extra=1, SearchMatch=2,
   Diagnostic=3, BracketMatch=4`. **Resolved**: `Extra` — the generic plugin-span tier
   driven by `set-extra-highlights!` — already beats `Syntax`, which is exactly what
   nvim's `priority = 200` buys (overriding treesitter fg). Diff highlights have no reason
   to outrank search matches or diagnostics. Closed — do not add a new tier.

**Caveat.** Inline *inserted* ghost text (LSP inlay hints, Steel-generated text) does not
require `'static` strings and no interning/leaking mechanism exists (checked
`hume-editor/src/ui/inlay_hints.rs`, `hume-editor/src/editor/decorations.rs`). Instead:
`CellContent` derives `Copy` (`hume-engine/src/types.rs:196`) so `Grapheme` stays cheap on
the hot per-frame path; ghost text is copied into a **per-frame text arena**
(`FormatScratch::virtual_texts: String`, `hume-engine/src/format.rs:36`, cleared per line,
`push_arena_text` at `:676-689`), and `CellContent::Virtual`/`Indicator` store `(start: u32,
len: u16)` into it — fully dynamic strings work fine, no lifetime constraint. `VirtualLine.text`
is a plain owned `String` per row (row-granularity, no `Copy` pressure, no arena needed —
`providers.rs:228`). Span highlights store no text at all, just `(byte_start, byte_end,
ScopeId)` over bytes already in the rope. `VirtualLine`'s owned-`String` model is the right
one for diff's deleted-line rendering.

---

## Phase 4 — Steel platform primitives (Rust, the "test surface")

Timer builtins already ship — not covered here. The decoration API already ships as
`DecorationHost`/`DecorationStores` — the remaining gaps are one new decoration kind (line
background) and one bridge fix (virtual-line anchor/segments), not a new store. There is no
Rust-enforced git sandbox to work around (full-trust plugin model). The real gaps are the
edit hook, buffer-text reads, non-blocking git execution (resolved by Phase 1), and the two
items below.

Each is general-purpose. Registered through `register_all`
(`hume-scripting/src/builtins/mod.rs`) and, where they touch editor state, the `EditorHost`
trait (`hume-scripting/src/host.rs`).

1. **`on-text-changed` hook.** Missing — current `HOOKS` table
   (`hume-scripting/src/hooks.rs:75-88`) has 12 entries (`on-buffer-open`, `on-buffer-close`,
   `on-buffer-save`, `on-mode-change`, `on-language-set`, `on-lsp-attach`, `on-lsp-detach`,
   `on-diagnostics-changed`, `on-viewport-change`, `on-trigger-char`, `on-completion-accept`,
   `on-completion-refilter`) and none fire on edit. Add `HookId::OnTextChanged` + Steel name;
   fire from the edit-apply path — `apply_edit`/`apply_edit_grouped`/`apply_edit_regrouped`
   on `Buffer` (`hume-editor/src/editor/buffer/mod.rs:457,477,507`), called from
   `hume-editor/src/editor/doc_ops.rs:106,137,171`, all routing through `set_text`
   (`buffer/mod.rs:261`) which is where `text_gen` is bumped — fire the hook there. Handler
   args `(bid)`. `on-buffer-save` already exists and fires from
   `hume-editor/src/editor/commands/typed_file.rs:261` — reuse it. Optional:
   `on-buffer-reload` for `:e!`.

2. **Buffer text reads.** Still the biggest gap for general-purpose Steel scripts —
   `builtins/buffers.rs` and `BufferHost` (`hume-scripting/src/host.rs:339-376`) remain
   metadata-only (`buffer-path`, `buffer-name`, `buffer-dirty?`, `buffer-generation`,
   `buffer-language`, cursor/selection reads). No text/content/slice/get-text builtin exists
   anywhere. Add `(buffer-text bid)` and `(buffer-lines bid)` returning the **live (dirty)
   in-memory** content. **Confirmed: no separate `buffer-text-gen` builtin needed** —
   `(buffer-generation bid)` already returns the `text_gen` counter
   (`hume-editor/src/editor/buffer/mod.rs:102`, registered `builtins/mod.rs:445`, impl
   `buffers.rs:99-107`, `BufferHost::buffer_generation` at `host.rs:367`) and is already used
   as a generation-paired-read guard by `runtime/plugins/core/lsp/format.scm:21`
   (`expect_gen`) — reuse that pattern rather than adding a second counter.

3. **Async git — ✅ resolved by Phase 1, no new native builtin needed here.** `spawn-async!`
   shipped; the plugin's `init.scm` calls it directly for `git show`/`git rev-parse`, the same
   way `runtime/plugins/core/plum/lib.scm:33` wraps sync `spawn-process` for clone/pull.

4. **Decoration API — extend, don't rebuild.** `DecorationHost`
   (`hume-scripting/src/host.rs:409-465`, seven methods: `set_inlay_hints`, `set_signs`,
   `set_virtual_lines`, `set_extra_highlights`, `set_inline_diagnostics`,
   `diagnostics_for_buffer`, `diagnostic_counts`) is backed by `DecorationStores`
   (`hume-editor/src/editor/decorations.rs:73-89` — five source-namespaced maps:
   `inlay_hints`, `signs`, `virtual_lines`, `extra_highlights`; plus single-owner
   `inline_diagnostics` and a `virtual_lines_generation` counter). Cross-check against the
   nvim extmark uses:
   - **span highlight** (char-relative range + scope) — already covered by
     `set-extra-highlights!` (`ExtraHighlightEntry { start, end, scope }`,
     `decorations.rs:66-70`, char offsets matching `diff_words`' `WordHunk` ranges).
   - **gutter sign** — already covered by `set-signs!`. Note: nvim's own inline-diff plugin
     (`/Users/matteo/dev/neovim-inline-diff`) defines **no signs at all** — this is HUME-side
     scope beyond parity; cross-reference `docs/ROADMAP.md`'s git-gutter decision row.
   - **virtual line** (styled segments, anchored `Before`/`After`) — **not fully covered; see
     Phase 4.5.** The engine type is ready but the bridge is not.
   - **line background** (full-width tint) — confirmed missing, and a real gap.
     `row_bg` still has exactly one hardcoded producer (Phase 3.2); `ExtraHighlightEntry`
     cannot substitute for it — it's a char-range span feeding the highlight pipeline, a
     different channel from `row_bg`. This is the one new decoration kind to add to
     `DecorationStores`, paired with the Phase 3.2 engine change.
   - **Risk note:** `docs/ROADMAP.md` lists an open, not-yet-started item — "Unified
     decoration system — single trait replacing the separate gutter/highlight/virtual-line/
     overlay provider traits; post-LSP, once the surface is stable." Building the new line-bg
     kind now means it rides on the current `DecorationStores` shape and may need adjustment
     when that unification lands. Not a blocker, just a known future churn point.

5. **`set-virtual-lines!` anchor + per-segment scopes — new, needed by Phase 5b.**
   **Verified plan-vs-code gap**, not previously called out: the engine type
   (`VirtualLine { anchor, provider_id, text, segments }`, `hume-engine/src/providers.rs:228`)
   supports `Before`/`After` anchoring and per-segment `ScopeId` styling, but the Steel bridge
   flattens both away. `Editor::update_virtual_line_providers`
   (`hume-editor/src/editor/lifecycle.rs:1613-1671`) does this, verbatim:
   ```rust
   by_line.entry(line).or_default().push(VirtualLine {
       anchor: VirtualLineAnchor::After(line),   // hardcoded
       provider_id: 0,
       text,
       segments: vec![(0..text_len, scope)],     // hardcoded, whole-line
   });
   ```
   and the store it reads from, `VirtualLineEntry` (`hume-editor/src/editor/decorations.rs:44-49`),
   carries only `scope: Option<String>` for the whole line — its own doc comment concedes it
   "predate[s] a segmented-styling API". Concretely, this means:
   - **Deleted lines can only render below the deletion point, never above.** A block deleted
     at the top of the visible range (or above line 0) has nowhere correct to anchor.
   - **A deleted line cannot have word-level highlighting inside it** — no way to mark which
     words were the actual removed tokens versus context.

   Fix: extend `set-virtual-lines!`'s per-entry shape (or add a new builtin) to accept an
   optional anchor (`'before` / `'after`, default `'after` for backward compatibility) and a
   list of `(start end scope)` segments instead of one line-level scope, then thread both
   through `VirtualLineEntry` and `update_virtual_line_providers`. Scoped to Phase 4.5,
   gating Phase 5b only — Phase 5a (signs) and Phase 5c (line background) don't touch this
   API at all.

---

## Phase 5a — The `git-diff` plugin: core + gutter signs (Steel)

> Deferred — documented here for completeness; **not to be built in this pass.**

Prerequisites: **Phase 2 (diff builtins) + Phase 4.1 (`on-text-changed`) only.** Neither
Phase 4.5 (virtual-line bridge) nor Phase 3.2/4.4 (line background) is needed for this layer.

Closest existing structural precedent in-repo: `runtime/plugins/core/lsp/inlay.scm` — uses
`debounce-by`, `register-hook!` (e.g. `'on-viewport-change`, `'on-diagnostics-changed`,
`'on-lsp-detach`), and a decoration setter (`set-inlay-hints!`) — the same shape `git-diff`
needs (`debounce-by` + `register-hook! 'on-text-changed` once it exists + `set-signs!`).
Worth reading before writing `init.scm`.

A plugin under `runtime/plugins/git-diff/`, mirroring the nvim five-module layout but with
`render` reduced, in this layer, to one function: hunks → signs.

- **state**: per-buffer table — enabled flags (`signs-enabled?`, `inline-enabled?`
  independently), ref (default `HEAD`), cached `ref_lines`, `ref_dirty`, **the full hunk
  tuples from `diff-buffer-lines`** (not a signs-derived shape — this is the additivity
  constraint from "Decisions" above), `prev_hunks` for equality-check no-op skipping,
  `generation`, debounce timer id. (← `state.lua`)
- **diff**: ref-content orchestration via `spawn-async!` (`git show <ref>:<path>` in the repo
  root); line diff calls native `(diff-buffer-lines bid ref-text)` (current buffer vs. ref,
  no manual `buffer-text` read needed). Word diff is not called in this layer — that's 5b.
  (← `diff.lua`, logic replaced by native calls, not ported)
- **render** (5a scope): `hunks → sign records → (set-signs! "git-diff" bid signs)`. One
  sign per hunk: `+` at the new-side start for a pure addition, `-` at the anchor line for a
  pure deletion, `~` for a change, scoped `diff.plus.gutter` / `diff.minus.gutter` /
  `diff.delta.gutter` respectively. This is the plugin's first Steel client of `set-signs!`
  (see "What HUME already has" above).
- **init**: `register-hook!` on `'on-text-changed` (→ `debounce-by` using the existing
  `after`/`debounce` builtins), `'on-buffer-save` (mark `ref_dirty`, refresh), buffer
  open/close as needed; define commands `toggle-git-signs` and `toggle-inline-diff`
  (`toggle-inline-diff` is a no-op stub in 5a, wired for real in 5b/5c), optional ref arg;
  fetch ref via `(spawn-async! "git" (list "show" (str ref ":" path)) repo-root cb)`
  (Phase 1/4.3 — no dedicated git builtin, direct `spawn-async!` call), invalidate on save.
  (← `init.lua`)
- **Wiring**: ship under `runtime/`, lazy-load via `(declare-plugin "git-diff" #:commands
  '(...) #:events '(...))` — `#:events` exists on `declare-plugin`
  (`hume-scripting/src/lib.rs:13`, documented `user-manual/docs/plugins.md:244`), so
  activation can be gated on an event rather than only `#:commands`. Caveat carried over from
  the LSP plugin's own docs: a manifest keyed *only* on a hook nothing fires before the
  plugin loads (e.g. `on-lsp-attach`) never activates
  (`runtime/plugins/core/lsp/README.md:57`) — the same trap would apply to gating solely on
  `on-text-changed`, since nothing fires it until some buffer is already open and this
  plugin has already registered for it. Hooks themselves are still registered imperatively
  via `register-hook!` in the plugin's own init code, not declared in the manifest. Suggest a
  binding in `runtime/init.scm.example` via `bind-key!`.

---

## Phase 5b — Virtual deleted lines + word-del highlights (Steel)

> Deferred — not to be built in this pass. Additive on top of 5a per the additivity
> constraint: no change to state, fetch, debounce, or init from 5a.

Prerequisites: **Phase 4.5 (virtual-line `Before` anchor + per-segment scopes)** and **a fix for
the top-line `before`-block disagreement** (see "Open risk" below) — both new requirements this
pass, not previously called out as blocking a shippable layer.

- **diff** (extended): for each changed hunk, also call `(diff-words old-line new-line)`,
  checking `deadline-hit?` to skip word highlights on a timeout and fall back to a whole-line
  scope. No Lua-style tokenizer/Myers port — both diff passes are native (Phase 2).
  (← `diff.lua`, logic replaced by native calls, not ported)
- **render** (5b addition): `hunks → virtual-line records`. Pure delete → one or more virtual
  lines anchored `Before` the new-side insertion point (falls back to `After` the previous
  line where `Before` isn't yet wired, per Phase 4.5's rollout); change → virtual old line
  with word-del segments (from the `diff-words` pass) + word-add span highlights via
  `set-extra-highlights!` on the live line. Three cases exactly as `render.lua`. (← `render.lua`)
- **highlight**: map word-diff kinds to the `diff.*.word` scopes, once they exist (theme
  prereq, Phase 5c's theme work — pull forward only the `.word` fg/bg pairs needed here if
  5b lands before 5c's full theme pass). nvim's runtime HSL derivation (`highlight.lua:61-72`,
  boosting `DiffAdd`/`DiffDelete` bg for word-level contrast) is **not ported** — the theme is
  the single source of truth for color in HUME, so boosted word-level colors are authored
  directly into each theme file instead of computed at runtime. (← `highlight.lua`)
- **init**: `toggle-inline-diff` becomes real (enables the 5b renderer in addition to 5a's
  signs).

**Open risk, not resolved by this doc:** scroll/cursor math counts virtual rows —
`hume-engine/src/format.rs:143` `display_rows_for_line` is the SSOT for "how many display rows
does line N occupy" (`before`/`content`/`after`), consumed by both
`hume-editor/src/editor/scroll.rs:230,313` and `hume-editor/src/editor/cursor.rs:69` on the
wrapped path. Fine-grained scrolling through a virtual block is not handled: it moves
atomically. `hume-engine/src/pipeline/pane_render.rs:171-183` never lets a dropped virtual row
decrement `top_skip_remaining` (that budget only counts `top_line`'s wrap rows), and
`hume-editor/src/editor/scroll.rs:326-342` clamps a cut point inside a `before`/`after` block to
`top_row_offset = 0` rather than splitting it — so a `Before(top_line)` block disappears as a
unit once the real wrap rows scroll into `top_line`. Acceptable for a 1-3 row deleted-line
block; no Steel builtin exists to nudge the scroll offset for a smoother path.

The blocker for 5b: the renderer and the cursor/scroll code disagree about whether the top
line's own `before` block is visible. The renderer draws `Before(top_line)` at screen row 0
when `top_row_offset == 0` (`pane_render.rs:118-125`, per
`hume-engine/src/pipeline/tests.rs:210` `before_virtual_line_renders_when_not_skipped`), but
`cursor.rs:79-85` and `scroll.rs:240-242` both treat the top line's `before` count as 0. This
makes terminal cursor placement, `screen_to_char_offset` (mouse), and scroll-margin row
counting land one row off from what's drawn whenever `top_row_offset == 0`. Fix required: pick
one rule (suppress `Before(top_line)` in the renderer, or make cursor/scroll count it) and add
a test pairing the render snapshot with `screen_pos`/`ensure_cursor_visible` for the same
viewport state.

---

## Phase 5c — Full-row background tint (Steel)

> Deferred — not to be built in this pass. Additive on top of 5a (and independent of 5b) per
> the additivity constraint.

Prerequisites: **Phase 3.2 (engine `row_bg` generalization) + Phase 4.4 (line-bg decoration
store) + theme `diff.*` `bg` values in all four themes.**

- **render** (5c addition): `hunks → line-background records → new line-bg setter`. Pure add
  → tint the new-side lines `diff.plus`; change → tint the new-side lines `diff.delta`. Pure
  deletes produce no line-bg record (they render as 5b's virtual lines instead).
- **init**: no new command — line-bg tint is part of `toggle-inline-diff` from 5b, not a
  third independent toggle (matches the "Defaults" decision: signs and inline rendering are
  the two user-facing knobs, not signs/virtual-lines/tint as three).

---

## Critical files

- **Async job execution (Phase 1) — ✅ shipped, files for reference.**
  `hume-platform/src/process/child.rs` (shared transport: `spawn_piped`, `WakeCallback`);
  `hume-platform/src/process/job.rs` (one-shot capture: `spawn_job`, `SpawnedJob`,
  `try_take_result`); `hume-scripting/src/builtins/process.rs` (`spawn_async`,
  `cancel_async`); `hume-scripting/src/host.rs:521-570` (`AsyncProcessHost`);
  `hume-editor/src/editor/async_job.rs` (per-frame drain → `queue_steel_call`);
  `hume-editor/src/editor/host_impl.rs:661-719` (impl); `runtime/plugins/core/pickers/plugin.scm:135-164`
  (git-modified picker, migrated); `hume-treesitter/src/parse_worker.rs` (optional future
  migration target, still out of scope).
- **Native diff (Phase 2 — next)**: new `hume-scripting/src/builtins/diff.rs`;
  `hume-scripting/src/builtins/mod.rs`; new `DiffHost` trait in
  `hume-scripting/src/host.rs` (alongside `BufferHost`/`DecorationHost`/`AsyncProcessHost`);
  `hume-editor`'s `Host` impl forwards to existing `hume-editing/src/diff.rs:149`
  (`diff_lines`) and `:254` (`diff_words`) — no new diff code, no new `hume-scripting`
  dependency (`hume-scripting/Cargo.toml` still has none).
- **Engine render**: `hume-engine/src/providers.rs` (`VirtualLine:228`,
  `VirtualLineAnchor:202-214`, `HighlightTier:37-45`), `hume-engine/src/pipeline/pane_render.rs`
  (`:118-125` top-line `before` render, `:171-183` scroll-vs-virtual-lines caveat, `:326-330`
  row_bg, `:356-437` emit_virtual_row — `pipeline` is a module dir, not one file),
  `hume-engine/src/format.rs` (`:143` `display_rows_for_line`, the scroll/cursor row-count SSOT),
  `hume-editor/src/editor/cursor.rs` (`:79-85` top-line `before` assumption),
  `hume-editor/src/editor/scroll.rs` (`:240-242`, `:326-342` top-line `before` assumption +
  atomic-block clamp), `hume-engine/src/render.rs`
  (`fill_row_bg` method `:96`, free fn `:559`, consumers at `:122,174,231,276,294`),
  `hume-engine/src/types.rs` (`Grapheme.scope` `:160`).
- **Steel surface**: `hume-scripting/src/hooks.rs:75-88` (HOOKS table, 12 entries, no
  `on-text-changed` yet), `host.rs:409-465` (`DecorationHost`, already implemented — extend,
  don't rebuild), `builtins/buffers.rs` (add text-read builtins), `builtins/timers.rs`
  (existing `after`/`cancel-timer!`/`debounce` — reuse), `hume-editor/src/editor/decorations.rs`
  (`DecorationStores` — add line-bg kind; extend `VirtualLineEntry` for Phase 4.5).
- **Virtual-line bridge (Phase 4.5 — new)**: `hume-editor/src/editor/lifecycle.rs:1613-1671`
  (`update_virtual_line_providers` — hardcodes `After` + whole-line segment, both to be made
  configurable), `hume-editor/src/editor/decorations.rs:41-49` (`VirtualLineEntry` — add
  anchor + segment list).
- **Editor glue**: `hume-editor/src/editor/doc_ops.rs:106,137,171` + `buffer/mod.rs:261`
  (`set_text` — fire `on-text-changed` here), `hume-editor/src/ui/highlight_providers.rs:40,82`
  (native-only decoration precedent, superseded by `DecorationHost` for Steel-facing work),
  `hume-editor/src/editor/buffer/mod.rs:102` (`text_gen`).
- **Themes**: `runtime/themes/{sand,dark,light,gruvbox}.toml` — `diff.*` scopes exist in all
  four but need `bg` and `.word` variants added (see theme prereq above).
- **Plugin (Phase 5a/5b/5c)**: new `runtime/plugins/git-diff/*.scm` (model on
  `runtime/plugins/core/lsp/inlay.scm`); `runtime/init.scm.example`.
- **Docs**: `docs/ROADMAP.md` (records the merge decision + points at this file in place of
  the standalone "Git gutter signs" item; note the existing open "Unified decoration system"
  item this phase's Phase 4.4 work will interact with); optional `docs/learning/*.md` on the
  job-execution/decoration design.

## Risks / watch-list

- **Word-diff performance on large changes**: word-diff is native (`diff_words`, Myers,
  50ms deadline) and already deadline-guarded — a changed line that can't be word-diffed in
  time comes back with `deadline_hit()` set; Steel must check this and fall back to a
  line-level tint instead of assuming word hunks are always present.
- **Decoration churn**: rebuild the whole decoration set per refresh (as nvim re-applies
  extmarks); guard with a `prev_hunks` equality check (nvim's `_hunks_equal`) to skip no-op
  refreshes. Optimize only if profiling shows cost.
- **Async git execution — resolved.** No sandbox constrains this — Steel can already
  `spawn-process` arbitrary git commands — but this was never a security question; Phase 1's
  `spawn-async!` shipped with the exactly-once, watchdog-guarded callback contract described
  above.
- **Virtual-line Steel bridge — new risk, Phase 4.5.** The engine type is ready
  (`Before`/`After` anchoring, per-segment scopes); the Steel-facing `set-virtual-lines!`
  bridge is not. This gates Phase 5b specifically — Phase 5a (signs) is unaffected.
- **Virtual-line scroll accounting — open.** Scroll/cursor math counts virtual rows via
  `display_rows_for_line` (`hume-engine/src/format.rs:143`); a virtual block scrolls atomically
  rather than row-by-row (`pane_render.rs:171-183`, `scroll.rs:326-342`) — acceptable for small
  blocks. The blocker for Phase 5b: `pane_render.rs:118-125` renders `Before(top_line)` at
  screen row 0 while `cursor.rs:79-85`/`scroll.rs:240-242` treat the top line's `before` block
  as never shown — the two disagree once `top_row_offset == 0`. Fix required (suppress the
  render or make cursor/scroll count it) plus a paired render/`screen_pos` test.
- **Native callbacks needing `&mut Editor`** — resolved by Phase 1: job-completion callbacks
  run through `Editor::drain_async_jobs` where the main loop holds `&mut Editor`, mirroring
  `line_source.rs`'s wake-based dispatch and the LSP `drain_lsp`/`queue_steel_call` template.
  Nothing calls back from worker threads directly.

## Verification (when built)

- **Engine** (`insta` snapshots): styled virtual line renders with scope fg/strikethrough
  (already covered by existing tests, per Phase 3.1); full-row bg fills to the right edge
  (new, Phase 3.2); `Extra` tier layers correctly for word-diff spans (Phase 3.3 is closed —
  no new tier — but a snapshot confirming `Extra` renders above `Syntax` and below
  `SearchMatch`/`Diagnostic` is still worth adding).
- **Theme** — currently a real gap: no test covers any `diff.*` scope
  (`hume-editor/src/ui/theme/tests.rs:40`'s allowlist and
  `editor/tests/commands.rs:2818` both omit them; deleting all six lines from every theme
  would leave CI green today). Extend the bundled-theme test per the drift-tolerant
  convention — compare embedded vs. on-disk-loaded, not hardcoded hex — once `bg`/`.word`
  values are added.
- **Async job execution (Phase 1) — ✅ already covered, shipped tests.**
  `hume-platform/src/process/job.rs:222-393` (unit: deadlock avoidance, drop-kills-child,
  exit-status handling); `hume-editor/src/editor/tests/unix/async_job.rs` (Rust-side host
  driving); `hume-editor/src/editor/tests/unix/async_job_steel.rs` (end-to-end through real
  Steel: missing binary, empty cmd, cancel).
- **Native diff** (Rust unit): `(diff-lines …)` for pure-add / pure-delete / change /
  multi-hunk, with a hand-computed oracle (not derived from `similar`); `(diff-words …)`
  round-trip against `hume-editing`'s own `diff_words` unit tests, plus a `deadline-hit?`
  case (force a timeout, confirm Steel gets the flag rather than a silent partial diff);
  `(diff-buffer-lines bid ref-text)` produces the same hunks as calling `(diff-lines …)` on
  `(buffer-text bid)` and `ref-text` directly — same output, cheaper path, oracle is the
  existing `diff-lines` builtin, not a new one.
- **Steel builtins**: `buffer-text`/`buffer-lines` return live dirty content; `on-text-changed`
  fires on edit; `debounce`/`after` (already tested) drive the refresh; each per-kind decoration
  setter (`set-signs!`, extended `set-virtual-lines!`, `set-extra-highlights!`, new line-bg
  setter) → render round-trips (snapshot).
- **5a end-to-end** (`/run`): open a tracked file with uncommitted changes, `toggle-git-signs`
  (or plugin default-on), confirm `+`/`-`/`~` signs render at `diff.plus.gutter` /
  `diff.minus.gutter` / `diff.delta.gutter`; edit a line, confirm the debounced sign update;
  save, confirm the ref re-fetch and sign refresh.
- **5b/5c end-to-end** (`/run`): `:toggle-inline-diff`, edit a line — deleted line struck
  above (or below, per Phase 4.5's anchor availability), changed line tinted +
  word-highlighted, live (debounced) updates; save re-fetches the ref.
