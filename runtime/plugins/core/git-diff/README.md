# core:git-diff

Live, VSCode-style inline git diff. As you type, compares the buffer against a git ref
(default `HEAD`) and renders gutter `+`/`-`/`~` signs, deleted lines as virtual rows,
added/changed lines with a background tint, and word-level highlights inside changed lines.

Requires `core:stdlib` declared or loaded first — config validation calls
`stdlib/config-boolean`/`stdlib/config-string` via `call!` while this plugin's own body is
evaluating, and `call!`'s lazy-miss retry inline-activates a merely declared `core:stdlib`
before the read runs. See ["Depending on another
plugin"](https://cvlmtg.github.io/HUME/plugins.html#depending-on-another-plugin) for why the
`(declared-plugins)` check at the top of `plugin.scm` is enough here.

## Usage

```scheme
(declare-plugin "core:stdlib")
(declare-plugin "core:git-diff")
```

The bare `declare-plugin` above resolves this plugin's `manifest.scm`, which activates it on
the first buffer opened or when typing one of its commands (`:toggle-git-signs` and
`:toggle-inline-diff`).

## Customizing activation

Pass `#:commands`/`#:events`/`#:languages` explicitly to override the manifest's defaults:

```scheme
(declare-plugin "core:git-diff"
  #:events '(on-buffer-open)
  #:commands '("toggle-git-signs" "toggle-inline-diff"))
```

Any explicit `#:commands`/`#:events`/`#:languages` bypasses the manifest entirely — the example
above is exactly what `manifest.scm` declares by default. `#:events` is `on-buffer-open` because
signs default on (see Config below), so the plugin must wake on the first buffer opened rather
than wait for a command typed by hand; `#:commands` covers both toggles, for a user who declares
an explicit activation list instead of taking the manifest's defaults.

## Commands

| Command | Effect |
|---|---|
| `:toggle-git-signs [ref]` | Toggle gutter signs for the current buffer |
| `:toggle-inline-diff [ref]` | Toggle inline rendering (virtual deleted lines, word highlights, background tint) for the current buffer |

Both commands take an optional git ref, e.g. `:toggle-inline-diff HEAD~2` or
`:toggle-git-signs c97ce99`. Giving a ref always turns that rendering on (never off) and
points it at the given ref — running it again with the same ref just re-fetches. The ref is
per-buffer and shared between the two commands: switching it from either one re-renders
whichever of the two is currently on. It's sticky across a bare toggle off/on — turning a
rendering back off and on again keeps the last ref you gave, not the config default. Without
an argument, both commands are a plain on/off toggle at whichever ref is currently in effect
(the config default until you've given one explicitly).

Inline rendering's background tint and word highlights depend on your theme defining colors
for them. HUME's four bundled themes (dark, light, sand, gruvbox) do; a custom theme that
doesn't define `diff.plus`/`diff.minus`/`diff.delta` (row tint) or
`diff.plus.word`/`diff.minus.word` (word highlights) shows those two with no visible color
until it adds them — gutter signs and the virtual deleted-line rows themselves are unaffected.

A file git doesn't know about yet (untracked, brand-new, or outside a repo) shows no diff.
It starts showing one once the file is tracked, after its buffer is next saved or one of the
toggles above is cycled. A ref that doesn't resolve (typo, doesn't exist) also shows no diff —
given explicitly, this is reported on the status line; the untracked-file case above stays
silent.

No default key bindings — bind them yourself, e.g.:

```scheme
(bind-key! 'normal "g Shift-d" "toggle-inline-diff")
```

## Config

Pass via `#:config` on `declare-plugin`/`load-plugin`:

```scheme
(load-plugin "core:stdlib")
(declare-plugin "core:git-diff"
  #:config (hash "signs" #t "inline" #f "ref" "HEAD"))
```

| Key | Type | Default | Effect |
|---|---|---|---|
| `"signs"` | bool | `#t` | Whether gutter signs start on for a newly opened buffer |
| `"inline"` | bool | `#f` | Whether inline rendering starts on for a newly opened buffer |
| `"ref"` | string | `"HEAD"` | The default git ref a buffer diffs against, until overridden per-buffer via `:toggle-git-signs`/`:toggle-inline-diff` — see Commands |

Signs default on: cheap, with no line-shifting side effects. Inline rendering defaults off:
it moves virtual rows into the buffer's visual flow, which not every user wants on by default.

`(plugin-config)` only returns the real config hash while `plugin.scm`'s body is being
evaluated, so all three values above are read once into `define`s at load time, never from
inside a command or hook handler. `"ref"`'s value is validated at that same point too, so a
bad config value fails at load rather than on the first debounced refresh — it's only the
*default*, though: `plugin.scm`'s `git-diff/buffer-ref` resolves the ref actually used for a
given buffer, falling back to this default only when no per-buffer runtime override (set via
an explicit-ref toggle invocation) is in state.

## How it works

### File layout

- `plugin.scm` — entry point; wires config, per-buffer state, and the fetch/diff pipeline to
  the buffer lifecycle hooks and the two toggle commands.
- `state.scm` — per-buffer state: the two enable flags, the cached ref blob, the current hunk
  set, the in-flight fetch job, and the per-buffer ref override. See "State" below.
- `diff.scm` — ref-content fetch (`git show` via `spawn-async!`) and the native line-diff call
  (`diff-buffer-lines`), debounced per buffer. See "Fetch/diff pipeline" below. `diff-words`
  (word-level diff) is not called here — it's called from `render.scm`, where the records it
  feeds are built.
- `render.scm` — pure `hunks → decoration records` functions, one per rendering (gutter signs,
  virtual deleted lines + word highlights, row background tint), each ending in a setter call.
  See "Rendering" below.

No native diff algorithm lives here — `diff-buffer-lines`/`diff-words` already wrap `similar`/
Myers in Rust. This plugin is orchestration (state, debounce, git process management,
decoration construction) over those.

### Signs and inline rendering are one plugin, not two

Both are renderings of the same underlying hunk data, produced by the same `git show`/
`diff-buffer-lines` pipeline. Shared: repo probe, ref fetch, line diff, debounce, ref-cache
invalidation, hunk-equality check to skip no-op refreshes. Differing: a `set-signs!` call
versus the virtual-line/tint/word-span construction — roughly 20% of the plugin, not enough to
justify splitting the other 80%. Splitting them would also open a window where the gutter and
the inline view disagree about the same file, since each would fetch and diff independently.

### State

One `(box (hash))` keyed by buffer id — the same per-key mutable-table idiom `debounce-by`
uses (`hume-scripting/src/builtins/bootstrap.scm`). Steel's `hash` is persistent, so mutation
is swap-the-box, not in-place update. Each entry, built from the single `fresh-entry` SSOT,
holds:

- `"signs?"`/`"inline?"` — the two independent enable flags.
- `"ref-text"` — the git-fetch/diff pipeline's cache: a string (the cached `git show` blob),
  `#f` (needs (re-)fetching), or `'unavailable` (the last fetch failed; a sticky negative
  cache so a doomed fetch isn't retried every debounce fire — see "Fetch/diff pipeline").
- `"hunks"` — the verbatim tuples `diff-buffer-lines` last returned, always kept in sync with
  what's actually painted, never a signs-derived shape. This is the *additivity invariant*:
  every renderer in `render.scm` stays a pure function over this one shared hunk set, so
  adding a new rendering is one function and one setter call, touching neither this file, the
  fetch pipeline, nor the lifecycle hooks in `plugin.scm`.
- `"job"` — the in-flight `spawn-async!` id, or `#f`, so a superseded fetch can be cancelled.
- `"ref"` — `#f` (use the config default) or a string (a runtime override set via
  `:toggle-git-signs <ref>`/`:toggle-inline-diff <ref>`, shared by both renderers — see
  `plugin.scm`'s `git-diff/buffer-ref`).

`entry-set!` is a no-op when `bid` has no tracked entry — a late `spawn-async!` callback for a
buffer closed while its fetch was in flight must not resurrect state for it. `ensure-entry!` is
the opposite: it resurrects a missing entry from `fresh-entry` rather than no-opping, for a
write path (an explicit-ref or bare toggle invocation) that must succeed even for a buffer
whose `on-buffer-open` never fired — a user can override the manifest's `#:events` with a
`#:commands`-only activation list, so the plugin only loads on the first toggle command.
`toggle-flag!` is built from `ensure-entry!` plus `entry-set!` rather than its own box/hash
pair — the caller needing the flipped value back is just an extra `hash-ref` around the two,
not a reason to duplicate them.

### Fetch/diff pipeline

`on-buffer-save` clears the cached `ref-text` (a commit, checkout, or index change while the
buffer was open makes it stale) and cancels any fetch already in flight first — otherwise a
fetch spawned just before the save could land inside the debounce window and re-populate
`ref-text` with the pre-save blob, which `refresh!` would then treat as a valid cache and
never re-fetch from.

`apply-hunks!` writes `hunks` to state and re-renders, but only when the new hunks actually
differ from what's already rendered (nvim's `_hunks_equal`) — this equality check is exactly
why state's `hunks` must always equal what's currently painted. It re-reads the buffer's entry
rather than trusting a caller-held one, since both its call sites (a live local diff and a
`spawn-async!` callback) can run after the buffer closed out from under them. Each rendering
is gated on its own flag independently — a user can have signs on and inline off, or the
reverse, and a refresh triggered for either must not touch the other.

`handle-fetch-result!` is the `spawn-async!` callback for `git show`. `stdout` is only trusted
on `exit-code 0`. Otherwise, three severities: `-1` (the contract at
`hume-scripting/src/builtins/process.rs:18-27`) means `git` couldn't even run — an environment
fault, not a fact about this file — logged `'error` so it reaches the status line and the
unseen-count indicator. A real nonzero exit against a buffer's runtime ref override is logged
`'warn` — the failure is a direct answer to a command the user just typed, unlike the
config-default case. Every other nonzero exit (untracked file, brand-new file, buffer outside
any repo, bad `ref` config — indistinguishable without parsing `stderr` further) is expected
and logged `'trace`: visible in `:messages` for diagnosis, but silent otherwise. Either way
`ref-text` becomes `'unavailable`, not `#f` — `#f` means "not fetched yet" and `refresh!` would
re-spawn a `git show` that fails identically on every debounced keystroke; `'unavailable` is a
sticky negative cache, cleared only by `on-buffer-save` or `force-refresh!`.

`fetch-ref!` runs `git show <ref>:./<name>` with cwd set to the buffer's own directory —
`./`-prefixing the name resolves it relative to cwd, so no `git rev-parse --show-toplevel`
call (and no cached repo root in state) is needed to locate the blob.

`refresh!` is the immediate, non-debounced entry point; `schedule-refresh!` (below) is the
debounced one every hook actually calls. It re-reads the buffer's live entry and path rather
than trusting stale arguments — the same reasoning as `inlay.scm`'s `refresh-hints`: a
debounced fire happens later, after state may have moved. Either rendering alone still needs a
fetch, since signs and inline are two independent consumers of the same hunk store. A pathless
buffer (`:messages`, `:ls`) still fires `on-text-changed`, so it's skipped here rather than
erroring — there's nothing to diff against. A cached `ref-text` (a string) means a local diff
with no git process on the keystroke path; `#f` means never fetched or invalidated by a save,
so it fetches; `'unavailable` means the last fetch failed, so it does neither — a save or
`force-refresh!` is what clears it.

`force-refresh!` forces a fetch even through a sticky `'unavailable` cache — used by both
toggle commands so turning signs/inline back on always re-tries rather than staying silent
because a previous fetch failed. It deliberately does *not* touch `hunks`: that field must keep
tracking whatever is actually painted, so that when the fetch reproduces a result different
from the last one (e.g. `'()`, the buffer now matches the ref exactly) `apply-hunks!`'s
equality check sees a real change and re-renders. The caller (`plugin.scm`'s toggle command) is
what paints an instant preview from the already-stored `hunks` first — force-refresh itself
doesn't reset `hunks`, so without that instant-preview step a refresh that legitimately comes
back empty would look identical to state's untouched value and `apply-hunks!` would skip
clearing it.

`cancel-fetch!` cancels any in-flight fetch for a buffer without firing its callback — used
both by a newer refresh superseding an older fetch, and from `on-buffer-close` (`state.scm` has
no async awareness of its own).

`schedule-refresh!` uses `debounce-by`, not `debounce` — keyed per `bid`, so one buffer's
edits never cancel another's pending refresh (`inlay.scm`'s same rationale). It debounces at
150ms rather than `inlay.scm`'s 200ms LSP round-trip budget — once the ref is cached, a
refresh is a local diff, not a network request.

### Rendering

Every function in `render.scm` is a pure `hunks → decoration records` view over the one hunk
shape `state.scm` stores (the additivity invariant described under "State"), ending in exactly
one setter call — `render-inline!` is the one documented exception, below. All setters share a
feature-scoped source name, `"git-diff"`, not the `core:git-diff` plugin id — matching
`core:lsp`'s own decoration sources (`"lsp-diagnostics"`, `"lsp-inlay-hints"`).

Signs render at VSCode/gitsigns density (one per changed line, not one per hunk — a 20-line
paste shows 20 `+` marks). `render-inline!` makes two setter calls
(`set-virtual-lines!`/`set-extra-highlights!`) instead of this file's usual one: a single
`diff-words` pass inherently produces two decoration kinds — old-side virtual rows and
new-side highlight spans — and splitting it into two renderers to keep one setter each would
call `diff-words` twice for no benefit. `render-for!` is the one place a flag key
(`"signs?"`/`"inline?"`) maps to its renderer(s), so every caller that needs to paint or clear
a rendering (`diff.scm`'s `apply-hunks!` on a live refresh, `plugin.scm`'s toggle command on
enable/disable) goes through it rather than re-stating the mapping.
