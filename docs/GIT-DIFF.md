# `git-diff` for HUME — implementation plan

> **Status.** This document is the agreed design. The plugin itself (Phase 5) is *not*
> to be built yet — everything below it is prerequisite work, tracked here as it lands.

| Phase | State |
|---|---|
| 1 — async subprocess execution | ✅ shipped (`2451d3d6`…`8bcb4606`) |
| theme `diff.*` scopes | ⚠️ partial — all 4 themes, **fg only**, no `bg` |
| 2 — native diff builtins | ⬜ **next** |
| 3 — engine: full-row background | ⬜ shape pinned (SPEC Prereq B) |
| 4.1 — `on-text-changed` hook | ⬜ |
| 4.2 — buffer-text reads | ⬜ |
| 4.3 — async git | ✅ subsumed by Phase 1 |
| 4.4 — line-bg decoration kind | ⬜ shape pinned (SPEC Prereq B) |
| 4.5 — `set-virtual-lines!` `Before` anchor + per-segment scopes | ✅ shipped |
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
  parity; see "Why gutter signs are merged into this plugin, not a separate one" below.)*

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
  (`hume-scripting/src/host.rs`) exposes `set-inlay-hints!`, `set-signs!`,
  `set-virtual-lines!`, `set-extra-highlights!`, `set-eol-text!` (was
  `set-inline-diagnostics!`), backed by `DecorationStores`
  (`hume-editor/src/editor/decorations.rs`), source-namespaced. See
  Phase 4.4/4.5 for the remaining gaps (line-background tint; virtual-line anchor/segments).
- **`VirtualLine` already carries per-segment styling — engine-side only.** `VirtualLine.segments:
  Vec<(Range<usize>, ScopeId)>` (`hume-engine/src/providers.rs:228`) and `Grapheme.scope:
  Option<ScopeId>` (`hume-engine/src/types.rs:160`), segmented by `rows::RowMap`'s virtual-row
  accessor and styled in `hume-engine/src/pipeline/pane_render.rs`. The engine type supports styled,
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
| `autocmd TextChanged` | ❌ no on-edit hook (14-entry `EditorEvent` set, none fire on edit) | `on-text-changed` hook (Phase 4.1) |
| `autocmd BufWritePost` | ✅ `on-buffer-save` (`hume-editor/src/editor/event.rs`) | reuse |
| `vim.uv` timer (debounce) | ✅ `(after ms thunk)` / `(cancel-timer! id)` / `debounce`, timer wheel in `editor/timers.rs` | reuse — nothing to build |
| `vim.system` (async git) | ✅ shipped — `(spawn-async! cmd args cwd callback)` / `(cancel-async! id)`, one-shot capture, exactly-once callback, never inline (`hume-scripting/src/builtins/process.rs`) | reuse — nothing to build |
| `nvim_buf_set_extmark` (virt_lines_above / inline hl / hl_eol) | ✅ **shipped (Phase 4.5).** `set-virtual-lines!` now accepts `'anchor`/`'segments` reaching the engine's `VirtualLineAnchor::Before/After` and per-segment `ScopeId` styling (`hume-engine/src/providers.rs:208-251`) via the Steel bridge (`Editor::update_virtual_line_providers`, `hume-editor/src/editor/decoration_providers.rs`). `set-extra-highlights!`/`set-signs!` unaffected. | still needed: a **line-background** (full-row tint) decoration kind (Phase 3.2 + 4.4) |
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
  (`hume-editor/src/editor/decorations.rs:75-79`). **`VirtualLine.segments` used to be a
  separate, byte-offset case** (Phase 3's `(byte_start, byte_end, ScopeId)`, shipped
  Steel-facing as byte ranges by Phase 4.5) — a Steel caller feeding `diff-words`' char output
  straight into `set-virtual-lines!`'s `'segments` would have had to do its own char→byte
  conversion, contradicting the rule just stated. **Fixed (SPEC.md Prereq B / §5a.2, shipped
  2026-08-06): `'segments` is char offsets at the Steel surface**, with char→byte conversion
  and the full validation suite (bounds, ordering, non-overlap, grapheme alignment) moved into
  the host boundary (`host_impl.rs`'s `virtual_line_segments_to_bytes`) — `diff-words`' output
  now feeds `set-virtual-lines!`'s `'segments` directly, no Steel-side conversion needed.
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

1. **Styled virtual lines — done, engine-side and Steel-side.** `VirtualLine.segments:
   Vec<(usize, usize, ScopeId)>` (`hume-engine/src/providers.rs:250`, byte offsets into
   `VirtualLine.text`) and `Grapheme.scope: Option<ScopeId>`
   (`hume-engine/src/types.rs:160`) already exist; `rows::RowMap` segments the row and
   `hume-engine/src/pipeline/pane_render.rs` resolves each grapheme's scope via
   `theme.resolve(id)`. Red, struck deleted lines + word-del highlighting inside
   them are representable at this layer with the current API — no engine change needed here.
   Reachable from Steel since Phase 4.5 shipped (`set-virtual-lines!`'s `'segments`).

2. **Provider-driven full-row background.** A real gap, still the one piece of Phase 3 to
   build. Full-width tint has exactly **one** hardcoded producer:
   `hume-engine/src/pipeline/pane_render.rs:148-151` sets `row_bg = is_head_line ?
   theme.ui.cursorline.bg : None`, consumed in `render.rs` at `:122, 174, 231, 276, 294`
   plus the fill helpers at `:96-97` (method) and `:559` (free fn). Generalize the `row_bg`
   decision so a provider can request an edge-to-edge background for a line
   (added/changed lines). Depends on the theme prereq above: `diff.plus`/`diff.minus` are
   currently `fg`-only in all four themes, so `bg` values (plus `.word` variants for the
   word-level boost) need adding before this has a color to render.

   **Shape pinned by SPEC.md Prereq B (§5a), build deferred to the unification steps.**
   Resolve the provider tint once per line, in `LineStyle::enter`
   (`hume-engine/src/pipeline/pane_render.rs`), into an `Option<Color>` via
   `theme.resolve(id).bg` — a theme lacking a `bg` on the scope renders nothing, which is
   today's fg-only reality. Both existing consumers of cursorline's bg must read that same
   value: the row-fill site (`row_bg = (is_head_line ? cursorline.bg : None).or(provider_bg)`,
   feeding gutter/trailing cells) and the per-grapheme layering site
   (`hume-engine/src/style/mod.rs:153-208` — the tint becomes the lowest decoration layer,
   between `theme.default` and cursorline, for occupied cells). **Precedence: cursorline
   wins over the tint** — a theme whose cursorline has no `bg` falls through to the tint
   automatically (`ResolvedStyle::layer` only overrides on `Some(bg)`); losing the tint on
   the one row under the cursor costs nothing, losing find-the-cursor inside a 30-line
   tinted hunk is a real regression. Fill extent matches cursorline today: gutter through
   the right edge, every wrap row of a tinted line, sign glyphs render over the tint.

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

1. **`on-text-changed` hook.** Missing — current `EditorEvent` set
   (`hume-editor/src/editor/event.rs`) has 14 variants (`on-buffer-open`, `on-buffer-close`,
   `on-buffer-save`, `on-buffer-enter`, `on-focus-gained`, `on-mode-change`, `on-language-set`,
   `on-lsp-attach`, `on-lsp-detach`, `on-diagnostics-changed`, `on-viewport-change`,
   `on-trigger-char`, `on-completion-accept`, `on-completion-refilter`) and none fire on edit.
   Add `EditorEvent::OnTextChanged` + Steel name;
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
   (`hume-scripting/src/host.rs`, seven methods: `set_inlay_hints`, `set_signs`,
   `set_virtual_lines`, `set_extra_highlights`, `set_eol_text`,
   `diagnostics_for_buffer`, `diagnostic_counts`) is backed by `DecorationStores`
   (`hume-editor/src/editor/decorations.rs` — **unified 2026-08-07**, SPEC.md's
   unified-store item: one generic `SourceStore<T>` instantiated per kind, all five
   uniformly per-source-keyed and char-offset-positioned, one store-wide `generation`
   counter). `set-inline-diagnostics!` is now `set-eol-text!` (never
   diagnostics-specific — the diagnostics plugin is its first client, not its owner).
   Cross-check against the nvim extmark uses:
   - **span highlight** (char-relative range + scope) — already covered by
     `set-extra-highlights!` (`ExtraHighlightEntry { start, end, scope }`,
     `decorations.rs:75-79`, char offsets matching `diff_words`' `WordHunk` ranges).
   - **gutter sign** — already covered by `set-signs!`. Note: nvim's own inline-diff plugin
     (`/Users/matteo/dev/neovim-inline-diff`) defines **no signs at all** — this is HUME-side
     scope beyond parity; cross-reference "Why gutter signs are merged into this plugin, not
     a separate one" above.
   - **virtual line** (styled segments, anchored `Before`/`After`) — ✅ **shipped, Phase
     4.5.** Both the engine type and the Steel-facing `set-virtual-lines!` bridge are ready.
   - **line background** (full-width tint) — confirmed missing, and a real gap.
     `row_bg` still has exactly one hardcoded producer (Phase 3.2); `ExtraHighlightEntry`
     cannot substitute for it — it's a char-range span feeding the highlight pipeline, a
     different channel from `row_bg`. This is the one new decoration kind to add to
     `DecorationStores`, paired with the Phase 3.2 engine change.

     **Shape pinned by SPEC.md Prereq B (§5a), build deferred to the unification steps.**
     Builtin `(set-line-backgrounds! source bid entries)` — the uniform §6
     `(set-X! source bid entries)` shape; entries are `(line scope)` tuples, `line`
     0-indexed at the Steel surface (matches signs' tuple style). Store entry
     `LineBgEntry { pos: usize /* line-start char offset */, scope: String }` in the
     unified per-source store (SPEC §6); the host boundary converts line → line-start char
     offset and fails fast on an out-of-range line (can only fire on a plugin bug — the
     diff runs synchronously against the live buffer in the same Steel evaluation as the
     setter call). Engine side: a `Decoration::LineBg(ScopeId)` variant on the unified kind
     enum, queried only at paint time — never by `RowMap`'s layout query, since a line
     background never affects row count or wrapping. No `priority` field: unlike signs, row
     tints have no single-slot contention and git-diff is the only specced consumer;
     deterministic source-name tie-break is enough. One record per line, not a range — the
     producer trivially expands hunks, and point remap is simpler than range-endpoint-merge
     semantics. Dirty tracking: covered by SPEC §6's store-wide generation; the payload is
     small enough that per-frame sync is fine — no dedicated gate needed. See Phase 3.2 for
     the `row_bg`/precedence contract this kind renders through.
   - **Risk note (partially resolved 2026-08-07):** the store-side half of `docs/ROADMAP.md`'s
     unified-decoration-system item has now landed (SPEC.md), so the line-bg kind's store
     shape above rides on the current, already-unified `SourceStore<T>` — no further store-side
     churn expected. The engine-side half (a single `DecorationSource` trait replacing
     `HighlightSource`/`VirtualLineSource`/`InlineDecoration`, SPEC.md §2/§6) is still open;
     building the line-bg kind's engine variant before that lands would still be a future
     churn point.

5. **`set-virtual-lines!` anchor + per-segment scopes — ✅ shipped, needed by Phase 5b.**
   Was a verified plan-vs-code gap: the engine type
   (`VirtualLine { anchor, provider_id, text, segments: Vec<(usize, usize, ScopeId)> }`,
   `hume-engine/src/providers.rs:234-251`) supported `Before`/`After` anchoring and
   per-segment `ScopeId` styling, but the Steel bridge
   (`Editor::update_virtual_line_providers`, `hume-editor/src/editor/decoration_providers.rs`
   — not `lifecycle.rs:1613-1671` as this doc previously said) flattened both away, hardcoding
   `VirtualLineAnchor::After(line)` and one whole-text segment. Concretely, that meant:
   - Deleted lines could only render below the deletion point, never above — a block deleted
     at the top of the visible range (or above line 0) had nowhere correct to anchor.
   - A deleted line couldn't have word-level highlighting inside it — no way to mark which
     words were the actual removed tokens versus context.

   Fix landed: `set-virtual-lines!` entries are now hashmaps taking an optional `'anchor`
   (`'before`/`'after`, default `'after`) and `'segments` (list of `(start end scope)` char
   ranges into `text` — the covered chars render with the segment's scope instead of
   `'scope`'s, not layered with it; the bridge gap-fills uncovered bytes with `'scope`),
   threaded through `VirtualLineEntry` and `update_virtual_line_providers`. Segment offsets
   were bytes as first shipped; SPEC.md Prereq B (§5a.2) flipped this to char offsets at the
   Steel surface (shipped 2026-08-06, before the unification lands) — see Phase 2's note above.
   Breaking change (old `(line text)`/`(line text scope)` entries are rejected) — acceptable
   since no `.scm` plugin called this builtin yet. Scoped to Phase 4.5, gating Phase 5b only —
   Phase 5a (signs) and Phase 5c (line background) don't touch this API at all.

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

Prerequisites: **Phase 4.5 (virtual-line `Before` anchor + per-segment scopes)** — the scroll
accounting a `Before`-anchored block needs (see below) is already in place.

- **diff** (extended): for each changed hunk, also call `(diff-words old-line new-line)`,
  checking `deadline-hit?` to skip word highlights on a timeout and fall back to a whole-line
  scope. No Lua-style tokenizer/Myers port — both diff passes are native (Phase 2).
  (← `diff.lua`, logic replaced by native calls, not ported)
- **render** (5b addition): `hunks → virtual-line records`. Pure delete → one or more virtual
  lines anchored `Before` the new-side insertion point; change → virtual old line
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

**Virtual-line scroll accounting — resolved, in every wrap mode.** `ViewportState::top_row_offset`
counts display rows of `top_line`'s whole visual block (`before` + content rows + `after`), not
content rows only — every row, virtual or real, is an equal scroll unit, and the pair
`(top_line, top_row_offset)` is the address of the viewport's top row.

`hume-engine/src/rows.rs`'s `RowMap` is the one implementation of that row list. Rendering,
scrolling, cursor placement, mouse mapping and visual movement all consume it, so "which row is
on screen" cannot be answered two ways: the renderer walks the map from `clamp(viewport top)`,
and the editor's row math asks the same map. That parity is now by construction rather than by
seven walkers agreeing — the shape this section originally described, where each consumer
re-derived the row list with its own skip arithmetic and clamp policy.

A `Before`/`After` block — including one anchored to buffer line 0 or the very last buffer line,
taller than the viewport — scrolls into and out of view one row at a time in either wrap mode. An
EOF-overshoot bug in wheel-scroll (resetting to the block's first row instead of clamping to its
last on a large notch) is fixed alongside this; `advance` saturates at the document's last row.
`scroll::clamp_viewport_top` self-heals a viewport top left stale by a write site that doesn't
validate it (`Pane::recall_scroll`, an LSP jump), once per pane per frame. Screen-relative
cursor-follow (mouse wheel, page/half-page scroll — `visual_move.rs`'s
`VerticalUnit::ScreenRow`) counts virtual rows toward its display-row budget so the cursor tracks
the same distance the viewport moved; plain `j`/`k` (`VerticalUnit::ContentRow`) keep treating
virtual rows as free, landing only on real content.

Inline decorations count toward the same budget: an inlay hint takes columns and so participates
in wrapping, and `RowMap` queries `InlineDecoration` when it counts a line's rows, so a hint that
pushes a line onto an extra wrap row moves the rows below it for scroll math exactly as it does
on screen.

See `hume-engine/src/rows/tests.rs`, `hume-engine/src/pipeline/tests.rs`'s
`virtual_before_block_taller_than_viewport_exposes_every_row`,
`hume-editor/src/editor/tests/virtual_line_scroll.rs` (including
`screen_pos_counts_an_inline_hints_extra_wrap_row`), and the `_no_wrap` test siblings in
`cursor/tests.rs`/`scroll/tests.rs`/`mouse/tests.rs`.

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
- **Engine render**: `hume-engine/src/rows.rs` (`RowMap`, the display-row authority every
  consumer below reads — block shape, stepping, char↔row mapping, render accessors),
  `hume-engine/src/providers.rs` (`VirtualLine`, `VirtualLineAnchor`, `HighlightTier`),
  `hume-engine/src/pipeline/pane_render.rs` (`render_pane`'s row walk — `pipeline` is a module
  dir, not one file), `hume-engine/src/layout.rs` (pane geometry only),
  `hume-editor/src/editor/cursor.rs` (`screen_pos`/`screen_to_char_offset`),
  `hume-editor/src/editor/scroll.rs`
  (`ensure_cursor_visible`/`scroll_cursor_to_row`/`clamp_viewport_top`),
  `hume-editor/src/editor/mouse.rs` (`scroll_viewport_up`/`scroll_viewport_down`),
  `hume-editor/src/editor/visual_move.rs` (`move_vertical`, `VerticalUnit`),
  `hume-engine/src/render.rs`
  (`fill_row_bg` method `:96`, free fn `:559`, consumers at `:122,174,231,276,294`),
  `hume-engine/src/types.rs` (`Grapheme.scope` `:160`).
- **Steel surface**: `hume-editor/src/editor/event.rs` (`EditorEvent`, 14 variants, no
  `on-text-changed` yet), `host.rs:409-465` (`DecorationHost`, already implemented — extend,
  don't rebuild), `builtins/buffers.rs` (add text-read builtins), `builtins/timers.rs`
  (existing `after`/`cancel-timer!`/`debounce` — reuse), `hume-editor/src/editor/decorations.rs`
  (`DecorationStores` — add line-bg kind; extend `VirtualLineEntry` for Phase 4.5).
- **Virtual-line bridge (Phase 4.5 — ✅ shipped)**: `hume-editor/src/editor/decoration_providers.rs`
  (`update_virtual_line_providers`, now anchor- and segment-aware; not `lifecycle.rs` — that
  was always the wrong path for this function), `hume-editor/src/editor/decorations.rs`
  (`VirtualLineEntry` — carries `before` + `segments`).
- **Editor glue**: `hume-editor/src/editor/doc_ops.rs:106,137,171` + `buffer/mod.rs:261`
  (`set_text` — fire `on-text-changed` here), `hume-editor/src/ui/highlight_providers.rs:40,82`
  (native-only decoration precedent, superseded by `DecorationHost` for Steel-facing work),
  `hume-editor/src/editor/buffer/mod.rs:102` (`text_gen`).
- **Themes**: `runtime/themes/{sand,dark,light,gruvbox}.toml` — `diff.*` scopes exist in all
  four but need `bg` and `.word` variants added (see theme prereq above).
- **Plugin (Phase 5a/5b/5c)**: new `runtime/plugins/git-diff/*.scm` (model on
  `runtime/plugins/core/lsp/inlay.scm`); `runtime/init.scm.example`.
- **Docs**: `docs/ROADMAP.md`'s `git-diff` plugin roadmap item already points at this file;
  note the existing open "Unified decoration system" item this phase's Phase 4.4 work will
  interact with; optional `docs/learning/*.md` on the job-execution/decoration design.

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
- **Virtual-line Steel bridge — ✅ resolved, Phase 4.5 shipped.** Both the engine type
  (`Before`/`After` anchoring, per-segment scopes) and the Steel-facing `set-virtual-lines!`
  bridge are ready. Was gating Phase 5b specifically — no longer a blocker.
- **Virtual-line scroll accounting — resolved, in every wrap mode.** `top_row_offset` counts
  display rows of `top_line`'s whole visual block (`before` + content + `after`); renderer and
  editor row math agree on every row in both wrap modes because they read the same
  `hume-engine/src/rows.rs` `RowMap`, and a `Before`/`After`-anchored block — including one
  anchored to line 0 or the last buffer line, taller than the viewport — scrolls into view one
  row at a time. Inline decorations count toward the same rows, so an inlay hint that wraps a
  line cannot desync scroll math from the screen. See the tests listed under Phase 5b above.
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
  above the deletion point, changed line tinted + word-highlighted, live (debounced)
  updates; save re-fetches the ref.
