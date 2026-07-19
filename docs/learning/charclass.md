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
| Word | alphanumeric (Unicode) + underscore | `a`–`z`, `A`–`Z`, `0`–`9`, `_`, `é`, `文` |
| Punctuation | printable non-word non-space | `.`, `(`, `#`, `-`, `"` |
| Space | inter-word whitespace | space, tab, no-break and ideographic spaces |
| Eol | end-of-line | `\n` |

The Word class follows Unicode's notion of "alphanumeric", so accented letters
like `é` and Han characters like `文` classify as Word just as `a` does. Space
covers the characters that genuinely act as spacing — including the two
invisible Unicode spaces — while every other exotic whitespace character
(form feed, thin space, …) is deliberately classed as Punctuation, so the
cursor stops on it rather than silently skipping something invisible.

For `word` boundaries, any adjacent class change is a boundary — `Word`→`Punctuation`,
`Punctuation`→`Space`, and so on. For `WORD` boundaries, the only merge is
Word+Punctuation — every other class change still counts, including
`Space`↔`Eol`, which is how a whitespace scan still notices line ends.

The same word-finding logic powers both `w` and `W` (and the inner-word text
objects) — the boundary rule is passed in as a parameter so there is no
duplicated code.

## Why Eol is its own class

`\n` could be treated as `Space` — it is whitespace, after all. But
collapsing the two would erase information the layers above want. With `Eol`
distinct, the low-level boundary scan always pauses at a newline, and the
*motion* layer decides deliberately what happens there. For `w` that decision
is to cross: it takes a second internal step over a non-final newline so you
land on the next line's first word (see the
[word motions doc](word-motions.md)). At the buffer's end there is nothing
beyond the trailing newline, so `w` on the last word becomes a clean no-op
instead of parking the cursor on an invisible character.

Treating `Eol` as distinct also gives text objects cleaner boundaries: an
`iw` selection on the last word of a line doesn't accidentally absorb the
newline, a `Space` skip always halts at line ends, and the around-word
whitespace rule can tell indentation (whitespace touching a line start) from
inter-word spacing.
