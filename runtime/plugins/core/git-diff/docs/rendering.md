# core:git-diff — Rendering (`render.scm`)

Every function in this file is a pure `hunks → decoration records` view over the one hunk
shape `state.scm` stores (see [One hunk store](architecture.md#one-hunk-store)), and ends in
one setter call. `render-inline!` makes two. The signs and the plugin's own inline rendering
share a feature-scoped source name, `"git-diff"`, not the `core:git-diff` plugin id,
matching `core:lsp`'s own decoration sources (`"lsp-diagnostics"`, `"lsp-inlay-hints"`).
The inline renderers take the source name as a parameter, so another plugin's hunks draw
under that plugin's own source (see [Rendering another plugin's hunks](#rendering-another-plugins-hunks)).

## Flag → renderer dispatch

`render-for!` is the one place a flag key (`"signs?"` or `"inline?"`) maps to its
renderers: `"signs?"` calls `render-signs!`, and any other key calls `render-diff!` under
the `"git-diff"` source. Every caller that paints or clears a rendering goes through it:
`diff.scm`'s `apply-hunks!` on a live refresh and `plugin.scm`'s toggle command on enable
and disable.

## Scope naming

| Scope | Used for |
|---|---|
| `diff.plus`, `diff.minus`, `diff.delta` | Helix's own names, read as `fg` for the gutter marker's color |
| `diff.plus.line`, `diff.minus.line`, `diff.delta.line` | HUME's row and virtual-line tint |
| `diff.plus.word`, `diff.minus.word` | Word-level highlights inside a changed line pair |

The bare names are Helix's own, so HUME's tint, which a Helix theme does not define, uses
the `.line` suffix.

## Signs

Signs render at VSCode/gitsigns density: one per changed line, not one per hunk, so a
20-line paste shows 20 `+` marks.

### Sign priority

The gutter slot comes from registering `"git-diff"` as a sign source for the buffer,
idempotently, right before every `set-signs!` call, and not from anything in the call
itself. A buffer whose signs never render, because it is untracked or `"signs?"` was never
turned on, never reserves the slot. The source's priority is `0`, ranked against every
other source registered for the buffer. `core:lsp`'s `"lsp-diagnostics"` is `10`; see the
`register-sign-source!` entry in [`docs/LSP.md`](../../../../../docs/LSP.md). Priority `0`
puts git-diff last, so its column is the first to fall off the signcolumn auto-size cap
when several higher-priority sources share the buffer.

### Sign kinds

| Hunk shape | Text | Scope | Line |
|---|---|---|---|
| Pure deletion (`new-count` 0) with `new-start` above 0 | `▁` (bottom-aligned) | `diff.minus` | `(- new-start 1)`, the line above the gap |
| Pure deletion at the very start (`new-count` 0, `new-start` 0) | `▔` (top-aligned) | `diff.minus` | `0` |
| Pure addition (`old-count` 0) | `+` | `diff.plus` | Every new-side line |
| Anything else | `~` | `diff.delta` | Every new-side line |

A pure deletion has no new-side lines to anchor on, so its sign lands on the line above the
gap (gitsigns' convention). `(- new-start 1)` also keeps a deletion at end of file from
addressing a line past the end. The bottom-aligned glyph reads as a mark on the boundary
below that line. A deletion at line 0 has no line above, so it gets the top-aligned glyph.

The `(apply append …)` that joins the per-hunk sign lists is not `flatten`: a sign entry
is itself a list, and `flatten` would tear each one apart. An empty `hunks` clears the
gutter, since `set-signs!` replaces a source's signs wholesale, so the same function clears.

## Line background tint

`render-line-bgs!` turns one hunk into `(hash 'line 'scope)` entries, one per new-side
line: `diff.plus.line` for a pure addition and `diff.delta.line` for a change. A pure
deletion adds nothing, since the inline pass's virtual lines cover the removed content and
carry `diff.minus.line` on their own scope. `set-line-backgrounds!` has no priority
argument; this plugin is the only producer of these scopes.

## Rendering another plugin's hunks

`render-diff!` is `render-inline!` followed by `render-line-bgs!`, both under a `source`
argument. The `git-diff/render-diff` command wraps it as the one entry point a plugin
outside this one reaches through `call!`, since plugins never `require` each other's
modules:

```scheme
(call! "git-diff/render-diff" pane "my-source" hunks)
```

`hunks` is a list in the shape `diff-buffer-lines` and `buffer-revision-diff` return.
Each call replaces everything that `source` drew for the buffer, so an empty list clears
it. The command is in this plugin's manifest, so calling it wakes the plugin even before
its first buffer-open event. Signs are not part of it.

## Inline: deleted lines and word highlights

`render-inline!` makes two setter calls, `set-virtual-lines!` and `set-extra-highlights!`.
One `diff-words` pass produces two decoration kinds, old-side virtual lines and new-side
highlight spans, and both come from the same call.

A pure addition (`old-count` 0) contributes nothing to this pass: there is nothing removed
to show. The line-background pass alone tints its new-side lines.

### Anchors

A hunk's removed old-side lines attach at a `(kind . line)` anchor pair: `'after
(- new-start 1)` when `new-start` is above 0, and `'before 0` for a deletion at the very
start. `'after (- new-start 1)` renders where `'before new-start` would, and stays valid
when `new-start` is the buffer's content line count, a deletion at end of file, where
`'before new-start` would address the phantom trailing line and raise.

### Virtual lines

Within a hunk, old-lines `[0, paired-count)` have a same-index new-line to word-diff
against, with `paired-count` the smaller of `old-count` and `new-count`. Any remainder gets
a plain whole-line virtual line, as does a pair whose `diff-words` call reports
`deadline-hit`. A plain virtual line passes the old line through as `'text`, with no
`'segments` key (it is omitted when empty and not set to `'()`). `set-virtual-lines!`
accepts a literal tab and expands it.

A paired line's `'segments` come from the `diff-words` hunks (`'old-start`, `'old-end`,
`'new-start`, `'new-end`, `'old-text`, `'new-text`), filtered to `'old-start < 'old-end`.
A pure insertion has nothing to underline on the old-side line, and a zero-width segment
would raise on `set-virtual-lines!`'s `start < end` check.

### Word spans from the hunk

A hunk with a `'words` key, as `buffer-revision-diff` returns, always carries its own spans, so no
`diff-words` call runs for it. `'words` is `(hash 'old spans 'new spans)`, each span
`(hash 'line 'start 'end)`, with `'line` counted from the hunk's first line on that side and
`'start`/`'end` char columns in that line. Spans are in ascending line order. Each old-side
line becomes one virtual line carrying the `'old` spans that name its line as segments. The
`'new` spans become buffer-offset highlights, from one `line->offset` call for the hunk's
first new-side line and a walk down `'new-lines` from there, like the paired-line walk
below. A hunk without `'words`, as `diff-buffer-lines` returns, takes the paired `diff-words` path described above.

### New-side spans

The new-side counterpart is `(hash 'start 'end 'scope)` spans in buffer char offsets, since
`set-extra-highlights!` addresses the whole buffer and not one line. They are filtered to
`new-start < new-end` for the same reason.

Offsets for a hunk's paired new-side lines come from one `line->offset` host call, for the
hunk's first new-side line. Each later line starts at its predecessor's offset plus its
length plus one `\n`.

One paired `(old-line . new-line)` becomes a `(virtual-line . spans)` pair from a single
`diff-words` call shared by both sides. The walk over old-lines, new-lines, offsets and the
remaining count advances with `cdr`. Steel lists are linked, so `list-ref` by index would
make it quadratic in `paired-count`.
