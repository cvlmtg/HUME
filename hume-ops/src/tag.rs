//! Lexical `<tag>` / `</tag>` matching for `%`-style navigation ([`matching_tag`]).
//!
//! Deliberately not tree-sitter-backed: `hume-ops` cannot depend on
//! `hume-treesitter` (a lower-level crate can't reach up to a higher one),
//! so a grammar-based version would have to live as an `EditorCmd` in
//! `hume-editor`, working only in a buffer with a parsed tree for a markup
//! grammar. This scan works in any buffer, including a scratch buffer with
//! no language at all — the same trade [`crate::pair`]'s lexical bracket and
//! quote scans already make.
//!
//! Scans ASCII markup characters only, one char at a time via `chars_at`
//! (never grapheme boundaries) — the same sanctioned exception `pair.rs`
//! uses, since no grapheme cluster can be mistaken for `<`, `>`, `/`, or a
//! quote.
//!
//! Tag names are compared case-sensitively (XML/JSX semantics) — HUME has no
//! per-buffer markup-language config to decide HTML's case-insensitive
//! convention from.

use hume_editing::text::BufferText;

/// One parsed `<name…>`, `<name…/>`, or `</name>` construct.
struct Tag {
    closing: bool,
    self_closing: bool,
    name: (usize, usize), // inclusive char range
    lt_pos: usize,
    gt_pos: usize,
}

impl Tag {
    /// Whether `pos` falls anywhere inside this tag's own markup — its `<`,
    /// its `>`, the name, or an attribute — matching matchit/vim's `%`, which
    /// fires from anywhere in the tag, not just its two delimiter chars.
    fn contains(&self, pos: usize) -> bool {
        pos >= self.lt_pos && pos <= self.gt_pos
    }
}

fn same_name(text: &BufferText, a: (usize, usize), b: (usize, usize)) -> bool {
    text.slice(a.0..a.1 + 1) == text.slice(b.0..b.1 + 1)
}

/// True if `<!--` starts at `lt_pos`.
fn is_comment_start(text: &BufferText, lt_pos: usize) -> bool {
    let mut cursor = text.chars_at(lt_pos);
    "<!--"
        .chars()
        .all(|expected| cursor.next().is_some_and(|(_, ch)| ch == expected))
}

/// Position just past the terminating `-->` of the comment starting at
/// `lt_pos`, or the end of the buffer if it's never closed — an unterminated
/// comment swallows everything after it, so no tag inside it is reachable.
fn comment_end(text: &BufferText, lt_pos: usize) -> usize {
    let cursor = text.chars_at(lt_pos + 4); // past the "<!--" already confirmed by is_comment_start
    let mut dashes = 0;
    for (i, ch) in cursor {
        match ch {
            '-' => dashes += 1,
            '>' if dashes >= 2 => return i + 1,
            _ => dashes = 0,
        }
    }
    text.len_chars()
}

/// Parse the tag construct starting at `lt_pos` (must be `<`, and not the
/// start of a comment). `None` for anything that isn't a well-formed
/// `<name…>`, `<name…/>`, or `</name>` — `<!DOCTYPE`, `<?xml`, a bare `<` in
/// running text or code, or an unterminated `<`. Quote-aware: a `>` inside a
/// `"…"`/`'…'` attribute value doesn't end the tag.
fn parse_tag(text: &BufferText, lt_pos: usize) -> Option<Tag> {
    let mut cursor = text.chars_at(lt_pos + 1);
    let (mut i, mut ch) = cursor.next()?;
    let closing = ch == '/';
    if closing {
        (i, ch) = cursor.next()?;
    }
    if !(ch.is_ascii_alphabetic() || ch == '_') {
        return None;
    }
    let name_start = i;
    let mut name_end = i;
    let (mut i, mut ch) = loop {
        match cursor.next()? {
            (j, c) if c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '.' | '-') => {
                name_end = j;
            }
            next => break next,
        }
    };

    let mut last_significant = None;
    loop {
        match ch {
            '"' | '\'' => {
                let quote = ch;
                loop {
                    let (_, c) = cursor.next()?;
                    if c == quote {
                        break;
                    }
                }
                last_significant = None;
            }
            '>' => {
                return Some(Tag {
                    closing,
                    self_closing: !closing && last_significant == Some('/'),
                    name: (name_start, name_end),
                    lt_pos,
                    gt_pos: i,
                });
            }
            c if !c.is_ascii_whitespace() => last_significant = Some(c),
            _ => {}
        }
        (i, ch) = cursor.next()?;
    }
}

/// Find the next tag construct at or after `from`, skipping HTML comments
/// and any `<` that doesn't parse as a well-formed tag.
fn next_tag(text: &BufferText, from: usize) -> Option<Tag> {
    let mut cursor = text.chars_at(from);
    while let Some((i, ch)) = cursor.next() {
        if ch != '<' {
            continue;
        }
        if is_comment_start(text, i) {
            cursor = text.chars_at(comment_end(text, i));
            continue;
        }
        if let Some(tag) = parse_tag(text, i) {
            return Some(tag);
        }
    }
    None
}

/// Find the matching partner of the tag at `pos`: given the cursor anywhere
/// inside a `<name…>` or `</name>` construct (its `<`, its `>`, the name, or
/// an attribute), return the `<` of its partner. `None` when `pos` isn't
/// inside a tag's own markup, the tag is self-closing (no partner), or it's
/// never closed or opened.
///
/// One forward pass over the whole buffer, pairing opens with closes on a
/// stack. This is what makes a same-name `<` inside a quoted attribute or an
/// HTML comment safe to ignore — `next_tag` already filters those out before
/// a tag construct ever reaches the stack, so the stack only ever sees real
/// tags in document order. An unmatched open (e.g. a void element like `<br>`
/// written without a self-closing slash) is discarded the moment a later
/// close doesn't share its name, the same tolerance real-world markup needs.
pub(crate) fn matching_tag(text: &BufferText, pos: usize) -> Option<usize> {
    let mut stack: Vec<Tag> = Vec::new();
    let mut from = 0;
    while let Some(tag) = next_tag(text, from) {
        from = tag.gt_pos + 1;
        if tag.closing {
            while let Some(open) = stack.pop() {
                if !same_name(text, open.name, tag.name) {
                    continue;
                }
                if open.contains(pos) {
                    return Some(tag.lt_pos);
                }
                if tag.contains(pos) {
                    return Some(open.lt_pos);
                }
                break;
            }
        } else if !tag.self_closing {
            stack.push(tag);
        }
        // A self-closing tag has no partner: it's neither pushed nor
        // checked, so `pos` on one falls through to the final `None`.
    }
    None
}
