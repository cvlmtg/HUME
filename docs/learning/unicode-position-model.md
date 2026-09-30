# Unicode Position Model: Bytes, Chars, and Grapheme Clusters

Understanding this hierarchy is essential for HUME's architecture. Three
different units can describe a "position" in text, and choosing the wrong one
at the wrong layer causes subtle, hard-to-reproduce bugs.

## Byte offset

A byte offset is a raw index into memory. In UTF-8 (Rust's string encoding),
characters are **variable-width**: 1 to 4 bytes each.

```
"café"
 c  a  f  é
 1  1  1  2   ← bytes per character
```

| Char | Bytes   | Byte offsets |
|------|---------|-------------|
| `c`  | `63`    | 0 |
| `a`  | `61`    | 1 |
| `f`  | `66`    | 2 |
| `é`  | `C3 A9` | 3, 4 |

`é` occupies bytes 3 **and** 4. Byte offset 4 points into the **middle** of a
character — it is not a valid character boundary. This is why `s[3..4]` on
`"café"` panics in Rust: slicing through a multi-byte character is undefined.

Byte offsets are used internally by Rust's `str` and by the rope library HUME
stores buffer text in, but they are **never used for buffer positions** in
HUME. They surface only at narrow interoperability seams, where an external
system speaks other units: regular-expression matchers and tree-sitter speak
bytes, and language servers speak line-plus-column positions whose column
counts UTF-8 bytes or UTF-16 code units — a fourth unit that exists only at
that wire boundary and is converted to char offsets on arrival. Outside those
seams, byte offsets are an implementation detail.

## Char offset

A char offset counts **Unicode scalar values** (Rust's `char` type),
regardless of how many bytes each one takes.

```
"café"
 c  a  f  é
 0  1  2  3   ← char offsets
```

`é` is a single `char` at offset 3 — no partial-character hazard. This is the
rope library's native addressing unit, and it is what HUME's buffer,
selections, and selection sets use for all positions.

Char offsets make sense for an editor at the storage layer:
- `insert(at, text)` and `remove(from, to)` can be expressed cleanly.
- The anchor and head of a selection are meaningful without knowing the
  encoding of any particular character.

## Grapheme cluster

A char offset solves the byte problem, but there is a level above it:
**grapheme clusters** — what a user perceives as a single indivisible
character, which may be composed of multiple Unicode scalar values.

```
"é"  can be:
  U+00E9             → 1 char  (precomposed NFC form)
  U+0065 + U+0301    → 2 chars (base 'e' + combining acute accent)

"👨‍👩‍👧"              → 1 visible character, but 5 chars
                       (joined with zero-width joiners U+200D)
```

Pressing the right-arrow key on `"👨‍👩‍👧"` should advance the cursor past the
entire emoji in one step, not stop five times. This is the job of the
grapheme layer: given the buffer, it returns the next/previous **valid
grapheme boundary** as a char offset.

## Architectural rule

| Unit | Granularity | Role in HUME |
|------|-------------|--------------|
| Byte offset | Raw memory | Internal to the text storage library — never exposed |
| Char offset | Unicode scalar value (`char`) | Storage, and positions from outside (a language server, a regex match) |
| Cluster position | User-perceived character | Selections, cursor movement, motions, text objects |

A char offset can point anywhere, including between a letter and its accent.
A **cluster position** cannot: it is a separate type, and only the grapheme
layer can make one, by walking the text's clusters. Everything that decides
where a selection starts or ends takes and returns cluster positions, so a
position inside a cluster is not a bug to look for; it cannot be written down.
A char offset from outside becomes a cluster position in one step that snaps
it onto the cluster holding it.

A run of whole clusters is its own type too. It knows its first cluster, its
last cluster and where it ends (the start of the next one), so there is no
question of whether "the end" means the last thing covered or the first thing
not covered.

## Selections address clusters

A selection's anchor and head are cluster positions: a cursor on `é` (`e` +
combining accent) sits on the cluster, which covers both chars, because the
cluster is the smallest thing a cursor can be on.

```
"café" with a decomposed é
 c  a  f  e  ◌́
 0  1  2  3  4
              cursor on é: one cluster, starting at offset 3, covering 3 and 4
```

A selection answers two different questions:

- the **head** is where the cursor is drawn and where the next motion starts;
- the **extent** is the clusters it covers.

A command that deletes, yanks, or replaces a selection needs the extent, and
it asks the selection for it rather than working it out from the head. Doing
the arithmetic in each command is a convention, and conventions get
forgotten; with the extent read from the model, `d` and `y` cannot leave half
a character behind.

A selection is only meaningful for the text it was made on. Each set of
selections remembers which version of the text it belongs to, and reading it
means pairing it with that text: pairing it with any other version is
reported as a bug.

An edit is the one place positions for a text that does not exist yet are
needed: the text it leaves behind can re-form clusters (deleting the base
letter leaves its accent joining the previous character; deleting one half of
a flag re-pairs the rest). So an edit describes its resulting selections
relative to what it changed, and they are resolved onto the clusters of the
new text in one place. Every position the editor keeps between commands —
other windows' selections, the jump history, a search prompt's starting
point — is carried through the same change at the same moment, so none of
them is ever read against a text it does not belong to.

The grapheme layer also answers a related family of questions for vertical and
horizontal layout: the display column of a position once tab stops are
expanded, the position that lands on a given display column, the number of
graphemes in a range. The same abstraction that gives you "next grapheme
boundary" gives you tab-aware column arithmetic for free.
