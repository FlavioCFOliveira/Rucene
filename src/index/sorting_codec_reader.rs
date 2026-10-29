//! `SortingCodecReader`, `SlowCompositeCodecReaderWrapper` and the
//! merged-segment warmer, ported from `org.apache.lucene.index`.

use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::codecs::doc_values::DocValuesProducer;
use crate::codecs::knn_vectors::KnnVectorsReader;
use crate::codecs::norms::NormsProducer;
use crate::codecs::points::PointsReader;
use crate::codecs::postings::FieldsProducer;
use crate::codecs::stored_fields::StoredFieldsReader;
use crate::codecs::term_vectors::TermVectorsReader;
use crate::error::{LuceneError, Result};
use crate::index::codec_reader::{final_members, CodecReader};
use crate::index::doc_values::{DocValues, OrdinalMap};
use crate::index::freq_prox_terms_writer::SortingTerms;
use crate::index::index_reader::{CacheHelper, IndexReader, IndexReaderCore, StoredFields};
use crate::index::index_sorter::SorterDocMap;
use crate::index::leaf_reader::{LeafMetaData, LeafReader, TermVectors};
use crate::index::multi_bits::MultiBits;
use crate::index::multi_doc_values::{
    MultiDocValues, MultiSortedDocValues, MultiSortedSetDocValues,
};
use crate::index::multi_fields::MultiFields;
use crate::index::multi_reader::{reader_util, MultiReader, ReaderSlice};
use crate::index::{
    BinaryDocValues, ByteVectorValues, DocIndexIterator, DocValuesIterator, DocValuesSkipper,
    FieldInfo, FieldInfos, Fields, FloatVectorValues, IntersectVisitor, KnnVectorValues,
    NumericDocValues, PointTree, PointValues, Relation, SortedDocValues, SortedNumericDocValues,
    SortedSetDocValues, StoredFieldVisitor, StoredFieldVisitorStatus, Terms,
};
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::knn::KnnCollector;
use crate::search::{AcceptDocs, Sort};
use crate::util::{BitSet, BitSetIterator, Bits, BytesRef, FixedBitSet, InfoStream};

/// Presents a segment's documents in a sorted order.
///
/// Equivalent to `org.apache.lucene.index.SortingCodecReader`, which index
/// sorting wraps a segment in so a flush or a merge writes its documents in the
/// index sort order.
///
/// Every producer Java wraps is wrapped here, so that it answers in the new
/// document order: the postings (through `FreqProxTermsWriter.SortingTerms`,
/// `SortingCodecReader.java:448-484`), the stored fields and term vectors
/// (`:486-522`, `:734-769`), the live docs (`:524-532`), the points
/// (`SortingPointValues`, `:71-206`, `:534-560`), the vectors
/// (`SortingFloatVectorValues`/`SortingByteVectorValues`, `:312-384`,
/// `:562-606`), the norms and the doc values (`:608-719`). The last
/// materialised doc-values or norms view is cached exactly as Java caches it —
/// see [`SortingCodecReaderState::get_or_create`].
///
/// Java's `SortingCodecReader` extends `FilterCodecReader`, so its leaf view —
/// `terms`, the doc-values and norms getters, `getPointValues`, the vector
/// getters, `storedFields`, `termVectors` — is `CodecReader`'s `final` one,
/// which reads through the wrapped producers above
/// (`CodecReader.java:86-290`). The [`LeafReader`] members here forward to the
/// same bodies in [`final_members`].
pub struct SortingCodecReader {
    inner: Arc<dyn CodecReader>,
    doc_map: Arc<SorterDocMap>,
    /// Equivalent to `SortingCodecReader.metaData`: the wrapped reader's
    /// metadata with the sort `wrap` was given (`SortingCodecReader.java:398-402`).
    meta_data: LeafMetaData,
    /// The half of the enclosing instance Java's anonymous producers reach
    /// through: the index sort, and the one-entry cache of the last sorted
    /// view built.
    state: Arc<SortingCodecReaderState>,
}

impl std::fmt::Debug for SortingCodecReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingCodecReader").finish_non_exhaustive()
    }
}

impl std::fmt::Display for SortingCodecReader {
    /// Port of `SortingCodecReader.toString()`
    /// (`SortingCodecReader.java:771-774`). Measured against
    /// `lucene-core-10.5.0.jar`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SortingCodecReader({})", self.inner)
    }
}

impl SortingCodecReader {
    /// Wraps `inner` so its documents appear in the order `doc_map` gives, and
    /// reports `sort` as its index sort.
    ///
    /// Equivalent to `SortingCodecReader.wrap(CodecReader, Sorter.DocMap, Sort)`
    /// (`SortingCodecReader.java:396-433`).
    ///
    /// **Divergence from Lucene 10.5.0.** Java accepts a `null` doc map, meaning
    /// the reader is already sorted, and then returns a plain
    /// `FilterCodecReader` that only replaces the metadata and drops the cache
    /// helpers (`SortingCodecReader.java:403-425`). `doc_map` is not optional
    /// here: no caller of this port holds a reader that is already in order
    /// when it wraps, so that branch has no counterpart.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when `inner.max_doc()` differs from `doc_map.size()`, and propagates
    /// whatever building the new metadata raises.
    pub fn wrap(
        inner: Arc<dyn CodecReader>,
        doc_map: Arc<SorterDocMap>,
        sort: Option<Sort>,
    ) -> Result<Self> {
        let meta_data = inner.get_meta_data();
        let meta_data = LeafMetaData::new(
            meta_data.created_version_major(),
            meta_data.min_version(),
            sort,
            meta_data.has_blocks(),
        )?;
        if inner.max_doc() != doc_map.size() {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "reader.maxDoc() should be equal to docMap.size(), got {} != {}",
                inner.max_doc(),
                doc_map.size()
            )));
        }
        let state = Arc::new(SortingCodecReaderState {
            sort: meta_data.sort().cloned(),
            cache: Mutex::new(SortedViewCache::default()),
        });
        Ok(Self {
            inner,
            doc_map,
            meta_data,
            state,
        })
    }

    /// Returns the doc map this reader applies.
    pub fn doc_map(&self) -> &Arc<SorterDocMap> {
        &self.doc_map
    }

    /// Returns the wrapped reader.
    pub fn get_delegate(&self) -> &Arc<dyn CodecReader> {
        &self.inner
    }

    /// Creates a factory for [`SortingValuesIterator`]s over `values`.
    ///
    /// Does the work of computing the (new docId to old ordinal) mapping once
    /// and caches the result, so that further iterators are cheap to create.
    ///
    /// `doc_to_ord` holds zero for documents that have no vector. That is
    /// harmless, exactly as it is in Java, because the iterator can never be
    /// positioned on such a document.
    ///
    /// Equivalent to `org.apache.lucene.index.SortingCodecReader#iteratorSupplier`.
    ///
    /// # Errors
    ///
    /// Propagates whatever taking or advancing `values`' iterator raises.
    pub fn iterator_supplier(
        values: &dyn KnnVectorValues,
        doc_map: &SorterDocMap,
    ) -> Result<SortingIteratorSupplier> {
        let mut doc_to_ord = vec![0i32; doc_map.size() as usize];
        let mut doc_bits = FixedBitSet::new(doc_map.size() as usize);
        let mut count = 0i32;
        let mut iter = values.iterator()?;
        loop {
            let doc = iter.next_doc()?;
            if doc == NO_MORE_DOCS {
                break;
            }
            let new_doc_id = doc_map.old_to_new(doc);
            if new_doc_id != -1 {
                doc_to_ord[new_doc_id as usize] = iter.index();
                doc_bits.set(new_doc_id as usize);
                count += 1;
            }
        }
        Ok(SortingIteratorSupplier {
            doc_bits: Arc::new(doc_bits),
            doc_to_ord: Arc::new(doc_to_ord),
            size: count,
        })
    }

    /// Wraps `delegate` so that every document number handed to it is mapped
    /// back to the underlying segment's numbering first.
    ///
    /// Equivalent to `org.apache.lucene.index.SortingCodecReader#newStoredFieldsReader`.
    /// Java's wrapper is an anonymous class, so its `clone()` has to call this
    /// method again; here the wrapper is a named type and
    /// [`StoredFieldsReader::clone_reader`] builds it directly, which is the
    /// same construction.
    fn new_stored_fields_reader(
        &self,
        delegate: Box<dyn StoredFieldsReader>,
    ) -> Box<dyn StoredFieldsReader> {
        Box::new(SortingStoredFieldsReader {
            delegate,
            doc_map: Arc::clone(&self.doc_map),
        })
    }

    /// Wraps `delegate` so that every document number handed to it is mapped
    /// back to the underlying segment's numbering first, answering `None` when
    /// the segment has no term vectors.
    ///
    /// Equivalent to `org.apache.lucene.index.SortingCodecReader#newTermVectorsReader`,
    /// which takes a possibly-null delegate and returns null for it.
    fn new_term_vectors_reader(
        &self,
        delegate: Option<Box<dyn TermVectorsReader>>,
    ) -> Option<Box<dyn TermVectorsReader>> {
        delegate.map(|delegate| {
            Box::new(SortingTermVectorsReader {
                delegate,
                doc_map: Arc::clone(&self.doc_map),
            }) as Box<dyn TermVectorsReader>
        })
    }
}

impl LeafReader for SortingCodecReader {
    fn core(&self) -> &IndexReaderCore {
        self.inner.core()
    }

    /// `CodecReader.termVectors()`, over this reader's sorted term vectors.
    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        final_members::term_vectors(self)
    }

    /// Equivalent to `FilterCodecReader.numDocs()`.
    fn num_docs(&self) -> i32 {
        self.inner.num_docs()
    }

    /// Equivalent to `FilterCodecReader.maxDoc()`.
    fn max_doc(&self) -> i32 {
        self.inner.max_doc()
    }

    /// `CodecReader.storedFields()`, over this reader's sorted stored fields.
    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        final_members::stored_fields(self)
    }

    /// Equivalent to `FilterCodecReader.doClose()`.
    fn do_close(&self) -> Result<()> {
        self.inner.do_close()
    }

    /// Equivalent to `SortingCodecReader.getReaderCacheHelper()`: no caching on
    /// sorted views (`SortingCodecReader.java:782-785`).
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    /// Equivalent to `SortingCodecReader.getCoreCacheHelper()`
    /// (`SortingCodecReader.java:777-780`).
    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        final_members::terms(self, field)
    }

    fn get_numeric_doc_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        final_members::numeric_doc_values(self, field)
    }

    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        final_members::binary_doc_values(self, field)
    }

    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        final_members::sorted_doc_values(self, field)
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        final_members::sorted_numeric_doc_values(self, field)
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        final_members::sorted_set_doc_values(self, field)
    }

    fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        final_members::norm_values(self, field)
    }

    fn get_doc_values_skipper(&self, field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        final_members::doc_values_skipper(self, field)
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        final_members::float_vector_values(self, field)
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        final_members::byte_vector_values(self, field)
    }

    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        final_members::search_nearest_vectors(self, field, target, collector, accept_docs)
    }

    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        final_members::search_nearest_vectors_byte(self, field, target, collector, accept_docs)
    }

    /// Equivalent to `FilterCodecReader.getFieldInfos()`.
    fn get_field_infos(&self) -> FieldInfos {
        self.inner.get_field_infos()
    }

    /// Equivalent to `SortingCodecReader.getLiveDocs()`
    /// (`SortingCodecReader.java:524-532`), which wraps the delegate's live
    /// docs in `SortingBits`.
    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.inner.get_live_docs().map(|in_live_docs| {
            Box::new(SortingBits {
                inner: in_live_docs,
                doc_map: Arc::clone(&self.doc_map),
            }) as Box<dyn Bits>
        })
    }

    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        final_members::point_values(self, field)
    }

    /// Equivalent to `FilterCodecReader.checkIntegrity()`.
    fn check_integrity(&self) -> Result<()> {
        self.inner.check_integrity()
    }

    /// Equivalent to `SortingCodecReader.getMetaData()`
    /// (`SortingCodecReader.java:787-790`).
    fn get_meta_data(&self) -> LeafMetaData {
        self.meta_data.clone()
    }
}

impl CodecReader for SortingCodecReader {
    /// Equivalent to `SortingCodecReader.getFieldsReader()`
    /// (`SortingCodecReader.java:486-493`).
    fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        match self.inner.get_fields_reader()? {
            None => Ok(None),
            Some(delegate) => Ok(Some(self.new_stored_fields_reader(delegate))),
        }
    }

    /// Equivalent to `SortingCodecReader.getTermVectorsReader()`
    /// (`SortingCodecReader.java:734-737`).
    fn get_term_vectors_reader(&self) -> Result<Option<Box<dyn TermVectorsReader>>> {
        Ok(self.new_term_vectors_reader(self.inner.get_term_vectors_reader()?))
    }

    /// Equivalent to `SortingCodecReader.getNormsReader()`
    /// (`SortingCodecReader.java:608-631`).
    fn get_norms_reader(&self) -> Result<Option<Arc<dyn NormsProducer>>> {
        match self.inner.get_norms_reader()? {
            None => Ok(None),
            Some(delegate) => Ok(Some(Arc::new(SortingNormsProducer {
                delegate,
                doc_map: Arc::clone(&self.doc_map),
                max_doc: LeafReader::max_doc(self),
                state: Arc::clone(&self.state),
            }))),
        }
    }

    /// Equivalent to `SortingCodecReader.getDocValuesReader()`
    /// (`SortingCodecReader.java:633-719`).
    fn get_doc_values_reader(&self) -> Result<Option<Arc<dyn DocValuesProducer>>> {
        match self.inner.get_doc_values_reader()? {
            None => Ok(None),
            Some(delegate) => Ok(Some(Arc::new(SortingDocValuesProducer {
                delegate,
                doc_map: Arc::clone(&self.doc_map),
                max_doc: LeafReader::max_doc(self),
                state: Arc::clone(&self.state),
            }))),
        }
    }

    /// Equivalent to `SortingCodecReader.getPostingsReader()`
    /// (`SortingCodecReader.java:448-484`).
    fn get_postings_reader(&self) -> Result<Option<Arc<dyn FieldsProducer>>> {
        match self.inner.get_postings_reader()? {
            None => Ok(None),
            Some(postings_reader) => Ok(Some(Arc::new(SortingFieldsProducer {
                postings_reader,
                field_infos: self.inner.get_field_infos(),
                doc_map: Arc::clone(&self.doc_map),
            }))),
        }
    }

    /// Equivalent to `SortingCodecReader.getPointsReader()`
    /// (`SortingCodecReader.java:534-560`).
    fn get_points_reader(&self) -> Result<Option<Arc<dyn PointsReader>>> {
        match self.inner.get_points_reader()? {
            None => Ok(None),
            Some(delegate) => Ok(Some(Arc::new(SortingPointsReader {
                delegate,
                doc_map: Arc::clone(&self.doc_map),
            }))),
        }
    }

    /// Equivalent to `SortingCodecReader.getVectorReader()`
    /// (`SortingCodecReader.java:562-606`).
    fn get_vector_reader(&self) -> Result<Option<Arc<dyn KnnVectorsReader>>> {
        match self.inner.get_vector_reader()? {
            None => Ok(None),
            Some(delegate) => Ok(Some(Arc::new(SortingKnnVectorsReader {
                delegate,
                doc_map: Arc::clone(&self.doc_map),
            }))),
        }
    }
}

// -----------------------------------------------------------------------------
// Iteration over reordered vector values
// -----------------------------------------------------------------------------

/// Factory for [`SortingValuesIterator`]s. This enables new iterators to be
/// created as needed without recomputing the sorting mappings.
///
/// Equivalent to `org.apache.lucene.index.SortingCodecReader.SortingIteratorSupplier`,
/// which implements `java.util.function.Supplier<SortingValuesIterator>`.
#[derive(Debug, Clone)]
pub struct SortingIteratorSupplier {
    doc_bits: Arc<FixedBitSet>,
    doc_to_ord: Arc<Vec<i32>>,
    size: i32,
}

impl SortingIteratorSupplier {
    /// Returns a fresh iterator over the mapping this factory holds.
    ///
    /// Equivalent to `SortingIteratorSupplier.get()`.
    pub fn get(&self) -> SortingValuesIterator {
        SortingValuesIterator::new(
            Arc::clone(&self.doc_bits),
            Arc::clone(&self.doc_to_ord),
            self.size,
        )
    }

    /// Returns how many documents have a vector.
    ///
    /// Equivalent to `SortingIteratorSupplier.size()`.
    pub fn size(&self) -> i32 {
        self.size
    }
}

/// Iterator over `KnnVectorValues` accepting a mapping to differently-sorted
/// docs. Consequently [`index`](DocIndexIterator::index) may skip around, not
/// increasing monotonically as iteration proceeds.
///
/// Equivalent to `org.apache.lucene.index.SortingCodecReader.SortingValuesIterator`.
pub struct SortingValuesIterator {
    doc_bits: Arc<FixedBitSet>,
    docs_with_values: BitSetIterator,
    doc_to_ord: Arc<Vec<i32>>,
    doc: i32,
}

impl std::fmt::Debug for SortingValuesIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingValuesIterator")
            .field("doc", &self.doc)
            .finish_non_exhaustive()
    }
}

impl SortingValuesIterator {
    /// Equivalent to `SortingValuesIterator(FixedBitSet, int[], int)`.
    fn new(doc_bits: Arc<FixedBitSet>, doc_to_ord: Arc<Vec<i32>>, size: i32) -> Self {
        let bits: Arc<dyn BitSet> = doc_bits.clone();
        let docs_with_values = BitSetIterator::new(bits, i64::from(size))
            .expect("INVARIANT: size counts the documents that have a vector, so it is >= 0");
        Self {
            doc_bits,
            docs_with_values,
            doc_to_ord,
            doc: -1,
        }
    }
}

impl DocIdSetIterator for SortingValuesIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.doc != NO_MORE_DOCS {
            self.doc = self.docs_with_values.next_doc()?;
        }
        Ok(self.doc)
    }

    /// Equivalent to `SortingValuesIterator.advance(int)`, which throws
    /// `UnsupportedOperationException`.
    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(crate::error::LuceneError::UnsupportedOperation(
            "SortingValuesIterator::advance".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.doc_bits.cardinality() as i64
    }
}

impl DocIndexIterator for SortingValuesIterator {
    fn index(&self) -> i32 {
        debug_assert!(self.doc_bits.get(self.doc as usize));
        self.doc_to_ord[self.doc as usize]
    }
}

// -----------------------------------------------------------------------------
// The sorted views' one-entry cache
// -----------------------------------------------------------------------------

/// The one-entry cache of the last materialised doc-values or norms view.
///
/// Equivalent to the `cachedField`, `cachedObject`, `cacheIsNorms` and
/// `cacheStats` fields of `org.apache.lucene.index.SortingCodecReader`
/// (`SortingCodecReader.java:792-816`). Java caches the last instance because
/// a merge pulls the same field more than once, and materialising a reordered
/// view is expensive.
#[derive(Default)]
struct SortedViewCache {
    cached_field: Option<String>,
    cached_object: Option<Arc<dyn Any + Send + Sync>>,
    cache_is_norms: bool,
    /// How many times a view was built for each `field` / `norms` pair. Only
    /// ever touched from [`SortingCodecReaderState::assert_created_only_once`],
    /// which runs only with debug assertions enabled — as in Java, where the
    /// map is only reached from an `assert` statement.
    cache_stats: HashMap<String, i32>,
}

/// The state Java's `SortingCodecReader` holds on behalf of the producers it
/// hands out: the index sort, and the cache of the last sorted view built.
///
/// **Divergence from Lucene 10.5.0.** Java's doc-values and norms producers
/// are anonymous inner classes of `SortingCodecReader`
/// (`SortingCodecReader.java:608-719`) and reach `getOrCreateDV`,
/// `getOrCreateNorms` and `metaData` through their enclosing instance. Rust
/// has neither inner classes nor a way to lend a producer a borrow of the
/// reader that outlives the call which created it, so the shared half of the
/// enclosing instance lives in this type and the reader and every producer it
/// creates hold the same [`Arc`] to it. The cache is therefore still exactly
/// one per reader, which is what makes the caching correct.
struct SortingCodecReaderState {
    /// The index sort, read only by [`Self::assert_created_only_once`].
    ///
    /// Equivalent to Java's `metaData.sort()`: the sort of the new
    /// `LeafMetaData` that `wrap(CodecReader, Sorter.DocMap, Sort)` builds
    /// around the sort it is given (`SortingCodecReader.java:398-402`).
    sort: Option<Sort>,
    /// Java declares `getOrCreate` `synchronized`; this mutex is that monitor.
    cache: Mutex<SortedViewCache>,
}

impl SortingCodecReaderState {
    /// Returns the cached norms view for `field`, building it with `supplier`
    /// when the cache holds something else.
    ///
    /// Equivalent to `org.apache.lucene.index.SortingCodecReader#getOrCreateNorms`.
    ///
    /// # Errors
    ///
    /// Propagates whatever `supplier` raises.
    fn get_or_create_norms<T, F>(&self, field: &str, supplier: F) -> Result<Arc<T>>
    where
        T: Any + Send + Sync,
        F: FnOnce() -> Result<T>,
    {
        self.get_or_create(field, true, supplier)
    }

    /// Returns the cached doc-values view for `field`, building it with
    /// `supplier` when the cache holds something else.
    ///
    /// Equivalent to `org.apache.lucene.index.SortingCodecReader#getOrCreateDV`.
    ///
    /// # Errors
    ///
    /// Propagates whatever `supplier` raises.
    fn get_or_create_dv<T, F>(&self, field: &str, supplier: F) -> Result<Arc<T>>
    where
        T: Any + Send + Sync,
        F: FnOnce() -> Result<T>,
    {
        self.get_or_create(field, false, supplier)
    }

    /// Returns the cached view for `field`, building it with `supplier` when
    /// the single cache slot holds a different field, or the same field in the
    /// other role.
    ///
    /// Equivalent to `org.apache.lucene.index.SortingCodecReader#getOrCreate`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java caches an `Object` and casts it
    /// back with an unchecked cast, which throws `ClassCastException` if the
    /// slot ever held another type for the same key. This port stores an
    /// `Arc<dyn Any + Send + Sync>` and downcasts it, turning that same
    /// impossible case into an
    /// [`IllegalState`](crate::error::LuceneError::IllegalState) error rather
    /// than a panic. It cannot happen for either caller: a field has exactly
    /// one doc-values type, and norms are cached under a distinct key.
    ///
    /// # Errors
    ///
    /// Propagates whatever `supplier` raises, and returns
    /// [`LuceneError::IllegalState`](crate::error::LuceneError::IllegalState)
    /// if the cached view is not of the requested type.
    fn get_or_create<T, F>(&self, field: &str, norms: bool, supplier: F) -> Result<Arc<T>>
    where
        T: Any + Send + Sync,
        F: FnOnce() -> Result<T>,
    {
        // Java has no lock poisoning; recovering the guard keeps a panic in one
        // caller from turning every later merge into a different failure.
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !(cache.cached_field.as_deref() == Some(field) && cache.cache_is_norms == norms) {
            debug_assert!(self.assert_created_only_once(&mut cache, field, norms));
            let object: Arc<dyn Any + Send + Sync> = Arc::new(supplier()?);
            cache.cached_object = Some(object);
            cache.cached_field = Some(field.to_string());
            cache.cache_is_norms = norms;
        }
        let cached = Arc::clone(
            cache
                .cached_object
                .as_ref()
                .expect("INVARIANT: the slot was just filled unless it already held this field"),
        );
        cached.downcast::<T>().map_err(|_| {
            crate::error::LuceneError::illegal_state(format!(
                "the cached sorted view of field \"{field}\" has a different type"
            ))
        })
    }

    /// Records that a sorted view was built for `field`, and checks Lucene's
    /// invariant that only an index-sort field is ever built twice.
    ///
    /// Always returns `true`, so that the call site can wrap it in a
    /// `debug_assert!` and drop it from a release build, exactly as Java's
    /// `assert assertCreatedOnlyOnce(field, norms)` disappears without `-ea`.
    ///
    /// Equivalent to `org.apache.lucene.index.SortingCodecReader#assertCreatedOnlyOnce`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java opens with
    /// `assert Thread.holdsLock(this)` (`SortingCodecReader.java:819`). Rust
    /// cannot ask a `Mutex` whether the current thread holds it, so the locked
    /// cache is passed in instead: only a caller that has already taken the
    /// lock can produce the `&mut SortedViewCache` this needs, which is the
    /// same guarantee moved from run time to compile time.
    fn assert_created_only_once(
        &self,
        cache: &mut SortedViewCache,
        field: &str,
        norms: bool,
    ) -> bool {
        // this is mainly there to make sure we change anything in the way we
        // merge we realize it early
        let times_cached = {
            let counter = cache
                .cache_stats
                .entry(format!("{field}N:{norms}"))
                .or_insert(0);
            *counter += 1;
            *counter
        };
        if times_cached > 1 {
            assert!(!norms, "[{field}] norms must not be cached twice");
            let mut is_sort_field = false;
            // For things that aren't sort fields, it's possible for sort to be
            // null here. In the event that we accidentally cache twice, its
            // better not to throw an NPE
            if let Some(sort) = &self.sort {
                for sort_field in sort.fields() {
                    if sort_field.field() == Some(field) {
                        is_sort_field = true;
                        break;
                    }
                }
            }
            assert!(
                times_cached == 2,
                "[{field}] must not be cached more than twice but was cached: {times_cached} \
                 times isSortField: {is_sort_field}"
            );
            assert!(
                is_sort_field,
                "only sort fields should be cached twice but [{field}] is not a sort field"
            );
        }
        true
    }
}

/// A very simple merged-segment warmer that just ensures the data structures
/// of each kind the segment holds are initialized.
///
/// Equivalent to `org.apache.lucene.index.SimpleMergedSegmentWarmer`.
pub struct SimpleMergedSegmentWarmer {
    info_stream: Arc<dyn InfoStream>,
}

impl SimpleMergedSegmentWarmer {
    /// Creates the warmer, which logs statistics about warming to
    /// `info_stream`.
    ///
    /// Equivalent to `SimpleMergedSegmentWarmer(InfoStream)`.
    pub fn new(info_stream: Arc<dyn InfoStream>) -> Self {
        Self { info_stream }
    }
}

impl std::fmt::Debug for SimpleMergedSegmentWarmer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleMergedSegmentWarmer")
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for SimpleMergedSegmentWarmer {
    /// `SimpleMergedSegmentWarmer` declares no `toString()`, so Java falls back
    /// to `Object.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.SimpleMergedSegmentWarmer",
            self,
        ))
    }
}

impl crate::index::index_writer_config::MergedSegmentWarmer for SimpleMergedSegmentWarmer {
    /// Touches the terms and norms of every indexed field, the doc values of
    /// every doc-values field, and the stored fields and term vectors of the
    /// first document.
    ///
    /// Equivalent to `SimpleMergedSegmentWarmer.warm(LeafReader)`
    /// (`SimpleMergedSegmentWarmer.java:35-90`).
    fn warm(&self, reader: &dyn LeafReader) -> Result<()> {
        let start_time = crate::index::merge_scheduler::current_time_millis();
        let mut indexed_count = 0;
        let mut doc_values_count = 0;
        let mut norms_count = 0;
        for info in reader.get_field_infos().iter() {
            if info.index_options != crate::index::IndexOptions::NONE {
                reader.terms(&info.name)?;
                indexed_count += 1;

                if info.has_norms() {
                    reader.get_norm_values(&info.name)?;
                    norms_count += 1;
                }
            }

            if info.doc_values_type != crate::index::DocValuesType::NONE {
                match info.doc_values_type {
                    crate::index::DocValuesType::NUMERIC => {
                        reader.get_numeric_doc_values(&info.name)?;
                    }
                    crate::index::DocValuesType::BINARY => {
                        reader.get_binary_doc_values(&info.name)?;
                    }
                    crate::index::DocValuesType::SORTED => {
                        reader.get_sorted_doc_values(&info.name)?;
                    }
                    crate::index::DocValuesType::SORTED_NUMERIC => {
                        reader.get_sorted_numeric_doc_values(&info.name)?;
                    }
                    crate::index::DocValuesType::SORTED_SET => {
                        reader.get_sorted_set_doc_values(&info.name)?;
                    }
                    // Unreachable: the enclosing test excludes it. Java's
                    // `default` arm asserts the same.
                    crate::index::DocValuesType::NONE => {}
                }
                doc_values_count += 1;
            }
        }

        reader.stored_fields()?.document(0)?;
        reader.term_vectors()?.get(0)?;

        if self.info_stream.is_enabled("SMSW") {
            self.info_stream.message(
                "SMSW",
                &format!(
                    "Finished warming segment: {reader}, indexed={indexed_count}, docValues={doc_values_count}, norms={norms_count}, time={}",
                    crate::index::merge_scheduler::current_time_millis() - start_time
                ),
            );
        }
        Ok(())
    }
}

/// A merged [`CodecReader`] view of several codec readers, concatenating their
/// documents. The view is primarily targeted at merging, not searching.
///
/// Equivalent to `org.apache.lucene.index.SlowCompositeCodecReaderWrapper`,
/// which `IndexWriter` hands to `OneMerge.reorder` and to `addIndexes` so that
/// a merge reads several segments as if they were one
/// (`IndexWriter.java:3550`, `:5342`).
///
/// Each producer accessor builds a producer over the matching producers of the
/// wrapped readers, as Java does: stored fields and term vectors by document
/// range (`SlowCompositeCodecReaderWrapper.java:132-262`), norms and doc values
/// through `MultiDocValues` (`:264-292`, `:340-462`), postings through
/// `MultiFields` (`:464-517`), points as a one-node tree over every reader's
/// tree (`:519-788`), and vectors as a concatenation of the readers' values
/// (`:790-1050`). The leaf view — `terms`, the doc-values getters,
/// `getPointValues`, `storedFields` and the rest — is `CodecReader`'s `final`
/// one, which reads through those producers (`CodecReader.java:86-290`); the
/// [`LeafReader`] members here forward to the same bodies in
/// [`final_members`].
///
/// Where `MultiDocValues` answers `null` — no wrapped reader has the field with
/// the requested type — the norms and doc-values producers answer `None`, as
/// Java's return that `null`.
///
/// **Divergence from Lucene 10.5.0.** Java's producers `close()` the producers
/// of the wrapped readers they hold (`IOUtils.close(producers)`). The norms,
/// doc-values, postings, points and vectors producers of a wrapped reader are
/// [`Arc`]s it shares with every other holder and closes itself, so the
/// producers built here release nothing on `close`; the stored-fields and
/// term-vectors readers, which each wrapped reader hands out as a fresh
/// instance, are closed as in Java.
pub struct SlowCompositeCodecReaderWrapper {
    /// This reader's own reference count and identity: Java's wrapper is an
    /// `IndexReader` of its own, distinct from every wrapped reader.
    core: IndexReaderCore,
    /// Equivalent to `SlowCompositeCodecReaderWrapper.meta`.
    meta: LeafMetaData,
    /// Equivalent to `SlowCompositeCodecReaderWrapper.codecReaders`.
    readers: Vec<Arc<dyn CodecReader>>,
    /// Equivalent to `SlowCompositeCodecReaderWrapper.docStarts`: the first
    /// document number of each reader, plus a trailing `maxDoc`.
    doc_starts: Arc<Vec<i32>>,
    /// Equivalent to `SlowCompositeCodecReaderWrapper.fieldInfos`
    /// (`SlowCompositeCodecReaderWrapper.java:95`).
    field_infos: FieldInfos,
    /// Equivalent to `SlowCompositeCodecReaderWrapper.liveDocs`
    /// (`SlowCompositeCodecReaderWrapper.java:96`).
    live_docs: Option<Arc<dyn Bits>>,
    /// The `MultiReader` over the wrapped readers.
    ///
    /// Java builds one in the constructor for the field infos and the live docs
    /// (`SlowCompositeCodecReaderWrapper.java:94-96`), and a fresh one each
    /// time its norms or doc-values producer asks `MultiDocValues` for a
    /// field. Every one of them holds the same readers in the same order, so
    /// this one is shared with the producers instead.
    multi_reader: Arc<dyn IndexReader>,
    /// Equivalent to `SlowCompositeCodecReaderWrapper.numDocs`, computed on
    /// first use (`SlowCompositeCodecReaderWrapper.java:1068-1079`).
    num_docs: OnceLock<i32>,
}

impl std::fmt::Debug for SlowCompositeCodecReaderWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlowCompositeCodecReaderWrapper")
            .field("readers", &self.readers.len())
            .field("doc_starts", &self.doc_starts)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for SlowCompositeCodecReaderWrapper {
    /// `org.apache.lucene.index.SlowCompositeCodecReaderWrapper` declares no
    /// `toString()`, so `Object`'s applies. The binary name was measured from
    /// `lucene-core-10.5.0.jar`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.SlowCompositeCodecReaderWrapper",
            self,
        ))
    }
}

impl SlowCompositeCodecReaderWrapper {
    /// Wraps `readers`, returning the single reader unchanged when there is one.
    ///
    /// Equivalent to `SlowCompositeCodecReaderWrapper.wrap(List<CodecReader>)`
    /// (`SlowCompositeCodecReaderWrapper.java:50-59`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `readers` is empty or the
    /// readers were created by different major versions, and propagates
    /// whatever merging the readers' field infos raises.
    pub fn wrap(readers: Vec<Arc<dyn CodecReader>>) -> Result<Arc<dyn CodecReader>> {
        match readers.len() {
            0 => Err(LuceneError::IllegalArgument(
                "Must take at least one reader, got 0".to_string(),
            )),
            1 => Ok(Arc::clone(&readers[0])),
            _ => Ok(Arc::new(Self::new(readers)?)),
        }
    }

    /// Equivalent to `SlowCompositeCodecReaderWrapper(List<CodecReader>)`
    /// (`SlowCompositeCodecReaderWrapper.java:67-97`).
    fn new(readers: Vec<Arc<dyn CodecReader>>) -> Result<Self> {
        let mut doc_starts = vec![0i32; readers.len() + 1];
        let mut doc_start = 0i32;
        for (i, reader) in readers.iter().enumerate() {
            doc_start += reader.max_doc();
            doc_starts[i + 1] = doc_start;
        }

        let mut major_version = -1;
        let mut min_version: Option<crate::util::Version> = None;
        let mut has_blocks = false;
        for reader in &readers {
            let reader_meta = reader.get_meta_data();
            if major_version == -1 {
                major_version = reader_meta.created_version_major();
            } else if major_version != reader_meta.created_version_major() {
                return Err(LuceneError::IllegalArgument(
                    "Cannot combine leaf readers created with different major versions".to_string(),
                ));
            }
            match (min_version.as_ref(), reader_meta.min_version()) {
                (None, reader_min) => min_version = reader_min,
                (Some(current), Some(reader_min)) => {
                    if current.on_or_after(&reader_min) {
                        min_version = Some(reader_min);
                    }
                }
                // Java's `minVersion.onOrAfter(null)` dereferences null.
                (Some(_), None) => {
                    return Err(LuceneError::illegal_state(
                        "cannot combine a reader that has no minimum version with one that has"
                            .to_string(),
                    ))
                }
            }
            has_blocks |= reader_meta.has_blocks();
        }
        let meta = LeafMetaData::new(major_version, min_version, None, has_blocks)?;

        let multi_reader: Arc<dyn IndexReader> = Arc::new(MultiReader::new(
            readers
                .iter()
                .map(|reader| Arc::clone(reader).as_index_reader())
                .collect(),
            true,
        )?);
        let field_infos = FieldInfos::get_merged_field_infos(Arc::clone(&multi_reader))?;
        let live_docs = MultiBits::get_live_docs(&multi_reader).map(Arc::<dyn Bits>::from);

        Ok(Self {
            core: IndexReaderCore::new(),
            meta,
            readers,
            doc_starts: Arc::new(doc_starts),
            field_infos,
            live_docs,
            multi_reader,
            num_docs: OnceLock::new(),
        })
    }

    /// Returns the wrapped readers.
    pub fn readers(&self) -> &[Arc<dyn CodecReader>] {
        &self.readers
    }

    /// Returns the first document number of each reader, plus a trailing
    /// `max_doc`.
    pub fn doc_starts(&self) -> &[i32] {
        &self.doc_starts
    }
}

/// Returns the index of the reader that owns `doc`.
///
/// Equivalent to `SlowCompositeCodecReaderWrapper.docIdToReaderId(int)`
/// (`SlowCompositeCodecReaderWrapper.java:99-106`), including Java's
/// `Arrays.binarySearch` choice among equal starts, which skips empty readers.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when `doc` is outside
/// `[0, maxDoc)`, as `Objects.checkIndex` throws.
fn doc_id_to_reader_id(doc_starts: &[i32], doc: i32) -> Result<usize> {
    let max_doc = doc_starts[doc_starts.len() - 1];
    if doc < 0 || doc >= max_doc {
        return Err(LuceneError::IllegalArgument(format!(
            "Index {doc} out of bounds for length {max_doc}"
        )));
    }
    Ok(
        match reader_util::binary_search_range(doc_starts, 0, doc_starts.len(), doc) {
            Ok(reader_id) => reader_id,
            Err(insertion_point) => insertion_point - 1,
        },
    )
}

/// Returns the field info the composite reader carries for `info`'s name.
///
/// Equivalent to `SlowCompositeCodecReaderWrapper.remap(FieldInfo)`
/// (`SlowCompositeCodecReaderWrapper.java:118-122`), so that consumers only
/// see field infos from the composite reader, not from individual leaves.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalState`] when the merged field infos lack the
/// name, where Java would hand the visitor `null`; every field a wrapped
/// reader reports was merged into them, so this cannot happen.
fn remap(field_infos: &FieldInfos, info: &FieldInfo) -> Result<FieldInfo> {
    field_infos.field_info(&info.name).cloned().ok_or_else(|| {
        LuceneError::illegal_state(format!(
            "field \"{}\" is missing from the merged field infos",
            info.name
        ))
    })
}

impl LeafReader for SlowCompositeCodecReaderWrapper {
    fn core(&self) -> &IndexReaderCore {
        &self.core
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        final_members::term_vectors(self)
    }

    /// Equivalent to `SlowCompositeCodecReaderWrapper.numDocs()`
    /// (`SlowCompositeCodecReaderWrapper.java:1068-1079`), which sums the
    /// readers' counts on first use.
    fn num_docs(&self) -> i32 {
        *self
            .num_docs
            .get_or_init(|| self.readers.iter().map(|reader| reader.num_docs()).sum())
    }

    /// Equivalent to `SlowCompositeCodecReaderWrapper.maxDoc()`
    /// (`SlowCompositeCodecReaderWrapper.java:1081-1084`).
    fn max_doc(&self) -> i32 {
        self.doc_starts[self.doc_starts.len() - 1]
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        final_members::stored_fields(self)
    }

    /// Inherited from `CodecReader.doClose()`, which does nothing: the
    /// wrapped readers belong to the caller (`CodecReader.java:291-292`).
    fn do_close(&self) -> Result<()> {
        Ok(())
    }

    /// Equivalent to `SlowCompositeCodecReaderWrapper.getReaderCacheHelper()`
    /// (`SlowCompositeCodecReaderWrapper.java:1086-1089`).
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    /// Equivalent to `SlowCompositeCodecReaderWrapper.getCoreCacheHelper()`
    /// (`SlowCompositeCodecReaderWrapper.java:1048-1051`).
    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        final_members::terms(self, field)
    }

    fn get_numeric_doc_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        final_members::numeric_doc_values(self, field)
    }

    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        final_members::binary_doc_values(self, field)
    }

    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        final_members::sorted_doc_values(self, field)
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        final_members::sorted_numeric_doc_values(self, field)
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        final_members::sorted_set_doc_values(self, field)
    }

    fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        final_members::norm_values(self, field)
    }

    fn get_doc_values_skipper(&self, field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        final_members::doc_values_skipper(self, field)
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        final_members::float_vector_values(self, field)
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        final_members::byte_vector_values(self, field)
    }

    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        final_members::search_nearest_vectors(self, field, target, collector, accept_docs)
    }

    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        final_members::search_nearest_vectors_byte(self, field, target, collector, accept_docs)
    }

    /// Equivalent to `SlowCompositeCodecReaderWrapper.getFieldInfos()`
    /// (`SlowCompositeCodecReaderWrapper.java:1053-1056`): the readers' field
    /// infos, merged.
    fn get_field_infos(&self) -> FieldInfos {
        self.field_infos.clone()
    }

    /// Equivalent to `SlowCompositeCodecReaderWrapper.getLiveDocs()`
    /// (`SlowCompositeCodecReaderWrapper.java:1058-1061`): the readers' live
    /// docs, concatenated by `MultiBits.getLiveDocs`.
    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.live_docs.as_ref().map(|bits| {
            Box::new(crate::index::segment_reader::SharedBits::from_arc(
                Arc::clone(bits),
            )) as Box<dyn Bits>
        })
    }

    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        final_members::point_values(self, field)
    }

    /// Inherited from `CodecReader.checkIntegrity()`
    /// (`CodecReader.java:294-332`), over the composite producers.
    fn check_integrity(&self) -> Result<()> {
        final_members::check_integrity(self)
    }

    /// Equivalent to `SlowCompositeCodecReaderWrapper.getMetaData()`
    /// (`SlowCompositeCodecReaderWrapper.java:1063-1066`).
    fn get_meta_data(&self) -> LeafMetaData {
        self.meta.clone()
    }
}

impl CodecReader for SlowCompositeCodecReaderWrapper {
    /// Equivalent to `SlowCompositeCodecReaderWrapper.getFieldsReader()`
    /// (`SlowCompositeCodecReaderWrapper.java:108-114`).
    fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        let readers = self
            .readers
            .iter()
            .map(|reader| reader.get_fields_reader())
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(Box::new(SlowCompositeStoredFieldsReader {
            readers,
            doc_starts: Arc::clone(&self.doc_starts),
            field_infos: self.field_infos.clone(),
        })))
    }

    /// Equivalent to `SlowCompositeCodecReaderWrapper.getTermVectorsReader()`
    /// (`SlowCompositeCodecReaderWrapper.java:198-205`).
    fn get_term_vectors_reader(&self) -> Result<Option<Box<dyn TermVectorsReader>>> {
        let readers = self
            .readers
            .iter()
            .map(|reader| reader.get_term_vectors_reader())
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(Box::new(SlowCompositeTermVectorsReader {
            readers,
            doc_starts: Arc::clone(&self.doc_starts),
        })))
    }

    /// Equivalent to `SlowCompositeCodecReaderWrapper.getNormsReader()`
    /// (`SlowCompositeCodecReaderWrapper.java:263-266`).
    fn get_norms_reader(&self) -> Result<Option<Arc<dyn NormsProducer>>> {
        let producers = self
            .readers
            .iter()
            .map(|reader| reader.get_norms_reader())
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(Arc::new(SlowCompositeNormsProducer {
            multi_reader: Arc::clone(&self.multi_reader),
            producers,
        })))
    }

    /// Equivalent to `SlowCompositeCodecReaderWrapper.getDocValuesReader()`
    /// (`SlowCompositeCodecReaderWrapper.java:367-370`).
    fn get_doc_values_reader(&self) -> Result<Option<Arc<dyn DocValuesProducer>>> {
        let producers = self
            .readers
            .iter()
            .map(|reader| reader.get_doc_values_reader())
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(Arc::new(SlowCompositeDocValuesProducer {
            readers: self.readers.clone(),
            multi_reader: Arc::clone(&self.multi_reader),
            producers,
            doc_starts: Arc::clone(&self.doc_starts),
            cached_ord_maps: Arc::new(Mutex::new(HashMap::new())),
        })))
    }

    /// Equivalent to `SlowCompositeCodecReaderWrapper.getPostingsReader()`
    /// (`SlowCompositeCodecReaderWrapper.java:464-471`).
    fn get_postings_reader(&self) -> Result<Option<Arc<dyn FieldsProducer>>> {
        let producers = self
            .readers
            .iter()
            .map(|reader| reader.get_postings_reader())
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(Arc::new(SlowCompositeFieldsProducer::new(
            producers,
            &self.doc_starts,
        ))))
    }

    /// Equivalent to `SlowCompositeCodecReaderWrapper.getPointsReader()`
    /// (`SlowCompositeCodecReaderWrapper.java:519-522`).
    fn get_points_reader(&self) -> Result<Option<Arc<dyn PointsReader>>> {
        let points_readers = self
            .readers
            .iter()
            .map(|reader| reader.get_points_reader())
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(Arc::new(SlowCompositePointsReader {
            readers: self.readers.clone(),
            points_readers,
            doc_starts: Arc::clone(&self.doc_starts),
        })))
    }

    /// Equivalent to `SlowCompositeCodecReaderWrapper.getVectorReader()`
    /// (`SlowCompositeCodecReaderWrapper.java:790-793`).
    fn get_vector_reader(&self) -> Result<Option<Arc<dyn KnnVectorsReader>>> {
        let vectors_readers = self
            .readers
            .iter()
            .map(|reader| reader.get_vector_reader())
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(Arc::new(SlowCompositeKnnVectorsReader {
            readers: self.readers.clone(),
            vectors_readers,
            doc_starts: Arc::clone(&self.doc_starts),
        })))
    }
}

// -----------------------------------------------------------------------------
// The composite producers
// -----------------------------------------------------------------------------

/// Stored fields of the composite reader: each document is read from the
/// reader that owns it, under the composite's field infos.
///
/// Equivalent to `SlowCompositeCodecReaderWrapper.SlowCompositeStoredFieldsReaderWrapper`
/// (`SlowCompositeCodecReaderWrapper.java:124-196`).
struct SlowCompositeStoredFieldsReader {
    readers: Vec<Option<Box<dyn StoredFieldsReader>>>,
    doc_starts: Arc<Vec<i32>>,
    /// The enclosing reader's merged field infos, which `remap` reads.
    field_infos: FieldInfos,
}

impl std::fmt::Debug for SlowCompositeStoredFieldsReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlowCompositeStoredFieldsReader")
            .field("readers", &self.readers.len())
            .finish_non_exhaustive()
    }
}

impl SlowCompositeStoredFieldsReader {
    /// The reader that owns `doc_id`, and the document's number within it.
    fn locate(&self, doc_id: i32) -> Result<(&dyn StoredFieldsReader, i32)> {
        let reader_id = doc_id_to_reader_id(&self.doc_starts, doc_id)?;
        let reader = self.readers[reader_id].as_deref().ok_or_else(|| {
            LuceneError::illegal_state(format!(
                "reader {reader_id} of the composite has no stored fields reader"
            ))
        })?;
        Ok((reader, doc_id - self.doc_starts[reader_id]))
    }
}

impl StoredFieldsReader for SlowCompositeStoredFieldsReader {
    /// Equivalent to `SlowCompositeStoredFieldsReaderWrapper.document(int,
    /// StoredFieldVisitor)` (`SlowCompositeCodecReaderWrapper.java:158-195`).
    fn document(&self, doc_id: i32, visitor: &mut dyn StoredFieldVisitor) -> Result<()> {
        let (reader, local_doc) = self.locate(doc_id)?;
        let mut remapping = RemappingStoredFieldVisitor {
            visitor,
            field_infos: &self.field_infos,
        };
        reader.document(local_doc, &mut remapping)
    }

    fn check_integrity(&self) -> Result<()> {
        for reader in self.readers.iter().flatten() {
            reader.check_integrity()?;
        }
        Ok(())
    }

    fn clone_reader(&self) -> Box<dyn StoredFieldsReader> {
        Box::new(SlowCompositeStoredFieldsReader {
            readers: self
                .readers
                .iter()
                .map(|reader| reader.as_ref().map(|reader| reader.clone_reader()))
                .collect(),
            doc_starts: Arc::clone(&self.doc_starts),
            field_infos: self.field_infos.clone(),
        })
    }

    fn prefetch(&self, doc_id: i32) -> Result<()> {
        let (reader, local_doc) = self.locate(doc_id)?;
        reader.prefetch(local_doc)
    }

    /// Equivalent to `IOUtils.close(readers)`: every reader is closed, and the
    /// first failure is reported.
    fn close(&mut self) -> Result<()> {
        close_all(
            self.readers
                .iter_mut()
                .flatten()
                .map(|reader| reader.close()),
        )
    }
}

/// Runs every close in `closes`, returning the first failure.
///
/// Equivalent to `IOUtils.close(Closeable...)`, which closes everything before
/// rethrowing the first exception.
fn close_all(closes: impl Iterator<Item = Result<()>>) -> Result<()> {
    let mut first_error = None;
    for result in closes {
        if let Err(error) = result {
            first_error.get_or_insert(error);
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Hands every field a wrapped reader visits to `visitor` under the
/// composite's field info.
///
/// Equivalent to the anonymous `StoredFieldVisitor` of
/// `SlowCompositeStoredFieldsReaderWrapper.document`
/// (`SlowCompositeCodecReaderWrapper.java:161-194`).
struct RemappingStoredFieldVisitor<'a> {
    visitor: &'a mut dyn StoredFieldVisitor,
    field_infos: &'a FieldInfos,
}

impl StoredFieldVisitor for RemappingStoredFieldVisitor<'_> {
    fn needs_field(&mut self, field_info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
        let info = remap(self.field_infos, field_info)?;
        self.visitor.needs_field(&info)
    }

    fn binary_field(&mut self, field_info: &FieldInfo, value: &[u8]) -> Result<()> {
        let info = remap(self.field_infos, field_info)?;
        self.visitor.binary_field(&info, value)
    }

    fn string_field(&mut self, field_info: &FieldInfo, value: &str) -> Result<()> {
        let info = remap(self.field_infos, field_info)?;
        self.visitor.string_field(&info, value)
    }

    fn int_field(&mut self, field_info: &FieldInfo, value: i32) -> Result<()> {
        let info = remap(self.field_infos, field_info)?;
        self.visitor.int_field(&info, value)
    }

    fn long_field(&mut self, field_info: &FieldInfo, value: i64) -> Result<()> {
        let info = remap(self.field_infos, field_info)?;
        self.visitor.long_field(&info, value)
    }

    fn float_field(&mut self, field_info: &FieldInfo, value: f32) -> Result<()> {
        let info = remap(self.field_infos, field_info)?;
        self.visitor.float_field(&info, value)
    }

    fn double_field(&mut self, field_info: &FieldInfo, value: f64) -> Result<()> {
        let info = remap(self.field_infos, field_info)?;
        self.visitor.double_field(&info, value)
    }
}

/// Term vectors of the composite reader: each document is read from the reader
/// that owns it.
///
/// Equivalent to `SlowCompositeCodecReaderWrapper.SlowCompositeTermVectorsReaderWrapper`
/// (`SlowCompositeCodecReaderWrapper.java:207-261`).
struct SlowCompositeTermVectorsReader {
    readers: Vec<Option<Box<dyn TermVectorsReader>>>,
    doc_starts: Arc<Vec<i32>>,
}

impl std::fmt::Debug for SlowCompositeTermVectorsReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlowCompositeTermVectorsReader")
            .field("readers", &self.readers.len())
            .finish_non_exhaustive()
    }
}

impl TermVectors for SlowCompositeTermVectorsReader {
    /// Equivalent to `SlowCompositeTermVectorsReaderWrapper.prefetch(int)`
    /// (`SlowCompositeCodecReaderWrapper.java:241-248`).
    fn prefetch(&mut self, doc_id: i32) -> Result<()> {
        let reader_id = doc_id_to_reader_id(&self.doc_starts, doc_id)?;
        let local_doc = doc_id - self.doc_starts[reader_id];
        match self.readers[reader_id].as_mut() {
            Some(reader) => reader.prefetch(local_doc),
            None => Ok(()),
        }
    }

    /// Equivalent to `SlowCompositeTermVectorsReaderWrapper.get(int)`
    /// (`SlowCompositeCodecReaderWrapper.java:250-258`).
    fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
        let reader_id = doc_id_to_reader_id(&self.doc_starts, doc)?;
        match self.readers[reader_id].as_ref() {
            Some(reader) => reader.get(doc - self.doc_starts[reader_id]),
            None => Ok(None),
        }
    }
}

impl TermVectorsReader for SlowCompositeTermVectorsReader {
    fn check_integrity(&self) -> Result<()> {
        for reader in self.readers.iter().flatten() {
            reader.check_integrity()?;
        }
        Ok(())
    }

    fn clone_reader(&self) -> Box<dyn TermVectorsReader> {
        Box::new(SlowCompositeTermVectorsReader {
            readers: self
                .readers
                .iter()
                .map(|reader| reader.as_ref().map(|reader| reader.clone_reader()))
                .collect(),
            doc_starts: Arc::clone(&self.doc_starts),
        })
    }

    /// Equivalent to `IOUtils.close(readers)`.
    fn close(&mut self) -> Result<()> {
        close_all(
            self.readers
                .iter_mut()
                .flatten()
                .map(|reader| reader.close()),
        )
    }
}

/// Norms of the composite reader, merged by `MultiDocValues.getNormValues`.
///
/// Equivalent to `SlowCompositeCodecReaderWrapper.SlowCompositeNormsProducer`
/// (`SlowCompositeCodecReaderWrapper.java:268-292`). The close divergence of
/// [`SlowCompositeCodecReaderWrapper`] applies.
#[derive(Clone)]
struct SlowCompositeNormsProducer {
    multi_reader: Arc<dyn IndexReader>,
    producers: Vec<Option<Arc<dyn NormsProducer>>>,
}

impl std::fmt::Debug for SlowCompositeNormsProducer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlowCompositeNormsProducer")
            .field("producers", &self.producers.len())
            .finish_non_exhaustive()
    }
}

impl NormsProducer for SlowCompositeNormsProducer {
    fn get_norms(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        MultiDocValues::get_norm_values(&self.multi_reader, &field.name)
    }

    fn check_integrity(&self) -> Result<()> {
        for producer in self.producers.iter().flatten() {
            producer.check_integrity()?;
        }
        Ok(())
    }

    /// Inherited from `NormsProducer.getMergeInstance()`, which returns `this`.
    fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(self.clone()))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Doc values of the composite reader, merged by `MultiDocValues`, with the
/// ordinal map of each sorted and sorted-set field cached.
///
/// Equivalent to `SlowCompositeCodecReaderWrapper.SlowCompositeDocValuesProducerWrapper`
/// (`SlowCompositeCodecReaderWrapper.java:372-462`). The close divergence of
/// [`SlowCompositeCodecReaderWrapper`] applies.
#[derive(Clone)]
struct SlowCompositeDocValuesProducer {
    readers: Vec<Arc<dyn CodecReader>>,
    multi_reader: Arc<dyn IndexReader>,
    producers: Vec<Option<Arc<dyn DocValuesProducer>>>,
    doc_starts: Arc<Vec<i32>>,
    /// Equivalent to `cachedOrdMaps`; shared with the merge instance, which in
    /// Java is the same object.
    cached_ord_maps: Arc<Mutex<HashMap<String, Arc<OrdinalMap>>>>,
}

impl std::fmt::Debug for SlowCompositeDocValuesProducer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlowCompositeDocValuesProducer")
            .field("producers", &self.producers.len())
            .finish_non_exhaustive()
    }
}

impl DocValuesProducer for SlowCompositeDocValuesProducer {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        MultiDocValues::get_numeric_values(&self.multi_reader, &field.name)
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        MultiDocValues::get_binary_values(&self.multi_reader, &field.name)
    }

    /// Equivalent to `SlowCompositeDocValuesProducerWrapper.getSorted(FieldInfo)`
    /// (`SlowCompositeCodecReaderWrapper.java:411-438`).
    fn get_sorted(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        {
            let map = {
                let mut cached = self
                    .cached_ord_maps
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match cached.get(&field.name) {
                    Some(map) => Arc::clone(map),
                    None => {
                        // uncached, or not a multi dv
                        let Some((values, mapping)) =
                            MultiDocValues::get_sorted_values_and_mapping(
                                &self.multi_reader,
                                &field.name,
                            )?
                        else {
                            return Ok(None);
                        };
                        if let Some(mapping) = mapping {
                            cached.insert(field.name.clone(), mapping);
                        }
                        return Ok(Some(values));
                    }
                }
            };
            let mut values: Vec<Box<dyn SortedDocValues>> = Vec::with_capacity(self.readers.len());
            let mut total_cost = 0i64;
            for reader in &self.readers {
                let v = match reader.get_sorted_doc_values(&field.name)? {
                    Some(v) => v,
                    None => Box::new(DocValues::empty_sorted()),
                };
                total_cost += v.cost();
                values.push(v);
            }
            Ok(Some(Box::new(MultiSortedDocValues::new(
                values,
                self.doc_starts.to_vec(),
                map,
                total_cost,
            )?)))
        }
    }

    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        MultiDocValues::get_sorted_numeric_values(&self.multi_reader, &field.name)
    }

    /// Equivalent to `SlowCompositeDocValuesProducerWrapper.getSortedSet(FieldInfo)`
    /// (`SlowCompositeCodecReaderWrapper.java:445-473`).
    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        {
            let map = {
                let mut cached = self
                    .cached_ord_maps
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match cached.get(&field.name) {
                    Some(map) => Arc::clone(map),
                    None => {
                        // uncached, or not a multi dv
                        let Some((values, mapping)) =
                            MultiDocValues::get_sorted_set_values_and_mapping(
                                &self.multi_reader,
                                &field.name,
                            )?
                        else {
                            return Ok(None);
                        };
                        if let Some(mapping) = mapping {
                            cached.insert(field.name.clone(), mapping);
                        }
                        return Ok(Some(values));
                    }
                }
            };
            let mut values: Vec<Box<dyn SortedSetDocValues>> =
                Vec::with_capacity(self.readers.len());
            let mut total_cost = 0i64;
            for reader in &self.readers {
                let v = match reader.get_sorted_set_doc_values(&field.name)? {
                    Some(v) => v,
                    None => Box::new(DocValues::empty_sorted_set()),
                };
                total_cost += v.cost();
                values.push(v);
            }
            Ok(Some(Box::new(MultiSortedSetDocValues::new(
                values,
                self.doc_starts.to_vec(),
                map,
                total_cost,
            )?)))
        }
    }

    /// Equivalent to `SlowCompositeDocValuesProducerWrapper.getSkipper(FieldInfo)`
    /// (`SlowCompositeCodecReaderWrapper.java:475-478`).
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneError::UnsupportedOperation`].
    fn get_skipper(&self, _field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        Err(LuceneError::UnsupportedOperation(
            "This method is for searching not for merging".to_string(),
        ))
    }

    fn check_integrity(&self) -> Result<()> {
        for producer in self.producers.iter().flatten() {
            producer.check_integrity()?;
        }
        Ok(())
    }

    /// Inherited from `DocValuesProducer.getMergeInstance()`, which returns
    /// `this`; the clone shares the ordinal-map cache.
    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(self.clone()))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A codec [`FieldsProducer`] seen as plain [`Fields`], so that it can be one
/// of the subs of a [`MultiFields`].
struct SharedFields(Arc<dyn FieldsProducer>);

impl Fields for SharedFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        self.0.iterator()
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.0.terms(field)
    }

    fn size(&self) -> i32 {
        self.0.size()
    }
}

/// Postings of the composite reader, merged by [`MultiFields`].
///
/// Equivalent to `SlowCompositeCodecReaderWrapper.SlowCompositeFieldsProducerWrapper`
/// (`SlowCompositeCodecReaderWrapper.java:473-517`): the `MultiFields` is built
/// once, in the constructor, and its per-field memo serves every later
/// `terms(field)`. The close divergence of [`SlowCompositeCodecReaderWrapper`]
/// applies.
struct SlowCompositeFieldsProducer {
    producers: Vec<Option<Arc<dyn FieldsProducer>>>,
    /// Equivalent to `SlowCompositeFieldsProducerWrapper.fields`; shared with
    /// the merge instance, which in Java is the same object.
    fields: Arc<MultiFields>,
}

impl SlowCompositeFieldsProducer {
    /// Equivalent to `SlowCompositeFieldsProducerWrapper(FieldsProducer[], int[])`
    /// (`SlowCompositeCodecReaderWrapper.java:477-491`). As in Java, each
    /// slice's second argument is the *end* of the reader's range,
    /// `docStarts[i + 1]`.
    fn new(producers: Vec<Option<Arc<dyn FieldsProducer>>>, doc_starts: &[i32]) -> Self {
        let mut subs: Vec<Box<dyn Fields>> = Vec::new();
        let mut slices = Vec::new();
        for (i, producer) in producers.iter().enumerate() {
            if let Some(producer) = producer {
                subs.push(Box::new(SharedFields(Arc::clone(producer))));
                slices.push(ReaderSlice::new(doc_starts[i], doc_starts[i + 1], i as i32));
            }
        }
        Self {
            producers,
            fields: Arc::new(MultiFields::new(subs, slices)),
        }
    }
}

impl Fields for SlowCompositeFieldsProducer {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        self.fields.iterator()
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.fields.terms(field)
    }

    fn size(&self) -> i32 {
        self.fields.size()
    }
}

impl FieldsProducer for SlowCompositeFieldsProducer {
    fn check_integrity(&self) -> Result<()> {
        for producer in self.producers.iter().flatten() {
            producer.check_integrity()?;
        }
        Ok(())
    }

    /// Inherited from `FieldsProducer.getMergeInstance()`, which returns `this`.
    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(SlowCompositeFieldsProducer {
            producers: self.producers.clone(),
            fields: Arc::clone(&self.fields),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Points of the composite reader: one tree per field, a single node over
/// every wrapped reader's tree.
///
/// Equivalent to `SlowCompositeCodecReaderWrapper.SlowCompositePointsReaderWrapper`
/// (`SlowCompositeCodecReaderWrapper.java:531-788`). The close divergence of
/// [`SlowCompositeCodecReaderWrapper`] applies.
#[derive(Clone)]
struct SlowCompositePointsReader {
    readers: Vec<Arc<dyn CodecReader>>,
    points_readers: Vec<Option<Arc<dyn PointsReader>>>,
    doc_starts: Arc<Vec<i32>>,
}

impl std::fmt::Debug for SlowCompositePointsReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlowCompositePointsReader")
            .field("readers", &self.points_readers.len())
            .finish_non_exhaustive()
    }
}

impl PointsReader for SlowCompositePointsReader {
    fn check_integrity(&self) -> Result<()> {
        for reader in self.points_readers.iter().flatten() {
            reader.check_integrity()?;
        }
        Ok(())
    }

    /// Equivalent to `SlowCompositePointsReaderWrapper.getValues(String)`
    /// (`SlowCompositeCodecReaderWrapper.java:560-582`).
    fn get_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        let mut values = Vec::new();
        for (i, reader) in self.readers.iter().enumerate() {
            let has_points = reader
                .get_field_infos()
                .field_info(field)
                .is_some_and(|fi| fi.point_dimension_count > 0);
            if has_points {
                let points_reader = self.points_readers[i].as_ref().ok_or_else(|| {
                    LuceneError::illegal_state(format!(
                        "reader {i} indexes points for field \"{field}\" but has no points reader"
                    ))
                })?;
                // Apparently FieldInfo can claim a field has points, yet the
                // returned PointValues is null.
                if let Some(v) = points_reader.get_values(field)? {
                    values.push(PointValuesSub {
                        sub: v,
                        doc_base: self.doc_starts[i],
                    });
                }
            }
        }
        if values.is_empty() {
            return Ok(None);
        }
        Ok(Some(Box::new(SlowCompositePointValues {
            values: Arc::new(values),
        })))
    }

    /// Inherited from `PointsReader.getMergeInstance()`, which returns `this`.
    fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(self.clone()))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// One wrapped reader's values of a point field, and its document base.
///
/// Equivalent to `SlowCompositeCodecReaderWrapper.PointValuesSub`
/// (`SlowCompositeCodecReaderWrapper.java:524-529`).
struct PointValuesSub {
    sub: Box<dyn PointValues>,
    doc_base: i32,
}

/// The anonymous `PointValues` `SlowCompositePointsReaderWrapper.getValues`
/// returns (`SlowCompositeCodecReaderWrapper.java:584-786`).
struct SlowCompositePointValues {
    values: Arc<Vec<PointValuesSub>>,
}

impl SlowCompositePointValues {
    /// The composite tree, typed: `getPointTree()`, which only captures the
    /// subs (`SlowCompositeCodecReaderWrapper.java:584-587`).
    fn tree(&self) -> SlowCompositePointTree {
        SlowCompositePointTree::new(Arc::clone(&self.values))
    }
}

impl PointValues for SlowCompositePointValues {
    fn point_tree(&self) -> Result<Box<dyn PointTree>> {
        Ok(Box::new(self.tree()))
    }

    /// Equivalent to `size()` (`SlowCompositeCodecReaderWrapper.java:768-775`):
    /// `getPointTree().size()`.
    fn size(&self) -> Result<i64> {
        self.tree().size()
    }

    /// Equivalent to `getDocCount()` (`SlowCompositeCodecReaderWrapper.java:777-784`).
    fn doc_count(&self) -> Result<i32> {
        let mut doc_count = 0;
        for sub in self.values.iter() {
            doc_count += sub.sub.doc_count()?;
        }
        Ok(doc_count)
    }

    /// Equivalent to `getMinPackedValue()`: `getPointTree().getMinPackedValue()`.
    fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
        Ok(Some(self.tree().min_packed_value()?.to_vec()))
    }

    /// Equivalent to `getMaxPackedValue()`: `getPointTree().getMaxPackedValue()`.
    fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
        Ok(Some(self.tree().max_packed_value()?.to_vec()))
    }

    fn num_dimensions(&self) -> Result<i32> {
        self.values[0].sub.num_dimensions()
    }

    fn num_index_dimensions(&self) -> Result<i32> {
        self.values[0].sub.num_index_dimensions()
    }

    fn bytes_per_dimension(&self) -> Result<i32> {
        self.values[0].sub.bytes_per_dimension()
    }
}

/// The single-node tree over every wrapped reader's tree: it cannot move, and a
/// visit walks each reader's tree in turn, shifting its documents by the
/// reader's base.
///
/// Equivalent to the anonymous `PointTree` of
/// `SlowCompositeCodecReaderWrapper.java:587-752`. Java computes the bounds on
/// each `getMinPackedValue()`/`getMaxPackedValue()` call; they are computed on
/// the first call here and kept, because [`PointTree`] lends them out by
/// reference, and the subs they are computed from do not change. `clone()`
/// returns `this` in Java; the tree has no position, so a copy answers
/// identically.
#[derive(Clone)]
struct SlowCompositePointTree {
    values: Arc<Vec<PointValuesSub>>,
    min_packed_value: OnceLock<Vec<u8>>,
    max_packed_value: OnceLock<Vec<u8>>,
}

impl SlowCompositePointTree {
    fn new(values: Arc<Vec<PointValuesSub>>) -> Self {
        Self {
            values,
            min_packed_value: OnceLock::new(),
            max_packed_value: OnceLock::new(),
        }
    }

    /// The bound in `cell`, computed on first use.
    fn cached_bound<'a>(
        &self,
        cell: &'a OnceLock<Vec<u8>>,
        wanted: std::cmp::Ordering,
    ) -> Result<&'a [u8]> {
        if let Some(bound) = cell.get() {
            return Ok(bound);
        }
        let bound = Self::bound(&self.values, wanted)?;
        Ok(cell.get_or_init(|| bound))
    }

    /// The per-dimension minimum (`wanted == Less`) or maximum (`Greater`) of
    /// the subs' bounds.
    ///
    /// Equivalent to the bodies of the tree's `getMinPackedValue()` and
    /// `getMaxPackedValue()` (`SlowCompositeCodecReaderWrapper.java:677-751`).
    fn bound(values: &[PointValuesSub], wanted: std::cmp::Ordering) -> Result<Vec<u8>> {
        let leaf_bound = |sub: &PointValuesSub| -> Result<Vec<u8>> {
            let bound = if wanted == std::cmp::Ordering::Less {
                sub.sub.min_packed_value()?
            } else {
                sub.sub.max_packed_value()?
            };
            bound.ok_or_else(|| {
                LuceneError::illegal_state(
                    "a reader with point values reports no packed bound".to_string(),
                )
            })
        };
        let mut packed: Option<Vec<u8>> = None;
        for sub in values {
            match packed.as_mut() {
                None => packed = Some(leaf_bound(sub)?),
                Some(packed) => {
                    let leaf = leaf_bound(sub)?;
                    let num_index_dimensions = sub.sub.num_index_dimensions()? as usize;
                    let num_bytes_per_dimension = sub.sub.bytes_per_dimension()? as usize;
                    for i in 0..num_index_dimensions {
                        let range = i * num_bytes_per_dimension..(i + 1) * num_bytes_per_dimension;
                        if leaf[range.clone()].cmp(&packed[range.clone()]) == wanted {
                            packed[range.clone()].copy_from_slice(&leaf[range]);
                        }
                    }
                }
            }
        }
        packed.ok_or_else(|| {
            LuceneError::illegal_state("a composite point tree needs a sub".to_string())
        })
    }
}

impl PointTree for SlowCompositePointTree {
    fn clone_tree(&self) -> Result<Box<dyn PointTree>> {
        Ok(Box::new(self.clone()))
    }

    fn move_to_child(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn move_to_sibling(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn move_to_parent(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn min_packed_value(&self) -> Result<&[u8]> {
        self.cached_bound(&self.min_packed_value, std::cmp::Ordering::Less)
    }

    fn max_packed_value(&self) -> Result<&[u8]> {
        self.cached_bound(&self.max_packed_value, std::cmp::Ordering::Greater)
    }

    fn size(&self) -> Result<i64> {
        let mut size = 0i64;
        for sub in self.values.iter() {
            size += sub.sub.size()?;
        }
        Ok(size)
    }

    fn visit_doc_ids(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        for sub in self.values.iter() {
            let mut shifted = DocBaseIntersectVisitor {
                visitor: &mut *visitor,
                doc_start: sub.doc_base,
            };
            sub.sub.point_tree()?.visit_doc_ids(&mut shifted)?;
        }
        Ok(())
    }

    fn visit_doc_values(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        for sub in self.values.iter() {
            let mut shifted = DocBaseIntersectVisitor {
                visitor: &mut *visitor,
                doc_start: sub.doc_base,
            };
            sub.sub.point_tree()?.visit_doc_values(&mut shifted)?;
        }
        Ok(())
    }
}

/// Shifts every document a wrapped reader's tree visits by the reader's base.
///
/// Equivalent to the visitor `wrapIntersectVisitor(IntersectVisitor, int)`
/// returns (`SlowCompositeCodecReaderWrapper.java:609-627`), which overrides
/// `visit(int)`, `visit(int, byte[])` and `compare`, and inherits every other
/// member.
struct DocBaseIntersectVisitor<'a> {
    visitor: &'a mut dyn IntersectVisitor,
    doc_start: i32,
}

impl IntersectVisitor for DocBaseIntersectVisitor<'_> {
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        self.visitor.visit(self.doc_start + doc_id)
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        self.visitor
            .visit_with_value(self.doc_start + doc_id, packed_value)
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Result<Relation> {
        self.visitor.compare(min_packed_value, max_packed_value)
    }
}

/// Vectors of the composite reader: each field's values are the wrapped
/// readers' values, concatenated.
///
/// Equivalent to `SlowCompositeCodecReaderWrapper.SlowCompositeKnnVectorsReaderWrapper`
/// (`SlowCompositeCodecReaderWrapper.java:795-1046`). The close divergence of
/// [`SlowCompositeCodecReaderWrapper`] applies.
#[derive(Clone)]
struct SlowCompositeKnnVectorsReader {
    readers: Vec<Arc<dyn CodecReader>>,
    vectors_readers: Vec<Option<Arc<dyn KnnVectorsReader>>>,
    doc_starts: Arc<Vec<i32>>,
}

impl std::fmt::Debug for SlowCompositeKnnVectorsReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlowCompositeKnnVectorsReader")
            .field("readers", &self.vectors_readers.len())
            .finish_non_exhaustive()
    }
}

impl KnnVectorsReader for SlowCompositeKnnVectorsReader {
    fn check_integrity(&self) -> Result<()> {
        for reader in self.vectors_readers.iter().flatten() {
            reader.check_integrity()?;
        }
        Ok(())
    }

    /// Equivalent to `SlowCompositeKnnVectorsReaderWrapper.getFloatVectorValues(String)`
    /// (`SlowCompositeCodecReaderWrapper.java:826-843`).
    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        let mut subs = Vec::with_capacity(self.readers.len());
        let mut dimension = -1;
        let mut size = 0;
        for (i, reader) in self.readers.iter().enumerate() {
            let values = reader.get_float_vector_values(field)?;
            if let Some(values) = values.as_ref() {
                if dimension == -1 {
                    dimension = values.dimension();
                }
            }
            let sub_size = values.as_ref().map_or(0, |values| values.size());
            subs.push(DocValuesSub {
                sub: values,
                doc_start: self.doc_starts[i],
                ord_start: size,
            });
            size += sub_size;
        }
        Ok(Some(Box::new(MergedFloatVectorValues::new(
            dimension, size, subs,
        )?)))
    }

    /// Equivalent to `SlowCompositeKnnVectorsReaderWrapper.getByteVectorValues(String)`
    /// (`SlowCompositeCodecReaderWrapper.java:917-934`).
    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        let mut subs = Vec::with_capacity(self.readers.len());
        let mut dimension = -1;
        let mut size = 0;
        for (i, reader) in self.readers.iter().enumerate() {
            let values = reader.get_byte_vector_values(field)?;
            if let Some(values) = values.as_ref() {
                if dimension == -1 {
                    dimension = values.dimension();
                }
            }
            let sub_size = values.as_ref().map_or(0, |values| values.size());
            subs.push(DocValuesSub {
                sub: values,
                doc_start: self.doc_starts[i],
                ord_start: size,
            });
            size += sub_size;
        }
        Ok(Some(Box::new(MergedByteVectorValues::new(
            dimension, size, subs,
        )?)))
    }

    /// Equivalent to `SlowCompositeKnnVectorsReaderWrapper.search(String,
    /// float[], ...)` (`SlowCompositeCodecReaderWrapper.java:1030-1035`).
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneError::UnsupportedOperation`].
    fn search(
        &self,
        _field: &str,
        _target: &[f32],
        _knn_collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "SlowCompositeKnnVectorsReaderWrapper.search".to_string(),
        ))
    }

    /// Equivalent to `SlowCompositeKnnVectorsReaderWrapper.search(String,
    /// byte[], ...)` (`SlowCompositeCodecReaderWrapper.java:1037-1042`).
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneError::UnsupportedOperation`].
    fn search_byte(
        &self,
        _field: &str,
        _target: &[u8],
        _knn_collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "SlowCompositeKnnVectorsReaderWrapper.search".to_string(),
        ))
    }

    /// Inherited from `KnnVectorsReader.getMergeInstance()`, which returns
    /// `this`.
    fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
        Ok(Box::new(self.clone()))
    }

    /// Equivalent to `SlowCompositeKnnVectorsReaderWrapper.getOffHeapByteSize(FieldInfo)`
    /// (`SlowCompositeCodecReaderWrapper.java:845-852`). Java dereferences
    /// every reader's vectors reader, including a `null` one; a reader without
    /// one contributes nothing here.
    fn get_off_heap_byte_size(&self, field_info: &FieldInfo) -> HashMap<String, i64> {
        let mut map = HashMap::new();
        for reader in self.vectors_readers.iter().flatten() {
            map = crate::codecs::knn_vectors::merge_off_heap_byte_size_maps(
                map,
                reader.get_off_heap_byte_size(field_info),
            );
        }
        map
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// One wrapped reader's vector values, with its document and ordinal bases.
///
/// Equivalent to `SlowCompositeCodecReaderWrapper.DocValuesSub`
/// (`SlowCompositeCodecReaderWrapper.java:294-300`).
struct DocValuesSub<T: ?Sized> {
    sub: Option<Box<T>>,
    doc_start: i32,
    ord_start: i32,
}

/// Iterates the documents of every sub in turn, numbering their vectors with
/// consecutive ordinals.
///
/// Equivalent to `SlowCompositeCodecReaderWrapper.MergedDocIterator`
/// (`SlowCompositeCodecReaderWrapper.java:302-365`).
struct MergedDocIterator<T: ?Sized + KnnVectorValues> {
    subs: Arc<Vec<DocValuesSub<T>>>,
    /// The index of `current` in `subs`; Java's `it` stands just past it.
    current: usize,
    current_iterator: Option<Box<dyn DocIndexIterator>>,
    ord: i32,
    doc: i32,
}

impl<T: ?Sized + KnnVectorValues> MergedDocIterator<T> {
    /// Equivalent to `MergedDocIterator(List<DocValuesSub<T>>)`.
    fn new(subs: Arc<Vec<DocValuesSub<T>>>) -> Result<Self> {
        let current_iterator = Self::iterator_of(&subs[0])?;
        Ok(Self {
            subs,
            current: 0,
            current_iterator,
            ord: -1,
            doc: -1,
        })
    }

    /// Equivalent to `MergedDocIterator.currentIterator()`.
    fn iterator_of(sub: &DocValuesSub<T>) -> Result<Option<Box<dyn DocIndexIterator>>> {
        match sub.sub.as_ref() {
            Some(values) => values.iterator().map(Some),
            None => Ok(None),
        }
    }
}

impl<T: ?Sized + KnnVectorValues> DocIdSetIterator for MergedDocIterator<T> {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    /// Equivalent to `MergedDocIterator.nextDoc()`
    /// (`SlowCompositeCodecReaderWrapper.java:333-349`).
    fn next_doc(&mut self) -> Result<i32> {
        loop {
            if let Some(iterator) = self.current_iterator.as_mut() {
                let next = iterator.next_doc()?;
                if next != NO_MORE_DOCS {
                    self.ord += 1;
                    self.doc = self.subs[self.current].doc_start + next;
                    return Ok(self.doc);
                }
            }
            if self.current + 1 == self.subs.len() {
                self.ord = NO_MORE_DOCS;
                self.doc = NO_MORE_DOCS;
                return Ok(NO_MORE_DOCS);
            }
            self.current += 1;
            self.current_iterator = Self::iterator_of(&self.subs[self.current])?;
            self.ord = self.subs[self.current].ord_start - 1;
        }
    }

    /// Equivalent to `MergedDocIterator.advance(int)`, which throws
    /// `UnsupportedOperationException`.
    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "MergedDocIterator.advance".to_string(),
        ))
    }

    /// Equivalent to `MergedDocIterator.cost()`
    /// (`SlowCompositeCodecReaderWrapper.java:365-368`), which throws the
    /// unchecked `UnsupportedOperationException`. [`DocIdSetIterator::cost`]
    /// returns a plain value, as Java's does, so the unchecked exception is a
    /// panic here.
    fn cost(&self) -> i64 {
        panic!("UnsupportedOperationException: MergedDocIterator.cost()")
    }
}

impl<T: ?Sized + KnnVectorValues> DocIndexIterator for MergedDocIterator<T> {
    fn index(&self) -> i32 {
        self.ord
    }
}

/// A handle onto a values object's one [`MergedDocIterator`].
///
/// Java's `MergedFloatVectorValues.iterator()` and
/// `MergedByteVectorValues.iterator()` return the same `iter` instance on every
/// call (`SlowCompositeCodecReaderWrapper.java:876-879`, `:961-964`), so every
/// caller advances one shared cursor. [`KnnVectorValues::iterator`] returns an
/// owned box, so each call hands out a handle onto that one cursor instead.
struct SharedMergedDocIterator<T: ?Sized + KnnVectorValues>(Arc<Mutex<MergedDocIterator<T>>>);

impl<T: ?Sized + KnnVectorValues> SharedMergedDocIterator<T> {
    fn lock(&self) -> std::sync::MutexGuard<'_, MergedDocIterator<T>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl<T: ?Sized + KnnVectorValues> DocIdSetIterator for SharedMergedDocIterator<T> {
    fn doc_id(&self) -> i32 {
        self.lock().doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.lock().next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.lock().advance(target)
    }

    fn cost(&self) -> i64 {
        self.lock().cost()
    }
}

impl<T: ?Sized + KnnVectorValues> DocIndexIterator for SharedMergedDocIterator<T> {
    fn index(&self) -> i32 {
        self.lock().index()
    }
}

/// The ordinal bounds of each sub, plus a trailing `size`.
///
/// The `starts` array the merged values' constructors build
/// (`SlowCompositeCodecReaderWrapper.java:866-873`).
fn ord_starts<T: ?Sized>(subs: &[DocValuesSub<T>], size: i32) -> Vec<i32> {
    let mut starts: Vec<i32> = subs.iter().map(|sub| sub.ord_start).collect();
    starts.push(size);
    starts
}

/// Returns the index of the sub that holds `ord`, starting from the last one
/// found.
///
/// Equivalent to `SlowCompositeKnnVectorsReaderWrapper.findSub(int, int, int[])`
/// (`SlowCompositeCodecReaderWrapper.java:1004-1013`).
fn find_sub(ord: i32, last_sub_index: usize, starts: &[i32]) -> usize {
    if ord >= starts[last_sub_index] {
        if ord >= starts[last_sub_index + 1] {
            return binary_search_starts(starts, ord, last_sub_index + 1, starts.len());
        }
    } else {
        return binary_search_starts(starts, ord, 0, last_sub_index);
    }
    last_sub_index
}

/// Equivalent to `SlowCompositeKnnVectorsReaderWrapper.binarySearchStarts(int[],
/// int, int, int)` (`SlowCompositeCodecReaderWrapper.java:1015-1028`): the
/// last of a run of equal starts, which skips subs without vectors.
fn binary_search_starts(starts: &[i32], ord: i32, from: usize, to: usize) -> usize {
    match reader_util::binary_search_range(starts, from, to, ord) {
        // subtract one since binarySearch returns an *insertion point*
        Err(insertion_point) => insertion_point - 1,
        Ok(mut pos) => {
            while pos < starts.len() - 1 && starts[pos + 1] == ord {
                // Arrays.binarySearch can return any of a sequence of repeated
                // value but we always want the last one
                pos += 1;
            }
            pos
        }
    }
}

/// The float vectors of every wrapped reader, concatenated.
///
/// Equivalent to `SlowCompositeKnnVectorsReaderWrapper.MergedFloatVectorValues`
/// (`SlowCompositeCodecReaderWrapper.java:854-915`).
struct MergedFloatVectorValues {
    dimension: i32,
    size: i32,
    subs: Arc<Vec<DocValuesSub<dyn FloatVectorValues>>>,
    iter: Arc<Mutex<MergedDocIterator<dyn FloatVectorValues>>>,
    starts: Vec<i32>,
    /// Java's `lastSubIndex`, written by `vectorValue`.
    last_sub_index: AtomicUsize,
}

impl MergedFloatVectorValues {
    fn new(
        dimension: i32,
        size: i32,
        subs: Vec<DocValuesSub<dyn FloatVectorValues>>,
    ) -> Result<Self> {
        let starts = ord_starts(&subs, size);
        let subs = Arc::new(subs);
        let iter = Arc::new(Mutex::new(MergedDocIterator::new(Arc::clone(&subs))?));
        Ok(Self {
            dimension,
            size,
            subs,
            iter,
            starts,
            last_sub_index: AtomicUsize::new(0),
        })
    }

    /// Equivalent to `MergedFloatVectorValues.copy()`
    /// (`SlowCompositeCodecReaderWrapper.java:891-898`).
    fn copy_merged(&self) -> Result<Self> {
        let mut subs_copy = Vec::with_capacity(self.subs.len());
        for sub in self.subs.iter() {
            subs_copy.push(DocValuesSub {
                // `DocValuesSub.copy()` calls `sub.copy()` unconditionally
                // (`SlowCompositeCodecReaderWrapper.java:297-299`), so a reader
                // without vectors makes the copy fail.
                sub: Some(
                    sub.sub
                        .as_ref()
                        .ok_or_else(|| {
                            LuceneError::illegal_state(
                                "cannot copy merged vector values over a reader without vectors"
                                    .to_string(),
                            )
                        })?
                        .copy_float()?,
                ),
                doc_start: sub.doc_start,
                ord_start: sub.ord_start,
            });
        }
        Self::new(self.dimension, self.size, subs_copy)
    }
}

impl KnnVectorValues for MergedFloatVectorValues {
    fn dimension(&self) -> i32 {
        self.dimension
    }

    fn size(&self) -> i32 {
        self.size
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.copy_merged()?))
    }

    fn encoding(&self) -> crate::index::VectorEncoding {
        crate::index::VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(Box::new(SharedMergedDocIterator(Arc::clone(&self.iter))))
    }
}

impl FloatVectorValues for MergedFloatVectorValues {
    /// Equivalent to `MergedFloatVectorValues.vectorValue(int)`
    /// (`SlowCompositeCodecReaderWrapper.java:900-910`): fully random access,
    /// for callers such as `SortingCodecReader` that rely on it.
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        debug_assert!(ord >= 0 && ord < self.size);
        let last = find_sub(
            ord,
            self.last_sub_index.load(Ordering::Relaxed),
            &self.starts,
        );
        // Relaxed: the index is only a search hint, never a synchronisation
        // point; any stale value still yields the right sub.
        self.last_sub_index.store(last, Ordering::Relaxed);
        let sub = &self.subs[last];
        let values = sub.sub.as_ref().ok_or_else(|| {
            LuceneError::illegal_state(format!("ordinal {ord} falls in a sub without vectors"))
        })?;
        values.vector_value(ord - sub.ord_start)
    }

    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(self.copy_merged()?))
    }
}

/// The byte vectors of every wrapped reader, concatenated.
///
/// Equivalent to `SlowCompositeKnnVectorsReaderWrapper.MergedByteVectorValues`
/// (`SlowCompositeCodecReaderWrapper.java:936-1002`).
struct MergedByteVectorValues {
    dimension: i32,
    size: i32,
    subs: Arc<Vec<DocValuesSub<dyn ByteVectorValues>>>,
    iter: Arc<Mutex<MergedDocIterator<dyn ByteVectorValues>>>,
    starts: Vec<i32>,
    /// Java's `lastSubIndex`, written by `vectorValue`.
    last_sub_index: AtomicUsize,
}

impl MergedByteVectorValues {
    fn new(
        dimension: i32,
        size: i32,
        subs: Vec<DocValuesSub<dyn ByteVectorValues>>,
    ) -> Result<Self> {
        let starts = ord_starts(&subs, size);
        let subs = Arc::new(subs);
        let iter = Arc::new(Mutex::new(MergedDocIterator::new(Arc::clone(&subs))?));
        Ok(Self {
            dimension,
            size,
            subs,
            iter,
            starts,
            last_sub_index: AtomicUsize::new(0),
        })
    }

    /// Equivalent to `MergedByteVectorValues.copy()`
    /// (`SlowCompositeCodecReaderWrapper.java:993-1001`).
    fn copy_merged(&self) -> Result<Self> {
        let mut subs_copy = Vec::with_capacity(self.subs.len());
        for sub in self.subs.iter() {
            subs_copy.push(DocValuesSub {
                // `DocValuesSub.copy()` calls `sub.copy()` unconditionally
                // (`SlowCompositeCodecReaderWrapper.java:297-299`), so a reader
                // without vectors makes the copy fail.
                sub: Some(
                    sub.sub
                        .as_ref()
                        .ok_or_else(|| {
                            LuceneError::illegal_state(
                                "cannot copy merged vector values over a reader without vectors"
                                    .to_string(),
                            )
                        })?
                        .copy_byte()?,
                ),
                doc_start: sub.doc_start,
                ord_start: sub.ord_start,
            });
        }
        Self::new(self.dimension, self.size, subs_copy)
    }
}

impl KnnVectorValues for MergedByteVectorValues {
    fn dimension(&self) -> i32 {
        self.dimension
    }

    fn size(&self) -> i32 {
        self.size
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.copy_merged()?))
    }

    fn encoding(&self) -> crate::index::VectorEncoding {
        crate::index::VectorEncoding::BYTE
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(Box::new(SharedMergedDocIterator(Arc::clone(&self.iter))))
    }
}

impl ByteVectorValues for MergedByteVectorValues {
    /// Equivalent to `MergedByteVectorValues.vectorValue(int)`
    /// (`SlowCompositeCodecReaderWrapper.java:981-991`).
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
        debug_assert!(ord >= 0 && ord < self.size);
        let last = find_sub(
            ord,
            self.last_sub_index.load(Ordering::Relaxed),
            &self.starts,
        );
        // Relaxed: the index is only a search hint; see MergedFloatVectorValues.
        self.last_sub_index.store(last, Ordering::Relaxed);
        let sub = &self.subs[last];
        let values = sub.sub.as_ref().ok_or_else(|| {
            LuceneError::illegal_state(format!("ordinal {ord} falls in a sub without vectors"))
        })?;
        values.vector_value(ord - sub.ord_start)
    }

    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        Ok(Box::new(self.copy_merged()?))
    }
}

// -----------------------------------------------------------------------------
// Sorted doc-values wrappers
// -----------------------------------------------------------------------------
//
// Each wrapper eagerly drains the old iterator once, keyed by the *new*
// document number `doc_map.old_to_new` assigns, then serves the
// `DocIdSetIterator`/value contract from that materialised form. This mirrors
// Java's `NumericDocValuesWriter.SortingNumericDocValues` and its four
// siblings, which `SortingCodecReader.getDocValuesReader()` installs — see
// that class's javadoc for why eager materialisation, rather than a lazy
// remap, is unavoidable: a merged docID is not monotonic in the old one, so
// the values cannot be streamed in the new order without buffering them.

/// The next document after `doc_id` that has a value, or `NO_MORE_DOCS`.
///
/// Every `nextDoc()` of Java's sorted views stops at `maxDoc` before looking
/// further — `if (docID + 1 == dvs.maxDoc()) docID = NO_MORE_DOCS`
/// (`NumericDocValuesWriter.java:247-254`), and the same bound on the
/// per-document arrays of the four other views
/// (`BinaryDocValuesWriter.java`, `SortedDocValuesWriter.java`,
/// `SortedNumericDocValuesWriter.java`, `SortedSetDocValuesWriter.java`,
/// each `Sorting*DocValues.nextDoc()`). Asking the bit set past its last bit
/// is out of range.
fn next_doc_with_value(docs_with_field: &FixedBitSet, doc_id: i32) -> i32 {
    if doc_id == NO_MORE_DOCS {
        return NO_MORE_DOCS;
    }
    let target = doc_id + 1;
    if target as usize >= docs_with_field.length() {
        NO_MORE_DOCS
    } else {
        docs_with_field.next_set_bit(target)
    }
}

/// The reordered values of a single-valued numeric field, keyed by the new
/// document number.
///
/// Equivalent to `NumericDocValuesWriter.NumericDVs`, the object
/// `SortingCodecReader.getNumericDocValues(NumericDocValues)` builds and
/// `getOrCreate` caches.
#[derive(Debug)]
struct NumericDVs {
    docs_with_field: FixedBitSet,
    values: Vec<i64>,
}

impl NumericDVs {
    /// Drains `old` once, keying every value by the new document number.
    ///
    /// Equivalent to `SortingCodecReader#getNumericDocValues(NumericDocValues)`.
    fn new(max_doc: i32, doc_map: &SorterDocMap, old: &mut dyn NumericDocValues) -> Result<Self> {
        let mut docs_with_field = FixedBitSet::new(max_doc as usize);
        let mut values = vec![0i64; max_doc as usize];
        loop {
            let doc_id = old.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let new_doc_id = doc_map.old_to_new(doc_id);
            docs_with_field.set(new_doc_id as usize);
            values[new_doc_id as usize] = old.long_value()?;
        }
        Ok(Self {
            docs_with_field,
            values,
        })
    }
}

/// Sorted view of a single-valued numeric field.
///
/// Equivalent to `NumericDocValuesWriter.SortingNumericDocValues`, backed by
/// the same `NumericDVs` (bitset plus dense array) Java builds.
#[derive(Debug)]
struct SortingNumericDocValues {
    dvs: Arc<NumericDVs>,
    doc_id: i32,
}

impl SortingNumericDocValues {
    /// Equivalent to `SortingNumericDocValues(NumericDVs)`.
    fn new(dvs: Arc<NumericDVs>) -> Self {
        Self { dvs, doc_id: -1 }
    }
}

impl DocIdSetIterator for SortingNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc_id = next_doc_with_value(&self.dvs.docs_with_field, self.doc_id);
        Ok(self.doc_id)
    }

    /// Equivalent to `NumericDocValuesWriter.SortingNumericDocValues.advance(int)`, which throws
    /// `UnsupportedOperationException("use nextDoc() instead")`.
    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "use nextDoc() instead".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.dvs.docs_with_field.cardinality() as i64
    }
}

impl DocValuesIterator for SortingNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc_id = target;
        Ok(self.dvs.docs_with_field.get(target as usize))
    }
}

impl NumericDocValues for SortingNumericDocValues {
    fn long_value(&self) -> Result<i64> {
        Ok(self.dvs.values[self.doc_id as usize])
    }
}

/// The reordered values of a single-valued binary field, keyed by the new
/// document number.
///
/// Equivalent to `BinaryDocValuesWriter.BinaryDVs`, the object the `getBinary`
/// supplier of `SortingCodecReader.getDocValuesReader()` builds and
/// `getOrCreate` caches.
///
/// **Divergence from Lucene 10.5.0.** Java stores the values in one
/// `BytesRefArray` and an `int[]` of one-based offsets into it, using offset
/// `0` to mean "no value". This port keeps one owned [`BytesRef`] per document
/// beside the `docsWithField` bitset its sibling views already use. The
/// information and the answers are the same; only the memory layout differs.
#[derive(Debug)]
struct BinaryDVs {
    docs_with_field: FixedBitSet,
    values: Vec<BytesRef>,
}

impl BinaryDVs {
    /// Equivalent to `BinaryDVs(int, Sorter.DocMap, BinaryDocValues)`.
    fn new(max_doc: i32, doc_map: &SorterDocMap, old: &mut dyn BinaryDocValues) -> Result<Self> {
        let mut docs_with_field = FixedBitSet::new(max_doc as usize);
        let mut values = vec![BytesRef::default(); max_doc as usize];
        loop {
            let doc_id = old.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let new_doc_id = doc_map.old_to_new(doc_id);
            docs_with_field.set(new_doc_id as usize);
            values[new_doc_id as usize] = BytesRef::deep_copy_of(&old.binary_value()?);
        }
        Ok(Self {
            docs_with_field,
            values,
        })
    }
}

/// Sorted view of a single-valued binary field.
///
/// Equivalent to `BinaryDocValuesWriter.SortingBinaryDocValues`.
#[derive(Debug)]
struct SortingBinaryDocValues {
    dvs: Arc<BinaryDVs>,
    doc_id: i32,
}

impl SortingBinaryDocValues {
    /// Equivalent to `SortingBinaryDocValues(BinaryDVs)`.
    fn new(dvs: Arc<BinaryDVs>) -> Self {
        Self { dvs, doc_id: -1 }
    }
}

impl DocIdSetIterator for SortingBinaryDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc_id = next_doc_with_value(&self.dvs.docs_with_field, self.doc_id);
        Ok(self.doc_id)
    }

    /// Equivalent to `BinaryDocValuesWriter.SortingBinaryDocValues.advance(int)`, which throws
    /// `UnsupportedOperationException("use nextDoc instead")`.
    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "use nextDoc instead".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.dvs.docs_with_field.cardinality() as i64
    }
}

impl DocValuesIterator for SortingBinaryDocValues {
    /// Equivalent to `BinaryDocValuesWriter.SortingBinaryDocValues.advanceExact(int)`,
    /// which throws `UnsupportedOperationException("use nextDoc instead")`.
    fn advance_exact(&mut self, _target: i32) -> Result<bool> {
        Err(LuceneError::UnsupportedOperation(
            "use nextDoc instead".to_string(),
        ))
    }
}

impl BinaryDocValues for SortingBinaryDocValues {
    fn binary_value(&self) -> Result<BytesRef> {
        Ok(self.dvs.values[self.doc_id as usize].clone())
    }
}

/// The reordered ordinals of a single-valued sorted field, keyed by the new
/// document number.
///
/// Equivalent to the `int[] ords` that the `getSorted` supplier of
/// `SortingCodecReader.getDocValuesReader()` fills and `getOrCreate` caches.
///
/// **Divergence from Lucene 10.5.0.** Java's array alone carries which
/// documents have a value, through a `-1` sentinel it scans for. This port
/// fills the same `-1` sentinel but also keeps the `docsWithField` bitset its
/// sibling views use, so that the iteration is a bitset walk rather than a
/// linear scan. The information and the answers are the same; only the memory
/// layout differs.
#[derive(Debug)]
struct SortedDVs {
    docs_with_field: FixedBitSet,
    ords: Vec<i32>,
}

impl SortedDVs {
    /// Equivalent to the body of the `getSorted` supplier
    /// (`SortingCodecReader.java:662-672`).
    fn new(max_doc: i32, doc_map: &SorterDocMap, old: &mut dyn SortedDocValues) -> Result<Self> {
        let mut docs_with_field = FixedBitSet::new(max_doc as usize);
        let mut ords = vec![-1i32; max_doc as usize];
        loop {
            let doc_id = old.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let new_doc_id = doc_map.old_to_new(doc_id);
            docs_with_field.set(new_doc_id as usize);
            ords[new_doc_id as usize] = old.ord_value()?;
        }
        Ok(Self {
            docs_with_field,
            ords,
        })
    }
}

/// Sorted view of a single-valued sorted (dictionary-backed) field.
///
/// Equivalent to `SortedDocValuesWriter.SortingSortedDocValues`. The term
/// dictionary is unaffected by document reordering — only which document
/// points at which ordinal changes — so ordinal lookups delegate to `old`,
/// which is kept alive purely for that dictionary.
struct SortingSortedDocValues {
    old: Box<dyn SortedDocValues>,
    dvs: Arc<SortedDVs>,
    doc_id: i32,
}

impl std::fmt::Debug for SortingSortedDocValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingSortedDocValues")
            .field("doc_id", &self.doc_id)
            .finish_non_exhaustive()
    }
}

impl SortingSortedDocValues {
    /// Equivalent to `SortingSortedDocValues(SortedDocValues, int[])`.
    fn new(old: Box<dyn SortedDocValues>, dvs: Arc<SortedDVs>) -> Self {
        Self {
            old,
            dvs,
            doc_id: -1,
        }
    }
}

impl DocIdSetIterator for SortingSortedDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc_id = next_doc_with_value(&self.dvs.docs_with_field, self.doc_id);
        Ok(self.doc_id)
    }

    /// Equivalent to `SortedDocValuesWriter.SortingSortedDocValues.advance(int)`, which throws
    /// `UnsupportedOperationException("use nextDoc instead")`.
    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "use nextDoc instead".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.dvs.docs_with_field.cardinality() as i64
    }
}

impl DocValuesIterator for SortingSortedDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc_id = target;
        Ok(self.dvs.ords[target as usize] != -1)
    }
}

impl SortedDocValues for SortingSortedDocValues {
    fn ord_value(&self) -> Result<i32> {
        Ok(self.dvs.ords[self.doc_id as usize])
    }

    fn get_value_count(&self) -> Result<i32> {
        self.old.get_value_count()
    }

    fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
        self.old.lookup_ord(ord)
    }
}

/// The reordered values of a multi-valued numeric field, keyed by the new
/// document number.
///
/// Equivalent to `SortedNumericDocValuesWriter.LongValues`, the object the
/// `getSortedNumeric` supplier of `SortingCodecReader.getDocValuesReader()`
/// builds and `getOrCreate` caches.
///
/// **Divergence from Lucene 10.5.0.** Java packs every document's values into
/// one `PackedLongValues` addressed through a `long[]` of one-based offsets.
/// This port keeps one `Vec<i64>` per document beside the `docsWithField`
/// bitset its sibling views use, which is observably identical — only the
/// memory layout differs, exactly the kind of divergence
/// [`FieldUpdatesBuffer`](crate::index::FieldUpdatesBuffer) already accepts
/// for the same reason.
#[derive(Debug)]
struct LongValues {
    docs_with_field: FixedBitSet,
    values: Vec<Vec<i64>>,
}

impl LongValues {
    /// Equivalent to
    /// `LongValues(int, Sorter.DocMap, SortedNumericDocValues, float)`.
    fn new(
        max_doc: i32,
        doc_map: &SorterDocMap,
        old: &mut dyn SortedNumericDocValues,
    ) -> Result<Self> {
        let mut docs_with_field = FixedBitSet::new(max_doc as usize);
        let mut values: Vec<Vec<i64>> = vec![Vec::new(); max_doc as usize];
        loop {
            let doc_id = old.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let new_doc_id = doc_map.old_to_new(doc_id);
            docs_with_field.set(new_doc_id as usize);
            let count = old.doc_value_count()?;
            let slot = &mut values[new_doc_id as usize];
            for _ in 0..count {
                slot.push(old.next_value()?);
            }
        }
        Ok(Self {
            docs_with_field,
            values,
        })
    }
}

/// Sorted view of a multi-valued numeric field.
///
/// Equivalent to `SortedNumericDocValuesWriter.SortingSortedNumericDocValues`.
#[derive(Debug)]
struct SortingSortedNumericDocValues {
    values: Arc<LongValues>,
    doc_id: i32,
    value_idx: usize,
}

impl SortingSortedNumericDocValues {
    /// Equivalent to
    /// `SortingSortedNumericDocValues(SortedNumericDocValues, LongValues)`.
    fn new(values: Arc<LongValues>) -> Self {
        Self {
            values,
            doc_id: -1,
            value_idx: 0,
        }
    }
}

impl DocIdSetIterator for SortingSortedNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc_id = next_doc_with_value(&self.values.docs_with_field, self.doc_id);
        self.value_idx = 0;
        Ok(self.doc_id)
    }

    /// Equivalent to `SortedNumericDocValuesWriter.SortingSortedNumericDocValues.advance(int)`, which throws
    /// `UnsupportedOperationException("use nextDoc instead")`.
    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "use nextDoc instead".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.values.docs_with_field.cardinality() as i64
    }
}

impl DocValuesIterator for SortingSortedNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc_id = target;
        self.value_idx = 0;
        Ok(self.values.docs_with_field.get(target as usize))
    }
}

impl SortedNumericDocValues for SortingSortedNumericDocValues {
    fn next_value(&mut self) -> Result<i64> {
        let value = self.values.values[self.doc_id as usize][self.value_idx];
        self.value_idx += 1;
        Ok(value)
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(self.values.values[self.doc_id as usize].len() as i32)
    }
}

/// The reordered ordinals of a multi-valued sorted-set field, keyed by the new
/// document number.
///
/// Equivalent to `SortedSetDocValuesWriter.DocOrds`, the object the
/// `getSortedSet` supplier of `SortingCodecReader.getDocValuesReader()` builds
/// and `getOrCreate` caches.
///
/// **Divergence from Lucene 10.5.0.** Same as [`LongValues`]: one `Vec<i64>`
/// per document beside a `docsWithField` bitset, instead of a packed structure
/// addressed through offsets.
#[derive(Debug)]
struct DocOrds {
    docs_with_field: FixedBitSet,
    ords: Vec<Vec<i64>>,
}

impl DocOrds {
    /// Equivalent to
    /// `DocOrds(int, Sorter.DocMap, SortedSetDocValues, float, int)`.
    fn new(max_doc: i32, doc_map: &SorterDocMap, old: &mut dyn SortedSetDocValues) -> Result<Self> {
        let mut docs_with_field = FixedBitSet::new(max_doc as usize);
        let mut ords: Vec<Vec<i64>> = vec![Vec::new(); max_doc as usize];
        loop {
            let doc_id = old.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let new_doc_id = doc_map.old_to_new(doc_id);
            docs_with_field.set(new_doc_id as usize);
            let count = old.doc_value_count()?;
            let slot = &mut ords[new_doc_id as usize];
            for _ in 0..count {
                slot.push(old.next_ord()?);
            }
        }
        Ok(Self {
            docs_with_field,
            ords,
        })
    }
}

/// Sorted view of a multi-valued sorted-set field.
///
/// Equivalent to `SortedSetDocValuesWriter.SortingSortedSetDocValues`. As with
/// [`SortingSortedDocValues`], the term dictionary is unaffected by document
/// reordering, so `old` is kept alive for ordinal lookups only.
struct SortingSortedSetDocValues {
    old: Box<dyn SortedSetDocValues>,
    dvs: Arc<DocOrds>,
    doc_id: i32,
    ord_idx: usize,
}

impl std::fmt::Debug for SortingSortedSetDocValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingSortedSetDocValues")
            .field("doc_id", &self.doc_id)
            .finish_non_exhaustive()
    }
}

impl SortingSortedSetDocValues {
    /// Equivalent to `SortingSortedSetDocValues(SortedSetDocValues, DocOrds)`.
    fn new(old: Box<dyn SortedSetDocValues>, dvs: Arc<DocOrds>) -> Self {
        Self {
            old,
            dvs,
            doc_id: -1,
            ord_idx: 0,
        }
    }
}

impl DocIdSetIterator for SortingSortedSetDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc_id = next_doc_with_value(&self.dvs.docs_with_field, self.doc_id);
        self.ord_idx = 0;
        Ok(self.doc_id)
    }

    /// Equivalent to `SortedSetDocValuesWriter.SortingSortedSetDocValues.advance(int)`, which throws
    /// `UnsupportedOperationException("use nextDoc instead")`.
    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "use nextDoc instead".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.dvs.docs_with_field.cardinality() as i64
    }
}

impl DocValuesIterator for SortingSortedSetDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc_id = target;
        self.ord_idx = 0;
        Ok(self.dvs.docs_with_field.get(target as usize))
    }
}

impl SortedSetDocValues for SortingSortedSetDocValues {
    fn next_ord(&mut self) -> Result<i64> {
        let ord = self.dvs.ords[self.doc_id as usize][self.ord_idx];
        self.ord_idx += 1;
        Ok(ord)
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(self.dvs.ords[self.doc_id as usize].len() as i32)
    }

    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        self.old.lookup_ord(ord)
    }

    fn get_value_count(&self) -> Result<i64> {
        self.old.get_value_count()
    }
}

// -----------------------------------------------------------------------------
// Sorted stored-fields and term-vectors wrappers
// -----------------------------------------------------------------------------

/// Sorted view of a [`StoredFieldsReader`]: every access remaps the requested
/// new document number to the old one before delegating.
///
/// Equivalent to the anonymous `StoredFieldsReader`
/// `SortingCodecReader.newStoredFieldsReader` builds.
struct SortingStoredFieldsReader {
    delegate: Box<dyn StoredFieldsReader>,
    doc_map: Arc<SorterDocMap>,
}

impl std::fmt::Debug for SortingStoredFieldsReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingStoredFieldsReader")
            .finish_non_exhaustive()
    }
}

impl StoredFieldsReader for SortingStoredFieldsReader {
    fn document(&self, doc_id: i32, visitor: &mut dyn StoredFieldVisitor) -> Result<()> {
        self.delegate
            .document(self.doc_map.new_to_old(doc_id), visitor)
    }

    fn check_integrity(&self) -> Result<()> {
        self.delegate.check_integrity()
    }

    fn clone_reader(&self) -> Box<dyn StoredFieldsReader> {
        Box::new(SortingStoredFieldsReader {
            delegate: self.delegate.clone_reader(),
            doc_map: Arc::clone(&self.doc_map),
        })
    }

    fn prefetch(&self, doc_id: i32) -> Result<()> {
        self.delegate.prefetch(self.doc_map.new_to_old(doc_id))
    }

    /// Equivalent to the anonymous reader's `close()`, which closes the
    /// delegate (`SortingCodecReader.java:517-520`).
    fn close(&mut self) -> Result<()> {
        self.delegate.close()
    }
}

/// Sorted view of a [`TermVectorsReader`]: `get` remaps the requested new
/// document number to the old one before delegating.
///
/// Equivalent to the anonymous `TermVectorsReader`
/// `SortingCodecReader.newTermVectorsReader` builds.
struct SortingTermVectorsReader {
    delegate: Box<dyn TermVectorsReader>,
    doc_map: Arc<SorterDocMap>,
}

impl std::fmt::Debug for SortingTermVectorsReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingTermVectorsReader")
            .finish_non_exhaustive()
    }
}

impl TermVectorsReader for SortingTermVectorsReader {
    fn check_integrity(&self) -> Result<()> {
        self.delegate.check_integrity()
    }

    fn clone_reader(&self) -> Box<dyn TermVectorsReader> {
        Box::new(SortingTermVectorsReader {
            delegate: self.delegate.clone_reader(),
            doc_map: Arc::clone(&self.doc_map),
        })
    }

    /// Equivalent to the anonymous reader's `close()`, which closes the
    /// delegate (`SortingCodecReader.java:765-768`).
    fn close(&mut self) -> Result<()> {
        self.delegate.close()
    }
}

impl TermVectors for SortingTermVectorsReader {
    fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
        self.delegate.get(self.doc_map.new_to_old(doc))
    }

    fn prefetch(&mut self, doc_id: i32) -> Result<()> {
        self.delegate.prefetch(self.doc_map.new_to_old(doc_id))
    }
}

/// Sorted view of a [`Bits`] (typically live docs): reads bit `doc_map.new_to_old(index)`.
///
/// Equivalent to `SortingCodecReader.SortingBits`.
struct SortingBits {
    inner: Box<dyn Bits>,
    doc_map: Arc<SorterDocMap>,
}

impl std::fmt::Debug for SortingBits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingBits").finish_non_exhaustive()
    }
}

impl Bits for SortingBits {
    fn get(&self, index: usize) -> bool {
        self.inner
            .get(self.doc_map.new_to_old(index as i32) as usize)
    }

    /// Forwards to the wrapped bits' `try_get`, so that an error they raise
    /// (`ExitableDirectoryReader.java:359-387`, whose `get` throws through
    /// every wrapper that delegates to it) is not swallowed here.
    fn try_get(&self, index: usize) -> Result<bool> {
        self.inner
            .try_get(self.doc_map.new_to_old(index as i32) as usize)
    }

    fn length(&self) -> usize {
        self.inner.length()
    }
}

// -----------------------------------------------------------------------------
// Sorted doc-values / norms producer wrappers
// -----------------------------------------------------------------------------

/// Sorted view of a [`DocValuesProducer`]: every accessor eagerly rebuilds its
/// values keyed by the new document order, through the reader's one-entry
/// cache.
///
/// Equivalent to the anonymous `DocValuesProducer`
/// `SortingCodecReader.getDocValuesReader()` builds.
///
/// **Divergence from Lucene 10.5.0.** Java's `close()` closes the delegate
/// (`SortingCodecReader.java:708-711`). The delegate here is the [`Arc`] the
/// wrapped reader shares with every other holder and closes itself, so `close`
/// releases nothing — as every other producer wrapper of this module does.
struct SortingDocValuesProducer {
    delegate: Arc<dyn DocValuesProducer>,
    doc_map: Arc<SorterDocMap>,
    max_doc: i32,
    state: Arc<SortingCodecReaderState>,
}

impl std::fmt::Debug for SortingDocValuesProducer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingDocValuesProducer")
            .finish_non_exhaustive()
    }
}

impl DocValuesProducer for SortingDocValuesProducer {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        let values: Result<Box<dyn NumericDocValues>> = {
            let dvs = self.state.get_or_create_dv(&field.name, || {
                let mut old =
                    crate::codecs::doc_values::non_null(self.delegate.get_numeric(field)?, field)?;
                NumericDVs::new(self.max_doc, &self.doc_map, &mut *old)
            })?;
            Ok(Box::new(SortingNumericDocValues::new(dvs)))
        };
        Ok(Some(values?))
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        let values: Result<Box<dyn BinaryDocValues>> = {
            let dvs = self.state.get_or_create_dv(&field.name, || {
                let mut old =
                    crate::codecs::doc_values::non_null(self.delegate.get_binary(field)?, field)?;
                BinaryDVs::new(self.max_doc, &self.doc_map, &mut *old)
            })?;
            Ok(Box::new(SortingBinaryDocValues::new(dvs)))
        };
        Ok(Some(values?))
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        let values: Result<Box<dyn SortedDocValues>> = {
            // As in Java, the old values are pulled outside the supplier: the
            // sorted view keeps them for its term dictionary even when the
            // reordered ordinals come from the cache and the supplier never runs.
            let mut old =
                crate::codecs::doc_values::non_null(self.delegate.get_sorted(field)?, field)?;
            let dvs = self.state.get_or_create_dv(&field.name, || {
                SortedDVs::new(self.max_doc, &self.doc_map, &mut *old)
            })?;
            Ok(Box::new(SortingSortedDocValues::new(old, dvs)))
        };
        Ok(Some(values?))
    }

    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        let values: Result<Box<dyn SortedNumericDocValues>> = {
            let mut old = crate::codecs::doc_values::non_null(
                self.delegate.get_sorted_numeric(field)?,
                field,
            )?;
            let values = self.state.get_or_create_dv(&field.name, || {
                LongValues::new(self.max_doc, &self.doc_map, &mut *old)
            })?;
            Ok(Box::new(SortingSortedNumericDocValues::new(values)))
        };
        Ok(Some(values?))
    }

    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        let values: Result<Box<dyn SortedSetDocValues>> = {
            let mut old =
                crate::codecs::doc_values::non_null(self.delegate.get_sorted_set(field)?, field)?;
            let dvs = self.state.get_or_create_dv(&field.name, || {
                DocOrds::new(self.max_doc, &self.doc_map, &mut *old)
            })?;
            Ok(Box::new(SortingSortedSetDocValues::new(old, dvs)))
        };
        Ok(Some(values?))
    }

    /// Equivalent to `SortingCodecReader.getDocValuesReader().getSkipper`
    /// (`index/SortingCodecReader.java:713-717`), which returns `null`:
    /// min/max-per-block skip metadata is meaningless once documents have been
    /// reordered, so no skip index is offered.
    fn get_skipper(&self, _field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        Ok(None)
    }

    fn check_integrity(&self) -> Result<()> {
        self.delegate.check_integrity()
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(SortingDocValuesProducer {
            delegate: Arc::clone(&self.delegate),
            doc_map: Arc::clone(&self.doc_map),
            max_doc: self.max_doc,
            state: Arc::clone(&self.state),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Sorted view of a [`NormsProducer`], reusing the same numeric-values
/// materialisation as [`SortingDocValuesProducer::get_numeric`], but through
/// the cache's norms half so that a field's norms and its doc values never
/// evict one another's key.
///
/// Equivalent to the anonymous `NormsProducer`
/// `SortingCodecReader.getNormsReader()` builds. The close divergence of
/// [`SortingDocValuesProducer`] applies (`SortingCodecReader.java:626-629`).
struct SortingNormsProducer {
    delegate: Arc<dyn NormsProducer>,
    doc_map: Arc<SorterDocMap>,
    max_doc: i32,
    state: Arc<SortingCodecReaderState>,
}

impl std::fmt::Debug for SortingNormsProducer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingNormsProducer")
            .finish_non_exhaustive()
    }
}

impl NormsProducer for SortingNormsProducer {
    fn get_norms(&self, field_info: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        let values: Result<Box<dyn NumericDocValues>> = {
            let dvs = self.state.get_or_create_norms(&field_info.name, || {
                let mut old = crate::codecs::doc_values::non_null(
                    self.delegate.get_norms(field_info)?,
                    field_info,
                )?;
                NumericDVs::new(self.max_doc, &self.doc_map, &mut *old)
            })?;
            Ok(Box::new(SortingNumericDocValues::new(dvs)))
        };
        Ok(Some(values?))
    }

    fn check_integrity(&self) -> Result<()> {
        self.delegate.check_integrity()
    }

    fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(SortingNormsProducer {
            delegate: Arc::clone(&self.delegate),
            doc_map: Arc::clone(&self.doc_map),
            max_doc: self.max_doc,
            state: Arc::clone(&self.state),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Sorted postings, points and vectors
// -----------------------------------------------------------------------------

/// Sorted view of a [`FieldsProducer`]: every field's terms are wrapped in
/// [`SortingTerms`], so their postings answer in the new document order.
///
/// Equivalent to the anonymous `FieldsProducer`
/// `SortingCodecReader.getPostingsReader()` builds
/// (`SortingCodecReader.java:448-484`).
///
/// **Divergence from Lucene 10.5.0.** Java's `close()` closes the delegate.
/// The delegate here is the [`Arc`] the wrapped reader shares with every other
/// holder, and it is closed by its owner, so `close` releases nothing — as
/// every other producer wrapper of this module does.
struct SortingFieldsProducer {
    postings_reader: Arc<dyn FieldsProducer>,
    /// `in.getFieldInfos()`, read for each field's index options.
    field_infos: FieldInfos,
    doc_map: Arc<SorterDocMap>,
}

impl Fields for SortingFieldsProducer {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        self.postings_reader.iterator()
    }

    /// Equivalent to the anonymous producer's `terms(String)`
    /// (`SortingCodecReader.java:471-477`).
    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        let Some(terms) = self.postings_reader.terms(field)? else {
            return Ok(None);
        };
        let index_options = self
            .field_infos
            .field_info(field)
            .ok_or_else(|| {
                crate::error::LuceneError::illegal_state(format!(
                    "field \"{field}\" has terms but no FieldInfo"
                ))
            })?
            .index_options;
        Ok(Some(Box::new(SortingTerms::new(
            terms,
            index_options,
            Arc::clone(&self.doc_map),
        ))))
    }

    fn size(&self) -> i32 {
        self.postings_reader.size()
    }
}

impl FieldsProducer for SortingFieldsProducer {
    fn check_integrity(&self) -> Result<()> {
        self.postings_reader.check_integrity()
    }

    /// Inherited from `FieldsProducer.getMergeInstance()`, which returns
    /// `this`; the owned equivalent is a view over the same delegate.
    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(SortingFieldsProducer {
            postings_reader: Arc::clone(&self.postings_reader),
            field_infos: self.field_infos.clone(),
            doc_map: Arc::clone(&self.doc_map),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Sorted view of a [`PointsReader`]: every field's values are wrapped in
/// [`SortingPointValues`].
///
/// Equivalent to the anonymous `PointsReader`
/// `SortingCodecReader.getPointsReader()` builds
/// (`SortingCodecReader.java:534-560`). The close divergence of
/// [`SortingFieldsProducer`] applies.
#[derive(Debug)]
struct SortingPointsReader {
    delegate: Arc<dyn PointsReader>,
    doc_map: Arc<SorterDocMap>,
}

impl PointsReader for SortingPointsReader {
    fn check_integrity(&self) -> Result<()> {
        self.delegate.check_integrity()
    }

    /// Equivalent to the anonymous reader's `getValues(String)`
    /// (`SortingCodecReader.java:547-553`). Java asks the delegate twice, once
    /// for the null check and once for the values it wraps; every codec
    /// answers both with the same values, so they are asked for once here.
    fn get_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        Ok(self.delegate.get_values(field)?.map(|values| {
            Box::new(SortingPointValues {
                inner: values,
                doc_map: Arc::clone(&self.doc_map),
            }) as Box<dyn PointValues>
        }))
    }

    /// Inherited from `PointsReader.getMergeInstance()`, which returns `this`.
    fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(SortingPointsReader {
            delegate: Arc::clone(&self.delegate),
            doc_map: Arc::clone(&self.doc_map),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Sorted view of a field's [`PointValues`]: the statistics are the wrapped
/// values', and the tree reports every document under its new number.
///
/// Equivalent to `SortingCodecReader.SortingPointValues`
/// (`SortingCodecReader.java:71-120`).
struct SortingPointValues {
    inner: Box<dyn PointValues>,
    doc_map: Arc<SorterDocMap>,
}

impl PointValues for SortingPointValues {
    fn point_tree(&self) -> Result<Box<dyn PointTree>> {
        Ok(Box::new(SortingPointTree {
            index_tree: self.inner.point_tree()?,
            doc_map: Arc::clone(&self.doc_map),
        }))
    }

    fn size(&self) -> Result<i64> {
        self.inner.size()
    }

    fn doc_count(&self) -> Result<i32> {
        self.inner.doc_count()
    }

    fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
        self.inner.min_packed_value()
    }

    fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
        self.inner.max_packed_value()
    }

    fn num_dimensions(&self) -> Result<i32> {
        self.inner.num_dimensions()
    }

    fn num_index_dimensions(&self) -> Result<i32> {
        self.inner.num_index_dimensions()
    }

    fn bytes_per_dimension(&self) -> Result<i32> {
        self.inner.bytes_per_dimension()
    }
}

/// Sorted view of a [`PointTree`]: navigation is the wrapped tree's, and every
/// visit maps the document through the doc map.
///
/// Equivalent to `SortingCodecReader.SortingPointTree`
/// (`SortingCodecReader.java:122-180`). Java keeps one
/// `SortingIntersectVisitor` per tree and re-targets it on each visit; a
/// [`SortingIntersectVisitor`] borrowing the caller's visitor is built per
/// visit here, which is the same object state at every call.
struct SortingPointTree {
    index_tree: Box<dyn PointTree>,
    doc_map: Arc<SorterDocMap>,
}

impl PointTree for SortingPointTree {
    fn clone_tree(&self) -> Result<Box<dyn PointTree>> {
        Ok(Box::new(SortingPointTree {
            index_tree: self.index_tree.clone_tree()?,
            doc_map: Arc::clone(&self.doc_map),
        }))
    }

    fn move_to_child(&mut self) -> Result<bool> {
        self.index_tree.move_to_child()
    }

    fn move_to_sibling(&mut self) -> Result<bool> {
        self.index_tree.move_to_sibling()
    }

    fn move_to_parent(&mut self) -> Result<bool> {
        self.index_tree.move_to_parent()
    }

    fn min_packed_value(&self) -> Result<&[u8]> {
        self.index_tree.min_packed_value()
    }

    fn max_packed_value(&self) -> Result<&[u8]> {
        self.index_tree.max_packed_value()
    }

    fn size(&self) -> Result<i64> {
        self.index_tree.size()
    }

    fn visit_doc_ids(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        let mut sorting = SortingIntersectVisitor {
            visitor,
            doc_map: &self.doc_map,
        };
        self.index_tree.visit_doc_ids(&mut sorting)
    }

    fn visit_doc_values(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        let mut sorting = SortingIntersectVisitor {
            visitor,
            doc_map: &self.doc_map,
        };
        self.index_tree.visit_doc_values(&mut sorting)
    }
}

/// An [`IntersectVisitor`] that maps each old document number to its new one
/// before handing it on.
///
/// Equivalent to `SortingCodecReader.SortingIntersectVisitor`
/// (`SortingCodecReader.java:182-206`), which overrides `visit(int)`,
/// `visit(int, byte[])` and `compare`, and inherits every other member.
struct SortingIntersectVisitor<'a> {
    visitor: &'a mut dyn IntersectVisitor,
    doc_map: &'a SorterDocMap,
}

impl IntersectVisitor for SortingIntersectVisitor<'_> {
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        self.visitor.visit(self.doc_map.old_to_new(doc_id))
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        self.visitor
            .visit_with_value(self.doc_map.old_to_new(doc_id), packed_value)
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Result<Relation> {
        self.visitor.compare(min_packed_value, max_packed_value)
    }
}

/// Sorted view of a [`KnnVectorsReader`]: each field's values are wrapped in
/// [`SortingFloatVectorValues`] or [`SortingByteVectorValues`], and searching
/// is unsupported.
///
/// Equivalent to the anonymous `KnnVectorsReader`
/// `SortingCodecReader.getVectorReader()` builds
/// (`SortingCodecReader.java:562-606`). The close divergence of
/// [`SortingFieldsProducer`] applies.
#[derive(Debug)]
struct SortingKnnVectorsReader {
    delegate: Arc<dyn KnnVectorsReader>,
    doc_map: Arc<SorterDocMap>,
}

impl KnnVectorsReader for SortingKnnVectorsReader {
    fn check_integrity(&self) -> Result<()> {
        self.delegate.check_integrity()
    }

    /// Equivalent to the anonymous reader's `getFloatVectorValues(String)`
    /// (`SortingCodecReader.java:575-577`). Java wraps whatever the delegate
    /// returns and fails on `null`; a field the delegate has no values for
    /// is answered with `None` here, which `CodecReader` never asks for.
    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        match self.delegate.get_float_vector_values(field)? {
            None => Ok(None),
            Some(values) => Ok(Some(Box::new(SortingFloatVectorValues::new(
                values,
                &self.doc_map,
            )?))),
        }
    }

    /// Equivalent to the anonymous reader's `getByteVectorValues(String)`
    /// (`SortingCodecReader.java:580-582`); see
    /// [`get_float_vector_values`](Self::get_float_vector_values).
    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        match self.delegate.get_byte_vector_values(field)? {
            None => Ok(None),
            Some(values) => Ok(Some(Box::new(SortingByteVectorValues::new(
                values,
                &self.doc_map,
            )?))),
        }
    }

    /// Equivalent to the anonymous reader's `search(String, float[], ...)`
    /// (`SortingCodecReader.java:585-589`).
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneError::UnsupportedOperation`](crate::error::LuceneError::UnsupportedOperation).
    fn search(
        &self,
        _field: &str,
        _target: &[f32],
        _knn_collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Err(crate::error::LuceneError::UnsupportedOperation(
            "SortingCodecReader's vectors reader does not search".to_string(),
        ))
    }

    /// Equivalent to the anonymous reader's `search(String, byte[], ...)`
    /// (`SortingCodecReader.java:591-595`).
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneError::UnsupportedOperation`](crate::error::LuceneError::UnsupportedOperation).
    fn search_byte(
        &self,
        _field: &str,
        _target: &[u8],
        _knn_collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Err(crate::error::LuceneError::UnsupportedOperation(
            "SortingCodecReader's vectors reader does not search".to_string(),
        ))
    }

    /// Inherited from `KnnVectorsReader.getMergeInstance()`, which returns
    /// `this`.
    fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
        Ok(Box::new(SortingKnnVectorsReader {
            delegate: Arc::clone(&self.delegate),
            doc_map: Arc::clone(&self.doc_map),
        }))
    }

    /// Equivalent to the anonymous reader's `getOffHeapByteSize(FieldInfo)`
    /// (`SortingCodecReader.java:597-600`).
    fn get_off_heap_byte_size(&self, field_info: &FieldInfo) -> HashMap<String, i64> {
        self.delegate.get_off_heap_byte_size(field_info)
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Sorted float vector values, which map ordinals through the doc map.
///
/// Equivalent to `SortingCodecReader.SortingFloatVectorValues`
/// (`SortingCodecReader.java:312-348`): ordinals stay in the delegate's
/// ordinal space, and the iterator visits the documents in their new order.
struct SortingFloatVectorValues {
    delegate: Box<dyn FloatVectorValues>,
    iterator_supplier: SortingIteratorSupplier,
}

impl SortingFloatVectorValues {
    /// Equivalent to `SortingFloatVectorValues(FloatVectorValues, Sorter.DocMap)`.
    fn new(delegate: Box<dyn FloatVectorValues>, sort_map: &SorterDocMap) -> Result<Self> {
        // SortingValuesIterator consumes the iterator and records the docs and
        // ord mapping.
        let iterator_supplier = SortingCodecReader::iterator_supplier(delegate.as_ref(), sort_map)?;
        Ok(Self {
            delegate,
            iterator_supplier,
        })
    }
}

impl KnnVectorValues for SortingFloatVectorValues {
    fn dimension(&self) -> i32 {
        self.delegate.dimension()
    }

    fn size(&self) -> i32 {
        self.iterator_supplier.size()
    }

    /// Equivalent to `SortingFloatVectorValues.copy()`, which throws
    /// `UnsupportedOperationException`.
    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Err(crate::error::LuceneError::UnsupportedOperation(
            "SortingFloatVectorValues.copy".to_string(),
        ))
    }

    fn encoding(&self) -> crate::index::VectorEncoding {
        crate::index::VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(Box::new(self.iterator_supplier.get()))
    }
}

impl FloatVectorValues for SortingFloatVectorValues {
    /// Ordinals are interpreted in the delegate's ordinal space.
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        self.delegate.vector_value(ord)
    }

    /// Equivalent to `SortingFloatVectorValues.copy()`, which throws
    /// `UnsupportedOperationException`.
    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Err(crate::error::LuceneError::UnsupportedOperation(
            "SortingFloatVectorValues.copy".to_string(),
        ))
    }
}

/// Sorted byte vector values, which map ordinals through the doc map.
///
/// Equivalent to `SortingCodecReader.SortingByteVectorValues`
/// (`SortingCodecReader.java:350-384`).
struct SortingByteVectorValues {
    delegate: Box<dyn ByteVectorValues>,
    iterator_supplier: SortingIteratorSupplier,
}

impl SortingByteVectorValues {
    /// Equivalent to `SortingByteVectorValues(ByteVectorValues, Sorter.DocMap)`.
    fn new(delegate: Box<dyn ByteVectorValues>, sort_map: &SorterDocMap) -> Result<Self> {
        // SortingValuesIterator consumes the iterator and records the docs and
        // ord mapping.
        let iterator_supplier = SortingCodecReader::iterator_supplier(delegate.as_ref(), sort_map)?;
        Ok(Self {
            delegate,
            iterator_supplier,
        })
    }
}

impl KnnVectorValues for SortingByteVectorValues {
    fn dimension(&self) -> i32 {
        self.delegate.dimension()
    }

    fn size(&self) -> i32 {
        self.iterator_supplier.size()
    }

    /// Equivalent to `SortingByteVectorValues.copy()`, which throws
    /// `UnsupportedOperationException`.
    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Err(crate::error::LuceneError::UnsupportedOperation(
            "SortingByteVectorValues.copy".to_string(),
        ))
    }

    fn encoding(&self) -> crate::index::VectorEncoding {
        crate::index::VectorEncoding::BYTE
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(Box::new(self.iterator_supplier.get()))
    }
}

impl ByteVectorValues for SortingByteVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
        self.delegate.vector_value(ord)
    }

    /// Equivalent to `SortingByteVectorValues.copy()`, which throws
    /// `UnsupportedOperationException`.
    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        Err(crate::error::LuceneError::UnsupportedOperation(
            "SortingByteVectorValues.copy".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{
        Document, IntPoint, KnnFloatVectorField, NumericDocValuesField, SortedDocValuesField,
        Store, StoredField, StringField,
    };
    use crate::index::index_sorter::Sorter;
    use crate::index::index_writer::IndexWriter;
    use crate::index::index_writer_config::IndexWriterConfig;
    use crate::index::segment_reader::SegmentReader;
    use crate::index::SegmentInfos;
    use crate::store::{Directory, RamDirectory, DEFAULT_IO_CONTEXT};

    /// One document per global number `doc`, every kind of data keyed by it.
    fn doc(doc: i32) -> Document {
        let mut d = Document::new();
        d.add(Box::new(
            StringField::new("id", format!("d{doc}"), Store::NO).unwrap(),
        ));
        d.add(Box::new(NumericDocValuesField::new(
            "n",
            i64::from(doc) * 10,
        )));
        d.add(Box::new(SortedDocValuesField::new(
            "s",
            BytesRef::new(format!("s{doc}").into_bytes()),
        )));
        d.add(Box::new(
            StoredField::new_string("sv", format!("v{doc}")).unwrap(),
        ));
        d.add(Box::new(IntPoint::new("p", &[doc]).unwrap()));
        d.add(Box::new(
            KnnFloatVectorField::with_euclidean("v", &[doc as f32, 1.0]).unwrap(),
        ));
        d
    }

    /// Two committed segments, of two and three documents, read as codec
    /// readers in segment order.
    fn two_segments() -> Vec<Arc<dyn CodecReader>> {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        for d in 0..2 {
            writer.add_document(&doc(d)).unwrap();
        }
        writer.commit().unwrap();
        for d in 2..5 {
            writer.add_document(&doc(d)).unwrap();
        }
        writer.commit().unwrap();
        writer.close().unwrap();
        let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
        assert_eq!(infos.size(), 2, "the fixture needs two segments");
        (0..infos.size())
            .map(|i| {
                Arc::new(
                    SegmentReader::new(
                        infos.info(i).clone(),
                        infos.index_created_version_major(),
                        &*DEFAULT_IO_CONTEXT,
                    )
                    .unwrap(),
                ) as Arc<dyn CodecReader>
            })
            .collect()
    }

    /// Every (doc, value) a numeric doc-values iterator yields.
    fn numeric_pairs(mut values: Box<dyn NumericDocValues>) -> Vec<(i32, i64)> {
        let mut pairs = Vec::new();
        loop {
            let doc = values.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                return pairs;
            }
            pairs.push((doc, values.long_value().unwrap()));
        }
    }

    /// Every (doc, term) a sorted doc-values iterator yields.
    fn sorted_pairs(mut values: Box<dyn SortedDocValues>) -> Vec<(i32, String)> {
        let mut pairs = Vec::new();
        loop {
            let doc = values.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                return pairs;
            }
            let term = values.lookup_ord(values.ord_value().unwrap()).unwrap();
            pairs.push((doc, String::from_utf8(term.slice().to_vec()).unwrap()));
        }
    }

    /// The documents the postings of `id:text` hold.
    fn postings_of(reader: &dyn CodecReader, text: &str) -> Vec<i32> {
        let terms = reader.terms("id").unwrap().unwrap();
        let mut terms_enum = terms.iterator().unwrap();
        assert!(terms_enum
            .seek_exact(&BytesRef::new(text.as_bytes().to_vec()))
            .unwrap());
        let mut postings = terms_enum.postings(None, 0).unwrap();
        let mut docs = Vec::new();
        loop {
            let doc = postings.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                return docs;
            }
            docs.push(doc);
        }
    }

    /// Records every (doc, first packed byte block as i32) a point traversal
    /// visits.
    struct CollectPoints(Vec<(i32, i32)>);

    impl IntersectVisitor for CollectPoints {
        fn visit(&mut self, _doc_id: i32) -> Result<()> {
            panic!("every cell crosses the query, so every point carries its value")
        }

        fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
            let value = crate::util::NumericUtils::sortable_bytes_to_int(packed_value, 0);
            self.0.push((doc_id, value));
            Ok(())
        }

        fn compare(&self, _min: &[u8], _max: &[u8]) -> Result<Relation> {
            Ok(Relation::CellCrossesQuery)
        }
    }

    fn points_of(reader: &dyn CodecReader) -> Vec<(i32, i32)> {
        let values = reader.get_point_values("p").unwrap().unwrap();
        let mut visitor = CollectPoints(Vec::new());
        values.intersect(&mut visitor).unwrap();
        visitor.0.sort_unstable();
        visitor.0
    }

    /// Every (doc, first vector component) a vector iterator yields.
    fn vectors_of(reader: &dyn CodecReader) -> Vec<(i32, f32)> {
        let values = reader.get_float_vector_values("v").unwrap().unwrap();
        let mut iterator = values.iterator().unwrap();
        let mut pairs = Vec::new();
        loop {
            let doc = iterator.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                return pairs;
            }
            pairs.push((doc, values.vector_value(iterator.index()).unwrap()[0]));
        }
    }

    /// The string value of `sv` and the field number it was reported under.
    struct CollectStored(Vec<(String, i32, String)>);

    impl StoredFieldVisitor for CollectStored {
        fn needs_field(&mut self, _field_info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
            Ok(StoredFieldVisitorStatus::Yes)
        }

        fn string_field(&mut self, field_info: &FieldInfo, value: &str) -> Result<()> {
            self.0.push((
                field_info.name.clone(),
                field_info.number,
                value.to_string(),
            ));
            Ok(())
        }
    }

    fn stored_of(reader: &dyn CodecReader, doc: i32) -> String {
        let mut visitor = CollectStored(Vec::new());
        reader
            .get_fields_reader()
            .unwrap()
            .unwrap()
            .document(doc, &mut visitor)
            .unwrap();
        let (name, number, value) = visitor.0.pop().unwrap();
        assert_eq!(name, "sv");
        assert_eq!(
            Some(number),
            reader
                .get_field_infos()
                .field_info("sv")
                .map(|fi| fi.number),
            "the field info is the composite's own (SlowCompositeCodecReaderWrapper.java:118-122)"
        );
        value
    }

    /// The merged view reads every wrapped reader, at its own document base,
    /// through both its producers and its leaf view
    /// (`SlowCompositeCodecReaderWrapper.java:108-1050`). Answering with the
    /// first reader's producers alone would lose documents 2-4.
    #[test]
    fn slow_composite_exposes_every_reader() {
        let readers = two_segments();
        let composite = SlowCompositeCodecReaderWrapper::wrap(readers).unwrap();
        assert_eq!(LeafReader::max_doc(composite.as_ref()), 5);
        assert_eq!(LeafReader::num_docs(composite.as_ref()), 5);

        let expected_numeric: Vec<(i32, i64)> = (0..5).map(|d| (d, i64::from(d) * 10)).collect();
        assert_eq!(
            numeric_pairs(composite.get_numeric_doc_values("n").unwrap().unwrap()),
            expected_numeric
        );
        let n = composite.get_field_infos().field_info("n").unwrap().clone();
        let producer = composite.get_doc_values_reader().unwrap().unwrap();
        assert_eq!(
            numeric_pairs(producer.get_numeric(&n).unwrap().unwrap()),
            expected_numeric
        );
        let s = composite.get_field_infos().field_info("s").unwrap().clone();
        let expected_sorted: Vec<(i32, String)> = (0..5).map(|d| (d, format!("s{d}"))).collect();
        assert_eq!(
            sorted_pairs(producer.get_sorted(&s).unwrap().unwrap()),
            expected_sorted
        );
        // The second call is served from the cached ordinal map
        // (SlowCompositeCodecReaderWrapper.java:411-438).
        assert_eq!(
            sorted_pairs(producer.get_sorted(&s).unwrap().unwrap()),
            expected_sorted
        );

        for d in 0..5 {
            assert_eq!(postings_of(composite.as_ref(), &format!("d{d}")), vec![d]);
            assert_eq!(stored_of(composite.as_ref(), d), format!("v{d}"));
        }
        assert_eq!(
            points_of(composite.as_ref()),
            (0..5).map(|d| (d, d)).collect::<Vec<_>>()
        );
        assert_eq!(
            vectors_of(composite.as_ref()),
            (0..5).map(|d| (d, d as f32)).collect::<Vec<_>>()
        );
        let points = composite.get_point_values("p").unwrap().unwrap();
        assert_eq!(points.size().unwrap(), 5);
        assert_eq!(points.doc_count().unwrap(), 5);
        composite.check_integrity().unwrap();
    }

    /// A sorted view over the merged view answers postings, points and
    /// vectors in the new order, as `SortingCodecReader`'s wrappers do
    /// (`SortingCodecReader.java:448-606`).
    #[test]
    fn sorting_codec_reader_reorders_postings_points_and_vectors() {
        let composite = SlowCompositeCodecReaderWrapper::wrap(two_segments()).unwrap();
        let reverse: crate::index::index_sorter::DocComparator = Box::new(|a, b| b.cmp(&a));
        let doc_map = Arc::new(Sorter::sort(5, &reverse).unwrap());
        let sorted = SortingCodecReader::wrap(composite, doc_map, None).unwrap();
        let new_of = |old: i32| 4 - old;

        for old in 0..5 {
            assert_eq!(postings_of(&sorted, &format!("d{old}")), vec![new_of(old)]);
            assert_eq!(stored_of(&sorted, new_of(old)), format!("v{old}"));
        }
        let mut expected_points: Vec<(i32, i32)> = (0..5).map(|old| (new_of(old), old)).collect();
        expected_points.sort_unstable();
        assert_eq!(points_of(&sorted), expected_points);
        let mut expected_vectors: Vec<(i32, f32)> =
            (0..5).map(|old| (new_of(old), old as f32)).collect();
        expected_vectors.sort_by_key(|(doc, _)| *doc);
        assert_eq!(vectors_of(&sorted), expected_vectors);
        let mut expected_numeric: Vec<(i32, i64)> = (0..5)
            .map(|old| (new_of(old), i64::from(old) * 10))
            .collect();
        expected_numeric.sort_unstable();
        assert_eq!(
            numeric_pairs(sorted.get_numeric_doc_values("n").unwrap().unwrap()),
            expected_numeric
        );
        // The sorted vectors reader does not search
        // (SortingCodecReader.java:585-595).
        let mut collector = crate::search::knn::TopKnnCollector::new(1, i64::MAX);
        let mut accept_docs = crate::search::from_live_docs(None, 5).unwrap();
        assert!(matches!(
            sorted.search_nearest_vectors("v", &[0.0, 1.0], &mut collector, &mut accept_docs),
            Err(LuceneError::UnsupportedOperation(_))
        ));
    }

    /// `wrap` rejects a doc map whose size is not the reader's `maxDoc`
    /// (`SortingCodecReader.java:426-432`).
    #[test]
    fn sorting_codec_reader_rejects_a_mismatched_doc_map() {
        let composite = SlowCompositeCodecReaderWrapper::wrap(two_segments()).unwrap();
        let reverse: crate::index::index_sorter::DocComparator = Box::new(|a, b| b.cmp(&a));
        let doc_map = Arc::new(Sorter::sort(4, &reverse).unwrap());
        assert!(matches!(
            SortingCodecReader::wrap(composite, doc_map, None),
            Err(LuceneError::IllegalArgument(_))
        ));
    }

    /// A postings producer with one field, counting how often its terms are
    /// asked for.
    struct CountingFields(Arc<std::sync::atomic::AtomicUsize>);

    impl Fields for CountingFields {
        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            Box::new(std::iter::once("f".to_string()))
        }

        fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Some(Box::new(crate::index::EmptyTerms::new())))
        }

        fn size(&self) -> i32 {
            1
        }
    }

    impl FieldsProducer for CountingFields {
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
            Ok(Box::new(CountingFields(Arc::clone(&self.0))))
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    /// The composite postings producer builds its `MultiFields` once, so its
    /// per-field memo serves later calls, its merge instance's included
    /// (`SlowCompositeCodecReaderWrapper.java:477-491`, `MultiFields.java:44-60`).
    #[test]
    fn slow_composite_postings_memoise_terms_per_field() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let producers: Vec<Option<Arc<dyn FieldsProducer>>> = vec![
            Some(Arc::new(CountingFields(Arc::clone(&calls)))),
            Some(Arc::new(CountingFields(Arc::clone(&calls)))),
        ];
        let producer = SlowCompositeFieldsProducer::new(producers, &[0, 2, 5]);
        producer.terms("f").unwrap().unwrap();
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "one call per sub"
        );
        producer.terms("f").unwrap().unwrap();
        producer
            .get_merge_instance()
            .unwrap()
            .terms("f")
            .unwrap()
            .unwrap();
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "the memo answers every later call"
        );
    }

    /// Where `MultiDocValues` answers `null`, the composite producers return it
    /// (`SlowCompositeCodecReaderWrapper.java:291-293`, `:412-414`).
    #[test]
    fn slow_composite_producers_answer_none_for_a_field_without_values() {
        let composite = SlowCompositeCodecReaderWrapper::wrap(two_segments()).unwrap();
        // `s` holds sorted doc values only, `id` indexes no norms.
        let s = composite.get_field_infos().field_info("s").unwrap().clone();
        let producer = composite.get_doc_values_reader().unwrap().unwrap();
        assert!(producer.get_numeric(&s).unwrap().is_none());
        assert!(producer.get_binary(&s).unwrap().is_none());
        assert!(producer.get_sorted_numeric(&s).unwrap().is_none());
        assert!(producer.get_sorted_set(&s).unwrap().is_none());
        let id = composite
            .get_field_infos()
            .field_info("id")
            .unwrap()
            .clone();
        let norms = composite.get_norms_reader().unwrap().unwrap();
        assert!(norms.get_norms(&id).unwrap().is_none());
    }

    /// The sorted views only iterate: `advance` throws, as Java's
    /// `Sorting*DocValues.advance` do (`NumericDocValuesWriter.java:256-259`).
    #[test]
    fn sorted_doc_values_views_do_not_advance() {
        let composite = SlowCompositeCodecReaderWrapper::wrap(two_segments()).unwrap();
        let reverse: crate::index::index_sorter::DocComparator = Box::new(|a, b| b.cmp(&a));
        let doc_map = Arc::new(Sorter::sort(5, &reverse).unwrap());
        let sorted = SortingCodecReader::wrap(composite, doc_map, None).unwrap();
        let mut numeric = sorted.get_numeric_doc_values("n").unwrap().unwrap();
        assert!(matches!(
            numeric.advance(1),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        let mut values = sorted.get_sorted_doc_values("s").unwrap().unwrap();
        assert!(matches!(
            values.advance(1),
            Err(LuceneError::UnsupportedOperation(_))
        ));
    }

    /// `SlowCodecReaderWrapper`'s norms producer answers the leaf's
    /// `getNormValues`, `null` included (`SlowCodecReaderWrapper.java:225-228`).
    #[test]
    fn slow_codec_reader_norms_pass_null_through() {
        let readers = two_segments();
        let leaf: Arc<dyn LeafReader> = readers[0].clone();
        let codec =
            crate::index::filter_directory_reader::SlowCodecReaderWrapper::wrap(leaf).unwrap();
        let id = codec.get_field_infos().field_info("id").unwrap().clone();
        let norms = codec.get_norms_reader().unwrap().unwrap();
        assert!(
            norms.get_norms(&id).unwrap().is_none(),
            "`id` indexes no norms"
        );
    }

    /// A stored-fields reader that counts its closes.
    #[derive(Debug)]
    struct ClosingStoredFields(Arc<std::sync::atomic::AtomicUsize>);

    impl StoredFieldsReader for ClosingStoredFields {
        fn document(&self, _doc_id: i32, _visitor: &mut dyn StoredFieldVisitor) -> Result<()> {
            Ok(())
        }

        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn clone_reader(&self) -> Box<dyn StoredFieldsReader> {
            Box::new(ClosingStoredFields(Arc::clone(&self.0)))
        }

        fn close(&mut self) -> Result<()> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    /// The sorted stored-fields reader closes its delegate
    /// (`SortingCodecReader.java:517-520`).
    #[test]
    fn sorted_stored_fields_reader_closes_its_delegate() {
        let closes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reverse: crate::index::index_sorter::DocComparator = Box::new(|a, b| b.cmp(&a));
        let mut reader = SortingStoredFieldsReader {
            delegate: Box::new(ClosingStoredFields(Arc::clone(&closes))),
            doc_map: Arc::new(Sorter::sort(2, &reverse).unwrap()),
        };
        reader.close().unwrap();
        assert_eq!(closes.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// `MergedFloatVectorValues.copy()` copies every sub, so a reader without
    /// vectors makes it fail (`SlowCompositeCodecReaderWrapper.java:297-299`).
    #[test]
    fn merged_vector_values_copy_fails_over_a_reader_without_vectors() {
        let with_vectors: Box<dyn FloatVectorValues> =
            Box::new(crate::index::from_floats(vec![vec![1.0, 2.0]], 2));
        let subs = vec![
            DocValuesSub {
                sub: Some(with_vectors),
                doc_start: 0,
                ord_start: 0,
            },
            DocValuesSub {
                sub: None,
                doc_start: 1,
                ord_start: 1,
            },
        ];
        let values = MergedFloatVectorValues::new(2, 1, subs).unwrap();
        assert_eq!(values.vector_value(0).unwrap(), vec![1.0, 2.0]);
        assert!(values.copy_float().is_err());
    }

    /// Point values whose bounds cannot be read.
    struct NoBounds;

    impl PointValues for NoBounds {
        fn point_tree(&self) -> Result<Box<dyn PointTree>> {
            Err(LuceneError::illegal_state("no tree".to_string()))
        }

        fn size(&self) -> Result<i64> {
            Ok(5)
        }

        fn doc_count(&self) -> Result<i32> {
            Ok(5)
        }

        fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
            Err(LuceneError::illegal_state("no bounds".to_string()))
        }

        fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
            Err(LuceneError::illegal_state("no bounds".to_string()))
        }

        fn num_dimensions(&self) -> Result<i32> {
            Ok(1)
        }

        fn num_index_dimensions(&self) -> Result<i32> {
            Ok(1)
        }

        fn bytes_per_dimension(&self) -> Result<i32> {
            Ok(4)
        }
    }

    /// The composite tree computes its bounds only when asked for them
    /// (`SlowCompositeCodecReaderWrapper.java:677-751`), so `size()` sums the
    /// subs' sizes without reading any bound (`:768-775`).
    #[test]
    fn composite_point_size_does_not_read_the_bounds() {
        let values = SlowCompositePointValues {
            values: Arc::new(vec![
                PointValuesSub {
                    sub: Box::new(NoBounds),
                    doc_base: 0,
                },
                PointValuesSub {
                    sub: Box::new(NoBounds),
                    doc_base: 5,
                },
            ]),
        };
        assert_eq!(values.size().unwrap(), 10);
        assert!(values.min_packed_value().is_err());
    }
}
