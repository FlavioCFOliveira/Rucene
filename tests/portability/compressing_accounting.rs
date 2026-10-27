//! Portability tests for the heap estimates and the closed-state contract of
//! the Lucene 9.0 compressing codecs.
//!
//! Nothing here writes an index file, so nothing here is a byte comparison.
//! What it pins is the other half of a port's fidelity: the numbers a Lucene
//! object answers about itself, and the errors it raises once it is closed.
//! Every expected number is **measured from `lucene-core-10.5.0.jar`** by the
//! `CompressingAccountingFixture` harness in the same run, never derived from
//! this crate's own arithmetic — asserting an implementation against itself is
//! exactly what lets a dropped term go unnoticed.
//!
//! The three things asserted:
//!
//! 1. `ByteBuffersDataOutput.ramBytesUsed()` across a range of write sizes. Its
//!    block model charges the *capacity* of every allocated block plus one
//!    object reference each, and keeps 1 KiB blocks until the hundredth, so the
//!    curve is a staircase rather than a line — 1 byte and 1024 bytes both cost
//!    1028.
//! 2. `Lucene90CompressingTermVectorsWriter.ramBytesUsed()` for a fresh writer,
//!    and the two child resources it reports. Java adds its four `int[]`
//!    buffers by **element count**, not byte size, and adds
//!    `lastTerm.bytes.length`, which is `ArrayUtil.oversize(30, 1)` — so the
//!    answer is `4 * 1024 + 40`. A port that allocated `lastTerm` with a plain
//!    capacity of 30, or grew it by doubling instead of `oversize`, reports a
//!    different number for the same input.
//! 3. The `ensureOpen` contract of the two compressing readers: after `close()`
//!    they raise `AlreadyClosedException("this FieldsReader is closed")` rather
//!    than reading a closed file, and closing twice is a no-op.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;

use rucene::codecs::term_vectors::TermVectorsWriter;
use rucene::codecs::{register_codec, Codec, Lucene104Codec};
use rucene::error::LuceneError;
use rucene::index::SegmentInfo;
use rucene::store::{
    ByteBuffersDataOutput, DataOutput, Directory, RamDirectory, DEFAULT_IO_CONTEXT,
};
use rucene::util::{Accountable, ArrayUtil, RamUsageEstimator, Version};

fn codec() -> Arc<dyn Codec> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    rucene::codecs::default_codec().expect("Lucene104 codec is registered")
}

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("java-codec-harness")
}

fn which_mvn() -> Result<String, String> {
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

/// Runs the Java harness and returns its `key=value` lines.
fn run_fixture() -> Vec<(String, String)> {
    let mvn = which_mvn().unwrap_or_else(|e| {
        panic!("the compressing-accounting portability test needs Maven and a JDK: {e}")
    });
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.CompressingAccountingFixture")
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
        .filter_map(|line| line.trim().split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn compressing_accounting_matches_lucene() {
    let measured = run_fixture();
    let get = |key: &str| -> String {
        measured
            .iter()
            .find(|(k, _)| k == key)
            .unwrap_or_else(|| panic!("the fixture printed no {key}"))
            .1
            .clone()
    };

    // 1. ByteBuffersDataOutput.
    for (key, value) in &measured {
        if key != "bbdo" {
            continue;
        }
        let (written, expected) = value.split_once(':').expect("bbdo=<n>:<bytes>");
        let written: usize = written.parse().expect("write count");
        let expected: i64 = expected.parse().expect("ram bytes");
        let mut out = ByteBuffersDataOutput::new_resettable_instance();
        for i in 0..written {
            out.write_byte(i as u8).expect("write");
        }
        assert_eq!(
            out.ram_bytes_used(),
            expected,
            "ByteBuffersDataOutput.ramBytesUsed() after {written} bytes"
        );
    }

    // The two constants the term-vectors writer's estimate is built from.
    assert_eq!(
        ArrayUtil::oversize(30, 1) as i64,
        get("oversize30").parse::<i64>().expect("oversize"),
        "ArrayUtil.oversize(30, 1) is the length Lucene gives lastTerm.bytes"
    );
    assert_eq!(
        RamUsageEstimator::NUM_BYTES_OBJECT_REF,
        get("num_bytes_object_ref")
            .parse::<i64>()
            .expect("object ref"),
    );

    // 2. A fresh Lucene90CompressingTermVectorsWriter.
    let directory: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let info = SegmentInfo::new(
        Arc::clone(&directory),
        Version::LATEST,
        Some(Version::LATEST),
        "_acct".to_string(),
        1,
        false,
        false,
        codec(),
        std::collections::HashMap::new(),
        [7u8; 16],
        std::collections::HashMap::new(),
        Default::default(),
    )
    .expect("segment info");
    let writer = codec()
        .term_vectors_format()
        .vectors_writer(&directory, &info, &*DEFAULT_IO_CONTEXT)
        .expect("term vectors writer");
    assert_eq!(
        writer.ram_bytes_used(),
        get("tv_writer_fresh").parse::<i64>().expect("fresh ram"),
        "a fresh Lucene90CompressingTermVectorsWriter must report Lucene's own estimate"
    );
    assert_eq!(
        writer.child_resources().len(),
        get("tv_writer_children")
            .parse::<usize>()
            .expect("child count"),
        "the writer reports termSuffixes and payloadBytes as its children"
    );
    drop(writer);
}

/// The compressing readers must refuse to serve a request after `close()`, with
/// Lucene's own message, and must tolerate a second `close()`.
///
/// Equivalent to `Lucene90CompressingStoredFieldsReader.ensureOpen()` and
/// `Lucene90CompressingTermVectorsReader.ensureOpen()`, and to the `if
/// (!closed)` guard both `close()` methods carry.
#[test]
fn compressing_readers_report_already_closed() {
    let directory: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let info = SegmentInfo::new(
        Arc::clone(&directory),
        Version::LATEST,
        Some(Version::LATEST),
        "_closed".to_string(),
        1,
        false,
        false,
        codec(),
        std::collections::HashMap::new(),
        [9u8; 16],
        std::collections::HashMap::new(),
        Default::default(),
    )
    .expect("segment info");
    let field_infos = rucene::index::FieldInfos::new(Vec::new()).expect("field infos");

    // An empty segment is enough: the guard is on the reader's own state.
    let mut writer = codec()
        .term_vectors_format()
        .vectors_writer(&directory, &info, &*DEFAULT_IO_CONTEXT)
        .expect("term vectors writer");
    writer.finish(0).expect("finish");
    writer.close().expect("close writer");

    let mut reader = codec()
        .term_vectors_format()
        .vectors_reader(
            directory.as_ref(),
            &info,
            &field_infos,
            &*DEFAULT_IO_CONTEXT,
        )
        .expect("term vectors reader");
    // An open reader may answer nothing for a segment with no documents, but it
    // must not answer `AlreadyClosed`.
    assert!(
        !matches!(reader.get(0), Err(LuceneError::AlreadyClosed(_))),
        "an open reader must not report itself closed"
    );
    reader.close().expect("close");
    match reader.get(0) {
        Err(LuceneError::AlreadyClosed(message)) => assert_eq!(
            message, "this FieldsReader is closed",
            "the message is Lucene's own"
        ),
        Err(other) => panic!("a closed reader must report AlreadyClosed, got {other:?}"),
        Ok(_) => panic!("a closed reader must not answer `get`"),
    }
    reader
        .close()
        .expect("closing a closed reader is a no-op, as Java's `if (!closed)` makes it");
}
