//! Postings iteration abstractions ported from `org.apache.lucene.index`.
//!
//! This module provides [`PostingsEnum`], the low-level iterator over the
//! inverted index for a single term, plus the impact-related types used by
//! block-max WAND.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};

/// Request no optional information from a postings enum.
///
/// Equivalent to `PostingsEnum.NONE`.
pub const POSTINGS_ENUM_NONE: i32 = 0x00;

/// Request term frequencies.
///
/// Equivalent to `PostingsEnum.FREQS`.
pub const POSTINGS_ENUM_FREQS: i32 = 0x04;

/// Request term positions.
///
/// Equivalent to `PostingsEnum.POSITIONS`.
pub const POSTINGS_ENUM_POSITIONS: i32 = POSTINGS_ENUM_FREQS | 0x08;

/// Request term offsets.
///
/// Equivalent to `PostingsEnum.OFFSETS`.
pub const POSTINGS_ENUM_OFFSETS: i32 = POSTINGS_ENUM_POSITIONS | 0x20;

/// Request term payloads.
///
/// Equivalent to `PostingsEnum.PAYLOADS`.
pub const POSTINGS_ENUM_PAYLOADS: i32 = POSTINGS_ENUM_POSITIONS | 0x40;

/// Request everything (frequencies, positions, payloads and offsets).
///
/// Equivalent to `PostingsEnum.ALL`.
pub const POSTINGS_ENUM_ALL: i32 = POSTINGS_ENUM_OFFSETS | POSTINGS_ENUM_PAYLOADS;

/// Returns `true` if `feature` is requested in `flags`.
///
/// Equivalent to `PostingsEnum.featureRequested(int, short)`.
pub fn feature_requested(flags: i32, feature: i32) -> bool {
    (flags & feature) == feature
}

/// Iterator over the postings of a single term.
///
/// Equivalent to `org.apache.lucene.index.PostingsEnum`.
///
/// The iterator is initially unpositioned; callers must call [`next_doc`]
/// before using any per-document method.
///
/// [`next_doc`]: crate::search::DocIdSetIterator::next_doc
pub trait PostingsEnum: DocIdSetIterator {
    /// Returns the term frequency in the current document, or `1` if the field
    /// was indexed with `IndexOptions::DOCS`.
    fn freq(&self) -> Result<i32>;

    /// Returns the next position, or `-1` if positions were not indexed.
    fn next_position(&mut self) -> Result<i32>;

    /// Returns the start offset for the current position, or `-1` if offsets
    /// were not indexed.
    fn start_offset(&self) -> i32;

    /// Returns the end offset for the current position, or `-1` if offsets were
    /// not indexed.
    fn end_offset(&self) -> i32;

    /// Returns the payload at the current position, or `None` if no payload was
    /// indexed.
    fn get_payload(&self) -> Result<Option<&[u8]>>;

    /// Fills `buffer` with doc IDs and frequencies starting at the current doc
    /// ID and ending before `up_to`.
    ///
    /// The default implementation copies at most 16 `(doc, freq)` pairs.
    fn next_postings(&mut self, up_to: i32, buffer: &mut DocAndFloatFeatureBuffer) -> Result<()> {
        let batch_size = 16;
        buffer.grow_no_copy(batch_size);
        let mut size = 0;
        let mut doc = self.doc_id();
        while doc < up_to && size < batch_size {
            buffer.docs[size] = doc;
            buffer.features[size] = self.freq()? as f32;
            size += 1;
            doc = self.next_doc()?;
        }
        buffer.size = size;
        Ok(())
    }
}

/// Buffer of doc IDs paired with float-valued features.
///
/// Equivalent to `org.apache.lucene.search.DocAndFloatFeatureBuffer`.
#[derive(Debug, Default, Clone)]
pub struct DocAndFloatFeatureBuffer {
    /// Doc IDs.
    pub docs: Vec<i32>,
    /// Float-valued features (typically frequencies or scores).
    pub features: Vec<f32>,
    /// Number of valid entries.
    pub size: usize,
}

impl DocAndFloatFeatureBuffer {
    /// Creates an empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Grows both arrays to store at least `min_size` entries.
    pub fn grow_no_copy(&mut self, min_size: usize) {
        if self.docs.len() < min_size {
            self.docs = vec![0; min_size];
            self.features = vec![0.0; min_size];
        }
    }

    /// Removes entries whose doc ID is unset in `live_docs`.
    pub fn apply(&mut self, live_docs: &dyn crate::util::Bits) {
        let mut new_size = 0;
        for i in 0..self.size {
            if live_docs.get(self.docs[i] as usize) {
                self.docs[new_size] = self.docs[i];
                self.features[new_size] = self.features[i];
                new_size += 1;
            }
        }
        self.size = new_size;
    }
}

/// Source of upcoming impacts for a postings list.
///
/// Equivalent to `org.apache.lucene.index.ImpactsSource`.
pub trait ImpactsSource {
    /// Shallow-advances to `target`.
    fn advance_shallow(&mut self, target: i32) -> Result<()>;

    /// Returns information about upcoming impacts.
    fn get_impacts(&mut self) -> Result<Box<dyn Impacts>>;
}

/// Extension of [`PostingsEnum`] that also exposes impacts.
///
/// Equivalent to `org.apache.lucene.index.ImpactsEnum`.
pub trait ImpactsEnum: PostingsEnum + ImpactsSource {}

/// Information about upcoming impacts, i.e. `(freq, norm)` pairs.
///
/// Equivalent to `org.apache.lucene.index.Impacts`.
pub trait Impacts {
    /// Number of impact levels available.
    fn num_levels(&self) -> i32;

    /// Maximum inclusive doc ID for which [`Self::get_impacts`] at `level` is
    /// valid.
    fn doc_id_up_to(&self, level: i32) -> i32;

    /// Returns the impacts at the given level.
    fn get_impacts(&self, level: i32) -> FreqAndNormBuffer;
}

/// Buffer of term frequencies paired with length-normalization factors.
///
/// Equivalent to `org.apache.lucene.index.FreqAndNormBuffer`.
#[derive(Debug, Default, Clone)]
pub struct FreqAndNormBuffer {
    /// Term frequencies.
    pub freqs: Vec<i32>,
    /// Length normalization factors.
    pub norms: Vec<i64>,
    /// Number of valid entries.
    pub size: usize,
}

impl FreqAndNormBuffer {
    /// Creates an empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Grows both arrays to store at least `min_size` entries.
    pub fn grow_no_copy(&mut self, min_size: usize) {
        if self.freqs.len() < min_size {
            self.freqs = vec![0; min_size];
            self.norms = vec![0; min_size];
        }
    }

    /// Adds a `(freq, norm)` pair at the end of the buffer.
    pub fn add(&mut self, freq: i32, norm: i64) {
        if self.freqs.len() == self.size {
            let new_len = (self.size + 1).max(self.freqs.len() * 2 + 1);
            self.freqs.resize(new_len, 0);
            self.norms.resize(new_len, 0);
        }
        self.freqs[self.size] = freq;
        self.norms[self.size] = norm;
        self.size += 1;
    }
}

/// An empty [`PostingsEnum`] instance.
#[derive(Debug, Clone)]
pub struct EmptyPostingsEnum;

impl Default for EmptyPostingsEnum {
    fn default() -> Self {
        Self::new()
    }
}

impl EmptyPostingsEnum {
    /// Creates an empty postings enum.
    pub fn new() -> Self {
        Self
    }
}

impl DocIdSetIterator for EmptyPostingsEnum {
    fn doc_id(&self) -> i32 {
        NO_MORE_DOCS
    }

    fn next_doc(&mut self) -> Result<i32> {
        Ok(NO_MORE_DOCS)
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Ok(NO_MORE_DOCS)
    }

    fn cost(&self) -> i64 {
        0
    }
}

impl PostingsEnum for EmptyPostingsEnum {
    fn freq(&self) -> Result<i32> {
        Ok(0)
    }

    fn next_position(&mut self) -> Result<i32> {
        Ok(-1)
    }

    fn start_offset(&self) -> i32 {
        -1
    }

    fn end_offset(&self) -> i32 {
        -1
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        Ok(None)
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::DocIdSetIterator;
    use crate::util::BytesRef;

    /// Stub postings enum backed by a list of `(doc, freq, positions, offsets,
    /// payload)` entries.
    struct VecPostingsEnum {
        docs: Vec<i32>,
        freqs: Vec<i32>,
        pos: Vec<Vec<i32>>,
        start_offsets: Vec<Vec<i32>>,
        end_offsets: Vec<Vec<i32>>,
        payloads: Vec<Option<BytesRef>>,
        idx: i32,
        pos_idx: i32,
    }

    impl VecPostingsEnum {
        fn new(
            docs: Vec<i32>,
            freqs: Vec<i32>,
            pos: Vec<Vec<i32>>,
            start_offsets: Vec<Vec<i32>>,
            end_offsets: Vec<Vec<i32>>,
            payloads: Vec<Option<BytesRef>>,
        ) -> Self {
            Self {
                docs,
                freqs,
                pos,
                start_offsets,
                end_offsets,
                payloads,
                idx: -1,
                pos_idx: -1,
            }
        }
    }

    impl DocIdSetIterator for VecPostingsEnum {
        fn doc_id(&self) -> i32 {
            if self.idx < 0 {
                -1
            } else if self.idx as usize >= self.docs.len() {
                NO_MORE_DOCS
            } else {
                self.docs[self.idx as usize]
            }
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.pos_idx = -1;
            self.idx += 1;
            if self.idx as usize >= self.docs.len() {
                Ok(NO_MORE_DOCS)
            } else {
                Ok(self.docs[self.idx as usize])
            }
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            self.pos_idx = -1;
            let start = (self.idx + 1).max(0) as usize;
            match self.docs[start..].iter().position(|&d| d >= target) {
                Some(p) => {
                    self.idx = (start + p) as i32;
                    Ok(self.docs[start + p])
                }
                None => {
                    self.idx = self.docs.len() as i32;
                    Ok(NO_MORE_DOCS)
                }
            }
        }

        fn cost(&self) -> i64 {
            self.docs.len() as i64
        }
    }

    impl PostingsEnum for VecPostingsEnum {
        fn freq(&self) -> Result<i32> {
            Ok(self.freqs[self.idx as usize])
        }

        fn next_position(&mut self) -> Result<i32> {
            self.pos_idx += 1;
            if self.pos_idx as usize >= self.pos[self.idx as usize].len() {
                Ok(-1)
            } else {
                Ok(self.pos[self.idx as usize][self.pos_idx as usize])
            }
        }

        fn start_offset(&self) -> i32 {
            if self.pos_idx < 0
                || self.pos_idx as usize >= self.start_offsets[self.idx as usize].len()
            {
                -1
            } else {
                self.start_offsets[self.idx as usize][self.pos_idx as usize]
            }
        }

        fn end_offset(&self) -> i32 {
            if self.pos_idx < 0
                || self.pos_idx as usize >= self.end_offsets[self.idx as usize].len()
            {
                -1
            } else {
                self.end_offsets[self.idx as usize][self.pos_idx as usize]
            }
        }

        fn get_payload(&self) -> Result<Option<&[u8]>> {
            if self.idx < 0 || self.idx as usize >= self.payloads.len() {
                Ok(None)
            } else {
                Ok(self.payloads[self.idx as usize].as_ref().map(|b| b.slice()))
            }
        }
    }

    #[test]
    fn empty_postings_enum_is_exhausted() {
        let mut it = EmptyPostingsEnum::new();
        assert_eq!(it.doc_id(), NO_MORE_DOCS);
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(it.freq().unwrap(), 0);
    }

    #[test]
    fn postings_flags_match_java_values() {
        assert_eq!(POSTINGS_ENUM_NONE, 0);
        assert_eq!(POSTINGS_ENUM_FREQS, 0x04);
        assert_eq!(POSTINGS_ENUM_POSITIONS, 0x04 | 0x08);
        assert_eq!(POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_POSITIONS | 0x20);
        assert_eq!(POSTINGS_ENUM_PAYLOADS, POSTINGS_ENUM_POSITIONS | 0x40);
        assert_eq!(
            POSTINGS_ENUM_ALL,
            POSTINGS_ENUM_OFFSETS | POSTINGS_ENUM_PAYLOADS
        );
    }

    #[test]
    fn feature_requested_detects_exact_flag() {
        assert!(feature_requested(POSTINGS_ENUM_FREQS, POSTINGS_ENUM_FREQS));
        assert!(!feature_requested(POSTINGS_ENUM_NONE, POSTINGS_ENUM_FREQS));
        assert!(feature_requested(
            POSTINGS_ENUM_ALL,
            POSTINGS_ENUM_POSITIONS
        ));
    }

    #[test]
    fn vec_postings_enum_iterates_docs_freqs_positions_offsets_payloads() {
        let mut it = VecPostingsEnum::new(
            vec![1, 5, 10],
            vec![2, 1, 3],
            vec![vec![0, 2], vec![0], vec![0, 1, 2]],
            vec![vec![0, 4], vec![0], vec![0, 2, 4]],
            vec![vec![3, 6], vec![5], vec![1, 3, 5]],
            vec![
                Some(BytesRef::new(vec![0x01])),
                None,
                Some(BytesRef::new(vec![0x02, 0x03])),
            ],
        );

        assert_eq!(it.next_doc().unwrap(), 1);
        assert_eq!(it.freq().unwrap(), 2);
        assert_eq!(it.next_position().unwrap(), 0);
        assert_eq!(it.start_offset(), 0);
        assert_eq!(it.end_offset(), 3);
        assert_eq!(it.next_position().unwrap(), 2);
        assert_eq!(it.start_offset(), 4);
        assert_eq!(it.end_offset(), 6);
        assert_eq!(it.get_payload().unwrap(), Some(&[0x01][..]));

        assert_eq!(it.next_doc().unwrap(), 5);
        assert_eq!(it.freq().unwrap(), 1);
        assert_eq!(it.next_position().unwrap(), 0);
        assert_eq!(it.get_payload().unwrap(), None);

        assert_eq!(it.advance(9).unwrap(), 10);
        assert_eq!(it.freq().unwrap(), 3);
        assert_eq!(it.next_position().unwrap(), 0);
        assert_eq!(it.start_offset(), 0);
        assert_eq!(it.end_offset(), 1);
        assert_eq!(it.get_payload().unwrap(), Some(&[0x02, 0x03][..]));

        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn next_postings_default_fills_buffer() {
        let mut it = VecPostingsEnum::new(
            vec![0, 2, 4, 6],
            vec![1, 2, 3, 4],
            vec![vec![], vec![], vec![], vec![]],
            vec![vec![], vec![], vec![], vec![]],
            vec![vec![], vec![], vec![], vec![]],
            vec![None, None, None, None],
        );
        it.next_doc().unwrap();
        let mut buffer = DocAndFloatFeatureBuffer::new();
        it.next_postings(100, &mut buffer).unwrap();
        assert_eq!(buffer.size, 4);
        assert_eq!(buffer.docs[..4], [0, 2, 4, 6]);
        assert_eq!(buffer.features[..4], [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn freq_and_norm_buffer_adds_pairs() {
        let mut buf = FreqAndNormBuffer::new();
        buf.add(1, 10);
        buf.add(2, 20);
        assert_eq!(buf.size, 2);
        assert_eq!(buf.freqs[..2], [1, 2]);
        assert_eq!(buf.norms[..2], [10, 20]);
    }
}
