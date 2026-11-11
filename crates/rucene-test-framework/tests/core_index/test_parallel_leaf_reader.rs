//! Port of `lucene/core/src/test/org/apache/lucene/index/TestParallelLeafReader.java`.
//!
//! The instance fields `parallel`, `single`, `dir`, `dir1` and `dir2`, which
//! only `testQueries` sets and clears, are the locals of that test.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericDocValuesField, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_policy::LogMergePolicy;
use rucene::index::parallel_reader::ParallelLeafReader;
use rucene::index::terms::Term;
use rucene::index::{IndexReader, LeafReader};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::term_query::TermQuery;
use rucene::search::{BooleanQueryBuilder, Occur, Query};
use rucene::util::IOUtils;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_null, assert_null, class, random, test, IAE};
use crate::support::npe;

/// `TestParallelLeafReader`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestParallelLeafReader",
        vec![
            test("testQueries", test_queries),
            test("testFieldNames", test_field_names),
            test("testRefCounts1", test_ref_counts1),
            test("testRefCounts2", test_ref_counts2),
            test("testCloseInnerReader", test_close_inner_reader),
            test("testIncompatibleIndexes", test_incompatible_indexes),
            test("testIgnoreStoredFields", test_ignore_stored_fields),
            test("testWithIndexSort1", test_with_index_sort1),
            test("testWithIndexSort2", test_with_index_sort2),
            test("testWithDocValuesUpdates", test_with_doc_values_updates),
        ],
    )
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn new_iwc() -> rucene::index::index_writer_config::IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `newTextField(name, value, Field.Store.YES)`.
fn text(name: &str, value: &str) -> Box<rucene::document::Field> {
    Box::new(LuceneTestCase::new_text_field(name, value, Store::YES))
}

/// `getOnlyLeafReader(DirectoryReader.open(dir))`.
fn only_leaf(dir: &TestDirectory) -> Result<Arc<dyn LeafReader>> {
    LuceneTestCase::get_only_leaf_reader(
        directory_reader::open(dir.as_directory())? as Arc<dyn IndexReader>
    )
}

/// `getOnlyLeafReader(r)`.
fn only_leaf_of(r: &Arc<dyn IndexReader>) -> Result<Arc<dyn LeafReader>> {
    LuceneTestCase::get_only_leaf_reader(Arc::clone(r))
}

/// `reader.getRefCount()`.
fn ref_count(reader: &Arc<dyn LeafReader>) -> i32 {
    Arc::clone(reader).as_index_reader().get_ref_count()
}

/// `reader.close()`.
fn close(reader: &Arc<dyn LeafReader>) -> Result<()> {
    Arc::clone(reader).as_index_reader().close()
}

/// `reader.storedFields().document(0).get(field)`.
fn stored(reader: &Arc<dyn LeafReader>, field: &str) -> Result<Option<String>> {
    Ok(Arc::clone(reader)
        .as_index_reader()
        .stored_fields()?
        .document(0)?
        .get(field))
}

/// `new ParallelLeafReader(...)` as the `LeafReader` Java holds.
fn parallel_reader(reader: ParallelLeafReader) -> Arc<dyn LeafReader> {
    Arc::new(reader)
}

/// Port of `TestParallelLeafReader.testQueries`.
fn test_queries() -> Result<()> {
    let (single, dir) = single()?;
    let (parallel, dir1, dir2) = parallel()?;

    let q =
        |f: &str, v: &str| -> Arc<dyn Query> { Arc::new(TermQuery::new(Term::from_text(f, v))) };
    query_test(&parallel, &single, q("f1", "v1"))?;
    query_test(&parallel, &single, q("f1", "v2"))?;
    query_test(&parallel, &single, q("f2", "v1"))?;
    query_test(&parallel, &single, q("f2", "v2"))?;
    query_test(&parallel, &single, q("f3", "v1"))?;
    query_test(&parallel, &single, q("f3", "v2"))?;
    query_test(&parallel, &single, q("f4", "v1"))?;
    query_test(&parallel, &single, q("f4", "v2"))?;

    let mut bq1 = BooleanQueryBuilder::new();
    bq1.add(q("f1", "v1"), Occur::MUST)?;
    bq1.add(q("f4", "v1"), Occur::MUST)?;
    query_test(&parallel, &single, Arc::new(bq1.build()))?;

    single.get_index_reader().close()?;
    drop(single);
    parallel.get_index_reader().close()?;
    drop(parallel);
    dir.close()?;
    dir1.close()?;
    dir2.close()
}

/// Port of `TestParallelLeafReader.testFieldNames`.
fn test_field_names() -> Result<()> {
    let dir1 = get_dir1()?;
    let dir2 = get_dir2()?;
    let pr = parallel_reader(ParallelLeafReader::new(vec![
        only_leaf(&dir1)?,
        only_leaf(&dir2)?,
    ])?);
    let field_infos = pr.get_field_infos();
    assert_equals(&4, &field_infos.size());
    assert_not_null(&field_infos.field_info("f1"));
    assert_not_null(&field_infos.field_info("f2"));
    assert_not_null(&field_infos.field_info("f3"));
    assert_not_null(&field_infos.field_info("f4"));
    close(&pr)?;
    dir1.close()?;
    dir2.close()
}

/// Port of `TestParallelLeafReader.testRefCounts1`.
fn test_ref_counts1() -> Result<()> {
    let dir1 = get_dir1()?;
    let dir2 = get_dir2()?;
    // close subreaders, ParallelReader will not change refCounts, but close on its own close
    let ir1 = only_leaf(&dir1)?;
    let ir2 = only_leaf(&dir2)?;
    let pr = parallel_reader(ParallelLeafReader::new(vec![
        Arc::clone(&ir1),
        Arc::clone(&ir2),
    ])?);

    // check RefCounts
    assert_equals(&1, &ref_count(&ir1));
    assert_equals(&1, &ref_count(&ir2));
    close(&pr)?;
    assert_equals(&0, &ref_count(&ir1));
    assert_equals(&0, &ref_count(&ir2));
    dir1.close()?;
    dir2.close()
}

/// Port of `TestParallelLeafReader.testRefCounts2`.
fn test_ref_counts2() -> Result<()> {
    let dir1 = get_dir1()?;
    let dir2 = get_dir2()?;
    let ir1 = only_leaf(&dir1)?;
    let ir2 = only_leaf(&dir2)?;
    // don't close subreaders, so ParallelReader will increment refcounts
    let pr = parallel_reader(ParallelLeafReader::with_close_flag(
        false,
        vec![Arc::clone(&ir1), Arc::clone(&ir2)],
    )?);
    // check RefCounts
    assert_equals(&2, &ref_count(&ir1));
    assert_equals(&2, &ref_count(&ir2));
    close(&pr)?;
    assert_equals(&1, &ref_count(&ir1));
    assert_equals(&1, &ref_count(&ir2));
    close(&ir1)?;
    close(&ir2)?;
    assert_equals(&0, &ref_count(&ir1));
    assert_equals(&0, &ref_count(&ir2));
    dir1.close()?;
    dir2.close()
}

/// Port of `TestParallelLeafReader.testCloseInnerReader`.
fn test_close_inner_reader() -> Result<()> {
    let dir1 = get_dir1()?;
    let ir1 = only_leaf(&dir1)?;

    // with overlapping
    let pr = parallel_reader(ParallelLeafReader::with_stored_fields(
        true,
        vec![Arc::clone(&ir1)],
        vec![Arc::clone(&ir1)],
    )?);

    close(&ir1)?;

    // should already be closed because inner reader is closed!
    expect_throws("org.apache.lucene.store.AlreadyClosedException", || {
        Arc::clone(&pr)
            .as_index_reader()
            .stored_fields()?
            .document(0)
    });

    // noop:
    close(&pr)?;
    dir1.close()
}

/// Port of `TestParallelLeafReader.testIncompatibleIndexes`.
fn test_incompatible_indexes() -> Result<()> {
    // two documents:
    let dir1 = get_dir1()?;

    // one document only:
    let dir2 = LuceneTestCase::new_directory();
    let w2 = IndexWriter::new(dir2.as_directory(), new_iwc())?;
    let mut d3 = Document::new();

    d3.add(text("f3", "v1"));
    w2.add_document(&d3)?;
    w2.close()?;

    let ir1 = only_leaf(&dir1)?;
    let ir2 = only_leaf(&dir2)?;

    // indexes don't have the same number of documents
    expect_throws(IAE, || {
        ParallelLeafReader::new(vec![Arc::clone(&ir1), Arc::clone(&ir2)])
    });

    expect_throws(IAE, || {
        ParallelLeafReader::with_stored_fields(
            random().next_boolean(),
            vec![Arc::clone(&ir1), Arc::clone(&ir2)],
            vec![Arc::clone(&ir1), Arc::clone(&ir2)],
        )
    });

    // check RefCounts
    assert_equals(&1, &ref_count(&ir1));
    assert_equals(&1, &ref_count(&ir2));
    close(&ir1)?;
    close(&ir2)?;
    dir1.close()?;
    dir2.close()
}

/// Port of `TestParallelLeafReader.testIgnoreStoredFields`.
fn test_ignore_stored_fields() -> Result<()> {
    let dir1 = get_dir1()?;
    let dir2 = get_dir2()?;
    let ir1 = only_leaf(&dir1)?;
    let ir2 = only_leaf(&dir2)?;

    // with overlapping
    let pr = parallel_reader(ParallelLeafReader::with_stored_fields(
        false,
        vec![Arc::clone(&ir1), Arc::clone(&ir2)],
        vec![Arc::clone(&ir1)],
    )?);
    assert_equals(&Some("v1".to_string()), &stored(&pr, "f1")?);
    assert_equals(&Some("v1".to_string()), &stored(&pr, "f2")?);
    assert_null(&stored(&pr, "f3")?);
    assert_null(&stored(&pr, "f4")?);
    // check that fields are there
    assert_not_null(&pr.terms("f1")?);
    assert_not_null(&pr.terms("f2")?);
    assert_not_null(&pr.terms("f3")?);
    assert_not_null(&pr.terms("f4")?);
    close(&pr)?;

    // no stored fields at all
    let pr = parallel_reader(ParallelLeafReader::with_stored_fields(
        false,
        vec![Arc::clone(&ir2)],
        vec![],
    )?);
    assert_null(&stored(&pr, "f1")?);
    assert_null(&stored(&pr, "f2")?);
    assert_null(&stored(&pr, "f3")?);
    assert_null(&stored(&pr, "f4")?);
    // check that fields are there
    assert_null(&pr.terms("f1")?.map(|_| "terms"));
    assert_null(&pr.terms("f2")?.map(|_| "terms"));
    assert_not_null(&pr.terms("f3")?);
    assert_not_null(&pr.terms("f4")?);
    close(&pr)?;

    // without overlapping
    let pr = parallel_reader(ParallelLeafReader::with_stored_fields(
        true,
        vec![Arc::clone(&ir2)],
        vec![Arc::clone(&ir1)],
    )?);
    assert_equals(&Some("v1".to_string()), &stored(&pr, "f1")?);
    assert_equals(&Some("v1".to_string()), &stored(&pr, "f2")?);
    assert_null(&stored(&pr, "f3")?);
    assert_null(&stored(&pr, "f4")?);
    // check that fields are there
    assert_null(&pr.terms("f1")?.map(|_| "terms"));
    assert_null(&pr.terms("f2")?.map(|_| "terms"));
    assert_not_null(&pr.terms("f3")?);
    assert_not_null(&pr.terms("f4")?);
    close(&pr)?;

    // no main readers
    expect_throws(IAE, || {
        ParallelLeafReader::with_stored_fields(true, vec![], vec![Arc::clone(&ir1)])
    });

    dir1.close()?;
    dir2.close()
}

/// `private void queryTest(Query query)`.
fn query_test(
    parallel: &IndexSearcher,
    single: &IndexSearcher,
    query: Arc<dyn Query>,
) -> Result<()> {
    let parallel_hits = parallel.search(Arc::clone(&query), 1000)?.score_docs;
    let single_hits = single.search(query, 1000)?.score_docs;
    assert_equals(&parallel_hits.len(), &single_hits.len());
    let parallel_fields = parallel.stored_fields()?;
    let single_fields = single.stored_fields()?;
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

/// `private IndexSearcher single(Random random)`: fields 1-4 indexed
/// together. Returns the searcher and the `dir` field.
fn single() -> Result<(IndexSearcher, TestDirectory)> {
    let dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), new_iwc())?;
    let mut d1 = Document::new();
    d1.add(text("f1", "v1"));
    d1.add(text("f2", "v1"));
    d1.add(text("f3", "v1"));
    d1.add(text("f4", "v1"));
    w.add_document(&d1)?;
    let mut d2 = Document::new();
    d2.add(text("f1", "v2"));
    d2.add(text("f2", "v2"));
    d2.add(text("f3", "v2"));
    d2.add(text("f4", "v2"));
    w.add_document(&d2)?;
    w.close()?;

    let ir = directory_reader::open(dir.as_directory())?;
    Ok((LuceneTestCase::new_searcher(ir), dir))
}

/// `private IndexSearcher parallel(Random random)`: fields 1 & 2 in one
/// index, 3 & 4 in other, with ParallelReader. Returns the searcher and the
/// `dir1` and `dir2` fields.
fn parallel() -> Result<(IndexSearcher, TestDirectory, TestDirectory)> {
    let dir1 = get_dir1()?;
    let dir2 = get_dir2()?;
    let pr = parallel_reader(ParallelLeafReader::new(vec![
        only_leaf(&dir1)?,
        only_leaf(&dir2)?,
    ])?);
    TestUtil::check_reader(&Arc::clone(&pr).as_index_reader())?;
    Ok((LuceneTestCase::new_searcher(pr), dir1, dir2))
}

/// `private Directory getDir1(Random random)`.
fn get_dir1() -> Result<TestDirectory> {
    let dir1 = LuceneTestCase::new_directory();
    let conf = new_iwc();
    conf.set_merge_policy(Some(Arc::new(LogMergePolicy::by_doc_count())))?;
    let w1 = IndexWriter::new(dir1.as_directory(), conf)?;
    let mut d1 = Document::new();
    d1.add(text("f1", "v1"));
    d1.add(text("f2", "v1"));
    w1.add_document(&d1)?;
    let mut d2 = Document::new();
    d2.add(text("f1", "v2"));
    d2.add(text("f2", "v2"));
    w1.add_document(&d2)?;
    w1.force_merge(1)?;
    w1.close()?;
    Ok(dir1)
}

/// `private Directory getDir2(Random random)`.
fn get_dir2() -> Result<TestDirectory> {
    let dir2 = LuceneTestCase::new_directory();
    let conf = new_iwc();
    conf.set_merge_policy(Some(Arc::new(LogMergePolicy::by_doc_count())))?;
    let w2 = IndexWriter::new(dir2.as_directory(), conf)?;
    let mut d3 = Document::new();
    d3.add(text("f3", "v1"));
    d3.add(text("f4", "v1"));
    w2.add_document(&d3)?;
    let mut d4 = Document::new();
    d4.add(text("f3", "v2"));
    d4.add(text("f4", "v2"));
    w2.add_document(&d4)?;
    w2.force_merge(1)?;
    w2.close()?;
    Ok(dir2)
}

/// An index of two empty documents in two commits, force-merged, sorted by
/// the `INT` field `sort_field` (or unsorted), as the index-sort tests build.
fn sorted_two_doc_index(sort_field: Option<&str>, two_commits: bool) -> Result<TestDirectory> {
    let dir = LuceneTestCase::new_directory();
    let iwc = new_iwc();
    if let Some(field) = sort_field {
        iwc.set_index_sort(Sort::new_fields(vec![SortField::new(
            Some(field.to_string()),
            SortFieldType::Int,
        )?])?)?;
    }
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    w.add_document(&Document::new())?;
    if two_commits {
        w.commit()?;
    }
    w.add_document(&Document::new())?;
    if two_commits {
        w.force_merge(1)?;
    }
    w.close()?;
    Ok(dir)
}

/// Port of `TestParallelLeafReader.testWithIndexSort1`: not ok to have one
/// leaf w/ index sort and another with a different index sort.
fn test_with_index_sort1() -> Result<()> {
    let dir1 = sorted_two_doc_index(Some("foo"), true)?;
    let r1: Arc<dyn IndexReader> = directory_reader::open(dir1.as_directory())?;

    let dir2 = sorted_two_doc_index(Some("bar"), true)?;
    let r2: Arc<dyn IndexReader> = directory_reader::open(dir2.as_directory())?;

    let message = expect_throws(IAE, || {
        ParallelLeafReader::new(vec![only_leaf_of(&r1)?, only_leaf_of(&r2)?])
    })
    .message()
    .map(str::to_string);
    assert_equals(
        &Some(
            "cannot combine LeafReaders that have different index sorts: saw both sort=<int: \"foo\"> and <int: \"bar\">"
                .to_string(),
        ),
        &message,
    );
    IOUtils::close([
        IOUtils::closeable(|| r1.close()),
        IOUtils::closeable(|| dir1.close()),
        IOUtils::closeable(|| r2.close()),
        IOUtils::closeable(|| dir2.close()),
    ])
}

/// Port of `TestParallelLeafReader.testWithIndexSort2`: ok to have one leaf
/// w/ index sort and the other with no sort.
fn test_with_index_sort2() -> Result<()> {
    let dir1 = sorted_two_doc_index(Some("foo"), true)?;
    let r1: Arc<dyn IndexReader> = directory_reader::open(dir1.as_directory())?;

    let dir2 = sorted_two_doc_index(None, false)?;

    let r2: Arc<dyn IndexReader> = directory_reader::open(dir2.as_directory())?;
    close(&parallel_reader(ParallelLeafReader::with_close_flag(
        false,
        vec![only_leaf_of(&r1)?, only_leaf_of(&r2)?],
    )?))?;
    close(&parallel_reader(ParallelLeafReader::with_close_flag(
        false,
        vec![only_leaf_of(&r2)?, only_leaf_of(&r1)?],
    )?))?;
    IOUtils::close([
        IOUtils::closeable(|| r1.close()),
        IOUtils::closeable(|| dir1.close()),
        IOUtils::closeable(|| r2.close()),
        IOUtils::closeable(|| dir2.close()),
    ])
}

/// Port of `TestParallelLeafReader.testWithDocValuesUpdates`.
fn test_with_doc_values_updates() -> Result<()> {
    let dir1 = LuceneTestCase::new_directory();
    let iwc1 = new_iwc();
    let w1 = IndexWriter::new(dir1.as_directory(), iwc1)?;
    let mut d = Document::new();
    d.add(Box::new(LuceneTestCase::new_text_field(
        "name",
        "billy",
        Store::NO,
    )));
    d.add(Box::new(NumericDocValuesField::new("age", 21)));
    w1.add_document(&d)?;
    w1.commit()?;
    w1.update_numeric_doc_value(Term::from_text("name", "billy"), "age", 22)?;
    w1.close()?;

    let r1: Arc<dyn IndexReader> = directory_reader::open(dir1.as_directory())?;
    let lr = parallel_reader(ParallelLeafReader::with_close_flag(
        false,
        vec![only_leaf_of(&r1)?],
    )?);

    let mut dv = lr
        .get_numeric_doc_values("age")?
        .unwrap_or_else(|| npe("getNumericDocValues"));
    assert_equals(&0, &dv.next_doc()?);
    assert_equals(&22, &dv.long_value()?);

    assert_equals(
        &1,
        &lr.get_field_infos()
            .field_info("age")
            .unwrap_or_else(|| npe("fieldInfo"))
            .get_doc_values_gen(),
    );

    IOUtils::close([
        IOUtils::closeable(|| close(&lr)),
        IOUtils::closeable(|| r1.close()),
        IOUtils::closeable(|| dir1.close()),
    ])
}
