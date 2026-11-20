# core:git-diff

<img src="/core_git_diff.png" alt="core:git-diff in action" />

Live inline git diff. As you type, it compares the buffer against a git ref (default `HEAD`) and renders:

- gutter `+`/`~` signs, plus a boundary mark (`▁`/`▔`) for deletions
- deleted lines as virtual lines
- added/changed lines with a background tint
- word-level highlights inside changed lines

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:git-diff")
```

Requires `core:stdlib` loaded first.

| Command | Effect |
|---------|--------|
| `:toggle-git-signs [ref]` | Toggle gutter signs for the current buffer |
| `:toggle-inline-diff [ref]` | Toggle inline rendering (virtual deleted lines, word highlights, background tint) for the current buffer |

- Both take an optional git ref, e.g. `:toggle-inline-diff HEAD~2`, and Tab-complete branches and tags from the current buffer's repo.
- Giving a ref always turns that rendering on and points it at that ref. The two commands share the ref, and it sticks across a later bare toggle off and on.
- A file git doesn't track (untracked, new, or outside a repo) shows no diff.
- A commit or checkout made outside HUME shows when you switch back to the buffer or HUME regains focus.

Both are typed commands, with no keys bound by default. Each has an editor-command twin, without the ref argument, that you can bind to a key:

```scheme
(bind-key! 'normal "space i" "git-diff/toggle-inline-diff")
(bind-key! 'normal "space s" "git-diff/toggle-git-signs")
```

Configure with `#:config`:

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:git-diff"
  #:config (hash 'signs #t 'inline #f 'ref "HEAD" 'branch-style 'parens))
```

| Key | Type | Default | Effect |
|---|---|---|---|
| `'signs` | bool | `#t` | Whether gutter signs start on for each buffer |
| `'inline` | bool | `#f` | Whether inline rendering starts on for each buffer |
| `'ref` | string | `"HEAD"` | The default git ref a buffer diffs against, until overridden per-buffer via the toggle commands |
| `'branch-style` | `'parens` or `'icon` | `'parens` | How the `"steel:git-branch"` statusline element writes the branch: `(main)` or `⎇ main` |

Inline rendering's background tint and word highlights need your theme to define colors for them; HUME's bundled themes do.

The plugin also keeps a `"steel:git-branch"` statusline element fresh for the focused buffer. Add it to your own `configure-statusline!` call (see [Statusline → Custom elements](../settings.md#custom-elements)). It updates when you switch to a buffer, when you save it and when HUME regains focus, and is empty for a buffer outside any repo.

## Drawing a diff from a script

A command of your own can check and turn on the inline diff, or draw diffs of its own the way `core:git-diff` draws them:

| Command | Effect |
|---------|--------|
| `(call! "git-diff/inline-diff?" pane)` | `#t` when inline rendering is on for the buffer, `#f` otherwise |
| `(call! "git-diff/show-inline-diff" pane ref)` | Turn inline rendering on against `ref`; it never turns it off |
| `(call! "git-diff/render-diff" pane source hunks)` | Draw `hunks` in the buffer as an inline diff |
| `(call! "git-diff/release-diff" pane source)` | Clear what `source` drew and bring the buffer's own inline diff back |

- `hunks` is the `'hunks` list that `diff-buffer-lines` or `buffer-revision-diff` returns (see [Diffing](../plugin-api.md#diffing)).
- `source` is a name of your plugin's own, so its drawing never mixes with `core:git-diff`'s.

Each `render-diff` call replaces what `source` drew for that buffer, and an empty list draws nothing. While a source draws, the buffer's own inline diff is hidden so changed lines don't show twice; the gutter signs stay.
