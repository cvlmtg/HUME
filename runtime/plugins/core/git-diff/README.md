# core:git-diff

Live, VSCode-style inline git diff — compares the buffer against a git ref (default
`HEAD`) as it's edited, rendering gutter `+`/`-`/`~` signs, deleted lines as virtual
lines, added/changed lines with a background tint, and word-level highlights inside
changed lines. Also keeps a `"steel:git-branch"` statusline element fresh for the focused
buffer — place it yourself, no config needed.

## Usage

```scheme
(declare-plugin "core:stdlib")
(declare-plugin "core:git-diff"
  #:config (hash "signs" #t "inline" #f "ref" "HEAD"))
```

- **Depends on:** `core:stdlib` — config validation calls `stdlib/config-boolean`/
  `stdlib/config-string` at load time.
- **Activates on:** the first buffer opened, or the first `:toggle-git-signs`/
  `:toggle-inline-diff` typed. `"signs"` defaults on (cheap, no line-shifting side
  effects); `"inline"` defaults off (it moves virtual lines into the buffer's visual
  flow).
- **Branch tracking has no config flag — placement is the switch.** It doesn't fetch
  until `"steel:git-branch"` appears in your own `configure-statusline!` call, and starts
  the moment it does.
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-git-diff)
  for value semantics and key-binding examples — no default key bindings ship with this
  plugin.

## Commands

| Command | Effect |
|---|---|
| `:toggle-git-signs [ref]` | Toggle gutter signs for the current buffer — Tab-completes branches/tags |
| `:toggle-inline-diff [ref]` | Toggle inline rendering (virtual deleted lines, word highlights, background tint) for the current buffer — Tab-completes branches/tags |

## Data flow

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

Branch tracking is a second, independent pipeline on `on-buffer-enter`/`on-buffer-save`,
also 150ms-debounced, running `git rev-parse --abbrev-ref HEAD` straight to the
statusline element instead of into the hunk store.

No native diff algorithm lives in this plugin — `diff-buffer-lines`/`diff-words` already
wrap `similar`/Myers in Rust. This plugin is orchestration (state, debounce, git process
management, decoration construction) over those.

## Signs and inline rendering are one plugin, not two

Both are renderings of the same underlying hunk data, produced by the same `git show`/
`diff-buffer-lines` pipeline. Shared: repo probe, ref fetch, line diff, debounce,
ref-cache invalidation, hunk-equality check to skip no-op refreshes. Differing: a
`set-signs!` call versus the virtual-line/tint/word-span construction — roughly 20% of
the plugin, not enough to justify splitting the other 80%. Splitting them would also open
a window where the gutter and the inline view disagree about the same file, since each
would fetch and diff independently.

## Documentation

| Doc | Covers |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | Per-buffer state model, ref handling |
| [`docs/pipeline.md`](docs/pipeline.md) | The fetch/diff pipeline (cache states, severity tiers, debounce) and branch tracking |
| [`docs/rendering.md`](docs/rendering.md) | Signs, virtual deleted lines + word highlights, line background tint, the flag→renderer dispatch |
