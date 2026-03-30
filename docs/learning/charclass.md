# CharClass: Word Boundaries and the Eol Split

## word vs WORD

Vim and Helix distinguish two kinds of "word":

- `word` (lowercase): a run of alphanumeric/underscore characters, a run of
  punctuation, or a run of whitespace. Any category change is a boundary.
- `WORD` (uppercase): a run of any non-whitespace characters. Only a
  whitespace boundary counts.

In `helpers.rs`, this is captured by `CharClass` and two boundary predicates:

```rust
pub enum CharClass { Word, Punctuation, Space, Eol }

// word: any class change is a boundary
fn is_word_boundary(a: CharClass, b: CharClass) -> bool { a != b }

// WORD: treat Punctuation as Word — only whitespace/Eol changes count
fn is_WORD_boundary(a: CharClass, b: CharClass) -> bool {
    let merge = |c| if c == Punctuation { Word } else { c };
    merge(a) != merge(b)
}
```

Text object and motion implementations take the boundary predicate as a
parameter (`impl Fn(CharClass, CharClass) -> bool`), so `inner_word_impl`
serves both `iw` and `iW` without duplication.

## Why Eol is its own class

`\n` could be treated as `Space` — after all, it is whitespace. But if it were,
`w` (move to next word start) would skip over newlines the same way it skips
spaces, meaning it could jump two logical lines in one keypress.

Helix stops `w` at newlines: moving forward from the last word on a line lands
on the `\n`, not on the first word of the next line. Making `Eol` a distinct
class in `CharClass` is what enforces this — the `\n` is always a class
boundary, so word-forward stops there.
