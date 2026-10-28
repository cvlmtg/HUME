# core:buffer-words

Offers every identifier already in the buffer as an Insert-mode completion. It works in
any buffer, including a scratch buffer or a `.txt` file where `core:lsp` has no server to
ask.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:buffer-words" #:config (hash 'match 'string 'chars 8000))
```

- **Depends on:** `core:stdlib`, at load time for config validation and at runtime for
  the scan.
- **Activates on:** `Ctrl-Space` (or a completion source's own trigger char) only. It
  has no `manifest.scm`, so it must be loaded eagerly (see the
  [core plugins index](../README.md#loading-model)).
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-buffer-words).

## Configuration

| Key | Default | Meaning |
|---|---|---|
| `'match` | `'string` | `'string`: a case-sensitive prefix gate, the vim `i_CTRL-N` feel. `'fuzzy`: subsequence-scored like `core:lsp`'s own candidates, so the two compete on score rather than priority |
| `'chars` | `8000` | Characters fetched and scanned per side (before/after the cursor) on each background indexing tick, and the most an edit's changed lines may hold for the index to be updated without a walk; an integer of at least 1. Lower trims the pause a keystroke can add near a huge buffer; raising it finishes indexing a large buffer in fewer ticks |

## How it works

Vim's `i_CTRL-N` rescans the buffer on every invocation, synchronously, in C. This plugin
scans in interpreted Steel, so it keeps a per-buffer cache instead. A background walk that
yields back to the editor between ticks builds the cache, and `Ctrl-Space` reads whatever
that walk has indexed so far. After an edit, the cache is updated from the lines the edit
changed (see [Updating after an edit](#updating-after-an-edit)); a walk runs again only
when that can't be done.

```
cursor line
     │
     ▼
  ◀── N chars ── │ ── N chars ──▶                 one tick
  ◀──── N chars ──── │ ──── N chars ────▶        next tick
        …outward in both directions until both hit a buffer edge…
```

`N` is `'chars`, rounded to whole lines.

### Cursor-outward, line-windowed indexing

A walk builds the cache on `on-buffer-open`, and again whenever a restart is needed (see
[Updating after an edit](#updating-after-an-edit)). The per-buffer entry has the same shape `core:git-diff` uses for its own state (see
the [core plugins index](../README.md#per-buffer-state)). Each walk goes outward from the
cursor's line in both directions, one batch of lines per direction per tick, sized by
the character budget below, fetched with `(buffer-lines pane #:range …)` and scanned to
completion in the same tick. A word cannot span two lines, so nothing carries across a
tick boundary.

Nothing blocks however large the buffer is, and the words nearest the cursor are indexed
first. An edit during a walk doesn't restart it: the next tick catches up with the edit
before it reads.

The walk fetches by line. `buffer-lines` seeks into the buffer's rope by line index, so
fetching a window of lines costs the same wherever the window is. Re-slicing a string at a
character offset would cost time proportional to the offset on every call. Each tick
converts only the lines it is about to scan, so memory during a walk is bounded by
`'chars`, not by buffer size. A single line is read and scanned whole, with no cap; see
[Known limitations](#known-limitations).

Each tick reschedules itself after a 16ms delay. A zero-delay timer is due the instant it
is scheduled, which pins the event loop's wake timeout at zero for the whole walk: the
loop never blocks on input and repaints an unchanged frame once per tick until the walk
finishes. The short delay lets the loop block between ticks. The walk takes proportionally
longer in wall-clock time, and nothing waits on it.

#### Character budget

A tick's cost grows with the characters it scans, so the budget counts characters. A line
count would let a tick over a file of very long lines (a markdown file with one paragraph
per line) block input several times longer than a tick over ordinary code.

Each side sizes its window before fetching anything, from the offsets where lines start
(`line->char-offset`, `char-offset->line`) and the buffer's end (`buffer-char-count`). It
takes the whole lines that fit in the budget, so a window never exceeds it, except that
each side takes at least one line.

#### Anchor line

The anchor is the cursor's line, resolved through `(buffer-panes pane)` first (see the
[core plugins index](../README.md#pane-values-vs-pane-less-values)).
`on-buffer-open`/`on-text-changed` hand the hook a pane-less value, and reading the cursor
needs a pane. A background buffer, one never focused or edited by a script or an LSP
`applyEdit`, has no cursor to anchor to and walks from the top. The split is line-grained:
the forward side scans the anchor line from its start, and the backward side starts at the
line before it.

### Updating after an edit

The entry keeps `"seen"`, the `buffer-generation` value its set and the walk's positions
match. Every walk tick and every update advance it. On an edit, that value gives
`buffer-changes-since` its starting point: everything since, as hunks. Two places use the
hunks, and both apply them with the same update (`bw/updated-set`):

- **A walk's next tick.** Before reading, a tick that finds the buffer changed updates the
  set being built and moves both of its positions through the hunks (`bw/shift-line`).
  Lines are read by number, so without the move a deletion above the forward side would
  carry unread lines past it, and an insertion above the backward side would carry unread
  lines below it; no hunk names those lines. A position under a hunk that spans it moves
  to that hunk's end (forward side) or start (backward side), since the update has already
  added the hunk's new lines. Lines read twice cost nothing in a set.
- **A finished index.** `on-text-changed` schedules a refresh, debounced 150ms per buffer.
  A debounced call sees only the last of several fires, so it ignores the hook's own hunks
  and asks for everything since `"seen"`. While a walk runs, the refresh does nothing: the
  walk's next tick catches up.

The update works from the hunks:

- **Adding.** The words of every new line, with their case twins, join the set.
- **Removing.** A word of an old line that the new lines don't contain may be gone, or may
  still occur elsewhere. `bw/present?` searches the buffer for it with
  `(buffer-search pane (word-search-pattern pane w))`, the whole-word pattern `*` uses. It
  searches the twin's all-lowercase form when the word is capitalized. An all-lowercase
  pattern ignores case, so one search answers for the word and its twin. When nothing
  matches, both leave the set. Only words already in the set are checked, so a walk that
  hasn't reached a line doesn't search for its words. Steel's `hashset-difference` returns
  the symmetric difference, so the words to remove are first intersected with the set:
  a twin the set never held, such as `ix` for `Ix` when only `ıx` added `Ix`, would
  otherwise join it.
- **Rebuilding.** When a word joins or leaves a finished index, the word list `Ctrl-Space`
  emits is rebuilt from the set. Steel's hashsets copy on change, so this costs time in
  proportion to the number of distinct words. An edit that adds and removes nothing skips
  it.

The walk restarts from the cursor instead when the update has nothing to start from or
would block for too long:

- `buffer-changes-since` returns `#f`;
- the hunks' lines hold more than `'chars` characters, one walk tick's budget;
- more than `bw/max-word-checks` (32) words need a search. A search for a word that is gone
  reads the whole buffer and blocks the editor while it does; a walk yields between ticks.

The other restarts are on `on-buffer-open` and on a global `word-chars` change.

### Staleness and cancellation

#### Generation guard

A tick closes over the generation the walk started under (`"gen"`, bumped by every
restart) and does nothing unless the entry's current generation still matches. This is
the stale-async guard from the [core plugins index](../README.md#stale-async-work).
A restart also cancels the pending tick, which stops it even once it is due, so the
generation is a second guard on a tick, not the only one.

#### Buffer close and reindex

`on-buffer-close` forgets the buffer's entry and cancels its pending tick. `on-buffer-open`
also forgets the entry before it reindexes. A debounced refresh can fire after the entry is
gone. With no entry it calls `bw/reindex!`, which creates a fresh entry only while the buffer
is still open (`buffer-live?`), so a refresh that outlives a close does not recreate state
for a closed buffer.

#### Restart and bounds

The backward window's upper bound is clamped against the live line count, the same as the
forward window's. A buffer that shrinks mid-walk (`:e!` onto a shorter file, a large undo,
an LSP `applyEdit`) would otherwise let the backward side request a range past the new
end, which `buffer-lines` raises on and does not clamp.

Every restart empties the in-progress word set along with bumping the generation: a
restart happens because the set can't be brought up to date.

### Double-buffered cache

Each buffer keeps two sets: the last complete index, which `Ctrl-Space` reads, and the
walk in progress. The in-progress set is rebuilt tick by tick. When a walk finishes it
becomes the complete index, kept both as the set that later updates change and as a plain
word list that a trigger can pass straight to `completion-emit!`. Until then the previous
complete index keeps answering triggers, so a deleted word can linger for up to one
refresh cycle. Before the first walk completes there is no complete index, and
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

While a walk is in progress, the completion source answers with `#:more #t`, which keeps the
request open, and stashes the request id, one per buffer. When the walk promotes its
in-progress set, it calls `completion-emit!` on that id with the finished list, without
`#:more`, which closes the request. A request made once the walk has finished gets its whole
answer at once, and no id is stashed for it, so a later reindex leaves that menu alone. A
stale id, from a menu that was dismissed or replaced by a later trigger, is dropped without
effect, and the plugin is not told which.

### Matching

The completion framework re-filters this source's items against what is typed on every
keystroke, without calling the source again: `#:match 'string`'s prefix gate and
`#:match 'fuzzy`'s subsequence score both run in Rust. The plugin therefore emits its whole
cached set once per trigger and does not pre-filter in Steel. The one later part is the push
described above.

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

- **Some deleted words stay offered.** A word stays while a form of it that differs only in
  case remains (`APPLY` keeps `apply`), since the search ignores case. It also stays while it
  ends a longer word that `word-chars` joins (`a$x` keeps `$x` when `$` is a word
  character), since `*`'s pattern can't check the character before such an edge. A word
  still in the buffer is never dropped.
- **A buffer-scoped `word-chars` change does not invalidate that buffer's cache.** Lines an
  edit changes are read under the new value, and every other line keeps the split the last
  walk made. Fixing it needs a Rust-side event this plugin cannot add.
- **A single line is scanned whole.** A very long line, a minified file for instance, costs
  one tick proportional to its length.
