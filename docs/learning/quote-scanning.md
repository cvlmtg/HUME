# Quote Scanning: Parity Instead of Depth

## Why quotes are different from brackets

Brackets use distinct open and close characters (`(` vs `)`), which allows a
depth-tracking scan: increment depth on open, decrement on close, stop at
depth zero. This correctly handles nesting.

Quotes use the *same* character for open and close (`"..."`, `'...'`). There
is no depth to track, and nesting is ambiguous anyway (most languages don't
allow nested same-character quotes). A different algorithm is needed.

## The parity scan

`find_quote_pair` scans the current line and uses parity to assign roles:

```
Position:  0   1   2   3   4   5   6   7
           "   h   e   l   l   o   "   !
           ↑                   ↑
         odd (open)          even (close)
```

Every quote character found on the line alternates between "opening" (odd
occurrence) and "closing" (even occurrence). When a complete pair is found,
the algorithm checks whether `pos` falls inside it (`open_pos <= pos <= close_pos`).

```rust
let mut open: Option<usize> = None;
for i in line_start..line_end {
    if buf.char_at(i) == Some(quote) {
        match open {
            None        => open = Some(i),            // odd → opening
            Some(op)    => {                           // even → closing
                if op <= pos && pos <= i {
                    return Some((op, i));
                }
                open = None;                           // reset for next pair
            }
        }
    }
}
```

A cursor ON a quote character is handled by the same parity logic: whether
that quote is the opener or closer depends on how many quotes precede it on
the line. The scan resolves this automatically.

## Why brackets can span lines but quotes cannot

Bracket text objects walk the entire buffer and track depth — they work
correctly across lines because `(` and `)` are asymmetric. The scanner can
always tell which direction it is going.

The parity scan cannot be extended to multiple lines with the same correctness
guarantee. Consider a file where line 1 has an unmatched `"` (a string that
continues on the next line, or a comment, or a stray character). The parity
count is now off by one for every line that follows — every pair assignment
downstream is inverted, and the text object selects the wrong region or fails
silently.

This applies to all three quote characters, including backticks. The backtick
text object silently fails on multiline inline code spans in Markdown
(CommonMark allows `` `foo\nbar` ``).

## The planned fix: tree-sitter when available, parity fallback

Tree-sitter builds a syntax tree that distinguishes string literals from other
uses of quote characters. When a tree-sitter grammar is loaded, quote text
objects can query the tree for the enclosing string node — getting multiline
correctness and proper handling of escaped quotes for free.

When no grammar is loaded (plain text, unsupported language), the line-bounded
parity scan remains the fallback. It is fast and correct for the common case
of same-line pairs; the limitation is documented and visible rather than
silently wrong.
