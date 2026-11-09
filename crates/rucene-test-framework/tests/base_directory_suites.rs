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
//! The classes' own extra test methods belong to the core-test port
//! (sprint 7) and are not declared here.
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
mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

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

/// A binding class of `BaseDirectoryTestCase`.
fn binding(name: &str, make: fn() -> DirectoryTestInstance) -> Arc<Class> {
    Class::builder(name)
        .extends(&base_directory_test_case::class())
        .constructor(Constructor::no_args(make))
        .build()
}

/// A binding class of `BaseChunkedDirectoryTestCase`.
fn chunked_binding(name: &str, make: fn() -> DirectoryTestInstance) -> Arc<Class> {
    Class::builder(name)
        .extends(&base_chunked_directory_test_case::class())
        .constructor(Constructor::no_args(make))
        .build()
}

fn nrt_caching_class() -> Arc<Class> {
    binding("org.apache.lucene.store.TestNRTCachingDirectory", || {
        DirectoryTestInstance::new(TestNRTCachingDirectory)
    })
}

fn tracking_class() -> Arc<Class> {
    binding(
        "org.apache.lucene.store.TestTrackingDirectoryWrapper",
        || DirectoryTestInstance::new(TestTrackingDirectoryWrapper),
    )
}

fn mmap_class() -> Arc<Class> {
    binding("org.apache.lucene.store.TestMMapDirectory", || {
        DirectoryTestInstance::new(TestMMapDirectory)
    })
}

fn niofs_class() -> Arc<Class> {
    binding("org.apache.lucene.store.TestNIOFSDirectory", || {
        DirectoryTestInstance::new(TestNIOFSDirectory)
    })
}

fn filter_class() -> Arc<Class> {
    binding("org.apache.lucene.store.TestFilterDirectory", || {
        DirectoryTestInstance::new(TestFilterDirectory)
    })
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
        .build()
}

fn multi_byte_buffers_class() -> Arc<Class> {
    chunked_binding(
        "org.apache.lucene.store.TestMultiByteBuffersDirectory",
        || DirectoryTestInstance::new(TestMultiByteBuffersDirectory),
    )
}

fn multi_mmap_class() -> Arc<Class> {
    chunked_binding("org.apache.lucene.store.TestMultiMMap", || {
        DirectoryTestInstance::new(TestMultiMMap)
    })
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
