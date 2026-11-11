//! Parity of `org.apache.lucene.tests.store` (rmp #343) with Lucene 10.5.0:
//! `MockDirectoryWrapper` and its streams, `SerialIOCountingDirectory`,
//! `CorruptingIndexOutput`, `RawDirectoryWrapper` and the slow open/closers,
//! against `mdw_steps.tsv` (`MockDirectoryWrapperOracle`, see the oracle
//! harness README).
//!
//! Each scenario below is the oracle's, step for step: the same calls on a
//! wrapper built from the same seed, recorded as the same rows (call, then
//! `ok<TAB>value` or `err<TAB>class<TAB>message`, with the cause of a
//! `RuntimeException`). The index scenarios read the files of the index the
//! oracle wrote (`mdw_index.tsv`); the `index_read` scenarios read the corpus
//! it wrote (`mdw_corpus.tsv`, `nocfs` and `cfs`) in full through the
//! wrapper — a `DirectoryReader` opened and closed, `CheckIndex` — and then
//! close the wrapper, which fails while any reader left an input open.
//!
//! The one deliberate difference in the scripts: before closing a wrapper,
//! the Rust script drops the streams it still holds, since a stream keeps the
//! wrapper's state shared and the delegate is closed only when it is not
//! (the declared divergence of `MockDirectoryWrapper::close`).

mod oracle_support;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Once};

use randomizedtesting::jdk::random::JdkRandom;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::check_index::Level as CheckIndexLevel;
use rucene::index::{directory_reader, IndexReader};
use rucene::store::java_nio_file::{java_class_of, java_message_of};
use rucene::store::{
    ByteArrayDataInput, ByteBuffersDirectory, ChecksumIndexInput, DataAccessHint, DataInput,
    DataOutput, Directory, FileOpenHint, FileTypeHint, IOContext, IndexInput, IndexOutput,
    DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT,
};
use rucene_test_framework::store::{
    fake_io_exception, CorruptingIndexOutput, Failure, MockDirectoryWrapper, MockIndexInputWrapper,
    RawDirectoryWrapper, SerialIOCountingDirectory, SlowClosingMockIndexInputWrapper,
    SlowOpeningMockIndexInputWrapper, Throttling,
};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let loader = rucene_test_framework::class_loader();
        rucene::codecs::reload_codecs(loader).unwrap();
        rucene::codecs::postings::reload_postings_formats(loader).unwrap();
        rucene::codecs::knn_vectors::reload_knn_vectors_format(loader).unwrap();
        rucene::codecs::doc_values::reload_doc_values_formats(loader).unwrap();
    });
}

// -----------------------------------------------------------------------------
// Recording
// -----------------------------------------------------------------------------

/// The oracle's `m(o)`: identity hashes masked, line breaks and tabs escaped.
fn m(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '@' {
            let mut hex = false;
            while let Some(&n) = chars.peek() {
                if n.is_ascii_digit() || ('a'..='f').contains(&n) {
                    hex = true;
                    chars.next();
                } else {
                    break;
                }
            }
            if hex {
                out.push_str("HASH");
            }
        }
    }
    out.replace('\n', "\\n").replace('\t', "\\t")
}

/// `class` and `getMessage()` of an error, as Java names them.
fn class_and_message(e: &LuceneError) -> (String, String) {
    let null = || "<null>".to_string();
    match e {
        LuceneError {
            kind: LuceneErrorKind::Io(io),
            ..
        } => (
            java_class_of(io).to_string(),
            java_message_of(io).unwrap_or_else(null),
        ),
        LuceneError {
            kind: LuceneErrorKind::Unchecked { class, message, .. },
            ..
        } => (class.to_string(), message.clone().unwrap_or_else(null)),
        LuceneError {
            kind: LuceneErrorKind::AssertionError(message),
            ..
        } => ("java.lang.AssertionError".to_string(), message.clone()),
        LuceneError {
            kind: LuceneErrorKind::AlreadyClosed { message, .. },
            ..
        } => (
            "org.apache.lucene.store.AlreadyClosedException".to_string(),
            message.clone(),
        ),
        LuceneError {
            kind: LuceneErrorKind::IllegalState { message, .. },
            ..
        } => (
            "java.lang.IllegalStateException".to_string(),
            message.clone(),
        ),
        LuceneError {
            kind: LuceneErrorKind::IllegalArgument { message, .. },
            ..
        } => (
            "java.lang.IllegalArgumentException".to_string(),
            if message.is_empty() {
                null()
            } else {
                message.clone()
            },
        ),
        LuceneError {
            kind: LuceneErrorKind::CorruptIndex(message),
            ..
        } => (
            "org.apache.lucene.index.CorruptIndexException".to_string(),
            message.clone(),
        ),
        LuceneError {
            kind: LuceneErrorKind::IndexNotFound(message),
            ..
        } => (
            "org.apache.lucene.index.IndexNotFoundException".to_string(),
            message.clone(),
        ),
        other => (
            format!("rucene::{}", other.java_simple_name()),
            other.to_string(),
        ),
    }
}

/// The oracle's `err(t)`: class and message, then the cause of a plain
/// `RuntimeException`.
fn err(e: &LuceneError) -> String {
    let (class, message) = class_and_message(e);
    let mut s = format!("{class}\t{}", m(&message));
    if let LuceneError {
        kind:
            LuceneErrorKind::Unchecked {
                class: "java.lang.RuntimeException",
                cause: Some(cause),
                ..
            },
        ..
    } = e
    {
        let (cc, cm) = class_and_message(cause);
        s.push_str(&format!("\tcause={cc}: {}", m(&cm)));
    }
    s
}

struct Recorder {
    rows: Vec<String>,
    scenario: &'static str,
    seed: i64,
    step: usize,
}

impl Recorder {
    fn begin(&mut self, scenario: &'static str, seed: i64) {
        self.scenario = scenario;
        self.seed = seed;
        self.step = 0;
    }

    fn push(&mut self, call: &str, outcome: String) {
        self.rows.push(format!(
            "{}\t{}\t{}\t{call}\t{outcome}",
            self.scenario, self.seed, self.step
        ));
        self.step += 1;
    }

    /// A step whose value the oracle prints with `m(v)`.
    fn rec(&mut self, call: &str, f: impl FnOnce() -> Result<String>) {
        let outcome = match catch_unwind(AssertUnwindSafe(f)) {
            Ok(Ok(v)) => format!("ok\t{}", m(&v)),
            Ok(Err(e)) => format!("err\t{}", err(&e)),
            Err(payload) => {
                let (class, message) = oracle_support::panic_error(payload);
                format!("err\t{class}\t{}", m(&message))
            }
        };
        self.push(call, outcome);
    }

    /// A step returning `null` in Java.
    fn rec_void(&mut self, call: &str, f: impl FnOnce() -> Result<()>) {
        self.rec(call, || f().map(|()| "null".to_string()))
    }

    /// A step returning a stream (printed as the empty string).
    fn rec_obj<T>(&mut self, call: &str, f: impl FnOnce() -> Result<T>) -> Option<T> {
        let mut out = None;
        self.rec(call, || {
            out = Some(f()?);
            Ok(String::new())
        });
        out
    }
}

fn bytes(n: usize, k: i32) -> Vec<u8> {
    (0..n as i32)
        .map(|i| (i.wrapping_mul(31) + k * 7) as u8)
        .collect()
}

fn fnv(b: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &x in b {
        h ^= u64::from(x);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn read(dir: &dyn Directory, name: &str) -> Result<Vec<u8>> {
    let mut input = dir.open_input(name, &*READONCE_IO_CONTEXT)?;
    let mut b = vec![0u8; input.length() as usize];
    let len = b.len();
    input.read_bytes(Some(&mut b), 0, len)?;
    input.close()?;
    Ok(b)
}

fn digest(dir: &dyn Directory) -> Result<String> {
    let mut parts = Vec::new();
    for name in dir.list_all()? {
        let b = read(dir, &name)?;
        parts.push(format!("{name}:{}:{:016x}", b.len(), fnv(&b)));
    }
    Ok(parts.join(","))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn java_list(items: &[String]) -> String {
    format!("[{}]", items.join(", "))
}

fn default() -> &'static dyn IOContext {
    &*DEFAULT_IO_CONTEXT
}

fn mdw(seed: i64) -> MockDirectoryWrapper {
    MockDirectoryWrapper::new(&JdkRandom::new(seed), Arc::new(ByteBuffersDirectory::new()))
}

fn next_long(r: &mut Recorder, dir: &MockDirectoryWrapper) {
    r.rec("nextLong", || {
        Ok(dir.random_state().next_long().to_string())
    });
}

fn close_dir(r: &mut Recorder, dir: MockDirectoryWrapper) {
    r.rec_void("close dir", || dir.close());
}

fn seeds() -> Vec<i64> {
    // OracleSupport.seeds(): edge values, then ten draws of SplittableRandom(20261103).
    oracle_support_seeds()
}

/// `OracleSupport.seeds()`, read back from the rows of the oracle (the
/// `crash` scenario runs every seed, in order).
fn oracle_support_seeds() -> Vec<i64> {
    let mut seeds = Vec::new();
    for row in oracle_support::rows("mdw_steps.tsv") {
        if row[0] == "crash" && row[2] == "0" {
            seeds.push(row[1].parse().unwrap());
        }
    }
    assert_eq!(seeds.len(), 20);
    seeds
}

// -----------------------------------------------------------------------------
// Scenarios
// -----------------------------------------------------------------------------

fn write_io(r: &mut Recorder, sd: i64) {
    r.begin("write_io", sd);
    let dir = mdw(sd);
    dir.set_check_index_on_close(false);
    dir.set_random_io_exception_rate(0.4);
    for i in 0..10 {
        let name = format!("w{i}");
        let Some(mut out) = r.rec_obj(&format!("createOutput {name}"), || {
            dir.create_output(&name, default())
        }) else {
            continue;
        };
        r.rec_void("writeInt", || out.write_int(i));
        r.rec_void("writeBytes 300", || out.write_bytes(&bytes(300, i), 0, 300));
        r.rec_void("writeString", || out.write_string(&format!("abc{i}")));
        r.rec_void("writeVLong", || {
            out.write_v_long(123_456_789i64 * i64::from(i))
        });
        r.rec("getFilePointer", || Ok(out.file_pointer().to_string()));
        r.rec_void("close", || out.close());
    }
    r.rec("files", || digest(dir.get_delegate().as_ref()));
    next_long(r, &dir);
    close_dir(r, dir);
}

fn open_io(r: &mut Recorder, sd: i64) {
    r.begin("open_io", sd);
    let dir = mdw(sd);
    dir.set_check_index_on_close(false);
    for i in 0..5 {
        let mut out = dir
            .get_delegate()
            .create_output(&format!("r{i}"), default())
            .unwrap();
        out.write_bytes(&bytes(10 + i, i as i32), 0, 10 + i)
            .unwrap();
        out.close().unwrap();
    }
    dir.set_random_io_exception_rate_on_open(0.5);
    for i in 0..16 {
        if i == 10 {
            dir.set_allow_random_file_not_found_exception(false);
        }
        let name = if i % 3 == 0 {
            format!("missing{i}")
        } else {
            format!("r{}", i % 5)
        };
        if let Some(mut input) = r.rec_obj(&format!("openInput {name}"), || {
            dir.open_input(&name, default())
        }) {
            r.rec("readByte", || Ok((input.read_byte()? as i8).to_string()));
            r.rec("length", || Ok(input.length().to_string()));
            r.rec_void("close", || input.close());
        }
        let c = format!("c{i}");
        if let Some(mut out) = r.rec_obj(&format!("createOutput {c}"), || {
            dir.create_output(&c, default())
        }) {
            r.rec_void("close", || out.close());
        }
        if let Some(mut tmp) = r.rec_obj("createTempOutput", || {
            dir.create_temp_output("p", "s", default())
        }) {
            r.rec("getName", || Ok(tmp.name().to_string()));
            r.rec_void("close", || tmp.close());
        }
    }
    next_long(r, &dir);
    close_dir(r, dir);
}

fn tracking(r: &mut Recorder) {
    // An unclosed output.
    r.begin("tracking", 1);
    let d1 = mdw(1);
    let mut o1 = d1.create_output("a", default()).unwrap();
    o1.write_bytes(&bytes(10, 1), 0, 10).unwrap();
    r.rec("getFileHandleCount", || {
        Ok(d1.get_file_handle_count().to_string())
    });
    r.rec_obj("openInput a (open for write)", || {
        d1.open_input("a", default())
    });
    r.rec_obj("createOutput a (again)", || {
        d1.create_output("a", default())
    });
    r.rec("toString", || Ok(d1.to_string()));
    r.rec("isOpen", || Ok(d1.is_open().to_string()));
    r.rec_void("close dir", || d1.close());
    r.rec("isOpen", || Ok(d1.is_open().to_string()));
    next_long(r, &d1);
    drop(o1);

    // An unclosed input.
    r.begin("tracking", 2);
    let d2 = mdw(2);
    let mut o2 = d2.create_output("b", default()).unwrap();
    o2.write_bytes(&bytes(10, 2), 0, 10).unwrap();
    o2.close().unwrap();
    let i2 = d2.open_input("b", default()).unwrap();
    r.rec("getFileHandleCount", || {
        Ok(d2.get_file_handle_count().to_string())
    });
    r.rec_void("close dir", || d2.close());
    next_long(r, &d2);
    drop(i2);

    // Several open files: the map in the message.
    r.begin("tracking", 3);
    let d3 = mdw(3);
    let mut keep = Vec::new();
    for n in ["zz", "a1", "_0.cfs", "b"] {
        let mut o = d3.create_output(n, default()).unwrap();
        o.write_byte(1).unwrap();
        o.close().unwrap();
        keep.push(d3.open_input(n, default()).unwrap());
    }
    keep.push(d3.open_input("b", default()).unwrap());
    let outcome = match d3.close() {
        Ok(()) => "ok\tnull".to_string(),
        Err(e) => {
            let (class, message) = class_and_message(&e);
            format!("err\t{class}\t{}", m(&message))
        }
    };
    r.push("close dir", outcome);
    drop(keep);

    // Deleting, renaming and overwriting open files; abusing closed streams.
    r.begin("tracking", 4);
    let d = mdw(4);
    d.set_check_index_on_close(false);
    for (n, k) in [("x", 4), ("segments_1", 5)] {
        let mut o = d.get_delegate().create_output(n, default()).unwrap();
        o.write_bytes(&bytes(20, k), 0, 20).unwrap();
        o.close().unwrap();
    }
    d.set_assert_no_delete_open_file(true);
    let mut x = r
        .rec_obj("openInput x", || d.open_input("x", default()))
        .unwrap();
    r.rec_void("deleteFile x (open)", || d.delete_file("x"));
    r.rec_void("rename x y (source open)", || d.rename("x", "y"));
    r.rec_void("rename z x (dest open)", || d.rename("z", "x"));
    r.rec_obj("createOutput x (open)", || d.create_output("x", default()));
    let open_deleted = |d: &MockDirectoryWrapper| {
        let mut v: Vec<String> = d.get_open_deleted_files().into_iter().collect();
        v.sort();
        java_list(&v)
    };
    r.rec("getOpenDeletedFiles", || Ok(open_deleted(&d)));
    r.rec_void("close x", || x.close());
    d.set_assert_no_delete_open_file(false);
    let mut x2 = r
        .rec_obj("openInput x", || d.open_input("x", default()))
        .unwrap();
    let mut c1 = r.rec_obj("clone", || x2.clone_input()).unwrap();
    let mut s1 = r.rec_obj("slice", || x2.slice("sl", 2, 5)).unwrap();
    r.rec("slice readByte", || Ok((s1.read_byte()? as i8).to_string()));
    r.rec("slice toString", || Ok(s1.to_java_string()));
    r.rec("getInputCloneCount", || {
        Ok(d.get_input_clone_count().to_string())
    });
    r.rec("toString", || Ok(x2.to_java_string()));
    r.rec_void("deleteFile x (open, allowed)", || d.delete_file("x"));
    r.rec("getOpenDeletedFiles", || Ok(open_deleted(&d)));
    r.rec("readByte", || Ok((x2.read_byte()? as i8).to_string()));
    r.rec_void("close x", || x2.close());
    r.rec("readByte (closed)", || {
        Ok((x2.read_byte()? as i8).to_string())
    });
    r.rec_obj("clone (closed)", || x2.clone_input());
    r.rec("clone readByte (parent closed)", || {
        Ok((c1.read_byte()? as i8).to_string())
    });
    r.rec_void("clone close", || c1.close());
    r.rec_void("close x again", || x2.close());
    r.rec_obj("openInput nope", || d.open_input("nope", default()));
    r.rec_void("openInput segments_1 READONCE", || {
        d.open_input("segments_1", &*READONCE_IO_CONTEXT)?.close()
    });
    for _ in 0..4 {
        r.rec_obj("openInput segments_1 DEFAULT", || {
            d.open_input("segments_1", default())
        });
    }
    let mut q = r
        .rec_obj("createOutput q", || d.create_output("q", default()))
        .unwrap();
    r.rec_void("write q", || q.write_bytes(&bytes(5, 6), 0, 5));
    r.rec_void("close q", || q.close());
    r.rec_void("writeByte q (closed)", || q.write_byte(1));
    r.rec_void("close q again", || q.close());
    r.rec("getFileHandleCount", || {
        Ok(d.get_file_handle_count().to_string())
    });
    // crash() with an output and an input open.
    let mut rr = r
        .rec_obj("createOutput r", || d.create_output("r", default()))
        .unwrap();
    r.rec_void("write r", || rr.write_bytes(&bytes(40, 7), 0, 40));
    let mut qi = r
        .rec_obj("openInput q", || d.open_input("q", default()))
        .unwrap();
    r.rec("getFileHandleCount", || {
        Ok(d.get_file_handle_count().to_string())
    });
    r.rec_void("crash", || d.crash());
    r.rec("getFileHandleCount", || {
        Ok(d.get_file_handle_count().to_string())
    });
    r.rec("files", || digest(d.get_delegate().as_ref()));
    r.rec_void("writeByte r (crashed)", || rr.write_byte(1));
    r.rec_void("close r", || rr.close());
    r.rec("readByte q (crashed)", || {
        Ok((qi.read_byte()? as i8).to_string())
    });
    r.rec_void("close q", || qi.close());
    r.rec_obj("createOutput after crash", || {
        d.create_output("t", default())
    });
    r.rec_obj("createTempOutput after crash", || {
        d.create_temp_output("t", "u", default())
    });
    r.rec_void("deleteFile after crash", || d.delete_file("q"));
    r.rec_void("rename after crash", || d.rename("q", "q2"));
    r.rec_void("sync after crash", || d.sync(&["q".to_string()]));
    r.rec_void("syncMetaData after crash", || d.sync_metadata());
    r.rec_void("openInput after crash", || {
        d.open_input("q", &*READONCE_IO_CONTEXT)?.close()
    });
    r.rec_void("clearCrash", || {
        d.clear_crash();
        Ok(())
    });
    r.rec_void("createOutput after clearCrash", || {
        d.create_output("t", default())?.close()
    });
    r.rec("listAll", || Ok(java_list(&d.list_all()?)));
    r.rec("fileLength t", || Ok(d.file_length("t")?.to_string()));
    r.rec("sizeInBytes", || Ok(d.size_in_bytes()?.to_string()));
    next_long(r, &d);
    drop((x, x2, c1, s1, q, rr, qi));
    close_dir(r, d);
}

struct CountingFailure {
    count: AtomicI32,
    do_fail: AtomicBool,
}

impl Failure for CountingFailure {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        self.count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// Fires (`FakeIOException`) when `doFail` is set and the call stack holds
/// a `deleteFile` frame. **Declared substitute:** Java's
/// `callStackContainsAnyOf("deleteFile")` names the Java method; the Rust
/// frame of `MockDirectoryWrapper::delete_file` is named `delete_file`.
struct OnDelete {
    do_fail: AtomicBool,
}

impl Failure for OnDelete {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if self.do_fail() && LuceneTestCase::call_stack_contains_any_of(&["delete_file"]) {
            return Err(fake_io_exception());
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// One call of the `failure` scenario, counted in failure points.
type OpMut<'a> = Box<dyn FnMut(&MockDirectoryWrapper) -> Result<()> + 'a>;
/// One call of the `failure` scenario that only reads its captures.
type Op<'a> = Box<dyn Fn(&MockDirectoryWrapper) -> Result<()> + 'a>;

fn failure(r: &mut Recorder) {
    r.begin("failure", 7);
    let d = mdw(7);
    d.set_check_index_on_close(false);
    let counting = Arc::new(CountingFailure {
        count: AtomicI32::new(0),
        do_fail: AtomicBool::new(false),
    });
    d.fail_on(Arc::clone(&counting) as Arc<dyn Failure>);
    let mut temp_name = String::new();
    let ops: Vec<OpMut<'_>> = vec![
        Box::new(|d| {
            let mut o = d.create_output("f", default())?;
            o.write_int(1)?;
            o.write_bytes(&bytes(100, 1), 0, 100)?;
            let b = bytes(30, 2);
            o.copy_bytes(&mut ByteArrayDataInput::new(&b), 30)?;
            o.close()
        }),
        Box::new(|d| {
            let mut i = d.open_input("f", default())?;
            i.read_byte()?;
            let mut c = i.clone_input()?;
            c.close()?;
            i.close()
        }),
        Box::new(|d| d.sync(&["f".to_string()])),
        Box::new(|d| d.sync_metadata()),
        Box::new(|d| d.rename("f", "g")),
        Box::new(|d| {
            d.list_all()?;
            d.file_length("g")?;
            d.size_in_bytes()?;
            Ok(())
        }),
        Box::new(|d| {
            let mut o = d.create_temp_output("p", "s", default())?;
            temp_name = o.name().to_string();
            o.close()
        }),
    ];
    let mut k = 0;
    for mut op in ops {
        let before = counting.count.load(Ordering::SeqCst);
        r.rec(&format!("op{k}"), || {
            op(&d)?;
            Ok((counting.count.load(Ordering::SeqCst) - before).to_string())
        });
        k += 1;
    }
    let tail: Vec<Op<'_>> = vec![
        Box::new(|d| d.delete_file(&temp_name)),
        Box::new(|d| {
            d.set_fail_on_create_output(false);
            d.create_output("h", default())?.close()
        }),
        Box::new(|d| {
            d.set_fail_on_open_input(false);
            d.open_input("h", default())?.close()
        }),
    ];
    for op in tail {
        let before = counting.count.load(Ordering::SeqCst);
        r.rec(&format!("op{k}"), || {
            op(&d)?;
            Ok((counting.count.load(Ordering::SeqCst) - before).to_string())
        });
        k += 1;
    }
    // A failure that fires on a stack frame: deleteFile.
    let on_delete = Arc::new(OnDelete {
        do_fail: AtomicBool::new(false),
    });
    d.fail_on(Arc::clone(&on_delete) as Arc<dyn Failure>);
    r.rec_void("deleteFile g (no doFail)", || {
        d.get_delegate()
            .copy_from(d.get_delegate().as_ref(), "g", "g2", default())?;
        d.delete_file("g")
    });
    on_delete.set_do_fail();
    r.rec_void("deleteFile g2 (doFail)", || d.delete_file("g2"));
    r.rec_void("createOutput k (doFail)", || {
        let mut o = d.create_output("k", default())?;
        o.write_byte(3)?;
        o.close()
    });
    r.rec_void("rename k k2 (doFail)", || d.rename("k", "k2"));
    on_delete.clear_do_fail();
    r.rec_void("deleteFile g2 (cleared)", || d.delete_file("g2"));
    r.rec("listAll", || Ok(java_list(&d.list_all()?)));
    next_long(r, &d);
    close_dir(r, d);
}

fn disk_full(r: &mut Recorder) {
    r.begin("disk_full", 11);
    let d = mdw(11);
    d.set_check_index_on_close(false);
    d.set_max_size_in_bytes(100);
    let mut x = d.create_output("x", default()).unwrap();
    r.rec_void("x writeBytes 60", || x.write_bytes(&bytes(60, 1), 0, 60));
    r.rec_void("x writeBytes 60", || x.write_bytes(&bytes(60, 2), 0, 60));
    r.rec_void("x close", || x.close());
    r.rec("sizeInBytes", || Ok(d.size_in_bytes()?.to_string()));
    let mut y = d.create_output("y", default()).unwrap();
    r.rec_void("y writeBytes 60", || y.write_bytes(&bytes(60, 3), 0, 60));
    r.rec_void("y close", || y.close());
    r.rec("getMaxUsedSizeInBytes", || {
        Ok(d.get_max_used_size_in_bytes().to_string())
    });
    d.set_max_size_in_bytes(150);
    let mut z = d.create_output("z", default()).unwrap();
    r.rec_void("z writeBytes 60", || z.write_bytes(&bytes(60, 4), 0, 60));
    r.rec_void("z close", || z.close());
    r.rec("sizeInBytes", || Ok(d.size_in_bytes()?.to_string()));
    r.rec("getMaxUsedSizeInBytes", || {
        Ok(d.get_max_used_size_in_bytes().to_string())
    });
    d.set_max_size_in_bytes(160);
    let mut v = d.create_output("v", default()).unwrap();
    r.rec_void("v copyBytes 50", || {
        let mut input = d.get_delegate().open_input("x", &*READONCE_IO_CONTEXT)?;
        let copied = v.copy_bytes(input.as_mut(), 50);
        let closed = input.close();
        copied.and(closed)
    });
    r.rec_void("v close", || v.close());
    r.rec("toString", || Ok(d.to_string()));
    r.rec("getMaxSizeInBytes", || {
        Ok(d.get_max_size_in_bytes().to_string())
    });
    d.set_max_size_in_bytes(0);
    d.set_track_disk_usage(true);
    r.rec("resetMaxUsedSizeInBytes", || {
        d.reset_max_used_size_in_bytes()?;
        Ok(d.get_max_used_size_in_bytes().to_string())
    });
    let mut u = d.create_output("u", default()).unwrap();
    r.rec_void("u writeBytes 70", || u.write_bytes(&bytes(70, 5), 0, 70));
    r.rec_void("u close", || u.close());
    r.rec("getMaxUsedSizeInBytes", || {
        Ok(d.get_max_used_size_in_bytes().to_string())
    });
    r.rec("files", || digest(d.get_delegate().as_ref()));
    next_long(r, &d);
    drop((x, y, z, v, u));
    close_dir(r, d);
}

fn crash(r: &mut Recorder, sd: i64, always_corrupt: bool) {
    r.begin(
        if always_corrupt {
            "crash_always"
        } else {
            "crash"
        },
        sd,
    );
    let d = mdw(sd);
    d.set_check_index_on_close(false);
    d.set_always_corrupt(always_corrupt);
    for k in 0..8usize {
        let name = format!("_{k}.dat");
        r.rec_void(&format!("write {name}"), || {
            let mut o = d.create_output(&name, default())?;
            let n = if k == 7 { 0 } else { 1 + (k * 397) % 1500 };
            let written = o.write_bytes(&bytes(n, k as i32), 0, n);
            let closed = o.close();
            written.and(closed)
        });
    }
    r.rec_void("sync", || {
        d.sync(&["_0.dat".to_string(), "_3.dat".to_string()])
    });
    r.rec_void("crash", || d.crash());
    r.rec("files", || digest(d.get_delegate().as_ref()));
    next_long(r, &d);
    close_dir(r, d);
}

/// The index of `mdw_index.tsv`, in a `ByteBuffersDirectory`.
fn load_index() -> ByteBuffersDirectory {
    let dir = ByteBuffersDirectory::new();
    for row in oracle_support::rows("mdw_index.tsv") {
        let b: Vec<u8> = (0..row[1].len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&row[1][i..i + 2], 16).unwrap())
            .collect();
        let mut out = dir.create_output(&row[0], default()).unwrap();
        out.write_bytes(&b, 0, b.len()).unwrap();
        out.close().unwrap();
    }
    dir
}

fn copy_index(index: &ByteBuffersDirectory, sd: i64, synced: &[String]) -> MockDirectoryWrapper {
    let d = mdw(sd);
    for name in index.list_all().unwrap() {
        d.copy_from(index, &name, &name, default()).unwrap();
    }
    d.sync(synced).unwrap();
    d
}

fn synced(index: &ByteBuffersDirectory, exts: &[&str]) -> Vec<String> {
    index
        .list_all()
        .unwrap()
        .into_iter()
        .filter(|name| {
            name.starts_with("segments")
                || name.ends_with(".si")
                || exts.iter().any(|e| name.ends_with(e))
        })
        .collect()
}

fn index(r: &mut Recorder, index: &ByteBuffersDirectory, sd: i64) {
    let none = synced(index, &[]);
    let most = synced(
        index,
        &[
            ".fdt", ".fdx", ".fdm", ".fnm", ".nvd", ".nvm", ".doc", ".tim", ".tip", ".tmd",
        ],
    );
    let all = index.list_all().unwrap();

    r.begin("index_crash", sd);
    let d = copy_index(index, sd, &none);
    d.set_check_index_on_close(false);
    r.rec_void("crash", || d.crash());
    r.rec("files", || digest(d.get_delegate().as_ref()));
    next_long(r, &d);

    r.begin("index_close", sd);
    let d2 = copy_index(index, sd, &none);
    r.rec_void("close dir", || d2.close());
    next_long(r, &d2);

    r.begin("index_close_most", sd);
    let d3 = copy_index(index, sd, &most);
    r.rec_void("close dir", || d3.close());
    next_long(r, &d3);

    r.begin("index_close_clean", sd);
    let d4 = copy_index(index, sd, &all);
    d4.set_cross_check_term_vectors_on_close(false);
    r.rec("getLevelForCheckOnClose", || {
        Ok(d4.get_level_for_check_on_close().to_string())
    });
    r.rec_void("close dir", || d4.close());
    next_long(r, &d4);

    r.begin("index_unknown", sd);
    let d5 = copy_index(index, sd, &all);
    d5.set_check_index_on_close(false);
    for n in [
        "_9.fake",
        "pending_segments_7",
        "write.lock",
        "other.txt",
        "_a_Lucene90_0.dvd",
    ] {
        let mut o = d5.get_delegate().create_output(n, default()).unwrap();
        o.write_bytes(&bytes(33, n.len() as i32), 0, 33).unwrap();
        o.close().unwrap();
    }
    r.rec_void("corruptUnknownFiles", || d5.corrupt_unknown_files());
    r.rec("files", || digest(d5.get_delegate().as_ref()));
    next_long(r, &d5);
}

/// The `case` (`nocfs` or `cfs`) of the corpus the oracle wrote.
fn load_corpus(case: &str) -> ByteBuffersDirectory {
    let dir = ByteBuffersDirectory::new();
    for row in oracle_support::rows("mdw_corpus.tsv") {
        if row[0] != case {
            continue;
        }
        let b: Vec<u8> = (0..row[2].len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&row[2][i..i + 2], 16).unwrap())
            .collect();
        let mut out = dir.create_output(&row[1], default()).unwrap();
        out.write_bytes(&b, 0, b.len()).unwrap();
        out.close().unwrap();
    }
    dir
}

/// `MockDirectoryWrapperOracle.indexRead`: the corpus read in full through
/// the wrapper, then the wrapper closed; the final `nextLong` pins how many
/// inputs were opened (the wrapper draws on every `openInput`).
fn index_read(r: &mut Recorder, corpus: &ByteBuffersDirectory, scenario: &'static str, sd: i64) {
    r.begin(scenario, sd);
    let all = corpus.list_all().unwrap();
    let d = copy_index(corpus, sd, &all);
    let mut reader: Option<Arc<dyn IndexReader>> = None;
    r.rec("open reader", || {
        let opened = directory_reader::open(d.as_directory())?;
        let leaves = Arc::clone(&opened).leaves()?.len();
        let row = format!("{}/{}/{leaves}", opened.max_doc(), opened.num_docs());
        reader = Some(opened);
        Ok(row)
    });
    r.rec_void("close reader", || {
        reader.take().expect("the reader opened").close()
    });
    r.rec("checkIndex", || {
        let status = TestUtil::check_index_with(
            d.as_directory(),
            CheckIndexLevel::MIN_LEVEL_FOR_SLOW_CHECKS,
            true,
            false,
            None,
        )?;
        Ok(format!("{}/{}", status.clean, status.num_segments))
    });
    r.rec_void("close dir", || d.close());
    next_long(r, &d);
}

fn unreferenced(r: &mut Recorder, index: &ByteBuffersDirectory) {
    r.begin("index_unref", 0);
    let d = copy_index(index, 0, &index.list_all().unwrap());
    d.set_check_index_on_close(false);
    d.set_assert_no_unrefenced_files_on_close(true);
    let mut o = d
        .get_delegate()
        .create_output("_77.fdt", default())
        .unwrap();
    o.write_bytes(&bytes(12, 1), 0, 12).unwrap();
    o.close().unwrap();
    r.rec_void("close dir", || d.close());

    r.begin("index_unref", 1);
    let d2 = copy_index(index, 1, &index.list_all().unwrap());
    d2.set_check_index_on_close(false);
    d2.set_assert_no_unrefenced_files_on_close(true);
    r.rec_void("close dir", || d2.close());
}

fn with_hint(hint: Arc<dyn FileOpenHint>) -> Box<dyn IOContext> {
    DEFAULT_IO_CONTEXT.with_hints(&[hint])
}

fn serial_io(r: &mut Recorder) {
    r.begin("serial_io", 0);
    let dir = SerialIOCountingDirectory::new(Box::new(ByteBuffersDirectory::new()));
    let mut o = dir.create_output("s", default()).unwrap();
    o.write_bytes(&bytes(256 * 1024, 9), 0, 256 * 1024).unwrap();
    o.close().unwrap();
    let mut input = dir.open_input("s", default()).unwrap();
    let count = |dir: &SerialIOCountingDirectory| Ok(dir.count().to_string());
    r.rec("toString", || Ok(input.to_java_string()));
    r.rec("readByte@0", || {
        input.read_byte()?;
        count(&dir)
    });
    r.rec("readByte@8192", || {
        input.seek(8192)?;
        input.read_byte()?;
        count(&dir)
    });
    r.rec("readBytes@40960x100", || {
        input.seek(40960)?;
        input.read_bytes(Some(&mut [0u8; 100]), 0, 100)?;
        count(&dir)
    });
    r.rec("readByte@12288 (backwards)", || {
        input.seek(12288)?;
        input.read_byte()?;
        count(&dir)
    });
    r.rec("readByte@16384 (read-ahead)", || {
        input.seek(16384)?;
        input.read_byte()?;
        count(&dir)
    });
    r.rec("readInt@36860 (page boundary)", || {
        input.seek(36860)?;
        input.read_int()?;
        count(&dir)
    });
    r.rec("prefetch@20*4096x8192", || {
        input.prefetch(20 * 4096, 8192)?;
        count(&dir)
    });
    r.rec("prefetch@40*4096x100", || {
        input.prefetch(40 * 4096, 100)?;
        count(&dir)
    });
    r.rec("readByte@20*4096", || {
        input.seek(20 * 4096)?;
        input.read_byte()?;
        count(&dir)
    });
    r.rec("readByte@40*4096", || {
        input.seek(40 * 4096)?;
        input.read_byte()?;
        count(&dir)
    });
    r.rec("prefetch@2*4096x10 (window)", || {
        input.prefetch(2 * 4096, 10)?;
        count(&dir)
    });
    r.rec("prefetch@50*4096x10", || {
        input.prefetch(50 * 4096, 10)?;
        count(&dir)
    });
    r.rec("readVLong@50*4096", || {
        input.seek(50 * 4096)?;
        input.read_v_long()?;
        count(&dir)
    });
    r.rec("skipBytes 9000", || {
        input.skip_bytes(9000)?;
        input.read_byte()?;
        count(&dir)
    });
    let mut sl = input.slice("sl", 30 * 4096, 5 * 4096).unwrap();
    r.rec("slice readByte@0", || {
        sl.read_byte()?;
        count(&dir)
    });
    r.rec("slice readBytes@4096x10", || {
        sl.seek(4096)?;
        sl.read_bytes(Some(&mut [0u8; 10]), 0, 10)?;
        count(&dir)
    });
    r.rec("slice getFilePointer", || Ok(sl.file_pointer().to_string()));
    r.rec("slice length", || Ok(sl.length().to_string()));
    r.rec_obj("slice of slice", || {
        input.slice("bad", 60 * 4096, 10 * 4096)
    });
    let mut cl = input.clone_input().unwrap();
    r.rec("clone readByte", || {
        cl.seek(0)?;
        cl.read_byte()?;
        count(&dir)
    });
    let mut rnd = dir
        .open_input("s", &*with_hint(Arc::new(DataAccessHint::Random)))
        .unwrap();
    r.rec("random readByte@0", || {
        rnd.read_byte()?;
        count(&dir)
    });
    r.rec("random readByte@4096", || {
        rnd.seek(4096)?;
        rnd.read_byte()?;
        count(&dir)
    });
    r.rec("random readByte@100 (same page)", || {
        rnd.seek(100)?;
        rnd.read_byte()?;
        count(&dir)
    });
    r.rec("random prefetch@4096*3x10", || {
        rnd.prefetch(3 * 4096, 10)?;
        count(&dir)
    });
    r.rec("random readByte@3*4096", || {
        rnd.seek(3 * 4096)?;
        rnd.read_byte()?;
        count(&dir)
    });
    let mut idx = dir
        .open_input("s", &*with_hint(Arc::new(FileTypeHint::Index)))
        .unwrap();
    r.rec("index open", || count(&dir));
    r.rec("index reads", || {
        idx.seek(50000)?;
        idx.read_byte()?;
        idx.seek(10)?;
        idx.read_byte()?;
        count(&dir)
    });
    let mut ck = dir.open_checksum_input("s").unwrap();
    r.rec("openChecksumInput", || count(&dir));
    r.rec("checksum reads", || {
        ck.read_bytes(Some(&mut vec![0u8; 20000]), 0, 20000)?;
        ck.read_byte()?;
        count(&dir)
    });
    input.close().unwrap();
    rnd.close().unwrap();
    idx.close().unwrap();
    ck.close().unwrap();
    drop((sl, cl));
    r.rec("close dir", || {
        dir.close()?;
        count(&dir)
    });
}

fn corrupting(r: &mut Recorder) {
    r.begin("corrupting", 0);
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let mut out = CorruptingIndexOutput::new(
        Arc::clone(&dir),
        5,
        dir.create_output("c", default()).unwrap(),
    );
    r.rec_void("write 20", || out.write_bytes(&bytes(20, 3), 0, 20));
    r.rec("getFilePointer", || Ok(out.file_pointer().to_string()));
    r.rec("getChecksum", || Ok(out.checksum()?.to_string()));
    r.rec("getName", || Ok(out.name().to_string()));
    r.rec("toString", || Ok(out.resource_description().to_string()));
    r.rec_void("close", || out.close());
    r.rec_void("close again", || out.close());
    r.rec("bytes c", || Ok(hex(&read(dir.as_ref(), "c")?)));
    r.rec("listAll", || Ok(java_list(&dir.list_all()?)));
    let mut out2 = CorruptingIndexOutput::new(
        Arc::clone(&dir),
        10,
        dir.create_output("d", default()).unwrap(),
    );
    r.rec_void("write 10", || out2.write_bytes(&bytes(10, 4), 0, 10));
    r.rec_void("close (byte past end)", || out2.close());
    r.rec("listAll", || Ok(java_list(&dir.list_all()?)));

    r.begin("corrupting", 5);
    let d = mdw(5);
    d.set_check_index_on_close(false);
    let mut out3 = CorruptingIndexOutput::new(
        d.as_directory(),
        3,
        d.create_output("m", default()).unwrap(),
    );
    r.rec_void("write 8", || out3.write_bytes(&bytes(8, 5), 0, 8));
    r.rec_void("close", || out3.close());
    r.rec("bytes m", || {
        Ok(hex(&read(d.get_delegate().as_ref(), "m")?))
    });
    r.rec("getFileHandleCount", || {
        Ok(d.get_file_handle_count().to_string())
    });
    r.rec("listAll", || Ok(java_list(&d.list_all()?)));
    next_long(r, &d);
    drop(out3);
    close_dir(r, d);
}

fn class_of(input: &dyn IndexInput) -> &'static str {
    let id = input.as_filter_index_input().map(|(id, _)| id);
    if id == Some(std::any::TypeId::of::<MockIndexInputWrapper>()) {
        "MockIndexInputWrapper"
    } else if id == Some(std::any::TypeId::of::<SlowClosingMockIndexInputWrapper>()) {
        "SlowClosingMockIndexInputWrapper"
    } else if id == Some(std::any::TypeId::of::<SlowOpeningMockIndexInputWrapper>()) {
        "SlowOpeningMockIndexInputWrapper"
    } else {
        "?"
    }
}

fn slow(r: &mut Recorder, sd: i64) {
    r.begin("slow", sd);
    let d = mdw(sd);
    d.set_check_index_on_close(false);
    d.set_use_slow_open_closers(true);
    let mut o = d.get_delegate().create_output("s", default()).unwrap();
    o.write_bytes(&bytes(10, 1), 0, 10).unwrap();
    o.close().unwrap();
    let mut hits = Vec::new();
    for i in 0..400 {
        let mut input = d.open_input("s", &*READONCE_IO_CONTEXT).unwrap();
        if class_of(input.as_ref()) != "MockIndexInputWrapper" {
            let c = input.clone_input().unwrap();
            hits.push(format!(
                "{i}:{}:{}",
                class_of(input.as_ref()),
                class_of(c.as_ref())
            ));
        }
        input.close().unwrap();
    }
    r.rec("slow wrappers", || Ok(hits.join(",")));
    next_long(r, &d);
    close_dir(r, d);
}

fn raw(r: &mut Recorder) {
    r.begin("raw", 0);
    let src = ByteBuffersDirectory::new();
    let mut o = src.create_output("a", default()).unwrap();
    o.write_bytes(&bytes(7, 1), 0, 7).unwrap();
    o.close().unwrap();
    let raw = RawDirectoryWrapper::new(Arc::new(ByteBuffersDirectory::new()));
    r.rec("toString", || Ok(raw.to_string()));
    r.rec("copyFrom", || {
        raw.copy_from(&src, "a", "b", default())?;
        Ok(java_list(&raw.list_all()?))
    });
    r.rec("openChecksumInput", || {
        let mut input = raw.open_checksum_input("b")?;
        input.read_bytes(Some(&mut [0u8; 7]), 0, 7)?;
        let checksum = input.get_checksum()?;
        input.close()?;
        Ok(checksum.to_string())
    });
    r.rec("isOpen", || Ok(raw.base().is_open().to_string()));
    r.rec("close", || {
        raw.close()?;
        Ok(raw.base().is_open().to_string())
    });
}

fn throttling(r: &mut Recorder, sd: i64) {
    r.begin("throttling", sd);
    let d = mdw(sd);
    d.set_check_index_on_close(false);
    d.set_throttling(Throttling::Always);
    let mut t = r
        .rec_obj("createOutput t", || d.create_output("t", default()))
        .unwrap();
    r.rec("toString", || Ok(t.resource_description().to_string()));
    r.rec("getName", || Ok(t.name().to_string()));
    r.rec_void("write", || t.write_bytes(&bytes(10, 1), 0, 10));
    r.rec_void("close", || t.close());
    d.set_throttling(Throttling::Sometimes);
    for i in 0..5 {
        let n = format!("s{i}");
        r.rec_void(&format!("createOutput {n}"), || {
            d.create_output(&n, default())?.close()
        });
    }
    r.rec("files", || digest(d.get_delegate().as_ref()));
    next_long(r, &d);
    drop(t);
    close_dir(r, d);
}

/// Every scenario of `MockDirectoryWrapperOracle`, in the oracle's order,
/// row for row.
#[test]
fn mock_directory_wrapper_matches_lucene() {
    init();
    let seeds = seeds();
    let mut r = Recorder {
        rows: Vec::new(),
        scenario: "",
        seed: 0,
        step: 0,
    };
    for &s in &seeds[..8] {
        write_io(&mut r, s);
    }
    for &s in &seeds[..8] {
        open_io(&mut r, s);
    }
    tracking(&mut r);
    failure(&mut r);
    disk_full(&mut r);
    for &s in &seeds {
        crash(&mut r, s, false);
    }
    for &s in &seeds[..4] {
        crash(&mut r, s, true);
    }
    let idx = load_index();
    for &s in &seeds[..10] {
        index(&mut r, &idx, s);
    }
    unreferenced(&mut r, &idx);
    serial_io(&mut r);
    corrupting(&mut r);
    for &s in &seeds[..4] {
        slow(&mut r, s);
    }
    raw(&mut r);
    for &s in &seeds[..3] {
        throttling(&mut r, s);
    }
    let nocfs = load_corpus("nocfs");
    for &s in &seeds[..4] {
        index_read(&mut r, &nocfs, "index_read_nocfs", s);
    }
    let cfs = load_corpus("cfs");
    for &s in &seeds[..4] {
        index_read(&mut r, &cfs, "index_read_cfs", s);
    }

    let expected: Vec<String> = oracle_support::rows("mdw_steps.tsv")
        .into_iter()
        .map(|row| row.join("\t"))
        .collect();
    let mut mismatches = Vec::new();
    for (i, exp) in expected.iter().enumerate() {
        let act = r.rows.get(i).map(String::as_str).unwrap_or("<missing>");
        if act != exp {
            mismatches.push(format!("row {i}\n  java: {exp}\n  rust: {act}"));
        }
    }
    if r.rows.len() > expected.len() {
        mismatches.push(format!("{} extra rows", r.rows.len() - expected.len()));
    }
    assert!(
        mismatches.is_empty(),
        "{} of {} rows differ:\n{}",
        mismatches.len(),
        expected.len(),
        mismatches[..mismatches.len().min(40)].join("\n")
    );
}

/// A failure's `reset()` and `do_fail` flags, and `PlainFailure`, Java's
/// `new Failure()`, whose `eval` never throws.
#[test]
fn plain_failure_never_throws() {
    let d = mdw(0);
    d.set_check_index_on_close(false);
    let plain = Arc::new(rucene_test_framework::store::PlainFailure::new());
    assert!(!plain.do_fail());
    plain.set_do_fail();
    assert!(plain.do_fail());
    plain.reset();
    plain.clear_do_fail();
    assert!(!plain.do_fail());
    d.fail_on(plain);
    d.create_output("a", default()).unwrap().close().unwrap();
    let d = d;
    d.close().unwrap();
}

/// `MockIndexInputWrapper` registers itself as a test filter input (its
/// `static {}` block), so `FilterIndexInput.unwrapOnlyTest` steps through it
/// to the delegate, as the vectorization lookups do.
#[test]
fn unwrap_only_test_steps_through_mock_inputs() {
    let d = mdw(3);
    d.set_check_index_on_close(false);
    let mut o = d.get_delegate().create_output("a", default()).unwrap();
    o.write_bytes(&[1, 2, 3], 0, 3).unwrap();
    o.close().unwrap();
    let mut input = d.open_input("a", default()).unwrap();
    let inner = rucene::store::FilterIndexInput::unwrap_only_test(input.as_ref());
    assert!(inner.as_filter_index_input().is_none());
    assert_eq!(inner.length(), 3);
    input.close().unwrap();
    drop(input);
    let d = d;
    d.close().unwrap();
}

/// An input opened with `READONCE` is confined to its thread:
/// `ensureAccessible()` throws `RuntimeException("Abusing from another
/// thread!")` elsewhere; `length()` does not check the thread.
#[test]
fn readonce_inputs_are_confined_to_their_thread() {
    let d = mdw(4);
    d.set_check_index_on_close(false);
    let mut o = d.get_delegate().create_output("a", default()).unwrap();
    o.write_bytes(&[1, 2, 3], 0, 3).unwrap();
    o.close().unwrap();
    let mut confined = d.open_input("a", &*READONCE_IO_CONTEXT).unwrap();
    let mut free = d.open_input("a", default()).unwrap();
    std::thread::scope(|s| {
        s.spawn(|| {
            let e = confined.read_byte().unwrap_err();
            assert_eq!(
                class_and_message(&e),
                (
                    "java.lang.RuntimeException".to_string(),
                    "Abusing from another thread!".to_string()
                )
            );
            assert_eq!(confined.length(), 3);
            let panic = catch_unwind(AssertUnwindSafe(|| confined.file_pointer())).unwrap_err();
            assert_eq!(
                oracle_support::panic_error(panic),
                (
                    "java.lang.RuntimeException".to_string(),
                    "Abusing from another thread!".to_string()
                )
            );
            assert_eq!(free.read_byte().unwrap(), 1);
        });
    });
    assert_eq!(confined.read_byte().unwrap(), 1);
    confined.close().unwrap();
    free.close().unwrap();
    drop((confined, free));
    let d = d;
    d.close().unwrap();
}

/// A `Failure` may call back into the wrapper from `eval` (the monitor is
/// re-entrant, as Java's `synchronized` is), and a wrapper shared by threads
/// tracks every stream they open and close.
#[test]
fn failures_reenter_and_threads_share_the_wrapper() {
    struct Reentrant {
        do_fail: AtomicBool,
        calls: AtomicI32,
    }
    impl Failure for Reentrant {
        fn eval(&self, dir: &MockDirectoryWrapper) -> Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            dir.get_file_handle_count();
            dir.size_in_bytes()?;
            Ok(())
        }
        fn do_fail_flag(&self) -> &AtomicBool {
            &self.do_fail
        }
    }
    let d = mdw(5);
    d.set_check_index_on_close(false);
    let failure = Arc::new(Reentrant {
        do_fail: AtomicBool::new(false),
        calls: AtomicI32::new(0),
    });
    d.fail_on(Arc::clone(&failure) as Arc<dyn Failure>);
    std::thread::scope(|s| {
        for t in 0..4 {
            let d = &d;
            s.spawn(move || {
                for i in 0..25 {
                    let name = format!("t{t}_{i}");
                    let mut o = d.create_output(&name, default()).unwrap();
                    o.write_bytes(&bytes(50, i), 0, 50).unwrap();
                    o.close().unwrap();
                    let mut input = d.open_input(&name, default()).unwrap();
                    assert_eq!(input.length(), 50);
                    input.close().unwrap();
                }
            });
        }
    });
    assert!(failure.calls.load(Ordering::SeqCst) > 0);
    assert_eq!(d.get_file_handle_count(), 0);
    assert_eq!(d.list_all().unwrap().len(), 100);
    let d = d;
    d.close().unwrap();
}

/// `crash()` closes an open `SlowClosingMockIndexInputWrapper` through its
/// own `close()` (the 50 ms sleep included): with seed -1 the eighth input
/// opened with slow open/closers is one (`mdw_steps.tsv`, scenario `slow`).
#[test]
fn crash_closes_slow_closing_inputs() {
    let d = mdw(-1);
    d.set_check_index_on_close(false);
    d.set_use_slow_open_closers(true);
    let mut o = d.get_delegate().create_output("s", default()).unwrap();
    o.write_bytes(&bytes(10, 1), 0, 10).unwrap();
    o.close().unwrap();
    for _ in 0..7 {
        d.open_input("s", &*READONCE_IO_CONTEXT)
            .unwrap()
            .close()
            .unwrap();
    }
    let mut slow = d.open_input("s", &*READONCE_IO_CONTEXT).unwrap();
    assert_eq!(class_of(slow.as_ref()), "SlowClosingMockIndexInputWrapper");
    assert_eq!(d.get_file_handle_count(), 1);
    let start = std::time::Instant::now();
    d.crash().unwrap();
    assert!(start.elapsed().as_millis() >= 45, "{:?}", start.elapsed());
    assert_eq!(d.get_file_handle_count(), 0);
    let e = slow.read_byte().unwrap_err();
    assert_eq!(class_and_message(&e).1, "Abusing closed IndexInput!");
    slow.close().unwrap();
    drop(slow);
    let d = d;
    d.close().unwrap();
}
