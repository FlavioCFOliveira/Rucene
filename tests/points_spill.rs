//! rmp #244: the BKD writer spills to the segment's directory, as Lucene does.
//!
//! Lucene gives every `BKDWriter` the segment's own directory for its
//! temporary files (`Lucene90PointsWriter.java:149-157, 249-257`).
//! `initPointWriter` buffers points in a temporary `spill` file once
//! `totalPointCount` exceeds what `maxMBSortInHeap` holds
//! (`BKDWriter.java:222-232`), and `finish` partitions through
//! `BKDRadixSelector`, writing every partition that does not fit in heap to a
//! further temporary file and deleting each one once consumed
//! (`BKDWriter.java:930-1034, 1922-2126`). The port handed the writer a
//! private `RamDirectory` and kept every point in heap.
//!
//! * `bkd_writer_spills_and_writes_what_lucene_writes` drives `BKDWriter`
//!   directly against `BkdSpillFixture` from
//!   `tests/fixtures/java-codec-harness`
//!   (`java -cp lucene-core-10.5.0.jar BkdSpillFixture.java <dir>`): the same
//!   points, the same heap budgets, the same temporary files created and
//!   deleted in the same order, and the same `.kdd`, `.kdi` and `.kdm` bytes.
//! * `a_points_merge_spills_to_and_cleans_the_segment_directory` force-merges
//!   two-dimensional points through `IndexWriter` with a points format whose
//!   heap budget is too small for the merged field, and observes the segment
//!   directory: temporary files are created there, every one is deleted, and
//!   the merged points files are byte for byte those of the same merge with
//!   Lucene's default budget, which keeps every point in heap.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use rucene::codecs::lucene90::{Lucene90PointsReader, Lucene90PointsWriter};
use rucene::codecs::state::{SegmentReadState, SegmentWriteState};
use rucene::codecs::{
    register_codec, Codec, FilterCodec, Lucene104Codec, PointsFormat, PointsReader, PointsWriter,
};
use rucene::document::{Document, IntPoint};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::SegmentInfos;
use rucene::store::{
    Directory, FSDirectory, IOContext, IndexInput, IndexOutput, Lock, RamDirectory,
    DEFAULT_IO_CONTEXT,
};
use rucene::util::bkd::{BKDConfig, BKDWriter};

// ---------------------------------------------------------------------------
// A directory that records its temporary-file traffic
// ---------------------------------------------------------------------------

/// Delegates to `inner` and records, in order, every temporary file created
/// and every file deleted, as `BkdSpillFixture`'s `FilterDirectory` does.
struct RecordingDirectory {
    inner: Box<dyn Directory>,
    events: Arc<Mutex<Vec<String>>>,
}

impl RecordingDirectory {
    fn new(inner: Box<dyn Directory>) -> (Self, Arc<Mutex<Vec<String>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                inner,
                events: Arc::clone(&events),
            },
            events,
        )
    }
}

impl std::fmt::Debug for RecordingDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingDirectory").finish_non_exhaustive()
    }
}

impl Directory for RecordingDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.inner.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.events.lock().unwrap().push(format!("delete {name}"));
        self.inner.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.inner.file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.inner.create_output(name, context)
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        let output = self.inner.create_temp_output(prefix, suffix, context)?;
        self.events
            .lock()
            .unwrap()
            .push(format!("create {}", output.name()));
        Ok(output)
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        self.inner.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.inner.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.inner.rename(source, dest)
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.inner.open_input(name, context)
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.inner.obtain_lock(name)
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn get_pending_deletions(&self) -> Result<std::collections::HashSet<String>> {
        self.inner.get_pending_deletions()
    }
}

fn read_file(dir: &dyn Directory, name: &str) -> Vec<u8> {
    let length = dir.file_length(name).unwrap() as usize;
    let mut input = dir.open_input(name, &*DEFAULT_IO_CONTEXT).unwrap();
    let mut bytes = vec![0u8; length];
    input.read_bytes(&mut bytes, 0, length).unwrap();
    bytes
}

// ---------------------------------------------------------------------------
// BKDWriter against BkdSpillFixture
// ---------------------------------------------------------------------------

/// `BkdSpillFixture.CASES`: name, numDims, numIndexDims, bytesPerDim,
/// maxPointsInLeafNode, points, maxMBSortInHeap.
const CASES: [(&str, i32, i32, i32, i32, i32, f64); 11] = [
    ("spill2d", 2, 2, 4, 64, 5000, 0.005),
    ("heap2d", 2, 2, 4, 64, 5000, 16.0),
    ("spill3d2i", 3, 2, 4, 64, 3000, 0.004),
    ("heap3d2i", 3, 2, 4, 64, 3000, 16.0),
    ("spill4d", 4, 4, 2, 32, 4000, 0.002),
    ("heap4d", 4, 4, 2, 32, 4000, 16.0),
    // Duplicates: a partition's common prefix spans past the dimension into
    // the data dimensions and the doc ID, where `dimCmpBytes` is negative
    // (`BKDRadixSelector.java:410-428`).
    ("dupspill2d", 2, 2, 4, 64, 5000, 0.005),
    ("dupspill1d", 1, 1, 4, 16, 5000, 0.001),
    ("ddspill2d", 2, 2, 4, 64, 5000, 0.005),
    ("ddspill3d", 3, 2, 4, 64, 5000, 0.005),
    // Dimension 0 is split first though not the last index dimension, and
    // ties on the doc ID; dimension 1 decreases as the doc ID grows.
    ("dupsplit0", 2, 2, 4, 64, 5000, 0.005),
];

/// `BkdSpillFixture.MULTIPLIERS` and `MODULI`.
const MULTIPLIERS: [i64; 4] = [7919, 104_729, 1_299_709, 15_485_863];
const MODULI: [i64; 4] = [1000, 997, 13, 50021];

/// `BkdSpillFixture.point(String, int, int, int)`.
fn point(name: &str, i: i32, num_dims: i32, bytes_per_dim: i32) -> Vec<u8> {
    let mut packed = vec![0u8; (num_dims * bytes_per_dim) as usize];
    let i64_i = i64::from(i);
    for d in 0..num_dims as usize {
        let value = if name.starts_with("dup") {
            if name == "dupsplit0" {
                if d == 0 {
                    (i64_i % 2) * 1_000_000
                } else {
                    (5000 - i64_i) % 4999
                }
            } else {
                (i64_i * MULTIPLIERS[d]) % 3
            }
        } else if name.starts_with("dd") {
            match d {
                0 => {
                    if i < 2500 {
                        0
                    } else {
                        1000
                    }
                }
                1 => (5000 - i64_i) % 997,
                _ => (i64_i * 31) % 7,
            }
        } else {
            (i64_i * MULTIPLIERS[d]) % MODULI[d]
        };
        for b in 0..bytes_per_dim as usize {
            packed[d * bytes_per_dim as usize + b] =
                (value >> (8 * (bytes_per_dim as usize - 1 - b))) as u8;
        }
    }
    packed
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

#[test]
fn bkd_writer_spills_and_writes_what_lucene_writes() {
    let java_tmp = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         BkdSpillFixture.java",
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(&fixture)
        .arg(java_tmp.path())
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "BkdSpillFixture failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.lines().any(|line| line == "done"));

    let rust_tmp = tempfile::tempdir().unwrap();
    for (name, num_dims, num_index_dims, bytes_per_dim, max_points_in_leaf, points, max_mb) in CASES
    {
        let out_dir = FSDirectory::open(rust_tmp.path()).unwrap();
        let temp_path = rust_tmp.path().join(format!("{name}-tmp"));
        std::fs::create_dir_all(&temp_path).unwrap();
        let (temp, events) =
            RecordingDirectory::new(Box::new(FSDirectory::open(&temp_path).unwrap()));
        let temp: Arc<dyn Directory> = Arc::new(temp);

        let config =
            BKDConfig::of(num_dims, num_index_dims, bytes_per_dim, max_points_in_leaf).unwrap();
        let mut writer = BKDWriter::new(
            points / 2 + 1,
            Arc::clone(&temp),
            &format!("bkd{name}"),
            config,
            max_mb,
            i64::from(points),
            BKDWriter::VERSION_CURRENT,
        )
        .unwrap();
        let mut meta = out_dir
            .create_output(&format!("{name}.kdm"), &*DEFAULT_IO_CONTEXT)
            .unwrap();
        let mut index = out_dir
            .create_output(&format!("{name}.kdi"), &*DEFAULT_IO_CONTEXT)
            .unwrap();
        let mut data = out_dir
            .create_output(&format!("{name}.kdd"), &*DEFAULT_IO_CONTEXT)
            .unwrap();
        for i in 0..points {
            writer
                .add(&point(name, i, num_dims, bytes_per_dim), i / 2)
                .unwrap();
        }
        let finalizer = writer
            .finish(data.as_mut())
            .unwrap()
            .expect("points were added");
        finalizer.run(meta.as_mut(), index.as_mut()).unwrap();
        writer.close().unwrap();
        meta.close().unwrap();
        index.close().unwrap();
        data.close().unwrap();

        let prefix = format!("event {name} ");
        let expected_events: Vec<String> = stdout
            .lines()
            .filter_map(|line| line.strip_prefix(&prefix).map(str::to_string))
            .collect();
        assert_eq!(
            *events.lock().unwrap(),
            expected_events,
            "[{name}] the temporary files must be created and deleted as Lucene does"
        );
        if !name.starts_with("heap") {
            assert!(
                expected_events
                    .first()
                    .is_some_and(|event| event.contains("_bkd_spill_")),
                "[{name}] the case must spill, or it tests nothing"
            );
        } else {
            assert!(
                expected_events.is_empty(),
                "[{name}] a heap case spills nothing"
            );
        }
        assert!(
            temp.list_all().unwrap().is_empty(),
            "[{name}] no temporary file is left behind"
        );
        assert!(stdout.lines().any(|line| line == format!("left {name} ")));

        for extension in ["kdd", "kdi", "kdm"] {
            let file = format!("{name}.{extension}");
            let expected = std::fs::read(java_tmp.path().join(&file)).unwrap();
            let actual = std::fs::read(rust_tmp.path().join(&file)).unwrap();
            assert!(
                actual == expected,
                "[{name}] {file} differs from Lucene's ({} vs {} bytes)",
                actual.len(),
                expected.len()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// A points merge through IndexWriter
// ---------------------------------------------------------------------------

/// `Lucene90PointsFormat` with an explicit heap budget, as Java's
/// `new Lucene90PointsWriter(writeState, maxPointsInLeafNode, maxMBSortInHeap)`
/// (`Lucene90PointsWriter.java:107-111`) allows.
#[derive(Debug)]
struct BudgetPointsFormat {
    max_mb_sort_in_heap: f64,
}

impl PointsFormat for BudgetPointsFormat {
    fn name(&self) -> &str {
        "Lucene90"
    }

    fn fields_writer(&self, state: &SegmentWriteState) -> Result<Box<dyn PointsWriter>> {
        Ok(Box::new(Lucene90PointsWriter::with_config(
            state,
            512,
            self.max_mb_sort_in_heap,
            rucene::codecs::lucene90::points::VERSION_CURRENT,
        )?))
    }

    fn fields_reader(&self, state: &SegmentReadState) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(Lucene90PointsReader::new(state)?))
    }
}

/// Writes 2 segments of 2500 two-dimensional points with a points format whose
/// heap budget is `max_mb`, force-merges them, and returns the directory's
/// recorded temporary-file traffic and the merged segment's points files.
/// The merge's temporary-file events, the merged points files by name, and
/// the files left in the directory.
type MergeOutcome = (Vec<String>, Vec<(String, Vec<u8>)>, Vec<String>);

fn merge_points(max_mb: f64, value: fn(i32) -> [i32; 2]) -> MergeOutcome {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let (dir, events) = RecordingDirectory::new(Box::new(RamDirectory::new()));
    let dir: Arc<dyn Directory> = Arc::new(dir);
    let codec: Arc<dyn Codec> = Arc::new(
        FilterCodec::new("Lucene104", Arc::new(Lucene104Codec::new())).with_points_format(
            BudgetPointsFormat {
                max_mb_sort_in_heap: max_mb,
            },
        ),
    );
    let mut config = IndexWriterConfig::new();
    config.set_codec(codec).unwrap();
    // Loose files, so the merged points files can be read one by one.
    config.set_use_compound_file(false).unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for segment in 0..2 {
        for i in 0..2500 {
            let i = segment * 2500 + i;
            let mut doc = Document::new();
            doc.add(Box::new(IntPoint::new("p", &value(i)).unwrap()));
            writer.add_document(&doc).unwrap();
        }
        writer.commit().unwrap();
    }
    let flush_events = events.lock().unwrap().len();
    writer.force_merge(1).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();

    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(infos.size(), 1);
    let merged = &infos.info(0).info;
    let id = merged.id();
    let mut files: Vec<(String, Vec<u8>)> = merged
        .files()
        .unwrap()
        .into_iter()
        .filter(|name| {
            [".kdd", ".kdi", ".kdm"]
                .iter()
                .any(|ext| name.ends_with(ext))
        })
        .map(|name| {
            let mut bytes = read_file(&*dir, &name);
            // The segment id in each header is random; mask it, and check the
            // footer's checksum on its own before it stops matching.
            let body = &bytes[..bytes.len() - 8];
            let stored = u64::from_be_bytes(bytes[bytes.len() - 8..].try_into().unwrap());
            assert_eq!(stored, u64::from(crc32fast::hash(body)), "{name} footer");
            let at = bytes
                .windows(16)
                .position(|window| window == id)
                .expect("the header carries the segment id");
            bytes[at..at + 16].fill(0);
            let footer = bytes.len() - 8;
            bytes[footer..].fill(0);
            (name, bytes)
        })
        .collect();
    files.sort();
    let merge_events = events.lock().unwrap()[flush_events..].to_vec();
    (merge_events, files, dir.list_all().unwrap())
}

/// Distinct-ish values.
fn spread(i: i32) -> [i32; 2] {
    [(i * 7919) % 1009, (i * 104_729) % 997]
}

/// Three values per dimension: every partition's common prefix runs into the
/// doc ID, where `dimCmpBytes` is negative (`BKDRadixSelector.java:410-428`).
fn duplicated(i: i32) -> [i32; 2] {
    [i % 3, (i * 7) % 3]
}

#[test]
fn a_points_merge_spills_to_and_cleans_the_segment_directory() {
    assert_spilled_merge_matches_heap_merge(spread);
}

#[test]
fn a_points_merge_of_duplicates_spills_and_matches_the_heap_merge() {
    assert_spilled_merge_matches_heap_merge(duplicated);
}

fn assert_spilled_merge_matches_heap_merge(value: fn(i32) -> [i32; 2]) {
    // 5000 merged points of 12 bytes each against a 0.01 MB budget, which
    // holds 873: `initPointWriter` spills, and so do the first partitions.
    let (events, spilled, left) = merge_points(0.01, value);
    let created: Vec<&String> = events.iter().filter(|e| e.starts_with("create ")).collect();
    assert!(
        created.iter().any(|e| e.contains("_bkd_spill_")),
        "the merge must spill into the segment directory: {events:?}"
    );
    for event in &created {
        let name = event.strip_prefix("create ").unwrap();
        assert!(
            events.contains(&format!("delete {name}")),
            "{name} was created by the merge and never deleted: {events:?}"
        );
    }
    assert!(
        !left.iter().any(|name| name.ends_with(".tmp")),
        "no temporary file is left in the directory: {left:?}"
    );

    // Lucene's default budget keeps the same merge in heap; the spill must not
    // change a byte of what it writes.
    let (heap_events, in_heap, _) = merge_points(16.0, value);
    assert!(
        !heap_events.iter().any(|e| e.contains("_bkd_")),
        "the default budget holds the merge in heap: {heap_events:?}"
    );
    assert_eq!(
        spilled.len(),
        3,
        "the merged segment has its three points files"
    );
    assert_eq!(spilled, in_heap);
}
