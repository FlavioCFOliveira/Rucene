//! `SortingCodecReader` and the merged-segment warmer, ported from
//! `org.apache.lucene.index`.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::codecs::doc_values::DocValuesProducer;
use crate::codecs::knn_vectors::KnnVectorsReader;
use crate::codecs::norms::NormsProducer;
use crate::codecs::points::PointsReader;
use crate::codecs::postings::FieldsProducer;
use crate::codecs::stored_fields::StoredFieldsReader;
use crate::codecs::term_vectors::TermVectorsReader;
use crate::error::Result;
use crate::index::codec_reader::CodecReader;
use crate::index::index_reader::{CacheHelper, IndexReaderCore, StoredFields};
use crate::index::index_sorter::SorterDocMap;
use crate::index::leaf_reader::{LeafMetaData, LeafReader, TermVectors};
use crate::index::{
    BinaryDocValues, ByteVectorValues, DocIndexIterator, DocValuesIterator, DocValuesSkipper,
    EmptyDocValuesSkipper, FieldInfo, FieldInfos, FieldInfosBuilder, FieldNumbers, Fields,
    FloatVectorValues, IntersectVisitor, KnnVectorValues, NumericDocValues, PointTree, PointValues,
    SortedDocValues, SortedNumericDocValues, SortedSetDocValues, StoredFieldVisitor, Terms,
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
/// The value-level reordering Java performs is implemented here: the
/// doc-values, norms, stored-fields and term-vectors producers, and the live
/// docs, are wrapped so that they answer in the new document order, and the
/// last materialised doc-values or norms view is cached exactly as Java caches
/// it — see [`SortingCodecReaderState::get_or_create`].
///
/// **Divergence from Lucene 10.5.0.** Three of Java's wrappers cannot be
/// installed yet, so the producers that would carry them still forward
/// unchanged:
///
/// * `getPostingsReader()`, which Java wraps in a `FieldsProducer` whose
///   `terms(String)` answers with `FreqProxTermsWriter.SortingTerms`
///   (`SortingCodecReader.java:448-484`). That class is not ported — see the
///   note at the top of `freq_prox_terms_writer.rs`.
/// * `getPointsReader()`, which Java wraps in `SortingPointValues`
///   (`SortingCodecReader.java:71-120`, `534-560`).
/// * `getVectorReader()`, which Java wraps in `SortingFloatVectorValues` and
///   `SortingByteVectorValues` (`SortingCodecReader.java:311-384`, `562-606`).
///   The mapping machinery those two need *is* ported, as
///   [`SortingCodecReader::iterator_supplier`].
pub struct SortingCodecReader {
    inner: Arc<dyn CodecReader>,
    doc_map: Arc<SorterDocMap>,
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
    /// Wraps `reader` so its documents appear in the order `doc_map` gives.
    ///
    /// Equivalent to `SortingCodecReader.wrap(CodecReader, Sorter.DocMap, Sort)`.
    pub fn wrap(inner: Arc<dyn CodecReader>, doc_map: Arc<SorterDocMap>) -> Self {
        let state = Arc::new(SortingCodecReaderState {
            sort: inner.get_meta_data().sort().cloned(),
            cache: Mutex::new(SortedViewCache::default()),
        });
        Self {
            inner,
            doc_map,
            state,
        }
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

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.inner.term_vectors()
    }

    fn num_docs(&self) -> i32 {
        self.inner.num_docs()
    }

    fn max_doc(&self) -> i32 {
        self.inner.max_doc()
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.inner.stored_fields()
    }

    fn do_close(&self) -> Result<()> {
        self.inner.do_close()
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        // A sorted view is not the same reader, so it must not share a cache key.
        None
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.inner.terms(field)
    }

    fn get_numeric_doc_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.inner.get_numeric_doc_values(field)
    }

    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        self.inner.get_binary_doc_values(field)
    }

    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        self.inner.get_sorted_doc_values(field)
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        self.inner.get_sorted_numeric_doc_values(field)
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        self.inner.get_sorted_set_doc_values(field)
    }

    fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.inner.get_norm_values(field)
    }

    fn get_doc_values_skipper(&self, field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        self.inner.get_doc_values_skipper(field)
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        self.inner.get_float_vector_values(field)
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        self.inner.get_byte_vector_values(field)
    }

    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        self.inner
            .search_nearest_vectors(field, target, collector, accept_docs)
    }

    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        self.inner
            .search_nearest_vectors_byte(field, target, collector, accept_docs)
    }

    fn get_field_infos(&self) -> FieldInfos {
        self.inner.get_field_infos()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        // Equivalent to `SortingCodecReader.getLiveDocs()`, which wraps the
        // delegate's live docs in `SortingBits`.
        self.inner.get_live_docs().map(|in_live_docs| {
            Box::new(SortingBits {
                inner: in_live_docs,
                doc_map: Arc::clone(&self.doc_map),
            }) as Box<dyn Bits>
        })
    }

    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        self.inner.get_point_values(field)
    }

    fn check_integrity(&self) -> Result<()> {
        self.inner.check_integrity()
    }

    fn get_meta_data(&self) -> LeafMetaData {
        self.inner.get_meta_data()
    }
}

impl CodecReader for SortingCodecReader {
    fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        // Equivalent to `SortingCodecReader.getFieldsReader()`.
        match self.inner.get_fields_reader()? {
            None => Ok(None),
            Some(delegate) => Ok(Some(self.new_stored_fields_reader(delegate))),
        }
    }

    fn get_term_vectors_reader(&self) -> Result<Option<Box<dyn TermVectorsReader>>> {
        // Equivalent to `SortingCodecReader.getTermVectorsReader()`.
        Ok(self.new_term_vectors_reader(self.inner.get_term_vectors_reader()?))
    }

    fn get_norms_reader(&self) -> Result<Option<Arc<dyn NormsProducer>>> {
        // Equivalent to `SortingCodecReader.getNormsReader()`.
        match self.inner.get_norms_reader()? {
            None => Ok(None),
            Some(delegate) => Ok(Some(Arc::new(SortingNormsProducer {
                delegate,
                doc_map: Arc::clone(&self.doc_map),
                max_doc: self.max_doc(),
                state: Arc::clone(&self.state),
            }))),
        }
    }

    fn get_doc_values_reader(&self) -> Result<Option<Arc<dyn DocValuesProducer>>> {
        // Equivalent to `SortingCodecReader.getDocValuesReader()`.
        match self.inner.get_doc_values_reader()? {
            None => Ok(None),
            Some(delegate) => Ok(Some(Arc::new(SortingDocValuesProducer {
                delegate,
                doc_map: Arc::clone(&self.doc_map),
                max_doc: self.max_doc(),
                state: Arc::clone(&self.state),
            }))),
        }
    }

    fn get_postings_reader(&self) -> Result<Option<Arc<dyn FieldsProducer>>> {
        self.inner.get_postings_reader()
    }

    fn get_points_reader(&self) -> Result<Option<Arc<dyn PointsReader>>> {
        self.inner.get_points_reader()
    }

    fn get_vector_reader(&self) -> Result<Option<Arc<dyn KnnVectorsReader>>> {
        self.inner.get_vector_reader()
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
    /// Equivalent to Java's `metaData.sort()`. Java's `metaData` is the
    /// *new* `LeafMetaData` that `wrap(CodecReader, Sorter.DocMap, Sort)`
    /// builds around the sort it is given
    /// (`SortingCodecReader.java:398-402`); this port's
    /// [`wrap`](SortingCodecReader::wrap) takes no sort and its
    /// `get_meta_data` forwards the wrapped reader's, so this is that
    /// forwarded sort. Until that pre-existing gap is closed, a segment being
    /// sorted for the first time reports no sort here, and the
    /// `is_sort_field` half of the assertion under-reports for it.
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
            crate::error::LuceneError::IllegalState(format!(
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

/// Warms a merged segment by touching each kind of data it holds, so the first
/// search does not pay for the cold read.
///
/// Equivalent to `org.apache.lucene.index.SimpleMergedSegmentWarmer`.
pub struct SimpleMergedSegmentWarmer {
    info_stream: Arc<dyn InfoStream>,
}

impl SimpleMergedSegmentWarmer {
    /// Creates the warmer.
    pub fn new(info_stream: Arc<dyn InfoStream>) -> Self {
        Self { info_stream }
    }

    /// Touches every indexed field, doc-values field, norm and vector of
    /// `reader`.
    ///
    /// Equivalent to `SimpleMergedSegmentWarmer.warm(LeafReader)`.
    pub fn warm(&self, reader: &dyn LeafReader) -> Result<()> {
        let start = std::time::Instant::now();
        let mut indexed_count = 0;
        let mut doc_values_count = 0;
        let mut norms_count = 0;

        let field_infos = reader.get_field_infos();
        for info in field_infos.iter() {
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
                    crate::index::DocValuesType::NONE => {}
                }
                doc_values_count += 1;
            }
        }

        if self.info_stream.is_enabled("SMSW") {
            self.info_stream.message(
                "SMSW",
                &format!(
                    "warmed segment: {indexed_count} indexed fields, {doc_values_count} doc values fields, {norms_count} norms fields in {} ms",
                    start.elapsed().as_millis()
                ),
            );
        }
        Ok(())
    }
}

/// Presents several codec readers as one, concatenating their documents.
///
/// Equivalent to `org.apache.lucene.index.SlowCompositeCodecReaderWrapper`,
/// which `IndexWriter.addIndexes(CodecReader...)` uses to merge a composite
/// reader as if it were a single segment.
///
/// **Divergence from Lucene 10.5.0.** Java synthesises a producer of each kind
/// that concatenates the corresponding producers of the wrapped readers, so a
/// merge reads straight through the composite. Those seven concatenating
/// producers need the producer traits to be implementable outside their codec
/// modules, which this port does not yet allow. This wrapper therefore carries
/// the readers and the document-start offsets — the arithmetic every one of
/// those producers needs — and serves the reader-level API; the concatenating
/// producers are the remaining half.
pub struct SlowCompositeCodecReaderWrapper {
    readers: Vec<Arc<dyn CodecReader>>,
    /// First document number of each reader, plus a trailing `max_doc`.
    doc_starts: Vec<i32>,
    max_doc: i32,
    num_docs: i32,
    /// The field infos of every wrapped reader, merged.
    ///
    /// Equivalent to `SlowCompositeCodecReaderWrapper.fieldInfos`
    /// (`SlowCompositeCodecReaderWrapper.java:92-96`), which the constructor
    /// builds with `FieldInfos.getMergedFieldInfos(multiReader)` so that
    /// [`remap`](Self::remap) always answers with the composite's own field
    /// numbering rather than a leaf's.
    field_infos: FieldInfos,
}

impl std::fmt::Debug for SlowCompositeCodecReaderWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlowCompositeCodecReaderWrapper")
            .field("readers", &self.readers.len())
            .field("max_doc", &self.max_doc)
            .finish()
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
    /// Equivalent to `SlowCompositeCodecReaderWrapper.wrap(List<CodecReader>)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when `readers` is empty, and propagates whatever merging the readers'
    /// field infos raises.
    pub fn wrap(readers: Vec<Arc<dyn CodecReader>>) -> Result<Arc<dyn CodecReader>> {
        match readers.len() {
            0 => Err(crate::error::LuceneError::IllegalArgument(
                "Must take at least one reader, got 0".to_string(),
            )),
            1 => Ok(Arc::clone(&readers[0])),
            _ => Ok(Arc::new(Self::new(readers)?)),
        }
    }

    fn new(readers: Vec<Arc<dyn CodecReader>>) -> Result<Self> {
        let mut doc_starts = vec![0i32; readers.len() + 1];
        let mut doc_start = 0i32;
        let mut num_docs = 0i32;
        for (i, reader) in readers.iter().enumerate() {
            doc_start += reader.max_doc();
            doc_starts[i + 1] = doc_start;
            num_docs += reader.num_docs();
        }
        let field_infos = Self::merge_field_infos(&readers)?;
        Ok(Self {
            readers,
            doc_starts,
            max_doc: doc_start,
            num_docs,
            field_infos,
        })
    }

    /// Merges the field infos of every reader in `readers`.
    ///
    /// Equivalent to the constructor's
    /// `fieldInfos = FieldInfos.getMergedFieldInfos(multiReader)`
    /// (`SlowCompositeCodecReaderWrapper.java:92-96`). Java builds a
    /// `MultiReader` over the leaves and calls
    /// [`FieldInfos::get_merged_field_infos`](FieldInfos::get_merged_field_infos),
    /// which walks `IndexReader::leaves()`; this wrapper already holds its
    /// leaves as a flat `Vec<Arc<dyn CodecReader>>`, so it merges them
    /// directly with the same algorithm
    /// (`FieldInfos.getMergedFieldInfos`/`getAndValidateParentField`) instead
    /// of first wrapping every reader to satisfy the generic
    /// `IndexReader`-based entry point.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`](crate::error::LuceneError::IllegalState)
    /// when the readers disagree about the parent field, and propagates
    /// whatever adding a field to the merged builder raises.
    fn merge_field_infos(readers: &[Arc<dyn CodecReader>]) -> Result<FieldInfos> {
        if readers.len() == 1 {
            return Ok(readers[0].get_field_infos());
        }
        let soft_deletes_field = readers.iter().find_map(|reader| {
            reader
                .get_field_infos()
                .get_soft_deletes_field()
                .map(str::to_string)
        });
        let mut parent_field: Option<String> = None;
        let mut parent_field_set = false;
        for reader in readers {
            let field = reader
                .get_field_infos()
                .get_parent_field()
                .map(str::to_string);
            if parent_field_set && field != parent_field {
                return Err(crate::error::LuceneError::IllegalState(format!(
                    "expected parent doc field to be \"{} \" across all segments but found a \
                     segment with different field \"{}\"",
                    parent_field.as_deref().unwrap_or("null"),
                    field.as_deref().unwrap_or("null")
                )));
            }
            parent_field = field;
            parent_field_set = true;
        }
        let numbers = Arc::new(FieldNumbers::new(soft_deletes_field, parent_field)?);
        let mut builder = FieldInfosBuilder::new(numbers);
        for reader in readers {
            for field_info in reader.get_field_infos().iter() {
                builder.add(field_info)?;
            }
        }
        builder.finish()
    }

    /// Remaps a leaf's [`FieldInfo`] to the one the composite reader's own,
    /// merged field infos carry for the same name.
    ///
    /// Equivalent to `SlowCompositeCodecReaderWrapper.remap(FieldInfo)`
    /// (`SlowCompositeCodecReaderWrapper.java:121-123`); used so that a caller
    /// walking a wrapped reader's stored fields always sees the composite's
    /// own field numbering rather than a leaf's.
    ///
    /// # Panics
    ///
    /// Panics if `info`'s name is absent from the merged field infos, which
    /// cannot happen for a `FieldInfo` obtained from one of the wrapped
    /// readers, since every leaf's fields were folded into them in
    /// [`new`](Self::new).
    fn remap(&self, info: &FieldInfo) -> FieldInfo {
        self.field_infos
            .field_info(&info.name)
            .expect("INVARIANT: every leaf field is present in the merged field infos")
            .clone()
    }

    /// Returns the index of the reader that owns `doc_id`, and the document's
    /// number within it.
    pub fn reader_for_doc(&self, doc_id: i32) -> Option<(usize, i32)> {
        if doc_id < 0 || doc_id >= self.max_doc {
            return None;
        }
        let hi = self.readers.len();
        let index = match self.doc_starts[..hi].binary_search(&doc_id) {
            Ok(index) => index,
            Err(insertion) => insertion - 1,
        };
        Some((index, doc_id - self.doc_starts[index]))
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

impl LeafReader for SlowCompositeCodecReaderWrapper {
    fn core(&self) -> &IndexReaderCore {
        self.readers[0].core()
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.readers[0].term_vectors()
    }

    fn num_docs(&self) -> i32 {
        self.num_docs
    }

    fn max_doc(&self) -> i32 {
        self.max_doc
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.readers[0].stored_fields()
    }

    fn do_close(&self) -> Result<()> {
        let mut first_error = None;
        for reader in &self.readers {
            if let Err(err) = reader.do_close() {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
        match first_error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.readers[0].terms(field)
    }

    fn get_numeric_doc_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.readers[0].get_numeric_doc_values(field)
    }

    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        self.readers[0].get_binary_doc_values(field)
    }

    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        self.readers[0].get_sorted_doc_values(field)
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        self.readers[0].get_sorted_numeric_doc_values(field)
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        self.readers[0].get_sorted_set_doc_values(field)
    }

    fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.readers[0].get_norm_values(field)
    }

    fn get_doc_values_skipper(&self, field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        self.readers[0].get_doc_values_skipper(field)
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        self.readers[0].get_float_vector_values(field)
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        self.readers[0].get_byte_vector_values(field)
    }

    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        self.readers[0].search_nearest_vectors(field, target, collector, accept_docs)
    }

    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        self.readers[0].search_nearest_vectors_byte(field, target, collector, accept_docs)
    }

    fn get_field_infos(&self) -> FieldInfos {
        self.readers[0].get_field_infos()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.readers[0].get_live_docs()
    }

    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        self.readers[0].get_point_values(field)
    }

    fn check_integrity(&self) -> Result<()> {
        for reader in &self.readers {
            reader.check_integrity()?;
        }
        Ok(())
    }

    fn get_meta_data(&self) -> LeafMetaData {
        self.readers[0].get_meta_data()
    }
}

impl CodecReader for SlowCompositeCodecReaderWrapper {
    fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        self.readers[0].get_fields_reader()
    }

    fn get_term_vectors_reader(&self) -> Result<Option<Box<dyn TermVectorsReader>>> {
        self.readers[0].get_term_vectors_reader()
    }

    fn get_norms_reader(&self) -> Result<Option<Arc<dyn NormsProducer>>> {
        self.readers[0].get_norms_reader()
    }

    fn get_doc_values_reader(&self) -> Result<Option<Arc<dyn DocValuesProducer>>> {
        self.readers[0].get_doc_values_reader()
    }

    fn get_postings_reader(&self) -> Result<Option<Arc<dyn FieldsProducer>>> {
        self.readers[0].get_postings_reader()
    }

    fn get_points_reader(&self) -> Result<Option<Arc<dyn PointsReader>>> {
        self.readers[0].get_points_reader()
    }

    fn get_vector_reader(&self) -> Result<Option<Arc<dyn KnnVectorsReader>>> {
        self.readers[0].get_vector_reader()
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
        let start = if self.doc_id < 0 { 0 } else { self.doc_id + 1 };
        self.doc_id = self.dvs.docs_with_field.next_set_bit(start);
        Ok(self.doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc_id = self.dvs.docs_with_field.next_set_bit(target);
        Ok(self.doc_id)
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
        let start = if self.doc_id < 0 { 0 } else { self.doc_id + 1 };
        self.doc_id = self.dvs.docs_with_field.next_set_bit(start);
        Ok(self.doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc_id = self.dvs.docs_with_field.next_set_bit(target);
        Ok(self.doc_id)
    }

    fn cost(&self) -> i64 {
        self.dvs.docs_with_field.cardinality() as i64
    }
}

impl DocValuesIterator for SortingBinaryDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc_id = target;
        Ok(self.dvs.docs_with_field.get(target as usize))
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
        let start = if self.doc_id < 0 { 0 } else { self.doc_id + 1 };
        self.doc_id = self.dvs.docs_with_field.next_set_bit(start);
        Ok(self.doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc_id = self.dvs.docs_with_field.next_set_bit(target);
        Ok(self.doc_id)
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
        let start = if self.doc_id < 0 { 0 } else { self.doc_id + 1 };
        self.doc_id = self.values.docs_with_field.next_set_bit(start);
        self.value_idx = 0;
        Ok(self.doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc_id = self.values.docs_with_field.next_set_bit(target);
        self.value_idx = 0;
        Ok(self.doc_id)
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
        let start = if self.doc_id < 0 { 0 } else { self.doc_id + 1 };
        self.doc_id = self.dvs.docs_with_field.next_set_bit(start);
        self.ord_idx = 0;
        Ok(self.doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc_id = self.dvs.docs_with_field.next_set_bit(target);
        self.ord_idx = 0;
        Ok(self.doc_id)
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

    fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
        self.delegate.get(self.doc_map.new_to_old(doc))
    }

    fn prefetch(&self, doc_id: i32) -> Result<()> {
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
    fn get_numeric(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
        let dvs = self.state.get_or_create_dv(&field.name, || {
            let mut old = self.delegate.get_numeric(field)?;
            NumericDVs::new(self.max_doc, &self.doc_map, &mut *old)
        })?;
        Ok(Box::new(SortingNumericDocValues::new(dvs)))
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Box<dyn BinaryDocValues>> {
        let dvs = self.state.get_or_create_dv(&field.name, || {
            let mut old = self.delegate.get_binary(field)?;
            BinaryDVs::new(self.max_doc, &self.doc_map, &mut *old)
        })?;
        Ok(Box::new(SortingBinaryDocValues::new(dvs)))
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Box<dyn SortedDocValues>> {
        // As in Java, the old values are pulled outside the supplier: the
        // sorted view keeps them for its term dictionary even when the
        // reordered ordinals come from the cache and the supplier never runs.
        let mut old = self.delegate.get_sorted(field)?;
        let dvs = self.state.get_or_create_dv(&field.name, || {
            SortedDVs::new(self.max_doc, &self.doc_map, &mut *old)
        })?;
        Ok(Box::new(SortingSortedDocValues::new(old, dvs)))
    }

    fn get_sorted_numeric(&self, field: &FieldInfo) -> Result<Box<dyn SortedNumericDocValues>> {
        let mut old = self.delegate.get_sorted_numeric(field)?;
        let values = self.state.get_or_create_dv(&field.name, || {
            LongValues::new(self.max_doc, &self.doc_map, &mut *old)
        })?;
        Ok(Box::new(SortingSortedNumericDocValues::new(values)))
    }

    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Box<dyn SortedSetDocValues>> {
        let mut old = self.delegate.get_sorted_set(field)?;
        let dvs = self.state.get_or_create_dv(&field.name, || {
            DocOrds::new(self.max_doc, &self.doc_map, &mut *old)
        })?;
        Ok(Box::new(SortingSortedSetDocValues::new(old, dvs)))
    }

    fn get_skipper(&self, _field: &FieldInfo) -> Result<Box<dyn DocValuesSkipper>> {
        // Equivalent to `SortingCodecReader.getDocValuesReader().getSkipper`,
        // which returns `null`: min/max-per-block skip metadata is meaningless
        // once documents have been reordered, so no skip index is offered.
        Ok(Box::new(EmptyDocValuesSkipper))
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
/// `SortingCodecReader.getNormsReader()` builds.
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
    fn get_norms(&self, field_info: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
        let dvs = self.state.get_or_create_norms(&field_info.name, || {
            let mut old = self.delegate.get_norms(field_info)?;
            NumericDVs::new(self.max_doc, &self.doc_map, &mut *old)
        })?;
        Ok(Box::new(SortingNumericDocValues::new(dvs)))
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
