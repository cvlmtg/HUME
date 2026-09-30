//! Rope-domain utilities shared across the HUME workspace.
//!
//! ## The trailing-newline invariant
//!
//! Every HUME buffer ends with a structural `\n`, so ropey reports one extra
//! empty "phantom" line past the real content. Line counts come in two
//! domains:
//!
//! - **Ropey domain** (`ropey_line_count`, `last_ropey_line`): phantom line
//!   included. Valid on any rope; for positions that must address every
//!   ropey line.
//! - **Content domain** (`content_line_count`, `last_content_line`): phantom
//!   line excluded. Assumes the invariant (debug-asserted); for user-facing
//!   counts and content bounds.
//!
//! ## LF is the only line break
//!
//! Ropey is compiled without `cr_lines` and `unicode_lines`, so lines split on
//! `\n` alone and `\r` (like VT, FF, NEL, LS, PS) is ordinary content. Line
//! terminator logic here is therefore single-char. `BufferText` normalizes
//! every line ending to `\n`, so a `\r` never reaches the functions that
//! walk a line's clusters up to its terminator (`char_pos_at_display_col`):
//! a `\r\n` cluster would straddle the terminator, and it debug-asserts
//! against it.

#![deny(rustdoc::broken_intra_doc_links)]

pub mod cluster;
pub mod column;
pub mod cursor;
pub mod grapheme;
pub mod line;
pub mod lines;
pub mod offset;
pub mod position_encoding;
pub mod width;

/// Test-only helpers shared by this crate's test submodules.
#[cfg(test)]
pub(crate) mod test_support {
    use ropey::Rope;
    use unicode_segmentation::UnicodeSegmentation;

    /// Mirrors `hume_editing::text::BufferText::from`'s trailing-newline
    /// invariant, so the algorithms under test are exercised against the same
    /// buffer shape production code always hands them. Line-ending
    /// normalization is not mirrored: it changes no rope this crate's tests
    /// build, since `\n` is the only break here.
    pub(crate) fn rope(s: &str) -> Rope {
        if s.ends_with('\n') {
            Rope::from_str(s)
        } else {
            let mut r = Rope::from_str(s);
            r.insert_char(r.len_chars(), '\n');
            r
        }
    }

    /// Char offsets of every cluster boundary in `text`, 0 and the text end
    /// included, straight from `unicode-segmentation`.
    pub(crate) fn segmentation_boundaries(text: &str) -> Vec<usize> {
        let mut out = vec![0];
        let mut chars = 0;
        for g in text.graphemes(true) {
            chars += g.chars().count();
            out.push(chars);
        }
        out
    }

    /// Byte offsets where ropey ends a chunk that fall inside a grapheme cluster
    /// of `r`.
    fn straddled_chunk_ends(r: &Rope) -> Vec<usize> {
        let text = r.to_string();
        let cluster_starts: std::collections::HashSet<usize> = text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain([text.len()])
            .collect();
        let mut byte = 0;
        r.chunks()
            .map(|chunk| {
                byte += chunk.len();
                byte
            })
            .filter(|b| !cluster_starts.contains(b))
            .collect()
    }

    /// A rope of clusters whose runs are long enough to straddle ropey chunk
    /// boundaries, shifted by an ASCII prefix until one does, with every char
    /// offset a chunk boundary sits at (and its neighbours) as a probe.
    pub(crate) fn chunk_straddling_text() -> (Rope, Vec<usize>) {
        for prefix in 0..4 {
            let text = format!(
                "{}{}{}",
                "a".repeat(prefix),
                test_fixtures::unicode::FLAG_RUN.repeat(600),
                test_fixtures::unicode::COMBINING.repeat(600)
            );
            let r = Rope::from_str(&text);
            if straddled_chunk_ends(&r).is_empty() {
                continue;
            }
            let mut probes = Vec::new();
            let mut byte = 0;
            for chunk in r.chunks() {
                byte += chunk.len();
                let at = r.byte_to_char(byte.min(r.len_bytes()));
                probes.extend(
                    (at.saturating_sub(4)..=(at + 4).min(r.len_chars())).filter(|&p| p > 0),
                );
            }
            return (r, probes);
        }
        panic!("no prefix made a cluster straddle a chunk boundary");
    }
}
