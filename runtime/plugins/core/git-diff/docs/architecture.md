# core:git-diff — Architecture

The plugin is orchestration: state, debounce, git process management and decoration
construction over one native builtin, `diff-buffer-lines` (line diff against a ref blob,
each hunk carrying its changed-word spans). Signs and inline rendering share
one hunk store and one fetch/diff pipeline: the repo probe, ref fetch, line diff, debounce,
ref-cache invalidation and the reconcile step that skips no-op repaints. They differ
only in the decoration construction (a `set-signs!` call, versus virtual lines, word spans
and a line tint).

## File layout

| File | Owns |
|---|---|
| `plugin.scm` | Entry point; wires config, per-buffer state, and the fetch/diff pipeline to the buffer lifecycle hooks and the two toggle commands |
| `state.scm` | Per-buffer state (see below) |
| `diff.scm` | Ref-content fetch, the native line-diff call, and the reconcile step that paints both renderings, debounced per buffer (see `docs/pipeline.md`) |
| `branch.scm` | Current-branch fetch, debounced per buffer, pushed to the statusline (see `docs/pipeline.md`) |
| `render.scm` | Pure `hunks → decoration records` functions, one per rendering, and the word spans that feed the inline records. The inline renderers take their decoration source as a parameter, so `plugin.scm`'s `git-diff/render-diff` command can draw another plugin's hunks (see `docs/rendering.md`) |

## State (`state.scm`)

One `(box (hash))` keyed by `(buffer-key pane)`, not by a pane value itself. A command's
own pane and a hook's pane-less value for the same buffer must resolve to the same entry
(see the [core plugins index](../../README.md#per-buffer-state)). Each entry is built from
one `fresh-entry` and holds:

| Field | Holds |
|---|---|
| `"signs?"` / `"inline?"` | The two independent enable flags |
| `"ref-text"` | The fetch/diff cache (see the table below) |
| `"signs-painted"` / `"inline-painted"` | The hunks each rendering last painted; `'()` when it shows nothing |
| `"job"` | The in-flight diff-fetch `spawn-async!` id, or `#f` |
| `"ref"` | `#f` (use the config default) or a runtime override string set via `:toggle-git-signs <ref>`/`:toggle-inline-diff <ref>` |
| `"branch-job"` | The in-flight branch-fetch `spawn-async!` id, or `#f`; independent of `"job"` |
| `"covered-by"` | The other plugins' decoration sources drawing through `git-diff/render-diff`, which hide the inline rendering (see `docs/rendering.md`) |

### One hunk set

Every renderer in `render.scm` is a pure function over the one hunk set a refresh passes
to `apply-hunks!`. The plugin does not store it. A rendering that is off or covered shows
nothing. `diff.scm` compares each rendering's target (the hunks, or `'()`) with its
`*-painted` field and paints only on a difference. A new rendering is one function,
one setter call and one `*-painted` field.

### `"ref-text"` states

| Value | Meaning |
|---|---|
| a string | The cached `git cat-file` blob; a refresh is a local diff, no process |
| `#f` | Not yet fetched, or invalidated by a save; the next refresh fetches |
| `'unavailable` | The last fetch failed: a sticky negative cache, so a doomed fetch is not retried on every debounce fire |

### `entry-set!` and `ensure-entry!`

Both write to an entry. They differ on a missing one, the two write-path shapes from the
[core plugins index](../../README.md#stale-async-work):

| | On a missing entry | Used by |
|---|---|---|
| `entry-set!` | No-ops | A `spawn-async!` callback. A late callback for a buffer closed while its fetch was in flight must not resurrect state for it |
| `ensure-entry!` | Resurrects one from `fresh-entry` | A toggle command. An explicit-ref or bare toggle must succeed even for a buffer whose `on-buffer-open` never fired (an activation list can override the manifest's `#:events` with a `#:commands`-only list) |

`toggle-flag!` is `ensure-entry!` plus `entry-set!`, with an extra `hash-ref` to return the
flipped value.

`spawn-job!` starts a `spawn-async!` job in a slot (`"job"`/`"branch-job"`), cancelling the
slot's current job first. The result callback runs only while its job still owns the slot, so
a result queued before a cancel or a replacement, or arriving after the buffer closed, is
dropped. `cancel-job!` cancels a slot's job without firing its callback. `diff.scm` and
`branch.scm` both go through these two, differing only in the key. `remove-buffer!` cancels
every slot's live job before it drops the entry, so a new slot needs no change in the close hook.

## Ref handling

### Per-buffer ref

Both commands share one per-buffer `"ref"` override. `buffer-ref` (`state.scm`) resolves
it: the per-buffer override when set, else the config `"ref"` default. A refresh resolves it
when it runs, not when a hook scheduled it. The shared body
`run-toggle!` handles both commands:

- With a ref argument it ensures the entry exists, turns that rendering on, stores the ref,
  drops `"ref-text"` and any fetch in flight (`drop-ref-text!`) and force-refreshes. A ref always turns the
  rendering on and always re-fetches, even if it is already on at the same ref.
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
