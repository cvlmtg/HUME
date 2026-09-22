# core:buffer-words

Offers every identifier already in the buffer as an Insert-mode completion —
works in any buffer, including a scratch buffer or a `.txt` file where
`core:lsp` has no server to ask.

## Usage

```scheme
(declare-plugin "core:stdlib")
(load-plugin "core:buffer-words")
```

Loads eagerly: nothing but `Ctrl-Space` (or a completion source's own
trigger char, if one applies) can ever invoke the completion source it
registers, so a lazy `declare-plugin` would have no other trigger to
activate it — `core:pickers`/`core:vim-keybind`/`core:classic-paste` are
the same shape, for the same reason. Requires `core:stdlib` declared or
loaded first — config validation calls `stdlib/config-enum`/
`stdlib/config-integer` via `call!` while this plugin's body evaluates.

## Configuration

```scheme
(load-plugin "core:buffer-words"
  #:config (hash "match" 'string "lines" 200))
```

| Key | Default | Meaning |
|---|---|---|
| `"match"` | `'string` | `'string` — a case-sensitive prefix gate, the vim `i_CTRL-N` feel; `'fuzzy` — subsequence-scored like `core:lsp`'s own candidates, so the two compete on score rather than priority |
| `"lines"` | `200` | Lines fetched and scanned per side (before/after the cursor) on each background indexing tick. Lower trims the pause a keystroke can add near a huge buffer; raising it finishes indexing a large buffer in fewer ticks |

## How it works

### Cursor-outward, line-windowed indexing

Vim's `i_CTRL-N` rescans the buffer on every invocation, synchronously, in
C — affordable only because it's native. This plugin can't have that: the
scan is interpreted Steel. So it keeps a per-buffer cache instead, rebuilt
on `on-buffer-open` and (debounced 150ms) `on-text-changed`, the same shape
`core:git-diff` uses for its own per-buffer state.

The rebuild walks outward from the cursor's line in both directions, one
bounded batch of lines (`"lines"`) per direction per tick, fetched with
`(buffer-lines bid #:start #:end)` and scanned to completion in the same
tick — a word can't span two lines, so there's nothing to carry across a
tick boundary. Each tick yields back to the editor loop via `(after 0 …)`
before the next runs. That buys three things: nothing blocks, however large
the buffer; the words nearest the cursor — the ones most likely to matter
right now — are indexed first; and an edit that interrupts a walk
(cancelling it, restarting from the new cursor position) has already
covered the region most likely to have changed. `Ctrl-Space` never scans —
it only ever reads whatever the background walk has indexed so far.

Worth being precise about what this buys and what it doesn't: starting at
the cursor *reorders* the work, it doesn't reduce it. `on-text-changed`
hands the hook only the buffer id, no edit range, so every refresh is a
full rescan of the buffer — the same cost `core:git-diff` already pays on
every debounced edit. A version of `on-text-changed` that carried a change
range would let a refresh touch only what changed; that's a real, separate
change to the editor's event system, not something a plugin can express on
its own, and isn't worth building until a second real consumer motivates it
the way this plugin motivated cursor-outward indexing itself.

The walk fetches by line, not by re-slicing a plain string at a char
offset: finding an arbitrary offset in a UTF-8 string costs time
proportional to that offset, on every call, so a chunked walk built that
way would cost *more* total work than one unchunked pass, growing
quadratically with buffer size. `(buffer-lines bid #:start #:end)` doesn't
have that problem — it seeks into the buffer's rope by line index, not by
scanning from the start — so fetching a growing window of lines is genuinely
cheap however far into the buffer that window is. No giant in-memory list of
the whole buffer is ever held: each tick converts only the handful of lines
it is about to scan, so memory use during a walk is bounded by `"lines"`,
not by buffer size.

One line is always read and scanned whole, uncapped — a pathologically huge
single line (a minified file, say) costs one tick proportional to its own
length rather than being split across several. Accepted as out of scope; a
`max-word-len` option to cap word length within a line is a plausible future
addition if it ever matters in practice.

The split at the cursor is line-grained: the forward side scans the
cursor's own line from its start, and the backward side starts at the line
strictly before it — so words earlier on the cursor's own line are indexed
together with the rest of that line on the first tick, rather than
prioritized separately from it. This is a minor reordering of *when* a word
is found relative to the old char-offset split, never a change to whether
it's found.

The anchor line comes from `(current-line-number)`, which takes no `bid`
and always answers for the focused buffer — there is no way to read
another buffer's cursor position at all. A background buffer (never
focused, or edited by a script or an LSP `applyEdit`) walks from the top
instead, since it has no meaningful cursor to anchor to.

Each tick re-reads the buffer's entry fresh from the shared table rather
than trusting a binding captured before the previous tick's update —
Steel's hashes are persistent, so a stale binding still points at the
pre-update snapshot. A tick silently no-ops if the entry is gone entirely:
closing a buffer cancels its pending timer, but a tick already queued past
that cancellation's reach needs this guard too. The same cancel-before-
replace guard runs when `:reload-config` replays `on-buffer-open` on an
already-open buffer — without it, a walk already in flight would keep
running against the entry that replay just replaced.

### Double-buffered cache

Each buffer keeps two sets: the last *complete* index (what `Ctrl-Space`
reads) and the walk in progress. The in-progress set is promoted once the
walk finishes; until then, the complete set keeps answering triggers
untouched. A deleted word can therefore linger for up to one refresh cycle
— the same bounded staleness the completion framework already accepts
elsewhere ("the old answer stays ranked until the new one lands, so the
menu never blinks empty"). Before the first walk ever completes there is no
complete index yet, so a trigger falls back to the in-progress set instead
— otherwise the very first `Ctrl-Space` after a buffer opens would show
nothing.

### Matching

The completion framework re-filters this source's items against whatever's
typed on every keystroke, without re-invoking the source
(`#:match 'string`'s prefix gate, or `#:match 'fuzzy`'s subsequence score —
both run in Rust). So this plugin emits its *whole* cached set on every
trigger rather than pre-filtering by what's typed so far in Steel: a
narrower Steel-side prefix filter would be pinned to whatever was typed at
trigger time, and Backspace could never widen the list back out, since the
source itself isn't asked again. The cached set never includes the exact
word being typed either — the scan collects it like any other word, but
offering it back would be a no-op to accept, so it's filtered out at read
time.

### Non-ASCII words

Steel has no Unicode character-category table (no `char-alphabetic?`, no
regex), so "is this character part of a word" is approximated: ASCII
letters/digits/`_`, a buffer's configured `word-chars`, or any codepoint
≥ 128 that isn't whitespace. The last clause is deliberately loose — without
it, a non-ASCII identifier like `café` would split into two candidates and
never match what the editor computes as the prefix under the cursor.
The cost is that non-ASCII *punctuation* reads as a word character too, so
`foo—bar` can surface as one candidate where a `w` motion would stop at the
dash. Noise, never a missing candidate.
