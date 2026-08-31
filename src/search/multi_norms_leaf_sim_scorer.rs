//! Scoring across the norms of several fields, ported from
//! `org.apache.lucene.search.MultiNormsLeafSimScorer`.

#![deny(unsafe_code)]

use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock};

use crate::error::Result;
use crate::index::{DocAndFloatFeatureBuffer, DocValuesIterator, LeafReader, NumericDocValues};
use crate::search::combined_field_query::FieldAndWeight;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::sim_scorer_source::{SharedSimScorer, SharedSimScorerRef};
use crate::search::similarities::{Explanation, SimScorer};
use crate::util::{FixedBitSet, SmallFloat};

/// The cache of decoded norms.
///
/// Equivalent to the `private static final float[] LENGTH_TABLE` of
/// `MultiNormsLeafSimScorer`, which Java fills in a static initialiser.
static LENGTH_TABLE: LazyLock<[f32; 256]> = LazyLock::new(|| {
    let mut table = [0f32; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        *entry = SmallFloat::byte4_to_int(i as u8) as f32;
    }
    table
});

/// A scorer that sums a document's norms across several fields.
///
/// Equivalent to the `final org.apache.lucene.search.MultiNormsLeafSimScorer`,
/// which is package-private in Java; it is public here because Rust has no
/// package visibility and [`CombinedFieldQuery`](crate::search::CombinedFieldQuery)
/// lives in a sibling module.
///
/// For all fields, norms must be encoded with
/// [`SmallFloat::int_to_byte4`]. This scorer also requires that either all
/// fields or no fields have norms enabled; having only some fields with norms
/// enabled can result in errors or undefined behaviour.
pub struct MultiNormsLeafSimScorer {
    scorer: SharedSimScorer,
    norms: Option<MultiFieldNormValues>,
    norm_values: Vec<i64>,
    score_spare: Vec<f32>,
}

impl std::fmt::Debug for MultiNormsLeafSimScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiNormsLeafSimScorer")
            .field("has_norms", &self.norms.is_some())
            .finish_non_exhaustive()
    }
}

impl MultiNormsLeafSimScorer {
    /// Sole constructor: scores the documents of `reader` with `scorer`.
    ///
    /// Equivalent to
    /// `MultiNormsLeafSimScorer(SimScorer, LeafReader, Collection<FieldAndWeight>, boolean)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java holds the `BulkSimScorer` the
    /// similarity scorer produces in a field; this port's
    /// `SimScorer::as_bulk_sim_scorer` borrows the scorer, which a field cannot
    /// hold beside it, so the bulk scorer is built on each
    /// [`score_range`](Self::score_range) call — the same adaptation
    /// [`MaxScoreCache`](crate::search::MaxScoreCache) makes. The scores
    /// produced are identical.
    ///
    /// # Panics
    ///
    /// In debug builds, panics when a field appears twice in `norm_fields`,
    /// which Java asserts.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the norms.
    pub fn new(
        scorer: SharedSimScorer,
        reader: &Arc<dyn LeafReader>,
        norm_fields: &[FieldAndWeight],
        needs_scores: bool,
    ) -> Result<Self> {
        let norms = if needs_scores {
            let mut norms_list: Vec<Box<dyn NumericDocValues>> = Vec::new();
            let mut weight_list: Vec<f32> = Vec::new();
            let mut duplicate_checking_set: BTreeSet<&str> = BTreeSet::new();
            for field in norm_fields {
                debug_assert!(
                    duplicate_checking_set.insert(field.field()),
                    "There is a duplicated field [{}] used to construct MultiNormsLeafSimScorer",
                    field.field()
                );
                if let Some(norms) = reader.get_norm_values(field.field())? {
                    norms_list.push(norms);
                    weight_list.push(field.weight());
                }
            }
            if norms_list.is_empty() {
                None
            } else {
                Some(MultiFieldNormValues {
                    norms_arr: norms_list,
                    weight_arr: weight_list,
                    acc_buf: Vec::new(),
                    current: 0,
                    doc_id: -1,
                })
            }
        } else {
            None
        };
        Ok(Self {
            scorer,
            norms,
            norm_values: Vec::new(),
            score_spare: Vec::new(),
        })
    }

    /// Returns the similarity scorer.
    ///
    /// Equivalent to the package-private
    /// `MultiNormsLeafSimScorer.getSimScorer()`.
    pub fn get_sim_scorer(&self) -> &SharedSimScorer {
        &self.scorer
    }

    /// Equivalent to the private
    /// `MultiNormsLeafSimScorer.getNormValue(int)`.
    fn get_norm_value(&mut self, doc: i32) -> Result<i64> {
        match self.norms.as_mut() {
            Some(norms) => {
                let found = norms.advance_exact(doc)?;
                debug_assert!(found);
                norms.long_value()
            }
            // The default norm.
            None => Ok(1),
        }
    }

    /// Scores the provided document, assuming the given term document
    /// frequency.
    ///
    /// Equivalent to `MultiNormsLeafSimScorer.score(int, float)`; see
    /// [`SimScorer::score`]. It must be called on non-decreasing sequences of
    /// doc IDs.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the norms.
    pub fn score(&mut self, doc: i32, freq: f32) -> Result<f32> {
        let norm = self.get_norm_value(doc)?;
        Ok(self.scorer.score(freq, norm))
    }

    /// Scores the documents contained in `buffer`, whose float feature store is
    /// assumed to be the frequency.
    ///
    /// Equivalent to
    /// `MultiNormsLeafSimScorer.scoreRange(DocAndFloatFeatureBuffer)`; see
    /// [`SimScorer::score`].
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the norms.
    pub fn score_range(&mut self, buffer: &mut DocAndFloatFeatureBuffer) -> Result<()> {
        let size = buffer.size;
        if self.norm_values.len() < size {
            self.norm_values.resize(size, 1);
        }
        match self.norms.as_mut() {
            Some(norms) => {
                norms.long_values(size as i32, &buffer.docs, 0, &mut self.norm_values, 0, 1)?;
            }
            None => {
                self.norm_values[..size].fill(1);
            }
        }
        if self.score_spare.len() < size {
            self.score_spare.resize(size, 0.0);
        }
        // Java passes `buffer.features` as both the frequencies and the
        // destination; this port's `BulkSimScorer` takes distinct slices, so the
        // scores are computed into a spare buffer and copied back. The values
        // are identical: Java reads every frequency before writing the score at
        // the same index.
        let scorer = SharedSimScorerRef::new(Arc::clone(&self.scorer));
        {
            let mut bulk = scorer.as_bulk_sim_scorer();
            bulk.score(
                size,
                &buffer.features,
                &self.norm_values,
                &mut self.score_spare,
            );
        }
        buffer.features[..size].copy_from_slice(&self.score_spare[..size]);
        Ok(())
    }

    /// Explains the score of the provided document, assuming the given term
    /// document frequency.
    ///
    /// Equivalent to
    /// `MultiNormsLeafSimScorer.explain(int, Explanation)`; see
    /// [`SimScorer::explain`]. It must be called on non-decreasing sequences of
    /// doc IDs.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the norms.
    pub fn explain(&mut self, doc: i32, freq_expl: &Explanation) -> Result<Explanation> {
        let norm = self.get_norm_value(doc)?;
        Ok(self.scorer.explain(freq_expl, norm))
    }
}

/// The weighted sum of the norms of several fields, seen as one
/// [`NumericDocValues`].
///
/// Equivalent to the private static
/// `MultiNormsLeafSimScorer.MultiFieldNormValues`.
struct MultiFieldNormValues {
    norms_arr: Vec<Box<dyn NumericDocValues>>,
    weight_arr: Vec<f32>,
    acc_buf: Vec<f32>,
    current: i64,
    doc_id: i32,
}

impl std::fmt::Debug for MultiFieldNormValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiFieldNormValues")
            .field("fields", &self.norms_arr.len())
            .finish_non_exhaustive()
    }
}

/// Encodes a summed norm the way `SmallFloat.intToByte4(Math.round(value))`
/// does.
///
/// `Math.round(float)` is `floor(value + 0.5)` and saturates at
/// [`i32::MIN`]/[`i32::MAX`]; `SmallFloat.intToByte4` rejects a negative
/// argument, which cannot arise here because every weight and every decoded
/// norm is positive.
fn encode_norm(value: f32) -> i64 {
    let rounded = java_math_round(value);
    i64::from(SmallFloat::int_to_byte4(rounded.max(0)).unwrap_or(0))
}

/// Java's `Math.round(float)`.
fn java_math_round(value: f32) -> i32 {
    if value.is_nan() {
        return 0;
    }
    let rounded = (value + 0.5).floor();
    if rounded >= i32::MAX as f32 {
        i32::MAX
    } else if rounded <= i32::MIN as f32 {
        i32::MIN
    } else {
        rounded as i32
    }
}

impl DocIdSetIterator for MultiFieldNormValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        Err(crate::error::LuceneError::UnsupportedOperation(
            "MultiFieldNormValues cannot be iterated".to_string(),
        ))
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(crate::error::LuceneError::UnsupportedOperation(
            "MultiFieldNormValues cannot be advanced".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        // Java throws `UnsupportedOperationException`; `cost` cannot report an
        // error, and nothing asks a norms view for its cost.
        0
    }

    fn into_bit_set(
        &mut self,
        _up_to: i32,
        _bit_set: &mut FixedBitSet,
        _offset: i32,
    ) -> Result<()> {
        Err(crate::error::LuceneError::UnsupportedOperation(
            "MultiFieldNormValues cannot be iterated".to_string(),
        ))
    }
}

impl DocValuesIterator for MultiFieldNormValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        let mut norm_value = 0f32;
        let mut found = false;
        for i in 0..self.norms_arr.len() {
            if self.norms_arr[i].advance_exact(target)? {
                let encoded = self.norms_arr[i].long_value()? as u8;
                norm_value += self.weight_arr[i] * LENGTH_TABLE[encoded as usize];
                found = true;
            }
        }
        self.current = encode_norm(norm_value);
        self.doc_id = target;
        Ok(found)
    }
}

impl NumericDocValues for MultiFieldNormValues {
    fn long_value(&self) -> Result<i64> {
        Ok(self.current)
    }

    fn long_values(
        &mut self,
        size: i32,
        docs: &[i32],
        docs_offset: i32,
        values: &mut [i64],
        values_offset: i32,
        default_value: i64,
    ) -> Result<()> {
        let size_usize = size.max(0) as usize;
        if self.acc_buf.len() < size_usize {
            self.acc_buf.resize(size_usize, 0.0);
        }
        self.acc_buf[..size_usize].fill(0.0);

        for i in 0..self.norms_arr.len() {
            // This code relies on the assumption that a document length can
            // never be 0, so `0` indicates the absence of a norm value.
            self.norms_arr[i].long_values(size, docs, docs_offset, values, values_offset, 0)?;
            let weight = self.weight_arr[i];
            for j in 0..size_usize {
                let encoded = values[values_offset as usize + j] as u8;
                self.acc_buf[j] += weight * LENGTH_TABLE[encoded as usize];
            }
        }

        for i in 0..size_usize {
            values[values_offset as usize + i] = if self.acc_buf[i] == 0.0 {
                default_value
            } else {
                encode_norm(self.acc_buf[i])
            };
        }
        Ok(())
    }
}
