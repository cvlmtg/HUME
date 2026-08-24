# core:git-diff

Live, VSCode-style inline git diff. As you type, compares the buffer against a git ref
(default `HEAD`) and renders gutter `+`/`-`/`~` signs, deleted lines as virtual rows,
added/changed lines with a background tint, and word-level highlights inside changed lines.

Requires `core:stdlib` declared or loaded first — config validation calls
`stdlib/config-boolean`/`stdlib/config-string` via `call!` while this plugin's own body is
evaluating, and `call!`'s lazy-miss retry inline-activates a merely declared `core:stdlib`
before the read runs.

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
above is exactly what `manifest.scm` declares by default.

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

## How it works

### File layout

- `plugin.scm` — entry point; wires config, per-buffer state, and the fetch/diff pipeline to
  the buffer lifecycle hooks and the two toggle commands.
- `state.scm` — per-buffer state: the two enable flags, the cached ref blob, the current hunk
  set, the in-flight fetch job, and the per-buffer ref override.
- `diff.scm` — ref-content fetch (`git show` via `spawn-async!`) and the native line-diff call
  (`diff-buffer-lines`), debounced per buffer.
- `render.scm` — pure `hunks → decoration records` functions, one per rendering (gutter signs,
  virtual deleted lines + word highlights, row background tint), each ending in a setter call.

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
