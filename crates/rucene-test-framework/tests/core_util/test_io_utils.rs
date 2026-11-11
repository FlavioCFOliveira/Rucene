//! Port of `lucene/core/src/test/org/apache/lucene/util/TestIOUtils.java`.

use std::io;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::store::java_nio_file::{
    access_denied_exception, FileChannel, FileSystem, Files, OpenOption, Path,
};
use rucene::util::{Constants, IOUtils};
use rucene_test_framework::mockfile::filter_file_system_provider::{
    self as ffsp, install, FilterFileSystemProvider, FilterState,
};
use rucene_test_framework::mockfile::filter_path::FilterPath;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, test};

/// `TestIOUtils`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestIOUtils",
        vec![
            test(
                "testDeleteFileIgnoringExceptions",
                test_delete_file_ignoring_exceptions,
            ),
            test(
                "testDontDeleteFileIgnoringExceptions",
                test_dont_delete_file_ignoring_exceptions,
            ),
            test(
                "testDeleteTwoFilesIgnoringExceptions",
                test_delete_two_files_ignoring_exceptions,
            ),
            test("testDeleteFileIfExists", test_delete_file_if_exists),
            test("testDontDeleteDoesntExist", test_dont_delete_doesnt_exist),
            test("testDeleteTwoFilesIfExist", test_delete_two_files_if_exist),
            test("testFsyncDirectory", test_fsync_directory),
            test(
                "testFsyncAccessDeniedOpeningDirectory",
                test_fsync_access_denied_opening_directory,
            ),
            test(
                "testFsyncNonExistentDirectory",
                test_fsync_non_existent_directory,
            ),
            test("testFsyncFile", test_fsync_file),
            test("testApplyToAll", test_apply_to_all),
        ],
    )
}

fn test_delete_file_ignoring_exceptions() -> Result<()> {
    let dir = LuceneTestCase::create_temp_dir();
    let file1 = dir.resolve_str("file1");
    Files::create_file(&file1)?;
    IOUtils::delete_files_ignoring_exceptions([Some(&file1)]);
    Assert::assert_false(Files::exists(&file1, &[]));
    // actually deletes
    Ok(())
}

fn test_dont_delete_file_ignoring_exceptions() -> Result<()> {
    let dir = LuceneTestCase::create_temp_dir();
    let file1 = dir.resolve_str("file1");
    IOUtils::delete_files_ignoring_exceptions([Some(&file1)]);
    // no exception
    Ok(())
}

fn test_delete_two_files_ignoring_exceptions() -> Result<()> {
    let dir = LuceneTestCase::create_temp_dir();
    let file1 = dir.resolve_str("file1");
    let file2 = dir.resolve_str("file2");
    // only create file2
    Files::create_file(&file2)?;
    IOUtils::delete_files_ignoring_exceptions([Some(&file1), Some(&file2)]);
    Assert::assert_false(Files::exists(&file2, &[]));
    // no exception
    // actually deletes file2
    Ok(())
}

fn test_delete_file_if_exists() -> Result<()> {
    let dir = LuceneTestCase::create_temp_dir();
    let file1 = dir.resolve_str("file1");
    Files::create_file(&file1)?;
    IOUtils::delete_files_if_exist([Some(&file1)])?;
    Assert::assert_false(Files::exists(&file1, &[]));
    // actually deletes
    Ok(())
}

fn test_dont_delete_doesnt_exist() -> Result<()> {
    let dir = LuceneTestCase::create_temp_dir();
    let file1 = dir.resolve_str("file1");
    IOUtils::delete_files_if_exist([Some(&file1)])?;
    // no exception
    Ok(())
}

fn test_delete_two_files_if_exist() -> Result<()> {
    let dir = LuceneTestCase::create_temp_dir();
    let file1 = dir.resolve_str("file1");
    let file2 = dir.resolve_str("file2");
    // only create file2
    Files::create_file(&file2)?;
    IOUtils::delete_files_if_exist([Some(&file1), Some(&file2)])?;
    Assert::assert_false(Files::exists(&file2, &[]));
    // no exception
    // actually deletes file2
    Ok(())
}

fn test_fsync_directory() -> Result<()> {
    let mut dir = LuceneTestCase::create_temp_dir();
    dir = FilterPath::unwrap(&dir).to_real_path(&[])?;

    let devdir = dir.resolve_str("dev");
    Files::create_directories(&devdir)?;
    IOUtils::fsync(&devdir, true)?;
    // no exception
    Ok(())
}

/// `AccessDeniedWhileOpeningDirectoryFileSystem extends
/// FilterFileSystemProvider`.
struct AccessDeniedWhileOpeningDirectoryFileSystem {
    filter: FilterState,
}

impl AccessDeniedWhileOpeningDirectoryFileSystem {
    /// `AccessDeniedWhileOpeningDirectoryFileSystem(delegate)`: `super("access_denied://",
    /// Objects.requireNonNull(delegate))`.
    fn new(delegate: Arc<dyn FileSystem>) -> Arc<AccessDeniedWhileOpeningDirectoryFileSystem> {
        let this = Arc::new(AccessDeniedWhileOpeningDirectoryFileSystem {
            filter: FilterState::new("access_denied://", &delegate),
        });
        install(&this, delegate);
        this
    }
}

impl FilterFileSystemProvider for AccessDeniedWhileOpeningDirectoryFileSystem {
    fn filter(&self) -> &FilterState {
        &self.filter
    }

    fn class_name(&self) -> &'static str {
        "org.apache.lucene.util.TestIOUtils$AccessDeniedWhileOpeningDirectoryFileSystem"
    }

    fn new_file_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Arc<dyn FileChannel>> {
        if Files::is_directory(path, &[]) {
            return Err(access_denied_exception(path.to_string()));
        }
        ffsp::new_file_channel(self, path, options)
    }
}

rucene_test_framework::impl_file_system_provider!(AccessDeniedWhileOpeningDirectoryFileSystem);

fn test_fsync_access_denied_opening_directory() -> Result<()> {
    let path = LuceneTestCase::create_temp_dir().to_real_path(&[])?;
    let provider =
        AccessDeniedWhileOpeningDirectoryFileSystem::new(Arc::clone(path.get_file_system()));
    let wrapped = provider.wrap_path(&path);
    if Constants::windows() {
        // no exception, we early return and do not even try to open the directory
        IOUtils::fsync(&wrapped, true)?;
    } else {
        expect_throws("java.nio.file.AccessDeniedException", || {
            IOUtils::fsync(&wrapped, true)
        });
    }
    Ok(())
}

fn test_fsync_non_existent_directory() -> Result<()> {
    let dir = FilterPath::unwrap(&LuceneTestCase::create_temp_dir()).to_real_path(&[])?;
    let non_existent_dir = dir.resolve_str("non-existent");
    expect_throws("java.nio.file.NoSuchFileException", || {
        IOUtils::fsync(&non_existent_dir, true)
    });
    Ok(())
}

fn test_fsync_file() -> Result<()> {
    let mut dir = LuceneTestCase::create_temp_dir();
    dir = FilterPath::unwrap(&dir).to_real_path(&[])?;

    let devdir = dir.resolve_str("dev");
    Files::create_directories(&devdir)?;
    let somefile = devdir.resolve_str("somefile");
    {
        let mut o = Files::new_output_stream(&somefile, &[])?;
        o.write(b"0\n", 0, 2)?;
        o.close()?;
    }
    IOUtils::fsync(&somefile, false)?;
    // no exception
    Ok(())
}

fn test_apply_to_all() -> Result<()> {
    let closed: Mutex<Vec<i32>> = Mutex::new(Vec::new());
    let runtime_exception = expect_throws("java.lang.RuntimeException", || {
        IOUtils::apply_to_all(vec![Some(1), Some(2)], |i| {
            closed.lock().unwrap_or_else(|e| e.into_inner()).push(i);
            Err(LuceneError::from(LuceneErrorKind::Unchecked {
                class: "java.lang.RuntimeException",
                message: Some(format!("{i}")),
                cause: None,
            }))
        })
    });
    assert_equals(&Some("1"), &runtime_exception.message());
    assert_equals(&1, &runtime_exception.suppressed().len());
    assert_equals(&Some("2"), &runtime_exception.suppressed()[0].message());
    let closed = closed.lock().unwrap_or_else(|e| e.into_inner());
    assert_equals(&2, &closed.len());
    assert_equals(&1, &closed[0]);
    assert_equals(&2, &closed[1]);
    Ok(())
}
