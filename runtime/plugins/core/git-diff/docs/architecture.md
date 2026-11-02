# core:git-diff — Architecture

The plugin is orchestration (state, debounce, git process management and decoration
construction) over one native builtin, `diff-buffer-lines`: a line diff against a ref blob,
each hunk carrying its changed-word spans.

Signs and inline rendering share each refresh's hunk set and one fetch/diff pipeline: the
repo probe, ref fetch, line diff, debounce, ref-cache invalidation and the reconcile step
that skips no-op repaints. They differ only in the decorations they build: a `set-signs!`
call, versus virtual lines, word spans and a line tint.

## File layout

| File | Owns |
|---|---|
| `plugin.scm` | Entry point; wires config, per-buffer state, and the fetch/diff pipeline to the buffer lifecycle hooks and the two toggle commands |
| `lib/state.scm` | Per-buffer state (see below) |
| `lib/diff.scm` | Ref-content fetch, the native line-diff call, and the reconcile step that paints both renderings, debounced per buffer (see `docs/pipeline.md`) |
| `lib/branch.scm` | Current-branch fetch, debounced per buffer, pushed to the statusline (see `docs/pipeline.md`) |
| `lib/render.scm` | Pure `hunks → decoration records` functions, one per rendering, and the word spans that feed the inline records. The inline renderers take their decoration source as a parameter, so `plugin.scm`'s `git-diff/render-diff` command can draw another plugin's hunks (see `docs/rendering.md`) |

## State (`state.scm`)

One `(box (hash))` keyed by `(buffer-key pane)`, the shape under the core plugins index's
[Per-buffer state](../../README.md#per-buffer-state). Each entry is built from one
`fresh-entry` and holds:

| Field | Holds |
|---|---|
| `"signs?"` / `"inline?"` | The two independent enable flags |
| `"ref-text"` | The fetch/diff cache (see the table below) |
| `"ref-path"` | The path the last fetch was for; a cache for another path is not used (see `docs/pipeline.md`'s "Hooks") |
| `"signs-painted"` / `"inline-painted"` | What each rendering last painted (the sign records, the hunks); `'()` when it shows nothing |
| `"jobs"` | The in-flight `spawn-async!` id of each job slot (`"diff"`, `"branch"`), keyed by slot; a slot with no job in flight has no key |
| `"ref"` | `#f` (use the config default) or a runtime override string set via `:toggle-git-signs <ref>`/`:toggle-inline-diff <ref>` |
| `"covered-by"` | The other plugins' decoration sources drawing through `git-diff/render-diff`, which hide the inline rendering (see `docs/rendering.md`) |

### One hunk set

Every renderer in `render.scm` is a pure function over the one hunk set a refresh passes
to `apply-hunks!`. The plugin does not store it. `diff.scm` paints a rendering only when
its target differs from its `*-painted` field (see
[Applying hunks](pipeline.md#applying-hunks)). A new rendering is one function, one setter
call and one `*-painted` field.

### `"ref-text"` states

| Value | Meaning |
|---|---|
| a string | The cached `git cat-file` blob; a refresh is a local diff, no process |
| `#f` | Not yet fetched, or dropped by `drop-ref-text!` (a buffer enter, focus gained, an explicit-ref toggle, a path change); the next refresh fetches |
| `'unavailable` | The last fetch failed: a sticky negative cache, so a doomed fetch is not retried on every debounce fire |

### Tracking a buffer

Every live entry point (each hook, each command) first calls `plugin.scm`'s `track!`, which
creates the buffer's entry with the `'signs`/`'inline` config defaults when it has none.

- A buffer whose `on-buffer-open` never fired, because an activation list overrode the
  manifest's `#:events` with a `#:commands`-only list, is tracked at its first enter,
  edit, save or command, with the same flags a buffer opened later gets.
- A toggle that is what creates the entry turns its own rendering on rather than flipping
  the default, since nothing was drawn for the buffer before.

So a missing entry means the buffer closed. `entry-set!` raises on one, and `entry-get`
reads `#f` from one. The paths that can run after a close test `entry-get` first:
`refresh!` and `refresh-branch!` when a debounce timer fires, and `spawn-job!`'s callback
through the slot it owns (see [Closed buffers](pipeline.md#closed-buffers)).

### Job slots

`spawn-job!` starts a `spawn-async!` job in a slot (`"diff"`/`"branch"`), cancelling the
slot's current job first. The result callback runs only while its job still owns the slot,
so a result queued before a cancel or a replacement, or arriving after the buffer closed,
is dropped. `cancel-job!` cancels a slot's job without firing its callback.

`diff.scm` and `branch.scm` both go through these two, differing only in the key.
`remove-buffer!` cancels every job in `"jobs"` before it drops the entry.

## Ref handling

### Per-buffer ref

Both commands share one per-buffer `"ref"` override. `buffer-ref` (`state.scm`) resolves
it: the per-buffer override when set, else the config `'ref` default. A refresh resolves it
when it runs, not when a hook scheduled it. The shared body `run-toggle!` handles both
commands:

- With a ref argument it ensures the entry exists, turns that rendering on, stores the
  ref, drops `"ref-text"` and any fetch in flight (`drop-ref-text!`) and force-refreshes.
  A ref always turns the rendering on and always re-fetches, even if it is already on at
  the same ref.
- With no argument it flips the flag. Turning it on force-refreshes; turning it off
  reconciles, which clears that rendering.

Switching the ref from either command re-renders whichever rendering is on. The override
survives a later bare toggle off and on, and does not reset to the config default.

### Ref completion

`git-diff:refs` is the completion source both commands complete their ref argument
against. It is a minibuffer-target source, so its callback gets `id`/`input`/`cursor` and no
pane. It reads `(focused-pane)`: the pane that opened the `:` command line, and the one
the typed command receives as its leading pane on Enter. The completion universe is every
local branch, tag and remote-tracking ref the focused buffer's repo knows (`git
for-each-ref`, spawned async in the buffer's directory, the same shape as `branch.scm`'s
branch fetch). It answers `'()` when there is no path, the directory is not a repo, or git
fails.
