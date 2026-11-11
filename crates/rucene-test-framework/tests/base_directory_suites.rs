//! The `BaseDirectoryTestCase` and `BaseChunkedDirectoryTestCase` suites
//! (rmp #350): the inherited cases run, under the ported runner and
//! `LuceneTestCase`'s rule chain, on the directories Lucene's own subclasses
//! bind (`lucene/core/src/test/org/apache/lucene/store/`):
//!
//! * `TestNRTCachingDirectory`, `TestTrackingDirectoryWrapper`,
//!   `TestMMapDirectory`, `TestNIOFSDirectory`, `TestFilterDirectory`,
//!   `TestFileSwitchDirectory` (with its override of `testNoDir()` and the
//!   `newFSSwitchDirectory` helpers it calls) and `TestByteBuffersDirectory`
//!   (its `@ParametersFactory`: the three output-to-input variants);
//! * `TestMultiByteBuffersDirectory` and `TestMultiMMap`
//!   (`BaseChunkedDirectoryTestCase`).
//!
//! Each binding also declares its class's own test methods (core-test port,
//! rmp #398): `TestNRTCachingDirectory`, `TestTrackingDirectoryWrapper`,
//! `TestMMapDirectory`, `TestNIOFSDirectory`, `TestFilterDirectory`,
//! `TestFileSwitchDirectory`,
//! `TestMultiMMap` and `TestByteBuffersDirectory`.
//!
//! `RUCENE_SUITE_SEEDS=<n>` runs each suite on the first `n` seeds of
//! `suite_seeds::SEEDS`; `test_*_suite_draws_match_lucene` compare the
//! seeded draws with `BaseSuitesDrawsOracle`'s.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use randomizedtesting::annotations::ParametersFactory;
use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method, Value};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::store::java_nio_file::Path;
use rucene::store::mmap::MMapDirectory;
use rucene::store::{
    output_as_byte_array, output_as_many_buffers, output_as_one_buffer, ByteBuffersDataOutput,
    ByteBuffersDirectory, Directory, FileSwitchDirectory, FilterDirectory, NIOFSDirectory,
    NRTCachingDirectory, NativeFSLockFactory, OutputToInputFn, SingleInstanceLockFactory,
    TrackingDirectoryWrapper, LIMIT_MIN_BITS_PER_BLOCK,
};
use rucene::util::BitUtil;
use rucene_test_framework::store::base_chunked_directory_test_case::{
    self, chunked_get_directory, BaseChunkedDirectoryTestCase,
};
use rucene_test_framework::store::base_directory_test_case::{
    self, close_shared, directory_test, BaseDirectoryTestCase, DirectoryTestInstance,
};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

mod draw_hooks;
mod lucene_core_shims;
mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

use lucene_core_shims::{jbytes, CountDownLatch};
use randomizedtesting::jdk::random::Random;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::index_writer::{IndexWriter, MAX_TERM_LENGTH};
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::Term;
use rucene::search::term_query::TermQuery;
use rucene::store::{
    flush_io_context, merge_io_context, DataAccessHint, DefaultIOContext, FSDirectory,
    FileDataHint, FileOpenHint, FileTypeHint, FlushInfo, IOContext, IndexInput, MergeInfo,
    PreloadHint, ReadAdvice, ReadOnceHint, DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT,
};
use rucene::util::{Accountable, BytesRef, Constants};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::base_index_file_format_test_case::assert_array_equals;
use rucene_test_framework::index::doc_helper::DocHelper;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::mockfile::filter_file_channel::FileChannelOverrides;
use rucene_test_framework::mockfile::windows_fs::WindowsFS;
use rucene_test_framework::mockfile::{
    FilterFileChannel, FilterFileSystemProvider, LeakFS, LeakFSOverrides,
};
use rucene_test_framework::store::base_directory_test_case::OwnedTestDirectory;
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::java_exceptions::{
    expect_throws_any_of, is_instance, java_throwable,
};
use rucene_test_framework::util::lucene_test_case::{RANDOM_MULTIPLIER, TEST_NIGHTLY, VERBOSE};
use rucene_test_framework::util::{English, LineFileDocs, TestUtil};

/// `TestNRTCachingDirectory`: `new NRTCachingDirectory(new
/// ByteBuffersDirectory(), .1 + 2.0 * random().nextDouble(), .1 + 5.0 *
/// random().nextDouble())`.
struct TestNRTCachingDirectory;

impl BaseDirectoryTestCase for TestNRTCachingDirectory {
    fn get_directory(&self, _path: &Path) -> Result<Box<dyn Directory>> {
        let inner: Box<dyn Directory> = Box::new(ByteBuffersDirectory::new());
        let max_merge_size_mb = 0.1 + 2.0 * LuceneTestCase::random().next_double();
        let max_cached_mb = 0.1 + 5.0 * LuceneTestCase::random().next_double();
        Ok(Box::new(NRTCachingDirectory::new(
            inner,
            max_merge_size_mb,
            max_cached_mb,
        )))
    }
}

/// `TestTrackingDirectoryWrapper`: `new TrackingDirectoryWrapper(new
/// ByteBuffersDirectory())`.
struct TestTrackingDirectoryWrapper;

impl BaseDirectoryTestCase for TestTrackingDirectoryWrapper {
    fn get_directory(&self, _path: &Path) -> Result<Box<dyn Directory>> {
        Ok(Box::new(TrackingDirectoryWrapper::new(Box::new(
            ByteBuffersDirectory::new(),
        ))))
    }
}

/// `TestMMapDirectory`: `new MMapDirectory(path)` preloading each file on a
/// coin flip.
struct TestMMapDirectory;

impl BaseDirectoryTestCase for TestMMapDirectory {
    fn get_directory(&self, path: &Path) -> Result<Box<dyn Directory>> {
        let m = MMapDirectory::open(path)?;
        m.set_preload(Box::new(|_file, _context| {
            LuceneTestCase::random().next_boolean()
        }));
        Ok(Box::new(m))
    }
}

/// `TestNIOFSDirectory`: `new NIOFSDirectory(path)`.
struct TestNIOFSDirectory;

impl BaseDirectoryTestCase for TestNIOFSDirectory {
    fn get_directory(&self, path: &Path) -> Result<Box<dyn Directory>> {
        Ok(Box::new(NIOFSDirectory::open(path)?))
    }
}

/// `TestFilterDirectory`: `new FilterDirectory(new ByteBuffersDirectory())
/// {}`.
struct TestFilterDirectory;

impl BaseDirectoryTestCase for TestFilterDirectory {
    fn get_directory(&self, _path: &Path) -> Result<Box<dyn Directory>> {
        Ok(Box::new(FilterDirectory::new(Box::new(
            ByteBuffersDirectory::new(),
        ))))
    }
}

/// `TestFileSwitchDirectory`.
struct TestFileSwitchDirectory;

impl TestFileSwitchDirectory {
    /// `newFSSwitchDirectory(Set<String> primaryExtensions)` (`private`).
    fn new_fs_switch_directory(primary_extensions: HashSet<String>) -> Result<Box<dyn Directory>> {
        let prim_dir = LuceneTestCase::create_temp_dir_prefix("foo");
        let second_dir = LuceneTestCase::create_temp_dir_prefix("bar");
        Self::new_fs_switch_directory_in(&prim_dir, &second_dir, primary_extensions)
    }

    /// `newFSSwitchDirectory(Path aDir, Path bDir, Set<String>
    /// primaryExtensions)` (`private`).
    fn new_fs_switch_directory_in(
        a_dir: &Path,
        b_dir: &Path,
        primary_extensions: HashSet<String>,
    ) -> Result<Box<dyn Directory>> {
        let a: Box<dyn Directory> = Box::new(NIOFSDirectory::open(a_dir)?);
        let b: Box<dyn Directory> = Box::new(NIOFSDirectory::open(b_dir)?);
        Ok(Box::new(FileSwitchDirectory::new(
            primary_extensions,
            a,
            b,
            true,
        )?))
    }

    /// `testNoDir()` (`@Override`): LUCENE-3380 -- make sure we get
    /// exception if the directory really does not exist.
    fn test_no_dir_override() -> Result<()> {
        let prim_dir = LuceneTestCase::create_temp_dir_prefix("foo");
        let second_dir = LuceneTestCase::create_temp_dir_prefix("bar");
        let dir: Arc<dyn Directory> = Arc::from(Self::new_fs_switch_directory_in(
            &prim_dir,
            &second_dir,
            HashSet::new(),
        )?);
        expect_throws("org.apache.lucene.index.IndexNotFoundException", || {
            directory_reader::open(Arc::clone(&dir))
        });
        close_shared(dir)
    }
}

impl BaseDirectoryTestCase for TestFileSwitchDirectory {
    fn get_directory(&self, _path: &Path) -> Result<Box<dyn Directory>> {
        let mut extensions = HashSet::new();
        for ext in ["cfs", "prx", "frq", "tip", "tim", "del"] {
            if LuceneTestCase::random().next_boolean() {
                extensions.insert(ext.to_string());
            }
        }
        Self::new_fs_switch_directory(extensions)
    }
}

/// `TestByteBuffersDirectory`, built by its `@ParametersFactory` with one of
/// the three suppliers (`implSupplier`).
struct TestByteBuffersDirectory {
    output_to_input: fn() -> OutputToInputFn,
}

impl BaseDirectoryTestCase for TestByteBuffersDirectory {
    fn get_directory(&self, _path: &Path) -> Result<Box<dyn Directory>> {
        Ok(Box::new(ByteBuffersDirectory::with_config(
            Box::new(SingleInstanceLockFactory::new()),
            Arc::new(ByteBuffersDataOutput::new),
            (self.output_to_input)(),
        )))
    }
}

/// `TestMultiByteBuffersDirectory extends BaseChunkedDirectoryTestCase`.
struct TestMultiByteBuffersDirectory;

impl BaseDirectoryTestCase for TestMultiByteBuffersDirectory {
    fn get_directory(&self, path: &Path) -> Result<Box<dyn Directory>> {
        chunked_get_directory(self, path)
    }

    fn as_chunked(&self) -> Option<&dyn BaseChunkedDirectoryTestCase> {
        Some(self)
    }
}

impl BaseChunkedDirectoryTestCase for TestMultiByteBuffersDirectory {
    fn get_directory_chunked(
        &self,
        _path: &Path,
        max_chunk_size: i32,
    ) -> Result<Box<dyn Directory>> {
        // round down huge values (above 20) to keep RAM usage low in tests (especially in nightly)
        let bits_per_block = 20.min(
            (LIMIT_MIN_BITS_PER_BLOCK as i32)
                .max(BitUtil::next_highest_power_of_two(max_chunk_size).trailing_zeros() as i32),
        ) as usize;
        Ok(Box::new(ByteBuffersDirectory::with_config(
            Box::new(SingleInstanceLockFactory::new()),
            Arc::new(move || {
                ByteBuffersDataOutput::with_config(bits_per_block, bits_per_block, false)
                    .unwrap_or_else(|e| {
                        rucene_test_framework::util::java_exceptions::rethrow_java(e)
                    })
            }),
            output_as_many_buffers(),
        )))
    }
}

/// `TestMultiMMap extends BaseChunkedDirectoryTestCase`: `new
/// MMapDirectory(path, maxChunkSize)`.
struct TestMultiMMap;

impl BaseDirectoryTestCase for TestMultiMMap {
    fn get_directory(&self, path: &Path) -> Result<Box<dyn Directory>> {
        chunked_get_directory(self, path)
    }

    fn as_chunked(&self) -> Option<&dyn BaseChunkedDirectoryTestCase> {
        Some(self)
    }
}

impl BaseChunkedDirectoryTestCase for TestMultiMMap {
    fn get_directory_chunked(
        &self,
        path: &Path,
        max_chunk_size: i32,
    ) -> Result<Box<dyn Directory>> {
        Ok(Box::new(MMapDirectory::with_options(
            path,
            Box::new(NativeFSLockFactory),
            max_chunk_size as usize,
        )?))
    }
}

/// A binding class of `BaseChunkedDirectoryTestCase`.
fn chunked_binding(name: &str, make: fn() -> DirectoryTestInstance) -> Arc<Class> {
    Class::builder(name)
        .extends(&base_chunked_directory_test_case::class())
        .constructor(Constructor::no_args(make))
        .build()
}

/// A binding class of `BaseDirectoryTestCase` with its own methods.
fn binding_with(
    name: &str,
    make: fn() -> DirectoryTestInstance,
    methods: Vec<randomizedtesting::jdk::reflect::Method>,
) -> Arc<Class> {
    let mut builder = Class::builder(name)
        .extends(&base_directory_test_case::class())
        .constructor(Constructor::no_args(make));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// A test method of a binding that does not reach its instance.
fn own(name: &str, body: fn() -> Result<()>) -> randomizedtesting::jdk::reflect::Method {
    directory_test(name, move |_| body())
}

fn nrt_caching_class() -> Arc<Class> {
    binding_with(
        "org.apache.lucene.store.TestNRTCachingDirectory",
        || DirectoryTestInstance::new(TestNRTCachingDirectory),
        vec![
            own("testNRTAndCommit", nrt_test_nrt_and_commit),
            // "NOTE: not a test": the method provider does not run it.
            own("verifyCompiles", nrt_verify_compiles),
            own(
                "testCreateTempOutputSameName",
                nrt_test_create_temp_output_same_name,
            ),
            own("testUnknownFileSize", nrt_test_unknown_file_size),
            own("testCacheSizeAfterDelete", nrt_test_cache_size_after_delete),
        ],
    )
}

fn tracking_class() -> Arc<Class> {
    binding_with(
        "org.apache.lucene.store.TestTrackingDirectoryWrapper",
        || DirectoryTestInstance::new(TestTrackingDirectoryWrapper),
        vec![
            own("testTrackEmpty", tracking_test_track_empty),
            own("testTrackCreate", tracking_test_track_create),
            own("testTrackDelete", tracking_test_track_delete),
            own("testTrackRename", tracking_test_track_rename),
            own("testTrackCopyFrom", tracking_test_track_copy_from),
        ],
    )
}

fn mmap_class() -> Arc<Class> {
    binding_with(
        "org.apache.lucene.store.TestMMapDirectory",
        || DirectoryTestInstance::new(TestMMapDirectory),
        vec![
            directory_test("testAceWithThreads", mmap_test_ace_with_threads),
            directory_test(
                "testNullParamsIndexInput",
                mmap_test_null_params_index_input,
            ),
            own("testMadviseAvail", mmap_test_madvise_avail),
            own("testWithNormal", mmap_test_with_normal),
            own("testAdviseByContextFunc", mmap_test_advise_by_context_func),
            own("testPreload", mmap_test_preload),
            own("testConfined", mmap_test_confined),
            own("testArenas", mmap_test_arenas),
            own(
                "testArenasManySegmentFiles",
                mmap_test_arenas_many_segment_files,
            ),
            own("testGroupBySegmentFunc", mmap_test_group_by_segment_func),
            own("testNoGroupingFunc", mmap_test_no_grouping_func),
            own(
                "testPrefetchWithSingleSegment",
                mmap_test_prefetch_with_single_segment,
            ),
            own(
                "testPrefetchWithMultiSegment",
                mmap_test_prefetch_with_multi_segment,
            ),
        ],
    )
}

fn niofs_class() -> Arc<Class> {
    binding_with(
        "org.apache.lucene.store.TestNIOFSDirectory",
        || DirectoryTestInstance::new(TestNIOFSDirectory),
        vec![own(
            "testHandleExceptionInConstructor",
            niofs_test_handle_exception_in_constructor,
        )],
    )
}

fn filter_class() -> Arc<Class> {
    binding_with(
        "org.apache.lucene.store.TestFilterDirectory",
        || DirectoryTestInstance::new(TestFilterDirectory),
        vec![
            own("testOverrides", filter_test_overrides),
            own("testUnwrap", filter_test_unwrap),
        ],
    )
}

fn file_switch_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.store.TestFileSwitchDirectory")
        .extends(&base_directory_test_case::class())
        .constructor(Constructor::no_args(|| {
            DirectoryTestInstance::new(TestFileSwitchDirectory)
        }))
        .method(directory_test("testNoDir", |_| {
            TestFileSwitchDirectory::test_no_dir_override()
        }))
        .method(own("testBasic", file_switch_test_basic))
        .method(directory_test(
            "testRenameTmpFile",
            file_switch_test_rename_tmp_file,
        ))
        .method(own("testDeleteAndList", file_switch_test_delete_and_list))
        .build()
}

/// The suppliers of `TestByteBuffersDirectory.parametersWithCustomName()`,
/// with their names.
const BYTE_BUFFERS_SUPPLIERS: [(fn() -> OutputToInputFn, &str); 3] = [
    (output_as_many_buffers, "many buffers (heap)"),
    (output_as_one_buffer, "one buffer (heap)"),
    (output_as_byte_array, "byte array (heap)"),
];

fn byte_buffers_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.store.TestByteBuffersDirectory")
        .extends(&base_directory_test_case::class())
        .constructor(Constructor::with_params(
            &[
                ("java.util.function.Supplier", None),
                ("java.lang.String", None),
            ],
            |args: &[Value]| {
                let index = match args.first() {
                    Some(Value::Int(i)) => *i as usize,
                    _ => 0,
                };
                DirectoryTestInstance::new(TestByteBuffersDirectory {
                    output_to_input: BYTE_BUFFERS_SUPPLIERS[index].0,
                })
            },
        ))
        .method(
            Method::new("parametersWithCustomName")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .returns("java.lang.Iterable<java.lang.Object[]>")
                .annotate(ParametersFactory {
                    argument_formatting: "impl=%2$s".to_string(),
                    shuffle: true,
                })
                .static_value(|_| {
                    Value::List(
                        BYTE_BUFFERS_SUPPLIERS
                            .iter()
                            .enumerate()
                            .map(|(i, (_, name))| {
                                Value::Array(vec![
                                    Value::Int(i as i32),
                                    Value::Str(name.to_string()),
                                ])
                            })
                            .collect(),
                    )
                }),
        )
        .method(
            directory_test("testBuildIndex", byte_buffers_test_build_index)
                .annotate(randomizedtesting::junit::Test::new()),
        )
        .build()
}

fn multi_byte_buffers_class() -> Arc<Class> {
    chunked_binding(
        "org.apache.lucene.store.TestMultiByteBuffersDirectory",
        || DirectoryTestInstance::new(TestMultiByteBuffersDirectory),
    )
}

fn multi_mmap_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.store.TestMultiMMap")
        .extends(&base_chunked_directory_test_case::class())
        .constructor(Constructor::no_args(|| {
            DirectoryTestInstance::new(TestMultiMMap)
        }))
        .method(directory_test(
            "testSeekingExceptions",
            multi_mmap_test_seeking_exceptions,
        ))
        .method(directory_test(
            "testCloneSafety",
            multi_mmap_test_clone_safety,
        ))
        .method(directory_test(
            "testCloneSliceSafety",
            multi_mmap_test_clone_slice_safety,
        ))
        .method(directory_test(
            "testImplementations",
            multi_mmap_test_implementations,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// The classes' own methods (core-test port, rmp #398)
// -----------------------------------------------------------------------------

fn rnd() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// `TestNRTCachingDirectory.testNRTAndCommit()`.
fn nrt_test_nrt_and_commit() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let cached_dir: Arc<dyn Directory> = Arc::new(NRTCachingDirectory::new(
        Box::new(SharedDirectory(dir.as_directory())),
        2.0,
        25.0,
    ));
    let analyzer = MockAnalyzer::new(&*rnd());
    analyzer.set_max_token_length(TestUtil::next_int(&*rnd(), 1, MAX_TERM_LENGTH as i32));
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer)));
    let w = RandomIndexWriter::new_with_config(&*rnd(), Arc::clone(&cached_dir), conf)?;
    let docs = LineFileDocs::with_default_file(&*rnd())?;
    let num_docs = TestUtil::next_int(&*rnd(), 100, 400);

    if *VERBOSE {
        println!("TEST: numDocs={num_docs}");
    }

    let mut ids: Vec<BytesRef> = Vec::new();
    let mut r: Option<Arc<dyn DirectoryReader>> = None;
    for doc_count in 0..num_docs {
        let doc = docs.next_doc()?;
        let doc = doc.borrow();
        ids.push(BytesRef::from_char_sequence(
            &doc.get("docid").expect("docid"),
        ));
        w.add_document(&*doc)?;
        if rnd().next_int_bound(20) == 17 {
            match r.take() {
                None => r = Some(directory_reader::open_with_writer(&**w.w())?),
                Some(old) => {
                    let r2 = directory_reader::open_if_changed(Arc::clone(&old))?;
                    match r2 {
                        Some(r2) => {
                            old.close()?;
                            r = Some(r2);
                        }
                        None => r = Some(old),
                    }
                }
            }
            let reader = r.as_ref().unwrap();
            Assert::assert_equals(1 + doc_count, reader.num_docs());
            let s = LuceneTestCase::new_searcher(Arc::clone(reader));
            // Just make sure search can run; we can't assert
            // totHits since it could be 0
            s.search(Arc::new(TermQuery::new(Term::from_text("body", "the"))), 10)?;
            // System.out.println("tot hits " + hits.totalHits);
        }
    }

    if let Some(r) = r.take() {
        r.close()?;
    }

    // Close should force cache to clear since all files are sync'd
    w.close()?;

    let cached_files = cached_dir
        .as_any()
        .and_then(|a| a.downcast_ref::<NRTCachingDirectory>())
        .expect("an NRTCachingDirectory")
        .list_cached_files()?;
    for file in &cached_files {
        println!("FAIL: cached file {file} remains after sync");
    }
    Assert::assert_equals(0usize, cached_files.len());

    let r = directory_reader::open(dir.as_directory())?;
    for id in &ids {
        Assert::assert_equals(1, r.doc_freq(&Term::new("docid", id.clone()))?);
    }
    r.close()?;
    close_shared(cached_dir)?;
    docs.close();
    Ok(())
}

/// `TestNRTCachingDirectory.verifyCompiles()`: "NOTE: not a test; just here
/// to make sure the code frag in the javadocs is correct!"
fn nrt_verify_compiles() -> Result<()> {
    let analyzer: Option<Arc<dyn rucene::analysis::Analyzer>> = None;
    let fs_dir = FSDirectory::open(LuceneTestCase::create_temp_dir_prefix("verify"))?;
    let cached_fs_dir: Arc<dyn Directory> =
        Arc::new(NRTCachingDirectory::new(Box::new(fs_dir), 2.0, 25.0));
    let conf = match analyzer {
        Some(a) => IndexWriterConfig::with_analyzer(Some(a)),
        None => IndexWriterConfig::new(),
    };
    let writer = IndexWriter::new(Arc::clone(&cached_fs_dir), conf)?;
    writer.close()?;
    close_shared(cached_fs_dir)
}

/// `TestNRTCachingDirectory.testCreateTempOutputSameName()`.
fn nrt_test_create_temp_output_same_name() -> Result<()> {
    let fs_dir = FSDirectory::open(LuceneTestCase::create_temp_dir_prefix("verify"))?;
    let nrt_dir = NRTCachingDirectory::new(Box::new(fs_dir), 2.0, 25.0);
    let name = "foo_bar_0.tmp";
    nrt_dir.create_output(name, &*DEFAULT_IO_CONTEXT)?.close()?;

    let mut out = nrt_dir.create_temp_output("foo", "bar", &*DEFAULT_IO_CONTEXT)?;
    Assert::assert_false(name == out.name());
    out.close()?;
    drop(out);
    // `nrtDir.close()` closes the wrapped `fsDir` too; `fsDir.close()` is
    // then a no-op.
    nrt_dir.close()
}

/// `TestNRTCachingDirectory.testUnknownFileSize()`.
///
/// The two anonymous `NRTCachingDirectory` subclasses overriding
/// `doCacheWrite` are [`NRTCachingDirectory::with_do_cache_write`]; both wrap
/// the same `dir`, shared through [`SharedDirectory`]. As in Java, neither
/// is closed.
fn nrt_test_unknown_file_size() -> Result<()> {
    let dir: Arc<dyn Directory> =
        Arc::new(OwnedTestDirectory::new(LuceneTestCase::new_directory()));

    let nrt_dir1 = NRTCachingDirectory::with_do_cache_write(
        Box::new(SharedDirectory(Arc::clone(&dir))),
        1.0,
        1.0,
        Arc::new(
            |this: &NRTCachingDirectory, name: &str, context: &dyn IOContext| {
                let cache = this.super_do_cache_write(name, context);
                Assert::assert_true(cache);
                cache
            },
        ),
    );
    let flush_context = flush_io_context(FlushInfo::new(3, 42));
    let mut io_context: &dyn IOContext = &*flush_context;
    nrt_dir1.create_output("foo", io_context)?.close()?;
    nrt_dir1
        .create_temp_output("bar", "baz", io_context)?
        .close()?;

    let nrt_dir2 = NRTCachingDirectory::with_do_cache_write(
        Box::new(SharedDirectory(Arc::clone(&dir))),
        1.0,
        1.0,
        Arc::new(
            |this: &NRTCachingDirectory, name: &str, context: &dyn IOContext| {
                let cache = this.super_do_cache_write(name, context);
                Assert::assert_false(cache);
                cache
            },
        ),
    );
    io_context = &*DEFAULT_IO_CONTEXT;
    nrt_dir2.create_output("foo", io_context)?.close()?;
    nrt_dir2
        .create_temp_output("bar", "baz", io_context)?
        .close()?;

    drop(nrt_dir1);
    drop(nrt_dir2);
    close_shared(dir)
}

/// `TestNRTCachingDirectory.testCacheSizeAfterDelete()`.
fn nrt_test_cache_size_after_delete() -> Result<()> {
    let io_context = flush_io_context(FlushInfo::new(3, 40));
    let fn_ = "f1";
    let dir = LuceneTestCase::new_directory();
    let nrt = NRTCachingDirectory::new(Box::new(OwnedTestDirectory::new(dir)), 1.0, 1.0);
    let result = (|| -> Result<()> {
        // deletes a closed file
        {
            let mut out = nrt.create_output(fn_, &*io_context)?;
            for i in 0..10 {
                out.write_int(i)?;
            }
            out.close()?;
        }
        Assert::assert_equals(40i64, nrt.ram_bytes_used());
        nrt.delete_file(fn_)?;
        Assert::assert_equals(0i64, nrt.ram_bytes_used());

        // Deletes an unclosed file (write before and after deletion
        {
            let mut out = nrt.create_output(fn_, &*io_context)?;
            for i in 0..10 {
                out.write_int(i)?;
            }
            nrt.delete_file(fn_)?;
            for i in 0..10 {
                out.write_int(i)?;
            }
            out.close()?;
        }
        Assert::assert_equals(0i64, nrt.ram_bytes_used());
        Ok(())
    })();
    result.and(nrt.close())
}

/// `TestTrackingDirectoryWrapper.testTrackEmpty()`.
fn tracking_test_track_empty() -> Result<()> {
    let dir = TrackingDirectoryWrapper::new(Box::new(ByteBuffersDirectory::new()));
    Assert::assert_true(dir.get_created_files().is_empty());
    Ok(())
}

/// `TestTrackingDirectoryWrapper.testTrackCreate()`.
fn tracking_test_track_create() -> Result<()> {
    let dir = TrackingDirectoryWrapper::new(Box::new(ByteBuffersDirectory::new()));
    dir.create_output("foo", &*LuceneTestCase::new_io_context(&*rnd()))?
        .close()?;
    Assert::assert_true(dir.get_created_files() == LuceneTestCase::as_set(["foo".to_string()]));
    Ok(())
}

/// `TestTrackingDirectoryWrapper.testTrackDelete()`.
fn tracking_test_track_delete() -> Result<()> {
    let dir = TrackingDirectoryWrapper::new(Box::new(ByteBuffersDirectory::new()));
    dir.create_output("foo", &*LuceneTestCase::new_io_context(&*rnd()))?
        .close()?;
    Assert::assert_true(dir.get_created_files() == LuceneTestCase::as_set(["foo".to_string()]));
    dir.delete_file("foo")?;
    Assert::assert_true(dir.get_created_files().is_empty());
    Ok(())
}

/// `TestTrackingDirectoryWrapper.testTrackRename()`.
fn tracking_test_track_rename() -> Result<()> {
    let dir = TrackingDirectoryWrapper::new(Box::new(ByteBuffersDirectory::new()));
    dir.create_output("foo", &*LuceneTestCase::new_io_context(&*rnd()))?
        .close()?;
    Assert::assert_true(dir.get_created_files() == LuceneTestCase::as_set(["foo".to_string()]));
    dir.rename("foo", "bar")?;
    Assert::assert_true(dir.get_created_files() == LuceneTestCase::as_set(["bar".to_string()]));
    Ok(())
}

/// `TestTrackingDirectoryWrapper.testTrackCopyFrom()`.
fn tracking_test_track_copy_from() -> Result<()> {
    let source = TrackingDirectoryWrapper::new(Box::new(ByteBuffersDirectory::new()));
    let dest = TrackingDirectoryWrapper::new(Box::new(ByteBuffersDirectory::new()));
    source
        .create_output("foo", &*LuceneTestCase::new_io_context(&*rnd()))?
        .close()?;
    Assert::assert_true(source.get_created_files() == LuceneTestCase::as_set(["foo".to_string()]));
    dest.copy_from(
        &source,
        "foo",
        "bar",
        &*LuceneTestCase::new_io_context(&*rnd()),
    )?;
    Assert::assert_true(dest.get_created_files() == LuceneTestCase::as_set(["bar".to_string()]));
    Assert::assert_true(source.get_created_files() == LuceneTestCase::as_set(["foo".to_string()]));
    Ok(())
}

/// `TestMMapDirectory.testAceWithThreads()`.
fn mmap_test_ace_with_threads(case: &dyn BaseDirectoryTestCase) -> Result<()> {
    let n_ints = 8 * 1024 * 1024;
    let dir = case.get_directory(&LuceneTestCase::create_temp_dir_prefix(
        "testAceWithThreads",
    ))?;
    let result = (|| -> Result<()> {
        {
            let mut out = dir.create_output("test", &*DEFAULT_IO_CONTEXT)?;
            let random = rnd();
            for _ in 0..n_ints {
                out.write_int(random.next_int())?;
            }
            out.close()?;
        }

        let iters = *RANDOM_MULTIPLIER * (if *TEST_NIGHTLY { 50 } else { 10 });
        for _ in 0..iters {
            let mut r#in = dir.open_input("test", &*DEFAULT_IO_CONTEXT)?;
            let mut clone = r#in.clone_input()?;
            let shotgun = Arc::new(CountDownLatch::new(1));
            let t1 = {
                let shotgun = Arc::clone(&shotgun);
                std::thread::spawn(move || {
                    let mut accum = vec![0u8; n_ints * 4];
                    shotgun.await_();
                    for _ in 0..10 {
                        let read = clone
                            .seek(0)
                            .and_then(|()| clone.read_bytes(Some(&mut accum), 0, n_ints * 4));
                        match read {
                            Ok(()) => {}
                            // OK
                            Err(e)
                                if is_instance(
                                    &e,
                                    "org.apache.lucene.store.AlreadyClosedException",
                                ) =>
                            {
                                return;
                            }
                            Err(e) => panic!("{}", java_throwable(&e)),
                        }
                    }
                })
            };
            shotgun.count_down();
            // this triggers "bad behaviour": closing input while other threads are running
            r#in.close()?;
            t1.join().expect("t1");
        }
        Ok(())
    })();
    result.and(dir.close())
}

/// `TestMMapDirectory.testNullParamsIndexInput()`.
fn mmap_test_null_params_index_input(case: &dyn BaseDirectoryTestCase) -> Result<()> {
    let mmap_dir = case.get_directory(&LuceneTestCase::create_temp_dir_prefix(
        "testNullParamsIndexInput",
    ))?;
    let result = (|| -> Result<()> {
        {
            let mut out =
                mmap_dir.create_output("bytes", &*LuceneTestCase::new_io_context(&*rnd()))?;
            let aligned = out.align_file_pointer(16).map(|_| ());
            aligned.and(out.close())?;
        }
        let mut r#in = mmap_dir.open_input("bytes", &*DEFAULT_IO_CONTEXT)?;
        expect_throws("java.lang.NullPointerException", || {
            r#in.read_bytes(None, 0, 1)
        });
        expect_throws("java.lang.NullPointerException", || {
            r#in.read_floats(None, 0, 1)
        });
        expect_throws("java.lang.NullPointerException", || {
            r#in.read_longs(None, 0, 1)
        });
        r#in.close()
    })();
    result.and(mmap_dir.close())
}

/// `TestMMapDirectory.testMadviseAvail()`.
fn mmap_test_madvise_avail() -> Result<()> {
    Assert::assert_equals_msg(
        Some("madvise should be supported on Linux and Macos (64 bit)"),
        Constants::jre_is_64bit() && (Constants::linux() || Constants::mac_os_x()),
        MMapDirectory::supports_madvise(),
    );
    Ok(())
}

/// `TestMMapDirectory.testWithNormal()`: `RANDOM` is the default (see
/// `Constants.DEFAULT_READADVICE`), so test with `NORMAL` too.
fn mmap_test_with_normal() -> Result<()> {
    let size = 8 * 1024;
    let mut bytes = vec![0u8; size];
    rnd().next_bytes(&mut bytes);

    let mut dir = MMapDirectory::open(LuceneTestCase::create_temp_dir_prefix("testWithRandom"))?;
    let result = (|| -> Result<()> {
        {
            let mut out = dir.create_output("test", &*DEFAULT_IO_CONTEXT)?;
            out.write_bytes(&bytes, 0, bytes.len())?;
            out.close()?;
        }
        dir.set_read_advice(Box::new(|_s, _c| Some(ReadAdvice::Normal)));
        let mut r#in = dir.open_input("test", &*DEFAULT_IO_CONTEXT)?;
        let mut read_bytes = vec![0u8; size];
        r#in.read_bytes(Some(&mut read_bytes), 0, size)?;
        assert_array_equals(&jbytes(&bytes), &jbytes(&read_bytes));
        r#in.close()
    })();
    result.and(dir.close())
}

/// `TestMMapDirectory.testAdviseByContextFunc()`.
fn mmap_test_advise_by_context_func() -> Result<()> {
    let func = MMapDirectory::advise_by_context();
    let flush = flush_io_context(FlushInfo::new(1, 2));
    let merge = merge_io_context(MergeInfo::new(1, 2, true, 4));
    let random = DefaultIOContext::new(vec![Arc::new(DataAccessHint::Random)])?;
    let sequential = DefaultIOContext::new(vec![Arc::new(DataAccessHint::Sequential)])?;

    Assert::assert_true(func("a", &*merge) == Some(ReadAdvice::Sequential));
    Assert::assert_true(func("b", &*flush) == Some(ReadAdvice::Sequential));
    Assert::assert_true(func("c", &sequential) == Some(ReadAdvice::Sequential));
    Assert::assert_true(func("d", &random) == Some(ReadAdvice::Random));

    // hint types other than DataAccessHint
    let hints: Vec<Arc<dyn FileOpenHint>> = vec![
        Arc::new(FileDataHint::Postings),
        Arc::new(FileDataHint::KnnVectors),
        Arc::new(FileTypeHint::Index),
        Arc::new(FileTypeHint::Data),
        Arc::new(PreloadHint::instance()),
        Arc::new(ReadOnceHint::instance()),
    ];
    for hint in &hints {
        let context = DefaultIOContext::new(vec![Arc::clone(hint)])?;
        Assert::assert_true(func("e", &context) == Some(Constants::default_read_advice()));
    }

    let mut contexts: Vec<Box<dyn IOContext>> = vec![
        flush,
        merge,
        Box::new(random),
        Box::new(sequential),
        Box::new(DEFAULT_IO_CONTEXT.clone()),
        Box::new(READONCE_IO_CONTEXT.clone()),
    ];
    for hint in &hints {
        contexts.push(Box::new(DefaultIOContext::new(vec![Arc::clone(hint)])?));
    }
    mmap_test_with_advise_by_context(&contexts)
}

/// `testWithAdviseByContext(List<IOContext>)`: trivially exercises
/// `MMapDirectory.ADVISE_BY_CONTEXT` with different contexts.
fn mmap_test_with_advise_by_context(contexts: &[Box<dyn IOContext>]) -> Result<()> {
    let size = 8 * 1024;
    let mut bytes = vec![0u8; size];
    rnd().next_bytes(&mut bytes);

    let mut dir = MMapDirectory::open(LuceneTestCase::create_temp_dir_prefix(
        "testWithAdviseByContext",
    ))?;
    let result = (|| -> Result<()> {
        {
            let mut out = dir.create_output("test", &*DEFAULT_IO_CONTEXT)?;
            out.write_bytes(&bytes, 0, bytes.len())?;
            out.close()?;
        }
        dir.set_read_advice(MMapDirectory::advise_by_context());
        for context in contexts {
            let mut r#in = dir.open_input("test", &**context)?;
            let mut read_bytes = vec![0u8; size];
            r#in.read_bytes(Some(&mut read_bytes), 0, size)?;
            assert_array_equals(&jbytes(&bytes), &jbytes(&read_bytes));
            r#in.close()?;
        }
        Ok(())
    })();
    result.and(dir.close())
}

/// `TestMMapDirectory.testPreload()`.
fn mmap_test_preload() -> Result<()> {
    LuceneTestCase::assume_true(
        "madvise for preloading only works on linux/mac",
        MMapDirectory::supports_madvise(),
    );
    let size = 8 * 1024;
    let mut bytes = vec![0u8; size];
    rnd().next_bytes(&mut bytes);

    let dir = MMapDirectory::open(LuceneTestCase::create_temp_dir_prefix("testPreload"))?;
    let result = (|| -> Result<()> {
        dir.set_preload(MMapDirectory::preload_hint());
        let preload: Arc<dyn FileOpenHint> = Arc::new(PreloadHint::instance());
        {
            let mut out = dir.create_output(
                "test",
                &*DEFAULT_IO_CONTEXT.with_hints(&[Arc::clone(&preload)]),
            )?;
            out.write_bytes(&bytes, 0, bytes.len())?;
            out.close()?;
        }
        let mut r#in = dir.open_input(
            "test",
            &*DEFAULT_IO_CONTEXT.with_hints(&[Arc::clone(&preload)]),
        )?;
        // the data should be loaded in memory
        Assert::assert_true(r#in.is_loaded().unwrap_or(false));
        let mut read_bytes = vec![0u8; size];
        r#in.read_bytes(Some(&mut read_bytes), 0, size)?;
        assert_array_equals(&jbytes(&bytes), &jbytes(&read_bytes));
        r#in.close()
    })();
    result.and(dir.close())
}

/// `ISE` (`static final Class<IllegalStateException>`).
const ISE: &str = "java.lang.IllegalStateException";

/// `TestMMapDirectory.testConfined()`: opens the input with
/// `ReadAdvice.READONCE` to ensure slice and clone are appropriately
/// confined.
///
/// **Divergence from Lucene 10.5.0.** Java submits each task to a
/// one-thread `ExecutorService` and unwraps the `ExecutionException`
/// (`getAndUnwrap`); each task here runs on a scoped thread whose result is
/// joined, the same "another thread" the confinement rejects.
fn mmap_test_confined() -> Result<()> {
    let size = 16;
    let mut bytes = vec![0u8; size];
    rnd().next_bytes(&mut bytes);

    let dir = MMapDirectory::open(LuceneTestCase::create_temp_dir_prefix("testConfined"))?;
    let result = (|| -> Result<()> {
        {
            let mut out = dir.create_output("test", &*DEFAULT_IO_CONTEXT)?;
            out.write_bytes(&bytes, 0, bytes.len())?;
            out.close()?;
        }
        let mut r#in = dir.open_input("test", &*READONCE_IO_CONTEXT)?;
        // ensure accessible
        Assert::assert_equals(16i64, r#in.slice("test", 0, r#in.length())?.length());
        Assert::assert_equals(15i64, r#in.slice("test", 1, r#in.length() - 1)?.length());

        // ensure not accessible
        let input: &dyn IndexInput = &*r#in;
        let x = expect_throws(ISE, || {
            get_and_unwrap(|| input.slice("test", 0, input.length()).map(drop))
        });
        Assert::assert_true(x.message().is_some_and(|m| m.contains("confined")));

        let offset = rnd().next_int_bound(input.length() as i32) as i64;
        let length = input.length() - offset;
        let x = expect_throws(ISE, || {
            get_and_unwrap(|| input.slice("test", offset, length).map(drop))
        });
        Assert::assert_true(x.message().is_some_and(|m| m.contains("confined")));

        // slice.slice
        let slice = input.slice("test", 0, input.length())?;
        let slice_ref: &dyn IndexInput = &*slice;
        let x = expect_throws(ISE, || {
            get_and_unwrap(|| slice_ref.slice("test", 0, input.length()).map(drop))
        });
        Assert::assert_true(x.message().is_some_and(|m| m.contains("confined")));

        // slice.clone
        let x = expect_throws(ISE, || get_and_unwrap(|| slice_ref.clone_input().map(drop)));
        Assert::assert_true(x.message().is_some_and(|m| m.contains("confined")));
        drop(slice);
        r#in.close()
    })();
    result.and(dir.close())
}

/// `getAndUnwrap(Future<Object>)` (`static`): runs `task` on another
/// thread and returns its outcome.
fn get_and_unwrap(task: impl FnOnce() -> Result<()> + Send) -> Result<()> {
    std::thread::scope(|s| {
        s.spawn(task)
            .join()
            .unwrap_or_else(|p| std::panic::resume_unwind(p))
    })
}

/// `TestMMapDirectory.testArenas()`.
fn mmap_test_arenas() -> Result<()> {
    let random_generation_or_none = || {
        if rnd().next_boolean() {
            format!("_{}", rnd().next_int_bound(5))
        } else {
            String::new()
        }
    };
    // First, create a number of segment specific file name lists to test with
    let exts = [
        ".si", ".cfs", ".cfe", ".dvd", ".dvm", ".nvd", ".nvm", ".fdt", ".vec", ".vex", ".vemf",
    ];
    let mut names: Vec<String> = Vec::new();
    for i in 0..50 {
        let s = format!("_{i}{}", random_generation_or_none());
        for ext in exts {
            names.push(format!("{s}{ext}"));
        }
    }
    // Second, create a number of non-segment file names
    for i in 0..50 {
        names.push(format!("foo{i}"));
    }
    randomizedtesting::jdk::util::shuffle(&mut names, &*rnd());

    let size = 6;
    let mut bytes = vec![0u8; size];
    rnd().next_bytes(&mut bytes);

    let dir = MMapDirectory::open(LuceneTestCase::create_temp_dir_prefix("testArenas"))?;
    let result = (|| -> Result<()> {
        for name in &names {
            let mut out = dir.create_output(name, &*DEFAULT_IO_CONTEXT)?;
            out.write_bytes(&bytes, 0, bytes.len())?;
            out.close()?;
        }

        let n_threads = 10;
        let per_list_size = (names.len() + n_threads) / n_threads;
        let name_lists: Vec<&[String]> = (0..n_threads)
            .map(|i| {
                &names[(per_list_size * i).min(names.len())
                    ..(per_list_size * i + per_list_size).min(names.len())]
            })
            .collect();

        let dir_ref: &dyn Directory = &dir;
        std::thread::scope(|s| -> Result<()> {
            let futures: Vec<_> = name_lists
                .iter()
                .map(|l| s.spawn(move || indices_open_task(l, dir_ref)))
                .collect();
            for future in futures {
                future
                    .join()
                    .unwrap_or_else(|p| std::panic::resume_unwind(p))?;
            }
            Ok(())
        })?;

        // `dir.attachment instanceof ConcurrentHashMap<?, ?> map`
        Assert::assert_equals(0usize, dir.shared_arena_count());
        Ok(())
    })();
    result.and(dir.close())
}

/// `IndicesOpenTask.call()` (`static class … implements Callable<Void>`).
fn indices_open_task(names: &[String], dir: &dyn Directory) -> Result<()> {
    let mut closeables: Vec<Box<dyn IndexInput>> = Vec::new();
    for name in names {
        closeables.push(dir.open_input(name, &*DEFAULT_IO_CONTEXT)?);
    }
    for closeable in &mut closeables {
        closeable.close()?;
    }
    Ok(())
}

/// `TestMMapDirectory.testArenasManySegmentFiles()`: opens more files in the
/// same group than the ref counting limit.
fn mmap_test_arenas_many_segment_files() -> Result<()> {
    let names: Vec<String> = (0..1024).map(|i| format!("_001.ext{i}")).collect();
    let size = 4;
    let mut bytes = vec![0u8; size];
    rnd().next_bytes(&mut bytes);

    let dir = MMapDirectory::open(LuceneTestCase::create_temp_dir_prefix(
        "testArenasManySegmentFiles",
    ))?;
    let result = (|| -> Result<()> {
        for name in &names {
            let mut out = dir.create_output(name, &*DEFAULT_IO_CONTEXT)?;
            out.write_bytes(&bytes, 0, bytes.len())?;
            out.close()?;
        }
        let mut closeables: Vec<Box<dyn IndexInput>> = Vec::new();
        for name in &names {
            closeables.push(dir.open_input(name, &*DEFAULT_IO_CONTEXT)?);
        }
        for closeable in &mut closeables {
            closeable.close()?;
        }
        // `dir.attachment instanceof ConcurrentHashMap<?, ?> map`
        Assert::assert_equals(0usize, dir.shared_arena_count());
        Ok(())
    })();
    result.and(dir.close())
}

/// `TestMMapDirectory.testGroupBySegmentFunc()`.
fn mmap_test_group_by_segment_func() -> Result<()> {
    let func = MMapDirectory::group_by_segment();
    let s = |n: &str| func(n).expect("orElseThrow");
    Assert::assert_equals_str(None, Some("0"), Some(&s("_0.doc")));
    Assert::assert_equals_str(None, Some("51"), Some(&s("_51.si")));
    Assert::assert_equals_str(None, Some("51-g"), Some(&s("_51_1.si")));
    Assert::assert_equals_str(None, Some("51-g"), Some(&s("_51_1_gg_ff.si")));
    Assert::assert_equals_str(None, Some("51-g"), Some(&s("_51_2_gg_ff.si")));
    Assert::assert_equals_str(None, Some("51-g"), Some(&s("_51_3_gg_ff.si")));
    Assert::assert_equals_str(None, Some("5987654321"), Some(&s("_5987654321.si")));
    Assert::assert_equals_str(None, Some("f"), Some(&s("_f.si")));
    Assert::assert_equals_str(None, Some("ff"), Some(&s("_ff.si")));
    Assert::assert_equals_str(None, Some("51a"), Some(&s("_51a.si")));
    Assert::assert_equals_str(None, Some("f51a"), Some(&s("_f51a.si")));
    Assert::assert_equals_str(None, Some("segment"), Some(&s("_segment.si")));

    // old style
    Assert::assert_equals_str(
        None,
        Some("5"),
        Some(&s("_5_Lucene90FieldsIndex-doc_ids_0.tmp")),
    );

    Assert::assert_false(func("").is_some());
    Assert::assert_false(func("_").is_some());
    Assert::assert_false(func("_.si").is_some());
    Assert::assert_false(func("foo").is_some());
    Assert::assert_false(func("_foo").is_some());
    Assert::assert_false(func("__foo").is_some());
    Assert::assert_false(func("_segment").is_some());
    Assert::assert_false(func("segment.si").is_some());
    Ok(())
}

/// `TestMMapDirectory.testNoGroupingFunc()`.
fn mmap_test_no_grouping_func() -> Result<()> {
    let func = MMapDirectory::no_grouping();
    for name in [
        "_0.doc",
        "_0.si",
        "_54.si",
        "_ff.si",
        "_.si",
        "foo",
        "_foo",
        "__foo",
        "_segment",
        "_segment.si",
        "segment.si",
        "_51a.si",
    ] {
        Assert::assert_false(func(name).is_some());
    }
    Ok(())
}

/// `TestMMapDirectory.testPrefetchWithSingleSegment()`.
fn mmap_test_prefetch_with_single_segment() -> Result<()> {
    mmap_test_prefetch_with_segments(64 * 1024)
}

/// `TestMMapDirectory.testPrefetchWithMultiSegment()`.
fn mmap_test_prefetch_with_multi_segment() -> Result<()> {
    mmap_test_prefetch_with_segments(16 * 1024)
}

/// `IOOBE` (`static final Class<IndexOutOfBoundsException>`).
const IOOBE: &str = "java.lang.IndexOutOfBoundsException";

/// `testPrefetchWithSegments(int maxChunkSize)`: does not verify that the
/// actual segment is prefetched, but rather exercises the code and bounds.
fn mmap_test_prefetch_with_segments(max_chunk_size: usize) -> Result<()> {
    let bytes = vec![0u8; (max_chunk_size * 2) + 1];
    let dir = MMapDirectory::with_options(
        LuceneTestCase::create_temp_dir_prefix("testPrefetchWithSegments"),
        Box::new(NativeFSLockFactory),
        max_chunk_size,
    )?;
    let result = (|| -> Result<()> {
        {
            let mut out = dir.create_output("test", &*DEFAULT_IO_CONTEXT)?;
            out.write_bytes(&bytes, 0, bytes.len())?;
            out.close()?;
        }
        let mut r#in = dir.open_input("test", &*READONCE_IO_CONTEXT)?;
        r#in.prefetch(0, r#in.length())?;
        expect_throws(IOOBE, || r#in.prefetch(1, r#in.length()));
        expect_throws(IOOBE, || r#in.prefetch(r#in.length(), 1));

        let slice1 = r#in.slice("slice-1", 1, r#in.length() - 1)?;
        slice1.prefetch(0, slice1.length())?;
        expect_throws(IOOBE, || slice1.prefetch(1, slice1.length()));
        expect_throws(IOOBE, || slice1.prefetch(slice1.length(), 1));

        // we sliced off all but one byte from the first complete memory segment
        let slice2 = r#in.slice(
            "slice-2",
            max_chunk_size as i64 - 1,
            r#in.length() - max_chunk_size as i64 + 1,
        )?;
        slice2.prefetch(0, slice2.length())?;
        expect_throws(IOOBE, || slice2.prefetch(1, slice2.length()));
        expect_throws(IOOBE, || slice2.prefetch(slice2.length(), 1));
        drop((slice1, slice2));
        r#in.close()
    })();
    result.and(dir.close())
}

/// The public instance methods of `Directory` (`Directory.java:58-208`) as
/// their Rucene counterparts in `trait Directory` (`src/store.rs`); Java's
/// `toString()` is `directory_to_string`.
///
/// Left out are the port's own trait members, which Java has no public
/// method for: `as_fs_directory`, `as_filter_directory` and `as_any` (its
/// stand-ins for `instanceof`), `directory_type_name` (`getClass()`), and
/// `get_temp_file_name` and `ensure_open`, which are `protected` in Java.
const DIRECTORY_METHODS: [&str; 15] = [
    "list_all",
    "delete_file",
    "file_length",
    "create_output",
    "create_temp_output",
    "sync",
    "sync_metadata",
    "rename",
    "open_input",
    "open_checksum_input",
    "obtain_lock",
    "close",
    "copy_from",
    "directory_to_string",
    "get_pending_deletions",
];

/// The methods `impl Directory for FilterDirectory` (`src/store.rs`)
/// implements explicitly, as Java's `FilterDirectory` overrides them
/// (`FilterDirectory.java:59-123`).
const FILTER_DIRECTORY_OVERRIDES: [&str; 15] = [
    "list_all",
    "delete_file",
    "file_length",
    "create_output",
    "create_temp_output",
    "sync",
    "sync_metadata",
    "rename",
    "open_input",
    "obtain_lock",
    "close",
    "get_pending_deletions",
    "ensure_open",
    "directory_to_string",
    "as_filter_directory",
];

/// `TestFilterDirectory.testOverrides()`.
///
/// Java walks `FilterDirectory.class.getMethods()` by reflection and keeps
/// those `Directory` declares, that is, those `FilterDirectory` does not
/// override; Rust has no reflection, so the methods are the declared lists
/// [`DIRECTORY_METHODS`] and [`FILTER_DIRECTORY_OVERRIDES`] (user decision,
/// sprint 9).
fn filter_test_overrides() -> Result<()> {
    // verify that all methods of Directory are overridden by FilterDirectory,
    // except those under the 'exclude' list
    let mut exclude: HashSet<&str> = HashSet::new();
    exclude.insert("copy_from");
    exclude.insert("open_checksum_input");
    for m in DIRECTORY_METHODS {
        if !FILTER_DIRECTORY_OVERRIDES.contains(&m) {
            Assert::assert_true_msg(&format!("method {m} not overridden!"), exclude.contains(m));
        }
    }
    Ok(())
}

/// `TestFilterDirectory.testUnwrap()`.
fn filter_test_unwrap() -> Result<()> {
    let dir: Box<dyn Directory> = Box::new(FSDirectory::open(LuceneTestCase::create_temp_dir())?);
    let dir_ptr = &*dir as *const dyn Directory as *const ();
    let dir2 = FilterDirectory::new(dir);
    Assert::assert_true(std::ptr::eq(
        dir2.get_delegate() as *const dyn Directory as *const (),
        dir_ptr,
    ));
    Assert::assert_true(std::ptr::eq(
        FilterDirectory::unwrap(&dir2) as *const dyn Directory as *const (),
        dir_ptr,
    ));
    dir2.close()
}

/// `TestFileSwitchDirectory.testBasic()`: writing doc stores to disk and
/// everything else to ram works.
fn file_switch_test_basic() -> Result<()> {
    let mut file_extensions = HashSet::new();
    // `Lucene90CompressingStoredFieldsWriter.FIELDS_EXTENSION`
    file_extensions.insert("fdt".to_string());
    file_extensions.insert("fdx".to_string());
    file_extensions.insert("fdm".to_string());

    let primary_dir = Arc::new(MockDirectoryWrapper::new(
        &*rnd(),
        Arc::new(ByteBuffersDirectory::new()),
    ));
    primary_dir.set_check_index_on_close(false); // only part of an index
    let secondary_dir = Arc::new(MockDirectoryWrapper::new(
        &*rnd(),
        Arc::new(ByteBuffersDirectory::new()),
    ));
    secondary_dir.set_check_index_on_close(false); // only part of an index

    let fsd: Arc<dyn Directory> = Arc::new(FileSwitchDirectory::new(
        file_extensions.clone(),
        Box::new(SharedDirectory(
            Arc::clone(&primary_dir) as Arc<dyn Directory>
        )),
        Box::new(SharedDirectory(
            Arc::clone(&secondary_dir) as Arc<dyn Directory>
        )),
        true,
    )?);
    // for now we wire the default codec because we rely upon its specific impl
    let config = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*rnd()))));
    config.set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(false)))?;
    config.set_codec(TestUtil::get_default_codec())?;
    config.set_use_compound_file(false)?;
    let writer = IndexWriter::new(Arc::clone(&fsd), config)?;
    create_index_no_close(true, "ram", &writer)?;
    let reader = directory_reader::open_with_writer(&*writer)?;
    Assert::assert_equals(100, reader.max_doc());
    writer.commit()?;
    // we should see only fdx,fdt files here
    let files = primary_dir.list_all()?;
    Assert::assert_true(!files.is_empty());
    for file in &files {
        let ext = FileSwitchDirectory::get_extension(file);
        Assert::assert_true(file_extensions.contains(&ext));
    }
    let files = secondary_dir.list_all()?;
    Assert::assert_true(!files.is_empty());
    // we should not see fdx,fdt files here
    for file in &files {
        let ext = FileSwitchDirectory::get_extension(file);
        Assert::assert_false(file_extensions.contains(&ext));
    }
    reader.close()?;
    writer.close()?;

    let files = fsd.list_all()?;
    for file in &files {
        Assert::assert_true(!file.is_empty()); // `assertNotNull(file)`
    }
    close_shared(fsd)
}

/// `TestIndexWriterReader.createIndexNoClose(boolean, String, IndexWriter)`
/// (`public static`), which `testBasic` calls.
fn create_index_no_close(multi_segment: bool, index_name: &str, w: &IndexWriter) -> Result<()> {
    for i in 0..100 {
        w.add_document(&DocHelper::create_document(i, index_name, 4))?;
    }
    if !multi_segment {
        w.force_merge(1)?;
    }
    Ok(())
}

/// A `Directory` reference the test keeps while `FileSwitchDirectory` owns
/// it, as Java shares `primaryDir`/`secondaryDir`.
struct SharedDirectory(Arc<dyn Directory>);

impl Directory for SharedDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.0.list_all()
    }
    fn delete_file(&self, name: &str) -> Result<()> {
        self.0.delete_file(name)
    }
    fn file_length(&self, name: &str) -> Result<i64> {
        self.0.file_length(name)
    }
    fn create_output(
        &self,
        name: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn rucene::store::IndexOutput>> {
        self.0.create_output(name, context)
    }
    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn rucene::store::IndexOutput>> {
        self.0.create_temp_output(prefix, suffix, context)
    }
    fn sync(&self, names: &[String]) -> Result<()> {
        self.0.sync(names)
    }
    fn sync_metadata(&self) -> Result<()> {
        self.0.sync_metadata()
    }
    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.0.rename(source, dest)
    }
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.0.open_input(name, context)
    }
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn rucene::store::Lock>> {
        self.0.obtain_lock(name)
    }
    fn close(&self) -> Result<()> {
        close_shared(Arc::clone(&self.0))
    }
    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.0.get_pending_deletions()
    }
    fn ensure_open(&self) -> Result<()> {
        self.0.ensure_open()
    }
}

/// `TestFileSwitchDirectory.testRenameTmpFile()`.
fn file_switch_test_rename_tmp_file(case: &dyn BaseDirectoryTestCase) -> Result<()> {
    {
        let directory = case.get_directory(&LuceneTestCase::create_temp_dir())?;
        let result = (|| -> Result<()> {
            let name;
            {
                let mut out = directory.create_temp_output("foo.cfs", "", &*DEFAULT_IO_CONTEXT)?;
                out.write_int(1)?;
                name = out.name().to_string();
                out.close()?;
            }
            let count = |d: &dyn Directory, f: &str| -> Result<usize> {
                Ok(d.list_all()?.iter().filter(|x| *x == f).count())
            };
            Assert::assert_equals(1usize, count(&*directory, &name)?);
            Assert::assert_equals(0usize, count(&*directory, "foo.cfs")?);
            directory.rename(&name, "foo.cfs")?;
            Assert::assert_equals(1usize, count(&*directory, "foo.cfs")?);
            Assert::assert_equals(0usize, count(&*directory, &name)?);
            Ok(())
        })();
        result.and(directory.close())?;
    }

    let directory =
        TestFileSwitchDirectory::new_fs_switch_directory(HashSet::from(["bar".to_string()]))?;
    let result = (|| -> Result<()> {
        let broken_name;
        {
            let mut out = directory.create_temp_output("foo", "bar", &*DEFAULT_IO_CONTEXT)?;
            out.write_int(1)?;
            broken_name = out.name().to_string();
            out.close()?;
        }
        let exception = expect_throws("java.nio.file.AtomicMoveNotSupportedException", || {
            directory.rename(&broken_name, "foo.bar")
        });
        Assert::assert_equals_str(
            None,
            Some("foo_bar_0.tmp -> foo.bar: source and dest are in different directories"),
            exception.message(),
        );
        Ok(())
    })();
    result.and(directory.close())
}

/// The anonymous `FilterFileChannel` of
/// `TestNIOFSDirectory.testHandleExceptionInConstructor`, whose `size()`
/// throws `IOException("simulated")`.
struct SimulatedSizeFailure;

impl FileChannelOverrides for SimulatedSizeFailure {
    fn size(
        &self,
        _delegate: &dyn rucene::store::java_nio_file::FileChannel,
    ) -> std::io::Result<i64> {
        Err(std::io::Error::other("simulated"))
    }
}

/// The anonymous `LeakFS` of
/// `TestNIOFSDirectory.testHandleExceptionInConstructor`, whose
/// `newFileChannel` wraps `super.newFileChannel(...)` in a
/// [`SimulatedSizeFailure`] channel.
struct SimulatedSizeFailureLeakFS;

impl LeakFSOverrides for SimulatedSizeFailureLeakFS {
    fn new_file_channel(
        &self,
        this: &LeakFS,
        path: &Path,
        options: &[rucene::store::java_nio_file::OpenOption],
    ) -> std::io::Result<Arc<dyn rucene::store::java_nio_file::FileChannel>> {
        Ok(Arc::new(FilterFileChannel::with_overrides(
            rucene_test_framework::mockfile::handle_tracking_fs::new_file_channel(
                this, path, options,
            )?,
            Box::new(SimulatedSizeFailure),
            "org.apache.lucene.store.TestNIOFSDirectory$1$1",
        )))
    }
}

/// `TestNIOFSDirectory.testHandleExceptionInConstructor()`.
fn niofs_test_handle_exception_in_constructor() -> Result<()> {
    let path = LuceneTestCase::create_temp_dir().to_real_path(&[])?;
    let leak_fs = LeakFS::with_overrides(
        Arc::clone(path.get_file_system()),
        Box::new(SimulatedSizeFailureLeakFS),
    );
    let wrapped = leak_fs.wrap_path(&path);
    let dir = NIOFSDirectory::open(&wrapped)?;
    let result = (|| -> Result<()> {
        {
            let mut out = dir.create_output("test.bin", &*DEFAULT_IO_CONTEXT)?;
            let written = out.write_string("hello");
            written.and(out.close())?;
        }
        let error = expect_throws("java.io.IOException", || {
            dir.open_input("test.bin", &*DEFAULT_IO_CONTEXT)
        });
        Assert::assert_equals_str(None, Some("simulated"), error.message());
        Ok(())
    })();
    result.and(dir.close())
}

/// `TestFileSwitchDirectory.testDeleteAndList()`: relies on windows
/// semantics.
fn file_switch_test_delete_and_list() -> Result<()> {
    let path = LuceneTestCase::create_temp_dir();
    let provider = WindowsFS::new(Arc::clone(path.get_file_system()));
    let index_path = provider.wrap_path(&path);
    let dir = FileSwitchDirectory::new(
        HashSet::from(["tim".to_string()]),
        Box::new(NIOFSDirectory::open(&index_path)?),
        Box::new(NIOFSDirectory::open(&index_path)?),
        true,
    )?;
    let result = (|| -> Result<()> {
        dir.create_output("foo.tim", &*DEFAULT_IO_CONTEXT)?
            .close()?;
        let strip_extra = |array: Vec<String>| -> i64 {
            array.iter().filter(|f| !f.starts_with("extra")).count() as i64
        };
        {
            let mut index_input = dir.open_input("foo.tim", &*DEFAULT_IO_CONTEXT)?;
            dir.delete_file("foo.tim")?;
            Assert::assert_equals(1usize, dir.primary_dir().get_pending_deletions()?.len());
            Assert::assert_equals(1usize, dir.get_pending_deletions()?.len());
            Assert::assert_equals(0i64, strip_extra(dir.list_all()?));
            Assert::assert_equals(0i64, strip_extra(dir.primary_dir().list_all()?));
            Assert::assert_equals(1i64, strip_extra(dir.secondary_dir().list_all()?));
            index_input.close()?;
        }
        Assert::assert_equals(0usize, dir.primary_dir().get_pending_deletions()?.len());
        Assert::assert_equals(0usize, dir.get_pending_deletions()?.len());
        Assert::assert_equals(0i64, strip_extra(dir.list_all()?));
        Assert::assert_equals(0i64, strip_extra(dir.primary_dir().list_all()?));
        Assert::assert_equals(0i64, strip_extra(dir.secondary_dir().list_all()?));
        Ok(())
    })();
    result.and(dir.close())
}

/// `getDirectory(Path path, int maxChunkSize)` of `TestMultiMMap`.
fn multi_mmap_get_directory(
    case: &dyn BaseDirectoryTestCase,
    path: &Path,
    max_chunk_size: i32,
) -> Result<Box<dyn Directory>> {
    case.as_chunked()
        .expect("TestMultiMMap is a BaseChunkedDirectoryTestCase")
        .get_directory_chunked(path, max_chunk_size)
}

/// `assertCorrectImpl(boolean isSingle, IndexInput ii)` (`private static`).
///
/// `ii.getClass().getSimpleName()` is the input's
/// `MemorySegmentAccessInput::class_simple_name`.
fn multi_mmap_assert_correct_impl(is_single: bool, ii: &dyn IndexInput) {
    let clazz = ii
        .as_memory_segment_access_input()
        .map(|access| access.class_simple_name())
        .unwrap_or("");
    let matches = |prefix: &str| {
        clazz
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix("Impl"))
            .is_some_and(|word| {
                !word.is_empty() && word.chars().all(|c| c.is_alphanumeric() || c == '_')
            })
    };
    if is_single {
        Assert::assert_true_msg(
            &format!("Require a single impl, got {clazz}"),
            matches("Single"),
        );
    } else {
        Assert::assert_true_msg(
            &format!("Require a multi impl, got {clazz}"),
            matches("Multi"),
        );
    }
}

/// `TestMultiMMap.testSeekingExceptions()`.
fn multi_mmap_test_seeking_exceptions(case: &dyn BaseDirectoryTestCase) -> Result<()> {
    let slice_size = 128;
    let dir = multi_mmap_get_directory(case, &LuceneTestCase::create_temp_dir(), slice_size)?;
    let result = (|| -> Result<()> {
        let size = 128 + 63;
        {
            let mut out = dir.create_output("a", &*DEFAULT_IO_CONTEXT)?;
            let written = (|| -> Result<()> {
                for _ in 0..size {
                    out.write_byte(0)?;
                }
                Ok(())
            })();
            written.and(out.close())?;
        }
        let mut r#in = dir.open_input("a", &*DEFAULT_IO_CONTEXT)?;
        let read = (|| -> Result<()> {
            let negative_pos: i64 = -1234;
            let e = expect_throws_any_of(
                &[
                    "java.lang.IllegalArgumentException",
                    "java.lang.AssertionError",
                ],
                || r#in.seek(negative_pos),
            );
            Assert::assert_true_msg(
                "does not mention negative position",
                e.message().is_some_and(|m| m.contains("negative position")),
            );

            let pos_after_eof: i64 = size + 123;
            let eof = expect_throws("java.io.EOFException", || r#in.seek(pos_after_eof));
            Assert::assert_true_msg(
                &format!("wrong position in error message: {eof}"),
                eof.message()
                    .is_some_and(|m| m.contains(&format!("(pos={pos_after_eof})"))),
            );

            // this test verifies that the invalid position is transformed back to original one
            // for exception by slicing:
            let mut slice = r#in.slice("slice", 33, (slice_size + 15) as i64)?;
            // ensure that the slice uses multi-mmap:
            multi_mmap_assert_correct_impl(false, &*slice);
            let eof = expect_throws("java.io.EOFException", || slice.seek(pos_after_eof));
            Assert::assert_true_msg(
                &format!("wrong position in error message: {eof}"),
                eof.message()
                    .is_some_and(|m| m.contains(&format!("(pos={pos_after_eof})"))),
            );
            Ok(())
        })();
        read.and(r#in.close())
    })();
    result.and(dir.close())
}

/// `TestMultiMMap.testImplementations()`.
fn multi_mmap_test_implementations(case: &dyn BaseDirectoryTestCase) -> Result<()> {
    for i in 2..12 {
        let chunk_size: i32 = 1 << i;
        let mmap_dir = multi_mmap_get_directory(
            case,
            &LuceneTestCase::create_temp_dir_prefix("testImplementations"),
            chunk_size,
        )?;
        let mut io = mmap_dir.create_output("bytes", &*LuceneTestCase::new_io_context(&*rnd()))?;
        let size = rnd().next_int_bound(chunk_size * 2) + 3; // add some buffer of 3 for slice tests
        let mut bytes = vec![0u8; size as usize];
        rnd().next_bytes(&mut bytes);
        io.write_bytes(&bytes, 0, bytes.len())?;
        io.close()?;
        drop(io);
        let mut ii = mmap_dir.open_input("bytes", &*LuceneTestCase::new_io_context(&*rnd()))?;
        let mut actual = vec![0u8; size as usize]; // first read all bytes
        let len = actual.len();
        ii.read_bytes(Some(&mut actual), 0, len)?;
        Assert::assert_equals(BytesRef::new(bytes.clone()), BytesRef::new(actual));
        // reinit:
        ii.seek(0)?;

        // check impl (we must check size < chunksize: currently, if size==chunkSize, we get 2
        // buffers, the second one empty:
        multi_mmap_assert_correct_impl(size < chunk_size, &*ii);

        // clone tests:
        // `assertSame(ii.getClass(), ii.clone().getClass())`
        let class_of = |input: &dyn IndexInput| {
            input
                .as_memory_segment_access_input()
                .map(|access| access.class_simple_name())
        };
        Assert::assert_equals(
            class_of(&*ii).unwrap_or(""),
            class_of(&*ii.clone_input()?).unwrap_or(""),
        );

        // slice test (offset 0)
        let mut slice_size = rnd().next_int_bound(size);
        let slice = ii.slice("slice", 0, slice_size as i64)?;
        multi_mmap_assert_correct_impl(slice_size < chunk_size, &*slice);

        // slice test (offset > 0 )
        let offset = rnd().next_int_bound(size - 1) + 1;
        slice_size = rnd().next_int_bound(size - offset + 1);
        let slice = ii.slice("slice", offset as i64, slice_size as i64)?;
        multi_mmap_assert_correct_impl(offset % chunk_size + slice_size < chunk_size, &*slice);

        ii.close()?;
        mmap_dir.close()?;
    }
    Ok(())
}

/// `TestMultiMMap.testCloneSafety()`.
fn multi_mmap_test_clone_safety(case: &dyn BaseDirectoryTestCase) -> Result<()> {
    let mmap_dir =
        case.get_directory(&LuceneTestCase::create_temp_dir_prefix("testCloneSafety"))?;
    let mut io = mmap_dir.create_output("bytes", &*LuceneTestCase::new_io_context(&*rnd()))?;
    io.write_v_int(5)?;
    io.close()?;
    drop(io);
    let mut one = mmap_dir.open_input("bytes", &*DEFAULT_IO_CONTEXT)?;
    let mut two = one.clone_input()?;
    let mut three = two.clone_input()?; // clone of clone
    one.close()?;
    expect_throws(ACE, || one.read_v_int());
    expect_throws(ACE, || two.read_v_int());
    expect_throws(ACE, || three.read_v_int());
    two.close()?;
    three.close()?;
    // test double close of master:
    one.close()?;
    mmap_dir.close()
}

/// `TestMultiMMap.testCloneSliceSafety()`.
fn multi_mmap_test_clone_slice_safety(case: &dyn BaseDirectoryTestCase) -> Result<()> {
    let mmap_dir = case.get_directory(&LuceneTestCase::create_temp_dir_prefix(
        "testCloneSliceSafety",
    ))?;
    let mut io = mmap_dir.create_output("bytes", &*LuceneTestCase::new_io_context(&*rnd()))?;
    io.write_int(1)?;
    io.write_int(2)?;
    io.close()?;
    drop(io);
    let mut slicer = mmap_dir.open_input("bytes", &*LuceneTestCase::new_io_context(&*rnd()))?;
    let mut one = slicer.slice("first int", 0, 4)?;
    let mut two = slicer.slice("second int", 4, 4)?;
    let mut three = one.clone_input()?; // clone of clone
    let mut four = two.clone_input()?; // clone of clone
    slicer.close()?;
    expect_throws(ACE, || one.read_int());
    expect_throws(ACE, || two.read_int());
    expect_throws(ACE, || three.read_int());
    expect_throws(ACE, || four.read_int());
    one.close()?;
    two.close()?;
    three.close()?;
    four.close()?;
    // test double-close of slicer:
    slicer.close()?;
    mmap_dir.close()
}

/// `AlreadyClosedException`.
const ACE: &str = "org.apache.lucene.store.AlreadyClosedException";

/// `TestByteBuffersDirectory.testBuildIndex()`.
fn byte_buffers_test_build_index(case: &dyn BaseDirectoryTestCase) -> Result<()> {
    // `getDirectory(null)`: the suppliers ignore the path.
    let no_path = Path::new(rucene::store::java_nio_file::FileSystems::get_default(), "");
    let dir: Arc<dyn Directory> = Arc::from(case.get_directory(&no_path)?);
    let config = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*rnd()))));
    config.set_open_mode(OpenMode::CREATE)?;
    let writer = IndexWriter::new(Arc::clone(&dir), config)?;
    let result = (|| -> Result<()> {
        let docs = randomizedtesting::RandomizedTest::random_int_between(0, 10);
        let mut i = docs;
        while i > 0 {
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_string_field(
                "content",
                English::int_to_english(i).trim(),
                Store::YES,
            )));
            writer.add_document(&doc)?;
            i -= 1;
        }
        writer.commit()?;
        Assert::assert_equals(docs, writer.get_doc_stats()?.num_docs);
        Ok(())
    })();
    let result = result.and(writer.close());
    result.and(close_shared(dir))
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestDirectorySuiteDraws, TestChunkedDirectorySuiteDraws
// -----------------------------------------------------------------------------

static DIRECTORY_DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static CHUNKED_DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestDirectorySuiteDraws` of `BaseSuitesDrawsOracle`: the cases of
/// `BaseDirectoryTestCase` on `TestTrackingDirectoryWrapper`'s directory.
fn directory_draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.store.TestDirectorySuiteDraws")
        .extends(&base_directory_test_case::class())
        .constructor(Constructor::no_args(|| {
            DirectoryTestInstance::new(TestTrackingDirectoryWrapper)
        }))
        .method(draw_hooks::record_tear_down(
            &DIRECTORY_DRAWS,
            draw_hooks::lucene_tear_down::<DirectoryTestInstance>,
        ))
        .method(draw_hooks::record_class_draw(&DIRECTORY_DRAWS))
        .build()
}

/// `TestChunkedDirectorySuiteDraws` of `BaseSuitesDrawsOracle`: the cases
/// of `BaseChunkedDirectoryTestCase` on `TestMultiByteBuffersDirectory`'s
/// directories.
fn chunked_draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.store.TestChunkedDirectorySuiteDraws")
        .extends(&base_chunked_directory_test_case::class())
        .constructor(Constructor::no_args(|| {
            DirectoryTestInstance::new(TestMultiByteBuffersDirectory)
        }))
        .method(draw_hooks::record_tear_down(
            &CHUNKED_DRAWS,
            draw_hooks::lucene_tear_down::<DirectoryTestInstance>,
        ))
        .method(draw_hooks::record_class_draw(&CHUNKED_DRAWS))
        .build()
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

#[test]
fn test_directory_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match(
        "base_directory_draws.tsv",
        directory_draws_class,
        &DIRECTORY_DRAWS,
    );
}

#[test]
fn test_chunked_directory_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match(
        "base_chunked_directory_draws.tsv",
        chunked_draws_class,
        &CHUNKED_DRAWS,
    );
}

#[test]
fn test_nrt_caching_directory() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(nrt_caching_class);
}

#[test]
fn test_tracking_directory_wrapper() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(tracking_class);
}

#[test]
fn test_mmap_directory() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(mmap_class);
}

#[test]
fn test_niofs_directory() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(niofs_class);
}

#[test]
fn test_filter_directory() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(filter_class);
}

#[test]
fn test_file_switch_directory() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(file_switch_class);
}

#[test]
fn test_byte_buffers_directory() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(byte_buffers_class);
}

#[test]
fn test_multi_byte_buffers_directory() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(multi_byte_buffers_class);
}

#[test]
fn test_multi_mmap() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(multi_mmap_class);
}
