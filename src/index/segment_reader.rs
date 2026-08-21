//! `SegmentReader`, `SegmentCoreReaders`, `SegmentDocValues`,
//! `SegmentDocValuesProducer` and `DocValuesLeafReader` ported from
//! `org.apache.lucene.index`.
//!
//! These types load a single segment's codec files into producers and expose the
//! [`LeafReader`] API. `SegmentCoreReaders` holds the shared readers (postings,
//! stored fields, term vectors, points, vectors, norms and field infos) and is
//! reference-counted so that reopened/cloned `SegmentReader`s can share the
//! same core data.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::{Debug, Formatter};
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};

use crate::codecs::compound::CompoundDirectory;
use crate::codecs::doc_values::{
    BinaryDocValues, DocValuesProducer, NumericDocValues, SortedDocValues, SortedNumericDocValues,
    SortedSetDocValues,
};
use crate::codecs::knn_vectors::KnnVectorsReader;
use crate::codecs::norms::NormsProducer;
use crate::codecs::points::PointsReader;
use crate::codecs::postings::FieldsProducer;
use crate::codecs::stored_fields::StoredFieldsReader;
use crate::codecs::stub::StoredFieldVisitor;
use crate::codecs::term_vectors::TermVectorsReader;
use crate::codecs::DocValuesSkipper;
use crate::document::{Document, NumericValue};
use crate::error::{LuceneError, Result};
use crate::index::index_reader::{
    CacheHelper, CacheKey, ClosedListener, IndexReaderCore, StoredFields,
};
use crate::index::leaf_reader::{LeafMetaData, LeafReader, TermVectors};
use crate::index::{
    BinaryDocValues as IndexBinaryDocValues, ByteVectorValues,
    DocValuesSkipper as IndexDocValuesSkipper, DocValuesType, FieldInfo, FieldInfos,
    FloatVectorValues, NumericDocValues as IndexNumericDocValues, PointValues as IndexPointValues,
    SegmentCommitInfo, SortedDocValues as IndexSortedDocValues,
    SortedNumericDocValues as IndexSortedNumericDocValues,
    SortedSetDocValues as IndexSortedSetDocValues, Terms, VectorEncoding,
};
use crate::search::knn::KnnCollector;
use crate::search::AcceptDocs;
use crate::store::{Directory, IOContext, READONCE_IO_CONTEXT};
use crate::util::Bits;
use crate::util::BytesRef;

// -----------------------------------------------------------------------------
// SegmentCoreReaders
// -----------------------------------------------------------------------------

/// Holds the core codec readers that are shared (unchanged) when a
/// `SegmentReader` is cloned or reopened.
///
/// Equivalent to `org.apache.lucene.index.SegmentCoreReaders`.
pub struct SegmentCoreReaders {
    fields: RwLock<Option<Box<dyn FieldsProducer>>>,
    norms_producer: RwLock<Option<Box<dyn NormsProducer>>>,
    fields_reader_orig: RwLock<Option<Box<dyn StoredFieldsReader>>>,
    term_vectors_reader_orig: RwLock<Option<Box<dyn TermVectorsReader>>>,
    points_reader: RwLock<Option<Box<dyn PointsReader>>>,
    knn_vectors_reader: RwLock<Option<Box<dyn KnnVectorsReader>>>,
    cfs_reader: RwLock<Option<Box<dyn CompoundDirectory>>>,
    segment: String,
    core_field_infos: FieldInfos,
    ref_count: AtomicI32,
    core_cache_key: CacheKey,
    core_closed_listeners: Mutex<Vec<Box<dyn ClosedListener>>>,
}

impl Debug for SegmentCoreReaders {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentCoreReaders")
            .field("segment", &self.segment)
            .field("ref_count", &self.ref_count.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl SegmentCoreReaders {
    /// Creates a new core and opens all of the segment's codec files.
    ///
    /// # Errors
    ///
    /// Returns a `CorruptIndexException` if any required file is missing or
    /// truncated, or any other I/O error thrown by the codec.
    pub fn new(
        dir: &dyn Directory,
        si: &SegmentCommitInfo,
        context: &dyn IOContext,
    ) -> Result<Self> {
        let codec = si
            .info
            .codec()
            .ok_or_else(|| LuceneError::IllegalState("segment has no codec".to_string()))?;

        let cfs_reader: Option<Box<dyn CompoundDirectory>> = if si.info.get_use_compound_file() {
            Some(codec.compound_format().get_compound_reader(dir, &si.info)?)
        } else {
            None
        };

        let mut core = Self {
            fields: RwLock::new(None),
            norms_producer: RwLock::new(None),
            fields_reader_orig: RwLock::new(None),
            term_vectors_reader_orig: RwLock::new(None),
            points_reader: RwLock::new(None),
            knn_vectors_reader: RwLock::new(None),
            cfs_reader: RwLock::new(cfs_reader),
            segment: si.info.name.clone(),
            core_field_infos: FieldInfos::empty(),
            ref_count: AtomicI32::new(1),
            core_cache_key: CacheKey,
            core_closed_listeners: Mutex::new(Vec::new()),
        };

        let result: Result<()> = (|| {
            let cfs_dir_guard = core.cfs_reader.read().map_err(|_| {
                LuceneError::IllegalState("compound file reader lock poisoned".to_string())
            })?;
            let cfs_dir: &dyn Directory = if let Some(cfs) = cfs_dir_guard.as_ref() {
                cfs.as_ref()
            } else {
                dir
            };

            core.core_field_infos = codec
                .field_infos_format()
                .read(cfs_dir, &si.info, "", context)?;

            let segment_read_state = crate::codecs::state::SegmentReadState::new(
                cfs_dir,
                &si.info,
                &core.core_field_infos,
                context,
            );

            if core.core_field_infos.has_postings() {
                *core.fields.write().map_err(|_| {
                    LuceneError::IllegalState("fields producer lock poisoned".to_string())
                })? = Some(
                    codec
                        .postings_format()
                        .fields_producer(&segment_read_state)?,
                );
            }

            if core.core_field_infos.has_norms() {
                *core.norms_producer.write().map_err(|_| {
                    LuceneError::IllegalState("norms producer lock poisoned".to_string())
                })? = Some(codec.norms_format().norms_producer(&segment_read_state)?);
            }

            *core.fields_reader_orig.write().map_err(|_| {
                LuceneError::IllegalState("stored fields reader lock poisoned".to_string())
            })? = Some(codec.stored_fields_format().fields_reader(
                cfs_dir,
                &si.info,
                &core.core_field_infos,
                context,
            )?);

            if core.core_field_infos.has_term_vectors() {
                *core.term_vectors_reader_orig.write().map_err(|_| {
                    LuceneError::IllegalState("term vectors reader lock poisoned".to_string())
                })? = Some(codec.term_vectors_format().vectors_reader(
                    cfs_dir,
                    &si.info,
                    &core.core_field_infos,
                    context,
                )?);
            }

            if core.core_field_infos.has_point_values() {
                *core.points_reader.write().map_err(|_| {
                    LuceneError::IllegalState("points reader lock poisoned".to_string())
                })? = Some(codec.points_format().fields_reader(&segment_read_state)?);
            }

            if core.core_field_infos.has_vector_values() {
                *core.knn_vectors_reader.write().map_err(|_| {
                    LuceneError::IllegalState("knn vectors reader lock poisoned".to_string())
                })? = Some(
                    codec
                        .knn_vectors_format()
                        .fields_reader(&segment_read_state)?,
                );
            }

            drop(cfs_dir_guard);
            Ok(())
        })();

        let success = result.is_ok();
        if !success {
            // Close any readers that were successfully opened before the
            // failure. This mirrors Java's try/finally cleanup.
            let _ = core.dec_ref();
        }
        result?;
        Ok(core)
    }

    /// Returns the field infos that were loaded for this core.
    pub fn core_field_infos(&self) -> &FieldInfos {
        &self.core_field_infos
    }

    /// Returns the number of live references to this core.
    pub fn ref_count(&self) -> i32 {
        self.ref_count.load(Ordering::SeqCst)
    }

    /// Increments the reference count of this core.
    ///
    /// # Errors
    ///
    /// Returns `AlreadyClosed` if the core has already been closed.
    pub fn inc_ref(&self) -> Result<()> {
        let mut count = self.ref_count.load(Ordering::SeqCst);
        while count > 0 {
            match self.ref_count.compare_exchange_weak(
                count,
                count + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return Ok(()),
                Err(actual) => count = actual,
            }
        }
        Err(LuceneError::AlreadyClosed(
            "SegmentCoreReaders is already closed".to_string(),
        ))
    }

    /// Decrements the reference count of this core, closing all codec readers
    /// when it reaches zero.
    pub fn dec_ref(&self) -> Result<()> {
        let rc = self.ref_count.fetch_sub(1, Ordering::SeqCst) - 1;
        if rc == 0 {
            let listener_result = self.notify_core_closed_listeners();
            let close_result = self.close_producers();
            listener_result?;
            return close_result;
        } else if rc < 0 {
            return Err(LuceneError::IllegalState(format!(
                "too many decRef calls: refCount is {rc} after decrement"
            )));
        }
        Ok(())
    }

    fn close_producers(&self) -> Result<()> {
        let mut first_error: Option<LuceneError> = None;

        let mut record = |result: Result<()>| {
            if let Err(err) = result {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        };

        record(close_lock(&self.fields, |producer| producer.close()));
        record(close_lock(&self.norms_producer, |producer| {
            producer.close()
        }));
        record(close_lock(&self.fields_reader_orig, |reader| {
            reader.close()
        }));
        record(close_lock(&self.term_vectors_reader_orig, |reader| {
            reader.close()
        }));
        record(close_lock(&self.points_reader, |producer| producer.close()));
        record(close_lock(&self.knn_vectors_reader, |producer| {
            producer.close()
        }));

        if let Ok(mut guard) = self.cfs_reader.write() {
            if let Some(mut reader) = guard.take() {
                record(reader.close());
            }
        }

        match first_error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn notify_core_closed_listeners(&self) -> Result<()> {
        let guard = self.core_closed_listeners.lock().map_err(|_| {
            LuceneError::IllegalState("core closed listeners lock poisoned".to_string())
        })?;
        for listener in guard.iter() {
            listener.on_close(CacheKey)?;
        }
        Ok(())
    }
}

/// Closes the value stored in `lock`, taking ownership of it.
///
/// The helper is generic over a sized `T` (e.g. `Box<dyn StoredFieldsReader>`)
/// so that it works through a concrete `RwLock` without trait-object
/// invariance issues.
fn close_lock<T>(
    lock: &RwLock<Option<T>>,
    mut close: impl FnMut(&mut T) -> Result<()>,
) -> Result<()> {
    let mut guard = lock
        .write()
        .map_err(|_| LuceneError::IllegalState("codec reader lock poisoned".to_string()))?;
    if let Some(mut value) = guard.take() {
        close(&mut value)?;
    }
    Ok(())
}

/// Checks the integrity of the value stored in `lock`.
fn check_lock<T>(lock: &RwLock<Option<T>>, mut check: impl FnMut(&T) -> Result<()>) -> Result<()> {
    let guard = lock
        .read()
        .map_err(|_| LuceneError::IllegalState("codec reader lock poisoned".to_string()))?;
    if let Some(ref value) = *guard {
        check(value)?;
    }
    Ok(())
}

impl Drop for SegmentCoreReaders {
    fn drop(&mut self) {
        if self.ref_count.load(Ordering::SeqCst) > 0 {
            let _ = self.dec_ref();
        }
    }
}

impl CacheHelper for SegmentCoreReaders {
    fn get_key(&self) -> &CacheKey {
        &self.core_cache_key
    }

    fn add_closed_listener(&self, listener: Box<dyn ClosedListener>) {
        if let Ok(mut listeners) = self.core_closed_listeners.lock() {
            listeners.push(listener);
        }
    }
}

// -----------------------------------------------------------------------------
// SegmentDocValues
// -----------------------------------------------------------------------------

/// Manages the [`DocValuesProducer`]s held by `SegmentReader`s and keeps track
/// of their reference counts.
///
/// Equivalent to `org.apache.lucene.index.SegmentDocValues`.
#[derive(Debug)]
pub struct SegmentDocValues {
    gen_producers: Mutex<HashMap<i64, Arc<DocValuesProducerHolder>>>,
}

impl SegmentDocValues {
    /// Creates an empty manager.
    pub fn new() -> Self {
        Self {
            gen_producers: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the doc-values producer for the given generation, creating and
    /// caching it if necessary.
    pub fn get_doc_values_producer(
        &self,
        gen: i64,
        si: &SegmentCommitInfo,
        dir: &dyn Directory,
        infos: &FieldInfos,
    ) -> Result<Arc<dyn DocValuesProducer>> {
        let mut map = self.gen_producers.lock().map_err(|_| {
            LuceneError::IllegalState("segment doc-values producer map lock poisoned".to_string())
        })?;

        if let Some(holder) = map.get(&gen) {
            holder.inc_ref()?;
            return Ok(Arc::clone(holder) as Arc<dyn DocValuesProducer>);
        }

        let (dv_dir, segment_suffix): (&dyn Directory, String) = if gen == -1 {
            (dir, String::new())
        } else {
            (&*si.info.directory, radix36(gen))
        };

        let state = crate::codecs::state::SegmentReadState::with_suffix(
            dv_dir,
            &si.info,
            infos,
            &*READONCE_IO_CONTEXT,
            segment_suffix,
        );

        let codec = si
            .info
            .codec()
            .ok_or_else(|| LuceneError::IllegalState("segment has no codec".to_string()))?;
        let producer = codec.doc_values_format().fields_producer(&state)?;
        let holder = Arc::new(DocValuesProducerHolder::new(producer)?);
        let cloned = Arc::clone(&holder) as Arc<dyn DocValuesProducer>;
        map.insert(gen, holder);
        Ok(cloned)
    }

    /// Decrements the reference counts for the given doc-values producer
    /// generations. When a generation's count reaches zero, the producer is
    /// closed and removed from the cache.
    pub fn dec_ref(&self, gens: &[i64]) -> Result<()> {
        let mut map = self.gen_producers.lock().map_err(|_| {
            LuceneError::IllegalState("segment doc-values producer map lock poisoned".to_string())
        })?;

        for gen in gens {
            if let Some(holder) = map.get(gen) {
                let new_count = holder.dec_ref()?;
                if new_count == 0 {
                    // Close the producer while we still hold the map entry, so
                    // that teardown happens eagerly during SegmentReader::close
                    // rather than waiting for the last Arc clone to drop.
                    holder.close_holder();
                    map.remove(gen);
                }
            }
        }
        Ok(())
    }
}

impl Default for SegmentDocValues {
    fn default() -> Self {
        Self::new()
    }
}

/// Holder that wraps a single doc-values producer with explicit reference
/// counting and safe close-on-drop semantics.
#[derive(Debug)]
struct DocValuesProducerHolder {
    producer: Mutex<Option<Box<dyn DocValuesProducer>>>,
    ref_count: AtomicI32,
}

impl DocValuesProducerHolder {
    fn new(producer: Box<dyn DocValuesProducer>) -> Result<Self> {
        Ok(Self {
            producer: Mutex::new(Some(producer)),
            ref_count: AtomicI32::new(1),
        })
    }

    fn inc_ref(&self) -> Result<()> {
        let mut count = self.ref_count.load(Ordering::SeqCst);
        while count > 0 {
            match self.ref_count.compare_exchange_weak(
                count,
                count + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return Ok(()),
                Err(actual) => count = actual,
            }
        }
        Err(LuceneError::AlreadyClosed(
            "DocValuesProducer is already closed".to_string(),
        ))
    }

    fn dec_ref(&self) -> Result<i32> {
        let rc = self.ref_count.fetch_sub(1, Ordering::SeqCst) - 1;
        if rc < 0 {
            return Err(LuceneError::IllegalState(
                "too many decRef calls on DocValuesProducer".to_string(),
            ));
        }
        Ok(rc)
    }

    fn close_holder(&self) {
        if let Ok(mut guard) = self.producer.lock() {
            if let Some(mut producer) = guard.take() {
                // Best-effort close; errors on teardown are swallowed because
                // there is no caller to report them to.
                let _ = producer.close();
            }
        }
    }
}

impl Drop for DocValuesProducerHolder {
    fn drop(&mut self) {
        self.close_holder();
    }
}

impl DocValuesProducer for DocValuesProducerHolder {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues + Send + Sync>> {
        let guard = self.producer.lock().map_err(|_| {
            LuceneError::IllegalState("doc-values producer lock poisoned".to_string())
        })?;
        if let Some(producer) = guard.as_ref() {
            producer.get_numeric(field)
        } else {
            Err(LuceneError::AlreadyClosed(
                "doc-values producer is closed".to_string(),
            ))
        }
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Box<dyn BinaryDocValues + Send + Sync>> {
        let guard = self.producer.lock().map_err(|_| {
            LuceneError::IllegalState("doc-values producer lock poisoned".to_string())
        })?;
        if let Some(producer) = guard.as_ref() {
            producer.get_binary(field)
        } else {
            Err(LuceneError::AlreadyClosed(
                "doc-values producer is closed".to_string(),
            ))
        }
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Box<dyn SortedDocValues + Send + Sync>> {
        let guard = self.producer.lock().map_err(|_| {
            LuceneError::IllegalState("doc-values producer lock poisoned".to_string())
        })?;
        if let Some(producer) = guard.as_ref() {
            producer.get_sorted(field)
        } else {
            Err(LuceneError::AlreadyClosed(
                "doc-values producer is closed".to_string(),
            ))
        }
    }

    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Box<dyn SortedNumericDocValues + Send + Sync>> {
        let guard = self.producer.lock().map_err(|_| {
            LuceneError::IllegalState("doc-values producer lock poisoned".to_string())
        })?;
        if let Some(producer) = guard.as_ref() {
            producer.get_sorted_numeric(field)
        } else {
            Err(LuceneError::AlreadyClosed(
                "doc-values producer is closed".to_string(),
            ))
        }
    }

    fn get_sorted_set(
        &self,
        field: &FieldInfo,
    ) -> Result<Box<dyn SortedSetDocValues + Send + Sync>> {
        let guard = self.producer.lock().map_err(|_| {
            LuceneError::IllegalState("doc-values producer lock poisoned".to_string())
        })?;
        if let Some(producer) = guard.as_ref() {
            producer.get_sorted_set(field)
        } else {
            Err(LuceneError::AlreadyClosed(
                "doc-values producer is closed".to_string(),
            ))
        }
    }

    fn get_skipper(&self, field: &FieldInfo) -> Result<Box<dyn DocValuesSkipper + Send + Sync>> {
        let guard = self.producer.lock().map_err(|_| {
            LuceneError::IllegalState("doc-values producer lock poisoned".to_string())
        })?;
        if let Some(producer) = guard.as_ref() {
            producer.get_skipper(field)
        } else {
            Err(LuceneError::AlreadyClosed(
                "doc-values producer is closed".to_string(),
            ))
        }
    }

    fn check_integrity(&self) -> Result<()> {
        let guard = self.producer.lock().map_err(|_| {
            LuceneError::IllegalState("doc-values producer lock poisoned".to_string())
        })?;
        if let Some(producer) = guard.as_ref() {
            producer.check_integrity()
        } else {
            Err(LuceneError::AlreadyClosed(
                "doc-values producer is closed".to_string(),
            ))
        }
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        let guard = self.producer.lock().map_err(|_| {
            LuceneError::IllegalState("doc-values producer lock poisoned".to_string())
        })?;
        if let Some(producer) = guard.as_ref() {
            producer.get_merge_instance()
        } else {
            Err(LuceneError::AlreadyClosed(
                "doc-values producer is closed".to_string(),
            ))
        }
    }

    fn close(&mut self) -> Result<()> {
        let mut guard = self.producer.lock().map_err(|_| {
            LuceneError::IllegalState("doc-values producer lock poisoned".to_string())
        })?;
        if let Some(mut producer) = guard.take() {
            producer.close()?;
        }
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// SegmentDocValuesProducer
// -----------------------------------------------------------------------------

/// Composite doc-values producer used when a segment has doc-values updates.
///
/// Equivalent to `org.apache.lucene.index.SegmentDocValuesProducer`.
#[derive(Debug, Clone)]
pub struct SegmentDocValuesProducer {
    // Kept alive so the per-generation cache in `SegmentDocValues` outlives the
    // producers that reference it.
    #[allow(dead_code)]
    seg_doc_values: Arc<SegmentDocValues>,
    dv_producers_by_field: HashMap<i32, Arc<dyn DocValuesProducer>>,
    dv_gens: Vec<i64>,
}

impl SegmentDocValuesProducer {
    /// Creates a composite producer that merges the base doc-values producer
    /// with per-generation producers for updated fields.
    pub fn new(
        si: &SegmentCommitInfo,
        dir: &dyn Directory,
        core_infos: &FieldInfos,
        all_infos: &FieldInfos,
        seg_doc_values: &Arc<SegmentDocValues>,
    ) -> Result<Self> {
        let mut dv_producers_by_field = HashMap::new();
        let mut dv_gens: Vec<i64> = Vec::new();
        let mut base_producer: Option<Arc<dyn DocValuesProducer>> = None;

        let result: Result<Self> = (|| {
            for fi in all_infos.iter() {
                if fi.doc_values_type == DocValuesType::NONE {
                    continue;
                }

                if fi.doc_values_gen == -1 {
                    if base_producer.is_none() {
                        let producer =
                            seg_doc_values.get_doc_values_producer(-1, si, dir, core_infos)?;
                        dv_gens.push(-1);
                        base_producer = Some(producer);
                    }
                    dv_producers_by_field
                        .insert(fi.number, Arc::clone(base_producer.as_ref().unwrap()));
                } else {
                    debug_assert!(!dv_gens.contains(&fi.doc_values_gen));
                    let field_infos = FieldInfos::new(vec![fi.clone()])?;
                    let producer = seg_doc_values.get_doc_values_producer(
                        fi.doc_values_gen,
                        si,
                        dir,
                        &field_infos,
                    )?;
                    dv_gens.push(fi.doc_values_gen);
                    dv_producers_by_field.insert(fi.number, producer);
                }
            }

            Ok(Self {
                seg_doc_values: Arc::clone(seg_doc_values),
                dv_producers_by_field,
                dv_gens: dv_gens.clone(),
            })
        })();

        if result.is_err() {
            // Mirror Java's try/catch: if opening any per-generation producer
            // fails, decrement reference counts for the producers we already
            // opened so that they are closed and removed from the cache.
            let _ = seg_doc_values.dec_ref(&dv_gens);
        }
        result
    }

    /// Returns the doc-values generations used by this producer.
    pub fn dv_gens(&self) -> &[i64] {
        &self.dv_gens
    }
}

impl DocValuesProducer for SegmentDocValuesProducer {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues + Send + Sync>> {
        self.dv_producers_by_field
            .get(&field.number)
            .ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "no doc-values producer for field {}",
                    field.name
                ))
            })?
            .get_numeric(field)
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Box<dyn BinaryDocValues + Send + Sync>> {
        self.dv_producers_by_field
            .get(&field.number)
            .ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "no doc-values producer for field {}",
                    field.name
                ))
            })?
            .get_binary(field)
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Box<dyn SortedDocValues + Send + Sync>> {
        self.dv_producers_by_field
            .get(&field.number)
            .ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "no doc-values producer for field {}",
                    field.name
                ))
            })?
            .get_sorted(field)
    }

    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Box<dyn SortedNumericDocValues + Send + Sync>> {
        self.dv_producers_by_field
            .get(&field.number)
            .ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "no doc-values producer for field {}",
                    field.name
                ))
            })?
            .get_sorted_numeric(field)
    }

    fn get_sorted_set(
        &self,
        field: &FieldInfo,
    ) -> Result<Box<dyn SortedSetDocValues + Send + Sync>> {
        self.dv_producers_by_field
            .get(&field.number)
            .ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "no doc-values producer for field {}",
                    field.name
                ))
            })?
            .get_sorted_set(field)
    }

    fn get_skipper(&self, field: &FieldInfo) -> Result<Box<dyn DocValuesSkipper + Send + Sync>> {
        self.dv_producers_by_field
            .get(&field.number)
            .ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "no doc-values producer for field {}",
                    field.name
                ))
            })?
            .get_skipper(field)
    }

    fn check_integrity(&self) -> Result<()> {
        for producer in self.dv_producers_by_field.values() {
            producer.check_integrity()?;
        }
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        // Lucene's default `DocValuesProducer.getMergeInstance()` returns `this`.
        // We return a boxed clone, which preserves the same semantics in Rust.
        Ok(Box::new(self.clone()))
    }

    fn close(&mut self) -> Result<()> {
        // Reference counting is performed explicitly by the owner
        // (`SegmentReader::do_close`) using `dv_gens`.
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// DocValuesLeafReader
// -----------------------------------------------------------------------------

/// Doc-values-only leaf reader wrapper.
///
/// Equivalent to `org.apache.lucene.index.DocValuesLeafReader`. This is the
/// reader type used by `IndexingChain` when it needs to sort a segment using
/// only its doc-values columns. All non-doc-values access methods return
/// `UnsupportedOperation`.
#[derive(Debug)]
pub struct DocValuesLeafReader {
    core: IndexReaderCore,
    max_doc: i32,
    field_infos: FieldInfos,
    doc_values_producer: Arc<dyn DocValuesProducer>,
}

impl DocValuesLeafReader {
    /// Creates a doc-values-only reader.
    pub fn new(
        max_doc: i32,
        field_infos: FieldInfos,
        doc_values_producer: Arc<dyn DocValuesProducer>,
    ) -> Self {
        Self {
            core: IndexReaderCore::new(),
            max_doc,
            field_infos,
            doc_values_producer,
        }
    }
}

impl LeafReader for DocValuesLeafReader {
    fn core(&self) -> &IndexReaderCore {
        &self.core
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        Err(LuceneError::UnsupportedOperation(
            "DocValuesLeafReader does not support term vectors".to_string(),
        ))
    }

    fn num_docs(&self) -> i32 {
        self.max_doc
    }

    fn max_doc(&self) -> i32 {
        self.max_doc
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        Err(LuceneError::UnsupportedOperation(
            "DocValuesLeafReader does not support stored fields".to_string(),
        ))
    }

    fn do_close(&self) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "DocValuesLeafReader does not support close".to_string(),
        ))
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
        Err(LuceneError::UnsupportedOperation(
            "DocValuesLeafReader does not support terms".to_string(),
        ))
    }

    fn get_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn IndexNumericDocValues>>> {
        if let Some(fi) = self.field_infos.field_info(field) {
            if fi.doc_values_type == DocValuesType::NUMERIC {
                return Ok(Some(self.doc_values_producer.get_numeric(fi)?));
            }
        }
        Ok(None)
    }

    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn IndexBinaryDocValues>>> {
        if let Some(fi) = self.field_infos.field_info(field) {
            if fi.doc_values_type == DocValuesType::BINARY {
                return Ok(Some(self.doc_values_producer.get_binary(fi)?));
            }
        }
        Ok(None)
    }

    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn IndexSortedDocValues>>> {
        if let Some(fi) = self.field_infos.field_info(field) {
            if fi.doc_values_type == DocValuesType::SORTED {
                return Ok(Some(self.doc_values_producer.get_sorted(fi)?));
            }
        }
        Ok(None)
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn IndexSortedNumericDocValues>>> {
        if let Some(fi) = self.field_infos.field_info(field) {
            if fi.doc_values_type == DocValuesType::SORTED_NUMERIC {
                return Ok(Some(self.doc_values_producer.get_sorted_numeric(fi)?));
            }
        }
        Ok(None)
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn IndexSortedSetDocValues>>> {
        if let Some(fi) = self.field_infos.field_info(field) {
            if fi.doc_values_type == DocValuesType::SORTED_SET {
                return Ok(Some(self.doc_values_producer.get_sorted_set(fi)?));
            }
        }
        Ok(None)
    }

    fn get_norm_values(&self, _field: &str) -> Result<Option<Box<dyn IndexNumericDocValues>>> {
        Ok(None)
    }

    fn get_doc_values_skipper(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn IndexDocValuesSkipper>>> {
        if let Some(fi) = self.field_infos.field_info(field) {
            if fi.doc_values_skip_index_type != crate::index::DocValuesSkipIndexType::NONE {
                return Ok(Some(self.doc_values_producer.get_skipper(fi)?));
            }
        }
        Ok(None)
    }

    fn get_float_vector_values(&self, _field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        Ok(None)
    }

    fn get_byte_vector_values(&self, _field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        Ok(None)
    }

    fn search_nearest_vectors(
        &self,
        _field: &str,
        _target: &[f32],
        _collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }

    fn search_nearest_vectors_byte(
        &self,
        _field: &str,
        _target: &[u8],
        _collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }

    fn get_field_infos(&self) -> FieldInfos {
        self.field_infos.clone()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        None
    }

    fn get_point_values(&self, _field: &str) -> Result<Option<Box<dyn IndexPointValues>>> {
        Ok(None)
    }

    fn check_integrity(&self) -> Result<()> {
        self.doc_values_producer.check_integrity()
    }

    fn get_meta_data(&self) -> LeafMetaData {
        LeafMetaData::new(0, None, None, false).expect("metadata is valid")
    }
}

// -----------------------------------------------------------------------------
// SegmentReader
// -----------------------------------------------------------------------------

/// Index reader implementation over a single segment.
///
/// Equivalent to `org.apache.lucene.index.SegmentReader`. Instances pointing to
/// the same segment (but with different deletes or doc-values updates) may
/// share the same `SegmentCoreReaders`.
pub struct SegmentReader {
    core: IndexReaderCore,
    segment_info: SegmentCommitInfo,
    original_segment_info: SegmentCommitInfo,
    meta_data: LeafMetaData,
    max_doc: i32,
    live_docs: Option<SharedBits>,
    hard_live_docs: Option<SharedBits>,
    num_docs: i32,
    // Stored to match Lucene's constructor state; not yet read by Rust code.
    #[allow(dead_code)]
    is_nrt: bool,
    core_readers: Arc<SegmentCoreReaders>,
    seg_doc_values: Arc<SegmentDocValues>,
    doc_values_producer: Option<Arc<dyn DocValuesProducer>>,
    doc_values_gens: Option<Vec<i64>>,
    field_infos: FieldInfos,
    reader_cache_helper: SegmentReaderCacheHelper,
    core_cache_helper: SegmentReaderCoreCacheHelper,
}

impl Debug for SegmentReader {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentReader")
            .field("segment", &self.segment_info.info.name)
            .field("max_doc", &self.max_doc)
            .field("num_docs", &self.num_docs)
            .finish_non_exhaustive()
    }
}

impl SegmentReader {
    /// Creates a new `SegmentReader` that opens its own core readers.
    ///
    /// # Errors
    ///
    /// Returns a `CorruptIndexException` or other I/O error if the segment
    /// files cannot be opened.
    pub fn new(
        segment_info: SegmentCommitInfo,
        created_version_major: i32,
        context: &dyn IOContext,
    ) -> Result<Self> {
        let max_doc = segment_info.info.max_doc()?;
        let si = segment_info.clone();
        let original_si = segment_info;
        let dir: &dyn Directory = &*original_si.info.directory;

        let meta_data = LeafMetaData::new(
            created_version_major,
            original_si.info.min_version(),
            Some(original_si.info.index_sort().clone()),
            original_si.info.get_has_blocks(),
        )?;

        let core_readers = Arc::new(SegmentCoreReaders::new(dir, &original_si, context)?);
        let seg_doc_values = Arc::new(SegmentDocValues::new());

        let result: Result<Self> = {
            let core_readers = Arc::clone(&core_readers);
            let seg_doc_values = Arc::clone(&seg_doc_values);
            let original_si_for_closure = original_si.clone();

            (|| {
                let field_infos =
                    Self::init_field_infos(&original_si_for_closure, &core_readers, context)?;

                let live_docs = if original_si_for_closure.has_deletions() {
                    let codec = original_si_for_closure.info.codec().ok_or_else(|| {
                        LuceneError::IllegalState("segment has no codec".to_string())
                    })?;
                    Some(SharedBits::new(codec.live_docs_format().read_live_docs(
                        dir,
                        &original_si_for_closure,
                        &*READONCE_IO_CONTEXT,
                    )?))
                } else {
                    None
                };
                let hard_live_docs = live_docs.clone();

                let num_docs = max_doc - original_si_for_closure.get_del_count();

                let cfs_dir_guard = core_readers.cfs_reader.read().map_err(|_| {
                    LuceneError::IllegalState("compound file reader lock poisoned".to_string())
                })?;
                let cfs_dir: &dyn Directory = if let Some(cfs) = cfs_dir_guard.as_ref() {
                    cfs.as_ref()
                } else {
                    dir
                };

                let (doc_values_producer, doc_values_gens) = Self::init_doc_values_producer(
                    &original_si_for_closure,
                    cfs_dir,
                    &core_readers,
                    &field_infos,
                    &seg_doc_values,
                )?;
                drop(cfs_dir_guard);

                let reader_cache_helper = SegmentReaderCacheHelper::new();
                let core_cache_helper =
                    SegmentReaderCoreCacheHelper::new(Arc::clone(&core_readers));

                Ok(Self {
                    core: IndexReaderCore::new(),
                    segment_info: si,
                    original_segment_info: original_si_for_closure,
                    meta_data,
                    max_doc,
                    live_docs,
                    hard_live_docs,
                    num_docs,
                    is_nrt: false,
                    core_readers,
                    seg_doc_values,
                    doc_values_producer,
                    doc_values_gens,
                    field_infos,
                    reader_cache_helper,
                    core_cache_helper,
                })
            })()
        };

        let success = result.is_ok();
        if !success {
            // Mirror Java's try/finally: if anything after core creation fails,
            // release the core reference so producers and the CFS reader close.
            let _ = core_readers.dec_ref();
        }
        result
    }

    /// Creates a new `SegmentReader` that shares the core readers and doc-values
    /// manager of an existing reader.
    ///
    /// This mirrors Java's `SegmentReader(SegmentCommitInfo, SegmentReader,
    /// Bits, Bits, int, boolean)` constructor used for NRT readers and cloned
    /// readers with in-memory live docs or doc-values updates.
    ///
    /// # Errors
    ///
    /// Returns `IllegalArgument` if `num_docs` is larger than the segment's
    /// `max_doc` or if `live_docs` has the wrong length. Returns any I/O error
    /// thrown while loading updated field infos or doc-values producers.
    #[allow(clippy::too_many_arguments)]
    pub fn new_shared(
        segment_info: SegmentCommitInfo,
        parent: &SegmentReader,
        live_docs: Option<Box<dyn Bits>>,
        hard_live_docs: Option<Box<dyn Bits>>,
        num_docs: i32,
        is_nrt: bool,
    ) -> Result<Self> {
        let max_doc = segment_info.info.max_doc()?;
        if num_docs > max_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "numDocs={num_docs} but maxDoc={max_doc}"
            )));
        }
        if let Some(ref bits) = live_docs {
            if bits.length() != max_doc as usize {
                return Err(LuceneError::IllegalArgument(format!(
                    "maxDoc={max_doc} but liveDocs.size()={}",
                    bits.length()
                )));
            }
        }

        let si = segment_info.clone();
        let original_si = segment_info;
        let live_docs = live_docs.map(SharedBits::new);
        let hard_live_docs = hard_live_docs.map(SharedBits::new);

        parent.core_readers.inc_ref()?;
        let core_readers = Arc::clone(&parent.core_readers);
        let seg_doc_values = Arc::clone(&parent.seg_doc_values);

        let result: Result<Self> = {
            let core_readers = Arc::clone(&core_readers);
            let seg_doc_values = Arc::clone(&seg_doc_values);
            let original_si_for_closure = original_si.clone();

            (|| {
                let field_infos = Self::init_field_infos(
                    &original_si_for_closure,
                    &core_readers,
                    &*READONCE_IO_CONTEXT,
                )?;

                let dir: &dyn Directory = &*original_si_for_closure.info.directory;
                let cfs_dir_guard = core_readers.cfs_reader.read().map_err(|_| {
                    LuceneError::IllegalState("compound file reader lock poisoned".to_string())
                })?;
                let cfs_dir: &dyn Directory = if let Some(cfs) = cfs_dir_guard.as_ref() {
                    cfs.as_ref()
                } else {
                    dir
                };

                let (doc_values_producer, doc_values_gens) = Self::init_doc_values_producer(
                    &original_si_for_closure,
                    cfs_dir,
                    &core_readers,
                    &field_infos,
                    &seg_doc_values,
                )?;
                drop(cfs_dir_guard);

                let reader_cache_helper = SegmentReaderCacheHelper::new();
                let core_cache_helper =
                    SegmentReaderCoreCacheHelper::new(Arc::clone(&core_readers));

                Ok(Self {
                    core: IndexReaderCore::new(),
                    segment_info: si,
                    original_segment_info: original_si_for_closure,
                    meta_data: parent.meta_data.clone(),
                    max_doc,
                    live_docs,
                    hard_live_docs,
                    num_docs,
                    is_nrt,
                    core_readers,
                    seg_doc_values,
                    doc_values_producer,
                    doc_values_gens,
                    field_infos,
                    reader_cache_helper,
                    core_cache_helper,
                })
            })()
        };

        let success = result.is_ok();
        if !success {
            // Release the shared core reference acquired above.
            let _ = core_readers.dec_ref();
        }
        result
    }

    fn init_field_infos(
        si: &SegmentCommitInfo,
        core: &SegmentCoreReaders,
        context: &dyn IOContext,
    ) -> Result<FieldInfos> {
        if !si.has_field_updates() {
            Ok(core.core_field_infos().clone())
        } else {
            let codec = si
                .info
                .codec()
                .ok_or_else(|| LuceneError::IllegalState("segment has no codec".to_string()))?;
            let segment_suffix = radix36(si.get_field_infos_gen());
            codec
                .field_infos_format()
                .read(&*si.info.directory, &si.info, &segment_suffix, context)
        }
    }

    #[allow(clippy::type_complexity)]
    fn init_doc_values_producer(
        si: &SegmentCommitInfo,
        dir: &dyn Directory,
        core: &SegmentCoreReaders,
        field_infos: &FieldInfos,
        seg_doc_values: &Arc<SegmentDocValues>,
    ) -> Result<(Option<Arc<dyn DocValuesProducer>>, Option<Vec<i64>>)> {
        if !field_infos.has_doc_values() {
            return Ok((None, None));
        }

        if si.has_field_updates() {
            let producer = SegmentDocValuesProducer::new(
                si,
                dir,
                core.core_field_infos(),
                field_infos,
                seg_doc_values,
            )?;
            let gens = producer.dv_gens().to_vec();
            Ok((
                Some(Arc::new(producer) as Arc<dyn DocValuesProducer>),
                Some(gens),
            ))
        } else {
            let producer = seg_doc_values.get_doc_values_producer(-1, si, dir, field_infos)?;
            Ok((Some(producer), Some(vec![-1])))
        }
    }

    fn ensure_open(&self) -> Result<()> {
        self.core.ensure_open()
    }

    /// Returns the field infos for this segment.
    pub fn get_field_infos_ref(&self) -> &FieldInfos {
        &self.field_infos
    }

    /// Returns the name of the segment this reader is reading.
    pub fn get_segment_name(&self) -> &str {
        &self.segment_info.info.name
    }

    /// Returns the `SegmentCommitInfo` of the segment this reader is reading.
    pub fn get_segment_info(&self) -> &SegmentCommitInfo {
        &self.segment_info
    }

    /// Returns the original `SegmentCommitInfo` passed to the constructor.
    pub fn get_original_segment_info(&self) -> &SegmentCommitInfo {
        &self.original_segment_info
    }

    /// Returns the directory this index resides in.
    pub fn directory(&self) -> Arc<dyn Directory> {
        Arc::clone(&self.original_segment_info.info.directory)
    }

    /// Returns the shared core readers.
    pub fn core_readers(&self) -> &Arc<SegmentCoreReaders> {
        &self.core_readers
    }

    /// Returns the live docs that are not hard-deleted, if any.
    pub fn get_hard_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.hard_live_docs
            .as_ref()
            .map(|bits| Box::new(bits.clone()) as Box<dyn Bits>)
    }

    /// Returns the underlying postings reader.
    pub fn get_postings_reader(&self) -> Result<Option<FieldsProducerGuard<'_>>> {
        self.ensure_open()?;
        let guard =
            self.core_readers.fields.read().map_err(|_| {
                LuceneError::IllegalState("fields producer lock poisoned".to_string())
            })?;
        match guard.as_ref() {
            Some(_) => Ok(Some(FieldsProducerGuard(guard))),
            None => Ok(None),
        }
    }

    /// Returns the underlying stored-fields reader.
    pub fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        self.ensure_open()?;
        let guard = self.core_readers.fields_reader_orig.read().map_err(|_| {
            LuceneError::IllegalState("stored fields reader lock poisoned".to_string())
        })?;
        Ok(guard.as_ref().map(|reader| reader.clone_reader()))
    }

    /// Returns the underlying term-vectors reader.
    pub fn get_term_vectors_reader(&self) -> Result<Option<Box<dyn TermVectorsReader>>> {
        self.ensure_open()?;
        let guard = self
            .core_readers
            .term_vectors_reader_orig
            .read()
            .map_err(|_| {
                LuceneError::IllegalState("term vectors reader lock poisoned".to_string())
            })?;
        Ok(guard.as_ref().map(|reader| reader.clone_reader()))
    }

    /// Returns the underlying norms producer.
    pub fn get_norms_reader(&self) -> Result<Option<NormsProducerGuard<'_>>> {
        self.ensure_open()?;
        let guard =
            self.core_readers.norms_producer.read().map_err(|_| {
                LuceneError::IllegalState("norms producer lock poisoned".to_string())
            })?;
        match guard.as_ref() {
            Some(_) => Ok(Some(NormsProducerGuard(guard))),
            None => Ok(None),
        }
    }

    /// Returns the underlying doc-values producer.
    pub fn get_doc_values_reader(&self) -> Result<Option<Arc<dyn DocValuesProducer>>> {
        self.ensure_open()?;
        Ok(self.doc_values_producer.as_ref().map(Arc::clone))
    }

    /// Returns the underlying points reader.
    pub fn get_points_reader(&self) -> Result<Option<PointsReaderGuard<'_>>> {
        self.ensure_open()?;
        let guard =
            self.core_readers.points_reader.read().map_err(|_| {
                LuceneError::IllegalState("points reader lock poisoned".to_string())
            })?;
        match guard.as_ref() {
            Some(_) => Ok(Some(PointsReaderGuard(guard))),
            None => Ok(None),
        }
    }

    /// Returns the underlying KNN vectors reader.
    pub fn get_vector_reader(&self) -> Result<Option<KnnVectorsReaderGuard<'_>>> {
        self.ensure_open()?;
        let guard = self.core_readers.knn_vectors_reader.read().map_err(|_| {
            LuceneError::IllegalState("knn vectors reader lock poisoned".to_string())
        })?;
        match guard.as_ref() {
            Some(_) => Ok(Some(KnnVectorsReaderGuard(guard))),
            None => Ok(None),
        }
    }
}

impl LeafReader for SegmentReader {
    fn core(&self) -> &IndexReaderCore {
        &self.core
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.ensure_open()?;
        let guard = self
            .core_readers
            .term_vectors_reader_orig
            .read()
            .map_err(|_| {
                LuceneError::IllegalState("term vectors reader lock poisoned".to_string())
            })?;
        if let Some(reader) = guard.as_ref() {
            Ok(Box::new(TermVectorsReaderWrapper(reader.clone_reader())))
        } else {
            Ok(Box::new(crate::index::leaf_reader::EmptyTermVectors))
        }
    }

    fn num_docs(&self) -> i32 {
        self.num_docs
    }

    fn max_doc(&self) -> i32 {
        self.max_doc
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.ensure_open()?;
        let guard = self.core_readers.fields_reader_orig.read().map_err(|_| {
            LuceneError::IllegalState("stored fields reader lock poisoned".to_string())
        })?;
        if let Some(_reader) = guard.as_ref() {
            Ok(Box::new(CodecStoredFields {
                core: Arc::clone(&self.core_readers),
                max_doc: self.max_doc,
            }))
        } else {
            Err(LuceneError::AlreadyClosed(
                "stored fields reader is closed".to_string(),
            ))
        }
    }

    fn do_close(&self) -> Result<()> {
        let core_result = self.core_readers.dec_ref();
        let dv_result = if let Some(ref gens) = self.doc_values_gens {
            self.seg_doc_values.dec_ref(gens)
        } else {
            Ok(())
        };
        let listener_result = self.reader_cache_helper.notify_closed_listeners();
        core_result?;
        dv_result?;
        listener_result
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        Some(Box::new(self.reader_cache_helper.clone()))
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        Some(Box::new(self.core_cache_helper.clone()))
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.ensure_open()?;
        let fi = self.field_infos.field_info(field);
        if fi.is_none() || fi.unwrap().index_options == crate::index::IndexOptions::NONE {
            return Ok(None);
        }
        let guard =
            self.core_readers.fields.read().map_err(|_| {
                LuceneError::IllegalState("fields producer lock poisoned".to_string())
            })?;
        if let Some(fields) = guard.as_ref() {
            fields.terms(field)
        } else {
            Ok(None)
        }
    }

    fn get_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn IndexNumericDocValues>>> {
        self.ensure_open()?;
        let fi = self.get_dv_field(field, DocValuesType::NUMERIC);
        if fi.is_none() {
            return Ok(None);
        }
        if let Some(ref producer) = self.doc_values_producer {
            Ok(Some(producer.get_numeric(fi.unwrap())?))
        } else {
            Ok(None)
        }
    }

    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn IndexBinaryDocValues>>> {
        self.ensure_open()?;
        let fi = self.get_dv_field(field, DocValuesType::BINARY);
        if fi.is_none() {
            return Ok(None);
        }
        if let Some(ref producer) = self.doc_values_producer {
            Ok(Some(producer.get_binary(fi.unwrap())?))
        } else {
            Ok(None)
        }
    }

    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn IndexSortedDocValues>>> {
        self.ensure_open()?;
        let fi = self.get_dv_field(field, DocValuesType::SORTED);
        if fi.is_none() {
            return Ok(None);
        }
        if let Some(ref producer) = self.doc_values_producer {
            Ok(Some(producer.get_sorted(fi.unwrap())?))
        } else {
            Ok(None)
        }
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn IndexSortedNumericDocValues>>> {
        self.ensure_open()?;
        let fi = self.get_dv_field(field, DocValuesType::SORTED_NUMERIC);
        if fi.is_none() {
            return Ok(None);
        }
        if let Some(ref producer) = self.doc_values_producer {
            Ok(Some(producer.get_sorted_numeric(fi.unwrap())?))
        } else {
            Ok(None)
        }
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn IndexSortedSetDocValues>>> {
        self.ensure_open()?;
        let fi = self.get_dv_field(field, DocValuesType::SORTED_SET);
        if fi.is_none() {
            return Ok(None);
        }
        if let Some(ref producer) = self.doc_values_producer {
            Ok(Some(producer.get_sorted_set(fi.unwrap())?))
        } else {
            Ok(None)
        }
    }

    fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn IndexNumericDocValues>>> {
        self.ensure_open()?;
        let fi = self.field_infos.field_info(field);
        if fi.is_none() || !fi.unwrap().has_norms() {
            return Ok(None);
        }
        let guard =
            self.core_readers.norms_producer.read().map_err(|_| {
                LuceneError::IllegalState("norms producer lock poisoned".to_string())
            })?;
        if let Some(producer) = guard.as_ref() {
            Ok(Some(producer.get_norms(fi.unwrap())?))
        } else {
            Ok(None)
        }
    }

    fn get_doc_values_skipper(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn IndexDocValuesSkipper>>> {
        self.ensure_open()?;
        let fi = self.field_infos.field_info(field);
        if fi.is_none()
            || fi.unwrap().doc_values_skip_index_type == crate::index::DocValuesSkipIndexType::NONE
        {
            return Ok(None);
        }
        if let Some(ref producer) = self.doc_values_producer {
            Ok(Some(producer.get_skipper(fi.unwrap())?))
        } else {
            Ok(None)
        }
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        self.ensure_open()?;
        let fi = self.field_infos.field_info(field);
        if fi.is_none()
            || fi.unwrap().vector_dimension == 0
            || fi.unwrap().vector_encoding != VectorEncoding::FLOAT32
        {
            return Ok(None);
        }
        let guard = self.core_readers.knn_vectors_reader.read().map_err(|_| {
            LuceneError::IllegalState("knn vectors reader lock poisoned".to_string())
        })?;
        if let Some(reader) = guard.as_ref() {
            Ok(Some(reader.get_float_vector_values(field)?))
        } else {
            Ok(None)
        }
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        self.ensure_open()?;
        let fi = self.field_infos.field_info(field);
        if fi.is_none()
            || fi.unwrap().vector_dimension == 0
            || fi.unwrap().vector_encoding != VectorEncoding::BYTE
        {
            return Ok(None);
        }
        let guard = self.core_readers.knn_vectors_reader.read().map_err(|_| {
            LuceneError::IllegalState("knn vectors reader lock poisoned".to_string())
        })?;
        if let Some(reader) = guard.as_ref() {
            Ok(Some(reader.get_byte_vector_values(field)?))
        } else {
            Ok(None)
        }
    }

    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        self.ensure_open()?;
        let fi = self.field_infos.field_info(field);
        if fi.is_none()
            || fi.unwrap().vector_dimension == 0
            || fi.unwrap().vector_encoding != VectorEncoding::FLOAT32
        {
            return Ok(());
        }
        let mut guard = self.core_readers.knn_vectors_reader.write().map_err(|_| {
            LuceneError::IllegalState("knn vectors reader lock poisoned".to_string())
        })?;
        if let Some(reader) = guard.as_mut() {
            reader.search(field, target, collector, accept_docs)
        } else {
            Ok(())
        }
    }

    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        self.ensure_open()?;
        let fi = self.field_infos.field_info(field);
        if fi.is_none()
            || fi.unwrap().vector_dimension == 0
            || fi.unwrap().vector_encoding != VectorEncoding::BYTE
        {
            return Ok(());
        }
        let mut guard = self.core_readers.knn_vectors_reader.write().map_err(|_| {
            LuceneError::IllegalState("knn vectors reader lock poisoned".to_string())
        })?;
        if let Some(reader) = guard.as_mut() {
            reader.search_byte(field, target, collector, accept_docs)
        } else {
            Ok(())
        }
    }

    fn get_field_infos(&self) -> FieldInfos {
        self.field_infos.clone()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.live_docs
            .as_ref()
            .map(|bits| Box::new(bits.clone()) as Box<dyn Bits>)
    }

    fn get_point_values(&self, _field: &str) -> Result<Option<Box<dyn IndexPointValues>>> {
        // TODO(rmp #119): bridge `PointsReader` to the index-layer
        // `PointValues` trait. The codec-level point values live in
        // `core_readers.points_reader`; exposing them as `index::PointValues`
        // needs the BKD-backed `PointTree` implementation, which task #119
        // ports. Until then this leaf reports no point values, which the
        // `PointValues` contract treats as "this field has no points".
        Ok(None)
    }

    fn check_integrity(&self) -> Result<()> {
        self.ensure_open()?;
        check_lock(&self.core_readers.fields, |producer| {
            producer.check_integrity()
        })?;
        check_lock(&self.core_readers.norms_producer, |producer| {
            producer.check_integrity()
        })?;
        if let Some(ref producer) = self.doc_values_producer {
            producer.check_integrity()?;
        }
        check_lock(&self.core_readers.fields_reader_orig, |reader| {
            reader.check_integrity()
        })?;
        check_lock(&self.core_readers.term_vectors_reader_orig, |reader| {
            reader.check_integrity()
        })?;
        check_lock(&self.core_readers.points_reader, |producer| {
            producer.check_integrity()
        })?;
        check_lock(&self.core_readers.knn_vectors_reader, |producer| {
            producer.check_integrity()
        })?;

        if let Ok(guard) = self.core_readers.cfs_reader.read() {
            if let Some(reader) = guard.as_ref() {
                reader.check_integrity()?;
            }
        }
        Ok(())
    }

    fn get_meta_data(&self) -> LeafMetaData {
        self.meta_data.clone()
    }
}

impl SegmentReader {
    /// Returns the field info for `field` only if it has the requested doc-values
    /// type, mirroring `CodecReader.getDVField`.
    fn get_dv_field(&self, field: &str, doc_values_type: DocValuesType) -> Option<&FieldInfo> {
        let fi = self.field_infos.field_info(field)?;
        if fi.doc_values_type == DocValuesType::NONE {
            return None;
        }
        if fi.doc_values_type != doc_values_type {
            return None;
        }
        Some(fi)
    }
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// Formats a non-negative generation number as a base-36 string, matching
/// Lucene's `Long.toString(gen, Character.MAX_RADIX)`.
fn radix36(gen: i64) -> String {
    if gen < 0 {
        return String::new();
    }
    let mut n = gen as u64;
    if n == 0 {
        return "0".to_string();
    }
    let mut s = String::new();
    while n > 0 {
        let digit = (n % 36) as u32;
        s.push(std::char::from_digit(digit, 36).unwrap());
        n /= 36;
    }
    s.chars().rev().collect()
}

/// Wrapper that lets a `Box<dyn TermVectorsReader>` satisfy the
/// `TermVectors` trait.
struct TermVectorsReaderWrapper(Box<dyn TermVectorsReader>);

impl Debug for TermVectorsReaderWrapper {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TermVectorsReaderWrapper")
            .finish_non_exhaustive()
    }
}

impl TermVectors for TermVectorsReaderWrapper {
    fn get(&self, doc: i32) -> Result<Option<Box<dyn crate::index::Fields>>> {
        self.0.get(doc)
    }
}

/// `Bits` implementation that shares an underlying `Arc<dyn Bits>`.
#[derive(Clone)]
struct SharedBits(Arc<dyn Bits>);

impl SharedBits {
    fn new(bits: Box<dyn Bits>) -> Self {
        Self(Arc::from(bits))
    }
}

impl Debug for SharedBits {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedBits")
            .field("length", &self.0.length())
            .finish()
    }
}

impl Bits for SharedBits {
    fn get(&self, index: usize) -> bool {
        self.0.get(index)
    }

    fn length(&self) -> usize {
        self.0.length()
    }
}

/// `StoredFields` implementation used by `SegmentReader` that performs
/// document-id bounds checks before delegating to the codec reader.
#[derive(Debug)]
struct CodecStoredFields {
    core: Arc<SegmentCoreReaders>,
    max_doc: i32,
}

impl StoredFields for CodecStoredFields {
    fn prefetch(&mut self, doc_id: i32) -> Result<()> {
        check_index(doc_id, self.max_doc)?;
        let guard = self.core.fields_reader_orig.read().map_err(|_| {
            LuceneError::IllegalState("stored fields reader lock poisoned".to_string())
        })?;
        if let Some(reader) = guard.as_ref() {
            reader.prefetch(doc_id)
        } else {
            Err(LuceneError::AlreadyClosed(
                "stored fields reader is closed".to_string(),
            ))
        }
    }

    fn document_with_visitor(
        &self,
        doc_id: i32,
        visitor: &mut dyn StoredFieldVisitor,
    ) -> Result<()> {
        check_index(doc_id, self.max_doc)?;
        let guard = self.core.fields_reader_orig.read().map_err(|_| {
            LuceneError::IllegalState("stored fields reader lock poisoned".to_string())
        })?;
        if let Some(reader) = guard.as_ref() {
            reader.document(doc_id, visitor)
        } else {
            Err(LuceneError::AlreadyClosed(
                "stored fields reader is closed".to_string(),
            ))
        }
    }

    fn document(&self, doc_id: i32) -> Result<Document> {
        let mut loader = StoredFieldLoader::new(None);
        self.document_with_visitor(doc_id, &mut loader)?;
        Ok(loader.into_document())
    }

    fn document_fields(&self, doc_id: i32, fields_to_load: &HashSet<String>) -> Result<Document> {
        let mut loader = StoredFieldLoader::new(Some(fields_to_load.clone()));
        self.document_with_visitor(doc_id, &mut loader)?;
        Ok(loader.into_document())
    }
}

/// Builds a `Document` from the stored fields visited by a codec reader.
#[derive(Debug)]
struct StoredFieldLoader {
    doc: Document,
    filter: Option<HashSet<String>>,
    current_name: String,
}

impl StoredFieldLoader {
    fn new(filter: Option<HashSet<String>>) -> Self {
        Self {
            doc: Document::new(),
            filter,
            current_name: String::new(),
        }
    }

    fn into_document(self) -> Document {
        self.doc
    }

    fn should_load(&mut self, info: &FieldInfo) -> bool {
        self.current_name = info.name.clone();
        match &self.filter {
            Some(set) => set.contains(&info.name),
            None => true,
        }
    }
}

impl StoredFieldVisitor for StoredFieldLoader {
    fn binary_field(&mut self, _info: &FieldInfo, _value: &[u8]) -> Result<()> {
        if self.should_load(_info) {
            self.doc
                .add(Box::new(crate::document::StoredField::new_bytes(
                    &self.current_name,
                    BytesRef::new(_value.to_vec()),
                )?));
        }
        Ok(())
    }

    fn string_field(&mut self, _info: &FieldInfo, _value: &str) -> Result<()> {
        if self.should_load(_info) {
            self.doc
                .add(Box::new(crate::document::StoredField::new_string(
                    &self.current_name,
                    _value.to_string(),
                )?));
        }
        Ok(())
    }

    fn int_field(&mut self, _info: &FieldInfo, _value: i32) -> Result<()> {
        if self.should_load(_info) {
            self.doc
                .add(Box::new(crate::document::StoredField::new_number(
                    &self.current_name,
                    NumericValue::Int(_value),
                )?));
        }
        Ok(())
    }

    fn long_field(&mut self, _info: &FieldInfo, _value: i64) -> Result<()> {
        if self.should_load(_info) {
            self.doc
                .add(Box::new(crate::document::StoredField::new_number(
                    &self.current_name,
                    NumericValue::Long(_value),
                )?));
        }
        Ok(())
    }

    fn float_field(&mut self, _info: &FieldInfo, _value: f32) -> Result<()> {
        if self.should_load(_info) {
            self.doc
                .add(Box::new(crate::document::StoredField::new_number(
                    &self.current_name,
                    NumericValue::Float(_value),
                )?));
        }
        Ok(())
    }

    fn double_field(&mut self, _info: &FieldInfo, _value: f64) -> Result<()> {
        if self.should_load(_info) {
            self.doc
                .add(Box::new(crate::document::StoredField::new_number(
                    &self.current_name,
                    NumericValue::Double(_value),
                )?));
        }
        Ok(())
    }

    fn needs_field(&mut self, info: &FieldInfo) -> crate::codecs::stub::StoredFieldVisitorStatus {
        if self.should_load(info) {
            crate::codecs::stub::StoredFieldVisitorStatus::Yes
        } else {
            crate::codecs::stub::StoredFieldVisitorStatus::No
        }
    }
}

/// Checks that `doc_id` is in the range `[0, max_doc)`.
fn check_index(doc_id: i32, max_doc: i32) -> Result<()> {
    if doc_id < 0 || doc_id >= max_doc {
        return Err(LuceneError::IllegalArgument(format!(
            "docID must be 0..{max_doc} but was {doc_id}"
        )));
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Cache helpers
// -----------------------------------------------------------------------------

/// Reader-level cache helper for `SegmentReader`.
#[derive(Clone)]
struct SegmentReaderCacheHelper {
    key: CacheKey,
    listeners: Arc<Mutex<Vec<Box<dyn ClosedListener>>>>,
    closed: Arc<AtomicBool>,
}

impl Debug for SegmentReaderCacheHelper {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentReaderCacheHelper")
            .field("closed", &self.closed.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl SegmentReaderCacheHelper {
    fn new() -> Self {
        Self {
            key: CacheKey,
            listeners: Arc::new(Mutex::new(Vec::new())),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    fn notify_closed_listeners(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let guard = self.listeners.lock().map_err(|_| {
            LuceneError::IllegalState("reader closed listeners lock poisoned".to_string())
        })?;
        for listener in guard.iter() {
            listener.on_close(CacheKey)?;
        }
        Ok(())
    }
}

impl CacheHelper for SegmentReaderCacheHelper {
    fn get_key(&self) -> &CacheKey {
        &self.key
    }

    fn add_closed_listener(&self, listener: Box<dyn ClosedListener>) {
        if let Ok(mut listeners) = self.listeners.lock() {
            listeners.push(listener);
        }
    }
}

/// Wrapper around the core cache helper used by `SegmentReader`.
#[derive(Clone, Debug)]
struct SegmentReaderCoreCacheHelper {
    core: Arc<SegmentCoreReaders>,
}

impl SegmentReaderCoreCacheHelper {
    fn new(core: Arc<SegmentCoreReaders>) -> Self {
        Self { core }
    }
}

impl CacheHelper for SegmentReaderCoreCacheHelper {
    fn get_key(&self) -> &CacheKey {
        self.core.get_key()
    }

    fn add_closed_listener(&self, listener: Box<dyn ClosedListener>) {
        self.core.add_closed_listener(listener);
    }
}

// -----------------------------------------------------------------------------
// Reader guards
// -----------------------------------------------------------------------------

/// Read guard that dereferences to the stored postings reader.
///
/// This lets the expert `get_postings_reader` API return the actual reader
/// held by `SegmentCoreReaders` without copying or creating a merge instance.
pub struct FieldsProducerGuard<'a>(RwLockReadGuard<'a, Option<Box<dyn FieldsProducer>>>);

impl<'a> Deref for FieldsProducerGuard<'a> {
    type Target = dyn FieldsProducer;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref().unwrap().as_ref()
    }
}

impl<'a> Debug for FieldsProducerGuard<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FieldsProducerGuard")
            .finish_non_exhaustive()
    }
}

/// Read guard that dereferences to the stored norms producer.
pub struct NormsProducerGuard<'a>(RwLockReadGuard<'a, Option<Box<dyn NormsProducer>>>);

impl<'a> Deref for NormsProducerGuard<'a> {
    type Target = dyn NormsProducer;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref().unwrap().as_ref()
    }
}

impl<'a> Debug for NormsProducerGuard<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NormsProducerGuard").finish_non_exhaustive()
    }
}

/// Read guard that dereferences to the stored points reader.
pub struct PointsReaderGuard<'a>(RwLockReadGuard<'a, Option<Box<dyn PointsReader>>>);

impl<'a> Deref for PointsReaderGuard<'a> {
    type Target = dyn PointsReader;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref().unwrap().as_ref()
    }
}

impl<'a> Debug for PointsReaderGuard<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PointsReaderGuard").finish_non_exhaustive()
    }
}

/// Read guard that dereferences to the stored KNN vectors reader.
///
/// Note: the underlying reader's `search` methods require `&mut self`. Use
/// `SegmentReader::search_nearest_vectors` for searching; this guard is intended
/// for read-only inspection of the stored reader.
pub struct KnnVectorsReaderGuard<'a>(RwLockReadGuard<'a, Option<Box<dyn KnnVectorsReader>>>);

impl<'a> Deref for KnnVectorsReaderGuard<'a> {
    type Target = dyn KnnVectorsReader;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref().unwrap().as_ref()
    }
}

impl<'a> Debug for KnnVectorsReaderGuard<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KnnVectorsReaderGuard")
            .finish_non_exhaustive()
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::doc_values::{
        BinaryDocValues, DocValuesConsumer, DocValuesProducer, NumericDocValues, SortedDocValues,
        SortedNumericDocValues, SortedSetDocValues,
    };
    use crate::codecs::field_infos::FieldInfosFormat;
    use crate::codecs::knn_vectors::{
        ByteVectorValues, FloatVectorValues, KnnVectorsFormat, KnnVectorsReader, KnnVectorsWriter,
    };
    use crate::codecs::norms::{NormsConsumer, NormsFormat, NormsProducer};
    use crate::codecs::points::{PointValues, PointsFormat, PointsReader, PointsWriter};
    use crate::codecs::postings::{Fields, FieldsConsumer, FieldsProducer, PostingsFormat};
    use crate::codecs::state::{SegmentReadState, SegmentWriteState};
    use crate::codecs::stored_fields::{
        StoredFieldsFormat, StoredFieldsReader, StoredFieldsWriter,
    };
    use crate::codecs::term_vectors::{TermVectorsFormat, TermVectorsReader, TermVectorsWriter};
    use crate::codecs::tests::{
        test_segment_commit_info, test_segment_info, DummyCodec, DummyFormat,
    };
    use crate::codecs::{
        CompoundDirectory, CompoundFormat, DocValuesFormat, DocValuesSkipper,
        EmptyCompoundDirectory, FilterCodec,
    };
    use crate::error::Result;
    use crate::index::{
        index_reader::IndexReader, DocValuesSkipIndexType, DocValuesType, FieldInfo, FieldInfos,
        IndexOptions, SegmentCommitInfo, SegmentInfo, Terms, VectorEncoding,
        VectorSimilarityFunction,
    };
    use crate::search::knn::KnnCollector;
    use crate::search::AcceptDocs;
    use crate::store::{Directory, IOContext, DEFAULT_IO_CONTEXT};
    use crate::util::{FixedBitSet, Version};

    #[test]
    fn segment_reader_exposes_identity() {
        let info = test_segment_commit_info("_0", 7);
        let reader = SegmentReader::new(
            info.clone(),
            Version::LATEST.major as i32,
            &*DEFAULT_IO_CONTEXT,
        )
        .unwrap();
        assert_eq!(reader.get_segment_name(), "_0");
        assert_eq!(LeafReader::max_doc(&reader), 7);
        assert_eq!(LeafReader::num_docs(&reader), 7);
        assert_eq!(reader.get_segment_info(), &info);
        assert!(!reader.has_deletions());
    }

    #[test]
    fn segment_reader_no_live_docs_when_no_deletions() {
        let info = test_segment_commit_info("_0", 5);
        let reader =
            SegmentReader::new(info, Version::LATEST.major as i32, &*DEFAULT_IO_CONTEXT).unwrap();
        assert!(reader.get_live_docs().is_none());
        assert!(reader.get_hard_live_docs().is_none());
        assert!(!reader.has_deletions());
    }

    #[test]
    fn segment_reader_closes_core_and_doc_values_producers() {
        let info = test_segment_commit_info("_0", 5);
        let reader =
            SegmentReader::new(info, Version::LATEST.major as i32, &*DEFAULT_IO_CONTEXT).unwrap();
        assert_eq!(reader.core_readers().ref_count(), 1);
        reader.close().unwrap();
        assert_eq!(reader.core_readers().ref_count(), 0);
    }

    #[test]
    fn segment_reader_shares_core_on_reference_count() {
        let info = test_segment_commit_info("_0", 5);
        let reader = Arc::new(
            SegmentReader::new(info, Version::LATEST.major as i32, &*DEFAULT_IO_CONTEXT).unwrap(),
        );
        let core = Arc::clone(reader.core_readers());
        assert_eq!(core.ref_count(), 1);
        core.inc_ref().unwrap();
        assert_eq!(core.ref_count(), 2);
        core.dec_ref().unwrap();
        assert_eq!(core.ref_count(), 1);
        Arc::clone(&reader).close().unwrap();
        assert_eq!(core.ref_count(), 0);
    }

    #[test]
    fn segment_reader_loads_empty_field_infos() {
        let info = test_segment_commit_info("_0", 3);
        let reader =
            SegmentReader::new(info, Version::LATEST.major as i32, &*DEFAULT_IO_CONTEXT).unwrap();
        let field_infos = reader.get_field_infos();
        assert!(field_infos.is_empty());
    }

    #[test]
    fn doc_values_leaf_reader_delegates_to_producer() {
        let max_doc = 4;
        let field_infos = FieldInfos::empty();
        let producer: Arc<dyn DocValuesProducer> =
            Arc::new(crate::codecs::doc_values::EmptyDocValuesProducer);
        let reader = DocValuesLeafReader::new(max_doc, field_infos, producer);
        assert_eq!(LeafReader::max_doc(&reader), max_doc);
        assert_eq!(LeafReader::num_docs(&reader), max_doc);
        assert!(reader.get_numeric_doc_values("missing").unwrap().is_none());
    }

    // -------------------------------------------------------------------------
    // Recording test doubles used by the close-coverage and field-info tests.
    // -------------------------------------------------------------------------

    #[derive(Clone, Default, Debug)]
    struct CloseRecorder {
        postings: Arc<AtomicBool>,
        norms: Arc<AtomicBool>,
        stored_fields: Arc<AtomicBool>,
        term_vectors: Arc<AtomicBool>,
        points: Arc<AtomicBool>,
        knn_vectors: Arc<AtomicBool>,
        doc_values: Arc<AtomicBool>,
        compound: Arc<AtomicBool>,
    }

    impl CloseRecorder {
        fn is_closed(&self, component: &str) -> bool {
            let flag = match component {
                "postings" => &self.postings,
                "norms" => &self.norms,
                "stored_fields" => &self.stored_fields,
                "term_vectors" => &self.term_vectors,
                "points" => &self.points,
                "knn_vectors" => &self.knn_vectors,
                "doc_values" => &self.doc_values,
                "compound" => &self.compound,
                _ => panic!("unknown component: {component}"),
            };
            flag.load(Ordering::SeqCst)
        }
    }

    struct RecordingFieldsProducer {
        inner: Box<dyn FieldsProducer>,
        closed: Arc<AtomicBool>,
    }

    impl Debug for RecordingFieldsProducer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("RecordingFieldsProducer")
                .field("closed", &self.closed.load(Ordering::SeqCst))
                .finish_non_exhaustive()
        }
    }

    impl RecordingFieldsProducer {
        fn new(inner: Box<dyn FieldsProducer>, closed: Arc<AtomicBool>) -> Self {
            Self { inner, closed }
        }
    }

    impl Fields for RecordingFieldsProducer {
        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            self.inner.iterator()
        }

        fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
            self.inner.terms(field)
        }

        fn size(&self) -> i32 {
            self.inner.size()
        }
    }

    impl FieldsProducer for RecordingFieldsProducer {
        fn check_integrity(&self) -> Result<()> {
            self.inner.check_integrity()
        }

        fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
            Ok(Box::new(RecordingFieldsProducer::new(
                self.inner.get_merge_instance()?,
                self.closed.clone(),
            )))
        }

        fn close(&mut self) -> Result<()> {
            self.closed.store(true, Ordering::SeqCst);
            self.inner.close()
        }
    }

    #[derive(Debug)]
    struct RecordingNormsProducer {
        inner: Box<dyn NormsProducer>,
        closed: Arc<AtomicBool>,
    }

    impl RecordingNormsProducer {
        fn new(inner: Box<dyn NormsProducer>, closed: Arc<AtomicBool>) -> Self {
            Self { inner, closed }
        }
    }

    impl NormsProducer for RecordingNormsProducer {
        fn get_norms(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
            self.inner.get_norms(field)
        }

        fn check_integrity(&self) -> Result<()> {
            self.inner.check_integrity()
        }

        fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
            Ok(Box::new(RecordingNormsProducer::new(
                self.inner.get_merge_instance()?,
                self.closed.clone(),
            )))
        }

        fn close(&mut self) -> Result<()> {
            self.closed.store(true, Ordering::SeqCst);
            self.inner.close()
        }
    }

    #[derive(Debug)]
    struct RecordingStoredFieldsReader {
        inner: Box<dyn StoredFieldsReader>,
        closed: Arc<AtomicBool>,
    }

    impl RecordingStoredFieldsReader {
        fn new(inner: Box<dyn StoredFieldsReader>, closed: Arc<AtomicBool>) -> Self {
            Self { inner, closed }
        }
    }

    impl StoredFieldsReader for RecordingStoredFieldsReader {
        fn document(&self, doc_id: i32, visitor: &mut dyn StoredFieldVisitor) -> Result<()> {
            self.inner.document(doc_id, visitor)
        }

        fn check_integrity(&self) -> Result<()> {
            self.inner.check_integrity()
        }

        fn clone_reader(&self) -> Box<dyn StoredFieldsReader> {
            Box::new(RecordingStoredFieldsReader::new(
                self.inner.clone_reader(),
                self.closed.clone(),
            ))
        }

        fn get_merge_instance(&self) -> Box<dyn StoredFieldsReader> {
            Box::new(RecordingStoredFieldsReader::new(
                self.inner.get_merge_instance(),
                self.closed.clone(),
            ))
        }

        fn prefetch(&self, doc_id: i32) -> Result<()> {
            self.inner.prefetch(doc_id)
        }

        fn close(&mut self) -> Result<()> {
            self.closed.store(true, Ordering::SeqCst);
            self.inner.close()
        }
    }

    #[derive(Debug)]
    struct RecordingTermVectorsReader {
        inner: Box<dyn TermVectorsReader>,
        closed: Arc<AtomicBool>,
    }

    impl RecordingTermVectorsReader {
        fn new(inner: Box<dyn TermVectorsReader>, closed: Arc<AtomicBool>) -> Self {
            Self { inner, closed }
        }
    }

    impl TermVectorsReader for RecordingTermVectorsReader {
        fn check_integrity(&self) -> Result<()> {
            self.inner.check_integrity()
        }

        fn clone_reader(&self) -> Box<dyn TermVectorsReader> {
            Box::new(RecordingTermVectorsReader::new(
                self.inner.clone_reader(),
                self.closed.clone(),
            ))
        }

        fn get_merge_instance(&self) -> Box<dyn TermVectorsReader> {
            Box::new(RecordingTermVectorsReader::new(
                self.inner.get_merge_instance(),
                self.closed.clone(),
            ))
        }

        fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
            self.inner.get(doc)
        }

        fn prefetch(&self, doc_id: i32) -> Result<()> {
            self.inner.prefetch(doc_id)
        }

        fn close(&mut self) -> Result<()> {
            self.closed.store(true, Ordering::SeqCst);
            self.inner.close()
        }
    }

    #[derive(Debug)]
    struct RecordingPointsReader {
        inner: Box<dyn PointsReader>,
        closed: Arc<AtomicBool>,
    }

    impl RecordingPointsReader {
        fn new(inner: Box<dyn PointsReader>, closed: Arc<AtomicBool>) -> Self {
            Self { inner, closed }
        }
    }

    impl PointsReader for RecordingPointsReader {
        fn check_integrity(&self) -> Result<()> {
            self.inner.check_integrity()
        }

        fn get_values(&self, field: &str) -> Result<Box<dyn PointValues>> {
            self.inner.get_values(field)
        }

        fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>> {
            Ok(Box::new(RecordingPointsReader::new(
                self.inner.get_merge_instance()?,
                self.closed.clone(),
            )))
        }

        fn close(&mut self) -> Result<()> {
            self.closed.store(true, Ordering::SeqCst);
            self.inner.close()
        }
    }

    #[derive(Debug)]
    struct RecordingKnnVectorsReader {
        inner: Box<dyn KnnVectorsReader>,
        closed: Arc<AtomicBool>,
    }

    impl RecordingKnnVectorsReader {
        fn new(inner: Box<dyn KnnVectorsReader>, closed: Arc<AtomicBool>) -> Self {
            Self { inner, closed }
        }
    }

    impl KnnVectorsReader for RecordingKnnVectorsReader {
        fn check_integrity(&self) -> Result<()> {
            self.inner.check_integrity()
        }

        fn get_float_vector_values(&self, field: &str) -> Result<Box<dyn FloatVectorValues>> {
            self.inner.get_float_vector_values(field)
        }

        fn get_byte_vector_values(&self, field: &str) -> Result<Box<dyn ByteVectorValues>> {
            self.inner.get_byte_vector_values(field)
        }

        fn search(
            &mut self,
            field: &str,
            target: &[f32],
            knn_collector: &mut dyn KnnCollector,
            accept_docs: &mut dyn AcceptDocs,
        ) -> Result<()> {
            self.inner.search(field, target, knn_collector, accept_docs)
        }

        fn search_byte(
            &mut self,
            field: &str,
            target: &[u8],
            knn_collector: &mut dyn KnnCollector,
            accept_docs: &mut dyn AcceptDocs,
        ) -> Result<()> {
            self.inner
                .search_byte(field, target, knn_collector, accept_docs)
        }

        fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
            Ok(Box::new(RecordingKnnVectorsReader::new(
                self.inner.get_merge_instance()?,
                self.closed.clone(),
            )))
        }

        fn finish_merge(&mut self) -> Result<()> {
            self.inner.finish_merge()
        }

        fn get_off_heap_byte_size(&self, field_info: &FieldInfo) -> HashMap<String, i64> {
            self.inner.get_off_heap_byte_size(field_info)
        }

        fn close(&mut self) -> Result<()> {
            self.closed.store(true, Ordering::SeqCst);
            self.inner.close()
        }
    }

    #[derive(Debug)]
    struct RecordingDocValuesProducer {
        inner: Box<dyn DocValuesProducer>,
        closed: Arc<AtomicBool>,
    }

    impl RecordingDocValuesProducer {
        fn new(inner: Box<dyn DocValuesProducer>, closed: Arc<AtomicBool>) -> Self {
            Self { inner, closed }
        }
    }

    impl DocValuesProducer for RecordingDocValuesProducer {
        fn get_numeric(
            &self,
            field: &FieldInfo,
        ) -> Result<Box<dyn NumericDocValues + Send + Sync>> {
            self.inner.get_numeric(field)
        }

        fn get_binary(&self, field: &FieldInfo) -> Result<Box<dyn BinaryDocValues + Send + Sync>> {
            self.inner.get_binary(field)
        }

        fn get_sorted(&self, field: &FieldInfo) -> Result<Box<dyn SortedDocValues + Send + Sync>> {
            self.inner.get_sorted(field)
        }

        fn get_sorted_numeric(
            &self,
            field: &FieldInfo,
        ) -> Result<Box<dyn SortedNumericDocValues + Send + Sync>> {
            self.inner.get_sorted_numeric(field)
        }

        fn get_sorted_set(
            &self,
            field: &FieldInfo,
        ) -> Result<Box<dyn SortedSetDocValues + Send + Sync>> {
            self.inner.get_sorted_set(field)
        }

        fn get_skipper(
            &self,
            field: &FieldInfo,
        ) -> Result<Box<dyn DocValuesSkipper + Send + Sync>> {
            self.inner.get_skipper(field)
        }

        fn check_integrity(&self) -> Result<()> {
            self.inner.check_integrity()
        }

        fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
            Ok(Box::new(RecordingDocValuesProducer::new(
                self.inner.get_merge_instance()?,
                self.closed.clone(),
            )))
        }

        fn close(&mut self) -> Result<()> {
            self.closed.store(true, Ordering::SeqCst);
            self.inner.close()
        }
    }

    #[derive(Debug)]
    struct RecordingCompoundDirectory {
        inner: Box<dyn CompoundDirectory>,
        closed: Arc<AtomicBool>,
    }

    impl RecordingCompoundDirectory {
        fn new(inner: Box<dyn CompoundDirectory>, closed: Arc<AtomicBool>) -> Self {
            Self { inner, closed }
        }
    }

    impl CompoundDirectory for RecordingCompoundDirectory {
        fn check_integrity(&self) -> Result<()> {
            self.inner.check_integrity()
        }
    }

    impl Directory for RecordingCompoundDirectory {
        fn list_all(&self) -> Result<Vec<String>> {
            self.inner.list_all()
        }

        fn delete_file(&self, name: &str) -> Result<()> {
            self.inner.delete_file(name)
        }

        fn file_length(&self, name: &str) -> Result<i64> {
            self.inner.file_length(name)
        }

        fn create_output(
            &self,
            name: &str,
            context: &dyn IOContext,
        ) -> Result<Box<dyn crate::store::IndexOutput>> {
            self.inner.create_output(name, context)
        }

        fn create_temp_output(
            &self,
            prefix: &str,
            suffix: &str,
            context: &dyn IOContext,
        ) -> Result<Box<dyn crate::store::IndexOutput>> {
            self.inner.create_temp_output(prefix, suffix, context)
        }

        fn sync(&self, names: &[String]) -> Result<()> {
            self.inner.sync(names)
        }

        fn sync_metadata(&self) -> Result<()> {
            self.inner.sync_metadata()
        }

        fn rename(&self, source: &str, dest: &str) -> Result<()> {
            self.inner.rename(source, dest)
        }

        fn open_input(
            &self,
            name: &str,
            context: &dyn IOContext,
        ) -> Result<Box<dyn crate::store::IndexInput>> {
            self.inner.open_input(name, context)
        }

        fn open_checksum_input(
            &self,
            name: &str,
        ) -> Result<Box<crate::store::BufferedChecksumIndexInput>> {
            self.inner.open_checksum_input(name)
        }

        fn obtain_lock(&self, name: &str) -> Result<Box<dyn crate::store::Lock>> {
            self.inner.obtain_lock(name)
        }

        fn copy_from(
            &self,
            from: &dyn Directory,
            src: &str,
            dest: &str,
            context: &dyn IOContext,
        ) -> Result<()> {
            self.inner.copy_from(from, src, dest, context)
        }

        fn get_pending_deletions(&self) -> Result<HashSet<String>> {
            self.inner.get_pending_deletions()
        }

        fn close(&mut self) -> Result<()> {
            self.closed.store(true, Ordering::SeqCst);
            self.inner.close()
        }
    }

    // Recording formats -------------------------------------------------------

    #[derive(Debug)]
    struct RecordingPostingsFormat {
        delegate: DummyFormat,
        closed: Arc<AtomicBool>,
    }

    impl RecordingPostingsFormat {
        fn new(closed: Arc<AtomicBool>) -> Self {
            Self {
                delegate: DummyFormat::new("dummy-postings"),
                closed,
            }
        }
    }

    impl PostingsFormat for RecordingPostingsFormat {
        fn name(&self) -> &str {
            <DummyFormat as PostingsFormat>::name(&self.delegate)
        }

        fn fields_consumer<'a>(
            &self,
            state: &SegmentWriteState<'a>,
        ) -> Result<Box<dyn FieldsConsumer + 'a>> {
            <DummyFormat as PostingsFormat>::fields_consumer(&self.delegate, state)
        }

        fn fields_producer<'a>(
            &self,
            state: &SegmentReadState<'a>,
        ) -> Result<Box<dyn FieldsProducer>> {
            Ok(Box::new(RecordingFieldsProducer::new(
                <DummyFormat as PostingsFormat>::fields_producer(&self.delegate, state)?,
                self.closed.clone(),
            )))
        }
    }

    #[derive(Debug)]
    struct RecordingNormsFormat {
        delegate: DummyFormat,
        closed: Arc<AtomicBool>,
    }

    impl RecordingNormsFormat {
        fn new(closed: Arc<AtomicBool>) -> Self {
            Self {
                delegate: DummyFormat::new("dummy-norms"),
                closed,
            }
        }
    }

    impl NormsFormat for RecordingNormsFormat {
        fn name(&self) -> &str {
            <DummyFormat as NormsFormat>::name(&self.delegate)
        }

        fn norms_consumer(&self, state: &SegmentWriteState) -> Result<Box<dyn NormsConsumer>> {
            <DummyFormat as NormsFormat>::norms_consumer(&self.delegate, state)
        }

        fn norms_producer(&self, state: &SegmentReadState) -> Result<Box<dyn NormsProducer>> {
            Ok(Box::new(RecordingNormsProducer::new(
                <DummyFormat as NormsFormat>::norms_producer(&self.delegate, state)?,
                self.closed.clone(),
            )))
        }
    }

    #[derive(Debug)]
    struct RecordingStoredFieldsFormat {
        delegate: DummyFormat,
        closed: Arc<AtomicBool>,
    }

    impl RecordingStoredFieldsFormat {
        fn new(closed: Arc<AtomicBool>) -> Self {
            Self {
                delegate: DummyFormat::new("dummy-stored-fields"),
                closed,
            }
        }
    }

    impl StoredFieldsFormat for RecordingStoredFieldsFormat {
        fn name(&self) -> &str {
            <DummyFormat as StoredFieldsFormat>::name(&self.delegate)
        }

        fn fields_reader(
            &self,
            directory: &dyn Directory,
            segment_info: &SegmentInfo,
            field_infos: &FieldInfos,
            context: &dyn IOContext,
        ) -> Result<Box<dyn StoredFieldsReader>> {
            Ok(Box::new(RecordingStoredFieldsReader::new(
                <DummyFormat as StoredFieldsFormat>::fields_reader(
                    &self.delegate,
                    directory,
                    segment_info,
                    field_infos,
                    context,
                )?,
                self.closed.clone(),
            )))
        }

        fn fields_writer(
            &self,
            directory: &dyn Directory,
            segment_info: &SegmentInfo,
            context: &dyn IOContext,
        ) -> Result<Box<dyn StoredFieldsWriter>> {
            <DummyFormat as StoredFieldsFormat>::fields_writer(
                &self.delegate,
                directory,
                segment_info,
                context,
            )
        }
    }

    #[derive(Debug)]
    struct RecordingTermVectorsFormat {
        delegate: DummyFormat,
        closed: Arc<AtomicBool>,
    }

    impl RecordingTermVectorsFormat {
        fn new(closed: Arc<AtomicBool>) -> Self {
            Self {
                delegate: DummyFormat::new("dummy-term-vectors"),
                closed,
            }
        }
    }

    impl TermVectorsFormat for RecordingTermVectorsFormat {
        fn name(&self) -> &str {
            <DummyFormat as TermVectorsFormat>::name(&self.delegate)
        }

        fn vectors_reader(
            &self,
            directory: &dyn Directory,
            segment_info: &SegmentInfo,
            field_infos: &FieldInfos,
            context: &dyn IOContext,
        ) -> Result<Box<dyn TermVectorsReader>> {
            Ok(Box::new(RecordingTermVectorsReader::new(
                <DummyFormat as TermVectorsFormat>::vectors_reader(
                    &self.delegate,
                    directory,
                    segment_info,
                    field_infos,
                    context,
                )?,
                self.closed.clone(),
            )))
        }

        fn vectors_writer(
            &self,
            directory: &dyn Directory,
            segment_info: &SegmentInfo,
            context: &dyn IOContext,
        ) -> Result<Box<dyn TermVectorsWriter>> {
            <DummyFormat as TermVectorsFormat>::vectors_writer(
                &self.delegate,
                directory,
                segment_info,
                context,
            )
        }
    }

    #[derive(Debug)]
    struct RecordingPointsFormat {
        delegate: DummyFormat,
        closed: Arc<AtomicBool>,
    }

    impl RecordingPointsFormat {
        fn new(closed: Arc<AtomicBool>) -> Self {
            Self {
                delegate: DummyFormat::new("dummy-points"),
                closed,
            }
        }
    }

    impl PointsFormat for RecordingPointsFormat {
        fn name(&self) -> &str {
            <DummyFormat as PointsFormat>::name(&self.delegate)
        }

        fn fields_writer(&self, state: &SegmentWriteState) -> Result<Box<dyn PointsWriter>> {
            <DummyFormat as PointsFormat>::fields_writer(&self.delegate, state)
        }

        fn fields_reader(&self, state: &SegmentReadState) -> Result<Box<dyn PointsReader>> {
            Ok(Box::new(RecordingPointsReader::new(
                <DummyFormat as PointsFormat>::fields_reader(&self.delegate, state)?,
                self.closed.clone(),
            )))
        }
    }

    #[derive(Debug)]
    struct RecordingKnnVectorsFormat {
        delegate: DummyFormat,
        closed: Arc<AtomicBool>,
    }

    impl RecordingKnnVectorsFormat {
        fn new(closed: Arc<AtomicBool>) -> Self {
            Self {
                delegate: DummyFormat::new("dummy-knn-vectors"),
                closed,
            }
        }
    }

    impl KnnVectorsFormat for RecordingKnnVectorsFormat {
        fn name(&self) -> &str {
            <DummyFormat as KnnVectorsFormat>::name(&self.delegate)
        }

        fn fields_writer<'a>(
            &self,
            state: &SegmentWriteState<'a>,
        ) -> Result<Box<dyn KnnVectorsWriter + 'a>> {
            <DummyFormat as KnnVectorsFormat>::fields_writer(&self.delegate, state)
        }

        fn fields_reader<'a>(
            &self,
            state: &SegmentReadState<'a>,
        ) -> Result<Box<dyn KnnVectorsReader>> {
            Ok(Box::new(RecordingKnnVectorsReader::new(
                <DummyFormat as KnnVectorsFormat>::fields_reader(&self.delegate, state)?,
                self.closed.clone(),
            )))
        }

        fn get_max_dimensions(&self, field_name: &str) -> i32 {
            <DummyFormat as KnnVectorsFormat>::get_max_dimensions(&self.delegate, field_name)
        }
    }

    #[derive(Debug)]
    struct RecordingDocValuesFormat {
        delegate: DummyFormat,
        closed: Arc<AtomicBool>,
    }

    impl RecordingDocValuesFormat {
        fn new(closed: Arc<AtomicBool>) -> Self {
            Self {
                delegate: DummyFormat::new("dummy-doc-values"),
                closed,
            }
        }
    }

    impl DocValuesFormat for RecordingDocValuesFormat {
        fn name(&self) -> &str {
            <DummyFormat as DocValuesFormat>::name(&self.delegate)
        }

        fn fields_consumer<'a>(
            &self,
            state: &SegmentWriteState<'a>,
        ) -> Result<Box<dyn DocValuesConsumer + 'a>> {
            <DummyFormat as DocValuesFormat>::fields_consumer(&self.delegate, state)
        }

        fn fields_producer<'a>(
            &self,
            state: &SegmentReadState<'a>,
        ) -> Result<Box<dyn DocValuesProducer>> {
            Ok(Box::new(RecordingDocValuesProducer::new(
                <DummyFormat as DocValuesFormat>::fields_producer(&self.delegate, state)?,
                self.closed.clone(),
            )))
        }
    }

    #[derive(Debug, Clone, Default)]
    struct RecordingCompoundFormat {
        closed: Arc<AtomicBool>,
    }

    impl RecordingCompoundFormat {
        fn new(closed: Arc<AtomicBool>) -> Self {
            Self { closed }
        }
    }

    impl CompoundFormat for RecordingCompoundFormat {
        fn name(&self) -> &str {
            "RecordingCompound"
        }

        fn get_compound_reader(
            &self,
            _dir: &dyn Directory,
            _segment_info: &SegmentInfo,
        ) -> Result<Box<dyn CompoundDirectory>> {
            Ok(Box::new(RecordingCompoundDirectory::new(
                Box::new(EmptyCompoundDirectory),
                self.closed.clone(),
            )))
        }

        fn write(
            &self,
            _dir: &dyn Directory,
            _segment_info: &SegmentInfo,
            _context: &dyn IOContext,
        ) -> Result<()> {
            Ok(())
        }
    }

    #[derive(Debug, Copy, Clone, Default)]
    struct AllFlagsFieldInfosFormat;

    impl FieldInfosFormat for AllFlagsFieldInfosFormat {
        fn name(&self) -> &str {
            "AllFlagsFieldInfos"
        }

        fn read(
            &self,
            _directory: &dyn Directory,
            _segment_info: &SegmentInfo,
            _segment_suffix: &str,
            _context: &dyn IOContext,
        ) -> Result<FieldInfos> {
            let field = FieldInfo::new_full(
                "all_flags_field",
                0,
                true,  // store_term_vector
                false, // omit_norms
                false, // store_payloads
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
                DocValuesType::NUMERIC,
                DocValuesSkipIndexType::NONE,
                -1, // doc_values_gen
                HashMap::new(),
                1, // point_dimension_count
                1, // point_index_dimension_count
                4, // point_num_bytes
                2, // vector_dimension
                VectorEncoding::FLOAT32,
                VectorSimilarityFunction::EUCLIDEAN,
                false, // soft_deletes_field
                false, // is_parent_field
            )?;
            FieldInfos::new(vec![field])
        }

        fn write(
            &self,
            _directory: &dyn Directory,
            _segment_info: &SegmentInfo,
            _segment_suffix: &str,
            _infos: &FieldInfos,
            _context: &dyn IOContext,
        ) -> Result<()> {
            Ok(())
        }
    }

    fn recording_segment_commit_info(
        name: &str,
        max_doc: i32,
        recorder: &CloseRecorder,
    ) -> SegmentCommitInfo {
        let info = test_segment_info(name, max_doc);

        let dummy = Arc::new(DummyCodec::new("Dummy"));
        let codec = FilterCodec::new("Recording", dummy)
            .with_field_infos_format(AllFlagsFieldInfosFormat)
            .with_postings_format(RecordingPostingsFormat::new(recorder.postings.clone()))
            .with_norms_format(RecordingNormsFormat::new(recorder.norms.clone()))
            .with_stored_fields_format(RecordingStoredFieldsFormat::new(
                recorder.stored_fields.clone(),
            ))
            .with_term_vectors_format(RecordingTermVectorsFormat::new(
                recorder.term_vectors.clone(),
            ))
            .with_points_format(RecordingPointsFormat::new(recorder.points.clone()))
            .with_knn_vectors_format(RecordingKnnVectorsFormat::new(recorder.knn_vectors.clone()))
            .with_doc_values_format(RecordingDocValuesFormat::new(recorder.doc_values.clone()))
            .with_compound_format(RecordingCompoundFormat::new(recorder.compound.clone()));

        let si = SegmentInfo::new(
            info.directory.clone(),
            Version::LUCENE_10_5_0,
            Some(Version::LUCENE_10_5_0),
            name.to_string(),
            max_doc,
            true,
            false,
            Arc::new(codec),
            HashMap::new(),
            [0u8; crate::util::string_helper::ID_LENGTH],
            HashMap::new(),
            crate::search::Sort::default(),
        )
        .expect("recording segment info should be valid");

        SegmentCommitInfo::new(
            si,
            0,
            0,
            -1,
            -1,
            -1,
            [0u8; crate::util::string_helper::ID_LENGTH],
        )
        .expect("recording segment commit info should be valid")
    }

    // -------------------------------------------------------------------------
    // Tests
    // -------------------------------------------------------------------------

    #[test]
    fn segment_reader_closes_all_opened_producers() {
        let recorder = CloseRecorder::default();
        let info = recording_segment_commit_info("_0", 5, &recorder);
        let reader = SegmentReader::new(info, Version::LATEST.major as i32, &*DEFAULT_IO_CONTEXT)
            .expect("segment reader should open");

        assert!(reader.core_readers().ref_count() > 0);
        reader.close().unwrap();

        assert!(
            recorder.is_closed("postings"),
            "postings producer should be closed"
        );
        assert!(
            recorder.is_closed("norms"),
            "norms producer should be closed"
        );
        assert!(
            recorder.is_closed("stored_fields"),
            "stored fields reader should be closed"
        );
        assert!(
            recorder.is_closed("term_vectors"),
            "term vectors reader should be closed"
        );
        assert!(
            recorder.is_closed("points"),
            "points reader should be closed"
        );
        assert!(
            recorder.is_closed("knn_vectors"),
            "knn vectors reader should be closed"
        );
        assert!(
            recorder.is_closed("doc_values"),
            "doc-values producer should be closed"
        );
        assert!(
            recorder.is_closed("compound"),
            "compound directory should be closed"
        );
    }

    #[test]
    fn segment_reader_num_docs_respects_deletion_bitset() {
        let max_doc = 6;
        let mut del_docs = FixedBitSet::new(max_doc as usize);
        del_docs.set(1);
        del_docs.set(3);
        let del_count = del_docs.cardinality() as i32;
        let info = {
            let mut i = test_segment_commit_info("_0", max_doc);
            i.set_del_count(del_count).unwrap();
            i
        };

        let parent = SegmentReader::new(
            test_segment_commit_info("_1", max_doc),
            Version::LATEST.major as i32,
            &*DEFAULT_IO_CONTEXT,
        )
        .unwrap();
        let reader = SegmentReader::new_shared(
            info,
            &parent,
            Some(Box::new(del_docs)),
            Some(Box::new(FixedBitSet::new(max_doc as usize))),
            max_doc - del_count,
            false,
        )
        .unwrap();

        assert_eq!(LeafReader::max_doc(&reader), max_doc);
        assert_eq!(LeafReader::num_docs(&reader), max_doc - del_count);
        assert!(reader.has_deletions());
    }

    #[test]
    fn segment_reader_field_infos_match_loaded_values() {
        let recorder = CloseRecorder::default();
        let info = recording_segment_commit_info("_0", 4, &recorder);
        let reader = SegmentReader::new(info, Version::LATEST.major as i32, &*DEFAULT_IO_CONTEXT)
            .expect("segment reader should open");

        let field_infos = reader.get_field_infos();
        assert_eq!(field_infos.size(), 1);

        let field = field_infos.field_info("all_flags_field").unwrap();
        assert!(field.has_term_vectors());
        assert!(field.has_norms());
        assert_eq!(field.get_doc_values_type(), DocValuesType::NUMERIC);
        assert!(field.get_point_dimension_count() > 0);
        assert!(field.get_vector_dimension() > 0);
    }
}
