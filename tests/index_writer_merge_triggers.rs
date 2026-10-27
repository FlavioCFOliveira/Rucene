//! What triggers, keeps and orders merges in `IndexWriter`, compared with
//! Apache Lucene Core 10.5.0.
//!
//! Every case runs the same steps as `WriterMergeTriggersFixture` from
//! `tests/fixtures/java-codec-harness` (`java -cp lucene-core-10.5.0.jar
//! WriterMergeTriggersFixture.java <case>`) and compares what both print:
//!
//! * `retain`: `MergePolicy.keepFullyDeletedSegment` is handed the segment's
//!   reader (`ReadersAndUpdates.java:897-899`), so a
//!   `SoftDeletesRetentionMergePolicy` keeps a fully soft-deleted segment its
//!   retention query matches (`SoftDeletesRetentionMergePolicy.java:79-94`).
//! * `commit-merge` and `commit-merge-nowait`: a commit asks for merges
//!   through the `maybeMerge` flag, outside `commitLock`
//!   (`IndexWriter.java:3891-3893, 4245-4248`); the second case disables the
//!   merges prepared during the commit, so only that path merges.
//! * `commit-pit`: a commit prepares point-in-time merges and waits for them
//!   up to `maxFullFlushMergeWaitMillis` (`IndexWriter.java:3848-3889`).
//! * `blocks`: `MultiSorter` sorts a segment with document blocks by each
//!   block's parent (`MultiSorter.java:47-66`); Rucene force-merges the index
//!   Lucene wrote.
//! * `force-merge`: a forced merge cascades over the segments
//!   `segmentsToMerge` tracks, merged segments included
//!   (`IndexWriter.java:2196-2280, 3580-3586`).
//! * `log-packing` and `log-levels`: `LogMergePolicy.findMerges` packs a
//!   merge below the minimum merge size only up to that size, and quantizes
//!   levels in `float` (`LogMergePolicy.java:94-250`).
//! * `counts`: `flushCount` and `flushDeletesCount` move as Lucene's do
//!   (`IndexWriter.java:3003-3009, 4342-4344`).
//! * `deletes-applied`: delete RAM crossing the buffer makes the documents
//!   writer freeze the global deletes into a ticket and call
//!   `onDeletesApplied`, whose event publishes the ticket and increments
//!   `flushCount` (`IndexWriter.java:450-460`, `DocumentsWriter.java:168-185`).
//! * `flush-publish`: a segment that `addDocument` flushes is published at
//!   once by `afterSegmentsFlushed` (`IndexWriter.java:440-443`,
//!   `DocumentsWriter.java:579`) and may trigger a merge
//!   (`maybeProcessEvents`, `IndexWriter.java:5578-5583`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::Lucene104Codec;
use rucene::document::{Document, NumericDocValuesField, Store, StringField};
use rucene::index::index_reader::IndexReader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::{LogMergePolicy, NoMergePolicy, SoftDeletesRetentionMergePolicy};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::terms::Term;
use rucene::index::{directory_reader, SegmentInfos};
use rucene::search::{MatchAllDocsQuery, Query, Sort, SortField, SortFieldType, NO_MORE_DOCS};
use rucene::store::{Directory, FSDirectory, RamDirectory};
use rucene::util::BytesRef;

fn ensure_codec() {
    let _ = rucene::codecs::register_codec("Lucene104", Lucene104Codec::new());
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

/// Runs one case of the fixture and returns its standard output.
fn run_java(args: &[&str]) -> String {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         WriterMergeTriggersFixture.java",
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(fixture)
        .args(args)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "WriterMergeTriggersFixture {args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// `WriterMergeTriggersFixture.segments(Directory)`.
fn segments(dir: &Arc<dyn Directory>) -> String {
    let infos = SegmentInfos::read_latest_commit(Arc::clone(dir)).unwrap();
    infos
        .iter()
        .map(|info| format!("{}:{}", info.info.name, info.info.max_doc().unwrap()))
        .collect::<Vec<_>>()
        .join(",")
}

fn id_doc(i: i32) -> Document {
    let mut d = Document::new();
    d.add(Box::new(
        StringField::new("id", format!("id{i}"), Store::NO).unwrap(),
    ));
    d
}

#[test]
fn a_retention_policy_keeps_a_fully_soft_deleted_segment_it_still_matches() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config.set_soft_deletes_field(Some("soft".to_string()));
    config
        .set_merge_policy(Arc::new(SoftDeletesRetentionMergePolicy::new(
            "soft",
            Arc::new(|| Arc::new(MatchAllDocsQuery) as Arc<dyn Query>),
            Arc::new(NoMergePolicy::new()),
        )))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    let mut live = Document::new();
    live.add(Box::new(
        StringField::new("id", "live".to_string(), Store::NO).unwrap(),
    ));
    writer.add_document(&live).unwrap();
    writer.commit().unwrap();
    for id in ["t1", "t2"] {
        let mut tombstone = Document::new();
        tombstone.add(Box::new(
            StringField::new("id", id.to_string(), Store::NO).unwrap(),
        ));
        tombstone.add(Box::new(NumericDocValuesField::new("soft", 1)));
        writer.add_document(&tombstone).unwrap();
    }
    writer.commit().unwrap();
    writer.close().unwrap();

    let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    let rucene = format!(
        "retain segments={} maxDoc={} numDocs={}",
        segments(&dir),
        reader.max_doc(),
        reader.num_docs()
    );
    IndexReader::close(reader.as_ref()).unwrap();
    let lucene = run_java(&["retain"]);
    assert_eq!(
        rucene,
        lucene.trim(),
        "Lucene keeps the segment the retention query matches"
    );
}

/// Runs `count` single-document commits and renders each commit point.
fn commits(case: &str, config: IndexWriterConfig, count: i32) -> String {
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    let mut lines = Vec::new();
    for i in 0..count {
        writer.add_document(&id_doc(i)).unwrap();
        let seq_no = writer.commit().unwrap();
        lines.push(format!(
            "{case} commit {i} seqNo={seq_no} segments={}",
            segments(&dir)
        ));
    }
    writer.close().unwrap();
    lines.join("\n")
}

/// `WriterMergeTriggersFixture`'s `commit-merge` configuration.
fn commit_merge_config(max_full_flush_merge_wait_millis: Option<i64>) -> IndexWriterConfig {
    let mut config = IndexWriterConfig::new();
    let mut policy = LogMergePolicy::by_doc_count();
    policy.set_merge_factor(2).unwrap();
    config.set_merge_policy(Arc::new(policy)).unwrap();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    if let Some(millis) = max_full_flush_merge_wait_millis {
        config.set_max_full_flush_merge_wait_millis(millis);
    }
    config
}

#[test]
fn a_commit_asks_for_merges_as_lucene_does() {
    ensure_codec();
    let rucene = commits("commit-merge", commit_merge_config(None), 8);
    assert_eq!(rucene, run_java(&["commit-merge"]).trim());
}

#[test]
fn a_commit_asks_for_merges_once_it_is_done_as_lucene_does() {
    ensure_codec();
    let rucene = commits("commit-merge-nowait", commit_merge_config(Some(0)), 8);
    assert_eq!(rucene, run_java(&["commit-merge-nowait"]).trim());
}

#[test]
fn a_commit_waits_for_point_in_time_merges_as_lucene_does() {
    ensure_codec();
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    let rucene = commits("commit-pit", config, 12);
    assert_eq!(rucene, run_java(&["commit-pit"]).trim());
}

/// `WriterMergeTriggersFixture.blocksConfig()`.
fn blocks_config() -> IndexWriterConfig {
    let mut config = IndexWriterConfig::new();
    config.set_parent_field(Some("_parent".to_string()));
    config
        .set_index_sort(
            Sort::new_fields(vec![SortField::new(
                Some("rank".to_string()),
                SortFieldType::Long,
            )
            .unwrap()])
            .unwrap(),
        )
        .unwrap();
    config
}

#[test]
fn a_sorted_merge_keeps_document_blocks_together_as_lucene_does() {
    ensure_codec();
    let tmp = tempfile::tempdir().unwrap();
    let java_dir = tmp.path().join("lucene");
    let lucene = run_java(&["blocks", java_dir.to_str().unwrap()]);
    let expected_order = lucene
        .lines()
        .find_map(|line| line.strip_prefix("blocks order="))
        .expect("the fixture printed the merged order")
        .to_string();
    let expected_segments = lucene
        .lines()
        .find_map(|line| line.strip_prefix("blocks after segments="))
        .expect("the fixture printed the merged segments")
        .to_string();

    // Rucene force-merges the very index Lucene wrote, sorted with blocks.
    let rust_dir = tmp.path().join("rucene");
    std::fs::create_dir_all(&rust_dir).unwrap();
    for entry in std::fs::read_dir(&java_dir).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), rust_dir.join(entry.file_name())).unwrap();
    }
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(&rust_dir).unwrap());
    let mut config = blocks_config();
    config.set_open_mode(OpenMode::APPEND).unwrap();
    config
        .set_merge_policy(Arc::new(LogMergePolicy::by_doc_count()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.force_merge(1).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();

    let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    let mut order = Vec::new();
    for leaf in Arc::clone(&reader).leaves() {
        let mut blk = leaf
            .leaf_reader()
            .get_numeric_doc_values("blk")
            .unwrap()
            .unwrap();
        loop {
            let doc = blk.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                break;
            }
            order.push(blk.long_value().unwrap().to_string());
        }
    }
    IndexReader::close(reader.as_ref()).unwrap();
    assert_eq!(segments(&dir), expected_segments);
    assert_eq!(
        order.join(","),
        expected_order,
        "every block stays together, in Lucene's order"
    );
}

#[test]
fn a_forced_merge_cascades_as_lucene_does() {
    ensure_codec();
    let rucene = forced_merge_cascade(commit_merge_config(None));
    assert_eq!(rucene, run_java(&["force-merge"]).trim());
}

/// The cascade under the default `ConcurrentMergeScheduler`, whose merge
/// threads schedule the next round when they finish
/// (`ConcurrentMergeScheduler.runOnMergeFinished`, `ConcurrentMergeScheduler.java:685-707`);
/// without that, `forceMerge` waits forever for a round nobody runs.
#[test]
fn a_forced_merge_cascades_under_the_concurrent_scheduler_as_lucene_does() {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        ensure_codec();
        let mut config = IndexWriterConfig::new();
        let mut policy = LogMergePolicy::by_doc_count();
        policy.set_merge_factor(2).unwrap();
        config.set_merge_policy(Arc::new(policy)).unwrap();
        let _ = tx.send(forced_merge_cascade(config));
    });
    let rucene = rx
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("forceMerge hung: the cascading round was never scheduled");
    assert_eq!(rucene, run_java(&["force-merge"]).trim());
}

/// `WriterMergeTriggersFixture.forceMerge` with the given writer configuration
/// for the forced merge.
fn forced_merge_cascade(config: IndexWriterConfig) -> String {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut initial = IndexWriterConfig::new();
    initial
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), initial).unwrap();
    for i in 0..4 {
        writer.add_document(&id_doc(i)).unwrap();
        writer.flush().unwrap();
    }
    writer.commit().unwrap();
    writer.close().unwrap();
    // Two merges of two segments, then the cascade over their results.
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.force_merge(1).unwrap();
    // Before the commit, whose own merges would hide a missing cascade.
    let segment_count = writer.get_segment_count().unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();
    format!(
        "force-merge segmentCount={segment_count}\nforce-merge segments={}",
        segments(&dir)
    )
}

#[test]
fn flush_counters_move_as_lucene_does() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    let counts = |writer: &IndexWriter| {
        format!(
            "flushCount={} flushDeletesCount={}",
            writer.get_flush_count(),
            writer.get_flush_deletes_count()
        )
    };
    let mut lines = Vec::new();
    writer.add_document(&id_doc(0)).unwrap();
    writer.add_document(&id_doc(1)).unwrap();
    writer.flush().unwrap();
    lines.push(format!("counts after flush {}", counts(&writer)));
    writer.flush().unwrap();
    lines.push(format!("counts after empty flush {}", counts(&writer)));
    writer
        .delete_documents_by_terms(vec![Term::new("id", BytesRef::new(b"id0".to_vec()))])
        .unwrap();
    writer.flush().unwrap();
    lines.push(format!("counts after delete flush {}", counts(&writer)));
    writer.add_document(&id_doc(2)).unwrap();
    writer.commit().unwrap();
    lines.push(format!("counts after commit {}", counts(&writer)));
    writer.close().unwrap();
    assert_eq!(lines.join("\n"), run_java(&["counts"]).trim());
}

/// `deletes-applied`: with a 0.01 MB RAM buffer, deletes until `flushCount`
/// moves past its value after the first commit, and both counts. How many
/// deletes that takes depends on the RAM the buffered deletes account for,
/// which is not compared here.
#[test]
fn applied_deletes_publish_and_count_a_flush_as_lucene_does() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config.set_ram_buffer_size_mb(0.01).unwrap();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    let counts = |writer: &IndexWriter| {
        format!(
            "flushCount={} flushDeletesCount={}",
            writer.get_flush_count(),
            writer.get_flush_deletes_count()
        )
    };
    writer.add_document(&id_doc(0)).unwrap();
    writer.commit().unwrap();
    let base = writer.get_flush_count();
    let mut lines = vec![format!("before {}", counts(&writer))];
    let mut deletes = 0;
    while writer.get_flush_count() == base && deletes < 100_000 {
        writer
            .delete_documents_by_terms(vec![Term::new(
                "id",
                BytesRef::new(format!("missing{deletes}").into_bytes()),
            )])
            .unwrap();
        deletes += 1;
    }
    lines.push(format!("applied={} {}", deletes < 100_000, counts(&writer)));
    writer.close().unwrap();
    assert_eq!(lines.join("\n"), run_java(&["deletes-applied"]).trim());
}

/// `flush-publish`: after each of ten `addDocument` calls with
/// `maxBufferedDocs=2`, the writer's segment count, without and with a merge
/// policy that merges what the flushes publish.
#[test]
fn segments_flushed_by_add_document_are_published_and_merged_as_lucene_does() {
    ensure_codec();
    let mut lines = Vec::new();
    for case in ["none", "logdoc2"] {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let mut config = IndexWriterConfig::new();
        config.set_max_buffered_docs(2).unwrap();
        config
            .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
            .unwrap();
        if case == "none" {
            config
                .set_merge_policy(Arc::new(NoMergePolicy::new()))
                .unwrap();
        } else {
            let mut policy = LogMergePolicy::by_doc_count();
            policy.set_merge_factor(2).unwrap();
            policy.set_min_merge_docs(1);
            config.set_merge_policy(Arc::new(policy)).unwrap();
        }
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        let mut counts = Vec::new();
        for i in 0..10 {
            writer.add_document(&id_doc(i)).unwrap();
            counts.push(writer.get_segment_count().unwrap().to_string());
        }
        writer.commit().unwrap();
        lines.push(format!(
            "flush-publish {case} counts={} segments={}",
            counts.join(" "),
            segments(&dir)
        ));
        writer.close().unwrap();
    }
    assert_eq!(lines.join("\n"), run_java(&["flush-publish"]).trim());
}

/// `WriterMergeTriggersFixture.logMerge`: segments of `sizes` documents written
/// without merging, then one `maybeMerge` with a `LogDocMergePolicy`.
fn log_merge(case: &str, merge_factor: i32, sizes: &[i32]) -> String {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut initial = IndexWriterConfig::new();
    initial
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), initial).unwrap();
    let mut id = 0;
    for &size in sizes {
        for _ in 0..size {
            writer.add_document(&id_doc(id)).unwrap();
            id += 1;
        }
        writer.flush().unwrap();
    }
    writer.commit().unwrap();
    writer.close().unwrap();
    let mut config = IndexWriterConfig::new();
    let mut policy = LogMergePolicy::by_doc_count();
    policy.set_merge_factor(merge_factor).unwrap();
    config.set_merge_policy(Arc::new(policy)).unwrap();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.maybe_merge().unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();
    format!("{case} segments={}", segments(&dir))
}

#[test]
fn a_log_merge_packs_only_up_to_the_minimum_merge_size_as_lucene_does() {
    let rucene = log_merge("log-packing", 2, &[400, 400, 400, 400]);
    assert_eq!(rucene, run_java(&["log-packing"]).trim());
}

#[test]
fn log_merge_levels_are_quantized_in_float_as_lucene_does() {
    let rucene = log_merge("log-levels", 4, &[24, 3, 3, 3]);
    assert_eq!(rucene, run_java(&["log-levels"]).trim());
}

/// Each point-in-time merge on getReader keeps its own merged segment
/// (`origInfo` is a field of each anonymous `OneMerge`, `IndexWriter.java:3944`).
/// Several such merges run at once under the `ConcurrentMergeScheduler`; were
/// they to share one slot, the reopened reader would list one merged segment
/// several times and miss the others.
#[test]
fn concurrent_merges_on_get_reader_each_keep_their_own_segment() {
    ensure_codec();
    for _ in 0..20 {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let mut initial = IndexWriterConfig::new();
        initial
            .set_merge_policy(Arc::new(NoMergePolicy::new()))
            .unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), initial).unwrap();
        for i in 0..40 {
            writer.add_document(&id_doc(i)).unwrap();
            writer.flush().unwrap();
        }
        writer.commit().unwrap();
        writer.close().unwrap();

        // The default `TieredMergePolicy` merges at most ten segments at once,
        // so the full-flush merges over 41 tiny segments are several.
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        writer.add_document(&id_doc(40)).unwrap();
        let reader = directory_reader::open_with_writer(writer.as_ref()).unwrap();
        assert_eq!(reader.max_doc(), 41);
        // Every document is in the reader exactly once.
        let searcher = rucene::search::index_searcher::IndexSearcher::new(
            Arc::clone(&reader) as Arc<dyn IndexReader>
        )
        .unwrap();
        for i in 0..41 {
            let hits = searcher
                .count(Arc::new(rucene::search::term_query::TermQuery::new(
                    Term::new("id", BytesRef::new(format!("id{i}").into_bytes())),
                )))
                .unwrap();
            assert_eq!(hits, 1, "document id{i} is in the reader {hits} times");
        }
        IndexReader::close(reader.as_ref()).unwrap();
        writer.close().unwrap();
    }
}
