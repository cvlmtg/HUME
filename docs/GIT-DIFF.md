# `git-diff` for HUME — implementation plan

> **Status.** This document is the agreed design. The plugin itself (Phase 5) is *not*
> to be built yet — everything below it is prerequisite work, tracked here as it lands.

| Phase | State |
|---|---|
| 1 — async subprocess execution | ✅ shipped (`2451d3d6`…`8bcb4606`) |
| theme `diff.*` scopes | ⚠️ partial — all 4 themes, **fg only**, no `bg` |
| 2a — native line-diff builtins (`diff-lines`, `diff-buffer-lines`) | ✅ shipped |
| 2b — native word-diff builtin (`diff-words`) | ✅ shipped, prerequisite of 5b only |
| 3 — engine: full-row background | ✅ shipped |
| 4.1 — `on-text-changed` hook | ✅ shipped |
| 4.2 — buffer-text reads | ✅ shipped |
| 4.3 — async git | ✅ subsumed by Phase 1 |
| 4.4 — line-bg decoration kind | ✅ shipped |
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

- **Rendering substrate** in `hume-engine/src/providers.rs`: the unified
  `DecorationSource` trait (produces virtual lines, tiered byte-range
  highlight spans, line-background tints — formerly three separate traits,
  merged since this section was written) plus `GutterColumn` and
  `OverlayProvider` (still their own traits), all aggregated per-pane in a
  `ProviderSet`.
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
  `set-inline-diagnostics!`), and `set-line-backgrounds!`, backed by
  `DecorationStores` (`hume-editor/src/editor/decorations.rs`),
  source-namespaced. Phase 4.4 (line-background tint) and 4.5 (virtual-line
  anchor/segments) have both shipped.
- **`VirtualLine` carries per-segment styling, engine- and Steel-side.**
  `VirtualLine.segments: Vec<(Range<usize>, ScopeId)>`
  (`hume-engine/src/providers.rs:228`) and `Grapheme.scope: Option<ScopeId>`
  (`hume-engine/src/types.rs:160`), segmented by `rows::RowMap`'s virtual-row
  accessor and styled in `hume-engine/src/pipeline/pane_render.rs`. The
  Steel-facing bridge (`set-virtual-lines!`'s `'anchor`/`'segments`, Phase
  4.5) exposes the full `Before`/`After`-anchored, per-segment-scoped shape.
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
- **The `ScopedHighlighter` pattern** (`hume-editor/src/ui/highlight_providers.rs:49-53`):
  an `Arc<RwLock<…>>`-backed provider the editor refreshes each frame. Relevant for
  native-only providers; anything Steel-facing goes through `DecorationHost` instead.
- **`set-signs!` has no Steel client today.** `grep -rn "set-signs!" runtime/` returns
  nothing — diagnostics signs are produced Rust-side, not through the Steel builtin.
  `git-diff` will be the *first* Steel caller of `set-signs!`, which is exactly what the
  `"vcs"` source-name fixtures already pre-validate:
  `hume-editor/src/editor/tests/lsp_signs.rs:287,320,338,384` and
  `lsp_decorations.rs:241,263,301` use `"vcs"` as their canonical non-LSP sign source.

### What is missing

| nvim primitive the plugin uses | HUME today | must build |
|---|---|---|
| `vim.diff` (line Myers) | ✅ **shipped (Phase 2a).** `diff-lines`/`diff-buffer-lines` Steel builtins, wrapping `diff_lines` (`hume-editing/src/diff.rs:157`) | reuse — nothing to build |
| word-diff Myers (in *Lua*) | ✅ **shipped (Phase 2b).** `diff-words` Steel builtin, wrapping `diff_words` (`hume-editing/src/diff.rs:253`) | reuse — nothing to build |
| `nvim_buf_get_lines` (live text) | ✅ **shipped (Phase 4.2).** `buffer-text`/`buffer-lines` Steel builtins, wrapping `BufferHost::{buffer_text, buffer_lines}` (`hume-editor/src/editor/host_impl.rs`) — general-purpose reads; the diff plugin itself still prefers `diff-buffer-lines` for its own per-keystroke hook | reuse — nothing to build |
| `autocmd TextChanged` | ✅ **shipped (Phase 4.1).** `on-text-changed` fires `(buffer-id)` — see Phase 4 item 1 below for the design. | reuse — nothing to build |
| `autocmd BufWritePost` | ✅ `on-buffer-save` (`hume-editor/src/editor/event.rs`) | reuse |
| `vim.uv` timer (debounce) | ✅ `(after ms thunk)` / `(cancel-timer! id)` / `debounce`, timer wheel in `editor/timers.rs` | reuse — nothing to build |
| `vim.system` (async git) | ✅ shipped — `(spawn-async! cmd args cwd callback)` / `(cancel-async! id)`, one-shot capture, exactly-once callback, never inline (`hume-scripting/src/builtins/process.rs`) | reuse — nothing to build |
| `nvim_buf_set_extmark` (virt_lines_above / inline hl / hl_eol) | ✅ **shipped (Phase 4.5).** `set-virtual-lines!` now accepts `'anchor`/`'segments` reaching the engine's `VirtualLineAnchor::Before/After` and per-segment `ScopeId` styling (`hume-engine/src/providers.rs:208-251`) via the Steel bridge (`Editor::update_virtual_line_providers`, `hume-editor/src/editor/decoration_providers.rs`). `set-extra-highlights!`/`set-signs!` unaffected. | reuse — nothing to build |
| line background (full-row tint) | ✅ **shipped (Phase 3.2 + 4.4).** `set-line-backgrounds!` → `Decoration::LineBg(ScopeId)`, paint-stage only. | reuse — nothing to build |
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
4. ✅ **Shipped.** A **line background (full-row tint)** decoration kind (Phase 3.2 + 4.4);
   virtual-line segment styling (Phase 4.5) and a generic plugin highlight tier (`Extra`,
   Phase 3.3) shipped alongside it. `Extra`'s tier ranking is confirmed sufficient for
   word-diff highlights — no new tier needed.
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

## Phase 2a — Native line-diff builtins (Rust) [`diff-lines`, `diff-buffer-lines`] — ✅ SHIPPED

Thin Steel wrappers over `hume-editing`'s existing diff code — no new diff algorithm, no new
`similar` usage.

**Routed through the `Host` trait, not a new crate dependency.** A `DiffHost` capability
trait landed on `hume-scripting/src/host.rs`, alongside `BufferHost`/`DecorationHost`/etc.,
accessed the same way as every other capability: `EditorHost::diff(&mut self) -> Option<&mut
dyn DiffHost>`, defaulted to `None` like the other eight optional accessors (so `NullHost`/
`MockHost` needed no changes). `hume-editor`'s `EditorHostImpl` implements it, forwarding to
`hume-editing/src/diff.rs`'s `diff_lines` through a bridge module — no new `hume-scripting`
dependency, matching `AsyncProcessHost` (Phase 1)'s precedent.

**Correction to this doc's original plan:** `ctx.host` is a **field** on `SteelCtx`, not a
method — the builtin body reaches the capability via `require_cap(ctx.host.diff(), "diff-lines")?`
(`hume-scripting/src/builtins/errors.rs`), not `ctx.host()?.diff()?`.

**Both texts are normalized through `Text::from` before tokenizing**, not compared as raw
`&str` line-splits. `hume-editing/src/text.rs`'s `Text` type already CRLF-normalizes and
forces a trailing newline on construction — routing both the `git show` ref text and the
comparison text through it means a ref blob missing its final newline (routine for git) or
checked in with CRLF produces no phantom hunk, because both sides are compared exactly as
HUME would load them as buffer content. This deliberately diverges from `git diff`'s raw byte
comparison: nothing about a file's *content* changes on save just because it lacked a final
newline on disk, and the ref/buffer comparison should agree with that. `diff-lines` and
`diff-buffer-lines` are two entry points onto one shared tokenize-and-translate function
(`hume-editor/src/editor/diff_bridge.rs`), so their agreement on the same input is structural,
not tested separately.

**Tokenization is one rope traversal** (`Text::line_tokens`, `hume-editing/src/text.rs`, built
on `Rope::lines()`), keeping each line's trailing `\n` so an `Equal` hunk stays comparable
across the trailing-empty-line boundary — shared with `changeset::diff_cs`'s reload-diff path
rather than a second hand-rolled tokenizer, and borrowing via `RopeSlice`'s `Cow` conversion
where a line sits inside one rope chunk. `LineHunkKind` carries no payload — a multi-line
change's lines are always re-sliced from the tokenized input.

**Steel-facing hunk shape — 0-based, not nvim's anchor convention**, not `vim.diff`'s 1-based
starts with a zero-count-side anchor shift; `set-signs!`/`set-virtual-lines!` are 0-indexed at
the Steel surface, so this needs no arithmetic at any call site. `Equal` hunks are dropped;
`LineDiff::deadline_hit()` (the histogram→Myers fallback) is **not** exposed — unlike
word-diff's coarse timeout fallback, Myers still returns a correct, complete line partition,
so there is nothing for a plugin to react to. Exact tuple shape: `builtins/diff.rs`'s
`diff-lines` doc comment; plugin-author-facing description: `user-manual/docs/plugins.md`'s
"Comparing text" section.

**No line-range parameters** (`range-start`/`range-end`) on either builtin. Considered and
rejected: a ranged diff can't correctly serve this plugin (hunk anchors depend on the full
line sequence on both sides — diffing a sub-range in isolation against a git ref gives wrong
anchors unless paired with real incremental re-diffing, a materially bigger, riskier feature),
and a repo-wide check (`docs/ROADMAP.md`, `docs/LSP.md`) found no planned feature that wants a
sub-buffer diff. If a future feature ever needs "diff lines X–Y", it composes a ranged
buffer-read (Phase 4.2) with the already-generic `diff-lines` — no new diff-side parameter.

Landed as `hume-scripting/src/builtins/diff.rs`, registered `cmd`-gated in `register_all`
(`hume-scripting/src/builtins/mod.rs`); see `user-manual/docs/plugins.md`'s "Comparing text"
section for the plugin-author-facing description.

## Phase 2b — Native word-diff builtin (Rust) [`diff-words`] — ✅ SHIPPED

Deferred out of 2a: word-diff has exactly one consumer, Phase 5b, so it landed next to that
prerequisite list rather than shipping unused. Extends the same `DiffHost` trait and
`hume-scripting/src/builtins/diff.rs` module 2a landed — one trait method, one type
(`WordDiffHunk`), one bridge function, one builtin, no new files, no rework of 2a.

### What shipped

- **`DiffHost::diff_words`**, `hume-scripting/src/host.rs`: `fn diff_words(&self, old: &str,
  new: &str) -> (Vec<WordDiffHunk>, bool)` — a plain tuple return, same shape as
  `DecorationHost::diagnostic_counts`.
- **`WordDiffHunk`**, next to `DiffHunk` in `host.rs`: one contiguous `(start, end)` char span
  per side plus its text, not a reuse of `DiffHunk`'s line-index shape — a word hunk has no
  line list to count.
- **`diff_bridge::word_hunks`**, `hume-editor/src/editor/diff_bridge.rs`: unlike line-diff, no
  `Text` normalization and no re-slicing — its inputs are two already-extracted, already
  `Text`-normalized line strings (one side of a `diff-lines` `Replace` hunk), and
  `WordHunkKind`'s payload is byte-for-byte the exact substring at that char range already
  (`split_word_bounds` tokens abut with no gap, unlike lines joined without their `\n`).
- **`(diff-words old-text new-text)` → `(hunks . deadline-hit?)`**, `builtins/diff.rs`: a
  dotted pair (`args::cons_pair`, same shape as `diagnostic-counts` → `(errors . warnings)`),
  `hunks` a list of `(old-start old-end new-start new-end old-text new-text)` tuples. Registered
  `cmd`-gated in `builtins/mod.rs`, right after `diff-buffer-lines`.

### Contracts the plugin must rely on

- No `diff-buffer-words` variant — word-diff's inputs are always two short strings the plugin
  already holds in Steel, so there's nothing to save by reaching into a buffer directly (unlike
  `diff-buffer-lines`, which exists because materializing a whole buffer per debounce tick is
  real cost).
- `deadline-hit?` is `#t` only on a forced Myers timeout (`Duration::ZERO` in tests; real
  inputs are short lines and don't hit it in practice) — treat it as "fall back to whole-line
  highlighting", not as an error.
- A hunk's `old-text`/`new-text` composes directly into `set-virtual-lines!`'s `'segments` or
  `set-extra-highlights!`'s spans — no arithmetic at the call site, same design goal as 2a's
  `diff-lines`.

Landed in `hume-scripting/src/builtins/diff.rs` (appended to 2a's file) and
`hume-editor/src/editor/diff_bridge.rs`; see `user-manual/docs/plugins.md`'s "Comparing text"
section for the plugin-author-facing description.

---

## Phase 3 — Engine rendering changes (Rust, `hume-engine`)

Styled virtual lines, a generic plugin highlight tier, and full-row background all exist at
the engine layer now — the two items below are historical record of what shipped.

1. **Styled virtual lines — done, engine-side and Steel-side.** `VirtualLine.segments:
   Vec<(usize, usize, ScopeId)>` (`hume-engine/src/providers.rs:250`, byte offsets into
   `VirtualLine.text`) and `Grapheme.scope: Option<ScopeId>`
   (`hume-engine/src/types.rs:160`) already exist; `rows::RowMap` segments the row and
   `hume-engine/src/pipeline/pane_render.rs` resolves each grapheme's scope via
   `theme.resolve(id)`. Red, struck deleted lines + word-del highlighting inside
   them are representable at this layer with the current API — no engine change needed here.
   Reachable from Steel since Phase 4.5 shipped (`set-virtual-lines!`'s `'segments`).

2. **Provider-driven full-row background — ✅ shipped, 2026-08-07.** Full-width tint used
   to have exactly **one** hardcoded producer: `hume-engine/src/pipeline/pane_render.rs`
   set `row_bg = is_head_line ? theme.ui.cursorline.bg : None`, consumed in `render.rs` at
   `:122, 174, 231, 276, 294` plus the fill helpers at `:96-97` (method) and `:559` (free
   fn). `row_bg` is now generalized so a `Decoration::LineBg`-emitting provider can request
   an edge-to-edge background for a line. Still depends on the theme prereq above:
   `diff.plus`/`diff.minus` are currently `fg`-only in all four themes, so `bg` values
   (plus `.word` variants for the word-level boost) need adding before *this specific
   plugin's* tint has a color to render — the mechanism itself works with any scope that
   has a `bg`, proven by this repo's own tests against `ui.selection.search`.

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
`DecorationHost`/`DecorationStores`, including the line-background kind and the
virtual-line anchor/segments bridge — no new store needed. There is no Rust-enforced git
sandbox to work around (full-trust plugin model). Phase 4.1 (edit hook) has shipped; the
remaining real gap is buffer-text reads, item 2 below.

Each is general-purpose. Registered through `register_all`
(`hume-scripting/src/builtins/mod.rs`) and, where they touch editor state, the `EditorHost`
trait (`hume-scripting/src/host.rs`).

1. **`on-text-changed` hook — ✅ shipped.** `EditorEvent` (`hume-editor/src/editor/event.rs`)
   now carries 16 variants, `OnTextChanged { buffer }` among them, Steel name
   `on-text-changed`, handler args `(bid)`.

   **Deviated from this doc's original plan, deliberately.** The original plan proposed
   raising from the edit-apply path — `apply_edit`/`apply_edit_grouped`/`apply_edit_regrouped`
   on `Buffer`, called from `doc_ops.rs`, all routing through `set_text` where `text_gen` is
   bumped — and firing there. That's the right *place* semantically (`set_text` is the one
   private chokepoint every text mutation funnels through), but unreachable: `Buffer` holds no
   `EditorState` and cannot call `EditorState::queue_event`, and `doc_ops`'s apply functions
   take disjoint field borrows (`&mut BufferStore`, `&DecorationStores`, …), not
   `&mut EditorState`, by design. Raising there would also miss `:e!` reload and read-only
   view refreshes (`:messages`/`:ls`), both of which mutate text through `set_text` outside
   `doc_ops` entirely — and a diff-driven `git-diff` plugin needs a reload to re-trigger it.

   Shipped instead as an **observation-point diff**, the same shape `OnBufferEnter` already
   uses for a value with no write-site chokepoint (`docs/LESSONS.md` L9):
   `Editor::detect_text_changed` (`scripting_setup.rs`) diffs each open buffer's `text_gen`
   against a new `Buffer::announced_text_gen` baseline every pass of `drain_pending_work`'s
   fixpoint, via `BufferStore::take_text_changed`. This **coalesces** — several mutations to
   one buffer between two drain passes fire exactly one event, not one per mutation — and
   fires for edits, undo, redo, and `:e!` reload alike, since all of them bump `text_gen`. No
   separate `on-buffer-reload` was added; `on-text-changed` already covers it.

2. **Buffer text reads — ✅ shipped.** `builtins/buffers.rs` and `BufferHost`
   (`hume-scripting/src/host.rs:368-407`) were metadata-only (`buffer-path`, `buffer-name`,
   `buffer-dirty?`, `buffer-generation`, `buffer-language`, cursor/selection reads) — no
   text/content/slice/get-text builtin existed anywhere. Added `(buffer-text bid)` → the
   buffer's full live (dirty) content, string, trailing `\n` included, and `(buffer-lines bid
   #:start .. #:end ..)` → its content lines, breaks stripped, phantom trailing line (the one
   ropey counts past the structural `\n`) excluded — matching what the statusline and `:w`
   already count. `#:start`/`#:end` is 0-based, end-exclusive, keyword args rather than
   positional-optional: steel-core 0.8.2 desugars `(define (f a [b 0]))` to a mixed
   fixed-plus-rest lambda, exactly the shape `bootstrap.scm`'s `get-option` comment documents
   as broken for 2+ positional args at a required-module call site (which every plugin body
   is) — keywords on a fixed leading positional (`diagnostics-for-buffer bid #:range ..`) are
   the proven-safe form, so `buffer-lines` follows it. An out-of-range `#:end`, or `#:start >
   #:end`, raises rather than silently clamping. **Confirmed: no separate `buffer-text-gen`
   builtin needed** — `(buffer-generation bid)` already returns the `text_gen` counter
   (`hume-editor/src/editor/buffer/mod.rs:111`, registered `builtins/mod.rs`, impl
   `buffers.rs:105-113`, `BufferHost::buffer_generation` at `host.rs:401`) and is already used
   as a generation-paired-read guard by `runtime/plugins/core/lsp/format.scm:21`
   (`expect_gen`) — reuse that pattern rather than adding a second counter.

   Landed as three new `BufferHost` methods (`buffer_text`, `buffer_line_count`,
   `buffer_lines`) implemented in `hume-editor/src/editor/host_impl.rs` via
   `Buffer::text()`/`Text::line_tokens_at()` — an `O(log n)` seek to the range's start, not a
   whole-buffer tokenize-then-skip. The line-break strip shared with `diff_bridge.rs`'s
   tokenization was promoted to `hume_editing::text::strip_line_break` (module-level SSOT)
   rather than staying duplicated between the two call sites.

3. **Async git — ✅ resolved by Phase 1, no new native builtin needed here.** `spawn-async!`
   shipped; the plugin's `init.scm` calls it directly for `git show`/`git rev-parse`, the same
   way `runtime/plugins/core/plum/lib.scm:33` wraps sync `spawn-process` for clone/pull.

4. **Decoration API — extend, don't rebuild.** `DecorationHost`
   (`hume-scripting/src/host.rs`, eight methods: `set_inlay_hints`, `set_signs`,
   `set_virtual_lines`, `set_extra_highlights`, `set_eol_text`, `set_line_backgrounds`,
   `diagnostics_for_buffer`, `diagnostic_counts`) is backed by `DecorationStores`
   (`hume-editor/src/editor/decorations.rs` — **unified 2026-08-07**, SPEC.md's
   unified-store item: one generic `SourceStore<K, T>` instantiated per kind, all six
   decoration kinds uniformly per-source-keyed and char-offset-positioned, one
   per-buffer `generation` stamp off a shared monotonic clock). `set-inline-diagnostics!`
   is now `set-eol-text!` (never
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
   - **line background** (full-width tint) — ✅ **shipped, 2026-08-07.** `row_bg`
     generalized past its one hardcoded cursorline producer (Phase 3.2); `ExtraHighlightEntry`
     never substituted for it — it's a char-range span feeding the highlight pipeline, a
     different channel from `row_bg`.

     Builtin `(set-line-backgrounds! source bid entries)` — the uniform §6
     `(set-X! source bid entries)` shape; entries are `(line scope)` tuples, `line`
     0-indexed at the Steel surface (matches signs' tuple style). Store entry
     `LineBgEntry { pos: usize /* line-start char offset */, scope: String }` in the
     unified per-source store (SPEC §6); the host boundary converts line → line-start char
     offset and fails fast on an out-of-range line. Engine side: `Decoration::LineBg(ScopeId)`
     on the unified `DecorationSource` kind enum, queried only at paint time — never by
     `RowMap`'s layout query, since a line background never affects row count or wrapping.
     No `priority` field: unlike signs, row tints have no single-slot contention;
     same-line entries from different sources break ties by ascending source name. One
     record per line, not a range. Dirty tracking: covered by SPEC §6's per-buffer
     generation stamp; the payload is small enough that per-frame sync is fine — no
     dedicated gate needed. See Phase 3.2 for the `row_bg`/precedence contract this
     kind renders through.

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

Prerequisites: **Phase 2a (line-diff builtins) + Phase 4.1 (`on-text-changed`) — both ✅
shipped.** Neither Phase 4.5 (virtual-line bridge) nor Phase 3.2/4.4 (line background) is
needed for this layer, and 5a doesn't call `diff-words` — Phase 2b is not a prerequisite
here. Nothing blocks starting 5a itself.

Closest existing structural precedent in-repo: `runtime/plugins/core/lsp/inlay.scm` — uses
`debounce-by`, `register-hook!` (e.g. `'on-viewport-change`, `'on-diagnostics-changed`,
`'on-lsp-detach`), and a decoration setter (`set-inlay-hints!`) — the same shape `git-diff`
needs (`debounce-by` + `register-hook! 'on-text-changed` (now shipped) + `set-signs!`).
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

Prerequisites: **Phase 2b (word-diff builtin) + Phase 4.5 (virtual-line `Before` anchor +
per-segment scopes)** — both have shipped, so this layer is unblocked on the native-diff side;
the scroll accounting a `Before`-anchored block needs (see below) is already in place too.

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
in wrapping, and `RowMap` queries `DecorationKinds::INLINE` when it counts a line's rows, so a hint
that pushes a line onto an extra wrap row moves the rows below it for scroll math exactly as it
does on screen.

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
- **Native line diff (Phase 2a — shipped)**: `hume-scripting/src/builtins/diff.rs`
  (`diff-lines`/`diff-buffer-lines`); `hume-scripting/src/builtins/mod.rs`; `DiffHost` trait in
  `hume-scripting/src/host.rs` (alongside `BufferHost`/`DecorationHost`/`AsyncProcessHost`);
  `hume-editor/src/editor/diff_bridge.rs` (tokenize + `LineHunk`→Steel-shape translation, both
  sides normalized through `Text::from`); `hume-editor/src/editor/host_impl.rs`'s `impl
  DiffHost` forwards to it — no new diff code, no new `hume-scripting` dependency
  (`hume-scripting/Cargo.toml` still has none).
- **Native word diff (Phase 2b — shipped)**: extends the same `DiffHost` trait and
  `builtins/diff.rs` with `diff-words`, forwarding via `diff_bridge::word_hunks` to
  `hume-editing/src/diff.rs:253` (`diff_words`) — no new diff code.
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
- **Steel surface**: `hume-editor/src/editor/event.rs` (`EditorEvent`, `on-text-changed`
  shipped, Phase 4.1), `host.rs` (`DecorationHost`, already implemented — extend, don't
  rebuild), `builtins/buffers.rs` (`buffer-text`/`%buffer-lines`, Phase 4.2 — ✅ shipped;
  `bootstrap.scm` wraps `%buffer-lines` in the `#:start`/`#:end` keyword-arg form),
  `builtins/timers.rs`
  (existing `after`/`cancel-timer!`/`debounce` — reuse), `hume-editor/src/editor/decorations.rs`
  (`DecorationStores` — line-bg kind (Phase 4.4) and `VirtualLineEntry`'s `before`/`segments`
  (Phase 4.5) both landed already).
- **Virtual-line bridge (Phase 4.5 — ✅ shipped)**: `hume-editor/src/editor/decoration_providers.rs`
  (`update_virtual_line_providers`, now anchor- and segment-aware; not `lifecycle.rs` — that
  was always the wrong path for this function), `hume-editor/src/editor/decorations.rs`
  (`VirtualLineEntry` — carries `before` + `segments`).
- **`on-text-changed` (Phase 4.1 — ✅ shipped)**: `hume-editor/src/editor/buffer/mod.rs`
  (`Buffer::text_gen`, `Buffer::announced_text_gen`), `hume-editor/src/editor/buffer/store.rs`
  (`BufferStore::take_text_changed`), `hume-editor/src/editor/scripting_setup.rs`
  (`Editor::detect_text_changed`, called from `drain_pending_work`'s fixpoint), `event.rs`
  (`EditorEvent::OnTextChanged`). Not `doc_ops.rs`/`set_text` — see Phase 4 item 1's "deviated
  from this doc's original plan" note for why the raise is an observation-point diff instead.
- **Themes**: `runtime/themes/{sand,dark,light,gruvbox}.toml` — `diff.*` scopes exist in all
  four but need `bg` and `.word` variants added (see theme prereq above).
- **Plugin (Phase 5a/5b/5c)**: new `runtime/plugins/git-diff/*.scm` (model on
  `runtime/plugins/core/lsp/inlay.scm`); `runtime/init.scm.example`.
- **Docs**: `docs/ROADMAP.md`'s `git-diff` plugin roadmap item already points at this file;
  optional `docs/learning/*.md` on the job-execution/decoration design.

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
