//! `SortingCodecReader.wrap(CodecReader, Sort)` sorts a segment as Lucene does.
//!
//! Java's `wrap(reader, sort)` is `wrap(reader, new Sorter(sort).sort(reader),
//! sort)` (`SortingCodecReader.java:390-392`): `Sorter(Sort)` refuses a sort
//! that needs scores, `Sorter.sort(LeafReader)` builds one comparator per sort
//! field from the field's `IndexSorter` — whose values come through
//! `DocValues.getNumeric`, `getSorted`, `getSortedNumeric` and `getSortedSet`
//! — and a reader already in order is returned as-is under the new metadata.
//! `SortingCodecReaderWrapFixture` (`tests/fixtures/java-codec-harness`, run
//! with `java -cp lucene-core-10.5.0.jar SortingCodecReaderWrapFixture.java`)
//! wraps one five-document segment with every sort below and prints the
//! resulting document order; Rucene must print the same. Two segments of
//! document blocks, with and without a parent field, cover the block half of
//! `Sorter.sort(LeafReader)` (`Sorter.java:190-207`).

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{
    Document, NumericDocValuesField, SortedDocValuesField, SortedNumericDocValuesField,
    SortedSetDocValuesField, StoredField,
};
use rucene::index::codec_reader::CodecReader;
use rucene::index::filter_reader::FilterCodecReader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::segment_reader::SegmentReader;
use rucene::index::sorting_codec_reader::SortingCodecReader;
use rucene::index::SegmentInfos;
use rucene::search::sorted_numeric_selector::SortedNumericSelectorType;
use rucene::search::sorted_set_selector::SortedSetSelectorType;
use rucene::search::{
    MissingValue, Sort, SortField, SortFieldType, SortedNumericSortField, SortedSetSortField,
};
use rucene::store::{ByteBuffersDirectory, Directory, DEFAULT_IO_CONTEXT};
use rucene::util::BytesRef;
use rucene::{LuceneError, LuceneErrorKind};

/// `SortingCodecReaderWrapFixture.LS`.
const LS: [i64; 5] = [5, 3, 9, 3, 1];

fn bytes(text: &str) -> BytesRef {
    BytesRef::new(text.as_bytes().to_vec())
}

/// The segment of `SortingCodecReaderWrapFixture.main`, read as a codec reader.
fn segment() -> Arc<dyn CodecReader> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    for (i, &l) in LS.iter().enumerate() {
        let mut doc = Document::new();
        doc.add(Box::new(
            StoredField::new_string("id", format!("d{i}")).unwrap(),
        ));
        doc.add(Box::new(NumericDocValuesField::new("l", l)));
        doc.add(Box::new(SortedDocValuesField::new(
            "s",
            bytes(&format!("s{}", 9 - l)),
        )));
        if i != 2 {
            doc.add(Box::new(NumericDocValuesField::new(
                "sparse",
                10 - i as i64,
            )));
        }
        doc.add(Box::new(SortedNumericDocValuesField::new("sn", l)));
        doc.add(Box::new(SortedNumericDocValuesField::new("sn", -l)));
        doc.add(Box::new(SortedSetDocValuesField::new(
            "ss",
            bytes(&format!("a{l}")),
        )));
        doc.add(Box::new(SortedSetDocValuesField::new(
            "ss",
            bytes(&format!("z{i}")),
        )));
        writer.add_document(&doc).unwrap();
    }
    writer.close().unwrap();
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(infos.size(), 1, "the fixture is one segment");
    Arc::new(
        SegmentReader::new(
            infos.info(0).clone(),
            infos.index_created_version_major(),
            &*DEFAULT_IO_CONTEXT,
        )
        .unwrap(),
    )
}

/// The segments of `SortingCodecReaderWrapFixture`'s block cases: three
/// blocks of a child and its parent, the parents' values `7, 2, 5`.
fn block_segment(with_parent: bool) -> Arc<dyn CodecReader> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let config = IndexWriterConfig::new();
    if with_parent {
        config.set_parent_field(Some("parent".to_string()));
    }
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for (b, parent_value) in [7i64, 2, 5].into_iter().enumerate() {
        let mut child = Document::new();
        child.add(Box::new(
            StoredField::new_string("id", format!("b{b}c")).unwrap(),
        ));
        child.add(Box::new(NumericDocValuesField::new("l", 100 - b as i64)));
        let mut parent = Document::new();
        parent.add(Box::new(
            StoredField::new_string("id", format!("b{b}p")).unwrap(),
        ));
        parent.add(Box::new(NumericDocValuesField::new("l", parent_value)));
        writer.add_documents(&[child, parent]).unwrap();
    }
    writer.close().unwrap();
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(infos.size(), 1, "the blocks are one segment");
    Arc::new(
        SegmentReader::new(
            infos.info(0).clone(),
            infos.index_created_version_major(),
            &*DEFAULT_IO_CONTEXT,
        )
        .unwrap(),
    )
}

fn long(field: &str) -> SortField {
    SortField::new(Some(field.to_string()), SortFieldType::Long).unwrap()
}

/// `SortingCodecReaderWrapFixture.sorts()`.
fn sorts() -> Vec<Sort> {
    let one = |field: SortField| Sort::new_fields(vec![field]).unwrap();
    let mut sparse_missing = long("sparse");
    sparse_missing
        .set_missing_value(Some(MissingValue::Long(100)))
        .unwrap();
    vec![
        one(long("l")),
        one(SortField::new_reverse(Some("l".to_string()), SortFieldType::Long, true).unwrap()),
        one(SortField::new(Some("s".to_string()), SortFieldType::String).unwrap()),
        one(long("sparse")),
        one(sparse_missing),
        one(SortedNumericSortField::new("sn", SortFieldType::Long)
            .unwrap()
            .into_sort_field()),
        one(SortedNumericSortField::with_selector(
            "sn",
            SortFieldType::Long,
            false,
            SortedNumericSelectorType::MAX,
        )
        .unwrap()
        .into_sort_field()),
        one(SortedSetSortField::new("ss", false)
            .unwrap()
            .into_sort_field()),
        one(
            SortedSetSortField::with_selector("ss", true, SortedSetSelectorType::MAX)
                .unwrap()
                .into_sort_field(),
        ),
        one(SortedNumericSortField::new("l", SortFieldType::Long)
            .unwrap()
            .into_sort_field()),
        one(SortedSetSortField::new("s", false)
            .unwrap()
            .into_sort_field()),
        one(long("id")),
        one(long("absent")),
        one(SortField::new(Some("id".to_string()), SortFieldType::String).unwrap()),
        one(SortedNumericSortField::new("id", SortFieldType::Long)
            .unwrap()
            .into_sort_field()),
        one(SortedSetSortField::new("id", false)
            .unwrap()
            .into_sort_field()),
        one(SortField::FIELD_DOC.clone()),
        one(SortField::FIELD_SCORE.clone()),
        Sort::new_fields(vec![
            long("l"),
            SortField::new(Some("s".to_string()), SortFieldType::String).unwrap(),
        ])
        .unwrap(),
    ]
}

fn java_exception(error: &LuceneError) -> String {
    let message = match error {
        LuceneError {
            kind: LuceneErrorKind::IllegalArgument { message, .. },
            ..
        } => message.clone(),
        LuceneError {
            kind: LuceneErrorKind::IllegalState { message, .. },
            ..
        } => message.clone(),
        other => other.to_string(),
    };
    format!("{}: {message}", error.java_simple_name())
}

fn meta_sort(reader: &dyn CodecReader) -> String {
    reader
        .get_meta_data()
        .sort()
        .map_or_else(|| "null".to_string(), ToString::to_string)
}

fn cache_helpers(reader: &dyn CodecReader) -> String {
    let null = |present: bool| if present { "present" } else { "null" };
    format!(
        "core={} readerCache={}",
        null(reader.get_core_cache_helper().is_some()),
        null(reader.get_reader_cache_helper().is_some())
    )
}

fn rucene_report() -> String {
    let leaf = segment();
    let mut out = String::new();
    for sort in sorts() {
        match SortingCodecReader::wrap_sort(Arc::clone(&leaf), sort.clone()) {
            Ok(sorted) => {
                let stored = sorted.stored_fields().unwrap();
                let ids: Vec<String> = (0..sorted.max_doc())
                    .map(|doc| stored.document(doc).unwrap().get_values("id").join(","))
                    .collect();
                out.push_str(&format!(
                    "W {sort} -> {} | metaSort={} {}\n",
                    ids.join(" "),
                    meta_sort(sorted.as_ref()),
                    cache_helpers(sorted.as_ref())
                ));
            }
            Err(error) => out.push_str(&format!("W {sort} -> {}\n", java_exception(&error))),
        }
    }
    for with_parent in [true, false] {
        let label = if with_parent {
            "B parent"
        } else {
            "B no-parent"
        };
        let blocks = block_segment(with_parent);
        match SortingCodecReader::wrap_sort(blocks, Sort::new_fields(vec![long("l")]).unwrap()) {
            Ok(sorted) => {
                let stored = sorted.stored_fields().unwrap();
                let ids: Vec<String> = (0..sorted.max_doc())
                    .map(|doc| stored.document(doc).unwrap().get_values("id").join(","))
                    .collect();
                out.push_str(&format!("{label} -> {}\n", ids.join(" ")));
            }
            Err(LuceneError {
                kind: LuceneErrorKind::CorruptIndex { message, .. },
                ..
            }) => {
                out.push_str(&format!("{label} -> CorruptIndexException: {message}\n"));
            }
            Err(error) => out.push_str(&format!("{label} -> {}\n", java_exception(&error))),
        }
    }
    let by_l = Sort::new_fields(vec![long("l")]).unwrap();
    let once = SortingCodecReader::wrap_sort(Arc::clone(&leaf), by_l.clone()).unwrap();
    let twice = SortingCodecReader::wrap_sort(Arc::clone(&once), by_l).unwrap();
    out.push_str(&format!(
        "A delegate={} unwrap={} toString={} metaSort={} {}\n",
        twice
            .filter_codec_reader_delegate()
            .is_some_and(|delegate| Arc::ptr_eq(delegate, &once)),
        Arc::ptr_eq(&FilterCodecReader::unwrap(&twice), &leaf),
        twice.to_string() == format!("SortingCodecReader({once})"),
        meta_sort(twice.as_ref()),
        cache_helpers(twice.as_ref())
    ));
    out
}

fn java_report() -> String {
    let jar = Path::new(&std::env::var("HOME").unwrap())
        .join(".m2/repository/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar");
    assert!(
        jar.is_file(),
        "the Lucene 10.5.0 jar is required: {}",
        jar.display()
    );
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         SortingCodecReaderWrapFixture.java",
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(jar)
        .arg(fixture)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "SortingCodecReaderWrapFixture failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Every sort orders the segment, reports itself in the metadata and refuses
/// what Lucene refuses, exactly as `SortingCodecReader.wrap(CodecReader,
/// Sort)` does; an already sorted view is wrapped as Lucene's `FilterCodecReader`.
#[test]
fn wrap_sort_orders_a_segment_as_lucene_does() {
    assert_eq!(rucene_report(), java_report());
}
