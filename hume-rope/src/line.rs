//! Domain-typed line indices and counts.
//!
//! Every HUME buffer ends with a structural `\n` (the trailing-newline
//! invariant — see [`crate::lines::ends_with_newline`]), which ropey does not
//! know about: it reports one line past the buffer's real content, the
//! "phantom" trailing line. Two domains answer "which line" / "how many
//! lines" differently, and mixing them up is an off-by-one:
//!
//! - **Ropey domain** ([`RopeyLine`], [`RopeyLineCount`]): ropey's own line
//!   indexing, phantom line included. Valid on any rope, invariant or not.
//! - **Content domain** ([`ContentLine`], [`ContentLineCount`]): the phantom
//!   line excluded. Assumes the trailing-newline invariant.
//!
//! These four types make the two domains distinct at compile time, so a
//! function taking a [`ContentLine`] cannot be handed a [`RopeyLine`] without
//! an explicit (and fallible, via [`RopeyLine::to_content`]) conversion, and
//! neither index type implements `Add`/`Sub`, so the `+ 1`/`- 1`
//! re-derivations these types exist to forbid are a compile error.
//!
//! Deliberately not a bare tuple struct with a `pub` field, unlike this
//! workspace's other newtypes (`ScopeId`, `ServerId`, `TimerId`): a `pub`
//! field would make `ContentLine(text.content_line_count().get() - 1)`
//! writable again, which is exactly the derivation these types exist to
//! forbid. The field stays private; every legitimate construction and every
//! legitimate use goes through a named method below.

use ropey::Rope;

use crate::lines::{content_line_count, last_content_line, last_ropey_line};

/// A line index in the ropey domain — ropey's own line indexing, phantom
/// trailing line included. See the module doc for the domain distinction.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RopeyLine(usize);

/// A line index in the content domain — a real line of buffer content, never
/// the phantom trailing line. See the module doc for the domain distinction.
///
/// `Default` is line 0 — see [`RopeyLine`]'s.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct ContentLine(usize);

/// How many lines ropey reports for a rope, phantom trailing line included.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RopeyLineCount(usize);

/// How many real content lines a buffer has, phantom trailing line excluded.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ContentLineCount(usize);

impl RopeyLine {
    /// Mint a ropey-domain line index already known to be valid — e.g. one
    /// just read back from another ropey-domain value. Does not check
    /// against any rope; a caller minting from unvalidated input wants
    /// [`RopeyLine::clamped`] instead.
    pub fn new(idx: usize) -> Self {
        Self(idx)
    }

    /// Clamp `idx` to `rope`'s last ropey line — the bound a scripted or
    /// wire-supplied line target must respect to stay addressable.
    pub fn clamped(rope: &Rope, idx: usize) -> Self {
        Self(idx.min(last_ropey_line(rope).0))
    }

    /// The bare 0-based index, for handing to ropey itself
    /// (`Rope::line_to_char` and friends) or to a foreign coordinate system
    /// (tree-sitter, LSP wire positions) that has no domain of its own.
    pub fn index(self) -> usize {
        self.0
    }

    /// `self`, `n` lines toward the end of the buffer. Unclamped — advancing
    /// past the last ropey line is a caller bug, not a value this type
    /// silently repairs. Bare `advance`, not `advance_saturating`: unlike
    /// `retreat_saturating`'s floor of 0, there is no ropey-domain ceiling
    /// this type can saturate against without a `&Rope` in hand, so this
    /// is the plain, unchecked op — matching `CharOffset::shift`/`retreat`'s
    /// own bare names for "no saturation," with `_saturating` reserved
    /// everywhere in the workspace for the clamped variant.
    pub fn advance(self, n: usize) -> Self {
        Self(self.0 + n)
    }

    /// Narrow to the content domain, or `None` if `self` is the phantom
    /// trailing line (or past it).
    pub fn to_content(self, rope: &Rope) -> Option<ContentLine> {
        (self.0 < content_line_count(rope).0).then_some(ContentLine(self.0))
    }
}

impl From<ContentLine> for RopeyLine {
    /// Widening a content-domain line into the ropey domain is always sound:
    /// every real content line is also a valid ropey line.
    fn from(line: ContentLine) -> Self {
        Self(line.0)
    }
}

impl ContentLine {
    /// Mint a content-domain line index already known to be valid.
    pub fn new(idx: usize) -> Self {
        Self(idx)
    }

    /// `idx` if it names a real content line of `rope`, else `None`.
    pub fn checked(rope: &Rope, idx: usize) -> Option<Self> {
        (idx < content_line_count(rope).0).then_some(Self(idx))
    }

    /// Clamp `idx` to `rope`'s last content line.
    pub fn clamped(rope: &Rope, idx: usize) -> Self {
        Self(idx.min(last_content_line(rope).0))
    }

    /// Decode a 1-based line number (CLI `path:line:col`, `:goto N`) into the
    /// content-domain index it addresses, or `None` if `n` is `0` — there is
    /// no line 0 to land on.
    pub fn from_number(n: usize) -> Option<Self> {
        n.checked_sub(1).map(Self)
    }

    /// The bare 0-based index.
    pub fn index(self) -> usize {
        self.0
    }

    /// The 1-based line number a user reads back (statusline, gutter,
    /// `:diagnostics`).
    pub fn number(self) -> usize {
        self.0 + 1
    }

    /// `self`, `n` lines toward the end of the buffer. Unclamped — a caller
    /// landing on or past the buffer's real content wants
    /// [`ContentLine::clamped`] to pull the result back in bounds. See
    /// [`RopeyLine::advance`] for why this is bare `advance`, not
    /// `advance_saturating`.
    pub fn advance(self, n: usize) -> Self {
        Self(self.0 + n)
    }

    /// `self`, `n` lines toward the start of the buffer, saturating at 0.
    /// Named `retreat_saturating`, not `up`, to match the same saturate-at-0
    /// contract's name on `CharOffset`/`BufferLineCol` — this codebase has
    /// no domain where a bare direction word and a `_saturating` suffix name
    /// the same behavior, so the two must not coexist for it here either.
    pub fn retreat_saturating(self, n: usize) -> Self {
        Self(self.0.saturating_sub(n))
    }

    /// Unsigned distance between `self` and `other`, direction discarded —
    /// the "how far apart are these two lines" metric a jump-distance
    /// threshold needs (`hume-editor`'s `step_record_jump`), where
    /// [`Self::lines_since`]'s ordering requirement would be the wrong tool.
    pub fn abs_diff(self, other: Self) -> usize {
        self.0.abs_diff(other.0)
    }

    /// Lines between `earlier` and `self` (`earlier <= self`) — the named
    /// forward-only form, mirroring `CharOffset::chars_since`. Debug-panics
    /// on inversion, where a raw subtraction would silently wrap.
    pub fn lines_since(self, earlier: Self) -> usize {
        debug_assert!(
            earlier <= self,
            "lines_since: {earlier:?} is after {self:?} — lines_since measures forward only"
        );
        self.0.saturating_sub(earlier.0)
    }
}

impl RopeyLineCount {
    pub(crate) fn new(count: usize) -> Self {
        Self(count)
    }

    /// The bare line count, phantom trailing line included.
    pub fn get(self) -> usize {
        self.0
    }
}

impl ContentLineCount {
    pub(crate) fn new(count: usize) -> Self {
        Self(count)
    }

    /// The bare line count, phantom trailing line excluded.
    pub fn get(self) -> usize {
        self.0
    }

    /// One past the last content line — the exclusive upper bound a
    /// half-open content-domain range (`viewport-range`, `buffer-lines`)
    /// ends at. The sanctioned way to name this index: unlike
    /// [`ContentLine::new`], which requires the value already be a real
    /// line, this index is one past every real line by construction.
    pub fn end_exclusive(self) -> ContentLine {
        ContentLine(self.0)
    }
}

#[cfg(test)]
mod tests;
