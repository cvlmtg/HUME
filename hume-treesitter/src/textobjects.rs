//! Vocabulary for tree-sitter structural text objects and navigation: kinds,
//! spans, and the per-`(kind, span)` capture-index table a compiled
//! `textobjects.scm` resolves to.
//!
//! This module only names things and resolves capture indices at attach
//! time — span collection (hulls over grouped matches), freshness, and
//! selection policy are later phases.

// ── ObjectKind ─────────────────────────────────────────────────────────────

/// A structural object a `textobjects.scm` query may define, named after the
/// `<kind>` half of its capture names (`function.inside`, `class.around`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Function,
    Class,
    Parameter,
    Comment,
    Test,
    Entry,
}

impl ObjectKind {
    pub const ALL: [ObjectKind; 6] = [
        ObjectKind::Function,
        ObjectKind::Class,
        ObjectKind::Parameter,
        ObjectKind::Comment,
        ObjectKind::Test,
        ObjectKind::Entry,
    ];

    /// The `<kind>` half of a capture name, e.g. `@function.inside` names
    /// `"function"`. Single source of truth: also used in reverse by
    /// [`Self::from_capture_name`].
    pub fn capture_name(self) -> &'static str {
        match self {
            ObjectKind::Function => "function",
            ObjectKind::Class => "class",
            ObjectKind::Parameter => "parameter",
            ObjectKind::Comment => "comment",
            ObjectKind::Test => "test",
            ObjectKind::Entry => "entry",
        }
    }

    fn from_capture_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.capture_name() == name)
    }
}

// ── ObjectSpan ─────────────────────────────────────────────────────────────

/// Which part of an object a capture spans. `Movement` is Helix's optional
/// navigation-only capture (a function's name node, say) — narrower than
/// `Around`, consumed only by navigation, never by selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectSpan {
    Inside,
    Around,
    Movement,
}

impl ObjectSpan {
    pub const ALL: [ObjectSpan; 3] = [ObjectSpan::Inside, ObjectSpan::Around, ObjectSpan::Movement];

    /// The `<span>` half of a capture name, e.g. `@function.inside` names
    /// `"inside"`.
    pub fn capture_name(self) -> &'static str {
        match self {
            ObjectSpan::Inside => "inside",
            ObjectSpan::Around => "around",
            ObjectSpan::Movement => "movement",
        }
    }

    fn from_capture_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|span| span.capture_name() == name)
    }
}

// ── Direction ──────────────────────────────────────────────────────────────

/// The only direction enum for structural navigation. `hume-ops` takes a
/// `backward: bool` at its API boundary (the `apply_word_select`
/// convention) rather than importing this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

// ── TextObjectsQuery ───────────────────────────────────────────────────────

/// A compiled `textobjects.scm` query plus a dense `(kind, span) → capture
/// index` table, resolved once at attach time by splitting each capture
/// name on its last `.` (`@parameter.inside.extra` would split on the last
/// dot, but no Helix query defines such a name today). Names that don't
/// parse as `<kind>.<span>` (`@_helper`, `@function.x`) map to nothing.
pub struct TextObjectsQuery {
    /// Read directly by Phase 2's span-collection executor
    /// (`ObjectSpans::collect`), which needs the compiled `Query` itself,
    /// not just this table.
    pub query: tree_sitter::Query,
    captures: [[Option<u32>; ObjectSpan::ALL.len()]; ObjectKind::ALL.len()],
}

impl TextObjectsQuery {
    pub(crate) fn new(query: tree_sitter::Query) -> Self {
        let mut captures = [[None; ObjectSpan::ALL.len()]; ObjectKind::ALL.len()];
        for (idx, name) in query.capture_names().iter().enumerate() {
            let Some((kind_name, span_name)) = name.rsplit_once('.') else {
                continue;
            };
            let (Some(kind), Some(span)) = (
                ObjectKind::from_capture_name(kind_name),
                ObjectSpan::from_capture_name(span_name),
            ) else {
                continue;
            };
            captures[kind as usize][span as usize] = Some(idx as u32);
        }
        Self { query, captures }
    }

    /// Whether this query defines a `<kind>.<span>` capture.
    pub fn defines(&self, kind: ObjectKind, span: ObjectSpan) -> bool {
        self.captures[kind as usize][span as usize].is_some()
    }
}

#[cfg(test)]
mod tests;
