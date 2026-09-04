# core:git-diff — Architecture

## File layout

- `plugin.scm` — entry point; wires config, per-buffer state, and the fetch/diff pipeline
  to the buffer lifecycle hooks and the two toggle commands.
- `state.scm` — per-buffer state: the two enable flags, the cached ref blob, the current
  hunk set, the in-flight fetch job, and the per-buffer ref override.
- `diff.scm` — ref-content fetch (`git show` via `spawn-async!`) and the native line-diff
  call (`diff-buffer-lines`), debounced per buffer. `diff-words` (word-level diff) is not
  called here — it's called from `render.scm`, where the records it feeds are built.
- `branch.scm` — current-branch fetch (`git rev-parse --abbrev-ref HEAD` via
  `spawn-async!`), debounced per buffer, pushed to the `"git-branch"` statusline element
  via `set-statusline-text!`.
- `render.scm` — pure `hunks → decoration records` functions, one per rendering (gutter
  signs, virtual deleted lines + word highlights, row background tint), each ending in a
  setter call.

No native diff algorithm lives here — `diff-buffer-lines`/`diff-words` already wrap
`similar`/Myers in Rust. This plugin is orchestration (state, debounce, git process
management, and decoration construction) over those.

## Signs and inline rendering are one plugin, not two

Both are renderings of the same underlying hunk data, produced by the same `git show`/
`diff-buffer-lines` pipeline. Shared: repo probe, ref fetch, line diff, debounce,
ref-cache invalidation, hunk-equality check to skip no-op refreshes. Differing: a
`set-signs!` call versus the virtual-line/tint/word-span construction — roughly 20% of the
plugin, not enough to justify splitting the other 80%. Splitting them would also open a
window where the gutter and the inline view disagree about the same file, since each
would fetch and diff independently.

## State (`state.scm`)

One `(box (hash))` keyed by buffer id — the same per-key mutable-table idiom
`debounce-by` uses (`hume-scripting/src/builtins/bootstrap.scm`). Steel's `hash` is
persistent, so mutation is swap-the-box, not in-place update. Each entry, built from the
single `fresh-entry` SSOT, holds:

- `"signs?"`/`"inline?"` — the two independent enable flags.
- `"ref-text"` — the git-fetch/diff pipeline's cache: a string (the cached `git show`
  blob), `#f` (needs (re-)fetching), or `'unavailable` (the last fetch failed; a sticky
  negative cache so a doomed fetch isn't retried every debounce fire — see
  `docs/pipeline.md`).
- `"hunks"` — the verbatim tuples `diff-buffer-lines` last returned, always kept in sync
  with what's actually painted, never a signs-derived shape. This is the *additivity
  invariant*: every renderer in `render.scm` stays a pure function over this one shared
  hunk set, so adding a new rendering is one function and one setter call, touching
  neither this file, the fetch pipeline, nor the lifecycle hooks in `plugin.scm`.
- `"job"` — the in-flight `spawn-async!` id, or `#f`, so a superseded fetch can be
  cancelled.
- `"ref"` — `#f` (use the config default) or a string (a runtime override set via
  `:toggle-git-signs <ref>`/`:toggle-inline-diff <ref>`, shared by both renderers — see
  `plugin.scm`'s `git-diff/buffer-ref`).
- `"branch-job"` — the in-flight branch-fetch `spawn-async!` id, or `#f` — `branch.scm`'s
  own cancel slot, independent of `"job"` (the diff fetch's).

`entry-set!` is a no-op when `bid` has no tracked entry — a late `spawn-async!` callback
for a buffer closed while its fetch was in flight must not resurrect state for it.
`ensure-entry!` is the opposite: it resurrects a missing entry from `fresh-entry` rather
than no-opping, for a write path (an explicit-ref or bare toggle invocation) that must
succeed even for a buffer whose `on-buffer-open` never fired — an activation list can
override the manifest's `#:events` with a `#:commands`-only list, so the plugin only
loads on the first toggle command. `toggle-flag!` is built from `ensure-entry!` plus
`entry-set!` rather than its own box/hash pair — the caller needing the flipped value
back is just an extra `hash-ref` around the two, not a reason to duplicate them.

`cancel-job!` cancels any in-flight `spawn-async!` job stored under a given key
(`"job"`/`"branch-job"`) for `bid`, without firing its callback — shared by `diff.scm`'s
and `branch.scm`'s otherwise-identical cancel functions, only the slot key differs
between them.

## Ref handling

Both commands share one per-buffer `"ref"` override (`state.scm`) — switching it from
either toggle re-renders whichever of the two is currently on, and it survives a later
bare toggle off/on rather than resetting to the config default. `git-diff/buffer-ref`
(`plugin.scm`) resolves it: the per-buffer override when set, else the config `"ref"`
default. Giving a ref always turns that rendering on, never off, and re-fetches even if
it's already on at the same ref.
