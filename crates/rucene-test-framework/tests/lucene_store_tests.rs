//! Port of the tests of `lucene/test-framework/src/test/org/apache/lucene/tests/store/`
//! (task #359, rest): `TestMockDirectoryWrapper` and
//! `TestSerializedIOCountingDirectory`, both `extends BaseDirectoryTestCase`,
//! ported in full (their own methods and the inherited cases), each run as a
//! suite by the ported runner and reporting what Lucene's own class reported
//! on the JVM (`framework_tests.tsv`, written by `FrameworkTestsOracle`).
//!
//! **Divergence** (`testCorruptOnCloseIsWorking`): Java wraps the directory
//! in one `PreventCloseDirectoryWrapper` and hands that one object to the
//! `MockDirectoryWrapper`, the `RandomIndexWriter` and the final
//! `openInput`. The port's `MockDirectoryWrapper` must own its delegate to
//! close it, so each of the three gets its own `PreventCloseDirectoryWrapper`
//! over the same directory; the wrapper holds no state of its own, so the
//! three behave as the one Java shares.

mod oracle_support;

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{Class, ClassBuilder, Constructor};
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::Result;
use rucene::store::java_nio_file::Path;
use rucene::store::{
    BufferedChecksumIndexInput, ByteArrayDataInput, ByteBuffersDirectory, DataAccessHint,
    Directory, FSDirectory, FileOpenHint, IOContext, IndexInput, IndexOutput, Lock,
    DEFAULT_IO_CONTEXT,
};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::store::base_directory_test_case::{
    self, directory_test, BaseDirectoryTestCase, DirectoryTestInstance, OwnedTestDirectory,
};
use rucene_test_framework::store::{MockDirectoryWrapper, SerialIOCountingDirectory};
use rucene_test_framework::util::java_exceptions::{expect_throws, is_instance};
use rucene_test_framework::util::lucene_test_case::Nightly;
use rucene_test_framework::util::LuceneTestCase;

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

fn run_suite(builder: ClassBuilder) {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    oracle_support::assert_framework_outcomes(&builder.build());
}

// -----------------------------------------------------------------------------
// TestMockDirectoryWrapper
// -----------------------------------------------------------------------------

/// `TestMockDirectoryWrapper extends BaseDirectoryTestCase`.
struct TestMockDirectoryWrapper;

impl BaseDirectoryTestCase for TestMockDirectoryWrapper {
    fn get_directory(&self, path: &Path) -> Result<Box<dyn Directory>> {
        let dir = if LuceneTestCase::random().next_boolean() {
            LuceneTestCase::new_mock_directory()
        } else {
            LuceneTestCase::new_mock_fs_directory(path)
        };
        Ok(Box::new(dir))
    }
}

/// `testDiskFull()`.
fn test_disk_full() -> Result<()> {
    // test writeBytes
    let mut dir = LuceneTestCase::new_mock_directory();
    dir.set_max_size_in_bytes(3);
    let bytes = [1u8, 2];
    let mut out = dir.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
    out.write_bytes(&bytes, 0, bytes.len())?; // first write should succeed
                                              // close() to ensure the written bytes are not buffered and counted
                                              // against the directory size
    out.close()?;
    let mut out2 = dir.create_output("bar", &*DEFAULT_IO_CONTEXT)?;
    expect_throws("java.io.IOException", || {
        out2.write_bytes(&bytes, 0, bytes.len())
    });
    out2.close()?;
    drop(out2);
    dir.close()?;

    // test copyBytes
    let mut dir = LuceneTestCase::new_mock_directory();
    dir.set_max_size_in_bytes(3);
    let mut out = dir.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
    out.copy_bytes(&mut ByteArrayDataInput::new(&bytes), bytes.len() as i64)?; // first copy should succeed
                                                                               // close() to ensure the written bytes are not buffered and counted
                                                                               // against the directory size
    out.close()?;
    let mut out3 = dir.create_output("bar", &*DEFAULT_IO_CONTEXT)?;
    expect_throws("java.io.IOException", || {
        out3.copy_bytes(&mut ByteArrayDataInput::new(&bytes), bytes.len() as i64)
    });
    out3.close()?;
    drop(out3);
    dir.close()
}

/// `testMDWinsideOfMDW()`: add MDW inside another MDW.
fn test_mdw_inside_of_mdw() -> Result<()> {
    let inner: Arc<dyn Directory> = Arc::new(LuceneTestCase::new_mock_directory());
    let mut dir = MockDirectoryWrapper::new(&*LuceneTestCase::random(), inner);
    {
        let iw = RandomIndexWriter::new_with_mock_analyzer(
            &*LuceneTestCase::random(),
            dir.as_directory(),
        )?;
        for _ in 0..20 {
            iw.add_document(&Document::new())?;
        }
        iw.commit()?;
        iw.close()?;
    }
    dir.close()
}

/// `PreventCloseDirectoryWrapper` (`private static class … extends
/// FilterDirectory`): just shields the wrapped directory from being closed.
struct PreventCloseDirectoryWrapper {
    r#in: Arc<dyn Directory>,
}

impl Directory for PreventCloseDirectoryWrapper {
    fn list_all(&self) -> Result<Vec<String>> {
        self.r#in.list_all()
    }
    fn delete_file(&self, name: &str) -> Result<()> {
        self.r#in.delete_file(name)
    }
    fn file_length(&self, name: &str) -> Result<i64> {
        self.r#in.file_length(name)
    }
    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.r#in.create_output(name, context)
    }
    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.r#in.create_temp_output(prefix, suffix, context)
    }
    fn sync(&self, names: &[String]) -> Result<()> {
        self.r#in.sync(names)
    }
    fn sync_metadata(&self) -> Result<()> {
        self.r#in.sync_metadata()
    }
    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.r#in.rename(source, dest)
    }
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.r#in.open_input(name, context)
    }
    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        self.r#in.open_checksum_input(name)
    }
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.r#in.obtain_lock(name)
    }
    /// `close()` (`@Override`): nothing.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
    fn get_pending_deletions(&self) -> Result<std::collections::HashSet<String>> {
        self.r#in.get_pending_deletions()
    }
    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        Some(self.r#in.as_ref())
    }
    fn directory_to_string(&self) -> String {
        format!(
            "PreventCloseDirectoryWrapper({})",
            self.r#in.directory_to_string()
        )
    }
}

/// `testCorruptOnCloseIsWorking(Directory dir)` (`private`).
fn test_corrupt_on_close_is_working(dir: &Arc<dyn Directory>) -> Result<()> {
    let prevent = |dir: &Arc<dyn Directory>| -> Arc<dyn Directory> {
        Arc::new(PreventCloseDirectoryWrapper {
            r#in: Arc::clone(dir),
        })
    };
    {
        let mut wrapped = MockDirectoryWrapper::new(&*LuceneTestCase::random(), prevent(dir));
        let result = (|| -> Result<()> {
            // otherwise MDW sometimes randomly leaves the file intact and we'll see false test
            // failures:
            wrapped.set_always_corrupt(true);

            // MDW will only try to corrupt things if it sees an index:
            let iw = RandomIndexWriter::new_with_mock_analyzer(
                &*LuceneTestCase::random(),
                prevent(dir),
            )?;
            iw.add_document(&Document::new())?;
            iw.close()?;
            drop(iw);

            // not sync'd!
            let mut out = wrapped.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
            for i in 0..100 {
                out.write_int(i)?;
            }
            out.close()
        })();
        // MDW.close now corrupts our unsync'd file (foo):
        let closed = wrapped.close();
        result.and(closed)?;
    }

    let shielded = prevent(dir);
    let mut changed = false;
    let mut input = match shielded.open_input("foo", &*DEFAULT_IO_CONTEXT) {
        Ok(input) => Some(input),
        Err(e)
            if is_instance(&e, "java.nio.file.NoSuchFileException")
                || is_instance(&e, "java.io.FileNotFoundException") =>
        {
            // ok
            changed = true;
            None
        }
        Err(e) => return Err(e),
    };
    if let Some(input) = input.as_mut() {
        for i in 0..100 {
            let x = match input.read_int() {
                Ok(x) => x,
                Err(e) if is_instance(&e, "java.io.EOFException") => {
                    changed = true;
                    break;
                }
                Err(e) => return Err(e),
            };
            if x != i {
                changed = true;
                break;
            }
        }
        input.close()?;
    }
    Assert::assert_true_msg(
        &format!(
            "MockDirectoryWrapper on dir={} failed to corrupt an unsync'd file",
            shielded.directory_to_string()
        ),
        changed,
    );
    Ok(())
}

/// `testCorruptOnCloseIsWorkingFSDir()`.
fn test_corrupt_on_close_is_working_fs_dir() -> Result<()> {
    let path = LuceneTestCase::create_temp_dir();
    let mut dir: Arc<dyn Directory> = Arc::new(OwnedTestDirectory::new(
        LuceneTestCase::new_fs_directory(&path),
    ));
    let result = test_corrupt_on_close_is_working(&dir);
    let closed = Arc::get_mut(&mut dir).map_or(Ok(()), |d| d.close());
    result.and(closed)
}

/// `testCorruptOnCloseIsWorkingOnByteBuffersDirectory()`.
fn test_corrupt_on_close_is_working_on_byte_buffers_directory() -> Result<()> {
    let mut dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let result = test_corrupt_on_close_is_working(&dir);
    let closed = Arc::get_mut(&mut dir).map_or(Ok(()), |d| d.close());
    result.and(closed)
}

/// `testAbuseClosedIndexInput()`.
fn test_abuse_closed_index_input() -> Result<()> {
    let mut dir = LuceneTestCase::new_mock_directory();
    let mut out = dir.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
    out.write_byte(42)?;
    out.close()?;
    let mut input = dir.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
    input.close()?;
    expect_throws("java.lang.RuntimeException", || input.read_byte());
    drop(input);
    dir.close()
}

/// `testAbuseCloneAfterParentClosed()`.
fn test_abuse_clone_after_parent_closed() -> Result<()> {
    let mut dir = LuceneTestCase::new_mock_directory();
    let mut out = dir.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
    out.write_byte(42)?;
    out.close()?;
    let mut input = dir.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
    let mut clone = input.clone_input()?;
    input.close()?;
    expect_throws("java.lang.RuntimeException", || clone.read_byte());
    drop(clone);
    drop(input);
    dir.close()
}

/// `testAbuseCloneOfCloneAfterParentClosed()`.
fn test_abuse_clone_of_clone_after_parent_closed() -> Result<()> {
    let mut dir = LuceneTestCase::new_mock_directory();
    let mut out = dir.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
    out.write_byte(42)?;
    out.close()?;
    let mut input = dir.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
    let clone1 = input.clone_input()?;
    let mut clone2 = clone1.clone_input()?;
    input.close()?;
    expect_throws("java.lang.RuntimeException", || clone2.read_byte());
    drop(clone2);
    drop(clone1);
    drop(input);
    dir.close()
}

fn mock_directory_wrapper_class() -> ClassBuilder {
    Class::builder("org.apache.lucene.tests.store.TestMockDirectoryWrapper")
        .extends(&base_directory_test_case::class())
        .constructor(Constructor::no_args(|| {
            DirectoryTestInstance::new(TestMockDirectoryWrapper)
        }))
        // we wrap the directory in slow stuff, so only run nightly
        .method(
            directory_test("testThreadSafetyInListAll", |c| {
                c.test_thread_safety_in_list_all()
            })
            .annotate(Nightly),
        )
        .method(directory_test("testDiskFull", |_| test_disk_full()))
        .method(directory_test("testMDWinsideOfMDW", |_| {
            test_mdw_inside_of_mdw()
        }))
        .method(directory_test("testCorruptOnCloseIsWorkingFSDir", |_| {
            test_corrupt_on_close_is_working_fs_dir()
        }))
        .method(directory_test(
            "testCorruptOnCloseIsWorkingOnByteBuffersDirectory",
            |_| test_corrupt_on_close_is_working_on_byte_buffers_directory(),
        ))
        .method(directory_test("testAbuseClosedIndexInput", |_| {
            test_abuse_closed_index_input()
        }))
        .method(directory_test("testAbuseCloneAfterParentClosed", |_| {
            test_abuse_clone_after_parent_closed()
        }))
        .method(directory_test(
            "testAbuseCloneOfCloneAfterParentClosed",
            |_| test_abuse_clone_of_clone_after_parent_closed(),
        ))
}

#[test]
fn test_mock_directory_wrapper() {
    run_suite(mock_directory_wrapper_class());
}

// -----------------------------------------------------------------------------
// TestSerializedIOCountingDirectory
// -----------------------------------------------------------------------------

/// `TestSerializedIOCountingDirectory extends BaseDirectoryTestCase`:
/// `new SerialIOCountingDirectory(FSDirectory.open(path))`.
struct TestSerializedIOCountingDirectory;

impl BaseDirectoryTestCase for TestSerializedIOCountingDirectory {
    fn get_directory(&self, path: &Path) -> Result<Box<dyn Directory>> {
        Ok(Box::new(SerialIOCountingDirectory::new(Box::new(
            FSDirectory::open(path)?,
        ))))
    }
}

/// `IOContext.DEFAULT.withHints(hint)`.
fn with_hint(hint: DataAccessHint) -> Box<dyn IOContext> {
    let hint: Arc<dyn FileOpenHint> = Arc::new(hint);
    DEFAULT_IO_CONTEXT.with_hints(&[hint])
}

/// `new SerialIOCountingDirectory(newDirectory())` with ten pages of zeros
/// in `test`, as both tests start.
fn counting_directory_with_test_file() -> Result<SerialIOCountingDirectory> {
    let dir = SerialIOCountingDirectory::new(Box::new(OwnedTestDirectory::new(
        LuceneTestCase::new_directory(),
    )));
    let mut out = dir.create_output("test", &*DEFAULT_IO_CONTEXT)?;
    let written = (|| -> Result<()> {
        for _ in 0..10 {
            out.write_bytes(&[0u8; 4096], 0, 4096)?;
        }
        Ok(())
    })();
    written.and(out.close())?;
    Ok(dir)
}

/// `testSequentialReads()`.
fn test_sequential_reads() -> Result<()> {
    let mut dir = counting_directory_with_test_file()?;
    let result = (|| -> Result<()> {
        {
            let mut input = dir.open_input("test", &*with_hint(DataAccessHint::Sequential))?;
            input.read_byte()?;
            let count = dir.count();
            while input.file_pointer() < input.length() {
                input.read_byte()?;
            }
            // Sequential reads are free with the normal advice
            Assert::assert_equals(count, dir.count());
            input.close()?;
        }
        {
            let mut input = dir.open_input("test", &*with_hint(DataAccessHint::Random))?;
            input.read_byte()?;
            let count = dir.count();
            while input.file_pointer() < input.length() {
                input.read_byte()?;
            }
            // But not with the random advice
            if dir.count() == count {
                Assert::fail(&format!(
                    "\nExpected: not <{count}L>\n     but: was <{}L>",
                    dir.count()
                ));
            }
            input.close()?;
        }
        Ok(())
    })();
    result.and(dir.close())
}

/// `testParallelReads()`.
fn test_parallel_reads() -> Result<()> {
    let mut dir = counting_directory_with_test_file()?;
    let result = (|| -> Result<()> {
        let mut input = dir.open_input("test", &*with_hint(DataAccessHint::Random))?;
        let mut count = dir.count();

        // count is incremented on the first prefetch
        input.prefetch(5_000, 1)?;
        Assert::assert_equals(count + 1, dir.count());
        count = dir.count();

        // but not on the second one since it can be performed in parallel
        input.prefetch(10_000, 1)?;
        Assert::assert_equals(count, dir.count());

        // and reading from a prefetched page doesn't increment the counter
        input.seek(5_000)?;
        input.read_byte()?;
        Assert::assert_equals(count, dir.count());

        input.seek(10_000)?;
        input.read_byte()?;
        Assert::assert_equals(count, dir.count());

        // reading data on a page that was not prefetched increments the counter
        input.seek(15_000)?;
        input.read_byte()?;
        Assert::assert_equals(count + 1, dir.count());
        input.close()
    })();
    result.and(dir.close())
}

fn serialized_io_counting_directory_class() -> ClassBuilder {
    Class::builder("org.apache.lucene.tests.store.TestSerializedIOCountingDirectory")
        .extends(&base_directory_test_case::class())
        .constructor(Constructor::no_args(|| {
            DirectoryTestInstance::new(TestSerializedIOCountingDirectory)
        }))
        .method(directory_test("testSequentialReads", |_| {
            test_sequential_reads()
        }))
        .method(directory_test("testParallelReads", |_| {
            test_parallel_reads()
        }))
}

#[test]
fn test_serialized_io_counting_directory() {
    run_suite(serialized_io_counting_directory_class());
}
