//! Parity tests for `PrefixCodedTerms` against Apache Lucene Core 10.5.0.
//!
//! Lucene keeps the prefix-coded payload as the `List<ByteBuffer>` a
//! `ByteBuffersDataOutput` produced (`PrefixCodedTerms.java:37`, `:103`) and
//! reads it back with a `ByteBuffersDataInput` over those same blocks
//! (`:170-172`). The block structure is observable in two places: the decoded
//! term sequence has to survive a payload split across blocks, and
//! `ramBytesUsed()` sums each block's *capacity* (`:49-51`), so it answers the
//! same 1040 for any packet that fits in one 1 KiB block and grows a block at a
//! time after that.
//!
//! Both are compared against `lucene-core-10.5.0.jar` in the same run, so
//! nothing here asserts the port against its own arithmetic.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use rucene::index::{PrefixCodedTermsBuilder, Term};
use rucene::util::{Accountable, BytesRef};

// ---------------------------------------------------------------------------
// Java harness driver
// ---------------------------------------------------------------------------

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java-codec-harness")
}

/// Serialises Maven invocations, which all share one `target/` directory.
static HARNESS_LOCK: Mutex<()> = Mutex::new(());

fn which_mvn() -> Result<String, String> {
    for candidate in ["mvn", "mvnw"] {
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

/// Fails the test when Maven is unavailable: a portability test that skipped
/// would report success while proving nothing.
fn require_maven() {
    if let Err(reason) = which_mvn() {
        panic!("prefix-coded-terms portability tests require Maven and a JDK: {reason}");
    }
}

/// One case: the terms to pack, in ascending `Term` order.
type Case = Vec<(String, Vec<u8>)>;

/// What each side reports for one case.
#[derive(Debug, PartialEq, Eq)]
struct Report {
    size: i64,
    ram_bytes_used: i64,
    /// `field:hexBytes` per decoded term, in iteration order.
    terms: Vec<String>,
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Runs every case through `lucene-core-10.5.0.jar`.
fn java_reports(cases: &[Case]) -> Vec<Report> {
    let _guard = HARNESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let dir = tempfile::tempdir().expect("temp dir");
    let input = dir.path().join("cases.tsv");
    let output = dir.path().join("reports.tsv");

    let mut wire = String::new();
    for case in cases {
        let fields: Vec<String> = case
            .iter()
            .flat_map(|(field, bytes)| [field.clone(), to_hex(bytes)])
            .collect();
        wire.push_str(&fields.join("\t"));
        wire.push('\n');
    }
    std::fs::write(&input, wire).expect("write cases");

    let mvn = which_mvn().expect("Maven");
    let result = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.PrefixCodedTermsFixture")
        .arg(format!(
            "-Dexec.args={} {}",
            input.display(),
            output.display()
        ))
        .current_dir(harness_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn Maven");
    assert!(
        result.status.success(),
        "Java harness failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );

    let text = std::fs::read_to_string(&output).expect("read reports");
    let reports: Vec<Report> = text
        .lines()
        .map(|line| {
            let mut fields = line.split('\t');
            let size = fields.next().expect("size").parse().expect("size is i64");
            let ram_bytes_used = fields.next().expect("ram").parse().expect("ram is i64");
            Report {
                size,
                ram_bytes_used,
                terms: fields.map(str::to_string).collect(),
            }
        })
        .collect();
    assert_eq!(reports.len(), cases.len());
    reports
}

/// Runs one case through the port.
fn rucene_report(case: &Case) -> Report {
    let mut builder = PrefixCodedTermsBuilder::new();
    for (field, bytes) in case {
        builder
            .add(&Term::new(field.clone(), BytesRef::new(bytes.clone())))
            .expect("terms are in ascending order");
    }
    let packed = builder.finish();

    let mut terms = Vec::new();
    let mut iterator = packed.iterator();
    while let Some(bytes) = iterator.next().expect("decode") {
        terms.push(format!(
            "{}:{}",
            iterator.field().unwrap(),
            to_hex(bytes.slice())
        ));
    }
    Report {
        size: packed.size(),
        ram_bytes_used: packed.ram_bytes_used(),
        terms,
    }
}

/// The case grid.
///
/// The sizes straddle the 1 KiB block boundary in both directions, because the
/// block count is what `ramBytesUsed` is a function of, and a payload that
/// spans blocks is what proves the reader crosses them correctly.
fn build_cases() -> Vec<Case> {
    let mut cases: Vec<Case> = Vec::new();

    // Empty.
    cases.push(Vec::new());
    // One term.
    cases.push(vec![("f".to_string(), b"a".to_vec())]);
    // The shape the accounting note quotes: three short terms over two fields.
    cases.push(vec![
        ("f".to_string(), b"aaa".to_vec()),
        ("f".to_string(), b"aab".to_vec()),
        ("g".to_string(), b"z".to_vec()),
    ]);
    // Shared prefixes of every length, including a term that is a prefix of the
    // next, so the prefix/suffix split is exercised at both extremes.
    cases.push(
        (0..32u8)
            .map(|i| ("field".to_string(), vec![b'p'; i as usize]))
            .collect(),
    );
    // Non-UTF-8 bytes, including 0x00 and 0xFF.
    cases.push(vec![
        ("bin".to_string(), vec![0x00]),
        ("bin".to_string(), vec![0x00, 0xFF]),
        ("bin".to_string(), vec![0x7F, 0x80, 0xFF]),
        ("bin".to_string(), vec![0xFF; 40]),
    ]);
    // A field name that needs multi-byte UTF-8, and the empty term.
    cases.push(vec![
        ("caça".to_string(), Vec::new()),
        ("caça".to_string(), b"\xc3\xa9".to_vec()),
    ]);
    // Just under, just over and well over one 1 KiB block.
    for count in [60usize, 90, 120, 400] {
        cases.push(
            (0..count)
                .map(|i| ("f".to_string(), format!("term{i:08}").into_bytes()))
                .collect(),
        );
    }
    // Many distinct fields, so a new-field code is written for every term.
    cases.push(
        (0..80u32)
            .map(|i| (format!("f{i:04}"), b"x".to_vec()))
            .collect(),
    );
    cases
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The decoded term sequence and the reported cost both match Lucene's.
///
/// The term sequence is what "behaviour unchanged" means for this class; the
/// cost is what proves the payload is held as a list of 1 KiB blocks and not as
/// one tightly sized buffer, which is the difference the migration made.
#[test]
fn decoded_terms_and_ram_bytes_used_match_lucene() {
    require_maven();
    let cases = build_cases();
    let expected = java_reports(&cases);
    for (index, (case, java)) in cases.iter().zip(expected.iter()).enumerate() {
        let ours = rucene_report(case);
        assert_eq!(
            ours.terms,
            java.terms,
            "case {index} ({} terms) decodes differently",
            case.len()
        );
        assert_eq!(ours.size, java.size, "case {index} has a different size");
        assert_eq!(
            ours.ram_bytes_used,
            java.ram_bytes_used,
            "case {index} ({} terms) costs {} here and {} in lucene-core-10.5.0",
            case.len(),
            ours.ram_bytes_used,
            java.ram_bytes_used
        );
    }
}

/// Every iterator starts at the beginning and they do not interfere.
///
/// Lucene's `iterator()` opens a fresh `ByteBuffersDataInput` over the shared
/// blocks each time (`PrefixCodedTerms.java:170-172`), so the position belongs
/// to the iterator and not to the packet. Two iterators advanced in lockstep
/// therefore see the same terms.
#[test]
fn two_iterators_over_one_packet_are_independent() {
    let case: Case = (0..200)
        .map(|i| ("f".to_string(), format!("term{i:08}").into_bytes()))
        .collect();
    let mut builder = PrefixCodedTermsBuilder::new();
    for (field, bytes) in &case {
        builder
            .add(&Term::new(field.clone(), BytesRef::new(bytes.clone())))
            .expect("ascending");
    }
    let packed = builder.finish();

    let mut first = packed.iterator();
    let mut second = packed.iterator();
    // Advance the first one alone, then check the second still starts over.
    for _ in 0..50 {
        first.next().expect("decode").expect("term");
    }
    let mut seen = 0usize;
    while let Some(bytes) = second.next().expect("decode") {
        assert_eq!(bytes.slice(), case[seen].1.as_slice(), "term {seen}");
        seen += 1;
    }
    assert_eq!(seen, case.len());
    // And the first one carries on from where it was.
    let mut rest = 0usize;
    while first.next().expect("decode").is_some() {
        rest += 1;
    }
    assert_eq!(rest, case.len() - 50);
}
