# core:buffer-words

Offers every identifier already in the buffer as an Insert-mode completion, in any buffer, including a scratch buffer or a `.txt` file where `core:lsp` has no server to ask.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:buffer-words")
```

Requires `core:stdlib` loaded first.

When both plugins are loaded, its candidates rank alongside `core:lsp`'s in the same menu, with `core:lsp`'s answers preferred on a tie.

A word written capitalized (`Apply`) is also offered lowercase (`apply`), and the reverse, following the case you type. A word capitalized only because it started a sentence is still found when you type it lowercase mid-sentence. A word with an inner capital (`HashMap`) or in all caps (`MAX_LEN`) is offered only as written.

Configure with `#:config`:

```scheme
(load-plugin! "core:buffer-words"
  #:config (hash 'match 'string 'chars 8000))
```

| Key | Type | Default | Effect |
|---|---|---|---|
| `'match` | `'string` \| `'fuzzy` | `'string` | `'string` narrows by prefix as you type; `'fuzzy` scores subsequence matches like `core:lsp`'s own candidates |
| `'chars` | integer (≥ 1) | `8000` | How much of a large buffer each indexing step reads around the cursor. Lower it to shorten the pauses on a huge buffer; raise it to index a large buffer in fewer, longer steps |
