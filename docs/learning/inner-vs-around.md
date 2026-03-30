# Inner vs Around: The Text Object Convention

Every text object in HUME comes in two flavours, following Vim/Helix convention:

- **`inner` (`i` prefix)**: the content *without* the delimiters. `i(` selects
  the text inside parentheses; `iw` selects the word without surrounding space.
- **`around` (`a` prefix)**: the content *including* the delimiters. `a(`
  selects the parentheses and their contents; `aw` selects the word plus one
  adjacent whitespace run.

In code, `inner_bracket` and `around_bracket` share `find_bracket_pair` to
locate the pair, then diverge on what range to return:

```rust
fn inner_bracket(buf, pos, open, close) -> Option<(usize, usize)> {
    let (open_pos, close_pos) = find_bracket_pair(buf, pos, open, close)?;
    Some((open_pos + 1, close_pos - 1))  // exclude the brackets
}

fn around_bracket(buf, pos, open, close) -> Option<(usize, usize)> {
    find_bracket_pair(buf, pos, open, close)  // include the brackets
}
```

The around-word rule is more nuanced: prefer including trailing whitespace
(so deleting `aw` leaves no double-space), fall back to leading whitespace if
there is no trailing space. This matches Vim's long-established behaviour.
