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
use std::fmt::{Debug, Display, Formatter};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, RwLock};

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
use crate::codecs::term_vectors::TermVectorsReader;
use crate::codecs::DocValuesSkipper;
use crate::error::{LuceneError, LuceneErrorKind, Result};
use crate::index::codec_reader::CodecReader;
use crate::index::index_reader::{
    add_closed_listener_to_set, java_object_to_string, CacheHelper, CacheKey, ClosedListener,
    EnsureOpen, IndexReaderCore, StoredFields,
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
use crate::util::bit_set::LiveDocs;
use crate::util::{Bits, DenseLiveDocs, RefCount, RefCountRelease, SparseLiveDocs};

// -----------------------------------------------------------------------------
// SegmentCoreReaders
// -----------------------------------------------------------------------------

/// Holds the core codec readers that are shared (unchanged) when a
/// `SegmentReader` is cloned or reopened.
///
/// Equivalent to `org.apache.lucene.index.SegmentCoreReaders`.
pub struct SegmentCoreReaders {
    fields: RwLock<Option<Arc<dyn FieldsProducer>>>,
    norms_producer: RwLock<Option<Arc<dyn NormsProducer>>>,
    fields_reader_orig: RwLock<Option<Box<dyn StoredFieldsReader>>>,
    term_vectors_reader_orig: RwLock<Option<Box<dyn TermVectorsReader>>>,
    points_reader: RwLock<Option<Arc<dyn PointsReader>>>,
    knn_vectors_reader: RwLock<Option<Arc<dyn KnnVectorsReader>>>,
    cfs_reader: RwLock<Option<Box<dyn CompoundDirectory>>>,
    segment: String,
    core_field_infos: FieldInfos,
    ref_count: AtomicI32,
    /// Java's `coreCacheKey`, created with the core.
    core_cache_key: CacheKey,
    core_closed_listeners: Mutex<Vec<Arc<dyn ClosedListener>>>,
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
            .ok_or_else(crate::error::LuceneError::null_pointer)?;

        // `catch (EOFException | FileNotFoundException e)` and `catch
        // (NoSuchFileException e)` around the whole body, the compound reader
        // included (`SegmentCoreReaders.java:81-146`).
        let wrap = |error: LuceneError| Self::wrap_missing_file_error(dir, error);
        let cfs_reader: Option<Box<dyn CompoundDirectory>> = if si.info.get_use_compound_file() {
            Some(
                codec
                    .compound_format()
                    .get_compound_reader(dir, &si.info)
                    .map_err(wrap)?,
            )
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
            core_cache_key: CacheKey::new(),
            core_closed_listeners: Mutex::new(Vec::new()),
        };

        let result: Result<()> = (|| {
            let cfs_dir_guard = core
                .cfs_reader
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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
                *core
                    .fields
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::from(
                    codec
                        .postings_format()
                        .fields_producer(&segment_read_state)?,
                ));
            }

            if core.core_field_infos.has_norms() {
                *core
                    .norms_producer
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::from(
                    codec.norms_format().norms_producer(&segment_read_state)?,
                ));
            }

            *core
                .fields_reader_orig
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(codec.stored_fields_format().fields_reader(
                    cfs_dir,
                    &si.info,
                    &core.core_field_infos,
                    context,
                )?);

            if core.core_field_infos.has_term_vectors() {
                *core
                    .term_vectors_reader_orig
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some(codec.term_vectors_format().vectors_reader(
                        cfs_dir,
                        &si.info,
                        &core.core_field_infos,
                        context,
                    )?);
            }

            if core.core_field_infos.has_point_values() {
                *core
                    .points_reader
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::from(
                    codec.points_format().fields_reader(&segment_read_state)?,
                ));
            }

            if core.core_field_infos.has_vector_values() {
                *core
                    .knn_vectors_reader
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::from(
                    codec
                        .knn_vectors_format()
                        .fields_reader(&segment_read_state)?,
                ));
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
        result.map_err(wrap)?;
        Ok(core)
    }

    /// The `catch` clauses of the constructor (`SegmentCoreReaders.java:
    /// 143-146`): an `EOFException` or `FileNotFoundException` becomes
    /// `CorruptIndexException("Problem reading index from " + dir,
    /// dir.toString(), e)`, a `NoSuchFileException` becomes
    /// `CorruptIndexException("Problem reading index.", e.getFile(), e)`;
    /// every other error passes through.
    fn wrap_missing_file_error(dir: &dyn Directory, error: LuceneError) -> LuceneError {
        use crate::store::java_nio_file as nio;
        if !error.is_eof_or_missing_file() {
            return error;
        }
        let no_such_file = match &error.kind {
            LuceneErrorKind::Io(io_error) => nio::java_exception_of(io_error)
                .filter(|exception| exception.class_name() == nio::NO_SUCH_FILE_EXCEPTION)
                .map(|exception| exception.get_file().unwrap_or("null").to_string()),
            _ => None,
        };
        match no_such_file {
            Some(file) => {
                LuceneError::corrupt_index_with_cause("Problem reading index.", file, error)
            }
            None => {
                let dir_string = dir.directory_to_string();
                LuceneError::corrupt_index_with_cause(
                    format!("Problem reading index from {dir_string}"),
                    dir_string,
                    error,
                )
            }
        }
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
        Err(LuceneError::already_closed(
            "SegmentCoreReaders is already closed".to_string(),
        ))
    }

    /// Decrements the reference count of this core, closing all codec readers
    /// when it reaches zero.
    pub fn dec_ref(&self) -> Result<()> {
        let rc = self.ref_count.fetch_sub(1, Ordering::SeqCst) - 1;
        if rc == 0 {
            // `try (Closeable finalizer = this::notifyCoreClosedListeners) {
            // IOUtils.close(...); }` (`SegmentCoreReaders.java:170-181`): the
            // producers close first and the listeners are notified after,
            // however the closing ends — with an error, or with the panic
            // that is the port's form of an unchecked throwable. A failure to
            // close wins, the listeners' failure then being suppressed.
            let closed =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.close_producers()));
            let notified = if matches!(closed, Ok(Ok(()))) {
                self.notify_core_closed_listeners()
            } else {
                // The finalizer's failure, error or panic, is suppressed.
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.notify_core_closed_listeners()
                }))
                .unwrap_or(Ok(()))
            };
            match closed {
                Ok(closed) => closed?,
                Err(payload) => std::panic::resume_unwind(payload),
            }
            return notified;
        } else if rc < 0 {
            return Err(LuceneError::illegal_state(format!(
                "too many decRef calls: refCount is {rc} after decrement"
            )));
        }
        Ok(())
    }

    /// `IOUtils.close(fields, termVectorsReaderOrig, fieldsReaderOrig,
    /// cfsReader, normsProducer, pointsReader, knnVectorsReader)`
    /// (`SegmentCoreReaders.java:172-179`): every one is closed, in Java's
    /// order, and the first failure is returned.
    ///
    /// Java's fields are `final`: a closed producer stays where it is, and a
    /// `SegmentReader` whose own `doClose()` ran without it being closed (a
    /// `FilterCodecReader` calls its delegate's `doClose()`,
    /// `FilterCodecReader.java:123-126`) still hands it out — cloning a closed
    /// stored-fields reader throws `this FieldsReader is closed`, cloning a
    /// closed term-vectors reader throws its closed input's `Already closed.`,
    /// and `checkIntegrity()` reaches the closed postings first. The producers
    /// are therefore closed in place, never taken out of their slots.
    fn close_producers(&self) -> Result<()> {
        use crate::util::IOUtils;
        IOUtils::close([
            IOUtils::closeable(|| close_lock_shared(&self.fields, |producer| producer.close())),
            IOUtils::closeable(|| {
                close_lock(&self.term_vectors_reader_orig, |reader| reader.close())
            }),
            IOUtils::closeable(|| close_lock(&self.fields_reader_orig, |reader| reader.close())),
            IOUtils::closeable(|| close_lock(&self.cfs_reader, |reader| reader.close())),
            IOUtils::closeable(|| {
                close_lock_shared(&self.norms_producer, |producer| producer.close())
            }),
            IOUtils::closeable(|| {
                close_lock_shared(&self.points_reader, |producer| producer.close())
            }),
            IOUtils::closeable(|| {
                close_lock_shared(&self.knn_vectors_reader, |producer| producer.close())
            }),
        ])
    }

    fn notify_core_closed_listeners(&self) -> Result<()> {
        let guard = self
            .core_closed_listeners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // `IOUtils.applyToAll(coreClosedListeners, l -> l.onClose(cacheHelper.getKey()))`
        // (`SegmentCoreReaders.java:203-207`): every listener runs; the first
        // error is returned with the later ones suppressed into it.
        crate::util::IOUtils::apply_to_all(guard.iter().map(Some).collect(), |listener| {
            listener.on_close(self.core_cache_key.clone())
        })
    }
}

/// Closes the value stored in `lock` in place: the closed value stays in its
/// slot, as Java's `final` field keeps the closed producer.
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
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(value) = guard.as_mut() {
        close(value)?;
    }
    Ok(())
}

/// Closes a producer held behind an `Arc` in place whoever else still shares
/// it, as Java's `close()` closes the one producer object: the closed producer
/// stays in its slot, as Java's `final` field keeps it.
fn close_lock_shared<T: ?Sized>(
    lock: &RwLock<Option<Arc<T>>>,
    close: impl FnOnce(&T) -> Result<()>,
) -> Result<()> {
    let guard = lock
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match guard.as_deref() {
        Some(inner) => close(inner),
        None => Ok(()),
    }
}

impl Drop for SegmentCoreReaders {
    fn drop(&mut self) {
        if self.ref_count.load(Ordering::SeqCst) > 0 {
            let _ = self.dec_ref();
        }
    }
}

impl CacheHelper for SegmentCoreReaders {
    /// Equivalent to `SegmentCoreReaders.getCacheHelper()`
    /// (`SegmentCoreReaders.java:199-201`) and to the anonymous
    /// `IndexReader.CacheHelper` it hands back, whose `getKey()` answers
    /// `coreCacheKey`.
    ///
    /// Java keeps that anonymous object in a field and returns it; both of its
    /// methods read state that lives on `SegmentCoreReaders` itself, so this
    /// port implements the trait on the type rather than building a second
    /// object to hold a borrow of it — which is why there is no `get_cache_helper`
    /// to call.
    fn get_key(&self) -> &CacheKey {
        &self.core_cache_key
    }

    /// The `addClosedListener` of the same anonymous `CacheHelper`, which
    /// adds to the `coreClosedListeners` set (`SegmentCoreReaders.java:194-196`).
    fn add_closed_listener(&self, listener: Arc<dyn ClosedListener>) {
        {
            let mut listeners = self
                .core_closed_listeners
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            add_closed_listener_to_set(&mut listeners, listener);
        }
    }
}

// -----------------------------------------------------------------------------
// SegmentDocValues
// -----------------------------------------------------------------------------

/// The reference-counted producers of a [`SegmentDocValues`], by generation.
///
/// Equivalent to `SegmentDocValues.genDVProducers`, a
/// `LongObjectHashMap<RefCount<DocValuesProducer>>`.
type GenProducers = HashMap<i64, Arc<RefCount<Arc<SharedDocValuesProducer>>>>;

/// Manages the [`DocValuesProducer`]s held by `SegmentReader`s and keeps track
/// of their reference counts.
///
/// Equivalent to `org.apache.lucene.index.SegmentDocValues`.
///
/// Java's two methods are `synchronized` on the instance, and the anonymous
/// `RefCount.release()` of each producer takes that same monitor again to
/// drop its generation from the map (`SegmentDocValues.java:56-63`). A Rust
/// `Mutex` is not reentrant, so the one monitor becomes two locks: `monitor`,
/// held for the whole of [`get_doc_values_producer`](Self::get_doc_values_producer)
/// and [`dec_ref`](Self::dec_ref), and the map's own lock, taken briefly
/// for each read or write of the map — including the one `release()` makes
/// while `dec_ref` holds the monitor. Every change to the map therefore still
/// happens under the monitor, as in Java.
#[derive(Debug, Default)]
pub struct SegmentDocValues {
    monitor: Mutex<()>,
    gen_dv_producers: Arc<Mutex<GenProducers>>,
}

impl SegmentDocValues {
    /// Creates an empty manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Locks the producer map for one read or write.
    fn producers(map: &Mutex<GenProducers>) -> std::sync::MutexGuard<'_, GenProducers> {
        map.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Returns the doc-values producer for the given generation, creating and
    /// caching it if necessary.
    ///
    /// Equivalent to the `synchronized`
    /// `SegmentDocValues.getDocValuesProducer(long, SegmentCommitInfo,
    /// Directory, FieldInfos)` (`SegmentDocValues.java:67-79`): a cached
    /// producer gets `incRef()`, a new one starts at a count of one, and the
    /// caller receives `dvp.get()`.
    ///
    /// # Errors
    ///
    /// Propagates the error opening a new producer raises.
    pub fn get_doc_values_producer(
        &self,
        gen: i64,
        si: &SegmentCommitInfo,
        dir: &dyn Directory,
        infos: &FieldInfos,
    ) -> Result<Arc<dyn DocValuesProducer>> {
        let _monitor = self
            .monitor
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cached = Self::producers(&self.gen_dv_producers).get(&gen).cloned();
        let dvp = match cached {
            Some(dvp) => {
                dvp.inc_ref();
                dvp
            }
            None => {
                let dvp = self.new_doc_values_producer(si, dir, gen, infos)?;
                Self::producers(&self.gen_dv_producers).insert(gen, Arc::clone(&dvp));
                dvp
            }
        };
        Ok(Arc::clone(dvp.get()) as Arc<dyn DocValuesProducer>)
    }

    /// Opens the doc-values producer of generation `gen`, wrapped in the
    /// [`RefCount`] this manager caches.
    ///
    /// Generation `-1` is the segment's own doc values, read from `dir` with no
    /// segment suffix; any other generation was written by a doc-values update,
    /// which always lands outside the compound file, so it is read from the
    /// segment's own directory under the base-36 generation suffix.
    ///
    /// Equivalent to the private
    /// `SegmentDocValues.newDocValuesProducer(SegmentCommitInfo, Directory, long, FieldInfos)`
    /// (`SegmentDocValues.java:35-64`), whose anonymous `RefCount` subclass
    /// overrides `release()` to close the producer and then drop `gen` from
    /// the map; the override is the release closure here.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalState`] when the segment carries no codec,
    /// and propagates whatever the doc-values format raises while opening the
    /// generation's files.
    fn new_doc_values_producer(
        &self,
        si: &SegmentCommitInfo,
        dir: &dyn Directory,
        gen: i64,
        infos: &FieldInfos,
    ) -> Result<Arc<RefCount<Arc<SharedDocValuesProducer>>>> {
        let (dv_dir, segment_suffix): (&dyn Directory, String) = if gen == -1 {
            (dir, String::new())
        } else {
            // Gen'd files are written outside the compound file, so use the
            // segment info's own directory.
            (&*si.info.directory, radix36(gen))
        };

        // The read state lists only the fields relevant to this generation;
        // its context is `IOContext.DEFAULT` (`SegmentDocValues.java:48-49`).
        let state = crate::codecs::state::SegmentReadState::with_suffix(
            dv_dir,
            &si.info,
            infos,
            &*crate::store::DEFAULT_IO_CONTEXT,
            segment_suffix,
        );

        let codec = si
            .info
            .codec()
            .ok_or_else(crate::error::LuceneError::null_pointer)?;
        let producer = codec.doc_values_format().fields_producer(&state)?;
        Ok(self.ref_counted(gen, producer))
    }

    /// The anonymous `RefCount<DocValuesProducer>` subclass of
    /// `newDocValuesProducer` (`SegmentDocValues.java:53-63`) over `producer`:
    /// a count of one, and a `release()` that closes the producer and then,
    /// under the monitor its caller holds, drops `gen` from the map.
    fn ref_counted(
        &self,
        gen: i64,
        producer: Box<dyn DocValuesProducer>,
    ) -> Arc<RefCount<Arc<SharedDocValuesProducer>>> {
        let gen_dv_producers = Arc::downgrade(&self.gen_dv_producers);
        let release: RefCountRelease<Arc<SharedDocValuesProducer>> =
            Box::new(move |object: &Arc<SharedDocValuesProducer>| {
                // `object.close(); synchronized (SegmentDocValues.this) {
                // genDVProducers.remove(gen); }`
                object.close_shared()?;
                if let Some(map) = gen_dv_producers.upgrade() {
                    Self::producers(&map).remove(&gen);
                }
                Ok(())
            });
        Arc::new(RefCount::with_release(
            Arc::new(SharedDocValuesProducer::new(producer)),
            release,
        ))
    }

    /// Decrements the reference counts for the given doc-values producer
    /// generations. When a generation's count reaches zero, its release closes
    /// the producer and removes it from the cache.
    ///
    /// Equivalent to the `synchronized` `SegmentDocValues.decRef(LongArrayList)`
    /// (`SegmentDocValues.java:82-90`), which applies `decRef()` to every
    /// generation with `IOUtils.applyToAll`.
    ///
    /// # Errors
    ///
    /// Returns the first error a release raised, the later ones suppressed
    /// into it, as `IOUtils.applyToAll` does; a failed release puts its
    /// reference back, as `RefCount.decRef` does.
    pub fn dec_ref(&self, gens: &[i64]) -> Result<()> {
        let _monitor = self
            .monitor
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::util::IOUtils::apply_to_all(gens.iter().map(|gen| Some(*gen)).collect(), |gen| {
            let dvp = Self::producers(&self.gen_dv_producers).get(&gen).cloned();
            // `assert dvp != null : "gen=" + gen`.
            debug_assert!(dvp.is_some(), "gen={gen}");
            match dvp {
                Some(dvp) => dvp.dec_ref(),
                None => Ok(()),
            }
        })
    }
}

/// A doc-values producer that the readers of a segment share and that its
/// [`RefCount`] release closes.
///
/// Java's `RefCount.release()` calls `object.close()` on the very producer
/// every reader holds (`SegmentDocValues.java:56-58`), which stays referenced
/// afterwards; a reader that still calls it reaches the closed producer, as
/// it does here: [`DocValuesProducer::close`] takes `&self`, so the shared
/// producer is closed in place and kept.
#[derive(Debug)]
struct SharedDocValuesProducer {
    producer: Box<dyn DocValuesProducer>,
}

impl SharedDocValuesProducer {
    fn new(producer: Box<dyn DocValuesProducer>) -> Self {
        Self { producer }
    }

    /// Closes the producer, as `object.close()` does in the release.
    ///
    /// # Errors
    ///
    /// Propagates the producer's close error.
    fn close_shared(&self) -> Result<()> {
        self.producer.close()
    }
}

impl DocValuesProducer for SharedDocValuesProducer {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.producer.get_numeric(field)
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        self.producer.get_binary(field)
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        self.producer.get_sorted(field)
    }

    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        self.producer.get_sorted_numeric(field)
    }

    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        self.producer.get_sorted_set(field)
    }

    fn get_skipper(&self, field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        self.producer.get_skipper(field)
    }

    fn check_integrity(&self) -> Result<()> {
        self.producer.check_integrity()
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        self.producer.get_merge_instance()
    }

    fn close(&self) -> Result<()> {
        self.producer.close()
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
    fn get_numeric(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        {
            debug_assert!(self.dv_producers_by_field.contains_key(&field.number));
            self.dv_producers_by_field
                .get(&field.number)
                .ok_or_else(LuceneError::null_pointer)?
                .get_numeric(field)
        }
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        {
            debug_assert!(self.dv_producers_by_field.contains_key(&field.number));
            self.dv_producers_by_field
                .get(&field.number)
                .ok_or_else(LuceneError::null_pointer)?
                .get_binary(field)
        }
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        {
            debug_assert!(self.dv_producers_by_field.contains_key(&field.number));
            self.dv_producers_by_field
                .get(&field.number)
                .ok_or_else(LuceneError::null_pointer)?
                .get_sorted(field)
        }
    }

    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        {
            debug_assert!(self.dv_producers_by_field.contains_key(&field.number));
            self.dv_producers_by_field
                .get(&field.number)
                .ok_or_else(LuceneError::null_pointer)?
                .get_sorted_numeric(field)
        }
    }

    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        {
            debug_assert!(self.dv_producers_by_field.contains_key(&field.number));
            self.dv_producers_by_field
                .get(&field.number)
                .ok_or_else(LuceneError::null_pointer)?
                .get_sorted_set(field)
        }
    }

    /// Equivalent to `SegmentDocValuesProducer.getSkipper(FieldInfo)`
    /// (`index/SegmentDocValuesProducer.java:126-131`): the answer of the
    /// producer serving the field, `None` included.
    fn get_skipper(&self, field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        debug_assert!(self.dv_producers_by_field.contains_key(&field.number));
        self.dv_producers_by_field
            .get(&field.number)
            .ok_or_else(LuceneError::null_pointer)?
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

    fn close(&self) -> Result<()> {
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
    field_infos: FieldInfos,
    doc_values_producer: Arc<dyn DocValuesProducer>,
}

impl Display for DocValuesLeafReader {
    /// `org.apache.lucene.index.DocValuesLeafReader` declares no `toString()`,
    /// so `java.lang.Object`'s applies: the binary class name, `@`, and the
    /// identity hash in hexadecimal. See [`java_object_to_string`].
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&java_object_to_string(
            "org.apache.lucene.index.DocValuesLeafReader",
            self,
        ))
    }
}

impl DocValuesLeafReader {
    /// Creates a doc-values-only reader.
    pub fn new(field_infos: FieldInfos, doc_values_producer: Arc<dyn DocValuesProducer>) -> Self {
        Self {
            core: IndexReaderCore::new(),
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
        // `throw new UnsupportedOperationException()`.
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn num_docs(&self) -> i32 {
        // `throw new UnsupportedOperationException()` (`DocValuesLeafReader.java`).
        crate::store::java_nio_file::throw_unchecked(
            "java.lang.UnsupportedOperationException",
            None,
        )
    }

    fn max_doc(&self) -> i32 {
        // `throw new UnsupportedOperationException()` (`DocValuesLeafReader.java`).
        crate::store::java_nio_file::throw_unchecked(
            "java.lang.UnsupportedOperationException",
            None,
        )
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        // `throw new UnsupportedOperationException()`.
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn do_close(&self) -> Result<()> {
        // `throw new UnsupportedOperationException()`.
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        // `throw new UnsupportedOperationException()` (`DocValuesLeafReader.java`).
        crate::store::java_nio_file::throw_unchecked(
            "java.lang.UnsupportedOperationException",
            None,
        )
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        // `throw new UnsupportedOperationException()` (`DocValuesLeafReader.java`).
        crate::store::java_nio_file::throw_unchecked(
            "java.lang.UnsupportedOperationException",
            None,
        )
    }

    fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
        // `throw new UnsupportedOperationException()`.
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn get_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn IndexNumericDocValues>>> {
        if let Some(fi) = self.field_infos.field_info(field) {
            if fi.doc_values_type == DocValuesType::NUMERIC {
                return self.doc_values_producer.get_numeric(fi);
            }
        }
        Ok(None)
    }

    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn IndexBinaryDocValues>>> {
        if let Some(fi) = self.field_infos.field_info(field) {
            if fi.doc_values_type == DocValuesType::BINARY {
                return self.doc_values_producer.get_binary(fi);
            }
        }
        Ok(None)
    }

    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn IndexSortedDocValues>>> {
        if let Some(fi) = self.field_infos.field_info(field) {
            if fi.doc_values_type == DocValuesType::SORTED {
                return self.doc_values_producer.get_sorted(fi);
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
                return self.doc_values_producer.get_sorted_numeric(fi);
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
                return self.doc_values_producer.get_sorted_set(fi);
            }
        }
        Ok(None)
    }

    fn get_norm_values(&self, _field: &str) -> Result<Option<Box<dyn IndexNumericDocValues>>> {
        // `throw new UnsupportedOperationException()` (`DocValuesLeafReader.java`).
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn get_doc_values_skipper(
        &self,
        _field: &str,
    ) -> Result<Option<Box<dyn IndexDocValuesSkipper>>> {
        // `throw new UnsupportedOperationException()` (`DocValuesLeafReader.java`).
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn get_float_vector_values(&self, _field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        // `throw new UnsupportedOperationException()` (`DocValuesLeafReader.java`).
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn get_byte_vector_values(&self, _field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        // `throw new UnsupportedOperationException()` (`DocValuesLeafReader.java`).
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn search_nearest_vectors(
        &self,
        _field: &str,
        _target: &[f32],
        _collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        // `throw new UnsupportedOperationException()` (`DocValuesLeafReader.java`).
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn search_nearest_vectors_byte(
        &self,
        _field: &str,
        _target: &[u8],
        _collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        // `throw new UnsupportedOperationException()` (`DocValuesLeafReader.java`).
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn get_field_infos(&self) -> FieldInfos {
        self.field_infos.clone()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        // `throw new UnsupportedOperationException()` (`DocValuesLeafReader.java`).
        crate::store::java_nio_file::throw_unchecked(
            "java.lang.UnsupportedOperationException",
            None,
        )
    }

    fn get_point_values(&self, _field: &str) -> Result<Option<Box<dyn IndexPointValues>>> {
        // `throw new UnsupportedOperationException()` (`DocValuesLeafReader.java`).
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn check_integrity(&self) -> Result<()> {
        // `throw new UnsupportedOperationException()` (`DocValuesLeafReader.java`).
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn get_meta_data(&self) -> LeafMetaData {
        // `throw new UnsupportedOperationException()`
        // (`DocValuesLeafReader.java:81-83`).
        panic!("java.lang.UnsupportedOperationException")
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

impl Display for SegmentReader {
    /// Port of `SegmentReader.toString()` (`SegmentReader.java:296-301`).
    ///
    /// Java reverse-computes the *pending* deletion count — the deletes this
    /// reader knows about that the commit does not — and hands it to
    /// `SegmentCommitInfo.toString(int)`, which adds it to the committed
    /// `delCount` before rendering. Note that the maximum comes from
    /// `si.info.maxDoc()` rather than this reader's own `maxDoc()`.
    ///
    /// Measured against `lucene-core-10.5.0.jar`: a three-document segment with
    /// no deletions renders as `_0(10.5.0):C3`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        // `self.max_doc` is initialised from `segment_info.info.max_doc()`, which
        // is the very value Java reads here — `SegmentReader.maxDoc()` returns
        // `si.info.maxDoc()` too. Reading the field avoids the fallible
        // accessor, which `Display` has no way to propagate.
        let pending = self.max_doc - self.num_docs - self.segment_info.get_del_count();
        f.write_str(&self.segment_info.to_string_with_pending_del_count(pending))
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
        let si = segment_info.java_clone(); // `this.si = si.clone()` (`SegmentReader.java:83`)
        let original_si = segment_info;
        let dir: &dyn Directory = &*original_si.info.directory;

        // `si.info.getIndexSort()`, `null` for a segment without an index
        // sort (`SegmentReader.java:85-90`).
        let index_sort = original_si.info.index_sort();
        let meta_data = LeafMetaData::new(
            created_version_major,
            original_si.info.min_version(),
            index_sort.cloned(),
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
                    let codec = original_si_for_closure
                        .info
                        .codec()
                        .ok_or_else(crate::error::LuceneError::null_pointer)?;
                    Some(SharedBits::new(codec.live_docs_format().read_live_docs(
                        dir,
                        &original_si_for_closure,
                        &*READONCE_IO_CONTEXT,
                    )?))
                } else {
                    debug_assert!(original_si_for_closure.get_del_count() == 0);
                    None
                };
                let hard_live_docs = live_docs.clone();

                let num_docs = max_doc - original_si_for_closure.get_del_count();

                // Java: `assert assertLiveDocs(isNRT, hardLiveDocs, liveDocs)`
                // (`SegmentReader.java:113`), with `isNRT` false here.
                debug_assert!(Self::assert_live_docs(
                    false,
                    hard_live_docs.as_ref(),
                    live_docs.as_ref()
                ));

                let cfs_dir_guard = core_readers
                    .cfs_reader
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
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

                // The two helpers are anonymous inner classes in Java, whose
                // `addClosedListener` calls this reader's `ensureOpen()`
                // (`SegmentReader.java:339-343, 363-367`); they reach the
                // reader's state through a handle on its core.
                let core = IndexReaderCore::new();
                let reader_cache_helper = SegmentReaderCacheHelper::new(core.ensure_open_handle());
                let core_cache_helper = SegmentReaderCoreCacheHelper::new(
                    Arc::clone(&core_readers),
                    core.ensure_open_handle(),
                );

                Ok(Self {
                    core,
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
        Self::new_shared_bits(
            segment_info,
            parent,
            live_docs.map(SharedBits::new),
            hard_live_docs.map(SharedBits::new),
            num_docs,
            is_nrt,
        )
    }

    /// [`new_shared`](Self::new_shared) over bits handed in as shared handles,
    /// so that a caller passing the same `Bits` object twice — as Java's
    /// `StandardDirectoryReader` does for a reader that is not near-real-time
    /// — keeps it one object (`SegmentReader.java:151`).
    pub(crate) fn new_shared_bits(
        segment_info: SegmentCommitInfo,
        parent: &SegmentReader,
        live_docs: Option<SharedBits>,
        hard_live_docs: Option<SharedBits>,
        num_docs: i32,
        is_nrt: bool,
    ) -> Result<Self> {
        let max_doc = segment_info.info.max_doc()?;
        if num_docs > max_doc {
            return Err(LuceneError::illegal_argument(format!(
                "numDocs={num_docs} but maxDoc={max_doc}"
            )));
        }
        if let Some(ref bits) = live_docs {
            if bits.length() != max_doc as usize {
                return Err(LuceneError::illegal_argument(format!(
                    "maxDoc={max_doc} but liveDocs.size()={}",
                    bits.length()
                )));
            }
        }

        let si = segment_info.java_clone(); // `this.si = si.clone()` (`SegmentReader.java:146`)
        let original_si = segment_info;
        // `SegmentReader.java:151`.
        debug_assert!(Self::assert_live_docs(
            is_nrt,
            hard_live_docs.as_ref(),
            live_docs.as_ref()
        ));

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
                let cfs_dir_guard = core_readers
                    .cfs_reader
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
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

                // The two helpers are anonymous inner classes in Java, whose
                // `addClosedListener` calls this reader's `ensureOpen()`
                // (`SegmentReader.java:339-343, 363-367`); they reach the
                // reader's state through a handle on its core.
                let core = IndexReaderCore::new();
                let reader_cache_helper = SegmentReaderCacheHelper::new(core.ensure_open_handle());
                let core_cache_helper = SegmentReaderCoreCacheHelper::new(
                    Arc::clone(&core_readers),
                    core.ensure_open_handle(),
                );

                Ok(Self {
                    core,
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
                .ok_or_else(crate::error::LuceneError::null_pointer)?;
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

    /// Returns whether this reader was opened over an in-memory segment that
    /// the writer may still be changing.
    ///
    /// Equivalent to reading `SegmentReader.isNRT`, which
    /// `StandardDirectoryReader.createOrReuseSegmentReader` consults to decide
    /// whether the live docs must be re-read from disk.
    pub fn is_nrt(&self) -> bool {
        self.is_nrt
    }

    /// The live docs as the shared handle this reader holds, Java's
    /// `liveDocs` object itself.
    pub(crate) fn live_docs_shared(&self) -> Option<SharedBits> {
        self.live_docs.clone()
    }

    /// The hard live docs as the shared handle this reader holds, Java's
    /// `hardLiveDocs` object itself.
    pub(crate) fn hard_live_docs_shared(&self) -> Option<SharedBits> {
        self.hard_live_docs.clone()
    }

    /// Returns the live docs that are not hard-deleted, if any.
    pub fn get_hard_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.hard_live_docs
            .as_ref()
            .map(|bits| Box::new(bits.clone()) as Box<dyn Bits>)
    }

    /// Returns the underlying postings reader.
    pub fn get_postings_reader(&self) -> Result<Option<Arc<dyn FieldsProducer>>> {
        self.ensure_open()?;
        let guard = self
            .core_readers
            .fields
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(guard.as_ref().map(Arc::clone))
    }

    /// Returns the underlying stored-fields reader.
    pub fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        self.ensure_open()?;
        let guard = self
            .core_readers
            .fields_reader_orig
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .as_ref()
            .map(|reader| reader.clone_reader())
            .transpose()
    }

    /// Returns the underlying term-vectors reader.
    pub fn get_term_vectors_reader(&self) -> Result<Option<Box<dyn TermVectorsReader>>> {
        self.ensure_open()?;
        let guard = self
            .core_readers
            .term_vectors_reader_orig
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .as_ref()
            .map(|reader| reader.clone_reader())
            .transpose()
    }

    /// Returns the underlying norms producer.
    pub fn get_norms_reader(&self) -> Result<Option<Arc<dyn NormsProducer>>> {
        self.ensure_open()?;
        let guard = self
            .core_readers
            .norms_producer
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(guard.as_ref().map(Arc::clone))
    }

    /// Returns the underlying doc-values producer.
    pub fn get_doc_values_reader(&self) -> Result<Option<Arc<dyn DocValuesProducer>>> {
        self.ensure_open()?;
        Ok(self.doc_values_producer.as_ref().map(Arc::clone))
    }

    /// Returns the underlying points reader.
    pub fn get_points_reader(&self) -> Result<Option<Arc<dyn PointsReader>>> {
        self.ensure_open()?;
        let guard = self
            .core_readers
            .points_reader
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(guard.as_ref().map(Arc::clone))
    }

    /// Returns the underlying KNN vectors reader.
    ///
    /// Equivalent to `SegmentReader.getVectorReader()` (`SegmentReader.java:282-285`),
    /// the one producer accessor of the class that does not call
    /// `ensureOpen()`.
    pub fn get_vector_reader(&self) -> Result<Option<Arc<dyn KnnVectorsReader>>> {
        let guard = self
            .core_readers
            .knn_vectors_reader
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(guard.as_ref().map(Arc::clone))
    }
}

impl LeafReader for SegmentReader {
    /// This reader is a `CodecReader`: Java's `instanceof CodecReader` on it
    /// is `true`.
    fn as_codec_reader(
        self: Arc<Self>,
    ) -> Option<Arc<dyn crate::index::codec_reader::CodecReader>> {
        Some(self)
    }

    fn as_segment_reader(&self) -> Option<&SegmentReader> {
        Some(self)
    }

    fn core(&self) -> &IndexReaderCore {
        &self.core
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.ensure_open()?;
        let guard = self
            .core_readers
            .term_vectors_reader_orig
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(reader) = guard.as_ref() {
            // `TermVectorsReader` extends `TermVectors`, so the codec reader
            // itself is the caller's `TermVectors` (`CodecReader.java:107-114`).
            Ok(reader.clone_reader()?)
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

    /// `CodecReader.storedFields()`, `final` (`CodecReader.java:86-104`):
    /// a view over the clone [`get_fields_reader`](SegmentReader::get_fields_reader)
    /// answers, which checks the document bounds before it dispatches.
    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        crate::index::codec_reader::final_members::stored_fields(self)
    }

    fn do_close(&self) -> Result<()> {
        let core_result = self.core_readers.dec_ref();
        let dv_result = if let Some(ref gens) = self.doc_values_gens {
            self.seg_doc_values.dec_ref(gens)
        } else {
            Ok(())
        };
        core_result?;
        dv_result
    }

    /// Tells everything registered on this reader's own cache helper that the
    /// reader has closed.
    ///
    /// Equivalent to `SegmentReader.notifyReaderClosedListeners()`
    /// (`SegmentReader.java:323-328`), which `IndexReader.decRef` runs *after*
    /// `doClose()` and before the parent readers are told — which is why it is
    /// an override here rather than the tail of
    /// [`do_close`](Self::do_close): a failure inside `doClose()` must not stop
    /// the listeners from running.
    fn notify_reader_closed_listeners(&self) -> Result<()> {
        self.reader_cache_helper.notify_closed_listeners()
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
        let guard = self
            .core_readers
            .fields
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            producer.get_numeric(fi.unwrap())
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
            producer.get_binary(fi.unwrap())
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
            producer.get_sorted(fi.unwrap())
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
            producer.get_sorted_numeric(fi.unwrap())
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
            producer.get_sorted_set(fi.unwrap())
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
        let guard = self
            .core_readers
            .norms_producer
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(producer) = guard.as_ref() {
            producer.get_norms(fi.unwrap())
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
            producer.get_skipper(fi.unwrap())
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
        let guard = self
            .core_readers
            .knn_vectors_reader
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(reader) = guard.as_ref() {
            reader.get_float_vector_values(field)
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
        let guard = self
            .core_readers
            .knn_vectors_reader
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(reader) = guard.as_ref() {
            reader.get_byte_vector_values(field)
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
        let mut guard = self
            .core_readers
            .knn_vectors_reader
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let mut guard = self
            .core_readers
            .knn_vectors_reader
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(reader) = guard.as_mut() {
            reader.search_byte(field, target, collector, accept_docs)
        } else {
            Ok(())
        }
    }

    /// Equivalent to `SegmentReader.getFieldInfos()` (`SegmentReader.java:233-237`),
    /// which calls `ensureOpen()` first.
    ///
    /// # Panics
    ///
    /// The unchecked `AlreadyClosedException` once the reader is closed.
    fn get_field_infos(&self) -> FieldInfos {
        self.core.ensure_open_unchecked();
        self.field_infos.clone()
    }

    /// Equivalent to `SegmentReader.getLiveDocs()` (`SegmentReader.java:213-217`),
    /// which calls `ensureOpen()` first.
    ///
    /// # Panics
    ///
    /// The unchecked `AlreadyClosedException` once the reader is closed.
    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.core.ensure_open_unchecked();
        self.live_docs
            .as_ref()
            .map(|bits| Box::new(bits.clone()) as Box<dyn Bits>)
    }

    /// Equivalent to `CodecReader.getPointValues(String)`
    /// (`CodecReader.java:221-231`), which calls `ensureOpen()` before it
    /// looks at the field.
    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn IndexPointValues>>> {
        self.ensure_open()?;
        // A field has point values iff its FieldInfo declares a non-zero
        // `point_dimension_count` (set when the field is first indexed with a
        // point value). Fields without points return `None`, matching Lucene's
        // `SegmentReader.getPointValues` contract.
        let Some(field_info) = self.field_infos.field_info(field) else {
            return Ok(None);
        };
        if field_info.get_point_dimension_count() == 0 {
            return Ok(None);
        }
        let Some(guard) = self.get_points_reader()? else {
            return Ok(None);
        };
        // The codec layer re-exports the index-layer `PointValues`, so the
        // reader's answer — `None` for a field with no tree in this segment,
        // Java's `null` — is directly this method's. The returned box is owned
        // and independent of the reader's lock.
        guard.get_values(field)
    }

    /// Equivalent to `SegmentReader.checkIntegrity()`
    /// (`SegmentReader.java:399-405`): `CodecReader.checkIntegrity()` through
    /// this reader's own accessors, in Java's order, then the compound file.
    fn check_integrity(&self) -> Result<()> {
        crate::index::codec_reader::final_members::check_integrity(self)?;
        {
            let guard = self
                .core_readers
                .cfs_reader
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    /// Checks the relationship the two live-docs views must have, and always
    /// returns `true`.
    ///
    /// A near-real-time reader may hold in-memory live docs on top of the
    /// on-disk ones, so `live_docs` must be present whenever `hard_live_docs`
    /// is; a reader that is not near-real-time has nothing on top, so the two
    /// are the very same bits.
    ///
    /// Equivalent to the private static
    /// `SegmentReader.assertLiveDocs(boolean, Bits, Bits)`, which Java only ever
    /// calls from inside an `assert`; this port calls it from inside a
    /// `debug_assert!` for the same reason.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's non-NRT branch asserts
    /// `hardLiveDocs == liveDocs`, a *reference* comparison — its constructors
    /// are handed the same `Bits` object twice. This port's [`SharedBits`]
    /// names that sharing with an `Arc`, so the identity test is
    /// [`Arc::ptr_eq`] over the two handles; a caller handing the same bits
    /// twice passes the same handle ([`SegmentReader::new_shared_bits`]).
    fn assert_live_docs(
        is_nrt: bool,
        hard_live_docs: Option<&SharedBits>,
        live_docs: Option<&SharedBits>,
    ) -> bool {
        if is_nrt {
            debug_assert!(
                hard_live_docs.is_none() || live_docs.is_some(),
                "liveDocs must be non null if hardLiveDocs are non null"
            );
        } else {
            debug_assert!(
                match (hard_live_docs, live_docs) {
                    (None, None) => true,
                    // Java's `hardLiveDocs == liveDocs`: the identity of the
                    // shared bits, which a reopen hands in through two
                    // handles (`SegmentReader.java:170-176`).
                    (Some(hard), Some(live)) => hard.identity() == live.identity(),
                    _ => false,
                },
                "non-nrt case must have identical liveDocs"
            );
        }
        true
    }

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

/// `Bits` implementation that shares an underlying `Arc<dyn Bits>`.
#[derive(Clone)]
/// A [`Bits`] one reader can hand to several owners.
///
/// Java shares the `liveDocs` object by reference — `SegmentReader`'s
/// reopen constructor is given the same instance as both `liveDocs` and
/// `hardLiveDocs`. Rust needs the sharing named, so the bits go behind an
/// `Arc` and each owner takes a clone of the handle rather than of the data.
pub(crate) struct SharedBits(Arc<dyn Bits>);

impl SharedBits {
    pub(crate) fn new(bits: Box<dyn Bits>) -> Self {
        Self(Arc::from(bits))
    }

    /// A handle onto bits that are already shared.
    pub(crate) fn from_arc(bits: Arc<dyn Bits>) -> Self {
        Self(bits)
    }

    /// The shared bits themselves: one object for every handle cloned from
    /// this one, as Java's `liveDocs` reference is.
    pub(crate) fn shared(&self) -> &Arc<dyn Bits> {
        &self.0
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

    /// Forwards to the wrapped bits' `try_get`, so that an error they raise
    /// (`ExitableDirectoryReader.java:359-387`, whose `get` throws through
    /// every wrapper that delegates to it) is not swallowed here.
    fn try_get(&self, index: usize) -> Result<bool> {
        self.0.try_get(index)
    }

    /// Forwards to the wrapped bits' `apply_mask`, so that an override — the
    /// `FixedBitSet`'s word-wise one, with its own checks — runs exactly as it
    /// would on the bits themselves.
    fn apply_mask(&self, bit_set: &mut crate::util::FixedBitSet, offset: usize) -> Result<()> {
        self.0.apply_mask(bit_set, offset)
    }

    fn length(&self) -> usize {
        self.0.length()
    }

    /// The identity of the shared bits, not of this handle.
    fn identity(&self) -> *const () {
        self.0.identity()
    }

    /// The handle is a Rust artefact: Java's `instanceof` sees the shared
    /// bits themselves.
    fn bits_as_any(&self) -> Option<&dyn std::any::Any> {
        self.0.bits_as_any()
    }

    /// Java's `liveDocs instanceof LiveDocs` sees the shared bits themselves:
    /// the `SparseLiveDocs` or `DenseLiveDocs` `Lucene90LiveDocsFormat`
    /// reads (`Lucene90LiveDocsFormat.java:108-128`) are [`LiveDocs`], so the
    /// handle answers as a [`SharedLiveDocs`] over them.
    fn into_live_docs(self: Box<Self>) -> Option<Box<dyn LiveDocs>> {
        SharedLiveDocs::live_docs_of(self.0.as_ref())?;
        Some(Box::new(SharedLiveDocs(self.0)))
    }
}

/// A [`SharedBits`] handle whose shared bits are [`LiveDocs`] (the
/// `SparseLiveDocs` or `DenseLiveDocs` of a segment), answering as them.
///
/// Rust cannot narrow the shared `Arc<dyn Bits>` to a `dyn LiveDocs`, so each
/// call reaches the concrete live docs through [`Bits::bits_as_any`].
struct SharedLiveDocs(Arc<dyn Bits>);

impl SharedLiveDocs {
    /// The live docs `bits` are, when they are one of the two `LiveDocs`
    /// implementations.
    fn live_docs_of(bits: &dyn Bits) -> Option<&dyn LiveDocs> {
        let any = bits.bits_as_any()?;
        if let Some(sparse) = any.downcast_ref::<SparseLiveDocs>() {
            return Some(sparse);
        }
        any.downcast_ref::<DenseLiveDocs>()
            .map(|dense| dense as &dyn LiveDocs)
    }

    /// The shared live docs; [`SharedBits::into_live_docs`] builds this
    /// handle only over bits [`live_docs_of`](Self::live_docs_of) accepts.
    fn live_docs(&self) -> &dyn LiveDocs {
        Self::live_docs_of(self.0.as_ref())
            .expect("a SharedLiveDocs is only built over SparseLiveDocs or DenseLiveDocs")
    }
}

impl Debug for SharedLiveDocs {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedLiveDocs")
            .field("length", &self.0.length())
            .finish()
    }
}

impl Bits for SharedLiveDocs {
    fn get(&self, index: usize) -> bool {
        self.0.get(index)
    }

    fn try_get(&self, index: usize) -> Result<bool> {
        self.0.try_get(index)
    }

    fn apply_mask(&self, bit_set: &mut crate::util::FixedBitSet, offset: usize) -> Result<()> {
        self.0.apply_mask(bit_set, offset)
    }

    fn length(&self) -> usize {
        self.0.length()
    }

    fn into_live_docs(self: Box<Self>) -> Option<Box<dyn LiveDocs>> {
        Some(self)
    }

    fn identity(&self) -> *const () {
        self.0.identity()
    }

    fn bits_as_any(&self) -> Option<&dyn std::any::Any> {
        self.0.bits_as_any()
    }
}

impl LiveDocs for SharedLiveDocs {
    fn live_docs_iterator(&self) -> Result<Box<dyn crate::search::DocIdSetIterator>> {
        self.live_docs().live_docs_iterator()
    }

    fn deleted_docs_iterator(&self) -> Result<Box<dyn crate::search::DocIdSetIterator>> {
        self.live_docs().deleted_docs_iterator()
    }

    fn deleted_count(&self) -> i32 {
        self.live_docs().deleted_count()
    }
}

// -----------------------------------------------------------------------------
// Cache helpers
// -----------------------------------------------------------------------------

/// Reader-level cache helper for `SegmentReader`.
///
/// Java keeps one anonymous `CacheHelper` per reader, with one `cacheKey`
/// object (`SegmentReader.java:330-349`), so every `getReaderCacheHelper()`
/// answers the same key. [`SegmentReader::get_reader_cache_helper`] hands out
/// clones of this helper; a [`CacheKey`] clone is the same key object, so every
/// clone answers the one key.
#[derive(Clone)]
struct SegmentReaderCacheHelper {
    key: CacheKey,
    listeners: Arc<Mutex<Vec<Arc<dyn ClosedListener>>>>,
    closed: Arc<AtomicBool>,
    /// The owning reader's `ensureOpen()`.
    reader: EnsureOpen,
}

impl Debug for SegmentReaderCacheHelper {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentReaderCacheHelper")
            .field("closed", &self.closed.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl SegmentReaderCacheHelper {
    fn new(reader: EnsureOpen) -> Self {
        Self {
            key: CacheKey::new(),
            listeners: Arc::new(Mutex::new(Vec::new())),
            closed: Arc::new(AtomicBool::new(false)),
            reader,
        }
    }

    fn notify_closed_listeners(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let guard = self
            .listeners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // `IOUtils.applyToAll(readerClosedListeners, l -> l.onClose(readerCacheHelper.getKey()))`
        // (`SegmentReader.java:323-327`): every listener runs; the first error
        // is returned with the later ones suppressed into it.
        crate::util::IOUtils::apply_to_all(guard.iter().map(Some).collect(), |listener| {
            listener.on_close(self.key.clone())
        })
    }
}

impl CacheHelper for SegmentReaderCacheHelper {
    fn get_key(&self) -> &CacheKey {
        &self.key
    }

    /// Equivalent to the anonymous helper's `addClosedListener`
    /// (`SegmentReader.java:338-343`): `ensureOpen()` on the reader, then the
    /// listener is recorded.
    ///
    /// # Panics
    ///
    /// The unchecked `AlreadyClosedException` once the reader is closed.
    fn add_closed_listener(&self, listener: Arc<dyn ClosedListener>) {
        self.reader.ensure_open_unchecked();
        {
            let mut listeners = self
                .listeners
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            add_closed_listener_to_set(&mut listeners, listener);
        }
    }
}

/// Wrapper around the core cache helper used by `SegmentReader`.
///
/// Equivalent to `SegmentReader.coreCacheHelper` (`SegmentReader.java:350-369`),
/// which wraps the core's helper "to add ensureOpen() calls that make sure
/// users do not register closed listeners on closed indices".
#[derive(Clone, Debug)]
struct SegmentReaderCoreCacheHelper {
    core: Arc<SegmentCoreReaders>,
    /// The owning reader's `ensureOpen()`.
    reader: EnsureOpen,
}

impl SegmentReaderCoreCacheHelper {
    fn new(core: Arc<SegmentCoreReaders>, reader: EnsureOpen) -> Self {
        Self { core, reader }
    }
}

impl CacheHelper for SegmentReaderCoreCacheHelper {
    fn get_key(&self) -> &CacheKey {
        self.core.get_key()
    }

    /// `ensureOpen()` on the reader, then the core helper's
    /// `addClosedListener` (`SegmentReader.java:362-367`).
    ///
    /// # Panics
    ///
    /// The unchecked `AlreadyClosedException` once the reader is closed.
    fn add_closed_listener(&self, listener: Arc<dyn ClosedListener>) {
        self.reader.ensure_open_unchecked();
        self.core.add_closed_listener(listener);
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

// -----------------------------------------------------------------------------
// TestSecrets registration
// -----------------------------------------------------------------------------

/// The [`SegmentReaderAccess`](crate::internal::tests_hooks::SegmentReaderAccess)
/// `SegmentReader` registers with
/// [`TestSecrets`](crate::internal::tests_hooks::TestSecrets).
///
/// Equivalent to the anonymous `SegmentReaderAccess` of `SegmentReader`'s
/// static initializer (`SegmentReader.java:408-414`), which returns the
/// reader's `core`.
#[derive(Debug)]
struct SegmentReaderAccessImpl;

impl crate::internal::tests_hooks::SegmentReaderAccess for SegmentReaderAccessImpl {
    fn get_core(&self, segment_reader: &SegmentReader) -> Arc<dyn std::any::Any + Send + Sync> {
        Arc::clone(segment_reader.core_readers()) as Arc<dyn std::any::Any + Send + Sync>
    }
}

/// Registers `SegmentReader`'s accessor with
/// [`TestSecrets`](crate::internal::tests_hooks::TestSecrets), once.
///
/// Equivalent to `SegmentReader`'s `static {}` block
/// (`SegmentReader.java:407-415`), which
/// `TestSecrets.ensureInitialized(SegmentReader.class)` triggers.
pub(crate) fn register_test_secrets() {
    static INITIALIZED: std::sync::Once = std::sync::Once::new();
    INITIALIZED.call_once(|| {
        crate::internal::tests_hooks::TestSecrets::set_segment_reader_access(Some(Arc::new(
            SegmentReaderAccessImpl,
        )))
        .expect("INVARIANT: SegmentReader's static initializer runs once, from this file");
    });
}

#[cfg(test)]
mod shared_bits_tests {
    use super::*;
    use crate::error::LuceneErrorKind;
    use crate::util::FixedBitSet;

    /// A segment's live docs answer `applyMask` with `FixedBitSet.applyMask`,
    /// which refuses a mask with bits past the live docs
    /// (`FixedBitSet.java:898-908`): `Some bits are set beyond the end of live
    /// docs`. The shared handle the reader hands out must answer the same.
    #[test]
    fn shared_bits_apply_the_fixed_bit_set_mask() {
        let mut live = FixedBitSet::new(10);
        live.set_range(0, 10);
        let shared = SharedBits::new(Box::new(live));
        let mut mask = FixedBitSet::new(64);
        mask.set(3);
        mask.set(20);
        assert!(matches!(
            shared.apply_mask(&mut mask, 0),
            Err(LuceneError { kind: LuceneErrorKind::IllegalArgument { ref message, .. }, .. })
                if message == "Some bits are set beyond the end of live docs"
        ));
    }
}

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
    use crate::codecs::stub::StoredFieldVisitor;
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
    use std::collections::HashSet;

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
        // `getSegmentInfo()` returns `si`, the reader's `si.clone()`
        // (`SegmentReader.java:72`): a copy, not the caller's object.
        assert!(!reader.get_segment_info().same_object(&info));
        assert_eq!(reader.get_segment_info().info.name, info.info.name);
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

    /// rmp #261: the core cache key `LRUQueryCache` keys its leaves on is the
    /// core's own object, distinct from every other live core's key, and the
    /// core cache helper answers that same key — the identity Java's
    /// `coreCacheKey` object has. Two reader cache helpers of one reader also
    /// answer one key (`SegmentReader.java:330-349`).
    #[test]
    fn core_cache_keys_are_distinct_objects() {
        let readers: Vec<SegmentReader> = (0..4)
            .map(|i| {
                SegmentReader::new(
                    test_segment_commit_info(&format!("_{i}"), 5),
                    Version::LATEST.major as i32,
                    &*DEFAULT_IO_CONTEXT,
                )
                .unwrap()
            })
            .collect();
        let mut keys = std::collections::HashSet::new();
        for reader in &readers {
            let core = reader.core_readers();
            let helper = reader.get_core_cache_helper().unwrap();
            assert_eq!(helper.get_key(), core.get_key());
            let first = LeafReader::get_reader_cache_helper(reader).unwrap();
            let second = LeafReader::get_reader_cache_helper(reader).unwrap();
            assert_eq!(first.get_key(), second.get_key());
            assert_ne!(first.get_key(), core.get_key());
            keys.insert(core.get_key().clone());
        }
        assert_eq!(keys.len(), readers.len());
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
        let field_infos = FieldInfos::empty();
        let producer: Arc<dyn DocValuesProducer> =
            Arc::new(crate::codecs::doc_values::EmptyDocValuesProducer);
        let reader = DocValuesLeafReader::new(field_infos, producer);
        assert!(reader.get_numeric_doc_values("missing").unwrap().is_none());
        // `maxDoc()` and `numDocs()` are final and throw
        // `UnsupportedOperationException` (`DocValuesLeafReader.java`).
        let max_doc = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            LeafReader::max_doc(&reader)
        }));
        assert!(max_doc.is_err());
        assert!(reader.get_norm_values("missing").is_err());
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
        /// Whether this wrapper came out of `get_merge_instance`.
        merge_instance: bool,
    }

    impl Debug for RecordingFieldsProducer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("RecordingFieldsProducer")
                .field("closed", &self.closed.load(Ordering::SeqCst))
                .field("merge_instance", &self.merge_instance)
                .finish_non_exhaustive()
        }
    }

    impl RecordingFieldsProducer {
        fn new(inner: Box<dyn FieldsProducer>, closed: Arc<AtomicBool>) -> Self {
            Self {
                inner,
                closed,
                merge_instance: false,
            }
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
            Ok(Box::new(RecordingFieldsProducer {
                inner: self.inner.get_merge_instance()?,
                closed: self.closed.clone(),
                merge_instance: true,
            }))
        }

        fn close(&self) -> Result<()> {
            self.closed.store(true, Ordering::SeqCst);
            self.inner.close()
        }
    }

    #[derive(Debug)]
    struct RecordingNormsProducer {
        inner: Box<dyn NormsProducer>,
        closed: Arc<AtomicBool>,
        /// Whether this wrapper came out of `get_merge_instance`. Tests read it
        /// through `Debug`, which dead-code analysis does not count.
        #[allow(dead_code)]
        merge_instance: bool,
    }

    impl RecordingNormsProducer {
        fn new(inner: Box<dyn NormsProducer>, closed: Arc<AtomicBool>) -> Self {
            Self {
                inner,
                closed,
                merge_instance: false,
            }
        }
    }

    impl NormsProducer for RecordingNormsProducer {
        fn get_norms(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
            self.inner.get_norms(field)
        }

        fn check_integrity(&self) -> Result<()> {
            self.inner.check_integrity()
        }

        fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
            Ok(Box::new(RecordingNormsProducer {
                inner: self.inner.get_merge_instance()?,
                closed: self.closed.clone(),
                merge_instance: true,
            }))
        }

        fn close(&self) -> Result<()> {
            self.closed.store(true, Ordering::SeqCst);
            self.inner.close()
        }
    }

    #[derive(Debug)]
    struct RecordingStoredFieldsReader {
        inner: Box<dyn StoredFieldsReader>,
        closed: Arc<AtomicBool>,
        /// Whether this wrapper came out of `get_merge_instance`. Tests read it
        /// through `Debug`, which dead-code analysis does not count.
        #[allow(dead_code)]
        merge_instance: bool,
    }

    impl RecordingStoredFieldsReader {
        fn new(inner: Box<dyn StoredFieldsReader>, closed: Arc<AtomicBool>) -> Self {
            Self {
                inner,
                closed,
                merge_instance: false,
            }
        }
    }

    impl StoredFieldsReader for RecordingStoredFieldsReader {
        fn document(&self, doc_id: i32, visitor: &mut dyn StoredFieldVisitor) -> Result<()> {
            self.inner.document(doc_id, visitor)
        }

        fn check_integrity(&self) -> Result<()> {
            self.inner.check_integrity()
        }

        fn clone_reader(&self) -> Result<Box<dyn StoredFieldsReader>> {
            Ok(Box::new(RecordingStoredFieldsReader::new(
                self.inner.clone_reader()?,
                self.closed.clone(),
            )))
        }

        fn get_merge_instance(&self) -> Result<Box<dyn StoredFieldsReader>> {
            Ok(Box::new(RecordingStoredFieldsReader {
                inner: self.inner.get_merge_instance()?,
                closed: self.closed.clone(),
                merge_instance: true,
            }))
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
        /// Whether this wrapper came out of `get_merge_instance`. Tests read it
        /// through `Debug`, which dead-code analysis does not count.
        #[allow(dead_code)]
        merge_instance: bool,
    }

    impl RecordingTermVectorsReader {
        fn new(inner: Box<dyn TermVectorsReader>, closed: Arc<AtomicBool>) -> Self {
            Self {
                inner,
                closed,
                merge_instance: false,
            }
        }
    }

    impl TermVectorsReader for RecordingTermVectorsReader {
        fn check_integrity(&self) -> Result<()> {
            self.inner.check_integrity()
        }

        fn clone_reader(&self) -> Result<Box<dyn TermVectorsReader>> {
            Ok(Box::new(RecordingTermVectorsReader::new(
                self.inner.clone_reader()?,
                self.closed.clone(),
            )))
        }

        fn get_merge_instance(&self) -> Result<Box<dyn TermVectorsReader>> {
            Ok(Box::new(RecordingTermVectorsReader {
                inner: self.inner.get_merge_instance()?,
                closed: self.closed.clone(),
                merge_instance: true,
            }))
        }

        fn close(&mut self) -> Result<()> {
            self.closed.store(true, Ordering::SeqCst);
            self.inner.close()
        }
    }

    impl TermVectors for RecordingTermVectorsReader {
        fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
            self.inner.get(doc)
        }

        fn prefetch(&mut self, doc_id: i32) -> Result<()> {
            self.inner.prefetch(doc_id)
        }
    }

    #[derive(Debug)]
    struct RecordingPointsReader {
        inner: Box<dyn PointsReader>,
        closed: Arc<AtomicBool>,
        /// Whether this wrapper came out of `get_merge_instance`. Tests read it
        /// through `Debug`, which dead-code analysis does not count.
        #[allow(dead_code)]
        merge_instance: bool,
    }

    impl RecordingPointsReader {
        fn new(inner: Box<dyn PointsReader>, closed: Arc<AtomicBool>) -> Self {
            Self {
                inner,
                closed,
                merge_instance: false,
            }
        }
    }

    impl PointsReader for RecordingPointsReader {
        fn check_integrity(&self) -> Result<()> {
            self.inner.check_integrity()
        }

        fn get_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
            self.inner.get_values(field)
        }

        fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>> {
            Ok(Box::new(RecordingPointsReader {
                inner: self.inner.get_merge_instance()?,
                closed: self.closed.clone(),
                merge_instance: true,
            }))
        }

        fn close(&self) -> Result<()> {
            self.closed.store(true, Ordering::SeqCst);
            self.inner.close()
        }
    }

    #[derive(Debug)]
    struct RecordingKnnVectorsReader {
        inner: Box<dyn KnnVectorsReader>,
        closed: Arc<AtomicBool>,
        /// Whether this wrapper came out of `get_merge_instance`. Tests read it
        /// through `Debug`, which dead-code analysis does not count.
        #[allow(dead_code)]
        merge_instance: bool,
    }

    impl RecordingKnnVectorsReader {
        fn new(inner: Box<dyn KnnVectorsReader>, closed: Arc<AtomicBool>) -> Self {
            Self {
                inner,
                closed,
                merge_instance: false,
            }
        }
    }

    impl KnnVectorsReader for RecordingKnnVectorsReader {
        fn check_integrity(&self) -> Result<()> {
            self.inner.check_integrity()
        }

        fn get_float_vector_values(
            &self,
            field: &str,
        ) -> Result<Option<Box<dyn FloatVectorValues>>> {
            self.inner.get_float_vector_values(field)
        }

        fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
            self.inner.get_byte_vector_values(field)
        }

        fn search(
            &self,
            field: &str,
            target: &[f32],
            knn_collector: &mut dyn KnnCollector,
            accept_docs: &mut dyn AcceptDocs,
        ) -> Result<()> {
            self.inner.search(field, target, knn_collector, accept_docs)
        }

        fn search_byte(
            &self,
            field: &str,
            target: &[u8],
            knn_collector: &mut dyn KnnCollector,
            accept_docs: &mut dyn AcceptDocs,
        ) -> Result<()> {
            self.inner
                .search_byte(field, target, knn_collector, accept_docs)
        }

        fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
            Ok(Box::new(RecordingKnnVectorsReader {
                inner: self.inner.get_merge_instance()?,
                closed: self.closed.clone(),
                merge_instance: true,
            }))
        }

        fn finish_merge(&mut self) -> Result<()> {
            self.inner.finish_merge()
        }

        fn get_off_heap_byte_size(
            &self,
            field_info: &FieldInfo,
        ) -> crate::error::Result<HashMap<String, i64>> {
            self.inner.get_off_heap_byte_size(field_info)
        }

        fn close(&self) -> Result<()> {
            self.closed.store(true, Ordering::SeqCst);
            self.inner.close()
        }
    }

    #[derive(Debug)]
    struct RecordingDocValuesProducer {
        inner: Box<dyn DocValuesProducer>,
        closed: Arc<AtomicBool>,
        /// Whether this wrapper came out of `get_merge_instance`. Tests read it
        /// through `Debug`, which dead-code analysis does not count.
        #[allow(dead_code)]
        merge_instance: bool,
    }

    impl RecordingDocValuesProducer {
        fn new(inner: Box<dyn DocValuesProducer>, closed: Arc<AtomicBool>) -> Self {
            Self {
                inner,
                closed,
                merge_instance: false,
            }
        }
    }

    impl DocValuesProducer for RecordingDocValuesProducer {
        fn get_numeric(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
            self.inner.get_numeric(field)
        }

        fn get_binary(&self, field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
            self.inner.get_binary(field)
        }

        fn get_sorted(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
            self.inner.get_sorted(field)
        }

        fn get_sorted_numeric(
            &self,
            field: &FieldInfo,
        ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
            self.inner.get_sorted_numeric(field)
        }

        fn get_sorted_set(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
            self.inner.get_sorted_set(field)
        }

        fn get_skipper(&self, field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
            self.inner.get_skipper(field)
        }

        fn check_integrity(&self) -> Result<()> {
            self.inner.check_integrity()
        }

        fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
            Ok(Box::new(RecordingDocValuesProducer {
                inner: self.inner.get_merge_instance()?,
                closed: self.closed.clone(),
                merge_instance: true,
            }))
        }

        /// Records the close. The wrapped dummy producer is Java's
        /// `EmptyDocValuesProducer`, whose `close()` throws
        /// `UnsupportedOperationException`; `SegmentDocValues` propagates a
        /// close error as Java's `RefCount.decRef` does, and a real producer
        /// closes cleanly, so the dummy is not closed.
        fn close(&self) -> Result<()> {
            self.closed.store(true, Ordering::SeqCst);
            Ok(())
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

        fn close(&self) -> Result<()> {
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
            directory: &std::sync::Arc<dyn Directory>,
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
            directory: &std::sync::Arc<dyn Directory>,
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

        fn fields_writer(
            &self,
            state: &crate::codecs::state::OwnedSegmentWriteState,
        ) -> Result<Box<dyn KnnVectorsWriter>> {
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
                crate::util::java_collections::JavaStringMap::default(),
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
            None,
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

    /// `MergeState` holds the merge instance of every reader it pulls off a
    /// segment, never the instance a search shares.
    ///
    /// Lucene's `MergeState` constructor replaces each non-null producer with
    /// its `getMergeInstance()` (`MergeState.java:134-168`): norms, doc values,
    /// stored fields, term vectors, postings, points and vectors alike. Every
    /// producer of the recording codec marks the wrapper its
    /// `get_merge_instance` returns, so each of the seven slots must hold a
    /// marked one while the segment reader keeps handing out unmarked ones.
    #[test]
    fn merge_state_holds_the_merge_instance_of_every_reader() {
        use crate::index::codec_reader::CodecReader;
        use crate::index::merge::MergeState;

        let recorder = CloseRecorder::default();
        let info = recording_segment_commit_info("_0", 5, &recorder);
        let reader = Arc::new(
            SegmentReader::new(info, Version::LATEST.major as i32, &*DEFAULT_IO_CONTEXT)
                .expect("segment reader should open"),
        );
        let readers: Vec<Arc<dyn CodecReader>> = vec![reader.clone()];
        let merge_state = MergeState::from_readers(
            &readers,
            test_segment_info("_1", -1),
            Arc::new(crate::util::NoOutputInfoStream),
            None,
            None,
        )
        .expect("merge state");

        let merge_instance = |debug: String| debug.contains("merge_instance: true");
        let slots = [
            (
                "norms",
                format!("{:?}", merge_state.norms_producers[0].as_ref().unwrap()),
                format!("{:?}", reader.get_norms_reader().unwrap().unwrap()),
            ),
            (
                "doc values",
                format!(
                    "{:?}",
                    merge_state.doc_values_producers[0].as_ref().unwrap()
                ),
                format!("{:?}", reader.get_doc_values_reader().unwrap().unwrap()),
            ),
            (
                "stored fields",
                format!(
                    "{:?}",
                    merge_state.stored_fields_readers[0].as_ref().unwrap()
                ),
                format!("{:?}", reader.get_fields_reader().unwrap().unwrap()),
            ),
            (
                "term vectors",
                format!(
                    "{:?}",
                    merge_state.term_vectors_readers[0].as_ref().unwrap()
                ),
                format!("{:?}", reader.get_term_vectors_reader().unwrap().unwrap()),
            ),
            (
                "points",
                format!("{:?}", merge_state.points_readers[0].as_ref().unwrap()),
                format!("{:?}", reader.get_points_reader().unwrap().unwrap()),
            ),
            (
                "vectors",
                format!("{:?}", merge_state.knn_vectors_readers[0].as_ref().unwrap()),
                format!("{:?}", reader.get_vector_reader().unwrap().unwrap()),
            ),
        ];
        for (kind, in_merge_state, from_reader) in slots {
            assert!(
                merge_instance(in_merge_state.clone()),
                "the merge state must hold the {kind} merge instance, got {in_merge_state}"
            );
            assert!(
                !merge_instance(from_reader.clone()),
                "the segment reader must still hand out the {kind} search instance, got {from_reader}"
            );
        }

        // `FieldsProducer` carries no `Debug`, so the postings slot is told
        // apart by identity: the segment reader hands out one shared producer,
        // and the merge state must not hold that one.
        let search_postings = reader.get_postings_reader().unwrap().unwrap();
        assert!(Arc::ptr_eq(
            &search_postings,
            &reader.get_postings_reader().unwrap().unwrap()
        ));
        assert!(
            !Arc::ptr_eq(
                &search_postings,
                merge_state.fields_producers[0].as_ref().unwrap()
            ),
            "the merge state must hold the postings merge instance, not the search one"
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
            let i = test_segment_commit_info("_0", max_doc);
            i.set_del_count(del_count).unwrap();
            i
        };

        let parent = SegmentReader::new(
            test_segment_commit_info("_1", max_doc),
            Version::LATEST.major as i32,
            &*DEFAULT_IO_CONTEXT,
        )
        .unwrap();
        // Not NRT: `liveDocs` and `hardLiveDocs` are one object
        // (`SegmentReader.java:151`).
        let live_docs = SharedBits::new(Box::new(del_docs));
        let reader = SegmentReader::new_shared_bits(
            info,
            &parent,
            Some(live_docs.clone()),
            Some(live_docs),
            max_doc - del_count,
            // Distinct live-docs and hard-live-docs instances are only legal
            // for an NRT reader: the non-NRT case asserts they are the same
            // object (`SegmentReader.java:170-176`).
            true,
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

    /// A codec reader that only counts the prefetch hints it is handed.
    #[derive(Debug, Default)]
    struct CountingPrefetchReader {
        prefetched: Arc<std::sync::Mutex<Vec<i32>>>,
    }

    impl TermVectorsReader for CountingPrefetchReader {
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn clone_reader(&self) -> Result<Box<dyn TermVectorsReader>> {
            Ok(Box::new(Self::default()))
        }
    }

    impl TermVectors for CountingPrefetchReader {
        fn get(&self, _doc: i32) -> Result<Option<Box<dyn crate::index::Fields>>> {
            Ok(None)
        }

        fn prefetch(&mut self, doc_id: i32) -> Result<()> {
            self.prefetched.lock().unwrap().push(doc_id);
            Ok(())
        }
    }

    #[test]
    fn the_codec_reader_is_the_term_vectors_and_receives_the_prefetch_hint() {
        // Java's `TermVectorsReader` *extends* `TermVectors`, so `CodecReader`
        // hands the codec reader itself to the caller and a prefetch reaches
        // the format's own override (`CodecReader.java:107-114`).
        let prefetched = Arc::new(std::sync::Mutex::new(Vec::new()));
        let reader: Box<dyn TermVectorsReader> = Box::new(CountingPrefetchReader {
            prefetched: Arc::clone(&prefetched),
        });
        let mut term_vectors: Box<dyn TermVectors> = reader;
        term_vectors.prefetch(3).expect("prefetch");
        term_vectors.prefetch(11).expect("prefetch");
        assert_eq!(*prefetched.lock().unwrap(), vec![3, 11]);
    }
}

impl CodecReader for SegmentReader {
    fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        SegmentReader::get_fields_reader(self)
    }

    fn get_term_vectors_reader(&self) -> Result<Option<Box<dyn TermVectorsReader>>> {
        SegmentReader::get_term_vectors_reader(self)
    }

    fn get_norms_reader(&self) -> Result<Option<Arc<dyn NormsProducer>>> {
        SegmentReader::get_norms_reader(self)
    }

    fn get_doc_values_reader(&self) -> Result<Option<Arc<dyn DocValuesProducer>>> {
        SegmentReader::get_doc_values_reader(self)
    }

    fn get_postings_reader(&self) -> Result<Option<Arc<dyn FieldsProducer>>> {
        SegmentReader::get_postings_reader(self)
    }

    fn get_points_reader(&self) -> Result<Option<Arc<dyn PointsReader>>> {
        SegmentReader::get_points_reader(self)
    }

    fn get_vector_reader(&self) -> Result<Option<Arc<dyn KnnVectorsReader>>> {
        SegmentReader::get_vector_reader(self)
    }
}

impl std::fmt::Display for SegmentCoreReaders {
    /// Port of `SegmentCoreReaders.toString()`, which names itself
    /// `SegmentCoreReader`, in the singular.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SegmentCoreReader({})", self.segment)
    }
}

impl std::fmt::Display for SegmentDocValuesProducer {
    /// Port of `SegmentDocValuesProducer.toString()`.
    ///
    /// Java counts `dvProducers`, the list of *distinct* producers, one per
    /// doc-values generation; this port keys them by field, so the distinct
    /// producers are counted through the generation list, which has the same
    /// length.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SegmentDocValuesProducer(producers={})",
            self.dv_gens.len()
        )
    }
}

#[cfg(test)]
mod segment_doc_values_ref_count_tests {
    use super::*;
    use crate::codecs::doc_values::{
        BinaryDocValues, NumericDocValues, SortedDocValues, SortedNumericDocValues,
        SortedSetDocValues,
    };
    use std::sync::atomic::AtomicUsize;

    /// A producer that counts its closes and fails them while `fail` is set.
    #[derive(Debug)]
    struct CountingProducer {
        closes: Arc<AtomicUsize>,
        fail: Arc<AtomicBool>,
    }

    impl DocValuesProducer for CountingProducer {
        fn get_numeric(&self, _: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
            Ok(None)
        }
        fn get_binary(&self, _: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
            Ok(None)
        }
        fn get_sorted(&self, _: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
            Ok(None)
        }
        fn get_sorted_numeric(
            &self,
            _: &FieldInfo,
        ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
            Ok(None)
        }
        fn get_sorted_set(&self, _: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
            Ok(None)
        }
        fn get_skipper(&self, _: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
            Ok(None)
        }
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }
        fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
            Err(LuceneError::illegal_state("no merge instance".to_string()))
        }
        fn close(&self) -> Result<()> {
            self.closes.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                return Err(LuceneError::illegal_state("close failed".to_string()));
            }
            Ok(())
        }
    }

    /// rmp #304: `SegmentDocValues` keeps each generation in a `RefCount`
    /// whose overridden `release()` closes the producer and drops the
    /// generation from the map (`SegmentDocValues.java:53-63`); `decRef`
    /// reaches it through `RefCount.decRef`, which puts the reference back when
    /// `release()` throws (`RefCount.java:43-59`).
    #[test]
    fn the_last_dec_ref_releases_through_the_ref_count() {
        let seg_doc_values = SegmentDocValues::new();
        let closes = Arc::new(AtomicUsize::new(0));
        let fail = Arc::new(AtomicBool::new(true));
        let dvp = seg_doc_values.ref_counted(
            7,
            Box::new(CountingProducer {
                closes: Arc::clone(&closes),
                fail: Arc::clone(&fail),
            }),
        );
        SegmentDocValues::producers(&seg_doc_values.gen_dv_producers).insert(7, Arc::clone(&dvp));
        dvp.inc_ref();
        seg_doc_values.dec_ref(&[7]).unwrap();
        assert_eq!((dvp.ref_count(), closes.load(Ordering::SeqCst)), (1, 0));

        // A failing release keeps the generation and puts the reference back.
        let error = seg_doc_values.dec_ref(&[7]).unwrap_err();
        assert!(error.to_string().contains("close failed"), "{error}");
        assert_eq!((dvp.ref_count(), closes.load(Ordering::SeqCst)), (1, 1));
        assert!(SegmentDocValues::producers(&seg_doc_values.gen_dv_producers).contains_key(&7));

        // A successful one closes the producer and removes the generation.
        fail.store(false, Ordering::SeqCst);
        seg_doc_values.dec_ref(&[7]).unwrap();
        assert_eq!((dvp.ref_count(), closes.load(Ordering::SeqCst)), (0, 2));
        assert!(!SegmentDocValues::producers(&seg_doc_values.gen_dv_producers).contains_key(&7));
        // Readers still holding the producer find it closed.
        assert!(dvp.get().check_integrity().is_err());
    }
}

#[cfg(test)]
mod core_close_order_tests {
    //! `SegmentCoreReaders.decRef()` (`SegmentCoreReaders.java:168-182`):
    //! `try (Closeable finalizer = this::notifyCoreClosedListeners) {
    //! IOUtils.close(fields, ..., knnVectorsReader); }` closes the producers
    //! first and notifies the core-closed listeners after.
    use super::*;
    use crate::codecs::tests::{CloseLog, CloseTrackingDirectory};
    use crate::document::{Document, Store, TextField};
    use crate::error::LuceneErrorKind;
    use crate::index::index_reader::{CacheHelper, CacheKey, ClosedListener};
    use crate::index::index_writer::IndexWriter;
    use crate::index::index_writer_config::IndexWriterConfig;
    use crate::index::segment_infos::SegmentInfos;
    use crate::store::{Directory, DEFAULT_IO_CONTEXT};
    use crate::util::Version;

    struct CloseCountAtNotification {
        log: Arc<CloseLog>,
        seen: Arc<std::sync::Mutex<Option<usize>>>,
    }

    impl ClosedListener for CloseCountAtNotification {
        fn on_close(&self, _key: CacheKey) -> Result<()> {
            *self.seen.lock().unwrap() = Some(self.log.closed().len());
            Ok(())
        }
    }

    #[test]
    fn the_producers_close_before_the_core_closed_listeners_run() -> Result<()> {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let tracking = Arc::new(CloseTrackingDirectory::default());
        let log = Arc::clone(&tracking.log);
        let dir: Arc<dyn Directory> = tracking;
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new())?;
        let mut doc = Document::new();
        doc.add(Box::new(TextField::new(
            "body",
            "some words".to_string(),
            Store::YES,
        )?));
        writer.add_document(&doc)?;
        writer.commit()?;
        writer.close()?;

        let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir))?;
        let reader = SegmentReader::new(
            infos.info(0).clone(),
            Version::LATEST.major as i32,
            &*DEFAULT_IO_CONTEXT,
        )?;
        let seen = Arc::new(std::sync::Mutex::new(None));
        CacheHelper::add_closed_listener(
            reader.core_readers().as_ref(),
            Arc::new(CloseCountAtNotification {
                log: Arc::clone(&log),
                seen: Arc::clone(&seen),
            }),
        );
        let before = log.closed().len();
        crate::index::index_reader::IndexReader::close(&reader)?;
        let after = log.closed().len();
        assert!(after > before, "closing the core closes its inputs");
        assert_eq!(
            *seen.lock().unwrap(),
            Some(after),
            "the listeners run once every producer is closed"
        );
        Ok(())
    }

    /// The finalizer of `SegmentCoreReaders.decRef()`'s try-with-resources
    /// runs however the body ends (`SegmentCoreReaders.java:170-181`): a
    /// panic closing a producer — the port's form of an unchecked throwable —
    /// still notifies the core-closed listeners, and then propagates.
    #[test]
    fn a_panic_closing_a_producer_still_notifies_the_core_closed_listeners() -> Result<()> {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let tracking = Arc::new(CloseTrackingDirectory::default());
        let log = Arc::clone(&tracking.log);
        let dir: Arc<dyn Directory> = tracking;
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new())?;
        let mut doc = Document::new();
        doc.add(Box::new(TextField::new(
            "body",
            "some words".to_string(),
            Store::YES,
        )?));
        writer.add_document(&doc)?;
        writer.commit()?;
        writer.close()?;

        let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir))?;
        let reader = SegmentReader::new(
            infos.info(0).clone(),
            Version::LATEST.major as i32,
            &*DEFAULT_IO_CONTEXT,
        )?;
        let seen = Arc::new(std::sync::Mutex::new(None));
        CacheHelper::add_closed_listener(
            reader.core_readers().as_ref(),
            Arc::new(CloseCountAtNotification {
                log: Arc::clone(&log),
                seen: Arc::clone(&seen),
            }),
        );
        log.panic_close("_0.cfs");
        let closed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::index::index_reader::IndexReader::close(&reader)
        }));
        assert!(closed.is_err(), "the producer's panic propagates");
        assert!(log.closed().contains(&"_0.cfs".to_string()));
        assert!(
            seen.lock().unwrap().is_some(),
            "the core-closed listeners run after the panic"
        );
        Ok(())
    }
    /// The `AlreadyClosedException` message of `error`, or a panic.
    fn already_closed_message(error: LuceneError) -> String {
        match error {
            LuceneError {
                kind: LuceneErrorKind::AlreadyClosed { message, .. },
                ..
            } => message,
            other => panic!("expected an AlreadyClosedException, got {other:?}"),
        }
    }

    /// `SegmentCoreReaders.decRef()` closes its producers but keeps them in
    /// its `final` fields (`SegmentCoreReaders.java:168-182`). A
    /// `SegmentReader` whose `doClose()` ran while it stays open — what
    /// `FilterCodecReader.doClose()` does to its delegate
    /// (`FilterCodecReader.java:123-126`) — still hands the closed producers
    /// out: `getFieldsReader()` clones the closed stored-fields reader, which
    /// throws `this FieldsReader is closed`; `getTermVectorsReader()` clones the
    /// closed term-vectors reader, whose closed `.tvd` input throws
    /// `Already closed.`; `checkIntegrity()` reaches the closed postings first
    /// (`CodecReader.java:294-332`). Measured against `lucene-core-10.5.0.jar`
    /// by `IndexWrappersOracle` (`core_filter_codec wrapper_closed.*`).
    #[test]
    fn a_closed_core_keeps_its_closed_producers() -> Result<()> {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(crate::store::ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new())?;
        let mut doc = Document::new();
        let term_vectors = crate::document::FieldType::new();
        term_vectors.set_tokenized(true)?;
        term_vectors.set_index_options(crate::index::IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)?;
        term_vectors.set_store_term_vectors(true)?;
        term_vectors.set_stored(true)?;
        term_vectors.freeze();
        doc.add(Box::new(crate::document::Field::new(
            "body",
            "some words".to_string(),
            term_vectors,
        )?));
        writer.add_document(&doc)?;
        writer.commit()?;
        writer.close()?;

        let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir))?;
        let reader = SegmentReader::new(
            infos.info(0).clone(),
            Version::LATEST.major as i32,
            &*DEFAULT_IO_CONTEXT,
        )?;
        // Open, the reader answers.
        assert!(reader.get_fields_reader()?.is_some());
        assert!(reader.get_term_vectors_reader()?.is_some());
        LeafReader::check_integrity(&reader)?;

        LeafReader::do_close(&reader)?;
        assert_eq!(reader.core_readers().ref_count(), 0);
        assert_eq!(
            crate::index::index_reader::IndexReader::get_ref_count(&reader),
            1,
            "the reader itself stays open"
        );
        assert!(
            reader.get_postings_reader()?.is_some(),
            "the closed producers stay in place"
        );
        assert_eq!(
            already_closed_message(reader.get_fields_reader().unwrap_err()),
            "this FieldsReader is closed"
        );
        assert_eq!(
            already_closed_message(LeafReader::stored_fields(&reader).unwrap_err()),
            "this FieldsReader is closed"
        );
        assert_eq!(
            already_closed_message(reader.get_term_vectors_reader().unwrap_err()),
            "Already closed."
        );
        assert_eq!(
            already_closed_message(LeafReader::term_vectors(&reader).unwrap_err()),
            "Already closed."
        );
        assert_eq!(
            already_closed_message(LeafReader::check_integrity(&reader).unwrap_err()),
            "Already closed."
        );
        Ok(())
    }
}
