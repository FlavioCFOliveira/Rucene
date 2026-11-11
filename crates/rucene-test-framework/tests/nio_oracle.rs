//! Task #331 oracle parity: the store's `java.nio.file` layer against JDK 25
//! and Lucene 10.5.0 (`tests/fixtures/nio_default.tsv` and
//! `nio_store_trace.tsv`, written by `NioOracle.java` of the workspace's
//! oracle harness).
//!
//! * The default provider runs the oracle's operation script and must report
//!   the same results, exception classes and messages.
//! * `FSDirectory`, `NIOFSDirectory`, `MMapDirectory`, both filesystem lock
//!   factories and `IOUtils.rm` run the oracle's scenario over a recording
//!   provider and must make the same provider calls in the same order: the
//!   recording provider is the counting provider of the positive control,
//!   observing every file operation the store makes.

mod oracle_support;

use std::collections::BTreeMap;
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

use oracle_support::{assert_same, io_error, labelled, lucene_error, mask, panic_error, TempDir};
use rucene::store::java_nio_file::{
    default_file_system, AccessMode, BasicFileAttributes, CopyOption, DirectoryStream,
    DirectoryStreamFilter, FileChannel, FileLock, FileSystem, FileSystemProvider, Files,
    LinkOption, OpenOption, OutputStream, Path, SeekableByteChannel,
};
use rucene::store::{
    mmap::MMapDirectory, Directory, FSDirectory, NIOFSDirectory, SimpleFSLockFactory,
    DEFAULT_IO_CONTEXT,
};
use rucene::util::IOUtils;
use rucene_test_framework::mockfile::filter_file_system_provider::{self as ffsp, install};
use rucene_test_framework::mockfile::filter_output_stream2::{
    FilterOutputStream2, OutputStreamOverrides,
};
use rucene_test_framework::mockfile::filter_seekable_byte_channel::{
    FilterSeekableByteChannel, SeekableByteChannelOverrides,
};
use rucene_test_framework::mockfile::{FilterFileSystemProvider, FilterState};

fn render<T: std::fmt::Display>(v: T) -> String {
    v.to_string()
}

fn list(v: &[String]) -> String {
    format!("[{}]", v.join(", "))
}

/// One row of the script: `OK\tresult` or `class\tmessage`.
fn row(
    rows: &mut BTreeMap<String, String>,
    dir: &str,
    label: &str,
    op: impl FnOnce() -> io::Result<String>,
) {
    let line = match catch_unwind(AssertUnwindSafe(op)) {
        Ok(Ok(r)) => format!("OK\t{}", mask(&r, dir, None)),
        Ok(Err(e)) => {
            let (c, m) = io_error(&e);
            format!("{c}\t{}", mask(&m, dir, None))
        }
        Err(p) => {
            let (c, m) = panic_error(p);
            format!("{c}\t{}", mask(&m, dir, None))
        }
    };
    rows.insert(label.to_string(), line);
}

fn names(mut s: Box<dyn DirectoryStream>) -> io::Result<Vec<String>> {
    let mut l = Vec::new();
    for p in s.iterator() {
        l.push(p?.get_file_name().unwrap().to_string());
    }
    s.close()?;
    Ok(l)
}

#[test]
fn default_provider_matches_jdk25() {
    let tmp = TempDir::new("nio-oracle");
    let ds = tmp.str();
    let d = Path::new(default_file_system(), &ds);
    let r = &mut BTreeMap::new();
    let nope = d.resolve_str("nope");
    let p = |s: &str| d.resolve_str(s);
    row(r, &ds, "delete missing", || {
        Files::delete(&nope).map(|_| "-".into())
    });
    row(r, &ds, "deleteIfExists missing", || {
        Files::delete_if_exists(&nope).map(render)
    });
    row(r, &ds, "createDirectory sub", || {
        Files::create_directory(&p("sub")).map(render)
    });
    row(r, &ds, "write sub/f", || {
        Files::write(&p("sub/f"), &[1, 2, 3], &[]).map(render)
    });
    row(r, &ds, "delete nonempty", || {
        Files::delete(&p("sub")).map(|_| "-".into())
    });
    row(r, &ds, "createDirectory exists", || {
        Files::create_directory(&p("sub")).map(render)
    });
    row(r, &ds, "createDirectory noparent", || {
        Files::create_directory(&p("x/y")).map(render)
    });
    row(r, &ds, "createDirectories nested", || {
        Files::create_directories(&p("p/q/r")).map(render)
    });
    row(r, &ds, "createDirectories under file", || {
        Files::create_directories(&p("sub/f/x")).map(render)
    });
    row(r, &ds, "createDirectories on file", || {
        Files::create_directories(&p("sub/f")).map(render)
    });
    row(r, &ds, "createFile exists", || {
        Files::create_file(&p("sub/f")).map(render)
    });
    row(r, &ds, "newOutputStream CREATE_NEW exists", || {
        Files::new_output_stream(&p("sub/f"), &[OpenOption::Write, OpenOption::CreateNew])?
            .close()?;
        Ok("-".into())
    });
    row(r, &ds, "newOutputStream READ", || {
        Files::new_output_stream(&p("sub/g"), &[OpenOption::Read]).map(|s| s.stream_to_string())
    });
    row(r, &ds, "newInputStream WRITE", || {
        Files::new_input_stream(&p("sub/f"), &[OpenOption::Write]).map(|s| s.stream_to_string())
    });
    row(r, &ds, "FileChannel READ missing", || {
        <dyn FileChannel>::open(&nope, &[OpenOption::Read]).map(|c| c.channel_to_string())
    });
    row(r, &ds, "FileChannel WRITE dir", || {
        <dyn FileChannel>::open(&p("sub"), &[OpenOption::Write]).map(|c| c.channel_to_string())
    });
    row(r, &ds, "FileChannel READ APPEND", || {
        <dyn FileChannel>::open(&p("sub/f"), &[OpenOption::Read, OpenOption::Append])
            .map(|c| c.channel_to_string())
    });
    row(r, &ds, "FileChannel APPEND TRUNCATE", || {
        <dyn FileChannel>::open(
            &p("sub/f"),
            &[OpenOption::Append, OpenOption::TruncateExisting],
        )
        .map(|c| c.channel_to_string())
    });
    row(r, &ds, "size", || Files::size(&p("sub/f")).map(render));
    row(r, &ds, "size missing", || Files::size(&nope).map(render));
    row(r, &ds, "move missing ATOMIC", || {
        Files::r#move(&nope, &p("nope2"), &[CopyOption::AtomicMove]).map(render)
    });
    row(r, &ds, "createDirectory sub2", || {
        Files::create_directory(&p("sub2")).map(render)
    });
    row(r, &ds, "write sub2/g", || {
        Files::write(&p("sub2/g"), &[1], &[]).map(render)
    });
    row(r, &ds, "move onto nonempty dir ATOMIC", || {
        Files::r#move(&p("sub/f"), &p("sub2"), &[CopyOption::AtomicMove]).map(render)
    });
    row(r, &ds, "move onto existing", || {
        Files::r#move(&p("sub/f"), &p("sub2/g"), &[]).map(render)
    });
    row(r, &ds, "move onto existing REPLACE", || {
        Files::r#move(&p("sub/f"), &p("sub2/g"), &[CopyOption::ReplaceExisting]).map(render)
    });
    row(r, &ds, "copy back", || {
        Files::copy(&p("sub2/g"), &p("sub/f"), &[]).map(render)
    });
    row(r, &ds, "copy onto existing", || {
        Files::copy(&p("sub2/g"), &p("sub/f"), &[]).map(render)
    });
    row(r, &ds, "toRealPath missing", || {
        nope.to_real_path(&[]).map(render)
    });
    row(r, &ds, "newDirectoryStream missing", || {
        Files::new_directory_stream(&nope).map(|s| s.stream_to_string())
    });
    row(r, &ds, "newDirectoryStream file", || {
        Files::new_directory_stream(&p("sub/f")).map(|s| s.stream_to_string())
    });
    row(r, &ds, "readSymbolicLink file", || {
        Files::read_symbolic_link(&p("sub/f")).map(render)
    });
    row(r, &ds, "createLink exists", || {
        Files::create_link(&p("sub/f"), &p("sub2/g")).map(render)
    });
    row(r, &ds, "exists", || {
        Ok(render(Files::exists(&p("sub/f"), &[])))
    });
    row(r, &ds, "isDirectory", || {
        Ok(render(Files::is_directory(&p("sub"), &[])))
    });
    row(r, &ds, "isRegularFile", || {
        Ok(render(Files::is_regular_file(&p("sub/f"), &[])))
    });
    row(r, &ds, "notExists", || {
        Ok(render(Files::not_exists(&nope, &[])))
    });
    row(r, &ds, "isHidden", || {
        Files::is_hidden(&p(".x")).map(render)
    });
    row(r, &ds, "isSameFile", || {
        Files::is_same_file(&p("sub/f"), &p("sub/../sub/f")).map(render)
    });
    row(r, &ds, "readAttributes basic", || {
        let a = Files::read_attributes_by_name(
            &p("sub/f"),
            "basic:size,isRegularFile,isDirectory",
            &[],
        )?;
        let parts: Vec<String> = a.iter().map(|(k, v)| format!("{k}={v}")).collect();
        Ok(format!("{{{}}}", parts.join(", ")))
    });
    row(r, &ds, "readAttributes view", || {
        Files::read_attributes_by_name(&p("sub/f"), "foo:size", &[]).map(|_| "-".into())
    });
    row(r, &ds, "getAttribute bogus", || {
        Files::read_attributes_by_name(&p("sub/f"), "basic:bogus", &[]).map(|_| "-".into())
    });
    row(r, &ds, "setAttribute bogus", || {
        Files::set_attribute(
            &p("sub/f"),
            "basic:bogus",
            &rucene::store::java_nio_file::AttributeValue::Long(1),
            &[],
        )
        .map(render)
    });
    row(r, &ds, "readAllLines", || {
        Files::write(&p("lines"), b"a\nb\r\nc", &[])?;
        Files::read_all_lines(&p("lines")).map(|l| list(&l))
    });
    row(r, &ds, "list sorted", || {
        let mut l = names(Files::new_directory_stream(&d)?)?;
        l.sort();
        Ok(list(&l))
    });
    row(r, &ds, "glob", || {
        let mut l = names(Files::new_directory_stream_glob(&d, "s*")?)?;
        l.sort();
        Ok(list(&l))
    });
    row(r, &ds, "IOUtils.rm", || {
        IOUtils::rm([p("p")]).map_err(|e| io::Error::other(e.to_string()))?;
        Ok(render(Files::exists(&p("p"), &[])))
    });
    row(r, &ds, "uri", || Ok(p("ä b%").to_uri()));
    row(r, &ds, "uri round trip", || {
        Ok(render(
            default_file_system()
                .provider()
                .get_path(&p("ä b%").to_uri()),
        ))
    });
    let provider = default_file_system().provider();
    row(r, &ds, "provider getPath authority", || {
        Ok(render(provider.get_path("file://host/x")))
    });
    row(r, &ds, "provider getPath scheme", || {
        Ok(render(provider.get_path("http:///x")))
    });
    row(r, &ds, "provider getPath query", || {
        Ok(render(provider.get_path("file:///x?q")))
    });
    row(r, &ds, "provider getPath fragment", || {
        Ok(render(provider.get_path("file:///x#f")))
    });
    row(r, &ds, "getPath nul", || {
        Ok(render(Path::new(default_file_system(), "a\0b")))
    });
    row(r, &ds, "pathMatcher syntax", || {
        default_file_system().get_path_matcher("foo:x");
        Ok("-".into())
    });
    row(r, &ds, "pathMatcher none", || {
        default_file_system().get_path_matcher("x");
        Ok("-".into())
    });
    row(r, &ds, "close default", || {
        default_file_system().close().map(|_| "-".into())
    });
    row(r, &ds, "views", || {
        let mut v = default_file_system().supported_file_attribute_views();
        v.sort();
        Ok(list(&v))
    });
    row(r, &ds, "IOUtils.fsync missing", || {
        IOUtils::fsync(&nope, false).map_err(|e| match e {
            rucene::error::LuceneError {
                kind: rucene::error::LuceneErrorKind::Io(io),
                ..
            } => io,
            other => io::Error::other(other.to_string()),
        })?;
        Ok("-".into())
    });
    row(r, &ds, "IOUtils.fsync dir", || {
        IOUtils::fsync(&d, true).map_err(|e| io::Error::other(e.to_string()))?;
        Ok("-".into())
    });
    row(r, &ds, "IOUtils.deleteFilesIfExist", || {
        IOUtils::delete_files_if_exist([Some(p("sub2/g")), Some(nope.clone())])
            .map_err(|e| io::Error::other(e.to_string()))?;
        Ok(render(Files::exists(&p("sub2/g"), &[])))
    });
    row(r, &ds, "FileLock", || {
        let fc = <dyn FileChannel>::open(&p("lock"), &[OpenOption::Create, OpenOption::Write])?;
        let l = fc.try_lock_whole()?.unwrap();
        let a = l.lock_to_string();
        fc.close()?;
        Ok(format!("{a} {} {}", l.lock_to_string(), l.is_valid()))
    });
    row(r, &ds, "closed channel size", || {
        let fc = <dyn FileChannel>::open(&p("lock"), &[OpenOption::Read])?;
        fc.close()?;
        fc.size().map(render)
    });
    assert_same("default provider", &labelled("nio_default.tsv"), r);
}

// -----------------------------------------------------------------------------
// The store over a recording provider.
// -----------------------------------------------------------------------------

/// The trace the recording provider appends to.
static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn rec(s: String) {
    TRACE.lock().unwrap().push(s);
}

fn opts<T: std::fmt::Display>(o: &[T]) -> String {
    let mut s: Vec<String> = o.iter().map(ToString::to_string).collect();
    s.sort();
    format!("[{}]", s.join(", "))
}

/// The oracle's `RecordingFS`: records every provider call, and the closes,
/// forces, sizes and locks of the channels.
struct RecordingFS {
    filter: FilterState,
}

impl RecordingFS {
    fn new(delegate: Arc<dyn FileSystem>) -> Arc<RecordingFS> {
        let this = Arc::new(RecordingFS {
            filter: FilterState::new("record://", &delegate),
        });
        install(&this, delegate);
        this
    }
}

struct RecordingChannel {
    path: Path,
    inner: Arc<dyn FileChannel>,
    open: std::sync::atomic::AtomicBool,
}

impl SeekableByteChannel for RecordingChannel {
    fn read(&self, dst: &mut [u8]) -> io::Result<i32> {
        self.inner.read(dst)
    }
    fn write(&self, src: &[u8]) -> io::Result<i32> {
        self.inner.write(src)
    }
    fn position(&self) -> io::Result<i64> {
        self.inner.position()
    }
    fn set_position(&self, p: i64) -> io::Result<()> {
        self.inner.set_position(p)
    }
    fn size(&self) -> io::Result<i64> {
        rec(format!("size {}", self.path));
        self.inner.size()
    }
    fn truncate(&self, s: i64) -> io::Result<()> {
        self.inner.truncate(s)
    }
    fn is_open(&self) -> bool {
        self.open.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn close(&self) -> io::Result<()> {
        if self.open.swap(false, std::sync::atomic::Ordering::SeqCst) {
            rec(format!("close FileChannel {}", self.path));
            self.inner.close()
        } else {
            Ok(())
        }
    }
    fn channel_to_string(&self) -> String {
        "RecordingChannel".into()
    }
}

impl FileChannel for RecordingChannel {
    fn read_scattering(&self, dsts: &mut [&mut [u8]]) -> io::Result<i64> {
        self.inner.read_scattering(dsts)
    }
    fn write_gathering(&self, srcs: &[&[u8]]) -> io::Result<i64> {
        self.inner.write_gathering(srcs)
    }
    fn force(&self, meta: bool) -> io::Result<()> {
        rec(format!("force {meta} {}", self.path));
        self.inner.force(meta)
    }
    fn transfer_to(&self, p: i64, c: i64, t: &dyn SeekableByteChannel) -> io::Result<i64> {
        self.inner.transfer_to(p, c, t)
    }
    fn transfer_from(&self, s: &dyn SeekableByteChannel, p: i64, c: i64) -> io::Result<i64> {
        self.inner.transfer_from(s, p, c)
    }
    fn read_at(&self, dst: &mut [u8], p: i64) -> io::Result<i32> {
        self.inner.read_at(dst, p)
    }
    fn write_at(&self, src: &[u8], p: i64) -> io::Result<i32> {
        self.inner.write_at(src, p)
    }
    fn lock(&self, p: i64, s: i64, sh: bool) -> io::Result<Box<dyn FileLock>> {
        self.inner.lock(p, s, sh)
    }
    fn try_lock(&self, p: i64, s: i64, sh: bool) -> io::Result<Option<Box<dyn FileLock>>> {
        rec(format!("tryLock {}", self.path));
        self.inner.try_lock(p, s, sh)
    }
}

struct RecordingClose(String);

impl OutputStreamOverrides for RecordingClose {
    fn close(&mut self, delegate: &mut dyn OutputStream) -> io::Result<()> {
        rec(self.0.clone());
        delegate.close()
    }
}

impl SeekableByteChannelOverrides for RecordingClose {
    fn close(&self, delegate: &dyn SeekableByteChannel) -> io::Result<()> {
        rec(self.0.clone());
        delegate.close()
    }
}

struct RecordingStream {
    dir: Path,
    inner: Box<dyn DirectoryStream>,
}

impl DirectoryStream for RecordingStream {
    fn iterator(&mut self) -> Box<dyn Iterator<Item = io::Result<Path>> + Send + '_> {
        self.inner.iterator()
    }
    fn close(&mut self) -> io::Result<()> {
        rec(format!("close DirectoryStream {}", self.dir));
        self.inner.close()
    }
    fn stream_to_string(&self) -> String {
        "RecordingStream".into()
    }
}

impl FilterFileSystemProvider for RecordingFS {
    fn filter(&self) -> &FilterState {
        &self.filter
    }
    fn class_name(&self) -> &'static str {
        "org.apache.lucene.tests.mockfile.NioOracle$RecordingFS"
    }
    fn create_directory(&self, dir: &Path) -> io::Result<()> {
        rec(format!("createDirectory {dir}"));
        ffsp::create_directory(self, dir)
    }
    fn delete(&self, path: &Path) -> io::Result<()> {
        rec(format!("delete {path}"));
        ffsp::delete(self, path)
    }
    fn delete_if_exists(&self, path: &Path) -> io::Result<bool> {
        rec(format!("deleteIfExists {path}"));
        ffsp::delete_if_exists(self, path)
    }
    fn copy(&self, s: &Path, t: &Path, o: &[CopyOption]) -> io::Result<()> {
        rec(format!("copy {} {s} {t}", opts(o)));
        ffsp::copy(self, s, t, o)
    }
    fn r#move(&self, s: &Path, t: &Path, o: &[CopyOption]) -> io::Result<()> {
        rec(format!("move {} {s} {t}", opts(o)));
        ffsp::r#move(self, s, t, o)
    }
    fn check_access(&self, path: &Path, modes: &[AccessMode]) -> io::Result<()> {
        let names: Vec<&str> = modes
            .iter()
            .map(|m| match m {
                AccessMode::Read => "READ",
                AccessMode::Write => "WRITE",
                AccessMode::Execute => "EXECUTE",
            })
            .collect();
        rec(format!("checkAccess {} {path}", opts(&names)));
        let f = self.filter();
        f.delegate().check_access(&f.to_delegate(path), modes)
    }
    fn read_attributes(&self, path: &Path, o: &[LinkOption]) -> io::Result<BasicFileAttributes> {
        rec(format!("readAttributes {} {path}", opts(o)));
        let f = self.filter();
        f.delegate().read_attributes(&f.to_delegate(path), o)
    }
    fn new_input_stream(
        &self,
        path: &Path,
        o: &[OpenOption],
    ) -> io::Result<Box<dyn rucene::store::java_nio_file::InputStream>> {
        rec(format!("newInputStream {} {path}", opts(o)));
        ffsp::new_input_stream(self, path, o)
    }
    fn new_output_stream(
        &self,
        path: &Path,
        o: &[OpenOption],
    ) -> io::Result<Box<dyn OutputStream>> {
        rec(format!("newOutputStream {} {path}", opts(o)));
        Ok(Box::new(FilterOutputStream2::with_overrides(
            ffsp::new_output_stream(self, path, o)?,
            Box::new(RecordingClose(format!("close OutputStream {path}"))),
            "RecordingOutputStream",
        )))
    }
    fn new_file_channel(&self, path: &Path, o: &[OpenOption]) -> io::Result<Arc<dyn FileChannel>> {
        rec(format!("newFileChannel {} {path}", opts(o)));
        Ok(Arc::new(RecordingChannel {
            path: path.clone(),
            inner: ffsp::new_file_channel(self, path, o)?,
            open: std::sync::atomic::AtomicBool::new(true),
        }))
    }
    fn new_byte_channel(
        &self,
        path: &Path,
        o: &[OpenOption],
    ) -> io::Result<Box<dyn SeekableByteChannel>> {
        rec(format!("newByteChannel {} {path}", opts(o)));
        Ok(Box::new(FilterSeekableByteChannel::with_overrides(
            ffsp::new_byte_channel(self, path, o)?,
            Box::new(RecordingClose(format!("close SeekableByteChannel {path}"))),
            "RecordingByteChannel",
        )))
    }
    fn new_directory_stream(
        &self,
        dir: &Path,
        filter: DirectoryStreamFilter,
    ) -> io::Result<Box<dyn DirectoryStream>> {
        rec(format!("newDirectoryStream {dir}"));
        Ok(Box::new(RecordingStream {
            dir: dir.clone(),
            inner: ffsp::new_directory_stream(self, dir, filter)?,
        }))
    }
}

rucene_test_framework::impl_file_system_provider!(RecordingFS);

fn step(
    rows: &mut BTreeMap<String, String>,
    dir: &str,
    name: &str,
    s: impl FnOnce() -> rucene::error::Result<()>,
) {
    TRACE.lock().unwrap().clear();
    let outcome = match catch_unwind(AssertUnwindSafe(s)) {
        Ok(Ok(())) => "OK".to_string(),
        Ok(Err(e)) => {
            let (c, m) = lucene_error(&e);
            format!("{c} {}", mask(&m, dir, None))
        }
        Err(p) => {
            let (c, m) = panic_error(p);
            format!("{c} {}", mask(&m, dir, None))
        }
    };
    let trace: Vec<String> = TRACE
        .lock()
        .unwrap()
        .iter()
        .map(|t| mask(t, dir, None))
        .collect();
    rows.insert(
        name.to_string(),
        format!("{outcome}\t{}", trace.join(" | ")),
    );
}

#[test]
fn store_calls_match_lucene_over_a_recording_provider() {
    let tmp = TempDir::new("nio-trace");
    let ds = tmp.str();
    let provider = RecordingFS::new(default_file_system());
    let fs = FileSystemProvider::get_file_system(&*provider);
    let root = Arc::clone(&fs).get_path(&ds, &[]);
    let r = &mut BTreeMap::new();
    let ctx = &*DEFAULT_IO_CONTEXT;
    let mut nio: Option<NIOFSDirectory> = None;
    step(r, &ds, "NIOFSDirectory.<init>", || {
        nio = Some(NIOFSDirectory::open(root.resolve_str("idx"))?);
        Ok(())
    });
    let mut dir = nio.expect("NIOFSDirectory opened");
    step(r, &ds, "createOutput", || {
        let mut o = dir.create_output("_0.cfs", ctx)?;
        o.write_bytes(&[0u8; 64], 0, 64)?;
        o.close()
    });
    step(r, &ds, "sync", || dir.sync(&["_0.cfs".to_string()]));
    let mut temp_name = String::new();
    step(r, &ds, "createTempOutput", || {
        let mut o = dir.create_temp_output("pending", "tmp", ctx)?;
        o.write_int(7)?;
        o.close()?;
        temp_name = o.name().to_string();
        Ok(())
    });
    step(r, &ds, "rename", || dir.rename(&temp_name, "segments_1"));
    step(r, &ds, "syncMetaData", || dir.sync_metadata());
    step(r, &ds, "fileLength", || {
        dir.file_length("segments_1").map(|_| ())
    });
    step(r, &ds, "openInput", || {
        let mut i = dir.open_input("segments_1", ctx)?;
        i.read_byte()?;
        i.close()
    });
    step(r, &ds, "listAll", || dir.list_all().map(|_| ()));
    step(r, &ds, "obtainLock", || {
        let l = dir.obtain_lock("write.lock")?;
        l.ensure_valid()?;
        l.close()
    });
    step(r, &ds, "deleteFile", || dir.delete_file("_0.cfs"));
    step(r, &ds, "deleteFile missing", || dir.delete_file("_0.cfs"));
    step(r, &ds, "close", || dir.close());
    step(r, &ds, "MMapDirectory", || {
        let mut mm = MMapDirectory::open(root.resolve_str("mm"))?;
        let mut o = mm.create_output("a", ctx)?;
        o.write_long(1)?;
        o.close()?;
        let mut i = mm.open_input("a", ctx)?;
        i.read_long()?;
        i.close()?;
        mm.close()
    });
    step(r, &ds, "SimpleFSLockFactory", || {
        let mut s = FSDirectory::open_with_lock_factory(
            root.resolve_str("s"),
            Box::new(SimpleFSLockFactory),
        )?;
        let l = s.obtain_lock("write.lock")?;
        l.ensure_valid()?;
        l.close()?;
        s.close()
    });
    step(r, &ds, "IOUtils.rm", || {
        IOUtils::rm([
            root.resolve_str("idx"),
            root.resolve_str("mm"),
            root.resolve_str("s"),
        ])
    });
    let expected = labelled("nio_store_trace.tsv");
    assert_same("store trace", &expected, r);

    // Positive control: every step but `close` reached the provider.
    for (name, line) in r.iter() {
        let trace = line.split_once('\t').map(|(_, t)| t).unwrap_or("");
        if name != "close" {
            assert!(!trace.is_empty(), "{name} made no provider call");
        }
    }
}
