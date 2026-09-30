use ropey::RopeSlice;
use ropey::iter::Chunks;
use ropey::str_utils::{byte_to_char_idx, char_to_byte_idx};
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

/// The chunk of a slice holding some byte, with the chunk's byte and char
/// origins. Opened by one rope seek, which reads that chunk alone; a query
/// that stays inside it costs nothing more. The chunk iterator for stepping
/// to a neighbouring chunk is built on the first step, by a second seek.
///
/// When built, `chunks` sits just before `chunk`: its `next()` yields `chunk`
/// again and its `prev()` yields the chunk before. `chunk` is empty only at
/// the text end, where its origins are the slice's lengths. Byte arguments
/// must lie in the current chunk or at its end.
#[derive(Clone)]
pub(super) struct ChunkCursor<'a> {
    slice: RopeSlice<'a>,
    chunks: Option<Chunks<'a>>,
    chunk: &'a str,
    chunk_byte_start: usize,
    chunk_char_start: usize,
}

impl<'a> ChunkCursor<'a> {
    /// The cursor on the chunk holding `byte`, which must not be past the
    /// slice.
    pub(super) fn at_byte(slice: RopeSlice<'a>, byte: usize) -> Self {
        assert!(
            byte <= slice.len_bytes(),
            "byte {byte} is past the text end ({})",
            slice.len_bytes()
        );
        if byte == slice.len_bytes() {
            return Self::text_end(slice);
        }
        let (chunk, chunk_byte_start, chunk_char_start, _) = slice.chunk_at_byte(byte);
        Self {
            slice,
            chunks: None,
            chunk,
            chunk_byte_start,
            chunk_char_start,
        }
    }

    /// The cursor on the chunk holding `char_idx`, which must not be past the
    /// slice, and that char's byte offset.
    pub(super) fn at_char(slice: RopeSlice<'a>, char_idx: usize) -> (Self, usize) {
        assert!(
            char_idx <= slice.len_chars(),
            "char {char_idx} is past the text end ({})",
            slice.len_chars()
        );
        if char_idx == slice.len_chars() {
            return (Self::text_end(slice), slice.len_bytes());
        }
        let (chunk, chunk_byte_start, chunk_char_start, _) = slice.chunk_at_char(char_idx);
        let byte = chunk_byte_start + char_to_byte_idx(chunk, char_idx - chunk_char_start);
        let cur = Self {
            slice,
            chunks: None,
            chunk,
            chunk_byte_start,
            chunk_char_start,
        };
        (cur, byte)
    }

    fn text_end(slice: RopeSlice<'a>) -> Self {
        Self {
            slice,
            chunks: None,
            chunk: "",
            chunk_byte_start: slice.len_bytes(),
            chunk_char_start: slice.len_chars(),
        }
    }

    /// The chunk iterator, built just before the current chunk on first use.
    fn chunks(&mut self) -> &mut Chunks<'a> {
        let (slice, byte_start) = (self.slice, self.chunk_byte_start);
        self.chunks
            .get_or_insert_with(|| slice.chunks_at_byte(byte_start).0)
    }

    pub(super) fn slice(&self) -> RopeSlice<'a> {
        self.slice
    }

    pub(super) fn chunk(&self) -> &'a str {
        self.chunk
    }

    pub(super) fn chunk_byte_start(&self) -> usize {
        self.chunk_byte_start
    }

    fn chunk_byte_end(&self) -> usize {
        self.chunk_byte_start + self.chunk.len()
    }

    /// Moves to the next non-empty chunk, or to the text end, where it
    /// returns `false`.
    pub(super) fn advance(&mut self) -> bool {
        if self.chunk.is_empty() {
            return false;
        }
        let departed = self.chunk;
        let chunks = self.chunks();
        chunks.next();
        let next = loop {
            match chunks.next() {
                Some("") => {}
                Some(chunk) => {
                    chunks.prev();
                    break Some(chunk);
                }
                None => break None,
            }
        };
        self.chunk_byte_start += departed.len();
        self.chunk_char_start += byte_to_char_idx(departed, departed.len());
        match next {
            Some(chunk) => {
                self.chunk = chunk;
                true
            }
            None => {
                self.chunk = "";
                false
            }
        }
    }

    /// Moves to the previous non-empty chunk; `false`, without moving, at the
    /// text start.
    pub(super) fn retreat(&mut self) -> bool {
        if self.chunk_byte_start == 0 {
            return false;
        }
        let chunks = self.chunks();
        let chunk = loop {
            match chunks.prev() {
                Some("") => {}
                Some(chunk) => break chunk,
                None => {
                    unreachable!("a chunk starting past byte 0 has a non-empty chunk before it")
                }
            }
        };
        self.chunk = chunk;
        self.chunk_byte_start -= chunk.len();
        self.chunk_char_start -= byte_to_char_idx(chunk, chunk.len());
        true
    }

    /// The char offset of `byte`. A byte inside a codepoint counts as that
    /// codepoint's char.
    pub(super) fn byte_to_char(&self, byte: usize) -> usize {
        debug_assert!(self.chunk_byte_start <= byte && byte <= self.chunk_byte_end());
        self.chunk_char_start + byte_to_char_idx(self.chunk, byte - self.chunk_byte_start)
    }

    /// The char holding `byte`, as its char offset and its first byte; the
    /// text end's offsets for the slice's byte length.
    pub(super) fn char_holding(&self, byte: usize) -> (usize, usize) {
        let local = self.chunk.floor_char_boundary(byte - self.chunk_byte_start);
        (
            self.chunk_char_start + byte_to_char_idx(self.chunk, local),
            self.chunk_byte_start + local,
        )
    }

    /// The char starting at `byte`, which must be below the slice's byte
    /// length. Moves to the next chunk when `byte` is the current one's end.
    pub(super) fn char_at(&mut self, byte: usize) -> char {
        if byte == self.chunk_byte_end() {
            self.advance();
        }
        self.chunk[byte - self.chunk_byte_start..]
            .chars()
            .next()
            .expect("a byte below the slice's length starts a char")
    }

    /// The char starting at `byte`, or `None` at the text end. The cursor
    /// ends where it started.
    pub(super) fn char_after(&mut self, byte: usize) -> Option<char> {
        if byte == self.slice.len_bytes() {
            return None;
        }
        if let Some(ch) = self.chunk[byte - self.chunk_byte_start..].chars().next() {
            return Some(ch);
        }
        self.advance();
        let ch = self.chunk.chars().next();
        self.retreat();
        ch
    }

    /// The char ending at `byte`, or `None` at the text start. The cursor
    /// ends where it started.
    pub(super) fn char_before(&mut self, byte: usize) -> Option<char> {
        if let Some(ch) = self.chunk[..byte - self.chunk_byte_start]
            .chars()
            .next_back()
        {
            return Some(ch);
        }
        if !self.retreat() {
            return None;
        }
        let ch = self.chunk.chars().next_back();
        self.advance();
        ch
    }

    /// The first boundary after `from_byte`, which must be below the slice's
    /// byte length. The cursor ends on the chunk holding the boundary.
    pub(super) fn next_boundary(&mut self, from_byte: usize) -> usize {
        let len = self.slice.len_bytes();
        if from_byte == self.chunk_byte_end() {
            self.advance();
        }
        // A fresh cursor per call: one carried over from the previous cluster
        // splits a regional-indicator pair that straddles a chunk boundary.
        let mut gc = GraphemeCursor::new(from_byte, len, true);
        let mut probe = None;
        loop {
            match gc.next_boundary(self.chunk, self.chunk_byte_start) {
                Ok(Some(b)) => return b,
                Ok(None) => return len,
                Err(GraphemeIncomplete::NextChunk) => {
                    if !self.advance() {
                        return len;
                    }
                }
                Err(GraphemeIncomplete::PreContext(n)) => {
                    self.provide_context(&mut probe, &mut gc, n)
                }
                Err(_) => unreachable!("next_boundary asks only for the next chunk or context"),
            }
        }
    }

    /// The last boundary before `from_byte`, which must be above 0. The
    /// cursor ends on the chunk holding the boundary.
    pub(super) fn prev_boundary(&mut self, from_byte: usize) -> usize {
        if from_byte == self.chunk_byte_start {
            self.retreat();
        }
        let mut gc = GraphemeCursor::new(from_byte, self.slice.len_bytes(), true);
        let mut probe = None;
        loop {
            match gc.prev_boundary(self.chunk, self.chunk_byte_start) {
                Ok(Some(b)) => return b,
                Ok(None) => return 0,
                Err(GraphemeIncomplete::PrevChunk) => {
                    if !self.retreat() {
                        return 0;
                    }
                }
                Err(GraphemeIncomplete::PreContext(n)) => {
                    self.provide_context(&mut probe, &mut gc, n)
                }
                Err(_) => unreachable!("prev_boundary asks only for the previous chunk or context"),
            }
        }
    }

    /// Hands `gc` the text ending at byte `n`, read by `probe`: a copy of
    /// this cursor that walks back so this one keeps its place. A resolution
    /// asks for context further back each time until it moves to another
    /// chunk, so the probe is copied afresh only when it is behind `n`.
    fn provide_context(&self, probe: &mut Option<Self>, gc: &mut GraphemeCursor, n: usize) {
        let probe = match probe {
            Some(p) if p.chunk_byte_end() >= n => p,
            _ => probe.insert(self.clone()),
        };
        while probe.chunk_byte_start >= n {
            probe.retreat();
        }
        gc.provide_context(
            &probe.chunk[..n - probe.chunk_byte_start],
            probe.chunk_byte_start,
        );
    }
}

#[cfg(test)]
mod tests;
