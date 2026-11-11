//! Port of `lucene/core/src/test/org/apache/lucene/index/TestParallelCompositeReader.java`.
//!
//! The instance fields `parallel`, `single`, `dir`, `dir1` and `dir2`, which
//! each query test sets and clears, are the locals of those tests.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_reader::CacheHelper;
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::multi_reader::MultiReader;
use rucene::index::parallel_reader::ParallelCompositeReader;
use rucene::index::terms::Term;
use rucene::index::{CompositeReader, IndexReader};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::term_query::TermQuery;
use rucene::search::{BooleanQueryBuilder, Occur, Query};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_null, assert_null, class, random, test, IAE,
};
use crate::support::npe;

/// `TestParallelCompositeReader`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestParallelCompositeReader",
        vec![
            test("testQueries", test_queries),
            test(
                "testQueriesCompositeComposite",
                test_queries_composite_composite,
            ),
            test("testRefCounts1", test_ref_counts1),
            test("testRefCounts2", test_ref_counts2),
            test("testReaderClosedListener1", test_reader_closed_listener1),
            test("testCloseInnerReader", test_close_inner_reader),
            test("testIncompatibleIndexes1", test_incompatible_indexes1),
            test("testIncompatibleIndexes2", test_incompatible_indexes2),
            test("testIgnoreStoredFields", test_ignore_stored_fields),
            test("testToString", test_to_string),
            test(
                "testToStringCompositeComposite",
                test_to_string_composite_composite,
            ),
        ],
    )
}

const ACE: &str = "org.apache.lucene.store.AlreadyClosedException";

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn new_iwc() -> rucene::index::index_writer_config::IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `newTextField(name, value, Field.Store.YES)`.
fn text(name: &str, value: &str) -> Box<rucene::document::Field> {
    Box::new(LuceneTestCase::new_text_field(name, value, Store::YES))
}

/// `DirectoryReader.open(dir)` as a `CompositeReader`.
fn open(dir: &TestDirectory) -> Result<Arc<dyn CompositeReader>> {
    Ok(directory_reader::open(dir.as_directory())? as Arc<dyn CompositeReader>)
}

/// `reader.storedFields().document(0).get(field)`.
fn stored(reader: &dyn IndexReader, field: &str) -> Result<Option<String>> {
    Ok(reader.stored_fields()?.document(0)?.get(field))
}

/// `MultiTerms.getTerms(pr, field)`, as `assertNull`/`assertNotNull` see it.
fn terms(pr: &Arc<ParallelCompositeReader>, field: &str) -> Result<Option<&'static str>> {
    let ir: Arc<dyn IndexReader> = Arc::clone(pr) as Arc<dyn IndexReader>;
    Ok(MultiTerms::get_terms(&ir, field)?.map(|_| "terms"))
}

/// `assertEquals(a, b)` over two cache helpers, which Java compares by
/// identity: the same key object, or both `null`.
fn assert_same_helper(a: Option<Box<dyn CacheHelper>>, b: Option<Box<dyn CacheHelper>>) {
    let same = match (&a, &b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.get_key() == b.get_key(),
        _ => false,
    };
    if !same {
        Assert::fail(&format!("expected:<{a:?}> but was:<{b:?}>"));
    }
}

/// `pr.getSequentialSubReaders().get(0)`.
fn first_sub(pr: &dyn CompositeReader) -> Arc<dyn IndexReader> {
    pr.get_sequential_sub_readers()
        .into_iter()
        .next()
        .unwrap_or_else(|| npe("getSequentialSubReaders().get(0)"))
}

/// The searchers and directories a query test builds and closes.
struct QueryFixture {
    parallel: IndexSearcher,
    single: IndexSearcher,
    dir: TestDirectory,
    dir1: TestDirectory,
    dir2: TestDirectory,
}

impl QueryFixture {
    fn new(composite_composite: bool) -> Result<Self> {
        let (single, dir) = single(composite_composite)?;
        let (parallel, dir1, dir2) = parallel(composite_composite)?;
        Ok(QueryFixture {
            parallel,
            single,
            dir,
            dir1,
            dir2,
        })
    }

    /// The closing statements of the query tests.
    fn close(self) -> Result<()> {
        let QueryFixture {
            parallel,
            single,
            mut dir,
            mut dir1,
            mut dir2,
        } = self;
        single.get_index_reader().close()?;
        drop(single);
        parallel.get_index_reader().close()?;
        drop(parallel);
        dir.close()?;
        dir1.close()?;
        dir2.close()
    }
}

/// Port of `TestParallelCompositeReader.testQueries`.
fn test_queries() -> Result<()> {
    let fixture = QueryFixture::new(false)?;

    queries(&fixture)?;

    fixture.close()
}

/// Port of `TestParallelCompositeReader.testQueriesCompositeComposite`.
fn test_queries_composite_composite() -> Result<()> {
    let fixture = QueryFixture::new(true)?;

    queries(&fixture)?;

    fixture.close()
}

/// `private void queries()`.
fn queries(f: &QueryFixture) -> Result<()> {
    let q =
        |f: &str, v: &str| -> Arc<dyn Query> { Arc::new(TermQuery::new(Term::from_text(f, v))) };
    query_test(f, q("f1", "v1"))?;
    query_test(f, q("f1", "v2"))?;
    query_test(f, q("f2", "v1"))?;
    query_test(f, q("f2", "v2"))?;
    query_test(f, q("f3", "v1"))?;
    query_test(f, q("f3", "v2"))?;
    query_test(f, q("f4", "v1"))?;
    query_test(f, q("f4", "v2"))?;

    let mut bq1 = BooleanQueryBuilder::new();
    bq1.add(q("f1", "v1"), Occur::MUST)?;
    bq1.add(q("f4", "v1"), Occur::MUST)?;
    query_test(f, Arc::new(bq1.build()))
}

/// Port of `TestParallelCompositeReader.testRefCounts1`.
fn test_ref_counts1() -> Result<()> {
    let mut dir1 = get_dir1()?;
    let mut dir2 = get_dir2()?;
    // close subreaders, ParallelReader will not change refCounts, but close on its own close
    let ir1 = directory_reader::open(dir1.as_directory())?;
    let ir2 = directory_reader::open(dir2.as_directory())?;
    let pr = Arc::new(ParallelCompositeReader::new(vec![
        Arc::clone(&ir1) as Arc<dyn CompositeReader>,
        Arc::clone(&ir2) as Arc<dyn CompositeReader>,
    ])?);
    let psub1 = first_sub(&*pr);
    // check RefCounts
    assert_equals(&1, &ir1.get_ref_count());
    assert_equals(&1, &ir2.get_ref_count());
    assert_equals(&1, &psub1.get_ref_count());
    pr.close()?;
    assert_equals(&0, &ir1.get_ref_count());
    assert_equals(&0, &ir2.get_ref_count());
    assert_equals(&0, &psub1.get_ref_count());
    dir1.close()?;
    dir2.close()
}

/// Port of `TestParallelCompositeReader.testRefCounts2`.
fn test_ref_counts2() -> Result<()> {
    let mut dir1 = get_dir1()?;
    let mut dir2 = get_dir2()?;
    let ir1 = directory_reader::open(dir1.as_directory())?;
    let ir2 = directory_reader::open(dir2.as_directory())?;

    // don't close subreaders, so ParallelReader will increment refcounts
    let pr = Arc::new(ParallelCompositeReader::with_close_flag(
        false,
        vec![
            Arc::clone(&ir1) as Arc<dyn CompositeReader>,
            Arc::clone(&ir2) as Arc<dyn CompositeReader>,
        ],
    )?);
    let psub1 = first_sub(&*pr);
    // check RefCounts
    assert_equals(&2, &ir1.get_ref_count());
    assert_equals(&2, &ir2.get_ref_count());
    assert_equals_msg(
        Some("refCount must be 1, as the synthetic reader was created by ParallelCompositeReader"),
        &1,
        &psub1.get_ref_count(),
    );
    pr.close()?;
    assert_equals(&1, &ir1.get_ref_count());
    assert_equals(&1, &ir2.get_ref_count());
    assert_equals_msg(
        Some("refcount must be 0 because parent was closed"),
        &0,
        &psub1.get_ref_count(),
    );
    ir1.close()?;
    ir2.close()?;
    assert_equals(&0, &ir1.get_ref_count());
    assert_equals(&0, &ir2.get_ref_count());
    assert_equals_msg(
        Some("refcount should not change anymore"),
        &0,
        &psub1.get_ref_count(),
    );
    dir1.close()?;
    dir2.close()
}

/// `private void testReaderClosedListener1(boolean closeSubReaders, int
/// wrapMultiReaderType)`.
fn reader_closed_listener1(close_sub_readers: bool, wrap_multi_reader_type: i32) -> Result<()> {
    let mut dir1 = get_dir1()?;
    let ir1 = open(&dir1)?;
    let ir2: Arc<dyn CompositeReader> = match wrap_multi_reader_type {
        0 => Arc::clone(&ir1),
        1 => {
            // default case, does close subreaders:
            Arc::new(MultiReader::new_closing(vec![
                Arc::clone(&ir1) as Arc<dyn IndexReader>
            ])?)
        }
        2 => Arc::new(MultiReader::new(
            vec![Arc::clone(&ir1) as Arc<dyn IndexReader>],
            false,
        )?),
        _ => Assert::fail_without_message(),
    };

    // with overlapping
    let pr = Arc::new(ParallelCompositeReader::with_stored_fields(
        close_sub_readers,
        vec![Arc::clone(&ir2)],
        vec![Arc::clone(&ir2)],
    )?);

    let pr_leaves = Arc::clone(&pr).leaves()?;
    assert_equals(&3, &pr_leaves.len());
    assert_same_helper(ir1.get_reader_cache_helper(), pr.get_reader_cache_helper());

    let ir1_leaves = Arc::clone(&ir1).leaves()?;
    let mut i = 0;
    for cxt in &pr_leaves {
        let original_leaf = ir1_leaves[i].leaf_reader();
        i += 1;
        assert_same_helper(
            original_leaf.get_core_cache_helper(),
            cxt.leaf_reader().get_core_cache_helper(),
        );
        assert_same_helper(
            original_leaf.get_reader_cache_helper(),
            cxt.leaf_reader().get_reader_cache_helper(),
        );
    }
    pr.close()?;
    if !close_sub_readers {
        ir1.close()?;
    }

    // We have to close the extra MultiReader, because it will not close its own subreaders:
    if wrap_multi_reader_type == 2 {
        ir2.close()?;
    }
    dir1.close()
}

/// Port of `TestParallelCompositeReader.testReaderClosedListener1`.
fn test_reader_closed_listener1() -> Result<()> {
    reader_closed_listener1(false, 0)?;
    reader_closed_listener1(true, 0)?;
    reader_closed_listener1(false, 1)?;
    reader_closed_listener1(true, 1)?;
    reader_closed_listener1(false, 2)
}

/// Port of `TestParallelCompositeReader.testCloseInnerReader`.
fn test_close_inner_reader() -> Result<()> {
    let mut dir1 = get_dir1()?;
    let ir1 = open(&dir1)?;
    assert_equals(&1, &first_sub(&*ir1).get_ref_count());

    // with overlapping
    let pr = Arc::new(ParallelCompositeReader::with_stored_fields(
        true,
        vec![Arc::clone(&ir1)],
        vec![Arc::clone(&ir1)],
    )?);

    let psub = first_sub(&*pr);
    assert_equals(&1, &psub.get_ref_count());

    ir1.close()?;

    assert_equals_msg(
        Some("refCount of synthetic subreader should be unchanged"),
        &1,
        &psub.get_ref_count(),
    );
    expect_throws(ACE, || psub.stored_fields()?.document(0));

    expect_throws(ACE, || pr.stored_fields()?.document(0));

    // noop:
    pr.close()?;
    assert_equals(&0, &psub.get_ref_count());
    dir1.close()
}

/// Port of `TestParallelCompositeReader.testIncompatibleIndexes1`.
fn test_incompatible_indexes1() -> Result<()> {
    // two documents:
    let mut dir1 = get_dir1()?;

    // one document only:
    let mut dir2 = LuceneTestCase::new_directory();
    let w2 = IndexWriter::new(dir2.as_directory(), new_iwc())?;
    let mut d3 = Document::new();

    d3.add(text("f3", "v1"));
    w2.add_document(&d3)?;
    w2.close()?;

    let ir1 = directory_reader::open(dir1.as_directory())?;
    let ir2 = directory_reader::open(dir2.as_directory())?;
    let readers = || {
        vec![
            Arc::clone(&ir1) as Arc<dyn CompositeReader>,
            Arc::clone(&ir2) as Arc<dyn CompositeReader>,
        ]
    };

    expect_throws(IAE, || ParallelCompositeReader::new(readers()));

    expect_throws(IAE, || {
        ParallelCompositeReader::with_close_flag(random().next_boolean(), readers())
    });

    assert_equals(&1, &ir1.get_ref_count());
    assert_equals(&1, &ir2.get_ref_count());
    ir1.close()?;
    ir2.close()?;
    assert_equals(&0, &ir1.get_ref_count());
    assert_equals(&0, &ir2.get_ref_count());
    dir1.close()?;
    dir2.close()
}

/// Port of `TestParallelCompositeReader.testIncompatibleIndexes2`.
fn test_incompatible_indexes2() -> Result<()> {
    let mut dir1 = get_dir1()?;
    let mut dir2 = get_invalid_structured_dir2()?;

    let ir1 = directory_reader::open(dir1.as_directory())?;
    let ir2 = directory_reader::open(dir2.as_directory())?;
    let readers: Vec<Arc<dyn CompositeReader>> = vec![
        Arc::clone(&ir1) as Arc<dyn CompositeReader>,
        Arc::clone(&ir2) as Arc<dyn CompositeReader>,
    ];
    expect_throws(IAE, || ParallelCompositeReader::new(readers.clone()));

    expect_throws(IAE, || {
        ParallelCompositeReader::with_stored_fields(
            random().next_boolean(),
            readers.clone(),
            readers.clone(),
        )
    });

    assert_equals(&1, &ir1.get_ref_count());
    assert_equals(&1, &ir2.get_ref_count());
    ir1.close()?;
    ir2.close()?;
    assert_equals(&0, &ir1.get_ref_count());
    assert_equals(&0, &ir2.get_ref_count());
    dir1.close()?;
    dir2.close()
}

/// Port of `TestParallelCompositeReader.testIgnoreStoredFields`.
fn test_ignore_stored_fields() -> Result<()> {
    let mut dir1 = get_dir1()?;
    let mut dir2 = get_dir2()?;
    let ir1 = open(&dir1)?;
    let ir2 = open(&dir2)?;

    // with overlapping
    let pr = Arc::new(ParallelCompositeReader::with_stored_fields(
        false,
        vec![Arc::clone(&ir1), Arc::clone(&ir2)],
        vec![Arc::clone(&ir1)],
    )?);
    assert_equals(&Some("v1".to_string()), &stored(&*pr, "f1")?);
    assert_equals(&Some("v1".to_string()), &stored(&*pr, "f2")?);
    assert_null(&stored(&*pr, "f3")?);
    assert_null(&stored(&*pr, "f4")?);
    // check that fields are there
    assert_not_null(&terms(&pr, "f1")?);
    assert_not_null(&terms(&pr, "f2")?);
    assert_not_null(&terms(&pr, "f3")?);
    assert_not_null(&terms(&pr, "f4")?);
    pr.close()?;

    // no stored fields at all
    let pr = Arc::new(ParallelCompositeReader::with_stored_fields(
        false,
        vec![Arc::clone(&ir2)],
        vec![],
    )?);
    assert_null(&stored(&*pr, "f1")?);
    assert_null(&stored(&*pr, "f2")?);
    assert_null(&stored(&*pr, "f3")?);
    assert_null(&stored(&*pr, "f4")?);
    // check that fields are there
    assert_null(&terms(&pr, "f1")?);
    assert_null(&terms(&pr, "f2")?);
    assert_not_null(&terms(&pr, "f3")?);
    assert_not_null(&terms(&pr, "f4")?);
    pr.close()?;

    // without overlapping
    let pr = Arc::new(ParallelCompositeReader::with_stored_fields(
        true,
        vec![Arc::clone(&ir2)],
        vec![Arc::clone(&ir1)],
    )?);
    assert_equals(&Some("v1".to_string()), &stored(&*pr, "f1")?);
    assert_equals(&Some("v1".to_string()), &stored(&*pr, "f2")?);
    assert_null(&stored(&*pr, "f3")?);
    assert_null(&stored(&*pr, "f4")?);
    // check that fields are there
    assert_null(&terms(&pr, "f1")?);
    assert_null(&terms(&pr, "f2")?);
    assert_not_null(&terms(&pr, "f3")?);
    assert_not_null(&terms(&pr, "f4")?);
    pr.close()?;

    // no main readers
    expect_throws(IAE, || {
        ParallelCompositeReader::with_stored_fields(true, vec![], vec![Arc::clone(&ir1)])
    });

    dir1.close()?;
    dir2.close()
}

/// Port of `TestParallelCompositeReader.testToString`.
fn test_to_string() -> Result<()> {
    let mut dir1 = get_dir1()?;
    let ir1 = open(&dir1)?;
    let pr = ParallelCompositeReader::new(vec![ir1])?;

    let s = pr.to_string();
    Assert::assert_true_msg(
        &format!("toString incorrect: {s}"),
        s.starts_with("ParallelCompositeReader(ParallelLeafReader("),
    );

    pr.close()?;
    dir1.close()
}

/// Port of `TestParallelCompositeReader.testToStringCompositeComposite`.
fn test_to_string_composite_composite() -> Result<()> {
    let mut dir1 = get_dir1()?;
    let ir1 = open(&dir1)?;
    let pr = ParallelCompositeReader::new(vec![Arc::new(MultiReader::new_closing(vec![
        ir1 as Arc<dyn IndexReader>,
    ])?) as Arc<dyn CompositeReader>])?;

    let s = pr.to_string();
    Assert::assert_true_msg(
        &format!("toString incorrect (should be flattened): {s}"),
        s.starts_with("ParallelCompositeReader(ParallelLeafReader("),
    );

    pr.close()?;
    dir1.close()
}

/// `private void queryTest(Query query)`.
fn query_test(f: &QueryFixture, query: Arc<dyn Query>) -> Result<()> {
    let parallel_hits = f.parallel.search(Arc::clone(&query), 1000)?.score_docs;
    let single_hits = f.single.search(query, 1000)?.score_docs;
    assert_equals(&parallel_hits.len(), &single_hits.len());
    let parallel_fields = f.parallel.stored_fields()?;
    let single_fields = f.single.stored_fields()?;
    for i in 0..parallel_hits.len() {
        Assert::assert_equals_float_msg(None, parallel_hits[i].score, single_hits[i].score, 0.001);
        let doc_parallel = parallel_fields.document(parallel_hits[i].doc)?;
        let doc_single = single_fields.document(single_hits[i].doc)?;
        assert_equals(&doc_parallel.get("f1"), &doc_single.get("f1"));
        assert_equals(&doc_parallel.get("f2"), &doc_single.get("f2"));
        assert_equals(&doc_parallel.get("f3"), &doc_single.get("f3"));
        assert_equals(&doc_parallel.get("f4"), &doc_single.get("f4"));
    }
    Ok(())
}

/// `private IndexSearcher single(Random random, boolean
/// compositeComposite)`: fields 1-4 indexed together. Returns the searcher
/// and the `dir` field.
fn single(composite_composite: bool) -> Result<(IndexSearcher, TestDirectory)> {
    let dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), new_iwc())?;
    for v in ["v1", "v2", "v3", "v4"] {
        let mut d = Document::new();
        d.add(text("f1", v));
        d.add(text("f2", v));
        d.add(text("f3", v));
        d.add(text("f4", v));
        w.add_document(&d)?;
    }
    w.close()?;

    let ir: Arc<dyn CompositeReader> = if composite_composite {
        Arc::new(MultiReader::new_closing(vec![
            directory_reader::open(dir.as_directory())? as Arc<dyn IndexReader>,
            directory_reader::open(dir.as_directory())? as Arc<dyn IndexReader>,
        ])?)
    } else {
        open(&dir)?
    };
    Ok((LuceneTestCase::new_searcher(ir), dir))
}

/// `private IndexSearcher parallel(Random random, boolean
/// compositeComposite)`: fields 1 & 2 in one index, 3 & 4 in other, with
/// ParallelReader. Returns the searcher and the `dir1` and `dir2` fields.
fn parallel(composite_composite: bool) -> Result<(IndexSearcher, TestDirectory, TestDirectory)> {
    let dir1 = get_dir1()?;
    let dir2 = get_dir2()?;
    let children = |r: &Arc<dyn CompositeReader>| -> Result<usize> {
        Ok(Arc::clone(r)
            .get_context()?
            .children()
            .unwrap_or_else(|| npe("children()"))
            .len())
    };
    let (rd1, rd2): (Arc<dyn CompositeReader>, Arc<dyn CompositeReader>) = if composite_composite {
        let rd1: Arc<dyn CompositeReader> = Arc::new(MultiReader::new_closing(vec![
            directory_reader::open(dir1.as_directory())? as Arc<dyn IndexReader>,
            directory_reader::open(dir1.as_directory())? as Arc<dyn IndexReader>,
        ])?);
        let rd2: Arc<dyn CompositeReader> = Arc::new(MultiReader::new_closing(vec![
            directory_reader::open(dir2.as_directory())? as Arc<dyn IndexReader>,
            directory_reader::open(dir2.as_directory())? as Arc<dyn IndexReader>,
        ])?);
        assert_equals(&2, &children(&rd1)?);
        assert_equals(&2, &children(&rd2)?);
        (rd1, rd2)
    } else {
        let rd1 = open(&dir1)?;
        let rd2 = open(&dir2)?;
        assert_equals(&3, &children(&rd1)?);
        assert_equals(&3, &children(&rd2)?);
        (rd1, rd2)
    };
    let pr: Arc<dyn CompositeReader> = Arc::new(ParallelCompositeReader::new(vec![rd1, rd2])?);
    Ok((LuceneTestCase::new_searcher(pr), dir1, dir2))
}

/// One index of `fields`, each document holding `"v1"` .. `"v4"`, committed
/// after the documents whose (0-based) positions `commits_after` lists.
fn structured_dir(fields: [&str; 2], commits_after: &[usize]) -> Result<TestDirectory> {
    let dir = LuceneTestCase::new_directory();
    let conf = new_iwc();
    conf.set_merge_policy(Some(NoMergePolicy::instance()))?;
    let w = IndexWriter::new(dir.as_directory(), conf)?;
    for (i, v) in ["v1", "v2", "v3", "v4"].into_iter().enumerate() {
        let mut d = Document::new();
        d.add(text(fields[0], v));
        d.add(text(fields[1], v));
        w.add_document(&d)?;
        if commits_after.contains(&i) {
            w.commit()?;
        }
    }
    w.close()?;
    Ok(dir)
}

/// `private Directory getDir1(Random random)`: subreader structure: (1,2,1).
fn get_dir1() -> Result<TestDirectory> {
    structured_dir(["f1", "f2"], &[0, 2])
}

/// `private Directory getDir2(Random random)`: subreader structure: (1,2,1).
fn get_dir2() -> Result<TestDirectory> {
    structured_dir(["f3", "f4"], &[0, 2])
}

/// `private Directory getInvalidStructuredDir2(Random random)`: this dir has
/// a different subreader structure (1,1,2).
fn get_invalid_structured_dir2() -> Result<TestDirectory> {
    structured_dir(["f3", "f4"], &[0, 1])
}
