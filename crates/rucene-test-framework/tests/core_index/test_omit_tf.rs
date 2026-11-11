//! Port of `lucene/core/src/test/org/apache/lucene/index/TestOmitTf.java`.

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, FieldType, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::{LogMergePolicy, MergePolicy};
use rucene::index::terms::Term;
use rucene::index::{IndexOptions, IndexReader, LeafReader};
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::scorable::Scorable;
use rucene::search::similarities::{
    tfidf_scorer, CollectionStatistics, Explanation, SimScorer, Similarity, TFIDFSimilarity,
    TermStatistics,
};
use rucene::search::term_query::TermQuery;
use rucene::search::{
    BooleanQueryBuilder, CollectionResult, CollectorManager, LeafCollector, Occur, Query,
    ScoreMode, SimpleCollector, SimpleCollectorImpl,
};
use rucene::store::Directory;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test};
use crate::support::npe;

/// `TestOmitTf`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestOmitTf",
        vec![
            test("testMixedRAM", test_mixed_ram),
            test("testNoPrxFile", test_no_prx_file),
            test("testBasic", test_basic),
            test("testStats", test_stats),
        ],
    )
}

/// `public static class SimpleSimilarity extends TFIDFSimilarity`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SimpleSimilarity;

impl Similarity for SimpleSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestOmitTf$SimpleSimilarity"
    }

    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        tfidf_scorer(self, boost, collection_stats, term_stats)
    }
}

impl TFIDFSimilarity for SimpleSimilarity {
    fn length_norm(&self, _length: i32) -> f32 {
        1.0
    }

    fn tf(&self, freq: f32) -> f32 {
        freq
    }

    fn idf(&self, _doc_freq: i64, _doc_count: i64) -> f32 {
        1.0
    }

    fn idf_explain_phrase(
        &self,
        _collection_stats: &CollectionStatistics,
        _term_stats: &[TermStatistics],
    ) -> Explanation {
        Explanation::matched(1.0f32, "Inexplicable", vec![])
    }
}

/// `private static final FieldType omitType`.
static OMIT_TYPE: LazyLock<FieldType> = LazyLock::new(|| {
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_index_options(IndexOptions::DOCS)
        .unwrap_or_else(|e| panic!("{e}"));
    ft
});

/// `private static final FieldType normalType`.
static NORMAL_TYPE: LazyLock<FieldType> =
    LazyLock::new(|| FieldType::new_from(TextField::type_not_stored()));

/// `newIndexWriterConfig(analyzer).setMaxBufferedDocs(n)`, the merge policy
/// created after the configuration, in Java's order.
fn config(
    analyzer: MockAnalyzer,
    max_buffered_docs: i32,
    merge_policy: impl FnOnce() -> Result<LogMergePolicy>,
) -> Result<IndexWriterConfig> {
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer)));
    conf.set_max_buffered_docs(max_buffered_docs)?;
    conf.set_merge_policy(Some(Arc::new(merge_policy()?)))?;
    Ok(conf)
}

/// `fi.fieldInfo(name).getIndexOptions()`.
fn index_options(reader: &Arc<dyn LeafReader>, name: &str) -> IndexOptions {
    reader
        .get_field_infos()
        .field_info(name)
        .unwrap_or_else(|| npe("fieldInfo"))
        .get_index_options()
}

/// Port of `TestOmitTf.testMixedRAM`: make sure first adding docs that do not
/// omitTermFreqAndPositions for field X, then adding docs that do
/// omitTermFreqAndPositions for that same field.
fn test_mixed_ram() -> Result<()> {
    let ram = LuceneTestCase::new_directory();
    let analyzer = MockAnalyzer::new(&*random());
    let writer = IndexWriter::new(
        ram.as_directory(),
        config(analyzer, 10, || {
            LuceneTestCase::new_log_merge_policy_merge_factor(2)
        })?,
    )?;
    let mut d = Document::new();

    // this field will have Tf
    let f1 = LuceneTestCase::new_field("f1", "This field has term freqs", &NORMAL_TYPE);
    d.add(Box::new(f1));

    // this field will NOT have Tf
    let f2 = LuceneTestCase::new_field("f2", "This field has NO Tf in all docs", &OMIT_TYPE);
    d.add(Box::new(f2));

    for _ in 0..5 {
        writer.add_document(&d)?;
    }

    for _ in 0..20 {
        writer.add_document(&d)?;
    }

    // force merge
    writer.force_merge(1)?;

    // flush
    writer.close()?;

    let reader = LuceneTestCase::get_only_leaf_reader(
        directory_reader::open(ram.as_directory())? as Arc<dyn IndexReader>
    )?;
    assert_equals_msg(
        Some("OmitTermFreqAndPositions field bit should not be set."),
        &IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
        &index_options(&reader, "f1"),
    );
    assert_equals_msg(
        Some("OmitTermFreqAndPositions field bit should be set."),
        &IndexOptions::DOCS,
        &index_options(&reader, "f2"),
    );

    Arc::clone(&reader).as_index_reader().close()?;
    ram.close()
}

/// `private void assertNoPrx(Directory dir)`.
fn assert_no_prx(dir: &Arc<dyn Directory>) -> Result<()> {
    let files = dir.list_all()?;
    for file in &files {
        Assert::assert_false(file.ends_with(".prx"));
        Assert::assert_false(file.ends_with(".pos"));
    }
    Ok(())
}

/// Port of `TestOmitTf.testNoPrxFile`: verifies no *.prx exists when all
/// fields omit term freq.
fn test_no_prx_file() -> Result<()> {
    let ram = LuceneTestCase::new_directory();
    let analyzer = MockAnalyzer::new(&*random());
    let writer = IndexWriter::new(
        ram.as_directory(),
        config(analyzer, 3, || Ok(LuceneTestCase::new_log_merge_policy()))?,
    )?;
    let mp = writer.get_config().merge_policy();
    let lmp = mp
        .as_any()
        .and_then(|a| a.downcast_ref::<LogMergePolicy>())
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::Throwable::new(
                "java.lang.ClassCastException",
                "not a LogMergePolicy",
            )
            .throw()
        });
    lmp.set_merge_factor(2)?;
    lmp.set_no_cfs_ratio(0.0)?;
    let mut d = Document::new();

    let f1 = LuceneTestCase::new_field("f1", "This field has term freqs", &OMIT_TYPE);
    d.add(Box::new(f1));

    for _ in 0..30 {
        writer.add_document(&d)?;
    }

    writer.commit()?;

    assert_no_prx(&ram.as_directory())?;

    writer.close()?;
    ram.close()
}

/// `private abstract static class ScoreAssertingCollector extends
/// SimpleCollector`, with the anonymous `collect` of each query of
/// `testBasic`.
struct ScoreAssertingCollector {
    check: fn(i32, f32),
}

impl LeafCollector for ScoreAssertingCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let score = scorer.score()?;
        (self.check)(doc, score);
        Ok(())
    }
}

impl SimpleCollectorImpl for ScoreAssertingCollector {
    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }
}

/// The anonymous `CollectorManager<SimpleCollector, Void>` of each query of
/// `testBasic`.
struct ScoreAssertingManager {
    check: fn(i32, f32),
}

impl CollectorManager for ScoreAssertingManager {
    type Collector = SimpleCollector<ScoreAssertingCollector>;
    type Output = ();

    fn new_collector(&self) -> Result<Self::Collector> {
        Ok(SimpleCollector::new(ScoreAssertingCollector {
            check: self.check,
        }))
    }

    fn reduce(&self, _collectors: Vec<Self::Collector>) -> Result<()> {
        Ok(())
    }
}

/// Port of `TestOmitTf.testBasic`: test scores with one field with Term
/// Freqs and one without, otherwise with equal content.
fn test_basic() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let analyzer = MockAnalyzer::new(&*random());
    let writer = IndexWriter::new(dir.as_directory(), {
        let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer)));
        conf.set_max_buffered_docs(2)?;
        conf.set_similarity(Some(Arc::new(SimpleSimilarity)))?;
        conf.set_merge_policy(Some(Arc::new(
            LuceneTestCase::new_log_merge_policy_merge_factor(2)?,
        )))?;
        conf
    })?;

    let mut sb = String::with_capacity(265);
    let term = "term";
    for i in 0..30 {
        let mut d = Document::new();
        sb.push_str(term);
        sb.push(' ');
        let content = sb.clone();
        let no_tf = LuceneTestCase::new_field(
            "noTf",
            &format!("{content}{}", if i % 2 == 0 { "" } else { " notf" }),
            &OMIT_TYPE,
        );
        d.add(Box::new(no_tf));

        let tf = LuceneTestCase::new_field(
            "tf",
            &format!("{content}{}", if i % 2 == 0 { " tf" } else { "" }),
            &NORMAL_TYPE,
        );
        d.add(Box::new(tf));

        writer.add_document(&d)?;
        // System.out.println(d);
    }

    writer.force_merge(1)?;
    // flush
    writer.close()?;

    /*
     * Verify the index
     */
    let reader_dr = directory_reader::open(dir.as_directory())?;
    let reader: Arc<dyn IndexReader> = Arc::clone(&reader_dr) as Arc<dyn IndexReader>;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader_dr));
    searcher.set_similarity(Arc::new(SimpleSimilarity));

    let a = Term::from_text("noTf", term);
    let b = Term::from_text("tf", term);
    let c = Term::from_text("noTf", "notf");
    let d = Term::from_text("tf", "tf");
    let q1: Arc<dyn Query> = Arc::new(TermQuery::new(a.clone()));
    let q2: Arc<dyn Query> = Arc::new(TermQuery::new(b));
    let q3: Arc<dyn Query> = Arc::new(TermQuery::new(c.clone()));
    let q4: Arc<dyn Query> = Arc::new(TermQuery::new(d));

    let pq: Arc<dyn Query> = Arc::new(PhraseQuery::from_bytes(
        a.field(),
        &[a.bytes().clone(), c.bytes().clone()],
    )?);
    let expected = expect_throws("java.lang.Exception", || {
        searcher.search(Arc::clone(&pq), 10)
    });
    let mut cause = &expected;
    // If the searcher uses an executor service, the IAE is wrapped into other exceptions
    while let Some(next) = cause.cause() {
        cause = next;
    }
    Assert::assert_true_msg(
        &format!("Expected an IAE, got {cause}"),
        cause.is_instance("java.lang.IllegalStateException"),
    );

    searcher.search_with_manager(
        Arc::clone(&q1),
        &ScoreAssertingManager {
            check: |_doc, score| {
                // System.out.println("Q1: Doc=" + doc + " score=" + score);
                Assert::assert_true_msg(&format!("got score={score}"), score == 1.0f32);
            },
        },
    )?;

    searcher.search_with_manager(
        q2,
        &ScoreAssertingManager {
            check: |doc, score| {
                // System.out.println("Q2: Doc=" + doc + " score=" + score);
                Assert::assert_equals_float_msg(None, 1.0f32 + doc as f32, score, 0.00001f32);
            },
        },
    )?;

    searcher.search_with_manager(
        q3,
        &ScoreAssertingManager {
            check: |doc, score| {
                // System.out.println("Q1: Doc=" + doc + " score=" + score);
                Assert::assert_true(score == 1.0f32);
                Assert::assert_false(doc % 2 == 0);
            },
        },
    )?;

    searcher.search_with_manager(
        Arc::clone(&q4),
        &ScoreAssertingManager {
            check: |doc, score| {
                // System.out.println("Q1: Doc=" + doc + " score=" + score);
                Assert::assert_true(score == 1.0f32);
                Assert::assert_true(doc % 2 == 0);
            },
        },
    )?;

    let mut bq = BooleanQueryBuilder::new();
    bq.add(q1, Occur::MUST)?;
    bq.add(q4, Occur::MUST)?;

    let count = searcher.count(Arc::new(bq.build()))?;
    assert_equals(&15, &count);

    drop(searcher);
    reader.close()?;
    dir.close()
}

/// Port of `TestOmitTf.testStats`: test that when freqs are omitted, that
/// totalTermFreq and sumTotalTermFreq are docFreq, and sumDocFreq.
fn test_stats() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_config(
        &*random(),
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;
    let mut doc = Document::new();
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_index_options(IndexOptions::DOCS)?;
    ft.freeze();
    let f = LuceneTestCase::new_field("foo", "bar", &ft);
    doc.add(Box::new(f));
    iw.add_document(&doc)?;
    let ir: Arc<dyn IndexReader> = iw.get_reader()?;
    iw.close()?;
    assert_equals(
        &(ir.doc_freq(&Term::new("foo", BytesRef::from_char_sequence("bar")))? as i64),
        &ir.total_term_freq(&Term::new("foo", BytesRef::from_char_sequence("bar")))?,
    );
    assert_equals(
        &ir.get_sum_doc_freq("foo")?,
        &ir.get_sum_total_term_freq("foo")?,
    );
    ir.close()?;
    dir.close()
}
