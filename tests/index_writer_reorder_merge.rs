//! A merge whose `OneMerge.reorder` renumbers the documents.
//!
//! Pins `IndexWriter.mergeMiddle` (`IndexWriter.java:5340-5362`) against Apache
//! Lucene Core 10.5.0: the segments are read through
//! `SlowCompositeCodecReaderWrapper`, the merged view is handed to
//! `OneMerge.reorder`, and a `SortingCodecReader` over it is what the merge
//! writes. Every kind of data — postings, norms, stored fields, the five kinds
//! of doc values, points and float vectors — goes through those two wrappers.
//!
//! # Where the expected bytes come from
//!
//! `ReorderMergeFixture` from `tests/fixtures/java-codec-harness`
//! (`java -cp lucene-core-10.5.0.jar ReorderMergeFixture.java <dir>`) writes the
//! same documents as two committed segments and force-merges them with a
//! `TieredMergePolicy` whose merges reverse the document order. A segment id is
//! random, so every merged file is compared with Lucene's after Lucene's
//! segment id is replaced by Rucene's; the footer's checksum is checked on its
//! own.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use rucene::codecs::Lucene104Codec;
use rucene::document::{
    BinaryDocValuesField, Document, IntPoint, KnnFloatVectorField, NumericDocValuesField,
    SortedDocValuesField, SortedNumericDocValuesField, SortedSetDocValuesField, Store, StoredField,
    StringField, TextField,
};
use rucene::error::Result;
use rucene::index::codec_reader::CodecReader;
use rucene::index::filter_directory_reader::SlowCodecReaderWrapper;
use rucene::index::index_sorter::{DocComparator, Sorter};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::{
    MergeContext, MergePolicy, MergeReorderer, MergeSpecification, MergeTrigger, NoMergePolicy,
    OneMerge, OneMergeWrappingMergePolicy, TieredMergePolicy,
};
use rucene::index::merge_scheduler::{MergeScheduler, SerialMergeScheduler};
use rucene::index::{directory_reader, SegmentInfos};
use rucene::search::NO_MORE_DOCS;
use rucene::store::{ByteBuffersDirectory, Directory, DEFAULT_IO_CONTEXT};
use rucene::util::BytesRef;

/// `ReorderMergeFixture.SIZES`.
const SIZES: [i32; 2] = [40, 60];

fn bytes(text: String) -> BytesRef {
    BytesRef::new(text.into_bytes())
}

/// `ReorderMergeFixture.doc(int)`, field for field and in the same order.
fn doc(i: i32) -> Document {
    let mut d = Document::new();
    d.add(Box::new(
        StringField::new("id", format!("id{i:04}"), Store::NO).unwrap(),
    ));
    d.add(Box::new(
        TextField::new("body", format!("w{} w{} x{i}", i % 7, i % 13), Store::NO).unwrap(),
    ));
    d.add(Box::new(
        StoredField::new_string("sv", format!("v{i}")).unwrap(),
    ));
    d.add(Box::new(NumericDocValuesField::new(
        "ndv",
        (i64::from(i) * 31) % 100,
    )));
    d.add(Box::new(BinaryDocValuesField::new(
        "bdv",
        bytes(format!("b{}", i % 17)),
    )));
    d.add(Box::new(SortedDocValuesField::new(
        "sdv",
        bytes(format!("s{}", i % 11)),
    )));
    d.add(Box::new(SortedNumericDocValuesField::new(
        "sndv",
        i64::from(i % 5),
    )));
    d.add(Box::new(SortedNumericDocValuesField::new(
        "sndv",
        i64::from(i % 3),
    )));
    d.add(Box::new(SortedSetDocValuesField::new(
        "ssdv",
        bytes(format!("a{}", i % 4)),
    )));
    d.add(Box::new(SortedSetDocValuesField::new(
        "ssdv",
        bytes(format!("b{}", i % 6)),
    )));
    d.add(Box::new(IntPoint::new("pt", &[i, (i * 7) % 50]).unwrap()));
    d.add(Box::new(
        KnnFloatVectorField::with_euclidean(
            "vec",
            &[(i % 9) as f32, ((i * 3) % 7) as f32, 1.0, (i % 4) as f32],
        )
        .unwrap(),
    ));
    d
}

/// `ReorderMergeFixture`'s merge policy: every merge reverses the documents.
fn reversing_policy() -> Arc<dyn MergePolicy> {
    Arc::new(OneMergeWrappingMergePolicy::new(
        Arc::new(TieredMergePolicy::new()),
        Arc::new(|mut merge: OneMerge| {
            merge.set_reorder(Arc::new(|reader, _dir, _executor| {
                let reverse: DocComparator = Box::new(|a, b| b.cmp(&a));
                Ok(Sorter::sort(reader.max_doc(), &reverse).map(Arc::new))
            }));
            merge
        }),
    ))
}

fn rust_merge() -> Arc<dyn Directory> {
    let _ = rucene::codecs::register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let mut initial = IndexWriterConfig::new();
    initial
        .set_merge_policy(Some(Arc::new(NoMergePolicy::new())))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), initial).unwrap();
    let mut i = 0;
    for size in SIZES {
        for _ in 0..size {
            writer.add_document(&doc(i)).unwrap();
            i += 1;
        }
        writer.commit().unwrap();
    }
    writer.close().unwrap();
    assert_eq!(
        SegmentInfos::read_latest_commit(Arc::clone(&dir))
            .unwrap()
            .size(),
        SIZES.len()
    );

    let mut config = IndexWriterConfig::new();
    config.set_merge_policy(Some(reversing_policy())).unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.force_merge(1).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();
    dir
}

/// What `ReorderMergeFixture` printed about Lucene's merged segment.
struct JavaMerge {
    dir: PathBuf,
    name: String,
    id: [u8; 16],
    max_doc: i32,
    files: BTreeMap<String, u64>,
    _tmp: tempfile::TempDir,
}

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

fn java_merge() -> JavaMerge {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         ReorderMergeFixture.java",
    );
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("reordered");
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(&fixture)
        .arg(&dir)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "ReorderMergeFixture failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let (mut name, mut id, mut max_doc) = (None, None, None);
    let mut files = BTreeMap::new();
    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("name=") {
            name = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("id=") {
            let decoded: Vec<u8> = (0..value.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&value[at..at + 2], 16).unwrap())
                .collect();
            id = Some(<[u8; 16]>::try_from(decoded).unwrap());
        } else if let Some(value) = line.strip_prefix("maxDoc=") {
            let (count, deletions) = value.split_once(" delCount=").unwrap();
            assert_eq!(deletions, "0");
            max_doc = Some(count.parse().unwrap());
        } else if let Some(value) = line.strip_prefix("file ") {
            let (file, length) = value.split_once(' ').unwrap();
            files.insert(file.to_string(), length.parse().unwrap());
        } else if let Some(value) = line.strip_prefix("segments=") {
            assert_eq!(value, "1");
        }
    }
    JavaMerge {
        dir,
        name: name.expect("the fixture printed the segment name"),
        id: id.expect("the fixture printed the segment id"),
        max_doc: max_doc.expect("the fixture printed the document count"),
        files,
        _tmp: tmp,
    }
}

fn read_file(dir: &dyn Directory, name: &str) -> Vec<u8> {
    let length = dir.file_length(name).unwrap() as usize;
    let mut input = dir.open_input(name, &*DEFAULT_IO_CONTEXT).unwrap();
    let mut bytes = vec![0u8; length];
    input.read_bytes(Some(&mut bytes), 0, length).unwrap();
    bytes
}

fn assert_valid_footer(side: &str, name: &str, file: &[u8]) {
    let body = &file[..file.len() - 8];
    let stored = u64::from_be_bytes(file[file.len() - 8..].try_into().unwrap());
    assert_eq!(
        stored,
        u64::from(crc32fast::hash(body)),
        "{side} {name}: the footer checksum must cover the file"
    );
}

fn replace_id(bytes: &mut [u8], from: &[u8; 16], to: &[u8; 16]) -> usize {
    let mut count = 0;
    let mut at = 0;
    while at + 16 <= bytes.len() {
        if &bytes[at..at + 16] == from {
            bytes[at..at + 16].copy_from_slice(to);
            count += 1;
            at += 16;
        } else {
            at += 1;
        }
    }
    count
}

#[test]
fn a_reordering_merge_writes_the_segment_lucene_writes() {
    let java = java_merge();
    let dir = rust_merge();

    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(infos.size(), 1);
    let merged = infos.info(0);
    assert_eq!(merged.info.name, java.name, "the merged segment's name");
    assert_eq!(merged.info.max_doc().unwrap(), java.max_doc, "maxDoc");
    let rust_id = merged.info.id();
    let files: BTreeSet<String> = merged.files().unwrap().into_iter().collect();
    assert_eq!(
        files,
        java.files.keys().cloned().collect::<BTreeSet<_>>(),
        "the merged segment's files"
    );
    let mut differences = Vec::new();
    for name in &files {
        if name.ends_with(".si") {
            continue;
        }
        let mut expected = std::fs::read(java.dir.join(name)).unwrap();
        let actual = read_file(dir.as_ref(), name);
        assert_valid_footer("Lucene", name, &expected);
        assert_valid_footer("Rucene", name, &actual);
        assert!(replace_id(&mut expected, &java.id, &rust_id) >= 1);
        let body = actual.len().min(expected.len()) - 8;
        let equal = actual.len() == expected.len() && actual[..body] == expected[..body];
        println!(
            "{name} lucene={} rucene={} equal={equal}",
            expected.len(),
            actual.len()
        );
        if !equal {
            differences.push(format!(
                "{name}: Lucene has {} bytes, Rucene {}; first difference at {:?}",
                expected.len(),
                actual.len(),
                (0..body).find(|&at| actual[at] != expected[at])
            ));
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));

    // The merge reversed the documents of the merged view. `TieredMergePolicy`
    // lists the larger segment first, so the view is documents 40..100 then
    // 0..40, and the merged segment holds that sequence backwards.
    let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    let leaves = Arc::clone(&reader).leaves().unwrap();
    let mut values = leaves[0]
        .leaf_reader()
        .get_numeric_doc_values("ndv")
        .unwrap()
        .unwrap();
    let view: Vec<i32> = (SIZES[0]..SIZES[0] + SIZES[1]).chain(0..SIZES[0]).collect();
    loop {
        let doc = values.next_doc().unwrap();
        if doc == NO_MORE_DOCS {
            break;
        }
        let original = view[view.len() - 1 - doc as usize];
        assert_eq!(
            values.long_value().unwrap(),
            (i64::from(original) * 31) % 100,
            "merged document {doc}"
        );
    }
}

/// The address of the executor a reorderer was handed, `None` for Java's
/// `null`.
fn executor_address(
    executor: Option<&Arc<dyn rucene::search::task_executor::Executor>>,
) -> Option<usize> {
    executor.map(|executor| Arc::as_ptr(executor) as *const () as usize)
}

/// A reorderer that records the executor it is handed and leaves the
/// documents alone.
fn recording_reorderer(seen: &Arc<Mutex<Vec<Option<usize>>>>) -> MergeReorderer {
    let seen = Arc::clone(seen);
    Arc::new(move |_reader, _dir, executor| {
        seen.lock().unwrap().push(executor_address(executor));
        Ok(None)
    })
}

/// A policy that merges nothing on its own and installs `reorderer` on the
/// merge `addIndexes(CodecReader...)` asks it for, as an override of
/// `MergePolicy.findMerges(CodecReader...)` would.
struct ReorderingReadersPolicy {
    reorderer: MergeReorderer,
}

impl std::fmt::Display for ReorderingReadersPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReorderingReadersPolicy")
    }
}

impl MergePolicy for ReorderingReadersPolicy {
    fn find_merges(
        &self,
        _merge_trigger: MergeTrigger,
        _segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }

    fn find_merges_for_readers(
        &self,
        readers: &[Arc<dyn CodecReader>],
    ) -> Result<Option<MergeSpecification>> {
        let mut merge = OneMerge::from_codec_readers(readers.to_vec());
        merge.set_reorder(Arc::clone(&self.reorderer));
        let mut spec = MergeSpecification::new();
        spec.add(merge);
        Ok(Some(spec))
    }

    fn find_forced_merges(
        &self,
        _segment_infos: &SegmentInfos,
        _max_num_segments: i32,
        _segments_to_merge: &HashMap<String, bool>,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }

    fn find_forced_deletes_merges(
        &self,
        _segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }
}

/// `OneMerge.reorder(CodecReader, Directory, Executor)` receives the executor
/// `MergeScheduler.getIntraMergeExecutor(merge)` returns, on both of the
/// writer's merge paths: `mergeMiddle` (`IndexWriter.java:5327-5343`) and
/// `addIndexesReaderMerge` (`IndexWriter.java:3547-3551`). A
/// `SerialMergeScheduler` answers with its own `SameThreadExecutorService`
/// (`MergeScheduler.java:60-67`), so the reorderer is handed exactly that
/// object, twice.
#[test]
fn the_reorderer_receives_the_schedulers_intra_merge_executor() {
    let _ = rucene::codecs::register_codec("Lucene104", Lucene104Codec::new());
    let scheduler = Arc::new(SerialMergeScheduler::new());
    let expected = Arc::as_ptr(scheduler.same_thread_executor()) as *const () as usize;
    let seen: Arc<Mutex<Vec<Option<usize>>>> = Arc::new(Mutex::new(Vec::new()));

    // Two committed segments, then a forced merge: `mergeMiddle`.
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let mut initial = IndexWriterConfig::new();
    initial
        .set_merge_policy(Some(Arc::new(NoMergePolicy::new())))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), initial).unwrap();
    for i in 0..4 {
        writer.add_document(&doc(i)).unwrap();
        if i % 2 == 1 {
            writer.commit().unwrap();
        }
    }
    writer.close().unwrap();

    let reorderer = recording_reorderer(&seen);
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Some(Arc::new(OneMergeWrappingMergePolicy::new(
            Arc::new(TieredMergePolicy::new()),
            Arc::new(move |mut merge: OneMerge| {
                merge.set_reorder(Arc::clone(&reorderer));
                merge
            }),
        ))))
        .unwrap();
    config
        .set_merge_scheduler(Some(Arc::clone(&scheduler) as Arc<dyn MergeScheduler>))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.force_merge(1).unwrap();
    writer.close().unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![Some(expected)]);

    // `addIndexes(CodecReader...)`: `addIndexesReaderMerge`.
    let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    let leaves: Vec<Arc<dyn CodecReader>> = Arc::clone(&reader)
        .leaves()
        .unwrap()
        .iter()
        .map(|leaf| SlowCodecReaderWrapper::wrap(leaf.leaf_reader()).unwrap())
        .collect();
    let dest: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Some(Arc::new(ReorderingReadersPolicy {
            reorderer: recording_reorderer(&seen),
        })))
        .unwrap();
    config
        .set_merge_scheduler(Some(Arc::clone(&scheduler) as Arc<dyn MergeScheduler>))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dest), config).unwrap();
    writer.add_indexes_codec_readers(&leaves).unwrap();
    writer.close().unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![Some(expected), Some(expected)]);
}
