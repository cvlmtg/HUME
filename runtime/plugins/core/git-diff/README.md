# core:git-diff

Live, VSCode-style inline git diff. It compares the buffer against a git ref (default
`HEAD`) as it is edited and renders gutter `+`/`~` signs with a `▁`/`▔` boundary mark for
a deletion, deleted lines as virtual lines, added and changed lines with a background tint,
and word-level highlights inside changed lines. It also keeps a `"steel:git-branch"`
statusline element fresh for the focused buffer, fetching only while a statusline places
that element.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:git-diff"
  #:config (hash 'signs #t 'inline #f 'ref "HEAD" 'branch-style 'parens))
```

- **Depends on:** `core:stdlib`, at load time for config validation and at runtime for
  rendering.
- **Activates on:** the first `on-buffer-open`, either typed command, or any `git-diff/*`
  command below (`manifest.scm`).
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-git-diff).

## Configuration

| Key | Type | Default | Effect |
|---|---|---|---|
| `'signs` | bool | `#t` | Gutter signs start on for a newly opened buffer. On by default: signs are cheap and shift no lines |
| `'inline` | bool | `#f` | Inline rendering starts on for a newly opened buffer. Off by default: its virtual lines move the buffer's visual flow |
| `'ref` | string | `"HEAD"` | The ref a buffer diffs against until a toggle command sets a per-buffer one (see [Per-buffer ref](docs/architecture.md#per-buffer-ref)) |
| `'branch-style` | `'parens` or `'icon` | `'parens` | How `"steel:git-branch"` writes the branch: `(main)` or `⎇ main` |

## Commands

| Command | Effect |
|---|---|
| `:toggle-git-signs [ref]` | Toggle gutter signs for the current buffer. Completes refs (see [Ref completion](docs/architecture.md#ref-completion)) |
| `:toggle-inline-diff [ref]` | Toggle inline rendering (virtual deleted lines, word highlights, background tint) for the current buffer. Completes refs |
| `git-diff/toggle-git-signs` | Bindable twin of `:toggle-git-signs`, with no ref |
| `git-diff/toggle-inline-diff` | Bindable twin of `:toggle-inline-diff`, with no ref |
| `(call! "git-diff/show-inline-diff" pane ref)` | Turn inline rendering on against `ref` (never off). Same as `:toggle-inline-diff <ref>` |
| `(call! "git-diff/inline-diff?" pane)` | `#t` when inline rendering is on for the buffer |
| `(call! "git-diff/render-diff" pane source hunks)` | Draw another plugin's `hunks` inline under the decoration source `source`, hiding the buffer's own inline diff until the source is released. See [Rendering another plugin's hunks](docs/rendering.md#rendering-another-plugins-hunks) |
| `(call! "git-diff/release-diff" pane source)` | Clear what `source` drew and show the buffer's own inline diff again |

## Documentation

| Doc | Covers |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | What the plugin builds on, file layout, per-buffer state, ref handling |
| [`docs/pipeline.md`](docs/pipeline.md) | The fetch/diff pipeline and its data-flow diagram (cache states, severity tiers, debounce) and branch tracking |
| [`docs/rendering.md`](docs/rendering.md) | Signs, virtual deleted lines + word highlights, line background tint, the flag→renderer dispatch |
