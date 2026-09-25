# core:git-diff — Fetch/diff pipeline and branch tracking

## Fetch/diff pipeline (`diff.scm`)

| Hook | Action |
|---|---|
| `on-buffer-open` | Init state, schedule a refresh |
| `on-text-changed` | Schedule a refresh |
| `on-buffer-save` | Cancel any in-flight fetch, clear `"ref-text"`, schedule a refresh (and a branch refresh) |
| `on-buffer-close` | Cancel any in-flight fetch, drop the entry |

`on-buffer-save` clears the cached `"ref-text"` and cancels any fetch already in flight
*first* — otherwise a fetch spawned just before the save could land inside the debounce
window and re-populate `"ref-text"` with the pre-save blob, which `refresh!` would then
treat as a valid cache and never re-fetch from.

`apply-hunks!` writes `hunks` to state and re-renders, but only when the new hunks
actually differ from what's already rendered — this equality check is exactly why state's
`"hunks"` must always equal what's currently painted. It re-reads the buffer's entry
rather than trusting a caller-held one, since both its call sites (a live local diff and a
`spawn-async!` callback) can run after the buffer closed out from under them. Each
rendering is gated on its own flag independently — signs on and inline off (or the
reverse) is a valid state, and a refresh triggered for either must not touch the other.

`handle-fetch-result!` is the `spawn-async!` callback for `git show`. `stdout` is only
trusted on exit code `0`.

### Severity routing

| Exit code | Severity | Why |
|---|---|---|
| `-1` | `'error` | `git` couldn't even run at all — an environment fault, not a fact about this file |
| any other nonzero, with a runtime ref override set | `'warn` | A direct answer to a command the user just typed |
| any other nonzero, no override | `'trace` | Untracked file, brand-new file, buffer outside any repo, bad `ref` config — indistinguishable without parsing `stderr` further; visible in `:messages` for diagnosis, silent otherwise |

Either way `"ref-text"` becomes `'unavailable`, not `#f` — `#f` means "not fetched yet"
and `refresh!` would re-spawn a `git show` that fails identically on every debounced
keystroke; `'unavailable` is a sticky negative cache, cleared only by `on-buffer-save` or
`force-refresh!`.

A buffer may have closed while the fetch job was in flight (`:bd` and job completion
landing in the same frame — `on-buffer-close`'s cancel can't help here, since the job was
already moved off the cancellable slot and into this very callback). The state writes
(`entry-set!`/`cancel-job!`) already no-op on a missing entry, but `diff-buffer-lines`
raises rather than no-ops for a closed buffer, so the success branch checks the entry
first rather than calling it blind.

`fetch-ref!` runs `git show <ref>:./<name>` with cwd set to the buffer's own directory —
`./`-prefixing the name resolves it relative to cwd, so no `git rev-parse
--show-toplevel` call (and no cached repo root in state) is needed to locate the blob.

`refresh!` is the immediate, non-debounced entry point; `schedule-refresh!` is the
debounced one every hook actually calls. It re-reads the buffer's live entry and path
rather than trusting stale arguments — a debounced fire happens later, after state may
have moved (the same reasoning `branch.scm`'s `refresh-branch!` uses below). Either
rendering alone still needs a fetch, since signs and inline are two independent consumers
of the same hunk store. A pathless buffer (`:messages`, `:ls`) still fires
`on-text-changed`, so it's skipped here rather than erroring — there's nothing to diff
against. A cached `"ref-text"` (a string) means a local diff with no git process on the
keystroke path; `#f` means never fetched or invalidated by a save, so it fetches;
`'unavailable` means the last fetch failed, so it does neither — a save or
`force-refresh!` is what clears it.

`force-refresh!` forces a fetch even through a sticky `'unavailable` cache — used by both
toggle commands so turning signs/inline back on always retries rather than staying silent
because a previous fetch failed. It deliberately does *not* touch `"hunks"`: that field
must keep tracking whatever is actually painted, so that when the fetch reproduces a
result different from the last one (e.g. `'()`, the buffer now matches the ref exactly)
`apply-hunks!`'s equality check sees a real change and re-renders. The caller
(`plugin.scm`'s toggle command) is what paints an instant preview from the already-stored
`"hunks"` first — `force-refresh!` itself doesn't reset `"hunks"`, so without that instant
preview a refresh that legitimately comes back empty would look identical to state's
untouched value and `apply-hunks!` would skip clearing it.

`cancel-fetch!` cancels any in-flight fetch for a buffer without firing its callback —
used both by a newer refresh superseding an older fetch, and from `on-buffer-close`.

`schedule-refresh!` uses `debounce-by`, keyed by `(buffer-key pane)` rather than the pane
value itself — see the [core plugins index](../../README.md#debouncing). It debounces at
150ms — once the ref is cached, a refresh is a local diff, not a network request, so this
can run tighter than `core:lsp`'s inlay hints, which debounce at 200ms to match an LSP
round-trip budget.

## Branch tracking (`branch.scm`)

A second, simpler fetch pipeline alongside `diff.scm`'s — `git rev-parse --abbrev-ref
HEAD` instead of `git show`, pushed straight to a statusline element instead of into
`"hunks"`/a decoration setter. Unlike diff content, a branch name is only ever shown for
the *focused* buffer, so the fetch is driven by `on-buffer-enter` — not `on-buffer-open`,
which fires for every buffer regardless of whether it's ever displayed. It also re-fires
on `on-buffer-save`, since a save-triggered hook or a checkout run alongside the editor in
another terminal can move HEAD without a focus change.

`refresh-branch!` also gates on `branch-element-placed?` — a substring check for
`"steel:git-branch"` in the statusline option — before spawning anything: nothing places
the element by default, so an unconditional fetch would spawn `git` (and its two
stdout/stderr capture threads) on every focus change and save for work nobody can see.
`plugin.scm`'s `on-option-change` hook re-runs `schedule-branch-refresh!` on the focused
buffer whenever the statusline changes, so placing the element drives the first fetch in
immediately rather than waiting for the next focus change or save. It reads
`(focused-pane)` at that moment rather than a captured pane value — the hook fires for a
*global* setting write with no buffer of its own, and this is genuinely follow-the-user
code, refreshing whatever's on screen right now.

There's no cache to invalidate: `refresh-branch!` re-spawns on every debounced fire
(unlike `"ref-text"`, a branch name has no local-diff fallback that would make caching
worth it), so `branch.scm` has no `force-refresh!`/`'unavailable` equivalent.

`refresh-branch!` and `handle-branch-result!` both gate on the entry existing before
touching anything that can outlive the buffer — `buffer-path` and the statusline setter,
unlike this plugin's own state writes, hard-error on a closed buffer rather than no-oping.
That's safe even for a debounce timer or a `spawn-async!` callback that fires *after* the
buffer closes: `on-buffer-close` removes the entry synchronously, before either can run,
so a missing entry is a reliable "this buffer is dead" signal — the same reasoning
`apply-hunks!` and `handle-fetch-result!` already rely on for the diff pipeline above.

### Severity: simpler than the diff pipeline's

| Exit code | Severity |
|---|---|
| `-1` (git couldn't run at all) | `'error` |
| any other nonzero (overwhelmingly "not a git repository") | cleared silently, no log |

Even with the element placed, branch tracking runs for every buffer that hooks fire on,
not only ones a user opted into with `:toggle-git-signs` — logging the expected-failure
case at any visible level would fill `:messages` for buffers that never asked for this.
