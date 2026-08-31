//! `FieldReader` ported from `org.apache.lucene.codecs.lucene103.blocktree`.
//!
//! One field's view of the block-tree terms dictionary: the per-field statistics
//! read from the `.tmd` file, plus the file pointers into the `.tim` and `.tip`
//! files that a terms cursor starts from.

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::terms::{Terms, TermsEnum};
use crate::index::{FieldInfo, IndexOptions};
use crate::store::IndexInput;
use crate::util::BytesRef;

/// The shared state a [`FieldReader`] needs to open a cursor: the two files the
/// dictionary lives in, and the postings reader that decodes each term's
/// metadata.
///
/// **Divergence from Lucene 10.5.0.** Java's `FieldReader` holds a reference to
/// its `Lucene103BlockTreeTermsReader` parent and reaches the shared inputs
/// through it. Rust cannot hold that back-reference without a cycle, so the
/// shared parts are extracted into this struct and shared by `Arc`.
pub struct BlockTreeShared {
    /// The `.tim` file, holding the term blocks.
    pub terms_in: Arc<dyn IndexInput>,
    /// The `.tip` file, holding the term index tries.
    pub index_in: Arc<dyn IndexInput>,
    /// The segment this dictionary belongs to.
    pub segment: String,
    /// Format version of the three files.
    pub version: i32,
}

impl std::fmt::Debug for BlockTreeShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockTreeShared")
            .field("segment", &self.segment)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// One field of the block-tree terms dictionary.
///
/// Equivalent to `org.apache.lucene.codecs.lucene103.blocktree.FieldReader`.
#[derive(Clone)]
pub struct FieldReader {
    shared: Arc<BlockTreeShared>,
    field_info: FieldInfo,
    num_terms: i64,
    sum_total_term_freq: i64,
    sum_doc_freq: i64,
    doc_count: i32,
    min_term: BytesRef,
    max_term: BytesRef,
    /// Offset in the `.tip` file where this field's trie starts.
    index_start: i64,
    /// Offset of the trie root, relative to `index_start`.
    root_fp: i64,
    /// Offset in the `.tip` file where this field's trie ends.
    index_end: i64,
}

impl std::fmt::Debug for FieldReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FieldReader")
            .field("field", &self.field_info.name)
            .field("num_terms", &self.num_terms)
            .field("doc_count", &self.doc_count)
            .finish_non_exhaustive()
    }
}

impl FieldReader {
    /// Builds the field's view, reading its three index pointers from `meta_in`.
    ///
    /// Equivalent to the `FieldReader` constructor, which reads `indexStart`,
    /// `rootFP` and `indexEnd` in that order.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        shared: Arc<BlockTreeShared>,
        field_info: FieldInfo,
        num_terms: i64,
        sum_total_term_freq: i64,
        sum_doc_freq: i64,
        doc_count: i32,
        min_term: BytesRef,
        max_term: BytesRef,
        meta_in: &mut dyn crate::store::DataInput,
    ) -> Result<Self> {
        if num_terms <= 0 {
            return Err(LuceneError::corrupt_index(
                format!(
                    "illegal numTerms for field {}: {num_terms}",
                    field_info.name
                ),
                &shared.segment,
            ));
        }
        let index_start = meta_in.read_v_long()?;
        let root_fp = meta_in.read_v_long()?;
        let index_end = meta_in.read_v_long()?;

        Ok(Self {
            shared,
            field_info,
            num_terms,
            sum_total_term_freq,
            sum_doc_freq,
            doc_count,
            min_term,
            max_term,
            index_start,
            root_fp,
            index_end,
        })
    }

    /// Returns the field's metadata.
    pub fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    /// Returns the shared dictionary state.
    pub fn shared(&self) -> &Arc<BlockTreeShared> {
        &self.shared
    }

    /// Returns the offset in the `.tip` file where this field's trie starts.
    pub fn index_start(&self) -> i64 {
        self.index_start
    }

    /// Returns the offset of the trie root, relative to `index_start`.
    pub fn root_fp(&self) -> i64 {
        self.root_fp
    }

    /// Returns the offset in the `.tip` file where this field's trie ends.
    pub fn index_end(&self) -> i64 {
        self.index_end
    }

    /// Returns the smallest term of the field.
    ///
    /// Equivalent to `FieldReader.getMin()`.
    pub fn get_min(&self) -> &BytesRef {
        &self.min_term
    }

    /// Returns the largest term of the field.
    ///
    /// Equivalent to `FieldReader.getMax()`.
    pub fn get_max(&self) -> &BytesRef {
        &self.max_term
    }
}

impl Terms for FieldReader {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(
            crate::codecs::lucene103::segment_terms_enum::SegmentTermsEnum::new(self.clone())?,
        ))
    }

    fn intersect(
        &self,
        compiled: &crate::util::automaton::CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        use crate::util::automaton::AutomatonType;
        if compiled.automaton_type != AutomatonType::Normal {
            return Err(LuceneError::IllegalArgument(
                "please use CompiledAutomaton.get_terms_enum instead".to_string(),
            ));
        }
        let automaton = compiled.automaton.clone().ok_or_else(|| {
            LuceneError::IllegalArgument("compiled automaton has no transitions".to_string())
        })?;
        let run_automaton = compiled.run_automaton.clone().ok_or_else(|| {
            LuceneError::IllegalArgument("compiled automaton has no runnable form".to_string())
        })?;
        Ok(Box::new(
            crate::codecs::lucene103::intersect_terms_enum::IntersectTermsEnum::new(
                self.clone(),
                Box::new(automaton),
                Box::new(run_automaton),
                compiled.common_suffix_ref.clone(),
                start_term.cloned(),
            )?,
        ))
    }

    fn size(&self) -> i64 {
        self.num_terms
    }

    fn sum_total_term_freq(&self) -> i64 {
        self.sum_total_term_freq
    }

    fn sum_doc_freq(&self) -> i64 {
        self.sum_doc_freq
    }

    fn doc_count(&self) -> i32 {
        self.doc_count
    }

    fn has_freqs(&self) -> bool {
        self.field_info
            .index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS)
    }

    fn has_offsets(&self) -> bool {
        self.field_info
            .index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
    }

    fn has_positions(&self) -> bool {
        self.field_info
            .index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
    }

    fn has_payloads(&self) -> bool {
        self.field_info.has_payloads()
    }

    fn min(&self) -> Result<Option<BytesRef>> {
        Ok(Some(self.min_term.clone()))
    }

    fn max(&self) -> Result<Option<BytesRef>> {
        Ok(Some(self.max_term.clone()))
    }
}
