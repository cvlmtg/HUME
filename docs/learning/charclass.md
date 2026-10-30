# Character Classes: Word Boundaries and the Eol Split

## word vs WORD

HUME, like Helix and Kakoune, distinguishes two kinds of "word":

- `word` (lowercase, `w`/`b`/`miw`): a run of alphanumeric characters
  (letters, digits, underscore), a run of punctuation, or a run of
  whitespace. Any class change is a boundary.
- `WORD` (uppercase, `W`/`B`/`miW`): a run of any non-whitespace characters.
  Only a whitespace boundary counts.

HUME classifies every character into one of four classes. A character
cluster (a letter with its accent, an emoji sequence) is classified by its
first character, so a cluster never changes class partway through:

| Class | Members | Example chars |
|-------|---------|---------------|
| Word | alphanumeric (Unicode) + underscore | `a`–`z`, `A`–`Z`, `0`–`9`, `_`, `é`, `文` |
| Punctuation | printable non-word non-space | `.`, `(`, `#`, `-`, `"` |
| Space | inter-word whitespace | space, tab, no-break and ideographic spaces |
| Eol | end-of-line | `\n` |

The Word class follows Unicode's notion of "alphanumeric", so accented
letters like `é` and Han characters like `文` classify as Word just as `a`
does. Space covers the characters that act as spacing, including the
no-break and ideographic spaces. Every other unusual whitespace character
(form feed, thin space, …) is classed as Punctuation, so the cursor stops on
it rather than skipping over something invisible.

For `word` boundaries, any adjacent class change is a boundary:
`Word`→`Punctuation`, `Punctuation`→`Space`, and so on. For `WORD`
boundaries, the only merge is Word+Punctuation. Every other class change
still counts, including `Space`↔`Eol`, which is how a whitespace scan still
notices line ends.

The same word-finding logic powers both `w` and `W` (and the inner-word text
objects). The boundary rule is passed in as a parameter, so there is no
duplicated code.

## Word is a policy, not a fact about the text

The built-in Word/Punctuation split above is a default, not a universal
truth: whether a hyphen belongs *inside* a word depends on the language you
are editing. In CSS, `foo-bar` is one identifier; in most other languages
it is a subtraction. HUME lets a buffer widen its own Word class with extra
characters (typically configured per language), so the same four-class
model still applies and only the boundary a hyphen draws changes.

## Why Eol is its own class

`\n` could be treated as `Space`, since it is whitespace. But collapsing the
two would erase information the layers above want. With `Eol` distinct, the
low-level boundary scan always pauses at a newline, and the *motion* layer
decides what happens there. For `w` that decision is to cross: it takes a
second internal step over a non-final newline so you land on the next line's
first word (see [Word Motions](word-motions.md)). At the buffer's end there
is nothing beyond the trailing newline, so `w` on the last word becomes a
clean no-op instead of parking the cursor on an invisible character.

Treating `Eol` as distinct also gives text objects cleaner boundaries: a
`miw` selection on the last word of a line does not absorb the newline, a
`Space` skip always halts at line ends, and the around-word whitespace rule
can tell indentation (whitespace touching a line start) from inter-word
spacing.

---

*See also: [Word Motions](word-motions.md) for the motions built on these
classes; [Inner vs Around](inner-vs-around.md) for the around-word
whitespace rule.*
