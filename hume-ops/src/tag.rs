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
///
/// Also recognizes HTML5's abruptly-closed forms `<!-->` and `<!--->` (a
/// comment with zero or one dash before the first `>`), and `--!>` in place
/// of `-->` — all three are HTML5 parse errors but real markup hits them.
fn comment_end(text: &BufferText, lt_pos: usize) -> usize {
    let start = lt_pos + 4; // past the "<!--" already confirmed by is_comment_start
    let cursor = text.chars_at(start);
    let mut dashes = 0;
    for (i, ch) in cursor {
        match ch {
            '-' => dashes += 1,
            // `--!>` closes like `-->` — keep `dashes` alive across the `!`.
            '!' if dashes >= 2 => {}
            '>' if dashes >= 2 || i == start || (i == start + 1 && dashes == 1) => {
                return i + 1;
            }
            _ => dashes = 0,
        }
    }
    text.len_chars()
}

/// Parse the tag construct starting at `lt_pos` (must be `<`, and not the
/// start of a comment). `None` for anything that isn't a well-formed
/// `<name…>`, `<name…/>`, or `</name>` — `<!DOCTYPE`, `<?xml`, a bare `<` in
/// running text or code, or an unterminated `<`. Quote-aware: a `>` inside a
/// `"…"`/`'…'` attribute value doesn't end the tag, and braced expression
/// attributes (`onClick={() => f()}`) don't end early at the arrow's `>`.
/// An unquoted, unbraced `<` — a stray comparison operator or the start of
/// the *next* tag — ends the parse with `None` rather than being consumed as
/// part of this one.
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
    let mut brace_depth = 0u32;
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
            '{' => {
                brace_depth += 1;
                last_significant = Some(ch);
            }
            '}' => {
                brace_depth = brace_depth.saturating_sub(1);
                last_significant = Some(ch);
            }
            '<' if brace_depth == 0 => return None,
            '>' if brace_depth == 0 => {
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

/// Find the tag construct whose own markup — its `<`, its `>`, the name, or
/// an attribute — contains `pos`. Walks left from `pos` for the nearest `<`
/// and parses forward from there.
///
/// The nearest preceding `<` fully determines the answer: if it starts a
/// comment or parses into a tag that doesn't reach `pos`, then `pos` sits in
/// plain text (or inside that comment) with no `<` between there and `pos`
/// — by construction, since this is the *first* `<` found scanning
/// backward — so there is no tag to find. If it doesn't parse as a tag at
/// all (a stray comparison operator, or the next tag's own `<`), keep
/// scanning left past it.
///
/// This keeps the common case — `#` pressed somewhere that isn't inside any
/// tag — to a short local walk instead of a whole-buffer parse.
fn tag_at(text: &BufferText, pos: usize) -> Option<Tag> {
    let mut cursor = text.chars_at(pos + 1);
    while let Some((i, ch)) = cursor.prev() {
        if ch != '<' {
            continue;
        }
        if is_comment_start(text, i) {
            return None;
        }
        return match parse_tag(text, i) {
            Some(tag) if tag.contains(pos) => Some(tag),
            _ => None,
        };
    }
    None
}

/// Forward scan for the tag that closes `open` (`<name>` → matching
/// `</name>`), tracking nesting depth for same-name tags only — the same
/// technique [`crate::pair::scan_right_for_close`] uses for brackets.
/// `open` must be a non-closing, non-self-closing tag.
fn close_after(text: &BufferText, open: &Tag) -> Option<usize> {
    let mut depth = 0usize;
    let mut from = open.gt_pos + 1;
    while let Some(tag) = next_tag(text, from) {
        from = tag.gt_pos + 1;
        if tag.self_closing || !same_name(text, tag.name, open.name) {
            continue;
        }
        if tag.closing {
            if depth == 0 {
                return Some(tag.lt_pos);
            }
            depth -= 1;
        } else {
            depth += 1;
        }
    }
    None
}

/// Forward scan from the start of the buffer for the tag that `close`
/// closes: the innermost still-open same-name tag at the point `close`
/// appears. Unlike [`close_after`], this can't start at `close` and walk
/// backward — an unmatched same-name open earlier in the buffer (e.g. a void
/// element written without a self-closing slash) must not be mistaken for
/// the partner, so the same forward, depth-tracked pairing `close_after`
/// does is run from 0 up to `close` itself. Cheaper than a full [`Tag`]
/// stack: only same-name open positions are pushed.
fn open_before(text: &BufferText, close: &Tag) -> Option<usize> {
    let mut opens: Vec<usize> = Vec::new();
    let mut from = 0;
    while let Some(tag) = next_tag(text, from) {
        if tag.lt_pos >= close.lt_pos {
            break;
        }
        from = tag.gt_pos + 1;
        if tag.self_closing || !same_name(text, tag.name, close.name) {
            continue;
        }
        if tag.closing {
            opens.pop();
        } else {
            opens.push(tag.lt_pos);
        }
    }
    opens.pop()
}

/// Find the matching partner of the tag at `pos`: given the cursor anywhere
/// inside a `<name…>` or `</name>` construct (its `<`, its `>`, the name, or
/// an attribute), return the `<` of its partner. `None` when `pos` isn't
/// inside a tag's own markup, the tag is self-closing (no partner), or it's
/// never closed or opened.
///
/// A same-name open discarded by an unmatched close never affects an
/// enclosing tag of a *different* name — [`close_after`] and
/// [`open_before`] each track only the one name they were asked about, so a
/// stray `</span>` can't drain an unrelated `<div>` off some shared stack.
pub(crate) fn matching_tag(text: &BufferText, pos: usize) -> Option<usize> {
    let tag = tag_at(text, pos)?;
    if tag.self_closing {
        return None;
    }
    if tag.closing {
        open_before(text, &tag)
    } else {
        close_after(text, &tag)
    }
}
