//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDemoParallelLeafReader.java`.
//!
//! Simple example showing how to use ParallelLeafReader to index new stuff
//! (postings, DVs, etc.) from previously stored fields, on the fly (during
//! NRT reader reopen), after the initial indexing. The test indexes just a
//! single stored field with text "content X" (X is a number embedded in the
//! text).
//!
//! Then, on reopen, for any newly created segments (flush or merge), it
//! builds a new parallel segment by loading all stored docs, parsing out
//! that X, and adding it as DV and numeric indexed (trie) field.
//!
//! Finally, for searching, it builds a top-level MultiReader, with
//! ParallelLeafReader for each segment, and then tests that random numeric
//! range queries, and sorting by the new DV field, work correctly.
//!
//! Each per-segment index lives in a private directory next to the main
//! index, and they are deleted once their segments are removed from the
//! index. They are "volatile", meaning if e.g. the index is replicated to
//! another machine, it's OK to not copy parallel segments indices, since
//! they will just be regenerated (at a cost though).
//!
//! Java's abstract `ReindexingReader` and its anonymous subclasses are
//! [`ReindexingReader`] over a [`ReindexingHooks`] implementation; the state
//! its inner classes (`ParallelLeafDirectoryReader`, `ParallelReaderClosed`,
//! `ReindexingMergePolicy`, the merged-segment warmer) reach through
//! `ReindexingReader.this` is [`ReindexingState`], which they share. The
//! anonymous `ParallelLeafReader` subclass, the `FilterDirectoryReader`
//! subclass and the `OneMerge` subclass are expressed with the override
//! hooks Rucene declares on those types.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, LongPoint, NumericDocValuesField, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::codec_reader::CodecReader;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, FilterDirectoryReader, SlowCodecReaderWrapper, SubReaderWrapper,
};
use rucene::index::index_reader::{CacheKey, ClosedListener};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, MergedSegmentWarmer};
use rucene::index::merge_policy::{
    CodecReaderSupplier, FilterMergePolicy, LogMergePolicy, MergeContext, MergePolicy,
    MergeSpecification, MergeTrigger, OneMerge, TieredMergePolicy,
};
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::parallel_reader::{ParallelLeafReader, ParallelLeafReaderOverrides};
use rucene::index::reader_manager::ReaderManager;
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::segment_reader::SegmentReader;
use rucene::index::terms::Term;
use rucene::index::{IndexReader, LeafReader, SegmentCommitInfo, SegmentInfo};
use rucene::search::field_comparator::SortValue;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::{IndexSearcher, MatchAllDocsQuery};
use rucene::store::java_nio_file::{Files, Path};
use rucene::store::DEFAULT_IO_CONTEXT;
use rucene::util::string_helper::StringHelper;
use rucene::util::{Bits, IOUtils, Version};
use rucene_test_framework::store::Throttling;
use rucene_test_framework::util::lucene_test_case::{TestDirectory, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class, random, test};

/// `TestDemoParallelLeafReader`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestDemoParallelLeafReader",
        vec![
            test(
                "testBasicMultipleSchemaGens",
                test_basic_multiple_schema_gens,
            ),
            test(
                "testRandomMultipleSchemaGens",
                test_random_multiple_schema_gens,
            ),
            test(
                "testRandomMultipleSchemaGensSameField",
                test_random_multiple_schema_gens_same_field,
            ),
            test("testBasic", test_basic),
            test("testRandom", test_random),
        ],
    )
}

const DEBUG: bool = false;

/// Key used to store the current schema gen in the SegmentInfo diagnostics
const SCHEMA_GEN_KEY: &str = "schema_gen";

/// `Long.parseLong(s)`.
fn parse_long(s: &str) -> i64 {
    s.parse().unwrap_or_else(|_| {
        Throwable::new(
            "java.lang.NumberFormatException",
            format!("For input string: \"{s}\""),
        )
        .throw()
    })
}

/// `Long.parseLong(doc.get("text").split(" ")[1])`.
fn number_of(doc: &Document) -> i64 {
    let text = doc
        .get("text")
        .unwrap_or_else(|| crate::support::npe("doc.get(\"text\")"));
    let part = text.split(' ').nth(1).unwrap_or_else(|| {
        Throwable::new(
            "java.lang.ArrayIndexOutOfBoundsException",
            "Index 1 out of bounds for length 1",
        )
        .throw()
    });
    parse_long(part)
}

/// `leaf.incRef()`, `decRef()` and `tryIncRef()` on a leaf.
fn leaf_dec_ref(r: &Arc<dyn LeafReader>) -> Result<()> {
    Arc::clone(r).as_index_reader().dec_ref()
}

fn leaf_try_inc_ref(r: &Arc<dyn LeafReader>) -> bool {
    Arc::clone(r).as_index_reader().try_inc_ref()
}

/// The methods `ReindexingReader`'s anonymous subclasses implement.
trait ReindexingHooks: Send + Sync {
    /// `protected abstract IndexWriterConfig getIndexWriterConfig()`.
    fn get_index_writer_config(&self) -> Result<IndexWriterConfig>;

    /// Optional method to validate that the provided parallel reader in fact
    /// reflects the changes in schemaGen.
    #[allow(dead_code)]
    fn check_parallel_reader(
        &self,
        _reader: &Arc<dyn LeafReader>,
        _parallel_reader: &Arc<dyn LeafReader>,
        _schema_gen: i64,
    ) -> Result<()> {
        Ok(())
    }

    /// Override to customize Directory impl.
    fn open_directory(&self, path: &Path) -> Result<TestDirectory>;

    /// `protected abstract void reindex(long oldSchemaGen, long newSchemaGen,
    /// LeafReader reader, Directory parallelDir)`.
    fn reindex(
        &self,
        old_schema_gen: i64,
        new_schema_gen: i64,
        reader: &Arc<dyn LeafReader>,
        parallel_dir: &TestDirectory,
    ) -> Result<()>;

    /// Returns the gen for the current schema.
    fn get_current_schema_gen(&self) -> i64;

    /// Returns the gen that should be merged, meaning those changes will be
    /// folded back into the main index.
    fn get_merging_schema_gen(&self) -> i64 {
        self.get_current_schema_gen()
    }
}

/// `private static class SegmentIDAndGen`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SegmentIDAndGen {
    seg_id: String,
    schema_gen: i64,
}

impl SegmentIDAndGen {
    fn new(seg_id: String, schema_gen: i64) -> Self {
        SegmentIDAndGen { seg_id, schema_gen }
    }

    /// `public SegmentIDAndGen(String s)`.
    fn parse(s: &str) -> Self {
        let parts: Vec<&str> = s.split('_').collect();
        if parts.len() != 2 {
            Throwable::new(
                "java.lang.IllegalArgumentException",
                format!("invalid SegmentIDAndGen \"{s}\""),
            )
            .throw()
        }
        // TODO: better checking of segID?
        SegmentIDAndGen {
            seg_id: parts[0].to_string(),
            schema_gen: parse_long(parts[1]),
        }
    }
}

impl std::fmt::Display for SegmentIDAndGen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}_{}", self.seg_id, self.schema_gen)
    }
}

/// The state of `ReindexingReader` its inner classes reach through
/// `ReindexingReader.this`.
struct ReindexingState {
    hooks: Arc<dyn ReindexingHooks>,
    index_dir: Mutex<Option<TestDirectory>>,
    segs_path: Path,
    /// Which segments have been closed, but their parallel index is not yet
    /// not removed.
    closed_segments: Mutex<HashSet<SegmentIDAndGen>>,
    /// Holds currently open parallel readers for each segment.
    parallel_readers: Mutex<HashMap<SegmentIDAndGen, Arc<dyn LeafReader>>>,
    /// `synchronized (ReindexingReader.this)`.
    monitor: Mutex<()>,
}

/// `abstract static class ReindexingReader implements Closeable`.
struct ReindexingReader {
    state: Arc<ReindexingState>,
    w: Arc<IndexWriter>,
    mgr: ReaderManager,
}

impl ReindexingReader {
    /// `public ReindexingReader(Path root)`.
    fn new(root: &Path, hooks: Arc<dyn ReindexingHooks>) -> Result<Self> {
        // Normal index is stored under "index":
        let index_dir = hooks.open_directory(&root.resolve_str("index"))?;

        // Per-segment parallel indices are stored under subdirs "segs":
        let segs_path = root.resolve_str("segs");
        Files::create_directories(&segs_path)?;

        let state = Arc::new(ReindexingState {
            hooks: Arc::clone(&hooks),
            index_dir: Mutex::new(None),
            segs_path,
            closed_segments: Mutex::new(HashSet::new()),
            parallel_readers: Mutex::new(HashMap::new()),
            monitor: Mutex::new(()),
        });

        let iwc = hooks.get_index_writer_config()?;
        let inner = iwc.merge_policy();
        iwc.set_merge_policy(Some(Arc::new(ReindexingMergePolicy {
            filter: FilterMergePolicy::new(inner),
            state: Arc::downgrade(&state),
        })))?;
        if DEBUG {
            println!("TEST: use IWC:\n{iwc}");
        }
        let w = IndexWriter::new(index_dir.as_directory(), iwc)?;
        *state.index_dir.lock().unwrap_or_else(|e| e.into_inner()) = Some(index_dir);

        w.get_config()
            .set_merged_segment_warmer(Some(Arc::new(ParallelWarmer {
                state: Arc::downgrade(&state),
            })));

        // start with empty commit:
        w.commit()?;
        let mgr = ReaderManager::from_reader(parallel_leaf_directory_reader(
            &state,
            directory_reader::open_with_writer(&*w)?,
        )?)?;
        Ok(ReindexingReader { state, w, mgr })
    }

    fn commit(&self) -> Result<()> {
        self.w.commit()?;
        Ok(())
    }

    #[allow(dead_code)]
    fn print_ref_counts(&self) {
        println!("All refCounts:");
        for (key, value) in self
            .state
            .parallel_readers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            println!(
                "  {key} {value} refCount={}",
                Arc::clone(value).as_index_reader().get_ref_count()
            );
        }
    }

    /// `public void close()`.
    fn close(self) -> Result<()> {
        self.w.close()?;
        if DEBUG {
            println!(
                "TEST: after close writer index={}",
                SegmentInfos::read_latest_commit(self.state.index_directory())?
            );
        }

        self.mgr.close()?;
        self.state.prune_old_segments(true)?;
        self.state.assert_no_extra_segments()?;
        drop(self.w);
        let index_dir = self
            .state
            .index_dir
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(mut index_dir) = index_dir {
            index_dir.close()?;
        }
        Ok(())
    }
}

impl ReindexingState {
    fn index_directory(&self) -> Arc<dyn rucene::store::Directory> {
        self.index_dir
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .unwrap_or_else(|| crate::support::npe("indexDir"))
            .as_directory()
    }

    /// `LeafReader getCurrentReader(LeafReader reader, long schemaGen)`.
    fn get_current_reader(
        self: &Arc<Self>,
        reader: Arc<dyn LeafReader>,
        schema_gen: i64,
    ) -> Result<Arc<dyn LeafReader>> {
        let parallel_reader = self.get_parallel_leaf_reader(&reader, true, schema_gen)?;
        if let Some(parallel_reader) = parallel_reader {
            // We should not be embedding one ParallelLeafReader inside another:
            Assert::assert_false(parallel_reader.as_parallel_leaf_reader().is_some());
            Assert::assert_false(reader.as_parallel_leaf_reader().is_some());

            // NOTE: important that parallelReader is first, so if there are field name overlaps,
            // because changes to the schema
            // overwrote existing field names, it wins:
            let new_reader: Arc<dyn LeafReader> = Arc::new(
                ParallelLeafReader::with_close_flag(
                    false,
                    vec![Arc::clone(&parallel_reader), reader],
                )?
                .with_overrides(Arc::new(LiveDocsOfSecondReader)),
            );

            // Because ParallelLeafReader does its own (extra) incRef:
            leaf_dec_ref(&parallel_reader)?;

            Ok(new_reader)
        } else {
            // This segment was already current as of currentSchemaGen:
            Ok(reader)
        }
    }

    /// Make sure we deleted all parallel indices for segments that are no
    /// longer in the main index.
    fn assert_no_extra_segments(&self) -> Result<()> {
        let mut live_ids = HashSet::new();
        for info in SegmentInfos::read_latest_commit(self.index_directory())?.iter() {
            let id_string = StringHelper::id_to_string(Some(&info.info.id()));
            live_ids.insert(id_string);
        }

        // At this point (closing) the only segments in closedSegments should be the still-live ones:
        for seg_id_gen in self
            .closed_segments
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            Assert::assert_true(live_ids.contains(&seg_id_gen.seg_id));
        }

        let mut fail = false;
        for path in seg_sub_dirs(&self.segs_path)? {
            let seg_id_gen = SegmentIDAndGen::parse(&file_name_of(&path));
            if !live_ids.contains(&seg_id_gen.seg_id) {
                if DEBUG {
                    println!(
                        "TEST: fail seg={} is not live but still has a parallel index",
                        file_name_of(&path)
                    );
                }
                fail = true;
            }
        }
        Assert::assert_false(fail);
        Ok(())
    }

    /// `LeafReader getParallelLeafReader(final LeafReader leaf, boolean
    /// doCache, long schemaGen)`: returns a ref.
    fn get_parallel_leaf_reader(
        self: &Arc<Self>,
        leaf: &Arc<dyn LeafReader>,
        do_cache: bool,
        schema_gen: i64,
    ) -> Result<Option<Arc<dyn LeafReader>>> {
        debug_assert!(leaf.as_segment_reader().is_some());
        let segment_reader = leaf.as_segment_reader().unwrap_or_else(|| {
            Throwable::new("java.lang.ClassCastException", "not a SegmentReader").throw()
        });
        let info: SegmentInfo = segment_reader.get_segment_info().info.clone();

        let info_schema_gen = get_schema_gen(&info);

        if DEBUG {
            println!(
                "TEST: getParallelLeafReader: {leaf} infoSchemaGen={info_schema_gen} vs schemaGen={schema_gen} doCache={do_cache}"
            );
        }

        if info_schema_gen == schema_gen {
            if DEBUG {
                println!("TEST: segment is already current schemaGen={schema_gen}; skipping");
            }
            return Ok(None);
        }

        if info_schema_gen > schema_gen {
            return Err(LuceneError::illegal_state(format!(
                "segment infoSchemaGen ({info_schema_gen}) cannot be greater than requested schemaGen ({schema_gen})"
            )));
        }

        let seg_id_gen =
            SegmentIDAndGen::new(StringHelper::id_to_string(Some(&info.id())), schema_gen);

        // While loop because the parallel reader may be closed out from under us, so we must retry:
        loop {
            // TODO: make this sync finer, i.e. just the segment + schemaGen
            let _sync = self.monitor.lock().unwrap_or_else(|e| e.into_inner());
            let par_reader = self
                .parallel_readers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&seg_id_gen)
                .cloned();

            debug_assert!(do_cache || par_reader.is_none());

            let par_reader = match par_reader {
                None => {
                    let leaf_index = self.segs_path.resolve_str(&seg_id_gen.to_string());

                    let mut dir = self.hooks.open_directory(&leaf_index)?;

                    if !LuceneTestCase::slow_file_exists(&*dir.as_directory(), "done")? {
                        if DEBUG {
                            println!(
                                "TEST: build segment index for {leaf} {seg_id_gen} (source: {:?}) dir={leaf_index}",
                                info.get_diagnostics().get("source")
                            );
                        }

                        if !dir.as_directory().list_all()?.is_empty() {
                            // It crashed before finishing last time:
                            if DEBUG {
                                println!("TEST: remove old incomplete index files: {leaf_index}");
                            }
                            IOUtils::rm([&leaf_index])?;
                        }

                        self.hooks
                            .reindex(info_schema_gen, schema_gen, leaf, &dir)?;

                        // Marker file, telling us this index is in fact done.  This way if we crash while
                        // doing the reindexing for a given segment, we will
                        // later try again:
                        dir.as_directory()
                            .create_output("done", &*DEFAULT_IO_CONTEXT)?
                            .close()?;
                    } else if DEBUG {
                        println!(
                            "TEST: segment index already exists for {leaf} {seg_id_gen} (source: {:?}) dir={leaf_index}",
                            info.get_diagnostics().get("source")
                        );
                    }

                    if DEBUG {
                        println!(
                            "TEST: now check index {}",
                            dir.as_directory().directory_to_string()
                        );
                    }
                    // TestUtil.checkIndex(dir);

                    let infos = SegmentInfos::read_latest_commit(dir.as_directory())?;
                    debug_assert!(infos.size() == 1);
                    let par_leaf_reader: Arc<dyn LeafReader> = Arc::new(SegmentReader::new(
                        infos.info(0).clone(),
                        i32::from(Version::LATEST.major),
                        &*DEFAULT_IO_CONTEXT,
                    )?);

                    // checkParallelReader(leaf, parLeafReader, schemaGen);

                    if DEBUG {
                        println!("TEST: opened parallel reader: {par_leaf_reader}");
                    }
                    if do_cache {
                        self.parallel_readers
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(seg_id_gen.clone(), Arc::clone(&par_leaf_reader));

                        // Our id+gen could have been previously closed, e.g. if it was a merged segment that
                        // was warmed, so we must clear this else
                        // the pruning may remove our directory:
                        self.closed_segments
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .remove(&seg_id_gen);

                        par_leaf_reader
                            .get_reader_cache_helper()
                            .unwrap_or_else(|| crate::support::npe("getReaderCacheHelper()"))
                            .add_closed_listener(Box::new(ParallelReaderClosed {
                                seg_id_gen: seg_id_gen.clone(),
                                dir: Mutex::new(Some(dir)),
                                state: Arc::downgrade(self),
                            }));
                    } else {
                        // Used only for merged segment warming:
                        // Messy: we close this reader now, instead of leaving open for reuse:
                        if DEBUG {
                            println!(
                                "TEST: now decRef non cached refCount={}",
                                Arc::clone(&par_leaf_reader)
                                    .as_index_reader()
                                    .get_ref_count()
                            );
                        }
                        leaf_dec_ref(&par_leaf_reader)?;
                        dir.close()?;

                        // Must do this after dir is closed, else another thread could "rm -rf" while we are
                        // closing (which makes MDW.close's
                        // checkIndex angry):
                        self.closed_segments
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(seg_id_gen.clone());
                    }
                    par_leaf_reader
                }
                Some(par_reader) => {
                    if !leaf_try_inc_ref(&par_reader) {
                        // We failed: this reader just got closed by another thread, e.g. refresh thread
                        // opening a new reader, so this reader is now
                        // closed and we must try again.
                        if DEBUG {
                            println!("TEST: tryIncRef failed for {par_reader}; retry");
                        }
                        continue;
                    }
                    if DEBUG {
                        println!(
                            "TEST: use existing already opened parReader={par_reader} refCount={}",
                            Arc::clone(&par_reader).as_index_reader().get_ref_count()
                        );
                    }
                    // checkParallelReader(leaf, parReader, schemaGen);
                    par_reader
                }
            };

            // We return the new reference to caller
            return Ok(Some(par_reader));
        }
    }

    /// Removes the parallel index that are no longer in the last commit
    /// point. We can't remove this when the parallel reader is closed because
    /// it may still be referenced by the last commit.
    fn prune_old_segments(&self, remove_old_gens: bool) -> Result<()> {
        let last_commit = SegmentInfos::read_latest_commit(self.index_directory())?;
        if DEBUG {
            println!("TEST: prune");
        }

        let mut live_ids = HashSet::new();
        for info in last_commit.iter() {
            let id_string = StringHelper::id_to_string(Some(&info.info.id()));
            live_ids.insert(id_string);
        }

        let current_schema_gen = self.hooks.get_current_schema_gen();

        if Files::exists(&self.segs_path, &[]) {
            for path in seg_sub_dirs(&self.segs_path)? {
                if Files::is_directory(&path, &[]) {
                    let seg_id_gen = SegmentIDAndGen::parse(&file_name_of(&path));
                    debug_assert!(seg_id_gen.schema_gen <= current_schema_gen);
                    let closed = self
                        .closed_segments
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .contains(&seg_id_gen);
                    if !live_ids.contains(&seg_id_gen.seg_id)
                        && (closed
                            || (remove_old_gens && seg_id_gen.schema_gen < current_schema_gen))
                    {
                        if DEBUG {
                            println!("TEST: remove {seg_id_gen}");
                        }
                        match IOUtils::rm([&path]) {
                            Ok(()) => {
                                self.closed_segments
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .remove(&seg_id_gen);
                            }
                            Err(ioe) => {
                                // OK, we'll retry later
                                if DEBUG {
                                    println!("TEST: ignore ioe during delete {path}:{ioe}");
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// The overrides of the anonymous `ParallelLeafReader` subclass of
/// `getCurrentReader`: `getLiveDocs()` and `numDocs()` answer from
/// `getParallelReaders()[1]`.
struct LiveDocsOfSecondReader;

impl ParallelLeafReaderOverrides for LiveDocsOfSecondReader {
    fn get_live_docs(&self, this: &ParallelLeafReader) -> Option<Box<dyn Bits>> {
        this.get_parallel_readers()[1].get_live_docs()
    }

    fn num_docs(&self, this: &ParallelLeafReader) -> i32 {
        this.get_parallel_readers()[1].num_docs()
    }
}

/// The merged-segment warmer of `ReindexingReader`'s constructor.
struct ParallelWarmer {
    state: Weak<ReindexingState>,
}

impl std::fmt::Debug for ParallelWarmer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ParallelWarmer")
    }
}

impl std::fmt::Display for ParallelWarmer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TestDemoParallelLeafReader$ReindexingReader$$Lambda")
    }
}

impl MergedSegmentWarmer for ParallelWarmer {
    fn warm(&self, reader: &Arc<dyn LeafReader>) -> Result<()> {
        let Some(state) = self.state.upgrade() else {
            return Ok(());
        };
        // This will build the parallel index for the merged segment before the merge
        // becomes visible, so reopen delay is only due to
        // newly flushed segments:
        if DEBUG {
            println!("TEST: now warm {reader}");
        }
        // TODO: it's not great that we pass false here; it means we close the reader &
        // reopen again for NRT reader; still we did "warm" by
        // building the parallel index, if necessary
        state.get_parallel_leaf_reader(reader, false, state.hooks.get_current_schema_gen())?;
        Ok(())
    }
}

/// `new ParallelLeafDirectoryReader(in)`: the `FilterDirectoryReader`
/// subclass `private class ParallelLeafDirectoryReader`.
fn parallel_leaf_directory_reader(
    state: &Arc<ReindexingState>,
    inner: Arc<dyn DirectoryReader>,
) -> Result<Arc<dyn DirectoryReader>> {
    let current_schema_gen = state.hooks.get_current_schema_gen();
    Ok(Arc::new(FilterDirectoryReader::new(
        inner,
        Arc::new(ParallelSubReaderWrapper {
            state: Arc::downgrade(state),
            current_schema_gen,
        }),
        Arc::new(ParallelDirectoryReaderWrapper {
            state: Arc::downgrade(state),
        }),
    )?))
}

/// The anonymous `FilterDirectoryReader.SubReaderWrapper` of
/// `ParallelLeafDirectoryReader`.
struct ParallelSubReaderWrapper {
    state: Weak<ReindexingState>,
    current_schema_gen: i64,
}

impl SubReaderWrapper for ParallelSubReaderWrapper {
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        let state = self
            .state
            .upgrade()
            .unwrap_or_else(|| crate::support::npe("ReindexingReader.this"));
        match state.get_current_reader(reader, self.current_schema_gen) {
            Ok(r) => Ok(r),
            // TODO: must close on exc here:
            Err(ioe) => Err(LuceneError::runtime_exception(ioe)),
        }
    }
}

/// `ParallelLeafDirectoryReader`'s overrides of `doWrapDirectoryReader`,
/// `doClose` and `getReaderCacheHelper`.
struct ParallelDirectoryReaderWrapper {
    state: Weak<ReindexingState>,
}

impl DirectoryReaderWrapper for ParallelDirectoryReaderWrapper {
    fn wrap(&self, inner: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        let state = self
            .state
            .upgrade()
            .unwrap_or_else(|| crate::support::npe("ReindexingReader.this"));
        parallel_leaf_directory_reader(&state, inner)
    }

    fn subclass_simple_name(&self) -> &'static str {
        "ParallelLeafDirectoryReader"
    }

    fn reader_cache_helper_is_null(&self) -> bool {
        true
    }

    fn do_close(&self, reader: &FilterDirectoryReader) -> Result<()> {
        let mut first_exc: Option<LuceneError> = None;
        for r in reader.wrapped_leaves() {
            if r.as_parallel_leaf_reader().is_some() {
                // try to close each reader, even if an exception is thrown
                if let Err(t) = leaf_dec_ref(r) {
                    if first_exc.is_none() {
                        first_exc = Some(t);
                    }
                }
            }
        }
        // Also close in, so it decRef's the SegmentInfos
        if let Err(t) = reader.get_delegate().do_close() {
            if first_exc.is_none() {
                first_exc = Some(t);
            }
        }

        // throw the first exception
        match first_exc {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// `private class ParallelReaderClosed implements IndexReader.ClosedListener`.
struct ParallelReaderClosed {
    seg_id_gen: SegmentIDAndGen,
    dir: Mutex<Option<TestDirectory>>,
    state: Weak<ReindexingState>,
}

impl ClosedListener for ParallelReaderClosed {
    fn on_close(&self, _ignored: CacheKey) -> Result<()> {
        let Some(state) = self.state.upgrade() else {
            return Ok(());
        };
        // TODO: make this sync finer, i.e. just the segment + schemaGen
        let _sync = state.monitor.lock().unwrap_or_else(|e| e.into_inner());
        let dir = self.dir.lock().unwrap_or_else(|e| e.into_inner()).take();
        if DEBUG {
            println!(
                "TEST: now close parallel parLeafReader dir={:?} segIDGen={}",
                dir.as_ref().map(|d| d.as_directory().directory_to_string()),
                self.seg_id_gen
            );
        }
        state
            .parallel_readers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.seg_id_gen);
        if let Some(mut dir) = dir {
            let description = dir.as_directory().directory_to_string();
            if let Err(ioe) = dir.close() {
                println!("TEST: hit IOExc closing dir={description}");
                println!("{ioe}");
                Throwable::from_cause(
                    "java.lang.RuntimeException",
                    rucene_test_framework::util::java_exceptions::java_throwable(&ioe),
                )
                .throw()
            }
        }
        state
            .closed_segments
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(self.seg_id_gen.clone());
        Ok(())
    }
}

/// Just replaces the sub-readers with parallel readers, so reindexed fields
/// are merged into new segments.
///
/// `private class ReindexingMergePolicy extends FilterMergePolicy`: every
/// decision it does not override is the wrapped policy's, through
/// `FilterMergePolicy`.
struct ReindexingMergePolicy {
    filter: FilterMergePolicy,
    state: Weak<ReindexingState>,
}

impl ReindexingMergePolicy {
    /// `new ReindexingOneMerge(segments)`: `class ReindexingOneMerge extends
    /// OneMerge`, its overrides of `wrapForMerge`, `mergeFinished` and
    /// `setMergeInfo` installed as the merge's hooks.
    fn reindexing_one_merge(&self, segments: Vec<SegmentCommitInfo>) -> Result<OneMerge> {
        let state = self
            .state
            .upgrade()
            .unwrap_or_else(|| crate::support::npe("ReindexingReader.this"));
        let mut merge = OneMerge::new(segments)?;
        // Commit up front to which schemaGen we will merge; we don't want a schema change
        // sneaking in for some of our leaf readers but not others:
        let schema_gen = state.hooks.get_merging_schema_gen();
        let current_schema_gen = state.hooks.get_current_schema_gen();

        // Defensive sanity check:
        if schema_gen > current_schema_gen {
            return Err(LuceneError::illegal_state(format!(
                "currentSchemaGen ({current_schema_gen}) must always be >= mergingSchemaGen ({schema_gen})"
            )));
        }

        let parallel_readers: Arc<Mutex<Vec<Arc<dyn LeafReader>>>> =
            Arc::new(Mutex::new(Vec::new()));

        {
            let state = Arc::downgrade(&state);
            let parallel_readers = Arc::clone(&parallel_readers);
            merge.set_wrap_for_merge(Arc::new(move |reader: Arc<dyn CodecReader>| {
                let state = state
                    .upgrade()
                    .unwrap_or_else(|| crate::support::npe("ReindexingReader.this"));
                let wrapped =
                    state.get_current_reader(reader as Arc<dyn LeafReader>, schema_gen)?;
                if wrapped.as_parallel_leaf_reader().is_some() {
                    parallel_readers
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(Arc::clone(&wrapped));
                }
                SlowCodecReaderWrapper::wrap(wrapped)
            }));
        }

        {
            let parallel_readers = Arc::clone(&parallel_readers);
            merge.set_merge_finished(Arc::new(
                move |_merge, _success, _segment_dropped, _deleter| {
                    // `super.mergeFinished(success, segmentDropped)` does nothing.
                    let mut th: Option<LuceneError> = None;
                    let readers = std::mem::take(
                        &mut *parallel_readers.lock().unwrap_or_else(|e| e.into_inner()),
                    );
                    for r in &readers {
                        if let Err(t) = leaf_dec_ref(r) {
                            if th.is_none() {
                                th = Some(t);
                            }
                        }
                    }

                    match th {
                        Some(th) => Err(th),
                        None => Ok(()),
                    }
                },
            ));
        }

        merge.set_set_merge_info_override(Arc::new(
            move |merge: &OneMerge, mut info: SegmentCommitInfo| {
                // Record that this merged segment is current as of this schemaGen:
                info.info.add_diagnostics(HashMap::from([(
                    SCHEMA_GEN_KEY.to_string(),
                    schema_gen.to_string(),
                )]));
                merge.super_set_merge_info(info);
            },
        ));

        Ok(merge)
    }

    /// `MergeSpecification wrap(MergeSpecification spec)`: a
    /// `ReindexingMergeSpecification`, whose `add(merge)` adds
    /// `new ReindexingOneMerge(merge.segments)`.
    ///
    /// Java's subclass also renders `segString` as
    /// `"ReindexingMergeSpec(" + super.segString(dir) + ")"`; Rucene's
    /// `MergeSpecification` is a concrete type, so the spec renders as the
    /// plain one in the writer's info stream.
    fn wrap(&self, spec: Option<MergeSpecification>) -> Result<Option<MergeSpecification>> {
        let Some(spec) = spec else {
            return Ok(None);
        };
        let mut wrapped = MergeSpecification::new();
        for merge in spec.merges {
            wrapped.add(self.reindexing_one_merge(merge.segments.clone())?);
        }
        Ok(Some(wrapped))
    }
}

impl std::fmt::Display for ReindexingMergePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ReindexingMergePolicy({})", self.filter.get_delegate())
    }
}

impl MergePolicy for ReindexingMergePolicy {
    fn find_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.wrap(self.filter.get_delegate().find_merges(
            merge_trigger,
            segment_infos,
            merge_context,
        )?)
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_segment_count: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        // TODO: do we need to force-force this?  Ie, wrapped MP may think index is already
        // optimized, yet maybe its schemaGen is old?  need test!
        self.wrap(self.filter.get_delegate().find_forced_merges(
            segment_infos,
            max_segment_count,
            segments_to_merge,
            merge_context,
        )?)
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.wrap(
            self.filter
                .get_delegate()
                .find_forced_deletes_merges(segment_infos, merge_context)?,
        )
    }

    fn find_full_flush_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.wrap(self.filter.get_delegate().find_full_flush_merges(
            merge_trigger,
            segment_infos,
            merge_context,
        )?)
    }

    fn use_compound_file(
        &self,
        segments: &SegmentInfos,
        new_segment: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        self.filter
            .get_delegate()
            .use_compound_file(segments, new_segment, merge_context)
    }

    // The rest is `FilterMergePolicy`'s delegation.

    fn find_merges_for_readers(
        &self,
        readers: &[Arc<dyn CodecReader>],
    ) -> Result<Option<MergeSpecification>> {
        self.filter.find_merges_for_readers(readers)
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.filter.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.filter.get_max_cfs_segment_size()
    }

    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.filter.get_max_cfs_segment_size_mb()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.filter.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.filter.set_max_cfs_segment_size_mb(v)
    }

    fn size(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        self.filter.size(info, merge_context)
    }

    fn keep_fully_deleted_segment(
        &self,
        reader_supplier: &mut CodecReaderSupplier<'_>,
    ) -> Result<bool> {
        self.filter.keep_fully_deleted_segment(reader_supplier)
    }

    fn num_deletes_to_merge_default(&self, info: &SegmentCommitInfo, del_count: i32) -> i32 {
        self.filter.num_deletes_to_merge_default(info, del_count)
    }

    fn num_deletes_to_merge_with_reader(
        &self,
        info: &SegmentCommitInfo,
        del_count: i32,
        reader_supplier: &mut dyn FnMut() -> Result<Arc<dyn CodecReader>>,
    ) -> Result<i32> {
        self.filter
            .num_deletes_to_merge_with_reader(info, del_count, reader_supplier)
    }

    fn max_full_flush_merge_size(&self) -> i64 {
        self.filter.max_full_flush_merge_size()
    }
}

/// `static long getSchemaGen(SegmentInfo info)`.
fn get_schema_gen(info: &SegmentInfo) -> i64 {
    match info.get_diagnostics().get(SCHEMA_GEN_KEY) {
        None => -1,
        Some(s) => parse_long(s),
    }
}

/// `newIndexWriterConfig()` with a `TieredMergePolicy` whose floor is
/// `.01` MB: "We write tiny docs, so we need tiny floor to avoid O(N^2)
/// merging".
fn tiny_floor_iwc() -> Result<IndexWriterConfig> {
    let iwc = LuceneTestCase::new_index_writer_config();
    let tmp = TieredMergePolicy::new();
    // We write tiny docs, so we need tiny floor to avoid O(N^2) merging:
    tmp.set_floor_segment_mb(0.01)?;
    iwc.set_merge_policy(Some(Arc::new(tmp)))?;
    Ok(iwc)
}

/// The `openDirectory(path)` override of every reindexer:
/// `newMockFSDirectory(path)` with slow open/closers off and throttling
/// `NEVER`.
fn open_mock_fs_directory(path: &Path) -> Result<TestDirectory> {
    let dir = LuceneTestCase::new_mock_fs_directory(path);
    dir.set_use_slow_open_closers(false);
    dir.set_throttling(Throttling::Never);
    Ok(TestDirectory::Mock(dir))
}

/// `newIndexWriterConfig()` with a `LogByteSizeMergePolicy`: "The order of
/// our docIDs must precisely match incoming reader".
fn reindex_iwc() -> Result<IndexWriterConfig> {
    let iwc = LuceneTestCase::new_index_writer_config();
    // The order of our docIDs must precisely match incoming reader:
    iwc.set_merge_policy(Some(Arc::new(LogMergePolicy::by_byte_size())))?;
    Ok(iwc)
}

/// The anonymous `ReindexingReader` of `getReindexer(root)`.
struct BasicReindexer;

impl ReindexingHooks for BasicReindexer {
    fn get_index_writer_config(&self) -> Result<IndexWriterConfig> {
        tiny_floor_iwc()
    }

    fn open_directory(&self, path: &Path) -> Result<TestDirectory> {
        open_mock_fs_directory(path)
    }

    fn reindex(
        &self,
        _old_schema_gen: i64,
        _new_schema_gen: i64,
        reader: &Arc<dyn LeafReader>,
        parallel_dir: &TestDirectory,
    ) -> Result<()> {
        let w = IndexWriter::new(parallel_dir.as_directory(), reindex_iwc()?)?;
        let max_doc = reader.max_doc();
        let stored_fields = reader.stored_fields()?;

        // Slowly parse the stored field into a new doc values field:
        for i in 0..max_doc {
            // TODO: is this still O(blockSize^2)?
            let old_doc = stored_fields.document(i)?;
            let mut new_doc = Document::new();
            let value = number_of(&old_doc);
            new_doc.add(Box::new(NumericDocValuesField::new("number", value)));
            new_doc.add(Box::new(LongPoint::new("number", &[value])?));
            w.add_document(&new_doc)?;
        }

        w.force_merge(1)?;

        w.close()
    }

    fn get_current_schema_gen(&self) -> i64 {
        0
    }
}

/// `private ReindexingReader getReindexer(Path root)`.
fn get_reindexer(root: &Path) -> Result<ReindexingReader> {
    ReindexingReader::new(root, Arc::new(BasicReindexer))
}

/// Schema change by adding a new number_<schemaGen> DV field each time.
///
/// The anonymous `ReindexingReader` of `getReindexerNewDVFields`.
struct NewDvFieldsReindexer {
    current_schema_gen: Arc<AtomicI64>,
}

impl ReindexingHooks for NewDvFieldsReindexer {
    fn get_index_writer_config(&self) -> Result<IndexWriterConfig> {
        tiny_floor_iwc()
    }

    fn open_directory(&self, path: &Path) -> Result<TestDirectory> {
        open_mock_fs_directory(path)
    }

    fn reindex(
        &self,
        old_schema_gen: i64,
        new_schema_gen: i64,
        reader: &Arc<dyn LeafReader>,
        parallel_dir: &TestDirectory,
    ) -> Result<()> {
        let w = IndexWriter::new(parallel_dir.as_directory(), reindex_iwc()?)?;
        let max_doc = reader.max_doc();
        let stored_fields = reader.stored_fields()?;

        if old_schema_gen <= 0 {
            // Must slowly parse the stored field into a new doc values field:
            for i in 0..max_doc {
                // TODO: is this still O(blockSize^2)?
                let old_doc = stored_fields.document(i)?;
                let mut new_doc = Document::new();
                let value = number_of(&old_doc);
                new_doc.add(Box::new(NumericDocValuesField::new(
                    &format!("number_{new_schema_gen}"),
                    value,
                )));
                new_doc.add(Box::new(LongPoint::new("number", &[value])?));
                w.add_document(&new_doc)?;
            }
        } else {
            // Just carry over doc values from previous field:
            let old_values = reader.get_numeric_doc_values(&format!("number_{old_schema_gen}"))?;
            crate::support::assert_not_null_msg(
                &format!("oldSchemaGen={old_schema_gen}"),
                &old_values,
            );
            let mut old_values = old_values.expect("checked by assertNotNull");
            for i in 0..max_doc {
                // TODO: is this still O(blockSize^2)?
                assert_equals(&i, &old_values.next_doc()?);
                stored_fields.document(i)?;
                let mut new_doc = Document::new();
                new_doc.add(Box::new(NumericDocValuesField::new(
                    &format!("number_{new_schema_gen}"),
                    old_values.long_value()?,
                )));
                w.add_document(&new_doc)?;
            }
        }

        w.force_merge(1)?;

        w.close()
    }

    fn get_current_schema_gen(&self) -> i64 {
        self.current_schema_gen.load(Ordering::SeqCst)
    }

    fn check_parallel_reader(
        &self,
        r: &Arc<dyn LeafReader>,
        par_r: &Arc<dyn LeafReader>,
        schema_gen: i64,
    ) -> Result<()> {
        let field_name = format!("number_{schema_gen}");
        if DEBUG {
            println!("TEST: now check parallel number DVs field={field_name} r={r} parR={par_r}");
        }
        let Some(mut numbers) = par_r.get_numeric_doc_values(&field_name)? else {
            return Ok(());
        };
        let max_doc = r.max_doc();
        let stored_fields = r.stored_fields()?;
        let mut failed = false;
        for i in 0..max_doc {
            let old_doc = stored_fields.document(i)?;
            let value = number_of(&old_doc);
            assert_equals(&i, &numbers.next_doc()?);
            if value != numbers.long_value()? {
                if DEBUG {
                    println!(
                        "FAIL: docID={i} {old_doc:?} value={value} number={}",
                        numbers.long_value()?
                    );
                }
                failed = true;
            } else if failed && DEBUG {
                println!(
                    "OK: docID={i} {old_doc:?} value={value} number={}",
                    numbers.long_value()?
                );
            }
        }
        Assert::assert_false_msg(&format!("FAILED field={field_name} r={r}"), failed);
        Ok(())
    }
}

/// `private ReindexingReader getReindexerNewDVFields(Path root, final
/// AtomicLong currentSchemaGen)`.
fn get_reindexer_new_dv_fields(
    root: &Path,
    current_schema_gen: &Arc<AtomicI64>,
) -> Result<ReindexingReader> {
    ReindexingReader::new(
        root,
        Arc::new(NewDvFieldsReindexer {
            current_schema_gen: Arc::clone(current_schema_gen),
        }),
    )
}

/// Schema change by adding changing how the same "number" DV field is
/// indexed.
///
/// The anonymous `ReindexingReader` of `getReindexerSameDVField`.
struct SameDvFieldReindexer {
    current_schema_gen: Arc<AtomicI64>,
    merging_schema_gen: Arc<AtomicI64>,
}

impl ReindexingHooks for SameDvFieldReindexer {
    fn get_index_writer_config(&self) -> Result<IndexWriterConfig> {
        let iwc = tiny_floor_iwc()?;
        if *TEST_NIGHTLY {
            // during nightly tests, we might use too many files if we aren't careful
            iwc.set_use_compound_file(true)?;
        }
        Ok(iwc)
    }

    fn open_directory(&self, path: &Path) -> Result<TestDirectory> {
        open_mock_fs_directory(path)
    }

    fn reindex(
        &self,
        old_schema_gen: i64,
        new_schema_gen: i64,
        reader: &Arc<dyn LeafReader>,
        parallel_dir: &TestDirectory,
    ) -> Result<()> {
        let w = IndexWriter::new(parallel_dir.as_directory(), reindex_iwc()?)?;
        let max_doc = reader.max_doc();
        let stored_fields = reader.stored_fields()?;

        if old_schema_gen <= 0 {
            // Must slowly parse the stored field into a new doc values field:
            for i in 0..max_doc {
                // TODO: is this still O(blockSize^2)?
                let old_doc = stored_fields.document(i)?;
                let mut new_doc = Document::new();
                let value = number_of(&old_doc);
                new_doc.add(Box::new(NumericDocValuesField::new(
                    "number",
                    new_schema_gen.wrapping_mul(value),
                )));
                new_doc.add(Box::new(LongPoint::new("number", &[value])?));
                w.add_document(&new_doc)?;
            }
        } else {
            let old_values = reader.get_numeric_doc_values("number")?;
            crate::support::assert_not_null_msg(
                &format!("oldSchemaGen={old_schema_gen}"),
                &old_values,
            );
            let mut old_values = old_values.expect("checked by assertNotNull");
            for i in 0..max_doc {
                // TODO: is this still O(blockSize^2)?
                stored_fields.document(i)?;
                let mut new_doc = Document::new();
                assert_equals(&i, &old_values.next_doc()?);
                let value = new_schema_gen.wrapping_mul(old_values.long_value()? / old_schema_gen);
                new_doc.add(Box::new(NumericDocValuesField::new("number", value)));
                new_doc.add(Box::new(LongPoint::new("number", &[value])?));
                w.add_document(&new_doc)?;
            }
        }

        w.force_merge(1)?;

        w.close()
    }

    fn get_current_schema_gen(&self) -> i64 {
        self.current_schema_gen.load(Ordering::SeqCst)
    }

    fn get_merging_schema_gen(&self) -> i64 {
        self.merging_schema_gen.load(Ordering::SeqCst)
    }

    fn check_parallel_reader(
        &self,
        r: &Arc<dyn LeafReader>,
        par_r: &Arc<dyn LeafReader>,
        schema_gen: i64,
    ) -> Result<()> {
        if DEBUG {
            println!("TEST: now check parallel number DVs r={r} parR={par_r}");
        }
        let Some(mut numbers) = par_r.get_numeric_doc_values("numbers")? else {
            return Ok(());
        };
        let max_doc = r.max_doc();
        let stored_fields = r.stored_fields()?;
        let mut failed = false;
        for i in 0..max_doc {
            let old_doc = stored_fields.document(i)?;
            let mut value = number_of(&old_doc);
            value = value.wrapping_mul(schema_gen);
            assert_equals(&i, &numbers.next_doc()?);
            if value != numbers.long_value()? {
                println!(
                    "FAIL: docID={i} {old_doc:?} value={value} number={}",
                    numbers.long_value()?
                );
                failed = true;
            } else if failed {
                println!(
                    "OK: docID={i} {old_doc:?} value={value} number={}",
                    numbers.long_value()?
                );
            }
        }
        Assert::assert_false_msg(&format!("FAILED r={r}"), failed);
        Ok(())
    }
}

/// `private ReindexingReader getReindexerSameDVField(Path root, final
/// AtomicLong currentSchemaGen, final AtomicLong mergingSchemaGen)`.
fn get_reindexer_same_dv_field(
    root: &Path,
    current_schema_gen: &Arc<AtomicI64>,
    merging_schema_gen: &Arc<AtomicI64>,
) -> Result<ReindexingReader> {
    ReindexingReader::new(
        root,
        Arc::new(SameDvFieldReindexer {
            current_schema_gen: Arc::clone(current_schema_gen),
            merging_schema_gen: Arc::clone(merging_schema_gen),
        }),
    )
}

/// `DirectoryReader r = mgr.acquire(); try { body(r) } finally {
/// mgr.release(r); }`.
fn with_acquired(
    mgr: &ReaderManager,
    body: impl FnOnce(&Arc<dyn DirectoryReader>) -> Result<()>,
) -> Result<()> {
    let r = mgr.acquire()?;
    if DEBUG {
        println!("TEST: got reader={r}");
    }
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(&r)));
    let released = mgr.release(r);
    match outcome {
        Ok(result) => result?,
        Err(payload) => std::panic::resume_unwind(payload),
    }
    released
}

/// `doc.add(newTextField("text", "number " + value, Field.Store.YES))`.
fn text_doc(value: impl std::fmt::Display) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "text",
        &format!("number {value}"),
        Store::YES,
    )));
    doc
}

/// Port of `TestDemoParallelLeafReader.testBasicMultipleSchemaGens`.
fn test_basic_multiple_schema_gens() -> Result<()> {
    let current_schema_gen = Arc::new(AtomicI64::new(0));

    // TODO: separate refresh thread, search threads, indexing threads
    let root = LuceneTestCase::create_temp_dir();
    let reindexer = get_reindexer_new_dv_fields(&root, &current_schema_gen)?;
    reindexer.commit()?;

    let doc = text_doc(random().next_long());
    reindexer.w.add_document(&doc)?;

    if DEBUG {
        println!("TEST: refresh @ 1 doc");
    }
    reindexer.mgr.maybe_refresh()?;
    with_acquired(&reindexer.mgr, |r| {
        check_all_number_dvs(
            r,
            &format!("number_{}", current_schema_gen.load(Ordering::SeqCst)),
            true,
            1,
        )
    })?;
    // reindexer.printRefCounts();

    current_schema_gen.fetch_add(1, Ordering::SeqCst);

    if DEBUG {
        println!("TEST: increment schemaGen");
        println!("TEST: commit");
    }
    reindexer.commit()?;

    let doc = text_doc(random().next_long());
    reindexer.w.add_document(&doc)?;

    if DEBUG {
        println!("TEST: refresh @ 2 docs");
    }
    reindexer.mgr.maybe_refresh()?;
    // reindexer.printRefCounts();
    with_acquired(&reindexer.mgr, |r| {
        check_all_number_dvs(
            r,
            &format!("number_{}", current_schema_gen.load(Ordering::SeqCst)),
            true,
            1,
        )
    })?;

    if DEBUG {
        println!("TEST: forceMerge");
    }
    reindexer.w.force_merge(1)?;

    current_schema_gen.fetch_add(1, Ordering::SeqCst);

    if DEBUG {
        println!("TEST: commit");
    }
    reindexer.commit()?;

    if DEBUG {
        println!("TEST: refresh after forceMerge");
    }
    reindexer.mgr.maybe_refresh()?;
    with_acquired(&reindexer.mgr, |r| {
        check_all_number_dvs(
            r,
            &format!("number_{}", current_schema_gen.load(Ordering::SeqCst)),
            true,
            1,
        )
    })?;

    if DEBUG {
        println!("TEST: close writer");
    }
    reindexer.close()
}

/// One document of the random tests: `id` (a replaced id when `updateID`
/// is set) and the text the caller builds; returns `(id, updateID)`.
fn next_id(max_id: &mut i32) -> (String, Option<String>) {
    if *max_id > 0 && random().next_int_bound(10) == 7 {
        // Replace a doc
        let id = random().next_int_bound(*max_id).to_string();
        (id.clone(), Some(id))
    } else {
        let id = max_id.to_string();
        *max_id += 1;
        (id, None)
    }
}

/// `if (updateID == null) addDocument(doc) else updateDocument(new
/// Term("id", updateID), doc)`.
fn add_or_update(w: &IndexWriter, doc: &Document, update_id: Option<String>) -> Result<()> {
    match update_id {
        None => w.add_document(doc)?,
        Some(update_id) => w.update_document(Term::from_text("id", &update_id), doc)?,
    };
    Ok(())
}

/// `doc.add(newStringField("id", id, NO)); doc.add(newTextField("text",
/// "number " + value, YES))`.
fn id_text_doc(id: &str, value: impl std::fmt::Display) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        id,
        Store::NO,
    )));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "text",
        &format!("number {value}"),
        Store::YES,
    )));
    doc
}

/// Port of `TestDemoParallelLeafReader.testRandomMultipleSchemaGens`.
fn test_random_multiple_schema_gens() -> Result<()> {
    let current_schema_gen = Arc::new(AtomicI64::new(0));
    let mut reindexer: Option<ReindexingReader> = None;

    // TODO: separate refresh thread, search threads, indexing threads
    let num_docs = LuceneTestCase::at_least(if *TEST_NIGHTLY { 20000 } else { 200 });
    let mut max_id = 0;
    let root = LuceneTestCase::create_temp_dir();
    let mut refresh_every_num_docs = 100;
    let mut commit_close_num_docs = 1000;
    for i in 0..num_docs {
        if reindexer.is_none() {
            reindexer = Some(get_reindexer_new_dv_fields(&root, &current_schema_gen)?);
        }
        let ri = reindexer.as_ref().expect("INVARIANT: opened above");

        let (id, update_id) = next_id(&mut max_id);
        let doc = id_text_doc(&id, random().next_long());
        add_or_update(&ri.w, &doc, update_id)?;
        if random().next_int_bound(refresh_every_num_docs) == 17 {
            if DEBUG {
                println!("TEST TOP: refresh @ {} docs", i + 1);
            }
            ri.mgr.maybe_refresh()?;

            with_acquired(&ri.mgr, |r| {
                check_all_number_dvs(
                    r,
                    &format!("number_{}", current_schema_gen.load(Ordering::SeqCst)),
                    true,
                    1,
                )
            })?;
            if DEBUG {
                ri.print_ref_counts();
            }
            refresh_every_num_docs = (1.25 * refresh_every_num_docs as f64) as i32;
        }

        if random().next_int_bound(500) == 17 {
            current_schema_gen.fetch_add(1, Ordering::SeqCst);
            if DEBUG {
                println!(
                    "TEST TOP: advance schemaGen to {}",
                    current_schema_gen.load(Ordering::SeqCst)
                );
            }
        }

        if i > 0 && random().next_int_bound(10) == 7 {
            // Random delete:
            ri.w.delete_documents_by_terms(vec![Term::from_text(
                "id",
                &random().next_int_bound(i).to_string(),
            )])?;
        }

        if random().next_int_bound(commit_close_num_docs) == 17 {
            if DEBUG {
                println!("TEST TOP: commit @ {} docs", i + 1);
            }
            ri.commit()?;
            // reindexer.printRefCounts();
            commit_close_num_docs = (1.25 * commit_close_num_docs as f64) as i32;
        }

        // Sometimes close & reopen writer/manager, to confirm the parallel segments persist:
        if random().next_int_bound(commit_close_num_docs) == 17 {
            if DEBUG {
                println!("TEST TOP: close writer @ {} docs", i + 1);
            }
            reindexer.take().expect("INVARIANT: open").close()?;
            commit_close_num_docs = (1.25 * commit_close_num_docs as f64) as i32;
        }
    }

    if let Some(reindexer) = reindexer {
        reindexer.close()?;
    }
    Ok(())
}

/// First schema change creates a new "number" DV field off the stored
/// field; subsequent changes just change the value of that number field for
/// all docs.
///
/// Port of `TestDemoParallelLeafReader.testRandomMultipleSchemaGensSameField`.
fn test_random_multiple_schema_gens_same_field() -> Result<()> {
    let current_schema_gen = Arc::new(AtomicI64::new(0));
    let merging_schema_gen = Arc::new(AtomicI64::new(0));

    let mut reindexer: Option<ReindexingReader> = None;

    // TODO: separate refresh thread, search threads, indexing threads
    let num_docs = LuceneTestCase::at_least(if *TEST_NIGHTLY { 20000 } else { 200 });
    let mut max_id = 0;
    let root = LuceneTestCase::create_temp_dir();
    let mut refresh_every_num_docs = 100;
    let mut commit_close_num_docs = 1000;

    for i in 0..num_docs {
        if reindexer.is_none() {
            if DEBUG {
                println!("TEST TOP: open new reader/writer");
            }
            reindexer = Some(get_reindexer_same_dv_field(
                &root,
                &current_schema_gen,
                &merging_schema_gen,
            )?);
        }
        let ri = reindexer.as_ref().expect("INVARIANT: opened above");

        let (id, update_id) = next_id(&mut max_id);
        let doc = id_text_doc(&id, TestUtil::next_int(&*random(), -10000, 10000));
        add_or_update(&ri.w, &doc, update_id)?;
        if random().next_int_bound(refresh_every_num_docs) == 17 {
            if DEBUG {
                println!("TEST TOP: refresh @ {} docs", i + 1);
            }
            ri.mgr.maybe_refresh()?;
            with_acquired(&ri.mgr, |r| {
                check_all_number_dvs(
                    r,
                    "number",
                    true,
                    current_schema_gen.load(Ordering::SeqCst) as i32,
                )
            })?;
            if DEBUG {
                ri.print_ref_counts();
            }
            refresh_every_num_docs = (1.25 * refresh_every_num_docs as f64) as i32;
        }

        if random().next_int_bound(500) == 17 {
            current_schema_gen.fetch_add(1, Ordering::SeqCst);
            if DEBUG {
                println!(
                    "TEST TOP: advance schemaGen to {}",
                    current_schema_gen.load(Ordering::SeqCst)
                );
            }
            if random().next_boolean() {
                merging_schema_gen.fetch_add(1, Ordering::SeqCst);
                if DEBUG {
                    println!(
                        "TEST TOP: advance mergingSchemaGen to {}",
                        merging_schema_gen.load(Ordering::SeqCst)
                    );
                }
            }
        }

        if i > 0 && random().next_int_bound(10) == 7 {
            // Random delete:
            ri.w.delete_documents_by_terms(vec![Term::from_text(
                "id",
                &random().next_int_bound(i).to_string(),
            )])?;
        }

        if random().next_int_bound(commit_close_num_docs) == 17 {
            if DEBUG {
                println!("TEST TOP: commit @ {} docs", i + 1);
            }
            ri.commit()?;
            // reindexer.printRefCounts();
            commit_close_num_docs = (1.25 * commit_close_num_docs as f64) as i32;
        }

        // Sometimes close & reopen writer/manager, to confirm the parallel segments persist:
        if random().next_int_bound(commit_close_num_docs) == 17 {
            if DEBUG {
                println!("TEST TOP: close writer @ {} docs", i + 1);
            }
            reindexer.take().expect("INVARIANT: open").close()?;
            commit_close_num_docs = (1.25 * commit_close_num_docs as f64) as i32;
        }
    }

    if let Some(reindexer) = reindexer {
        reindexer.close()?;
    }

    // Verify main index never reflects schema changes beyond mergingSchemaGen:
    let mut dir = LuceneTestCase::new_fs_directory(&root.resolve_str("index"));
    let r: Arc<dyn IndexReader> = crate::support::open_reader(&dir)?;
    let body = || -> Result<()> {
        for ctx in Arc::clone(&r).leaves()? {
            let leaf = ctx.leaf_reader();
            if let Some(mut numbers) = leaf.get_numeric_doc_values("number")? {
                let max_doc = leaf.max_doc();
                let stored_fields = leaf.stored_fields()?;
                for i in 0..max_doc {
                    let doc = stored_fields.document(i)?;
                    let value = number_of(&doc);
                    assert_equals(&i, &numbers.next_doc()?);
                    let dv_value = numbers.long_value()?;
                    if value == 0 {
                        assert_equals(&0, &dv_value);
                    } else {
                        Assert::assert_true(dv_value % value == 0);
                        Assert::assert_true(
                            dv_value / value <= merging_schema_gen.load(Ordering::SeqCst),
                        );
                    }
                }
            }
        }
        Ok(())
    };
    let outcome = body();
    // try-with-resources closes `r`, then `dir`.
    let closed_r = r.close();
    let closed_dir = dir.close();
    outcome?;
    closed_r?;
    closed_dir
}

/// `checkAllNumberDVs(r); IndexSearcher s = newSearcher(r);
/// testNumericDVSort(s); testPointRangeQuery(s);`
fn check_and_search(r: &Arc<dyn DirectoryReader>) -> Result<()> {
    check_all_number_dvs(r, "number", true, 1)?;
    let s = LuceneTestCase::new_searcher(Arc::clone(r));
    test_numeric_dv_sort(&s)?;
    test_point_range_query(&s)
}

/// Port of `TestDemoParallelLeafReader.testBasic`.
fn test_basic() -> Result<()> {
    let temp_path = LuceneTestCase::create_temp_dir();
    let reindexer = get_reindexer(&temp_path)?;

    // Start with initial empty commit:
    reindexer.commit()?;

    let doc = text_doc(random().next_long());
    reindexer.w.add_document(&doc)?;

    if DEBUG {
        println!("TEST: refresh @ 1 doc");
    }
    reindexer.mgr.maybe_refresh()?;
    with_acquired(&reindexer.mgr, check_and_search)?;
    // reindexer.printRefCounts();

    if DEBUG {
        println!("TEST: commit");
    }
    reindexer.commit()?;

    let doc = text_doc(random().next_long());
    reindexer.w.add_document(&doc)?;

    if DEBUG {
        println!("TEST: refresh @ 2 docs");
    }
    reindexer.mgr.maybe_refresh()?;
    // reindexer.printRefCounts();
    with_acquired(&reindexer.mgr, check_and_search)?;

    if DEBUG {
        println!("TEST: forceMerge");
    }
    reindexer.w.force_merge(1)?;

    if DEBUG {
        println!("TEST: commit");
    }
    reindexer.commit()?;

    if DEBUG {
        println!("TEST: refresh after forceMerge");
    }
    reindexer.mgr.maybe_refresh()?;
    with_acquired(&reindexer.mgr, check_and_search)?;

    if DEBUG {
        println!("TEST: close writer");
    }
    reindexer.close()
}

/// Port of `TestDemoParallelLeafReader.testRandom`.
fn test_random() -> Result<()> {
    let root = LuceneTestCase::create_temp_dir();
    let mut reindexer: Option<ReindexingReader> = None;

    // TODO: separate refresh thread, search threads, indexing threads
    let num_docs = LuceneTestCase::at_least(if *TEST_NIGHTLY { 20000 } else { 200 });
    let mut max_id = 0;
    let mut refresh_every_num_docs = 100;
    let mut commit_close_num_docs = 1000;
    for i in 0..num_docs {
        if reindexer.is_none() {
            reindexer = Some(get_reindexer(&root)?);
        }
        let ri = reindexer.as_ref().expect("INVARIANT: opened above");

        let (id, update_id) = next_id(&mut max_id);
        let doc = id_text_doc(&id, random().next_long());
        add_or_update(&ri.w, &doc, update_id)?;

        if random().next_int_bound(refresh_every_num_docs) == 17 {
            if DEBUG {
                println!("TEST: refresh @ {} docs", i + 1);
            }
            ri.mgr.maybe_refresh()?;
            with_acquired(&ri.mgr, check_and_search)?;
            refresh_every_num_docs = (1.25 * refresh_every_num_docs as f64) as i32;
        }

        if i > 0 && random().next_int_bound(10) == 7 {
            // Random delete:
            ri.w.delete_documents_by_terms(vec![Term::from_text(
                "id",
                &random().next_int_bound(i).to_string(),
            )])?;
        }

        if random().next_int_bound(commit_close_num_docs) == 17 {
            if DEBUG {
                println!("TEST: commit @ {} docs", i + 1);
            }
            ri.commit()?;
            commit_close_num_docs = (1.25 * commit_close_num_docs as f64) as i32;
        }

        // Sometimes close & reopen writer/manager, to confirm the parallel segments persist:
        if random().next_int_bound(commit_close_num_docs) == 17 {
            if DEBUG {
                println!("TEST: close writer @ {} docs", i + 1);
            }
            reindexer.take().expect("INVARIANT: open").close()?;
            commit_close_num_docs = (1.25 * commit_close_num_docs as f64) as i32;
        }
    }
    if let Some(reindexer) = reindexer {
        reindexer.close()?;
    }
    Ok(())
}

/// `private static void checkAllNumberDVs(IndexReader r, String fieldName,
/// boolean doThrow, int multiplier)`.
fn check_all_number_dvs(
    r: &Arc<dyn DirectoryReader>,
    field_name: &str,
    do_throw: bool,
    multiplier: i32,
) -> Result<()> {
    let r: Arc<dyn IndexReader> = Arc::clone(r) as Arc<dyn IndexReader>;
    check_all_number_dvs_of(&r, false, field_name, do_throw, multiplier)
}

/// The body of `checkAllNumberDVs` over any reader; `is_leaf` is `r
/// instanceof LeafReader`.
fn check_all_number_dvs_of(
    r: &Arc<dyn IndexReader>,
    is_leaf: bool,
    field_name: &str,
    do_throw: bool,
    multiplier: i32,
) -> Result<()> {
    let mut numbers = MultiDocValues::get_numeric_values(r, field_name)?
        .unwrap_or_else(|| crate::support::npe("MultiDocValues.getNumericValues"));
    let max_doc = r.max_doc();
    let stored_fields = r.stored_fields()?;
    let mut failed = false;
    for i in 0..max_doc {
        let old_doc = stored_fields.document(i)?;
        let value = i64::from(multiplier).wrapping_mul(number_of(&old_doc));
        assert_equals(&i, &numbers.next_doc()?);
        if value != numbers.long_value()? {
            println!(
                "FAIL: docID={i} {old_doc:?} value={value} number={}",
                numbers.long_value()?
            );
            failed = true;
        } else if failed {
            println!(
                "OK: docID={i} {old_doc:?} value={value} number={}",
                numbers.long_value()?
            );
        }
    }
    if failed {
        if !is_leaf {
            println!("TEST FAILED; check leaves");
            for ctx in Arc::clone(r).leaves()? {
                println!("CHECK LEAF={}", ctx.leaf_reader());
                let leaf: Arc<dyn IndexReader> = ctx.leaf_reader().as_index_reader();
                check_all_number_dvs_of(&leaf, true, field_name, false, 1)?;
            }
        }
        if do_throw {
            Assert::assert_false_msg(&format!("FAILED field={field_name} r={r}"), failed);
        } else {
            println!("FAILED field={field_name} r={r}");
        }
    }
    Ok(())
}

/// `private static void testNumericDVSort(IndexSearcher s)`.
fn test_numeric_dv_sort(s: &IndexSearcher) -> Result<()> {
    // Confirm we can sort by the new DV field:
    let hits = s.search_with_sort(
        Arc::new(MatchAllDocsQuery::instance()),
        100,
        &Sort::new_fields(vec![SortField::new(
            Some("number".to_string()),
            SortFieldType::Long,
        )?])?,
    )?;
    let stored_fields = s.stored_fields()?;
    let mut last = i64::MIN;
    for score_doc in &hits.score_docs {
        let value = number_of(&stored_fields.document(score_doc.score_doc.doc)?);
        Assert::assert_true(value >= last);
        let field = score_doc
            .fields
            .as_ref()
            .and_then(|f| f.first())
            .unwrap_or_else(|| crate::support::npe("((FieldDoc) scoreDoc).fields[0]"));
        match field {
            SortValue::Long(v) => assert_equals(&value, v),
            other => Throwable::new(
                "java.lang.ClassCastException",
                format!("{other:?} cannot be cast to class java.lang.Long"),
            )
            .throw(),
        }
        last = value;
    }
    Ok(())
}

/// `private static void testPointRangeQuery(IndexSearcher s)`.
fn test_point_range_query(s: &IndexSearcher) -> Result<()> {
    for _ in 0..100 {
        // Confirm we can range search by the new indexed (numeric) field:
        let mut min = random().next_long();
        let mut max = random().next_long();
        if min > max {
            std::mem::swap(&mut min, &mut max);
        }

        let mut hits = s
            .search(LongPoint::new_range_query("number", &[min], &[max])?, 100)?
            .score_docs;
        let stored_fields = s.stored_fields()?;
        for score_doc in &hits {
            let value = number_of(&stored_fields.document(score_doc.doc)?);
            Assert::assert_true(value >= min);
            Assert::assert_true(value <= max);
        }

        hits.sort_by_key(|a| a.doc);

        let mut numbers = MultiDocValues::get_numeric_values(s.get_index_reader(), "number")?
            .unwrap_or_else(|| crate::support::npe("MultiDocValues.getNumericValues"));
        for hit in &hits {
            if numbers.doc_id() < hit.doc {
                numbers.advance(hit.doc)?;
            }
            assert_equals(&hit.doc, &numbers.doc_id());
            let value = number_of(&stored_fields.document(hit.doc)?);
            assert_equals(&value, &numbers.long_value()?);
        }
    }
    Ok(())
}

/// The file name of `path`, as a string.
fn file_name_of(path: &Path) -> String {
    path.get_file_name()
        .map(|n| n.to_string())
        .unwrap_or_default()
}

/// `SEG_GEN_SUB_DIR_PATTERN.matcher(name).matches()`, the pattern
/// `^[a-z0-9]+_([0-9]+)$`.
// TODO: maybe the leading id could be further restricted?  It's from StringHelper.idToString:
fn matches_seg_gen_sub_dir(name: &str) -> bool {
    let Some((id, gen)) = name.split_once('_') else {
        return false;
    };
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && !gen.is_empty()
        && gen.bytes().all(|b| b.is_ascii_digit())
}

/// `private static List<Path> segSubDirs(Path segsPath)`.
fn seg_sub_dirs(segs_path: &Path) -> Result<Vec<Path>> {
    let mut result = Vec::new();
    let mut stream = Files::new_directory_stream(segs_path)?;
    for path in stream.iterator() {
        let path = path?;
        // Must be form <segIDString>_<longGen>
        if Files::is_directory(&path, &[]) && matches_seg_gen_sub_dir(&file_name_of(&path)) {
            result.push(path);
        }
    }
    stream.close()?;

    Ok(result)
}

// TODO: test exceptions
