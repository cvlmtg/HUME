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
   git show <ref>:./<file>  (async)                              │
        │                                                        │
        ▼                                                        │
handle-fetch-result! ──▶ diff-buffer-lines                       │
                                                                 ▼
                                                          apply-hunks!
                                                    (skips if unchanged from
                                                     what's already painted)
                                                                 │
                                                                 ▼
                                                   render-for! (signs?/inline?)
                                                       ┌─────────┴─────────┐
                                                       ▼                   ▼
                                                  render-signs!    render-inline! +
                                                                   render-line-bgs!
```

### Hooks

| Hook | Action |
|---|---|
| `on-buffer-open` | Init state, schedule a refresh |
| `on-text-changed` | Schedule a refresh |
| `on-buffer-save` | Cancel any in-flight fetch, clear `"ref-text"`, schedule a refresh and a branch refresh |
| `on-buffer-close` | Cancel any in-flight diff and branch fetch, drop the entry |

`on-buffer-save` clears the cached `"ref-text"` and cancels any fetch already in flight
before it schedules the refresh. A fetch spawned just before the save could otherwise land
inside the debounce window and repopulate `"ref-text"` with the pre-save blob, which
`refresh!` would treat as a valid cache and never re-fetch.

### Fetching

`fetch-ref!` cancels any earlier fetch, then runs `git show <ref>:./<name>` with cwd set to
the buffer's own directory. The `./` prefix resolves the name relative to cwd, so locating
the blob needs no `git rev-parse --show-toplevel` call and no cached repo root in state.

`handle-fetch-result!` is the `spawn-async!` callback. It clears `"job"` and trusts `stdout`
only on exit code `0`. On success it stores the blob in `"ref-text"` and applies the diff;
on failure it sets `"ref-text"` to `'unavailable` and applies `'()`, which clears the painted
hunks.

A buffer can close while its fetch is in flight, for example when `:bd` and the job's
completion land in the same frame. `on-buffer-close`'s cancel cannot help, because the job
has already left the cancellable slot and entered this callback. The state writes
(`entry-set!`, `cancel-job!`) no-op on a missing entry, but `diff-buffer-lines` raises for
a closed buffer, so the success branch checks that the entry exists before calling it. See
[Closed buffers](#closed-buffers).

#### Severity routing

| Exit code | Severity | Why |
|---|---|---|
| `-1` | `'error` | `git` could not run: an environment fault, not a fact about this file |
| any other nonzero, with a runtime ref override set | `'warn` | A direct answer to a command the user just typed |
| any other nonzero, no override | `'trace` | Untracked file, brand-new file, buffer outside any repo, bad `ref` config. These cannot be told apart without parsing `stderr` further; the line is visible in `:messages` for diagnosis and otherwise quiet |

Either way `"ref-text"` becomes `'unavailable`, not `#f`. `#f` means "not fetched yet", and
`refresh!` would respawn a `git show` that fails the same way on every debounced keystroke.
`'unavailable` is a sticky negative cache, cleared by `on-buffer-save` or `force-refresh!`.

### Refresh entry points

`schedule-refresh!` is the debounced entry point every hook calls. It uses `debounce-by`,
keyed by `(buffer-key pane)` and not the pane value (see the
[core plugins index](../../README.md#debouncing)), at 150ms. With the ref cached a refresh
is a local diff and not a git process, so it can debounce tighter than `core:lsp`'s inlay
hints at 200ms.

`refresh!` is the immediate entry point. It re-reads the buffer's live entry and path, since
a debounced fire happens after state may have moved. A buffer with neither rendering on is
skipped, as is a pathless buffer (`:messages`, `:ls`), which still fires `on-text-changed`
and has nothing to diff against. The `"ref-text"` value decides the rest:

| `"ref-text"` | `refresh!` does |
|---|---|
| a string | A local diff, no git process |
| `#f` | Fetches |
| `'unavailable` | Nothing |

`force-refresh!` clears `"ref-text"` unless it is a string, then calls `refresh!`, so a
failed earlier fetch is retried and a cached blob gives a local diff. Both toggle commands
call it, so turning a rendering back on retries and does not stay empty because a previous
fetch failed. It leaves `"hunks"` as it is: that field must keep equal to what is painted.
The toggle command paints the stored hunks before it calls `force-refresh!`. Without that,
a re-enabled rendering whose fetch returns the same hunks as the stored ones would be
skipped by `apply-hunks!`'s equality check and stay blank.

`cancel-fetch!` cancels any in-flight fetch for a buffer without firing its callback. It is
called from `on-buffer-save`, `on-buffer-close` and `fetch-ref!`.

### Applying hunks

`apply-hunks!` writes the hunks to state and re-renders only when they differ from what is
already painted, which is why `"hunks"` must equal what is painted. It re-reads the buffer's
entry and does not trust one held by the caller. Each rendering is gated on its own flag:
signs on with inline off, or the reverse, is a valid state, and a refresh for one must not
touch the other.

## Branch tracking (`branch.scm`)

A second, simpler pipeline next to `diff.scm`'s. It runs `git rev-parse --abbrev-ref HEAD`
and pushes the result to the `"steel:git-branch"` statusline element, not into `"hunks"` or
a decoration setter. A branch name is shown only for the focused buffer, so the fetch is
driven by `on-buffer-enter`, not `on-buffer-open`, which fires for every buffer whether or
not it is displayed. It also runs on `on-buffer-save`, since a hook run on save or a
checkout in another terminal can move HEAD without a focus change.

`refresh-branch!` checks `branch-element-placed?` (a substring check for
`"steel:git-branch"` in the statusline option) before it spawns anything. Nothing places the
element by default, so an unconditional fetch would spawn `git` on every focus change and
save for work nobody can see. `plugin.scm`'s `on-option-change` hook re-runs
`schedule-branch-refresh!` on the focused buffer whenever the `statusline` option changes,
so placing the element starts the first fetch at once. It reads `(focused-pane)` at that
moment: the hook fires for a global setting write with no buffer of its own, and the branch
shown is the one on screen.

A branch name has no local-diff fallback that would make caching worthwhile, so
`refresh-branch!` respawns on every debounced fire and `branch.scm` has no `force-refresh!`
or `'unavailable` state. The refresh debounces at 150ms, keyed per buffer.

| Exit code | Result |
|---|---|
| `0` | The element shows `(<branch>)` |
| `-1` (git could not run) | Logs `'error`, clears the element |
| any other nonzero (usually "not a git repository") | Clears the element, logs nothing |

Branch tracking runs for every buffer the hooks fire on, not only those opted in with
`:toggle-git-signs`, so the usual failure stays out of `:messages`.

## Closed buffers

`buffer-path`, the statusline setter and `diff-buffer-lines` raise for a closed buffer where
this plugin's own state writes no-op. `refresh-branch!`, `handle-branch-result!`,
`apply-hunks!` and `handle-fetch-result!` therefore check that the buffer's entry exists
before they call one of them. That check is reliable, even in a debounce timer or a
`spawn-async!` callback that fires after the buffer closes, because `on-buffer-close`
removes the entry synchronously before either can run: a missing entry means the buffer is
gone.
