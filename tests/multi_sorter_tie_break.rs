//! `MultiSorter` breaks ties between readers as Lucene does.
//!
//! When the sort values of the queued documents are equal, the
//! `PriorityQueue<LeafAndDocID>` of `MultiSorter.sort` prefers the lower reader
//! index, then the lower document id (`MultiSorter.java:87-104`).
//! `MultiSorterTieFixture` (`tests/fixtures/java-codec-harness`, run with
//! `java -cp lucene-core-10.5.0.jar MultiSorterTieFixture.java`) sorts three
//! readers whose keys tie across readers — `[1, 2]`, `[1, 2]` and `[2]` — and
//! prints every document's merged number; Rucene must produce the same.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, NumericDocValuesField};
use rucene::index::directory_reader;
use rucene::index::index_sorter::MultiSorter;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::LeafReader;
use rucene::search::{SortField, SortFieldType};
use rucene::store::{Directory, RamDirectory};

/// `MultiSorterTieFixture.READERS`.
const READERS: [&[i64]; 3] = [&[1, 2], &[1, 2], &[2]];

fn reader(values: &[i64]) -> Arc<dyn LeafReader> {
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    for &value in values {
        let mut document = Document::new();
        document.add(Box::new(NumericDocValuesField::new("n", value)));
        writer.add_document(&document).unwrap();
    }
    writer.commit().unwrap();
    writer.close().unwrap();
    let reader = directory_reader::open(dir).unwrap();
    let leaves = Arc::clone(&reader).leaves();
    assert_eq!(leaves.len(), 1);
    leaves[0].leaf_reader()
}

fn java_doc_maps() -> String {
    let jar = Path::new(&std::env::var("HOME").unwrap())
        .join(".m2/repository/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar");
    assert!(
        jar.is_file(),
        "the Lucene 10.5.0 jar is required: {}",
        jar.display()
    );
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         MultiSorterTieFixture.java",
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(jar)
        .arg(fixture)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "MultiSorterTieFixture failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn ties_across_readers_merge_in_lucenes_order() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let readers: Vec<Arc<dyn LeafReader>> = READERS.iter().map(|values| reader(values)).collect();
    let sorter = SortField::new(Some("n".to_string()), SortFieldType::Long)
        .unwrap()
        .get_index_sorter()
        .expect("a long field sorts an index");
    let maps = MultiSorter::sort(&[sorter], &[1], &readers)
        .unwrap()
        .expect("the readers interleave, so a merge sort is needed");
    let mut rust = String::new();
    for (i, (map, reader)) in maps.iter().zip(&readers).enumerate() {
        rust.push_str(&format!("reader {i}:"));
        for doc in 0..reader.max_doc() {
            rust.push_str(&format!(" {}", map(doc)));
        }
        rust.push('\n');
    }
    assert_eq!(rust, java_doc_maps());
}
