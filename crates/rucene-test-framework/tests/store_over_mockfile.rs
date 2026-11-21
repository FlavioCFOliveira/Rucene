//! The store over the mock filesystems: an `FSDirectory` opened on a path of
//! a mock filesystem performs every file operation through it, so the faults
//! the filesystem injects reach the store as they reach Lucene's.

mod oracle_support;

use std::sync::Arc;

use oracle_support::{lucene_error, TempDir};
use rucene::store::java_nio_file::{default_file_system, FileSystemProvider};
use rucene::store::{Directory, NIOFSDirectory, DEFAULT_IO_CONTEXT};
use rucene_test_framework::mockfile::{LeakFS, WindowsFS};

/// `LeakFS`: an output left open is a leak at close; closing it clears it.
#[test]
fn leak_fs_sees_an_unclosed_index_output() {
    let tmp = TempDir::new("store-leak");
    let fs = LeakFS::new(default_file_system()).get_file_system();
    let root = Arc::clone(&fs).get_path(&tmp.str(), &[]);
    let dir = NIOFSDirectory::open(root.resolve_str("idx")).unwrap();
    let mut out = dir.create_output("a", &*DEFAULT_IO_CONTEXT).unwrap();
    out.write_int(1).unwrap();
    let leak = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fs.close())).unwrap_err();
    let t = randomizedtesting::jdk::exception::Throwable::from_panic(leak);
    assert_eq!(t.class_name(), "java.lang.RuntimeException");
    assert!(
        t.message()
            .unwrap()
            .starts_with("file handle leaks: [OutputStream("),
        "{t}"
    );
    out.close().unwrap();
    fs.close().unwrap();
}

/// `WindowsFS`: deleting a file an input holds open fails, and `FSDirectory`
/// keeps it pending, as on Windows; it is deleted once the input is closed.
#[test]
fn windows_fs_makes_deletes_of_open_files_pending() {
    let tmp = TempDir::new("store-windows");
    let fs = WindowsFS::new(default_file_system()).get_file_system();
    let root = Arc::clone(&fs).get_path(&tmp.str(), &[]);
    let dir = NIOFSDirectory::open(root.resolve_str("idx")).unwrap();
    let mut out = dir.create_output("a", &*DEFAULT_IO_CONTEXT).unwrap();
    out.write_int(1).unwrap();
    out.close().unwrap();
    let mut input = dir.open_input("a", &*DEFAULT_IO_CONTEXT).unwrap();
    dir.delete_file("a").unwrap();
    assert!(dir.get_pending_deletions().unwrap().contains("a"));
    assert_eq!(dir.list_all().unwrap(), Vec::<String>::new());
    let err = dir.open_input("a", &*DEFAULT_IO_CONTEXT).err().unwrap();
    assert_eq!(
        lucene_error(&err),
        (
            "java.nio.file.NoSuchFileException".to_string(),
            "file \"a\" is pending delete and cannot be opened for read".to_string()
        )
    );
    input.close().unwrap();
    assert!(dir.get_pending_deletions().unwrap().is_empty());
}
