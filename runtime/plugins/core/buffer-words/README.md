# core:buffer-words

Offers every identifier already in the buffer as an Insert-mode completion. It works in
any buffer, including a scratch buffer or a `.txt` file where `core:lsp` has no server to
ask.

## Usage

```scheme
(declare-plugin! "core:stdlib")
(load-plugin! "core:buffer-words" #:config (hash "match" 'string "lines" 100))
```

- **Depends on:** `core:stdlib`: config validation calls `stdlib/config-enum`/
  `stdlib/config-integer` at load time, and the scan calls `stdlib/split-words`.
- **Activates on:** `Ctrl-Space` (or a completion source's own trigger char) only. It
  has no `manifest.scm`, so it must be loaded eagerly (see the
  [core plugins index](../README.md#loading-model)).
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-buffer-words).

## Configuration

| Key | Default | Meaning |
|---|---|---|
| `"match"` | `'string` | `'string`: a case-sensitive prefix gate, the vim `i_CTRL-N` feel. `'fuzzy`: subsequence-scored like `core:lsp`'s own candidates, so the two compete on score rather than priority |
| `"lines"` | `100` | Lines fetched and scanned per side (before/after the cursor) on each background indexing tick; an integer of at least 1. Lower trims the pause a keystroke can add near a huge buffer; raising it finishes indexing a large buffer in fewer ticks |

## How it works

Vim's `i_CTRL-N` rescans the buffer on every invocation, synchronously, in C. This plugin
scans in interpreted Steel, so it keeps a per-buffer cache instead. A background walk that
yields back to the editor between ticks builds the cache, and `Ctrl-Space` reads whatever
that walk has indexed so far.

```
cursor line
     │
     ▼
  ◀── N lines ── │ ── N lines ──▶                 one tick
  ◀──── N lines ──── │ ──── N lines ────▶        next tick
        …outward in both directions until both hit a buffer edge…
```

`N` is `"lines"`.

### Cursor-outward, line-windowed indexing

The cache is rebuilt on `on-buffer-open` and, debounced 150ms, on `on-text-changed`, the
same shape `core:git-diff` uses for its own per-buffer state (see the
[core plugins index](../README.md#per-buffer-state)). Each rebuild walks outward from the
cursor's line in both directions, one batch of `"lines"` lines per direction per tick,
fetched with `(buffer-lines pane #:start #:end)` and scanned to completion in the same
tick. A word cannot span two lines, so nothing carries across a tick boundary.

Nothing blocks however large the buffer is, and the words nearest the cursor are indexed
first. An edit that interrupts a walk restarts it from the new cursor position, so the
region most likely to have changed is already covered.

The walk fetches by line. `buffer-lines` seeks into the buffer's rope by line index, so
fetching a window of lines costs the same wherever the window is. Re-slicing a string at a
character offset would cost time proportional to the offset on every call. Each tick
converts only the lines it is about to scan, so memory during a walk is bounded by
`"lines"`, not by buffer size. A single line is read and scanned whole, with no cap; see
[Known limitations](#known-limitations).

Each tick reschedules itself after a 16ms delay. A zero-delay timer is due the instant it
is scheduled, which pins the event loop's wake timeout at zero for the whole walk: the
loop never blocks on input and repaints an unchanged frame once per tick until the walk
finishes. The short delay lets the loop block between ticks. The walk takes proportionally
longer in wall-clock time, and nothing waits on it.

#### Anchor line

The anchor is the cursor's line, resolved through `(buffer-panes pane)` first (see the
[core plugins index](../README.md#pane-values-vs-pane-less-values)).
`on-buffer-open`/`on-text-changed` hand the hook a pane-less value, and reading the cursor
needs a pane. A background buffer, one never focused or edited by a script or an LSP
`applyEdit`, has no cursor to anchor to and walks from the top. The split is line-grained:
the forward side scans the anchor line from its start, and the backward side starts at the
line before it.

### Staleness and cancellation

#### Generation guard

A tick closes over the generation the walk started under (`"gen"`, bumped by every
restart) and does nothing unless the entry's current generation still matches. This is
the stale-async guard from the [core plugins index](../README.md#stale-async-work).
`cancel-timer!` cannot stop a tick that has already been dequeued, and a new walk can
start before that tick fires, so the entry still existing does not tell the earlier walk from
the new one. The generation does.

#### Buffer close and reindex

`on-buffer-close` forgets the buffer's entry and cancels its pending tick. `on-buffer-open`
also forgets the entry before it reindexes. A debounced reindex can fire after the entry is
gone, and `bw/reindex!` creates a fresh entry only while the buffer is still open (`buffer-live?`),
so a reindex that outlives a close does not recreate state for a buffer that no longer
exists.

#### Restart and bounds

The backward window's upper bound is clamped against the live line count, the same as the
forward window's. A buffer that shrinks mid-walk (`:e!` onto a shorter file, a large undo,
an LSP `applyEdit`) would otherwise let the backward side request a range past the new
end, which `buffer-lines` raises on and does not clamp.

Every restart empties the in-progress word set along with bumping the generation. A
cancelled walk's partial set is stale by the time a new one starts, since an edit
triggered the restart.

### Double-buffered cache

Each buffer keeps two sets: the last complete index, which `Ctrl-Space` reads, and the
walk in progress. The in-progress set is rebuilt tick by tick. When a walk finishes it is
promoted into a plain word list that a trigger can pass straight to `completion-emit!`.
Until then the complete index keeps answering triggers, so a deleted word can linger for
up to one refresh cycle. Before the first walk completes there is no complete index, and
a trigger falls back to the in-progress set's word list, so the first `Ctrl-Space` after
a buffer opens is not empty.

### Case twins

`#:match 'string`'s prefix gate is case-sensitive, so a word cached as written answers
only a prefix typed in the same case. A buffer holding `Apply` (capitalized because it
started a sentence) would offer nothing for `app` typed mid-sentence.

`bw/add-word` inserts a plain word's other-case twin alongside it. A word is plain when
everything after its first letter is already lowercase:

| Original | Twin | Rule |
|---|---|---|
| `apply` (all-lowercase) | `Apply` | Titlecase the first letter |
| `Apply` (Titlecase) | `apply` | Lowercase the first letter |
| `a` (single letter) | `A` | A one-letter word is plain |
| `HashMap`, `iPhone` (inner capital) | none | Flipping only the first letter would lose case the tail still carries |
| `MAX_LEN` (ALL-CAPS) | none | Same |
| `1st` (non-letter first) | none | There is no other case to flip to |

The case-sensitive gate then does the rest: typing `app` matches only the `apply` twin and
typing `App` matches only `Apply`, the vim `'infercase'` feel, with no sentence-boundary
detection. Nothing is rewritten. The original word and its twin sit in the index as two
independent candidates, so a mixed-case identifier keeps its own form.

A plain lowercase identifier in a code buffer also gets a capitalized twin that was never
written (`config` adds `Config`), and the reverse. `core:lsp` outranks `core:buffer-words`
on any real score once the user types, so a code buffer with an attached server rarely
shows the extra twin.

The twin is computed once per word at scan time, so the in-progress fallback (see
[Double-buffered cache](#double-buffered-cache)) already carries twins for what has been
walked. In `#:match 'fuzzy` mode the twins are inserted too, so a lowercase fuzzy query can
match both a word and its Titlecase twin.

### Pushing a finished index to an open menu

A trigger answers only the invocation it was called for, and it does not learn of later
events. The editor's own re-ranking follows the live cursor on every keystroke without help
from this plugin. The background walk finishing is different: if `Ctrl-Space` lands on a
large buffer whose walk is still partial, the open menu would stay at that partial answer.

The completion source stashes the invocation id it was last called with, one per buffer.
When a walk promotes its in-progress set, it calls `completion-emit!` on that id with the
finished list, as an unsolicited second answer. `completion-emit!` accepts it the way it
accepts an LSP source streaming a second answer for an open call: the id must still be the
latest one for this source's slot. A stale id, from a menu that was dismissed or replaced by
a later trigger, is dropped without effect, and the plugin is not told which.

### Matching

The completion framework re-filters this source's items against what is typed on every
keystroke, without calling the source again: `#:match 'string`'s prefix gate and
`#:match 'fuzzy`'s subsequence score both run in Rust. The plugin therefore emits its whole
cached set once per trigger and does not pre-filter in Steel. The one extra answer is the
push described above.

The scan includes the word under the cursor like any other word. The editor's ranking drops
any item equal to the live token before scoring, for every source. It derives that from the
live cursor on every keystroke, so typing past an excluded word (`ca` → `cat`) drops `cat`
at once and backspacing away brings it back.

### `word-chars` invalidation

`word-chars` is read on every scan tick and baked into the cached word set for as long as
that index stands. A change during a scan applies to the lines not yet scanned. A global
`:set global word-chars=…` reindexes every open buffer through the `on-option-change` hook.
A buffer-scoped `:set buffer word-chars=…` does not, because that hook is raised only by the
global write path; see [Known limitations](#known-limitations).

### Non-ASCII words

Steel has no Unicode character-category table (no `char-alphabetic?`, no regex), so the
scan hands each line to `core:stdlib`'s `stdlib/split-words`. That reads the buffer's own
`word-chars` and runs the native `split-words` builtin, which tokenizes with the same
classifier the `w`/`b` motions and text objects use (`hume-ops`'s word-motion scan
primitives, built on grapheme-cluster-safe stepping). A word this plugin offers is what a
`w` motion would select: `café` (with a combining accent) stays one candidate,
`l'élément` splits at the curly apostrophe into `l` and `élément`, and `foo—bar` (em dash)
splits into `foo` and `bar`. `word-chars` applies per buffer as it does for motions, so
`foo-bar` is one candidate wherever `word-chars` includes `-` (a CSS buffer, say) and two
words elsewhere.

## Known limitations

- **A full rescan runs on every edit.** `on-text-changed` carries no change range, so every
  refresh rescans the whole buffer, the same cost `core:git-diff` pays on every debounced
  edit. Cursor-outward indexing changes the order of the work and does not reduce it.
- **Continuous typing near the cursor can starve the far ends of a large buffer.** Every
  restart discards in-progress work, so the far ends stay unindexed while edits keep
  arriving.
- **A buffer-scoped `word-chars` change does not invalidate that buffer's cache.** Fixing it
  needs a Rust-side event this plugin cannot add.
- **A single line is scanned whole.** A very long line, a minified file for instance, costs
  one tick proportional to its length.
