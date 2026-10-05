# core:git-diff — Rendering (`render.scm`)

Every function in this file is a pure `hunks → decoration records` view over the one hunk
shape `diff-buffer-lines` returns (see [One hunk set](architecture.md#one-hunk-set)), and ends in
one setter call. `render-inline!` makes two. The signs and the plugin's own inline rendering
share a feature-scoped source name, `"git-diff"`, not the `core:git-diff` plugin id,
matching `core:lsp`'s own decoration sources (`"lsp-diagnostics"`, `"lsp-inlay-hints"`).
The inline renderers take the source name as a parameter, so another plugin's hunks draw
under that plugin's own source (see [Rendering another plugin's hunks](#rendering-another-plugins-hunks)).

## Flag → renderer dispatch

`diff.scm`'s `apply-hunks!` and `hide-renderings!` are the only places a flag key
(`"signs?"` or `"inline?"`) is paired with its renderer: `"signs?"` with `render-signs!`, and
`"inline?"` with `render-diff!` under
the `"git-diff"` source, with no hunks while another plugin's source covers the buffer (see
[Rendering another plugin's hunks](#rendering-another-plugins-hunks)). Every caller that
paints or clears a rendering goes through one of them: `apply-hunks!` on a live refresh,
`hide-renderings!` for the toggle command on disable and the `render-diff` command when it
adds a cover.

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
Each call replaces everything that `source` drew for the buffer, so an empty list draws
nothing. Signs are not part of it.

Two inline diffs of the same lines would draw every changed line twice, so a drawing from
another source covers the buffer's own inline rendering. The command adds `source` to the
entry's `"covered-by"` list, and the inline rendering draws the `"git-diff"` source with no
hunks while that list is not empty. Every refresh and toggle goes through those two, so
an edit, such as the one a caller's own revision jump makes, cannot paint the own diff back
over the caller's. The gutter signs stay. The caller hands the buffer back with
`git-diff/release-diff`, which clears what `source` drew, removes it from the list and
refreshes at once, so the own diff comes back for the buffer's current text:

```scheme
(call! "git-diff/release-diff" pane "my-source")
```

Covering follows the drawing and releasing is its own call, so an empty list stays covered.
A caller with nothing to show for one state, like `core:undotree` on the root revision,
keeps the own diff hidden instead of showing a diff against git that reads as its own. A
caller passing `"git-diff"` as `source` raises, since it would cover itself.

Both commands are in this plugin's manifest, so calling either wakes the plugin even before
its first buffer-open event.

## Inline: deleted lines and word highlights

`render-inline!` makes two setter calls, `set-virtual-lines!` and `set-extra-highlights!`.
A hunk's `'words` produce two decoration kinds, old-side virtual lines and new-side
highlight spans.

A pure addition (`old-count` 0) contributes nothing to this pass: there is nothing removed
to show. The line-background pass alone tints its new-side lines.

### Anchors

A hunk's removed old-side lines attach at a `(kind . line)` anchor pair: `'after
(- new-start 1)` when `new-start` is above 0, and `'before 0` for a deletion at the very
start. `'after (- new-start 1)` renders where `'before new-start` would, and stays valid
when `new-start` is the buffer's content line count, a deletion at end of file, where
`'before new-start` would address the phantom trailing line and raise.

### Word spans

Every hunk carries `'words`, `(hash 'old spans 'new spans)`, each span
`(hash 'line 'start 'end)`, with `'line` counted from the hunk's first line on that side
and `'start`/`'end` char columns in that line. Spans are in ascending line order. For a
hunk from a text diff, the Rust side word-diffs the whole old side against the whole new
side, so a span marks the edit itself whatever the line counts are. Hunks from
`buffer-revision-diff` carry the columns the edit changed.

Each old-side line becomes one virtual line, passing the old line through as `'text` with
the `'old` spans that name its line as `'segments` (the key is omitted when empty, not set
to `'()`). `set-virtual-lines!` accepts a literal tab and expands it. The engine wraps each
virtual line under the pane's wrap mode, as it does the buffer line it sits beside, so a
long old-side line occupies as many rows as its new-side twin.

The `'new` spans become `(hash 'start 'end 'scope)` highlights in buffer char offsets,
since `set-extra-highlights!` addresses the whole buffer and not one line. Offsets come
from one `line->offset` host call for the hunk's first new-side line and a walk down
`'new-lines` from there: each later line starts at its predecessor's offset plus its
length plus one `\n`.
