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
use hume_rope::cursor::CharCursor;

/// One parsed `<name…>`, `<name…/>`, or `</name>` construct.
struct Tag {
    closing: bool,
    self_closing: bool,
    name: (usize, usize), // inclusive char range
    lt_pos: usize,
    gt_pos: usize,
}

fn same_name(text: &BufferText, a: (usize, usize), b: (usize, usize)) -> bool {
    // Names are ASCII-only (`parse_tag` only accepts ascii_alphanumeric plus
    // `_`/`:`/`.`/`-`), so char length equals byte length — an exact,
    // zero-cost rejection that skips two `RopeSlice` tree walks (`slice`'s
    // `PartialEq` only short-circuits on `len_bytes` *after* building both)
    // for the common case of hunting one tag name through many others.
    (a.1 - a.0) == (b.1 - b.0) && text.slice(a.0..a.1 + 1) == text.slice(b.0..b.1 + 1)
}

/// True if `<!--` starts at `lt_pos`.
fn is_comment_start(text: &BufferText, lt_pos: usize) -> bool {
    let mut cursor = text.chars_at(lt_pos);
    "<!--"
        .chars()
        .all(|expected| cursor.next().is_some_and(|(_, ch)| ch == expected))
}

/// Consume `cursor` through the terminator of an HTML comment whose `<!--`
/// prefix `cursor` has already consumed, or to the end of the buffer if it's
/// never closed — an unterminated comment swallows everything after it, so
/// no tag inside it is reachable.
///
/// Also recognizes HTML5's abruptly-closed forms `<!-->` and `<!--->` (a
/// comment with zero or one dash before the first `>`), and `--!>` in place
/// of `-->` — all three are HTML5 parse errors but real markup hits them.
fn skip_comment_body(cursor: &mut CharCursor<'_>) {
    let mut dashes = 0u32;
    let mut offset = 0usize; // chars consumed since the confirmed "<!--"
    while let Some((_, ch)) = cursor.next() {
        match ch {
            '-' => dashes += 1,
            // `--!>` closes like `-->` — keep `dashes` alive across the `!`.
            '!' if dashes >= 2 => {}
            '>' if dashes >= 2 || offset == 0 || (offset == 1 && dashes == 1) => return,
            _ => dashes = 0,
        }
        offset += 1;
    }
}

/// Parse the tag construct starting at `lt_pos` (must be `<`, and not the
/// start of a comment) via `cursor`, which must already sit at `lt_pos + 1`
/// with `first` the pair that position yielded — [`next_tag`]'s shared
/// forward scan has already consumed it to rule out a comment, and a
/// `CharCursor` can't be rewound to hand it back. `None` for anything that
/// isn't a well-formed `<name…>`, `<name…/>`, or `</name>` — `<!DOCTYPE`,
/// `<?xml`, a bare `<` in running text or code, or an unterminated `<`.
/// Quote-aware: a `>` inside a `"…"`/`'…'` attribute value doesn't end the
/// tag, and braced expression attributes (`onClick={() => f()}`) don't end
/// early at the arrow's `>`. An unquoted, unbraced `<` — a stray comparison
/// operator or the start of the *next* tag — ends the parse with `None`
/// rather than being consumed as part of this one.
fn parse_tag(cursor: &mut CharCursor<'_>, lt_pos: usize, first: (usize, char)) -> Option<Tag> {
    let (mut i, mut ch) = first;
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

/// Find the next tag construct `cursor` reaches, skipping HTML comments and
/// any `<` that doesn't parse as a well-formed tag. Advances `cursor` in
/// place rather than taking a `from: usize` — [`close_after`]/[`open_before`]
/// share one cursor across every tag in their scan, so ropey's O(log n) tree
/// descent to seek a starting position is paid once per scan, not once per
/// tag. A failed attempt (a stray `<`, or a `<!` that isn't a comment after
/// all) still needs to resume scanning right after that `<` — reseeking
/// there with a fresh [`BufferText::chars_at`] is the one exception, kept to
/// that uncommon case instead of paid on every well-formed tag.
fn next_tag<'a>(text: &'a BufferText, cursor: &mut CharCursor<'a>) -> Option<Tag> {
    loop {
        let (lt_pos, ch) = cursor.next()?;
        if ch != '<' {
            continue;
        }
        let Some(first) = cursor.next() else {
            return None; // buffer ends right after '<' — nothing more to find.
        };
        if first.1 == '!' {
            // Comments are the one construct `parse_tag` doesn't parse — it
            // would reject '!' as a name-start character regardless of what
            // follows, so a well-formed comment is resolved here instead of
            // costing a doomed `parse_tag` call.
            if matches!(cursor.next(), Some((_, '-'))) && matches!(cursor.next(), Some((_, '-'))) {
                skip_comment_body(cursor);
                continue;
            }
            // Not a comment either (`<!DOCTYPE`, stray `<!` junk) — `parse_tag`
            // would reject '!' anyway, so fall through to the reseek below
            // instead of calling it.
        } else if let Some(tag) = parse_tag(cursor, lt_pos, first) {
            return Some(tag);
        }
        // Failed to parse a tag at `lt_pos` — resume right after it,
        // discarding whatever the comment check or `parse_tag` looked ahead
        // at, which could itself be the next tag's own `<` (as in `a<b\n<div>`).
        *cursor = text.chars_at(lt_pos + 1);
    }
}

/// Find the tag construct whose own markup — its `<`, its `>`, the name, or
/// an attribute — contains `pos`. Walks left from `pos` for the nearest `<`
/// and parses forward from there. Matching matchit/vim's `%`, which fires
/// from anywhere in a tag's own markup, not just its two delimiter chars.
///
/// The nearest preceding `<` does *not* always settle the answer on its own:
/// a `<` can sit inside an *enclosing* tag's own markup — a quoted attribute
/// value (`<a t="<">`) or a braced JSX expression (`<div onClick={a < b}>`)
/// — where it parses as nothing at all. When that happens, `pos` may still
/// be inside that enclosing tag, so scanning must continue left past it.
/// Two other outcomes end the walk immediately: a `<` starting a comment (no
/// tag can be found through it), and a `<` that parses into a well-formed
/// tag not reaching `pos` (a real tag boundary lies between it and `pos`,
/// which therefore isn't inside any tag's markup).
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
        match parse_tag_at(text, i) {
            Some(tag) if (tag.lt_pos..=tag.gt_pos).contains(&pos) => return Some(tag),
            Some(_) => return None,
            None => continue,
        }
    }
    None
}

/// [`parse_tag`] for a one-off call, building its own cursor from `lt_pos`.
/// [`tag_at`]'s single backward-search call per `#` press is the only
/// caller — sharing a cursor across calls buys nothing there, unlike
/// [`next_tag`]'s forward multi-tag scans.
fn parse_tag_at(text: &BufferText, lt_pos: usize) -> Option<Tag> {
    let mut cursor = text.chars_at(lt_pos + 1);
    let first = cursor.next()?;
    parse_tag(&mut cursor, lt_pos, first)
}

/// Forward scan for the tag that closes `open` (`<name>` → matching
/// `</name>`), tracking nesting depth for same-name tags only — the same
/// technique [`crate::pair::scan_right_for_close`] uses for brackets.
/// `open` must be a non-closing, non-self-closing tag.
fn close_after(text: &BufferText, open: &Tag) -> Option<usize> {
    let mut depth = 0usize;
    let mut cursor = text.chars_at(open.gt_pos + 1);
    while let Some(tag) = next_tag(text, &mut cursor) {
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

/// Backward counterpart to [`next_tag`]: the nearest tag construct strictly
/// before `before`, skipping comments and any `<` that doesn't parse as a
/// well-formed tag — the same two exemptions [`tag_at`] uses. Reuses the
/// forward [`parse_tag_at`] rather than a mirrored reverse lexer: finding
/// `<` backward is unambiguous, but a lexer that also had to un-consume a
/// quoted attribute or braced JSX expression in reverse would duplicate
/// [`parse_tag`]'s whole state machine for zero correctness gain over just
/// parsing forward from each candidate.
///
/// That reuse has a real gap: a `<` inside an *enclosing* tag's own quoted
/// attribute or JSX brace — `title="</div>"`, `<!-- <div> -->` — is
/// invisible to the forward lexer (consumed whole as part of the enclosing
/// tag's own parse) but can itself parse as a well-formed tag when found
/// this way, corrupting [`open_before`]'s depth count. An unbalanced
/// fragment there (one tag-shaped construct, not a real pair) triggers it; a
/// balanced one cancels itself out and is harmless. Accepted as a known
/// limitation of the lexical scanner rather than fixed, since a
/// tree-sitter-backed matcher is expected to supersede this path.
fn prev_tag(text: &BufferText, before: usize) -> Option<Tag> {
    let mut cursor = text.chars_at(before);
    while let Some((i, ch)) = cursor.prev() {
        if ch == '<'
            && !is_comment_start(text, i)
            && let Some(tag) = parse_tag_at(text, i)
        {
            return Some(tag);
        }
    }
    None
}

/// Depth-tracked backward walk from `close` for the innermost still-open
/// same-name tag: the same technique [`crate::pair::scan_left_for_open`]
/// uses for brackets, applied via [`prev_tag`] instead of a raw char scan.
/// Scans outward from the cursor rather than from the start of the buffer —
/// depth-tracked matching is direction-symmetric, so this returns the same
/// answer [`close_after`]'s forward, from-open-tag scan would give in
/// reverse, while typically touching only the handful of tags between
/// `close` and its partner instead of the whole preceding buffer. See
/// [`prev_tag`]'s doc for the one case this reuse doesn't handle.
fn open_before(text: &BufferText, close: &Tag) -> Option<usize> {
    let mut depth = 0usize;
    let mut before = close.lt_pos;
    while let Some(tag) = prev_tag(text, before) {
        before = tag.lt_pos;
        if tag.self_closing || !same_name(text, tag.name, close.name) {
            continue;
        }
        if tag.closing {
            depth += 1;
        } else if depth == 0 {
            return Some(tag.lt_pos);
        } else {
            depth -= 1;
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
