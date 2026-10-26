//! The RAM the five doc-values writers charge while documents are buffered.
//!
//! Pins rmp #192 rows 64 and 66-69 against Apache Lucene Core 10.5.0: the
//! writers buffer their values in `PackedLongValues` builders and a
//! `PagedBytes` (`NumericDocValuesWriter.java`, `BinaryDocValuesWriter.java`,
//! `SortedDocValuesWriter.java`, `SortedNumericDocValuesWriter.java`,
//! `SortedSetDocValuesWriter.java`), intern the sorted values into the
//! indexing chain's shared `docValuesBytePool`, and charge the writer's RAM
//! counter exactly what those structures report. The port buffered in plain
//! vectors, so its RAM figure — and therefore its RAM-triggered flush points —
//! differed from Lucene's.
//!
//! # Where the expected values come from
//!
//! `DocValuesRamFixture` from `tests/fixtures/java-codec-harness`
//! (`java -cp lucene-core-10.5.0.jar DocValuesRamFixture.java`) builds one
//! writer of each kind as `IndexingChain.initializeFieldInfo` does — one shared
//! `Counter`, and one `ByteBlockPool` over a `DirectTrackingAllocator` of it for
//! the two sorted kinds — feeds them the same values, and prints the counter
//! after every document.
//!
//! The values reach the growth paths: up to twelve values per sorted-set
//! document grow `currentValues` past its initial 8 slots
//! (`SortedSetDocValuesWriter.java:207-210`), and the binary values fill more
//! than 16 `PagedBytes` blocks, which grows the blocks array
//! (`PagedBytes.java:165-168`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use rucene::index::{
    BinaryDocValuesWriter, DocValuesType, FieldInfo, NumericDocValuesWriter, SortedDocValuesWriter,
    SortedNumericDocValuesWriter, SortedSetDocValuesWriter,
};
use rucene::util::ByteBlockPool;

/// `DocValuesRamFixture.DOCS`.
const DOCS: i32 = 400;

fn lucene_jar() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME is set");
    let jar = Path::new(&home)
        .join(".m2/repository/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar");
    assert!(
        jar.is_file(),
        "the Lucene 10.5.0 jar is required: {}",
        jar.display()
    );
    jar
}

fn field(name: &str, number: i32, doc_values_type: DocValuesType) -> FieldInfo {
    let mut info = FieldInfo::new(name, number);
    info.doc_values_type = doc_values_type;
    info
}

#[test]
fn the_doc_values_writers_charge_what_lucene_charges() {
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
             DocValuesRamFixture.java",
        ))
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "DocValuesRamFixture failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let java: Vec<String> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter(|line| line.starts_with("init ") || line.starts_with("doc "))
        .map(str::to_string)
        .collect();
    assert_eq!(java.len(), DOCS as usize + 1);

    let bytes_used = Arc::new(AtomicI64::new(0));
    // The indexing chain's `docValuesBytePool`, charging the same counter.
    let mut pool = Arc::new(ByteBlockPool::new(Arc::clone(&bytes_used)));
    let mut num = NumericDocValuesWriter::new(
        field("num", 0, DocValuesType::NUMERIC),
        Arc::clone(&bytes_used),
    );
    let mut blob = BinaryDocValuesWriter::new(
        field("blob", 1, DocValuesType::BINARY),
        Arc::clone(&bytes_used),
    );
    let mut kind = SortedDocValuesWriter::new(
        field("kind", 2, DocValuesType::SORTED),
        Arc::clone(&bytes_used),
    );
    let mut multi = SortedNumericDocValuesWriter::new(
        field("multi", 3, DocValuesType::SORTED_NUMERIC),
        Arc::clone(&bytes_used),
    );
    let mut tags = SortedSetDocValuesWriter::new(
        field("tags", 4, DocValuesType::SORTED_SET),
        Arc::clone(&bytes_used),
    );
    let mut rust = vec![format!("init {}", bytes_used.load(Ordering::SeqCst))];
    for i in 0..DOCS {
        let i64v = i64::from(i);
        if i % 5 != 3 {
            num.add_value(i, (i64v * 7919) % 100003 - 50000).unwrap();
        }
        if i % 3 != 1 {
            let length = if i % 17 == 0 { 5000 + i } else { (i * 13) % 40 };
            let value: Vec<u8> = (0..length).map(|k| ((i + k) % 251) as u8).collect();
            blob.add_value(i, &value).unwrap();
        }
        if i % 4 != 2 {
            kind.add_value(i, format!("kind{}", (i * 31) % 97).as_bytes(), &mut pool)
                .unwrap();
        }
        for k in 0..i % 4 {
            multi
                .add_value(i, (i64v * 1000003 + i64::from(k) * 17) % 5000)
                .unwrap();
        }
        for k in 0..(i * 7) % 13 {
            tags.add_value(
                i,
                format!("tag{}", (i + k * 11) % 230).as_bytes(),
                &mut pool,
            )
            .unwrap();
        }
        rust.push(format!("doc {i} {}", bytes_used.load(Ordering::SeqCst)));
    }

    let first_difference = rust.iter().zip(&java).position(|(r, j)| r != j);
    assert!(
        first_difference.is_none(),
        "the doc-values RAM diverges from Lucene's at line {:?}: rucene `{}` vs lucene `{}`",
        first_difference,
        first_difference.map_or("", |at| rust[at].as_str()),
        first_difference.map_or("", |at| java[at].as_str()),
    );
}
