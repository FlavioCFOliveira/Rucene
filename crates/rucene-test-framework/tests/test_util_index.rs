//! The index helpers of `org.apache.lucene.tests.util.TestUtil` (rmp #342):
//! `checkIndex`, `alwaysPostingsFormat`, `getPostingsFormatWithOrds`,
//! `ramCopyOf`, `anyFilesExceptWriteLock` and `docs`, over an index of
//! `LineFileDocs` documents; `BaseDirectoryWrapper` and `CloseableDirectory`.
//!
//! The codecs are found by name through `rucene_test_framework::class_loader()`,
//! the services of `lucene-test-framework` (which register `Asserting` and
//! `LuceneFixedGap`), as Java's class path resolves them.
//!
//! The indexes are written to a `ByteBuffersDirectory`, which fails a write
//! after close: the `AssertingCodec` flush closes every core consumer twice
//! (`AssertingNormsConsumer.close`, `AssertingNormsFormat.java:78-79`), so it
//! also proves those closes idempotent, as Java's are.

mod oracle_support;

use std::sync::{Arc, Once};

use randomizedtesting::jdk::random::JdkRandom;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::IndexReader;
use rucene::store::{ByteBuffersDirectory, Directory, DEFAULT_IO_CONTEXT};
use rucene::util::BytesRef;
use rucene_test_framework::store::BaseDirectoryWrapper;
use rucene_test_framework::util::{
    CloseableDirectory, LineFileDocs, TestRuleMarkFailure, TestUtil,
};

fn install_codecs() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let loader = rucene_test_framework::class_loader();
        rucene::codecs::reload_codecs(loader).unwrap();
        rucene::codecs::postings::reload_postings_formats(loader).unwrap();
        rucene::codecs::knn_vectors::reload_knn_vectors_format(loader).unwrap();
        rucene::codecs::doc_values::reload_doc_values_formats(loader).unwrap();
    });
}

/// An index of 60 `LineFileDocs` documents in two commits, written with
/// `alwaysPostingsFormat(getPostingsFormatWithOrds(random))`.
fn line_docs_index(seed: i64) -> (Arc<dyn Directory>, String) {
    install_codecs();
    let random = JdkRandom::new(seed);
    let format = TestUtil::get_postings_format_with_ords(&random);
    let format_name = format.name().to_string();
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let config = IndexWriterConfig::new();
    config
        .set_codec(TestUtil::always_postings_format(format))
        .unwrap();
    config.set_open_mode(OpenMode::CREATE).unwrap();
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
        .unwrap();
    let w = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    let docs = LineFileDocs::new(&random, "europarl.lines.txt.gz").unwrap();
    for i in 0..60 {
        w.add_document(&*docs.next_doc().unwrap().borrow()).unwrap();
        if i == 29 {
            w.commit().unwrap();
        }
    }
    w.close().unwrap();
    (dir, format_name)
}

/// `checkIndex` passes on a clean index written with either postings format
/// with ords (both seeds' draws), records its messages, and fails with
/// `CheckIndex failed` once the largest file is corrupted.
///
/// The level is `MIN_LEVEL_FOR_SLOW_CHECKS` (3), `checkIndex(dir)`'s: its
/// term-vector cross-check reads the `AssertingCodec`'s term-vector impacts
/// (`SlowImpactsEnum`, which reports one impact without reading the
/// postings) and advances their postings past the only document
/// (`TVPostingsEnum.advance`, `slowAdvance`), both as Java does.
#[test]
fn check_index_on_clean_and_corrupt_index() {
    let mut formats = Vec::new();
    // Seeds of `OracleSupport.seeds()`; their first `nextInt(2)` is 1, 0, 1, 0.
    for seed in [42, 0x5DEE_CE66D, 0x0123_4567_89AB_CDEF, -1] {
        let (dir, format) = line_docs_index(seed);
        formats.push(format);
        let mut output = Vec::new();
        let status =
            TestUtil::check_index_with(Arc::clone(&dir), 3, false, false, Some(&mut output))
                .unwrap();
        assert!(status.clean);
        assert!(!output.is_empty());
        assert!(TestUtil::any_files_except_write_lock(&*dir).unwrap());

        // ramCopyOf copies every segments and codec file (not write.lock).
        let ram = TestUtil::ram_copy_of(&*dir).unwrap();
        let mut index_files = dir.list_all().unwrap();
        index_files.retain(|n| n != "write.lock");
        assert_eq!(ram.list_all().unwrap(), index_files);
        for name in index_files {
            assert_eq!(
                ram.file_length(&name).unwrap(),
                dir.file_length(&name).unwrap()
            );
        }

        // Corrupt one byte in the middle of the largest file.
        let name = dir
            .list_all()
            .unwrap()
            .into_iter()
            .max_by_key(|n| dir.file_length(n).unwrap())
            .unwrap();
        let len = dir.file_length(&name).unwrap() as usize;
        let mut bytes = vec![0u8; len];
        {
            let mut input = dir.open_input(&name, &*DEFAULT_IO_CONTEXT).unwrap();
            input.read_bytes(Some(&mut bytes), 0, len).unwrap();
        }
        bytes[len / 2] ^= 0x5a;
        dir.delete_file(&name).unwrap();
        {
            let mut out = dir.create_output(&name, &*DEFAULT_IO_CONTEXT).unwrap();
            out.write_bytes(&bytes, 0, len).unwrap();
            out.close().unwrap();
        }
        let err = TestUtil::check_index_with(Arc::clone(&dir), 3, false, false, None).unwrap_err();
        assert_eq!(
            err.to_string(),
            "java.lang.RuntimeException: CheckIndex failed"
        );
    }
    // The seeds draw both formats.
    formats.sort();
    formats.dedup();
    assert_eq!(formats, ["BlockTreeOrds", "LuceneFixedGap"]);
}

/// `docs` finds a term of the index and returns `None` for a missing field
/// or term.
#[test]
fn docs_finds_terms() {
    let (dir, _) = line_docs_index(5);
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir).unwrap();
    let random = JdkRandom::new(7);
    let id = BytesRef::from_char_sequence("7");
    let found = TestUtil::docs(&random, &reader, "docid", &id, None, 0).unwrap();
    let mut postings = found.unwrap();
    assert_eq!(postings.next_doc().unwrap(), 7);
    assert!(TestUtil::docs(&random, &reader, "nofield", &id, None, 0)
        .unwrap()
        .is_none());
    let missing = BytesRef::from_char_sequence("no such id");
    assert!(TestUtil::docs(&random, &reader, "docid", &missing, None, 0)
        .unwrap()
        .is_none());
}

/// `BaseDirectoryWrapper` is open until closed, closes its delegate, and
/// `CloseableDirectory` fails the test for a directory left open, unless an
/// earlier failure was marked.
#[test]
fn base_directory_wrapper_and_closeable_directory() {
    let wrapper = Arc::new(BaseDirectoryWrapper::new(Arc::new(
        ByteBuffersDirectory::new(),
    )));
    assert!(wrapper.is_open());
    assert!(wrapper.get_check_index_on_close());
    assert_eq!(wrapper.get_level_for_check_on_close(), 3);
    wrapper.set_cross_check_term_vectors_on_close(false);
    assert_eq!(wrapper.get_level_for_check_on_close(), 2);
    wrapper.set_cross_check_term_vectors_on_close(true);
    assert_eq!(wrapper.get_level_for_check_on_close(), 3);

    let marker = Arc::new(TestRuleMarkFailure::new(Vec::new()));
    let closeable = CloseableDirectory::new(Arc::clone(&wrapper), Arc::clone(&marker));
    let failure =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| closeable.close())).unwrap_err();
    let (class, message) = oracle_support::panic_error(failure);
    assert_eq!(class, "java.lang.AssertionError");
    assert!(
        message.contains("Directory not closed: BaseDirectoryWrapper(ByteBuffersDirectory"),
        "{message}"
    );
    drop(closeable);

    // An empty directory holds no index: close does not check, and closes.
    wrapper.close().unwrap();
    assert!(!wrapper.is_open());
    CloseableDirectory::new(Arc::clone(&wrapper), marker).close();
}

// -----------------------------------------------------------------------------
// The other members of `TestUtil` (rmp #342), against
// `testutil_members.tsv` (`TestUtilOracle.members`).
// -----------------------------------------------------------------------------

use randomizedtesting::jdk::lang::double_to_string;
use rucene::codecs::Codec;
use rucene::document::{
    BinaryDocValuesField, BinaryPoint, Document, IntField, IntPoint, KeywordField, LongField,
    NumericDocValuesField, NumericValue, SortedDocValuesField, SortedSetDocValuesField, Store,
    StoredField, StringField, TextField,
};
use rucene::index::index_writer_config::IndexWriterConfig as Iwc;
use rucene::index::merge_policy::{
    FilterMergePolicy, LogMergePolicy, MergePolicy, TieredMergePolicy,
};
use rucene::index::merge_scheduler::{ConcurrentMergeScheduler, MergeScheduler};
use rucene::index::{IndexableField, Term};
use rucene::search::{
    FieldDoc, ScoreDoc, SortValue, TopDocs, TopFieldDocs, TotalHits, TotalHitsRelation,
};
use rucene_test_framework::codecs::asserting::AssertingCodec;
use rucene_test_framework::codecs::blockterms::LuceneFixedGap;
use rucene_test_framework::codecs::cheapbastard::CheapBastardCodec;

/// The rows of `testutil_members.tsv` for `method`, as `(case, result)`
/// (an error is `class<TAB>message`, its line breaks escaped as `\n`).
fn member_rows(method: &str) -> Vec<(String, String)> {
    oracle_support::rows("testutil_members.tsv")
        .into_iter()
        .filter(|r| r[0] == method)
        .map(|r| (r[1].clone(), r[2..].join("\t")))
        .collect()
}

/// `TestUtilOracle.describe(MergePolicy)`.
fn describe_policy(mp: &dyn MergePolicy, filter_inner: Option<&TieredMergePolicy>) -> String {
    let mut d = format!("noCFSRatio={}", double_to_string(mp.get_no_cfs_ratio()));
    if let Some(any) = mp.as_any() {
        if let Some(l) = any.downcast_ref::<LogMergePolicy>() {
            d += &format!(" mergeFactor={}", l.get_merge_factor());
        } else if let Some(t) = any.downcast_ref::<TieredMergePolicy>() {
            d += &format!(
                " maxMergeAtOnce={} segmentsPerTier={}",
                t.get_max_merge_at_once(),
                double_to_string(t.get_segments_per_tier())
            );
        }
    }
    if let Some(inner) = filter_inner {
        d += &format!(" delegate({})", describe_policy(inner, None));
    }
    d
}

/// `TestUtilOracle.describe(MergeScheduler)`.
fn describe_scheduler(ms: &dyn MergeScheduler) -> String {
    match ms
        .as_any()
        .and_then(|a| a.downcast_ref::<ConcurrentMergeScheduler>())
    {
        Some(c) => format!(
            "maxMergeCount={} maxThreadCount={}",
            c.get_max_merge_count(),
            c.get_max_thread_count()
        ),
        None => "SerialMergeScheduler".to_string(),
    }
}

/// `reduceOpenFiles` changes the live policy and scheduler of a writer as
/// Java's does, for every policy and scheduler of the oracle; the
/// `FilterMergePolicy` only takes the `noCFSRatio` (its `instanceof` tests
/// fail), and `syncConcurrentMerges` returns.
///
/// The auto-detected counts of a default `ConcurrentMergeScheduler` before
/// the call depend on the machine's cores and are not compared.
#[test]
fn reduce_open_files_is_lucenes() {
    let rows = member_rows("reduceOpenFiles");
    assert_eq!(rows.len(), 18);
    for (case, expected) in rows {
        let (policy, scheduler) = case.split_once(' ').unwrap();
        let mut filter_inner: Option<Arc<TieredMergePolicy>> = None;
        let mp: Arc<dyn MergePolicy> = match policy {
            "tiered" => Arc::new(TieredMergePolicy::new()),
            "tiered(3,4.0)" | "tiered(20,30.0)" => {
                let (a, s) = if policy == "tiered(3,4.0)" {
                    (3, 4.0)
                } else {
                    (20, 30.0)
                };
                let t = TieredMergePolicy::new();
                t.set_max_merge_at_once(a).unwrap();
                t.set_segments_per_tier(s).unwrap();
                Arc::new(t)
            }
            "logdoc" => Arc::new(LogMergePolicy::by_doc_count()),
            "logbytes(3)" => {
                let l = LogMergePolicy::by_byte_size();
                l.set_merge_factor(3).unwrap();
                Arc::new(l)
            }
            "filter(tiered)" => {
                let inner = Arc::new(TieredMergePolicy::new());
                filter_inner = Some(Arc::clone(&inner));
                Arc::new(FilterMergePolicy::new(inner))
            }
            other => panic!("{other}"),
        };
        let ms: Arc<dyn MergeScheduler> = match scheduler {
            "cms" => Arc::new(ConcurrentMergeScheduler::new()),
            "cms(6,4)" => {
                let c = ConcurrentMergeScheduler::new();
                c.set_max_merges_and_threads(6, 4).unwrap();
                Arc::new(c)
            }
            "serial" => Arc::new(SerialMergeScheduler::new()),
            other => panic!("{other}"),
        };
        let c = Iwc::new();
        c.set_merge_policy(Some(mp)).unwrap();
        c.set_merge_scheduler(Some(ms)).unwrap();
        let w = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), c).unwrap();
        let describe = |w: &IndexWriter| {
            format!(
                "{} {}",
                describe_policy(&*w.get_config().merge_policy(), filter_inner.as_deref()),
                describe_scheduler(&*w.get_config().merge_scheduler())
            )
        };
        let before = describe(&w);
        TestUtil::reduce_open_files(&w).unwrap();
        TestUtil::sync_concurrent_merges_of_writer(&w);
        let after = describe(&w);
        let (expected_before, expected_after) = expected.split_once(" -> ").unwrap();
        assert_eq!(after, expected_after, "{case}");
        if scheduler == "cms" {
            let cut = |s: &str| s[..s.find(" maxMergeCount=").unwrap()].to_string();
            assert_eq!(cut(&before), cut(expected_before), "{case}");
        } else {
            assert_eq!(before, expected_before, "{case}");
        }
        w.close().unwrap();
    }
}

/// `getPostingsFormat` and `getDocValuesFormat` name the format of a field
/// through a codec's per-field format, or the codec's own format, as Java's
/// do; the one-argument forms use the default codec.
#[test]
fn get_formats_are_lucenes() {
    install_codecs();
    let codecs: Vec<(&str, Arc<dyn Codec>)> = vec![
        ("lucene104", TestUtil::get_default_codec()),
        ("asserting", Arc::new(AssertingCodec::new())),
        (
            "alwaysFixedGap",
            TestUtil::always_postings_format(Arc::new(LuceneFixedGap::new())),
        ),
        (
            "alwaysLucene90",
            TestUtil::always_doc_values_format(TestUtil::get_default_doc_values_format()),
        ),
        (
            "simpletext",
            Arc::new(rucene_codecs::simpletext::codec::SimpleTextCodec::new()),
        ),
        ("cheapbastard", Arc::new(CheapBastardCodec::new())),
    ];
    let mut seen = 0;
    for (case, expected) in member_rows("getPostingsFormat")
        .into_iter()
        .chain(member_rows("getDocValuesFormat"))
    {
        let (codec, field) = case.split_once(' ').unwrap();
        let postings = oracle_support::rows("testutil_members.tsv")
            .iter()
            .any(|r| r[0] == "getPostingsFormat" && r[1] == case && r[2] == expected);
        let actual = if codec == "default" {
            if postings {
                TestUtil::get_postings_format(field)
            } else {
                TestUtil::get_doc_values_format(field)
            }
        } else {
            let codec = &codecs.iter().find(|(n, _)| *n == codec).unwrap().1;
            if postings {
                TestUtil::get_postings_format_of(&**codec, field)
            } else {
                TestUtil::get_doc_values_format_of(&**codec, field)
            }
        };
        assert_eq!(actual, expected, "{case}");
        seen += 1;
    }
    assert_eq!(seen, 26);
    let rows = member_rows("fieldSupportsHugeBinaryDocValues");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        TestUtil::field_supports_huge_binary_doc_values(&rows[0].0).to_string(),
        rows[0].1
    );
}

/// `cloneDocument` rebuilds every field kind of the oracle's document with
/// the field Java builds (the clone's fields print as Java's `toString()`),
/// and fails as Java does for sorted-numeric and sorted-set doc values.
#[test]
fn clone_document_is_lucenes() {
    let mut doc = Document::new();
    doc.add(Box::new(
        KeywordField::new("kw", "v1".into(), Store::YES).unwrap(),
    ));
    doc.add(Box::new(
        KeywordField::new("kw2", "v2".into(), Store::NO).unwrap(),
    ));
    doc.add(Box::new(IntField::new("int", 7, Store::YES)));
    doc.add(Box::new(IntField::new("int2", -3, Store::NO)));
    doc.add(Box::new(NumericDocValuesField::new("ndv", 42)));
    doc.add(Box::new(BinaryDocValuesField::new(
        "bdv",
        BytesRef::from_char_sequence("bin"),
    )));
    doc.add(Box::new(SortedDocValuesField::new(
        "sdv",
        BytesRef::from_char_sequence("sorted"),
    )));
    doc.add(Box::new(
        BinaryPoint::new(
            "bp",
            &[BytesRef::new(vec![1, 2]), BytesRef::new(vec![3, 4])],
        )
        .unwrap(),
    ));
    doc.add(Box::new(IntPoint::new("ip", &[5, 6]).unwrap()));
    doc.add(Box::new(
        StringField::new("s", "str".into(), Store::YES).unwrap(),
    ));
    doc.add(Box::new(
        TextField::new("t", "some text".into(), Store::NO).unwrap(),
    ));
    doc.add(Box::new(
        StoredField::new_string("st", "stored".to_string()).unwrap(),
    ));
    let clone = TestUtil::clone_document(&doc).unwrap();
    let rows = member_rows("cloneDocument");
    assert_eq!(rows.len(), 15);
    let actual: Vec<(String, String)> = clone
        .get_fields()
        .iter()
        .map(|f| (f.name().to_string(), f.to_string()))
        .collect();
    assert_eq!(actual, rows[..12].to_vec());

    let bad: Vec<Box<dyn IndexableField>> = vec![
        Box::new(LongField::new("lf", 5, Store::NO)),
        Box::new(SortedSetDocValuesField::new(
            "ssdv",
            BytesRef::from_char_sequence("x"),
        )),
        Box::new(StoredField::new_number("sti", NumericValue::Int(5)).unwrap()),
    ];
    for (field, (name, expected)) in bad.into_iter().zip(&rows[12..]) {
        assert_eq!(field.name(), name);
        let mut one = Document::new();
        one.add(field);
        let actual = match TestUtil::clone_document(&one) {
            Ok(d) => d.get_fields()[0].to_string(),
            // The port's `IllegalStateException`.
            Err(rucene::error::LuceneError {
                kind: rucene::error::LuceneErrorKind::IllegalState { message, .. },
                ..
            }) => {
                format!("java.lang.IllegalStateException\t{message}")
            }
            Err(e) => panic!("{name}: {e}"),
        };
        assert_eq!(&actual, expected, "{name}");
    }
}

/// `addIndexesSlowly` adds every leaf of the readers through
/// `SlowCodecReaderWrapper`: one merged segment of the live documents.
#[test]
fn add_indexes_slowly_is_lucenes() {
    let sources: Vec<Arc<dyn Directory>> = (0..2)
        .map(|s| {
            let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
            let w = IndexWriter::new(Arc::clone(&dir), Iwc::new()).unwrap();
            for i in 0..(if s == 0 { 3 } else { 2 }) {
                let mut d = Document::new();
                d.add(Box::new(
                    StringField::new("id", i.to_string(), Store::YES).unwrap(),
                ));
                w.add_document(&d).unwrap();
            }
            if s == 1 {
                w.delete_documents_by_terms(vec![Term::new(
                    "id",
                    BytesRef::from_char_sequence("0"),
                )])
                .unwrap();
            }
            w.close().unwrap();
            dir
        })
        .collect();
    let readers: Vec<_> = sources
        .iter()
        .map(|d| directory_reader::open(Arc::clone(d)).unwrap())
        .collect();
    let target: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let c = Iwc::new();
    c.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
        .unwrap();
    let w = IndexWriter::new(Arc::clone(&target), c).unwrap();
    TestUtil::add_indexes_slowly(&w, &readers).unwrap();
    w.commit().unwrap();
    let r: Arc<dyn IndexReader> = directory_reader::open(Arc::clone(&target)).unwrap();
    let actual = format!(
        "segments={} maxDoc={} numDocs={}",
        Arc::clone(&r).leaves().unwrap().len(),
        r.max_doc(),
        r.num_docs()
    );
    let rows = member_rows("addIndexesSlowly");
    assert_eq!(rows, vec![("a(3)+b(2,-1)".to_string(), actual)]);
    w.close().unwrap();
}

fn sd(doc: i32, score: f32) -> ScoreDoc {
    ScoreDoc::new(doc, score)
}

fn fd(doc: i32, score: f32, fields: Option<Vec<SortValue>>) -> FieldDoc {
    let mut f = FieldDoc::new(doc, score);
    f.fields = fields;
    f
}

fn hits(n: i64, equal: bool) -> TotalHits {
    let relation = if equal {
        TotalHitsRelation::EQUAL_TO
    } else {
        TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO
    };
    TotalHits::new(n, relation).unwrap()
}

fn td(n: i64, equal: bool, docs: Vec<ScoreDoc>) -> TopDocs {
    TopDocs {
        total_hits: hits(n, equal),
        score_docs: docs,
    }
}

fn tfd(n: i64, equal: bool, docs: Vec<FieldDoc>) -> TopFieldDocs {
    TopFieldDocs::new(hits(n, equal), docs, Vec::new())
}

/// `ok`, or the `class<TAB>message` of the `assertConsistent` failure, its
/// line breaks escaped as the fixture holds them.
fn consistent_outcome(f: impl FnOnce()) -> String {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => "ok".to_string(),
        Err(payload) => {
            let (class, message) = oracle_support::panic_error(payload);
            format!("{class}\t{}", message.replace('\n', "\\n"))
        }
    }
}

/// `assertConsistent` passes and fails on every case of the oracle with
/// Java's message: the hit counts by relation (Hamcrest's `equalTo`,
/// `lessThanOrEqualTo`, `greaterThanOrEqualTo`), the hit count
/// (`arrayWithSize`), docs, scores (JUnit's `double` comparison), sort values
/// (JUnit's `assertArrayEquals`) and the hit classes (`instanceOf`).
#[test]
fn assert_consistent_is_lucenes() {
    use SortValue::{Bytes, Float, Int, Long, Null};
    let b = |s: &str| Bytes(BytesRef::from_char_sequence(s));
    let three = || vec![sd(1, 2.0), sd(2, 1.0), sd(3, 0.5)];
    let one = |f: Option<Vec<SortValue>>| vec![fd(1, 1.0, f)];
    let mut outcomes: Vec<(&str, String)> = Vec::new();
    let mut run = |label: &'static str, f: Box<dyn FnOnce()>| {
        outcomes.push((label, consistent_outcome(f)));
    };
    run(
        "same",
        Box::new(move || TestUtil::assert_consistent(&td(3, true, three()), &td(3, true, three()))),
    );
    run(
        "zero-vs-two",
        Box::new(|| {
            TestUtil::assert_consistent(
                &td(0, true, vec![]),
                &td(2, true, vec![sd(1, 1.0), sd(2, 1.0)]),
            )
        }),
    );
    run(
        "two-vs-zero",
        Box::new(|| {
            TestUtil::assert_consistent(
                &td(2, false, vec![sd(1, 1.0), sd(2, 1.0)]),
                &td(0, true, vec![]),
            )
        }),
    );
    for (label, (en, ee), (an, ae)) in [
        ("eq5-vs-eq4", (5, true), (4, true)),
        ("eq5-vs-gte6", (5, true), (6, false)),
        ("eq5-vs-gte3", (5, true), (3, false)),
        ("gte5-vs-eq4", (5, false), (4, true)),
        ("gte5-vs-eq7", (5, false), (7, true)),
        ("gte5-vs-gte1", (5, false), (1, false)),
    ] {
        run(
            label,
            Box::new(move || {
                TestUtil::assert_consistent(
                    &td(en, ee, vec![sd(1, 1.0)]),
                    &td(an, ae, vec![sd(1, 1.0)]),
                )
            }),
        );
    }
    run(
        "size",
        Box::new(move || {
            TestUtil::assert_consistent(
                &td(3, true, three()),
                &td(3, true, vec![sd(1, 2.0), sd(2, 1.0)]),
            )
        }),
    );
    run(
        "doc",
        Box::new(move || {
            TestUtil::assert_consistent(
                &td(3, true, three()),
                &td(3, true, vec![sd(1, 2.0), sd(4, 1.0), sd(3, 0.5)]),
            )
        }),
    );
    run(
        "score",
        Box::new(|| {
            TestUtil::assert_consistent(
                &td(1, true, vec![sd(1, 1.5)]),
                &td(1, true, vec![sd(1, 0.1)]),
            )
        }),
    );
    run(
        "score-nan",
        Box::new(|| {
            TestUtil::assert_consistent(
                &td(1, true, vec![sd(1, f32::NAN)]),
                &td(1, true, vec![sd(1, f32::NAN)]),
            )
        }),
    );
    run(
        "score-zero",
        Box::new(|| {
            TestUtil::assert_consistent(
                &td(1, true, vec![sd(1, 0.0)]),
                &td(1, true, vec![sd(1, -0.0)]),
            )
        }),
    );
    type Fields = Option<Vec<SortValue>>;
    let field_cases: Vec<(&'static str, Fields, Fields)> = vec![
        (
            "fields-same",
            Some(vec![Int(3), b("ab")]),
            Some(vec![Int(3), b("ab")]),
        ),
        ("fields-int", Some(vec![Int(3)]), Some(vec![Int(4)])),
        (
            "fields-bytes",
            Some(vec![Int(1), b("ab")]),
            Some(vec![Int(1), b("ac")]),
        ),
        (
            "fields-float",
            Some(vec![Float(1.0)]),
            Some(vec![Float(2.0)]),
        ),
        ("fields-long-int", Some(vec![Long(5)]), Some(vec![Int(5)])),
        ("fields-null-value", Some(vec![Null]), Some(vec![Int(5)])),
        (
            "fields-longer",
            Some(vec![Int(3)]),
            Some(vec![Int(3), Float(1.0)]),
        ),
        (
            "fields-shorter",
            Some(vec![Int(3), Float(1.0)]),
            Some(vec![Int(3)]),
        ),
        ("fields-null", None, Some(vec![Int(3)])),
        ("fields-null-actual", Some(vec![Int(3)]), None),
        ("fields-both-null", None, None),
    ];
    for (label, e, a) in field_cases {
        run(
            label,
            Box::new(move || {
                TestUtil::assert_consistent(&tfd(1, true, one(e)), &tfd(1, true, one(a)))
            }),
        );
    }
    run(
        "fielddoc-vs-scoredoc",
        Box::new(move || {
            TestUtil::assert_consistent(
                &tfd(1, true, one(Some(vec![Int(3)]))),
                &td(1, true, vec![sd(1, 1.0)]),
            )
        }),
    );
    run(
        "scoredoc-vs-fielddoc",
        Box::new(move || {
            TestUtil::assert_consistent(
                &td(1, true, vec![sd(1, 1.0)]),
                &tfd(1, true, one(Some(vec![Int(3), Float(2.5)]))),
            )
        }),
    );
    let expected = member_rows("assertConsistent");
    assert_eq!(expected.len(), outcomes.len());
    for ((label, outcome), (case, result)) in outcomes.iter().zip(&expected) {
        assert_eq!(label, case);
        assert_eq!(outcome, result, "{label}");
    }
}

// -----------------------------------------------------------------------------
// checkReader, the mock-filesystem probes, shutdownExecutorService and
// assertAttributeReflection.
// -----------------------------------------------------------------------------

use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::{
    BinaryDocValues, ByteVectorValues, CacheHelper, DocValuesSkipper, FieldInfos,
    FloatVectorValues, IndexReaderCore, LeafMetaData, LeafReader, NumericDocValues, PointValues,
    SortedDocValues, SortedNumericDocValues, SortedSetDocValues, StoredFields, TermVectors, Terms,
};
use rucene::search::knn::KnnCollector;
use rucene::search::AcceptDocs;
use rucene::util::Bits;

/// A 4-document index (default codec) with a norms-less `id`, a `body` with
/// norms, numeric `n` and sorted `s` doc values; its only leaf.
fn small_leaf() -> Arc<dyn LeafReader> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let w = IndexWriter::new(Arc::clone(&dir), Iwc::new()).unwrap();
    for i in 0..4i64 {
        let mut d = Document::new();
        d.add(Box::new(
            StringField::new("id", i.to_string(), Store::YES).unwrap(),
        ));
        d.add(Box::new(
            TextField::new("body", format!("text number {i}"), Store::NO).unwrap(),
        ));
        d.add(Box::new(NumericDocValuesField::new("n", i)));
        d.add(Box::new(SortedDocValuesField::new(
            "s",
            BytesRef::from_char_sequence(&i.to_string()),
        )));
        w.add_document(&d).unwrap();
    }
    w.close().unwrap();
    let r: Arc<dyn IndexReader> = directory_reader::open(dir).unwrap();
    let leaves = r.leaves().unwrap();
    assert_eq!(leaves.len(), 1);
    leaves[0].leaf_reader()
}

/// How a [`Liar`] breaks the reader API.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Lie {
    /// `id` (no norms) returns `body`'s norms.
    Norms,
    /// `id` (no doc values) returns `n`'s numeric doc values.
    DocValues,
    /// `s` (sorted) also returns `n`'s numeric doc values.
    MultipleDocValues,
    /// Every `getFieldInfos()` builds new field infos.
    FieldInfos,
}

/// A `FilterLeafReader` (not a `CodecReader`) whose reader API lies.
#[derive(Debug)]
struct Liar {
    inner: FilterLeafReader,
    lie: Lie,
}

impl std::fmt::Display for Liar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Liar({})", self.inner)
    }
}

impl LeafReader for Liar {
    fn core(&self) -> &IndexReaderCore {
        LeafReader::core(&self.inner)
    }
    fn term_vectors(&self) -> rucene::error::Result<Box<dyn TermVectors>> {
        LeafReader::term_vectors(&self.inner)
    }
    fn num_docs(&self) -> i32 {
        LeafReader::num_docs(&self.inner)
    }
    fn max_doc(&self) -> i32 {
        LeafReader::max_doc(&self.inner)
    }
    fn stored_fields(&self) -> rucene::error::Result<Box<dyn StoredFields>> {
        LeafReader::stored_fields(&self.inner)
    }
    fn do_close(&self) -> rucene::error::Result<()> {
        LeafReader::do_close(&self.inner)
    }
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
    fn terms(&self, field: &str) -> rucene::error::Result<Option<Box<dyn Terms>>> {
        self.inner.terms(field)
    }
    fn get_numeric_doc_values(
        &self,
        field: &str,
    ) -> rucene::error::Result<Option<Box<dyn NumericDocValues>>> {
        match (self.lie, field) {
            (Lie::DocValues, "id") | (Lie::MultipleDocValues, "s") => {
                self.inner.get_numeric_doc_values("n")
            }
            _ => self.inner.get_numeric_doc_values(field),
        }
    }
    fn get_binary_doc_values(
        &self,
        field: &str,
    ) -> rucene::error::Result<Option<Box<dyn BinaryDocValues>>> {
        self.inner.get_binary_doc_values(field)
    }
    fn get_sorted_doc_values(
        &self,
        field: &str,
    ) -> rucene::error::Result<Option<Box<dyn SortedDocValues>>> {
        self.inner.get_sorted_doc_values(field)
    }
    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> rucene::error::Result<Option<Box<dyn SortedNumericDocValues>>> {
        self.inner.get_sorted_numeric_doc_values(field)
    }
    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> rucene::error::Result<Option<Box<dyn SortedSetDocValues>>> {
        self.inner.get_sorted_set_doc_values(field)
    }
    fn get_norm_values(
        &self,
        field: &str,
    ) -> rucene::error::Result<Option<Box<dyn NumericDocValues>>> {
        match (self.lie, field) {
            (Lie::Norms, "id") => self.inner.get_norm_values("body"),
            _ => self.inner.get_norm_values(field),
        }
    }
    fn get_doc_values_skipper(
        &self,
        field: &str,
    ) -> rucene::error::Result<Option<Box<dyn DocValuesSkipper>>> {
        self.inner.get_doc_values_skipper(field)
    }
    fn get_float_vector_values(
        &self,
        field: &str,
    ) -> rucene::error::Result<Option<Box<dyn FloatVectorValues>>> {
        self.inner.get_float_vector_values(field)
    }
    fn get_byte_vector_values(
        &self,
        field: &str,
    ) -> rucene::error::Result<Option<Box<dyn ByteVectorValues>>> {
        self.inner.get_byte_vector_values(field)
    }
    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> rucene::error::Result<()> {
        self.inner
            .search_nearest_vectors(field, target, collector, accept_docs)
    }
    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> rucene::error::Result<()> {
        self.inner
            .search_nearest_vectors_byte(field, target, collector, accept_docs)
    }
    fn get_field_infos(&self) -> FieldInfos {
        let infos = self.inner.get_field_infos();
        if self.lie == Lie::FieldInfos {
            FieldInfos::new(infos.iter().cloned().collect()).unwrap()
        } else {
            infos
        }
    }
    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.inner.get_live_docs()
    }
    fn get_point_values(&self, field: &str) -> rucene::error::Result<Option<Box<dyn PointValues>>> {
        self.inner.get_point_values(field)
    }
    fn check_integrity(&self) -> rucene::error::Result<()> {
        self.inner.check_integrity()
    }
    fn get_meta_data(&self) -> LeafMetaData {
        self.inner.get_meta_data()
    }
}

/// `checkReader` passes on a clean index, through the reader itself (a
/// `CodecReader`) and through `SlowCodecReaderWrapper` (a
/// `FilterLeafReader`), and fails with Java's message on each lie of the
/// reader API.
#[test]
fn check_reader_checks_the_reader_api() {
    let leaf = small_leaf();
    assert!(Arc::clone(&leaf).as_codec_reader().is_some());
    TestUtil::check_leaf_reader(Arc::clone(&leaf), 3).unwrap();
    let r: Arc<dyn IndexReader> = Arc::clone(&leaf).as_index_reader();
    TestUtil::check_reader(&r).unwrap();
    let filtered: Arc<dyn LeafReader> = Arc::new(FilterLeafReader::new(Arc::clone(&leaf)).unwrap());
    assert!(Arc::clone(&filtered).as_codec_reader().is_none());
    TestUtil::check_leaf_reader(filtered, 3).unwrap();

    for (lie, message) in [
        (Lie::Norms, "field: id should omit norms but has them!".to_string()),
        (Lie::DocValues, "field: id has docvalues but should omit them!".to_string()),
        (Lie::MultipleDocValues, "s returns multiple docvalues types!".to_string()),
        (
            Lie::FieldInfos,
            format!(
                "getFieldInfos() returned different instances for class: Liar(FilterLeafReader({}))",
                leaf
            ),
        ),
    ] {
        let liar: Arc<dyn LeafReader> = Arc::new(Liar {
            inner: FilterLeafReader::new(Arc::clone(&leaf)).unwrap(),
            lie,
        });
        match TestUtil::check_leaf_reader(liar, 3).unwrap_err() {
            rucene::error::LuceneError { kind: rucene::error::LuceneErrorKind::Unchecked {
                class,
                message: actual,
                ..
            }, .. } => {
                assert_eq!(class, "java.lang.RuntimeException", "{lie:?}");
                assert_eq!(actual.as_deref(), Some(message.as_str()), "{lie:?}");
            }
            other => panic!("{lie:?}: {other}"),
        }
    }
}

/// The mock-filesystem probes find `WindowsFS` and `VirusCheckingFS` under
/// an `FSDirectory` and its `FilterDirectory` wrappers, not on other
/// directories; `disableVirusChecker` reports whether the checker was
/// enabled, and `enableVirusChecker` enables it again.
#[test]
fn mock_filesystem_probes() {
    use rucene::store::java_nio_file::{default_file_system, FileSystemProvider};
    use rucene::store::{FilterDirectory, NIOFSDirectory};
    use rucene_test_framework::mockfile::{VirusCheckingFS, WindowsFS};

    let tmp = oracle_support::TempDir::new("testutil-probes");
    let ram = ByteBuffersDirectory::new();
    assert!(!TestUtil::has_windows_fs(&ram).unwrap());
    assert!(!TestUtil::has_virus_checker(&ram).unwrap());
    assert!(!TestUtil::disable_virus_checker(&ram).unwrap());
    TestUtil::enable_virus_checker(&ram).unwrap();

    // WindowsFS over VirusCheckingFS over the default filesystem.
    let virus = VirusCheckingFS::new(default_file_system(), 42);
    let windows = WindowsFS::new(Arc::clone(&virus).get_file_system());
    let fs = windows.get_file_system();
    let root = Arc::clone(&fs).get_path(&tmp.str(), &[]);
    assert!(TestUtil::has_windows_fs_path(&root));
    assert!(TestUtil::has_virus_checker_path(&root));
    let plain = default_file_system().get_path(&tmp.str(), &[]);
    assert!(!TestUtil::has_windows_fs_path(&plain));
    assert!(!TestUtil::has_virus_checker_path(&plain));

    let dir = FilterDirectory::new(Box::new(
        NIOFSDirectory::open(root.resolve_str("idx")).unwrap(),
    ));
    assert!(TestUtil::has_windows_fs(&dir).unwrap());
    assert!(TestUtil::has_virus_checker(&dir).unwrap());
    assert!(virus.is_enabled());
    assert!(TestUtil::disable_virus_checker(&dir).unwrap());
    assert!(!virus.is_enabled());
    assert!(!TestUtil::disable_virus_checker(&dir).unwrap());
    TestUtil::enable_virus_checker(&dir).unwrap();
    assert!(virus.is_enabled());

    // Only the virus checker.
    let root = Arc::clone(&virus)
        .get_file_system()
        .get_path(&tmp.str(), &[]);
    let dir = NIOFSDirectory::open(root.resolve_str("idx2")).unwrap();
    assert!(!TestUtil::has_windows_fs(&dir).unwrap());
    assert!(TestUtil::has_virus_checker(&dir).unwrap());
}

/// `shutdownExecutorService` shuts the executor down; `null` is ignored.
#[test]
fn shutdown_executor_service_shuts_down() {
    let ex = rucene::util::concurrent::SameThreadExecutorService::new();
    TestUtil::shutdown_executor_service(Some(&ex));
    assert!(ex.is_shutdown());
    TestUtil::shutdown_executor_service(None);
}

/// `assertAttributeReflection` accepts the map the attribute reflects and
/// fails with JUnit's message on any other.
#[test]
fn assert_attribute_reflection_compares_the_reflected_map() {
    use rucene::analysis::tokenattributes::{FlagsAttribute, FlagsAttributeImpl};
    let mut att = FlagsAttributeImpl::default();
    att.set_flags(5);
    let name = std::any::type_name::<FlagsAttributeImpl>();
    let expected = std::collections::HashMap::from([(format!("{name}#flags"), "5".to_string())]);
    TestUtil::assert_attribute_reflection(&att, &expected);
    let wrong = std::collections::HashMap::from([(format!("{name}#flags"), "6".to_string())]);
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        TestUtil::assert_attribute_reflection(&att, &wrong)
    }))
    .unwrap_err();
    assert_eq!(
        oracle_support::panic_error(failure),
        (
            "java.lang.AssertionError".to_string(),
            format!(
                "Reflection does not produce same map expected:<{{{name}#flags=6}}> but was:<{{{name}#flags=5}}>"
            )
        )
    );
}

/// `syncConcurrentMerges` waits for a `ConcurrentMergeScheduler` and ignores
/// any other scheduler.
#[test]
fn sync_concurrent_merges_on_any_scheduler() {
    TestUtil::sync_concurrent_merges(&SerialMergeScheduler::new());
    TestUtil::sync_concurrent_merges(&ConcurrentMergeScheduler::new());
}

/// `checkIndex` reports a corrupted terms index (`.tip`) or terms dictionary
/// (`.tim`) of a default-codec index as Java does: the segment's `check
/// integrity` step fails with `checksum failed (hardware problem?)` naming
/// the file (`Lucene103BlockTreeTermsReader.checkIntegrity` checksums both,
/// `Lucene103BlockTreeTermsReader.java:303-313`), before any test reads the
/// corrupted terms (where a lookup used to panic, `TrieReader`).
#[test]
fn check_index_reports_a_corrupt_terms_index_or_dictionary() {
    for extension in [".tip", ".tim"] {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let config = Iwc::new();
        config.set_use_compound_file(false).unwrap();
        config
            .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
            .unwrap();
        let w = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        let docs = LineFileDocs::new(&JdkRandom::new(42), "europarl.lines.txt.gz").unwrap();
        for _ in 0..60 {
            w.add_document(&*docs.next_doc().unwrap().borrow()).unwrap();
        }
        w.close().unwrap();
        let name = dir
            .list_all()
            .unwrap()
            .into_iter()
            .find(|n| n.ends_with(extension))
            .unwrap();
        let len = dir.file_length(&name).unwrap() as usize;
        let mut bytes = vec![0u8; len];
        {
            let mut input = dir.open_input(&name, &*DEFAULT_IO_CONTEXT).unwrap();
            input.read_bytes(Some(&mut bytes), 0, len).unwrap();
        }
        bytes[len / 2] ^= 0x5a;
        dir.delete_file(&name).unwrap();
        {
            let mut out = dir.create_output(&name, &*DEFAULT_IO_CONTEXT).unwrap();
            out.write_bytes(&bytes, 0, len).unwrap();
            out.close().unwrap();
        }
        let mut output = Vec::new();
        let err = TestUtil::check_index_with(Arc::clone(&dir), 3, false, false, Some(&mut output))
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "java.lang.RuntimeException: CheckIndex failed"
        );
        let output = String::from_utf8(output).unwrap();
        let failure = output
            .lines()
            .find(|l| l.contains("checksum failed (hardware problem?) : expected="))
            .unwrap_or_else(|| panic!("{extension}: {output}"));
        assert!(failure.contains(&format!("file={name}")), "{failure}");
        assert!(output.contains("test: open reader.........OK"), "{output}");
        assert!(!output.contains("test: check integrity.....OK"), "{output}");
    }
}
