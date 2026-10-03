# core:git-diff

Live, VSCode-style inline git diff. It compares the buffer against a git ref (default
`HEAD`) as it is edited and renders gutter `+`/`-`/`~` signs, deleted lines as virtual
lines, added and changed lines with a background tint, and word-level highlights inside
changed lines. It also keeps a `"steel:git-branch"` statusline element fresh for the
focused buffer; add that element to your own `configure-statusline!` call to show it.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:git-diff"
  #:config (hash "signs" #t "inline" #f "ref" "HEAD"))
```

- **Depends on:** `core:stdlib`: config validation calls `stdlib/config-boolean`/
  `stdlib/config-string` at load time.
- **Activates on:** the first buffer opened, the first `:toggle-git-signs`/
  `:toggle-inline-diff` typed, or the first `git-diff/render-diff` call. Its `manifest.scm`
  has one entry for `plugin.scm` with `#:events '(on-buffer-open)`, those two typed commands
  and that command. `"signs"` defaults on (cheap, no line-shifting side
  effects); `"inline"` defaults off (it moves virtual lines into the buffer's visual
  flow).
- **Branch tracking has no config flag; placement is the switch.** It does not fetch until
  `"steel:git-branch"` appears in your `configure-statusline!` call, and it starts when it
  does.
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-git-diff)
  for value semantics and key-binding examples. No default key bindings ship with this
  plugin.

## Commands

| Command | Effect |
|---|---|
| `:toggle-git-signs [ref]` | Toggle gutter signs for the current buffer. Tab-completes branches/tags |
| `:toggle-inline-diff [ref]` | Toggle inline rendering (virtual deleted lines, word highlights, background tint) for the current buffer. Tab-completes branches/tags |
| `(call! "git-diff/render-diff" pane source hunks)` | Draw `hunks` inline under the decoration source `source`. Another plugin's entry point to the renderer, e.g. `core:undotree`'s revision diff. See [`docs/rendering.md`](docs/rendering.md#rendering-another-plugins-hunks) |

## Documentation

| Doc | Covers |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | What the plugin builds on, file layout, per-buffer state, ref handling |
| [`docs/pipeline.md`](docs/pipeline.md) | The fetch/diff pipeline and its data-flow diagram (cache states, severity tiers, debounce) and branch tracking |
| [`docs/rendering.md`](docs/rendering.md) | Signs, virtual deleted lines + word highlights, line background tint, the flag→renderer dispatch |
