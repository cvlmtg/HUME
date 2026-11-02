# core:git-diff — Fetch/diff pipeline and branch tracking

## Fetch/diff pipeline (`diff.scm`)

```
on-buffer-open/on-text-changed/on-buffer-save
        │
        ▼
schedule-refresh! (150ms debounce-by, keyed by buffer)
        │
        ▼
ref-text cached? ──yes──▶ diff-buffer-lines (local, no process) ─┐
        │no                                                      │
        ▼                                                        │
   git cat-file --filters <ref>:./<file>  (async)                │
        │                                                        │
        ▼                                                        │
handle-fetch-result! ──▶ diff-buffer-lines                       │
                                                                 ▼
                                                          apply-hunks!
                                                                 │
                                                                 ▼
                                                      per-rendering reconcile
                                                    (paints only what differs
                                                     from what's already painted)
                                                       ┌─────────┴─────────┐
                                                       ▼                   ▼
                                                  render-signs!    render-inline! +
                                                                   render-line-bgs!
```

### Hooks

| Hook | Action |
|---|---|
| `on-buffer-open` | Track the buffer, schedule a refresh |
| `on-buffer-enter` | Cancel any in-flight fetch, clear `"ref-text"`, schedule a refresh and a branch refresh |
| `on-focus-gained` | The same for every buffer a pane shows, with the branch refresh for the focused one |
| `on-text-changed` | Schedule a refresh when the buffer can diff |
| `on-buffer-save` | Schedule a refresh and a branch refresh |
| `on-buffer-close` | `remove-buffer!` cancels any in-flight diff and branch fetch and drops the entry |

A commit or checkout in another terminal moves the ref without any event, so
`on-buffer-enter` and `on-focus-gained` refetch through `invalidate-ref!`: it clears the
cached `"ref-text"` and cancels any fetch already in flight before it schedules the refresh.
Without the cancel, a fetch spawned just before could land inside the debounce window and
repopulate `"ref-text"` with the stale blob, which `refresh!` would treat as a valid cache
and never re-fetch. `on-focus-gained` fires when the terminal regains focus and when an
inline shell command returns, so a commit made with `:!git commit` is seen too. A terminal
that does not report focus (tmux without `focus-events on`), or a commit made by a background
process while HUME keeps focus, is seen at the next buffer enter.

A save does not change the blob at `<ref>:./<name>`, so `on-buffer-save` keeps the cache.
The cache belongs to the path it was fetched for (`"ref-path"`, recorded when the fetch
starts): a save to a new path (`:w <path>`) changes the buffer's path, and the refresh the
save schedules finds the cache belongs to another path and fetches again.

### Fetching

`refresh!` starts `git cat-file --filters --end-of-options <ref>:./<name>` through
`spawn-job!`, which cancels any earlier fetch, with cwd set to the buffer's own directory.
The `./` prefix resolves the name relative to cwd, so locating the blob needs no
`git rev-parse --show-toplevel` call and no cached repo root in state. `--filters` applies
the file's checkout filters to the blob, so a file stored through Git LFS, `ident` or a
smudge filter diffs as the text on disk. `--end-of-options` keeps a ref that starts with `-`
from being read as an option.

`handle-fetch-result!` is the `spawn-job!` callback, so it runs only for the fetch that
still owns the `"diff"` slot. It trusts `stdout` only on exit code `0`. On success it stores the blob in `"ref-text"` and applies the diff;
on failure it sets `"ref-text"` to `'unavailable` and applies `'()`, which clears the painted
hunks.

A buffer can close, or a save can cancel the fetch, while its completion is already queued
for the same frame. The cancel drops that completion, since `cancel-async!` stops a callback
even once it is due. `spawn-job!` also runs a result only while its job still owns the slot,
so a result is dropped when the entry is gone or the slot holds another job (or none). See
[Closed buffers](#closed-buffers).

#### Severity routing

The severity of a failure is chosen when the fetch starts and travels with it: `refresh!`
takes it and passes it to `handle-fetch-result!`.

| Exit code | Fetch started by | Severity | Why |
|---|---|---|---|
| `#f` (none) | anything | `'error` | `git` could not run: an environment fault, not a fact about this file |
| any other nonzero | a toggle, with a runtime ref override set | `'warning` | A direct answer to a command the user just typed |
| any other nonzero | anything else | `'trace` | Untracked file, brand-new file, buffer outside any repo, bad `ref` config, or a background refetch against an override already reported. These cannot be told apart without parsing `stderr` further; the line is visible in `:messages` for diagnosis and otherwise quiet |

Either way `"ref-text"` becomes `'unavailable`, not `#f`. `#f` means "not fetched yet", and
`refresh!` would respawn a `git cat-file` that fails the same way on every debounced keystroke.
`'unavailable` is a sticky negative cache, cleared by `drop-ref-text!` (`on-buffer-enter`,
`on-focus-gained`, an explicit-ref toggle, a path change) or by `force-refresh!`.

### Refresh entry points

`schedule-refresh!` is the debounced entry point every hook calls. Hooks pass the config
ref, and `refresh!` resolves the buffer's own ref when it fires, so a ref switched inside the
debounce window is the one fetched. It uses `debounce-by`,
keyed by `(buffer-key pane)` and not the pane value (see the
[core plugins index](../../README.md#debouncing)), at 150ms. With the ref cached a refresh
is a local diff and not a git process, so it can debounce tighter than `core:lsp`'s inlay
hints at 200ms.

`refresh!` is the immediate entry point. It re-reads the buffer's live entry and path, since
a debounced fire happens after state may have moved. `diffable-path` skips a buffer that
shows no rendering (both off, or inline covered with signs off) and a pathless buffer
(`:messages`), which has nothing to diff against. The `on-text-changed` hook asks it too
before scheduling, since `:messages` changes on every log line. The `"ref-text"` value
decides the rest:

| `"ref-text"` | `refresh!` does |
|---|---|
| a string | A local diff, no git process |
| `#f`, no fetch in flight | Fetches |
| `#f`, a fetch in flight | Nothing: the fetch's callback diffs the live text when it lands |
| `'unavailable` | Nothing |

`force-refresh!` clears `"ref-text"` unless it is a string, then calls `refresh!`, so a
failed earlier fetch is retried and a cached blob gives a local diff. Both toggle commands
call it, so turning a rendering back on retries and does not stay empty because a previous
fetch failed. Turning a rendering on paints nothing itself: it waits for the refresh's
fresh diff.

`drop-ref-text!` cancels any in-flight fetch for a buffer without firing its callback and
clears `"ref-text"`. It is called from `invalidate-ref!`, the explicit-ref toggle, and
`refresh!` when the cache belongs to another path.

### Applying hunks

`apply-hunks!` takes the fresh hunks and compares each rendering's target with what it last
painted (`"signs-painted"`/`"inline-painted"`), painting only on a difference. A rendering's
target is what it draws when its flag is on (for inline, also when no other plugin's source
covers the buffer), else `'()`. For signs that is the sign records, which read only the
hunks' line numbers, so typing inside an already changed line repaints no signs. For inline
it is the hunks. A painted target is kept only for that comparison: only a refresh paints
hunks, so nothing else can paint hunks for text that has since changed. `hide-renderings!`, which the
toggle-off and the `render-diff` command's cover call, takes no hunks and only clears the
renderings that are hidden. Signs on with
inline off, or the reverse, is a valid state, and a refresh for one must not touch the other.
A rendering that never painted anything is never touched, so a buffer whose signs are
turned off before any appeared never reserves a gutter slot.

## Branch tracking (`branch.scm`)

A second, simpler pipeline next to `diff.scm`'s. It runs `git rev-parse --abbrev-ref HEAD`
and pushes the result to the `"steel:git-branch"` statusline element, not into the
diff renderings or a decoration setter. A branch name is shown only for the focused buffer, so the fetch is
driven by `on-buffer-enter`, not `on-buffer-open`, which fires for every buffer whether or
not it is displayed. It also runs on `on-buffer-save`, since a save to a new path can
move the buffer into another repository, and on `on-focus-gained`, for a checkout made in
another terminal.

Branch tracking has no on/off flag: placing the element is the switch. `refresh-branch!`
checks `branch-element-placed?` (whether `"steel:git-branch"` is one of the statusline
option's left, center or right elements) before it spawns anything. Nothing places the
element by default, so an unconditional fetch would spawn `git` on every focus change and
save for work nobody can see. `plugin.scm`'s `on-option-change` hook re-runs
`schedule-branch-refresh!` on the focused buffer whenever the `statusline` option changes,
so placing the element starts the first fetch at once. It reads `(focused-pane)` at that
moment: the hook fires for a global setting write with no buffer of its own, and the branch
shown is the one on screen.

The text comes from the `'branch-style` config value, which `plugin.scm` passes down
through `schedule-branch-refresh!` the way the diff pipeline receives `ref`;
`format-branch` is the one place that turns a name into the element's text.

A branch name has no local-diff fallback that would make caching worthwhile, so
`refresh-branch!` respawns on every debounced fire and `branch.scm` has no `force-refresh!`
or `'unavailable` state. The refresh debounces at 150ms, keyed per buffer.

| Exit code | Result |
|---|---|
| `0` | The element shows the branch as `format-branch` writes it |
| `#f` (git could not run) | Logs `'error`, clears the element |
| any other nonzero (usually "not a git repository") | Clears the element, logs nothing |

Branch tracking runs for every buffer the hooks fire on, not only those opted in with
`:toggle-git-signs`, so the usual failure stays out of `:messages`.

## Closed buffers

`buffer-path`, the statusline setter, `diff-buffer-lines` and the plugin's own
`entry-set!` raise for a closed buffer. `refresh-branch!` and `refresh!` therefore check that
the buffer's entry exists before they call one of them, and `spawn-job!` runs a result
callback only while its job still owns its slot, which needs the entry. That check is reliable, even in a debounce timer or a
`spawn-async!` callback that fires after the buffer closes, because `on-buffer-close`
removes the entry synchronously before either can run: a missing entry means the buffer is
gone.
