# core:git-diff — Architecture

## File layout

| File | Owns |
|---|---|
| `plugin.scm` | Entry point; wires config, per-buffer state, and the fetch/diff pipeline to the buffer lifecycle hooks and the two toggle commands |
| `state.scm` | Per-buffer state (see below) |
| `diff.scm` | Ref-content fetch and the native line-diff call, debounced per buffer (see `docs/pipeline.md`) |
| `branch.scm` | Current-branch fetch, debounced per buffer, pushed to the statusline (see `docs/pipeline.md`) |
| `render.scm` | Pure `hunks → decoration records` functions, one per rendering (see `docs/rendering.md`) |

`diff-words` (word-level diff) is called from `render.scm`, not `diff.scm`: the records
it feeds are built there.

## State (`state.scm`)

One `(box (hash))` keyed by `(buffer-key pane)`, not by a pane value itself. A command's
own pane and a hook's pane-less value for the same buffer must resolve to the same entry
(see the [core plugins index](../../README.md#per-buffer-state)). Each entry, built from
a single `fresh-entry` source of truth, holds:

| Field | Holds |
|---|---|
| `"signs?"` / `"inline?"` | The two independent enable flags |
| `"ref-text"` | The fetch/diff cache (see the table below) |
| `"hunks"` | The verbatim hunk hashes `diff-buffer-lines` last returned, always kept in sync with what's actually painted |
| `"job"` | The in-flight diff-fetch `spawn-async!` id, or `#f` |
| `"ref"` | `#f` (use the config default) or a runtime override string set via `:toggle-git-signs <ref>`/`:toggle-inline-diff <ref>` |
| `"branch-job"` | The in-flight branch-fetch `spawn-async!` id, or `#f`; independent of `"job"` |

`"hunks"` staying in sync with what's painted is the *additivity invariant*: every
renderer in `render.scm` is a pure function over this one shared hunk set, so adding a new
rendering is one function and one setter call, touching neither this file, the fetch
pipeline, nor the lifecycle hooks in `plugin.scm`.

### `"ref-text"` states

| Value | Meaning |
|---|---|
| a string | The cached `git show` blob; a refresh is a local diff, no process |
| `#f` | Not yet fetched, or invalidated by a save; the next refresh fetches |
| `'unavailable` | The last fetch failed: a sticky negative cache, so a doomed fetch isn't retried on every debounce fire |

### `entry-set!` vs. `ensure-entry!`

Both write to an entry, but disagree on what to do when one doesn't exist yet: the
[core plugins index](../../README.md#stale-async-work)'s two write-path shapes, side by
side:

| | On a missing entry | Used by |
|---|---|---|
| `entry-set!` | No-ops | A `spawn-async!` callback. A late callback for a buffer closed while its fetch was in flight must not resurrect state for it |
| `ensure-entry!` | Resurrects one from `fresh-entry` | A toggle command. An explicit-ref or bare toggle invocation must succeed even for a buffer whose `on-buffer-open` never fired (an activation list can override the manifest's `#:events` with a `#:commands`-only list) |

`toggle-flag!` is built from `ensure-entry!` plus `entry-set!` rather than its own
box/hash pair: needing the flipped value back is just an extra `hash-ref` around the
two, not a reason to duplicate them.

`cancel-job!` cancels any in-flight `spawn-async!` job stored under a given key
(`"job"`/`"branch-job"`) for a buffer, without firing its callback. It is shared by `diff.scm`'s
and `branch.scm`'s otherwise-identical cancel functions, only the key differs between them.

## Ref handling

Both commands share one per-buffer `"ref"` override. Switching it from either toggle
re-renders whichever of the two is currently on, and it survives a later bare toggle
off/on rather than resetting to the config default. `buffer-ref` (`plugin.scm`) resolves
it: the per-buffer override when set, else the config `"ref"` default. Giving a ref always
turns that rendering on, never off, and re-fetches even if it's already on at the same
ref.

`git-diff:refs`, the completion source both toggles complete their ref argument against,
is a minibuffer-target source: its callback gets only `id`/`input`/`cursor`, no pane of
its own. It reads `(focused-pane)` for the buffer whose repo to look in: the same pane
that opened the `:` command line this completes for, and the one its typed command will
receive as its own leading pane once Enter is pressed. Its universe is every local
branch, tag, and remote-tracking ref the focused buffer's repo knows about
(`git for-each-ref`, spawned async against its directory, the same shape as `branch.scm`'s
own branch fetch); it answers `'()` on any failure (no path, not a repo, git missing), since a
ref name is a nice-to-have completion, never worth erroring the command line over.
