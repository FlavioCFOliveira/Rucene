//! Portability tests for the `Lucene90DocValuesFormat` skip index and the
//! `rangeIntoBitSet` overrides of its producer.
//!
//! The Java harness (`DocValuesSkipperFixture`) writes one segment with
//! Apache Lucene Core 10.5.0, whose doc-values fields carry a skip index (the
//! `.dvs` file), walks every skipper with Lucene's own reader, and evaluates
//! `rangeIntoBitSet` on fresh iterators. This test opens the very same index
//! with Rucene and asserts that:
//!
//! 1. every skipper reports Lucene's global bounds and maximum value count, and
//!    visits Lucene's intervals with the same levels, document ranges, value
//!    ranges and document counts (`Lucene90DocValuesProducer.getSkipper`,
//!    `Lucene90DocValuesProducer.java:2368-2470`);
//! 2. `rangeIntoBitSet` sets the same bits **and leaves the iterator where
//!    Lucene leaves it**: the dense single-block overrides read the values by
//!    document number without moving the iterator
//!    (`Lucene90DocValuesProducer.java:911-970, 1898-1925, 1997-2028`), while
//!    the generic `NumericDocValues.rangeIntoBitSet` advances it. The fields
//!    cover the GCD-1, GCD/delta, fixed-cardinality, sparse and constant
//!    layouts;
//! 3. walks with targets several intervals and level groups ahead (`jump=`
//!    lines) visit the same intervals, which exercises the per-level jump
//!    (`Lucene90DocValuesProducer.java:2408-2411`) and its table
//!    (`Lucene90DocValuesFormat.java:232-246`) at every level: the `seq`
//!    field is written with a 16-document skip interval so that all four
//!    levels exist; and value-range walks through
//!    `DocValuesSkipper.advance(long, long)` (`DocValuesSkipper.java:118-136`,
//!    `vrange=` lines) visit the same intervals.
//!
//! A missing Maven or JDK is a hard failure: a portability test has nothing to
//! assert without the reference implementation.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::index::directory_reader;
use rucene::index::doc_values::DocValuesSkipper;
use rucene::search::NO_MORE_DOCS;
use rucene::store::{Directory, FSDirectory};
use rucene::util::FixedBitSet;

const SKIPPER_FIELDS: [&str; 5] = ["num", "gcd", "snfix", "snvar", "snconst"];
const JUMP_FIELDS: [&str; 6] = ["num", "gcd", "snfix", "snvar", "snconst", "seq"];
const JUMP_STRIDES: [i32; 6] = [700, 1500, 5000, 9000, 13000, 33000];
const VALUE_RANGES: [(i64, i64); 5] = [
    (43, 50),
    (2000, 2100),
    (5000, 5000),
    (9000, 9500),
    (13000, 13010),
];

static HARNESS_LOCK: Mutex<()> = Mutex::new(());

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java-codec-harness")
}

fn run_java(out_dir: &Path) -> Vec<String> {
    let _guard = HARNESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let output = Command::new("mvn")
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.DocValuesSkipperFixture")
        .arg(format!("-Dexec.args={}", out_dir.display()))
        .current_dir(harness_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| panic!("doc-values skipper portability tests need Maven: {e}"));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success() && stdout.lines().any(|l| l.trim() == "done=true"),
        "the Java fixture failed:\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| {
            l.starts_with("max_doc=")
                || l.starts_with("skipper=")
                || l.starts_with("interval=")
                || l.starts_with("range=")
                || l.starts_with("jump=")
                || l.starts_with("vrange=")
        })
        .map(str::to_string)
        .collect()
}

fn describe(field: &str, bits: &FixedBitSet, doc_id: i32) -> String {
    let mut hash: i64 = 0;
    for i in 0..bits.length() {
        if bits.get(i) {
            hash = hash.wrapping_mul(31).wrapping_add(i as i64);
        }
    }
    format!(
        "range={field} cardinality={} hash={hash} docID={doc_id}",
        bits.cardinality()
    )
}

fn levels(skipper: &dyn DocValuesSkipper) -> String {
    let mut line = format!(" levels={}", skipper.num_levels());
    for level in 0..skipper.num_levels() {
        line.push_str(&format!(
            " [{},{},{},{},{}]",
            skipper.min_doc_id(level),
            skipper.max_doc_id(level),
            skipper.min_value(level),
            skipper.max_value(level),
            skipper.doc_count(level)
        ));
    }
    line
}

/// Mirrors the fixture's `jump=` walk: each advance targets `stride`
/// documents past the previous target, or the next interval when that is
/// further.
fn jump_walk(field: &str, stride: i32, skipper: &mut dyn DocValuesSkipper, out: &mut Vec<String>) {
    let mut target = stride;
    loop {
        skipper.advance(target).expect("advance the skipper");
        if skipper.min_doc_id(0) == NO_MORE_DOCS {
            out.push(format!("jump={field} stride={stride} exhausted"));
            break;
        }
        out.push(format!(
            "jump={field} stride={stride} target={target}{}",
            levels(skipper)
        ));
        target = (skipper.max_doc_id(0) + 1).max(target + stride);
    }
}

/// Mirrors the fixture's `vrange=` walk over `DocValuesSkipper.advance(long,
/// long)`.
fn value_range_walk(
    field: &str,
    (min, max): (i64, i64),
    skipper: &mut dyn DocValuesSkipper,
    out: &mut Vec<String>,
) {
    loop {
        skipper
            .advance_range(min, max)
            .expect("advance the skipper by value");
        if skipper.min_doc_id(0) == NO_MORE_DOCS {
            out.push(format!("vrange={field} {min}..{max} exhausted"));
            break;
        }
        out.push(format!("vrange={field} {min}..{max}{}", levels(skipper)));
        let next = skipper.max_doc_id(0) + 1;
        skipper.advance(next).expect("advance the skipper");
    }
}

fn walk_skipper(field: &str, skipper: &mut dyn DocValuesSkipper, out: &mut Vec<String>) {
    out.push(format!(
        "skipper={field} min={} max={} docCount={} maxValueCount={}",
        skipper.global_min_value(),
        skipper.global_max_value(),
        skipper.global_doc_count(),
        skipper.max_value_count()
    ));
    let mut target = 0;
    loop {
        skipper.advance(target).expect("advance the skipper");
        if skipper.min_doc_id(0) == NO_MORE_DOCS {
            out.push(format!("interval={field} exhausted"));
            break;
        }
        let mut line = format!("interval={field} levels={}", skipper.num_levels());
        for level in 0..skipper.num_levels() {
            line.push_str(&format!(
                " [{},{},{},{},{}]",
                skipper.min_doc_id(level),
                skipper.max_doc_id(level),
                skipper.min_value(level),
                skipper.max_value(level),
                skipper.doc_count(level)
            ));
        }
        out.push(line);
        target = skipper.max_doc_id(0) + 1;
    }
}

fn read_with_rucene(dir: &Path) -> Vec<String> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let directory: Arc<dyn Directory> = Arc::new(FSDirectory::open(dir).expect("open"));
    let reader = directory_reader::open(directory).expect("open the reader");
    let leaves = reader.leaves();
    assert_eq!(leaves.len(), 1, "the fixture force-merges to one segment");
    let leaf = leaves[0].leaf_reader();
    let mut out = vec![format!("max_doc={}", leaf.max_doc())];
    for field in SKIPPER_FIELDS {
        let mut skipper = leaf
            .get_doc_values_skipper(field)
            .expect("skipper")
            .unwrap_or_else(|| panic!("{field} has a skip index"));
        walk_skipper(field, skipper.as_mut(), &mut out);
    }

    for field in JUMP_FIELDS {
        let skipper = || {
            leaf.get_doc_values_skipper(field)
                .expect("skipper")
                .unwrap_or_else(|| panic!("{field} has a skip index"))
        };
        for stride in JUMP_STRIDES {
            jump_walk(field, stride, skipper().as_mut(), &mut out);
        }
        for range in VALUE_RANGES {
            value_range_walk(field, range, skipper().as_mut(), &mut out);
        }
    }

    for (field, from, to, min, max) in [
        ("num", 100, 30000, 20000, 60000),
        ("gcd", 100, 30000, 100000, 300000),
    ] {
        let mut values = leaf
            .get_numeric_doc_values(field)
            .expect("numeric")
            .expect("numeric field");
        let mut bits = FixedBitSet::new((to - from) as usize);
        values
            .range_into_bit_set(from, to, min, max, &mut bits, from)
            .expect("range into bit set");
        out.push(describe(field, &bits, values.doc_id()));
    }
    for (field, from, to, min, max) in [
        ("snfix", 100, 30000, 2000, 2500),
        ("snvar", 100, 30000, 5000, 9000),
        ("snconst", 100, 30000, 40, 50),
        ("snconst", 100, 30000, 43, 50),
    ] {
        let mut values = leaf
            .get_sorted_numeric_doc_values(field)
            .expect("sorted numeric")
            .expect("sorted numeric field");
        let mut bits = FixedBitSet::new((to - from) as usize);
        values
            .range_into_bit_set(from, to, min, max, &mut bits, from)
            .expect("range into bit set");
        out.push(describe(field, &bits, values.doc_id()));
    }
    out
}

#[test]
fn skipper_and_range_into_bit_set_match_lucene_over_a_java_index() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let java = run_java(tmp.path());
    assert!(
        java.iter().any(|l| l.starts_with("interval=snvar levels=")),
        "the fixture must expose at least one interval: {java:?}"
    );
    let rucene = read_with_rucene(tmp.path());
    assert_eq!(
        java.len(),
        rucene.len(),
        "java:\n{java:#?}\nrucene:\n{rucene:#?}"
    );
    for (j, r) in java.iter().zip(&rucene) {
        assert_eq!(j, r, "Rucene diverges from Lucene");
    }
}
