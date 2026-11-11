//! Portability tests for the `FST.BytesReader`s `FSTEnum` opens and for
//! `BytesRefFSTEnum.current()`, for rmp tasks #332 and #333.
//!
//! Every expected value is **measured from `lucene-core-10.5.0.jar`** by
//! `org.apache.lucene.util.fst.RuceneFSTEnumReaderFixture` in the same run:
//! the FST is read through a counting `FSTReader`, and every operation of a
//! `BytesRefFSTEnum` reports how many `FST.getBytesReader()` calls it made.
//!
//! What is pinned:
//!
//! 1. The constructor opens the one reader the enumeration keeps
//!    (`FSTEnum.java:50-52`); `next` opens none.
//! 2. `seekCeil` and `seekFloor` open one more reader for **each** array node
//!    they scan (`FSTEnum.java:149`, `:345`) and none for a list node, so a
//!    seek costs zero, one, two or three readers depending on the path.
//! 3. `seekExact` opens exactly one, the local `fstReader` that shadows the
//!    field for the whole method (`FSTEnum.java:659`).
//! 4. `current()` always returns the enum's one `InputOutput`: an empty input
//!    and a `null` output before the first term, the last term once `next`
//!    is exhausted, and the bytes a failed `seekExact` wrote over the current
//!    term (`BytesRefFSTEnum.java:49-51`, `:116-123`).
//!
//! Both constructors are checked against the same trace: the borrowing
//! [`BytesRefFSTEnum::new`] and the owning [`BytesRefFSTEnum::new_shared`].

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rucene::error::Result;
use rucene::store::DataOutput;
use rucene::util::fst::{
    Builder, BytesReader, BytesRefFSTEnum, BytesRefInputOutput, FSTReader, InputType,
    PositiveIntOutputs, Util, FST,
};
use rucene::util::{Accountable, BytesRef, IntsRefBuilder};

/// `RuceneFSTEnumReaderFixture.TERMS`, as Latin-1 bytes.
const TERMS: [&[u8]; 19] = [
    b"",
    b"a",
    b"aa",
    b"ab",
    b"abc",
    b"abd",
    b"ac",
    b"ae",
    b"ag",
    b"ai",
    b"b",
    b"ba",
    b"c",
    b"d",
    b"e",
    b"f",
    b"g",
    b"z\xff",
    b"\xff\xff",
];

/// `RuceneFSTEnumReaderFixture.TARGETS`, as Latin-1 bytes.
const TARGETS: [&[u8]; 15] = [
    b"",
    b"a",
    b"aa",
    b"abz",
    b"ab",
    b"abcd",
    b"ad",
    b"aj",
    b"b",
    b"bb",
    b"c",
    b"h",
    b"z",
    b"\xff",
    b"\xff\xff\xff",
];

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("java-codec-harness")
}

fn which_mvn() -> std::result::Result<String, String> {
    for candidate in ["mvn", "/usr/bin/mvn", "/usr/local/bin/mvn"] {
        if Command::new(candidate)
            .arg("-v")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            return Ok(candidate.to_string());
        }
    }
    Err("Maven is not available on PATH".to_string())
}

/// The fixture's `key=value` lines, in order.
fn run_fixture() -> Vec<String> {
    let mvn = which_mvn().unwrap_or_else(|e| {
        panic!("the FSTEnum reader portability test needs Maven and a JDK: {e}")
    });
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.util.fst.RuceneFSTEnumReaderFixture")
        .current_dir(harness_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn Maven: {e}"));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success() && stdout.lines().any(|l| l.trim() == "read_ok=true"),
        "the Java fixture failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| l.contains('=') && *l != "read_ok=true")
        .map(str::to_string)
        .collect()
}

/// An [`FSTReader`] that counts the readers opened over it, as the
/// fixture's `CountingReader` does.
struct CountingReader {
    inner: Arc<dyn FSTReader>,
    opened: Arc<AtomicUsize>,
}

impl Accountable for CountingReader {
    fn ram_bytes_used(&self) -> i64 {
        self.inner.ram_bytes_used()
    }
}

impl FSTReader for CountingReader {
    fn get_reverse_bytes_reader(&self) -> Result<Box<dyn BytesReader + '_>> {
        self.opened.fetch_add(1, Ordering::Relaxed);
        self.inner.get_reverse_bytes_reader()
    }

    fn get_owned_reverse_bytes_reader(self: Arc<Self>) -> Result<Box<dyn BytesReader>> {
        self.opened.fetch_add(1, Ordering::Relaxed);
        Arc::clone(&self.inner).get_owned_reverse_bytes_reader()
    }

    fn write_to(&self, out: &mut dyn DataOutput) -> Result<()> {
        self.inner.write_to(out)
    }
}

/// The fixture's FST, read through a [`CountingReader`].
fn counted_fst() -> (FST<PositiveIntOutputs>, Arc<AtomicUsize>) {
    let mut compiler = Builder::new(InputType::Byte1, PositiveIntOutputs::get_singleton())
        .build()
        .unwrap();
    let mut scratch = IntsRefBuilder::new();
    for (i, term) in TERMS.iter().enumerate() {
        let input = Util::to_ints_ref(&BytesRef::new(term.to_vec()), &mut scratch);
        compiler.add(&input, i as i64 * 3).unwrap();
    }
    let metadata = compiler.compile().unwrap().unwrap();
    let opened = Arc::new(AtomicUsize::new(0));
    let reader = CountingReader {
        inner: Arc::from(compiler.into_fst_reader().unwrap()),
        opened: Arc::clone(&opened),
    };
    (FST::from_reader(metadata, Box::new(reader)), opened)
}

/// `RuceneFSTEnumReaderFixture.hex`.
fn hex(b: &[u8]) -> String {
    if b.is_empty() {
        return "-".to_string();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `RuceneFSTEnumReaderFixture.result`.
fn result(io: Option<BytesRefInputOutput<'_, i64>>) -> String {
    io.map_or_else(
        || "null".to_string(),
        |io| format!("{}:{}", hex(io.input.slice()), io.output),
    )
}

/// `RuceneFSTEnumReaderFixture.current`.
fn current(e: &BytesRefFSTEnum<'_, PositiveIntOutputs>) -> String {
    let io = e.current();
    let output = io
        .output
        .map_or_else(|| "null".to_string(), |o| o.to_string());
    format!("{}:{output}", hex(io.input.slice()))
}

/// The fixture's trace, run on `e`, built by a constructor that opened
/// `ctor` readers.
fn trace(
    mut e: BytesRefFSTEnum<'_, PositiveIntOutputs>,
    ctor: usize,
    opened: &AtomicUsize,
) -> Vec<String> {
    let mut out = vec![
        format!("ctor={ctor}"),
        format!("current.fresh={}", current(&e)),
    ];
    let count = || opened.load(Ordering::Relaxed);
    for i in 0.. {
        let before = count();
        let next = result(e.next().unwrap());
        let done = next == "null";
        out.push(format!("next.{i}={}:{next}", count() - before));
        if i == 0 {
            out.push(format!("current.first={}", current(&e)));
        }
        if done {
            break;
        }
    }
    out.push(format!("current.exhausted={}", current(&e)));
    for t in TARGETS {
        let target = BytesRef::new(t.to_vec());
        let key = hex(t);
        let before = count();
        let ceil = result(e.seek_ceil(&target).unwrap());
        out.push(format!("ceil.{key}={}:{ceil}", count() - before));
        let before = count();
        let floor = result(e.seek_floor(&target).unwrap());
        out.push(format!("floor.{key}={}:{floor}", count() - before));
        let before = count();
        let exact = result(e.seek_exact(&target).unwrap());
        out.push(format!("exact.{key}={}:{exact}", count() - before));
        out.push(format!("current.{key}={}", current(&e)));
    }
    out
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[test]
fn fst_enum_opens_the_readers_lucene_opens() {
    let expected = run_fixture();
    assert!(
        expected.len() > TARGETS.len() * 4,
        "the fixture printed too little"
    );

    let (fst, opened) = counted_fst();
    let before = opened.load(Ordering::Relaxed);
    let e = BytesRefFSTEnum::new(&fst).unwrap();
    let ctor = opened.load(Ordering::Relaxed) - before;
    assert_eq!(trace(e, ctor, &opened), expected, "BytesRefFSTEnum::new");

    let (fst, opened) = counted_fst();
    let fst = Arc::new(fst);
    let before = opened.load(Ordering::Relaxed);
    let e = BytesRefFSTEnum::new_shared(Arc::clone(&fst)).unwrap();
    let ctor = opened.load(Ordering::Relaxed) - before;
    assert_eq!(
        trace(e, ctor, &opened),
        expected,
        "BytesRefFSTEnum::new_shared"
    );
}
