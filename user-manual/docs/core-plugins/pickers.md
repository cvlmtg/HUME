# core:pickers

<img src="/core_pickers.png" alt="core:pickers in action with a custom picker" />

Fuzzy file, buffer, and modified-file finders:

- `z f` opens a file picker. Inside a git repository it lists the repository's files; elsewhere it needs `fd`.
- `z b` opens a buffer switcher.
- `z m` opens a picker over files with staged or unstaged git changes.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:pickers")
```

Requires `core:stdlib` loaded first.

See [Fuzzy pickers](../pickers.md) for the file sources, keys, buffer display, modified-files details, and options.
