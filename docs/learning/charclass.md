# CharClass: Word Boundaries and the Eol Split

## word vs WORD

Vim and Helix distinguish two kinds of "word":

- `word` (lowercase, `w`/`b`/`iw`): a run of alphanumeric characters
  (letters, digits, underscore), a run of punctuation, or a run of whitespace.
  Any class change is a boundary.
- `WORD` (uppercase, `W`/`B`/`iW`): a run of any non-whitespace characters.
  Only a whitespace boundary counts.

HUME classifies every character into one of four classes:

| Class | Members | Example chars |
|-------|---------|---------------|
| Word | alphanumeric + underscore | `a`–`z`, `A`–`Z`, `0`–`9`, `_` |
| Punctuation | printable non-word non-space | `.`, `(`, `#`, `-`, `"` |
| Space | horizontal whitespace | space, tab |
| Eol | end-of-line | `\n` |

For `word` boundaries, any adjacent class change is a boundary — `Word`→`Punctuation`,
`Punctuation`→`Space`, and so on. For `WORD` boundaries, Punctuation is
treated as Word — the only boundaries that count are `(Word or Punctuation)`
↔ `(Space or Eol)`.

The same word-finding logic powers both `w` and `W` (and `iw` and `iW`) — the
boundary rule is passed in as a parameter so there is no duplicated code.

## Why Eol is its own class

`\n` could be treated as `Space` — it is whitespace, after all. But if it
were, `w` (move to next word start) would skip over newlines the same way it
skips spaces. A cursor at the end of one line would jump directly to the first
word of the next line — or further if successive lines are blank. HUME follows
Helix's behaviour here: `w` stops at the newline, not past it.

Making `Eol` a distinct class is what enforces this — a newline is always a
class change from whatever precedes it, so word-forward always pauses there.
The [word motions doc](word-motions.md) describes how a second step then crosses
the newline when the user expects to land on the next word.

Treating `Eol` as distinct also gives text objects cleaner boundaries: an
`iw` selection on the last word of a line doesn't accidentally absorb the
newline, and a `Space` skip always halts at line ends.
