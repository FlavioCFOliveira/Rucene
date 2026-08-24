//! Portability tests for the BKD leaf doc-id codec against Apache Lucene Core
//! 10.5.0.
//!
//! The Java fixture `BkdDocIdsFixture` writes a points index whose four fields
//! exercise the four doc-id encodings of `DocIdsWriter` — BITSET_IDS,
//! DELTA_BPV_16, BPV_21 and BPV_24 — and reads such an index back, printing
//! every doc id produced by a full-range `intersect`. Each test here drives
//! both sides of the same corpus:
//!
//! * `java_writes_rucene_reads` — Java writes the index, Rucene's `BKDReader`
//!   reads it and must reproduce every doc id. This is the direction that
//!   catches a Rucene reader that mis-decodes a Java leaf (the BITSET_IDS
//!   absolute-vs-relative bug, the missing BPV_21 reader, the scalar-vs-
//!   vectorized BPV_24 dispatch).
//! * `rucene_writes_java_reads` — Rucene's `BKDWriter` writes the index, Java
//!   reads it and must reproduce every doc id. This is the direction that
//!   catches a Rucene writer that emits a different byte layout than Java (the
//!   DELTA_BPV_16 adjacent-pairs bug, the missing BPV_21 writer, the scalar
//!   BPV_24 written under version 10).
//!
//! The corpus is a single leaf of 512 points per field with distinct values,
//! so the doc-id encoding is chosen purely by the doc-id layout:
//!
//! | field   | first doc | step | span / max                          | encoding   |
//! |---------|-----------|------|-------------------------------------|------------|
//! | `bitset`| 64        | 2    | span 1023 ≤ 512<<4                  | BITSET_IDS |
//! | `delta` | 2000      | 128  | span 65409 > 512<<4, ≤ 0xFFFF       | DELTA_BPV_16 |
//! | `bpv21` | 100000    | 256  | max ≤ 0x1FFFFF                      | BPV_21     |
//! | `bpv24` | 1966336   | 256  | max > 0x1FFFFF, ≤ 0xFFFFFF          | BPV_24     |
//!
//! A missing Maven or JDK is a hard failure, not a skip: a portability test has
//! nothing to assert without the reference implementation, so skipping would
//! report success while proving nothing. This matches the other tests under
//! `tests/portability/`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use rucene::document::IntPoint;
use rucene::index::point_values::{IntersectVisitor, Relation};
use rucene::store::{
    Directory, FSDirectory, RamDirectory, DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT,
};
use rucene::util::bkd::{BKDConfig, BKDReader, BKDWriter};

// ---------------------------------------------------------------------------
// Corpus
// ---------------------------------------------------------------------------

/// One more than the largest doc id used (2_097_152), matching the fixture.
const MAX_DOC: i32 = 2_097_153;

/// Points per field: one full leaf of the default 512-point leaf size.
const POINTS_PER_FIELD: i32 = 512;

/// Bytes per dimension: a single `IntPoint` dimension.
const BYTES_PER_DIM: i32 = 4;

struct FieldSpec {
    name: &'static str,
    first_doc: i32,
    step: i32,
}

const FIELDS: [FieldSpec; 4] = [
    FieldSpec {
        name: "bitset",
        first_doc: 64,
        step: 2,
    },
    FieldSpec {
        name: "delta",
        first_doc: 2000,
        step: 128,
    },
    FieldSpec {
        name: "bpv21",
        first_doc: 100000,
        step: 256,
    },
    FieldSpec {
        name: "bpv24",
        first_doc: 1966336,
        step: 256,
    },
];

fn expected_doc_ids(spec: &FieldSpec) -> Vec<i32> {
    (0..POINTS_PER_FIELD)
        .map(|i| spec.first_doc + spec.step * i)
        .collect()
}

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

/// Fails the test when Maven is unavailable.
fn require_maven() {
    if let Err(reason) = which_mvn() {
        panic!("BKD doc-id portability tests require Maven and a JDK: {reason}");
    }
}

/// Runs the fixture with the given mode and returns its stdout.
fn run_fixture(out_dir: &Path, mode: &str) -> Result<String, String> {
    let _guard = HARNESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let harness = harness_dir();
    if !harness.join("pom.xml").exists() {
        return Err(format!("pom.xml not found in {}", harness.display()));
    }
    let mvn = which_mvn()?;
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dlucene.useScalarFMA=false")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.BkdDocIdsFixture")
        .arg(format!("-Dexec.args={} {mode}", out_dir.display()))
        .current_dir(&harness)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("failed to spawn Maven: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(format!(
            "BkdDocIdsFixture/{mode} failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ));
    }
    Ok(stdout)
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rucene-portability-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory is creatable");
    dir
}

// ---------------------------------------------------------------------------
// Rucene side
// ---------------------------------------------------------------------------

/// Writes the corpus with Rucene's `BKDWriter`, producing `<field>.kdm`,
/// `<field>.kdi` and `<field>.kdd` in `dir`, exactly as the Java fixture's
/// write mode does.
fn write_corpus(dir: &Path) {
    let directory = FSDirectory::open(dir).expect("directory");
    for spec in FIELDS {
        let config = BKDConfig::of(
            1,
            1,
            BYTES_PER_DIM,
            BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE,
        )
        .expect("BKDConfig");
        let mut meta = directory
            .create_output(&format!("{}.kdm", spec.name), &*DEFAULT_IO_CONTEXT)
            .expect("kdm output");
        let mut index = directory
            .create_output(&format!("{}.kdi", spec.name), &*DEFAULT_IO_CONTEXT)
            .expect("kdi output");
        let mut data = directory
            .create_output(&format!("{}.kdd", spec.name), &*DEFAULT_IO_CONTEXT)
            .expect("kdd output");

        // The writer's temp directory is never touched for this corpus: 512
        // points fit in the heap budget, so the point writer stays in memory.
        let temp_dir: Box<dyn Directory> = Box::new(RamDirectory::new());
        let mut writer = BKDWriter::new_default(
            MAX_DOC,
            temp_dir,
            spec.name,
            config,
            16.0,
            POINTS_PER_FIELD as i64,
        )
        .expect("BKDWriter");
        for i in 0..POINTS_PER_FIELD {
            let mut packed = vec![0u8; BYTES_PER_DIM as usize];
            IntPoint::encode_dimension(i, &mut packed, 0);
            writer
                .add(&packed, spec.first_doc + spec.step * i)
                .expect("add point");
        }
        writer
            .finish(&mut *meta, &mut *index, &mut *data)
            .expect("finish");
        writer.close().expect("close writer");
        meta.close().expect("close meta");
        index.close().expect("close index");
        data.close().expect("close data");
    }
}

/// Collects every doc id a full-range `intersect` produces.
///
/// Only the single-document callbacks are overridden; the bulk ones keep their
/// interface defaults, which fan out to the single-document ones. That matters:
/// `DocIdsWriter` picks between `visit(IntsRef)`, `visit(DocIdSetIterator)` and
/// `visit(int)` according to how the doc ids of a leaf were encoded, so only
/// the fanned-out trace is a property of the traversal algorithm rather than
/// of the doc-id codec.
#[derive(Default)]
struct DocIdCollector {
    doc_ids: Vec<i32>,
}

impl IntersectVisitor for DocIdCollector {
    fn visit(&mut self, doc_id: i32) -> rucene::error::Result<()> {
        self.doc_ids.push(doc_id);
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, _packed_value: &[u8]) -> rucene::error::Result<()> {
        self.doc_ids.push(doc_id);
        Ok(())
    }

    fn compare(&self, _min_packed_value: &[u8], _max_packed_value: &[u8]) -> Relation {
        Relation::CellCrossesQuery
    }
}

/// Reads the corpus with Rucene's `BKDReader` and returns the doc ids of every
/// field, in field order.
fn read_corpus(dir: &Path) -> Vec<Vec<i32>> {
    let directory = FSDirectory::open(dir).expect("directory");
    FIELDS
        .iter()
        .map(|spec| {
            let mut meta = directory
                .open_input(&format!("{}.kdm", spec.name), &*READONCE_IO_CONTEXT)
                .expect("kdm input");
            let mut index = directory
                .open_input(&format!("{}.kdi", spec.name), &*READONCE_IO_CONTEXT)
                .expect("kdi input");
            let mut data = directory
                .open_input(&format!("{}.kdd", spec.name), &*READONCE_IO_CONTEXT)
                .expect("kdd input");
            let mut reader =
                BKDReader::new(&mut *meta, &mut *index, &mut *data).expect("BKDReader");
            let mut collector = DocIdCollector::default();
            reader.intersect(&mut collector).expect("intersect");
            collector.doc_ids
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Java writes the index, Rucene reads it and reproduces every doc id.
#[test]
fn java_writes_rucene_reads() {
    require_maven();
    let dir = scratch_dir("bkd-doc-ids-java-writes");
    let stdout = run_fixture(&dir, "write").expect("fixture write mode");

    // The fixture reports the corpus it wrote; sanity-check the header lines so
    // a fixture regression fails here rather than as a confusing doc-id diff.
    assert!(
        stdout.contains("fixture=BkdDocIdsFixture"),
        "fixture header missing:\n{stdout}"
    );
    assert!(
        stdout.contains("mode=write"),
        "fixture mode missing:\n{stdout}"
    );
    assert!(
        stdout.contains("max_doc=2097153"),
        "fixture max_doc missing:\n{stdout}"
    );
    assert!(
        stdout.contains("points_per_field=512"),
        "fixture points_per_field missing:\n{stdout}"
    );

    let doc_ids = read_corpus(&dir);
    for (spec, actual) in FIELDS.iter().zip(&doc_ids) {
        assert_eq!(
            *actual,
            expected_doc_ids(spec),
            "field {}: Rucene mis-read a Java-written leaf",
            spec.name
        );
    }
}

/// Rucene writes the index, Java reads it and reproduces every doc id.
#[test]
fn rucene_writes_java_reads() {
    require_maven();
    let dir = scratch_dir("bkd-doc-ids-rucene-writes");
    write_corpus(&dir);
    let stdout = run_fixture(&dir, "read").expect("fixture read mode");

    for spec in FIELDS {
        let line = stdout
            .lines()
            .find(|line| line.starts_with(&format!("field {} ", spec.name)))
            .unwrap_or_else(|| {
                panic!("fixture printed no line for field {}:\n{stdout}", spec.name)
            });
        let value = line.split_once("doc_ids=").expect("doc_ids= token").1;
        let actual: Vec<i32> = if value.is_empty() {
            Vec::new()
        } else {
            value
                .split(',')
                .map(|token| token.parse().expect("doc id"))
                .collect()
        };
        assert_eq!(
            actual,
            expected_doc_ids(&spec),
            "field {}: Java mis-read a Rucene-written leaf",
            spec.name
        );
    }
}
