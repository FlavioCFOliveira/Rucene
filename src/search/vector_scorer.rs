//! Vector similarity scoring, ported from
//! `org.apache.lucene.search.VectorScorer`.
//!
//! # Adaptation: the scorer is shared, not aliased
//!
//! Java's `VectorScorer` hands out its `DocIdSetIterator` with `iterator()`,
//! lets a conjunction drive that iterator, and reads the resulting score back
//! through `score()` on the very same object. Rust forbids that alias, so the
//! scorer lives in a [`SharedVectorScorer`] — an `Rc<RefCell<_>>` handle, the
//! same shape [`SharedPostings`](crate::search::SharedPostings) already uses
//! for the identical problem in the phrase matchers. Both halves see one
//! scorer, positioned exactly where Java positions it.

#![deny(unsafe_code)]

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::Result;
use crate::index::{DocAndFloatFeatureBuffer, DocIndexIterator};
use crate::search::conjunction_utils::ConjunctionUtils;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::util::hnsw::RandomVectorScorer;
use crate::util::Bits;

/// The number of documents one bulk-scoring call fills in.
///
/// Equivalent to `VectorScorer.DEFAULT_BULK_BATCH_SIZE`.
pub const DEFAULT_BULK_BATCH_SIZE: usize = 64;

/// Computes the similarity score between a query vector and the vectors of
/// different documents, for exact searching and scoring.
///
/// Equivalent to the interface `org.apache.lucene.search.VectorScorer`.
pub trait VectorScorer {
    /// Computes the score for the current document ID.
    ///
    /// Equivalent to `VectorScorer.score()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the vector.
    fn score(&mut self) -> Result<f32>;

    /// Returns the iterator over the documents that have a vector.
    ///
    /// Equivalent to `VectorScorer.iterator()`. As in Java the returned
    /// iterator is a *view* on this scorer: advancing it is what moves the
    /// document [`score`](Self::score) answers for. Rust expresses the view as
    /// a borrow of the scorer rather than as an independent object.
    fn iterator(&mut self) -> &mut dyn DocIdSetIterator;

    /// Returns a bulk scorer over the documents present in both this scorer's
    /// iterator and `matching_docs`; `None` scores every document.
    ///
    /// Equivalent to `VectorScorer.bulk(DocIdSetIterator)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java gives this method a body on the
    /// interface. A default body here would have to coerce `Box<Self>` into
    /// `Box<dyn VectorScorer>`, which Rust only allows for a sized `Self`, so
    /// the method is left abstract and the shared body lives in
    /// [`default_bulk`] — every implementation is the one line
    /// `default_bulk(SharedVectorScorer::new(self), matching_docs)`, which is
    /// exactly Java's default.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while positioning the iterators.
    fn bulk(
        self: Box<Self>,
        matching_docs: Option<Box<dyn DocIdSetIterator>>,
    ) -> Result<Box<dyn VectorScorerBulk>>;

    /// Returns this scorer's iterator as a [`DocIndexIterator`] when the vectors
    /// it scores are indexed, and `None` otherwise.
    ///
    /// **Divergence from Lucene 10.5.0.** Java has no such method: callers that
    /// need the vector ordinal behind a document — `SeededKnnVectorQuery` and
    /// `AbstractKnnVectorQuery.ReentrantKnnCollectorManager` — write
    /// `scorer.iterator() instanceof KnnVectorValues.DocIndexIterator`. Rust
    /// cannot downcast a `dyn DocIdSetIterator`, so the test is declared as a
    /// method, the same way [`Scorer::as_scorable`](crate::search::Scorer::as_scorable)
    /// declares an upcast Java gets for free. The default answers `None`, which
    /// is the `false` branch of Java's `instanceof`.
    fn doc_index_iterator(&mut self) -> Option<&mut dyn DocIndexIterator> {
        None
    }
}

/// Scores several vectors at once.
///
/// Equivalent to the nested interface `org.apache.lucene.search.VectorScorer.Bulk`.
pub trait VectorScorerBulk {
    /// Scores doc IDs up to `up_to`, storing the results in `buffer` and
    /// returning the maximum score of the scored documents.
    ///
    /// Equivalent to
    /// `VectorScorer.Bulk.nextDocsAndScores(int, Bits, DocAndFloatFeatureBuffer)`,
    /// and behaves like
    /// [`Scorer::next_docs_and_scores`](crate::search::Scorer::next_docs_and_scores).
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while iterating or scoring.
    fn next_docs_and_scores(
        &mut self,
        up_to: i32,
        live_docs: Option<&dyn Bits>,
        buffer: &mut DocAndFloatFeatureBuffer,
    ) -> Result<f32>;
}

/// A shared handle on a [`VectorScorer`].
///
/// See the module documentation: it exists so that a conjunction can drive the
/// scorer's iterator while the caller reads scores off the same scorer, which
/// is what Java's aliasing does.
#[derive(Clone)]
pub struct SharedVectorScorer {
    inner: Rc<RefCell<Box<dyn VectorScorer>>>,
}

impl SharedVectorScorer {
    /// Takes ownership of `scorer` and shares it.
    pub fn new(scorer: Box<dyn VectorScorer>) -> Self {
        Self {
            inner: Rc::new(RefCell::new(scorer)),
        }
    }

    /// Computes the score for the current document ID.
    ///
    /// Equivalent to `VectorScorer.score()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the vector.
    pub fn score(&self) -> Result<f32> {
        self.inner.borrow_mut().score()
    }

    /// Returns an owned view of the scorer's iterator.
    ///
    /// Equivalent to `VectorScorer.iterator()`.
    pub fn iterator(&self) -> SharedVectorScorerIterator {
        SharedVectorScorerIterator {
            inner: Rc::clone(&self.inner),
        }
    }
}

/// The [`DocIdSetIterator`] view of a [`SharedVectorScorer`].
///
/// Equivalent to the object `VectorScorer.iterator()` returns; every call
/// forwards to the one scorer, so several views agree with each other exactly
/// as Java's single instance does.
pub struct SharedVectorScorerIterator {
    inner: Rc<RefCell<Box<dyn VectorScorer>>>,
}

impl DocIdSetIterator for SharedVectorScorerIterator {
    fn doc_id(&self) -> i32 {
        self.inner.borrow_mut().iterator().doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.borrow_mut().iterator().next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.borrow_mut().iterator().advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.borrow_mut().iterator().cost()
    }
}

/// The body of `VectorScorer.bulk(DocIdSetIterator)`.
///
/// Equivalent to the default method of the Java interface: it intersects
/// `matching_docs` with the scorer's own iterator, positions the result, and
/// returns a bulk scorer over it.
///
/// # Errors
///
/// Propagates any I/O error raised while building or positioning the iterators.
pub fn default_bulk(
    scorer: SharedVectorScorer,
    matching_docs: Option<Box<dyn DocIdSetIterator>>,
) -> Result<Box<dyn VectorScorerBulk>> {
    let mut iterator: Box<dyn DocIdSetIterator> = match matching_docs {
        None => Box::new(scorer.iterator()),
        Some(matching_docs) => {
            ConjunctionUtils::intersect_iterators(vec![matching_docs, Box::new(scorer.iterator())])?
                .into_doc_id_set_iterator()
        }
    };
    if iterator.doc_id() == -1 {
        iterator.next_doc()?;
    }
    Ok(Box::new(DefaultVectorScorerBulk { scorer, iterator }))
}

/// The bulk scorer [`default_bulk`] returns.
struct DefaultVectorScorerBulk {
    scorer: SharedVectorScorer,
    iterator: Box<dyn DocIdSetIterator>,
}

impl VectorScorerBulk for DefaultVectorScorerBulk {
    fn next_docs_and_scores(
        &mut self,
        up_to: i32,
        live_docs: Option<&dyn Bits>,
        buffer: &mut DocAndFloatFeatureBuffer,
    ) -> Result<f32> {
        debug_assert!(up_to > 0);
        buffer.grow_no_copy(DEFAULT_BULK_BATCH_SIZE);
        let mut size = 0usize;
        let mut max_score = f32::NEG_INFINITY;
        let mut doc = self.iterator.doc_id();
        while doc < up_to && size < DEFAULT_BULK_BATCH_SIZE {
            if live_docs.is_none_or_set(doc) {
                buffer.docs[size] = doc;
                buffer.features[size] = self.scorer.score()?;
                max_score = java_max_f32(max_score, buffer.features[size]);
                size += 1;
            }
            doc = self.iterator.next_doc()?;
        }
        buffer.size = size;
        Ok(max_score)
    }
}

/// A shared handle on a [`DocIndexIterator`].
///
/// The sparse bulk scorer reads `index()` off the very iterator a conjunction
/// is advancing, which is the same alias [`SharedVectorScorer`] solves.
#[derive(Clone)]
struct SharedDocIndexIterator {
    inner: Rc<RefCell<Box<dyn DocIndexIterator>>>,
}

impl SharedDocIndexIterator {
    fn new(iterator: Box<dyn DocIndexIterator>) -> Self {
        Self {
            inner: Rc::new(RefCell::new(iterator)),
        }
    }

    fn index(&self) -> i32 {
        self.inner.borrow().index()
    }
}

impl DocIdSetIterator for SharedDocIndexIterator {
    fn doc_id(&self) -> i32 {
        self.inner.borrow().doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.borrow_mut().next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.borrow_mut().advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.borrow().cost()
    }
}

/// Builds a bulk scorer over a dense [`RandomVectorScorer`], where a document
/// ID is its own vector ordinal.
///
/// Equivalent to
/// `VectorScorer.Bulk.fromRandomScorerDense(RandomVectorScorer, KnnVectorValues.DocIndexIterator, DocIdSetIterator)`.
///
/// # Errors
///
/// Propagates any I/O error raised while building the conjunction.
pub fn bulk_from_random_scorer_dense(
    scorer: Box<dyn RandomVectorScorer>,
    iterator: Box<dyn DocIndexIterator>,
    matching_docs: Option<Box<dyn DocIdSetIterator>>,
) -> Result<Box<dyn VectorScorerBulk>> {
    let matches: Box<dyn DocIdSetIterator> = match matching_docs {
        None => iterator,
        Some(matching_docs) => {
            ConjunctionUtils::intersect_iterators(vec![matching_docs, iterator])?
                .into_doc_id_set_iterator()
        }
    };
    Ok(Box::new(DenseRandomScorerBulk { scorer, matches }))
}

struct DenseRandomScorerBulk {
    scorer: Box<dyn RandomVectorScorer>,
    matches: Box<dyn DocIdSetIterator>,
}

impl VectorScorerBulk for DenseRandomScorerBulk {
    fn next_docs_and_scores(
        &mut self,
        up_to: i32,
        live_docs: Option<&dyn Bits>,
        buffer: &mut DocAndFloatFeatureBuffer,
    ) -> Result<f32> {
        debug_assert!(up_to > 0);
        if self.matches.doc_id() == -1 {
            self.matches.next_doc()?;
        }
        buffer.grow_no_copy(DEFAULT_BULK_BATCH_SIZE);
        let mut size = 0usize;
        let mut doc = self.matches.doc_id();
        while doc < up_to && size < DEFAULT_BULK_BATCH_SIZE {
            if live_docs.is_none_or_set(doc) {
                buffer.docs[size] = doc;
                size += 1;
            }
            doc = self.matches.next_doc()?;
        }
        buffer.size = size;
        self.scorer
            .bulk_score(&buffer.docs, &mut buffer.features, size as i32)
    }
}

/// Builds a bulk scorer over a sparse [`RandomVectorScorer`], where a document
/// ID must be translated into a vector ordinal.
///
/// Equivalent to
/// `VectorScorer.Bulk.fromRandomScorerSparse(RandomVectorScorer, KnnVectorValues.DocIndexIterator, DocIdSetIterator)`.
///
/// # Errors
///
/// Propagates any I/O error raised while building the conjunction.
pub fn bulk_from_random_scorer_sparse(
    scorer: Box<dyn RandomVectorScorer>,
    iterator: Box<dyn DocIndexIterator>,
    matching_docs: Option<Box<dyn DocIdSetIterator>>,
) -> Result<Box<dyn VectorScorerBulk>> {
    let shared = SharedDocIndexIterator::new(iterator);
    let matches: Box<dyn DocIdSetIterator> = match matching_docs {
        None => Box::new(shared.clone()),
        Some(matching_docs) => {
            ConjunctionUtils::intersect_iterators(vec![matching_docs, Box::new(shared.clone())])?
                .into_doc_id_set_iterator()
        }
    };
    Ok(Box::new(SparseRandomScorerBulk {
        scorer,
        iterator: shared,
        matches,
        doc_ids: Vec::new(),
    }))
}

struct SparseRandomScorerBulk {
    scorer: Box<dyn RandomVectorScorer>,
    iterator: SharedDocIndexIterator,
    matches: Box<dyn DocIdSetIterator>,
    doc_ids: Vec<i32>,
}

impl VectorScorerBulk for SparseRandomScorerBulk {
    fn next_docs_and_scores(
        &mut self,
        up_to: i32,
        live_docs: Option<&dyn Bits>,
        buffer: &mut DocAndFloatFeatureBuffer,
    ) -> Result<f32> {
        debug_assert!(up_to > 0);
        if self.matches.doc_id() == -1 {
            self.matches.next_doc()?;
        }
        buffer.grow_no_copy(DEFAULT_BULK_BATCH_SIZE);
        if self.doc_ids.len() < DEFAULT_BULK_BATCH_SIZE {
            // Equivalent to `ArrayUtil.growNoCopy(int[], int)`: the previous
            // contents are never read again, only the first `size` entries
            // written below are.
            self.doc_ids = vec![0; DEFAULT_BULK_BATCH_SIZE];
        }
        let mut size = 0usize;
        let mut doc = self.matches.doc_id();
        while doc < up_to && size < DEFAULT_BULK_BATCH_SIZE {
            if live_docs.is_none_or_set(doc) {
                buffer.docs[size] = self.iterator.index();
                self.doc_ids[size] = doc;
                size += 1;
            }
            doc = self.matches.next_doc()?;
        }
        buffer.size = size;
        let max_score = self
            .scorer
            .bulk_score(&buffer.docs, &mut buffer.features, size as i32)?;
        // Copy back the real doc IDs.
        buffer.docs[..size].copy_from_slice(&self.doc_ids[..size]);
        Ok(max_score)
    }
}

/// `java.lang.Math.max(float, float)`, which propagates `NaN` where
/// [`f32::max`] would discard it.
fn java_max_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a > b {
        a
    } else if a < b {
        b
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_positive() {
            a
        } else {
            b
        }
    } else {
        a
    }
}

/// Answers Java's `liveDocs == null || liveDocs.get(doc)` on an optional
/// [`Bits`].
trait LiveDocsExt {
    fn is_none_or_set(&self, doc: i32) -> bool;
}

impl LiveDocsExt for Option<&dyn Bits> {
    fn is_none_or_set(&self, doc: i32) -> bool {
        match self {
            None => true,
            Some(bits) => bits.get(doc as usize),
        }
    }
}
