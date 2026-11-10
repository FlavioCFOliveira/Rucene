//! Port of `lucene/core/src/test/org/apache/lucene/search/TestFieldExistsQuery.java`.
//!
//! `testNormsQueryMatchesCount` reuses one `Document` (`doc.clear()` then
//! new fields); Rucene's `Document::clear` does the same.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{
    BinaryPoint, Document, DoubleDocValuesField, Field, FieldType, KnnFloatVectorField, LongPoint,
    NumericDocValuesField, SortedDocValuesField, SortedNumericDocValuesField, Store, StringField,
    TextField,
};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{IndexOptions, IndexReader};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::term_query::TermQuery;
use rucene::search::{
    BooleanQueryBuilder, BoostQuery, ConstantScoreQuery, FieldExistsQuery, MatchAllDocsQuery,
    Occur, Query, ScoreMode, Sort, SortField,
};
use rucene::util::bit_set::BitSet;
use rucene::util::vector_util;
use rucene::util::{BytesRef, FixedBitSet};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{score_docs_of, term};
use crate::core_support::{assert_equals, assert_equals_delta, class, random, test};

/// `TestFieldExistsQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestFieldExistsQuery",
        vec![
            test(
                "testDocValuesRewriteWithTermsPresent",
                test_doc_values_rewrite_with_terms_present,
            ),
            test(
                "testDocValuesRewriteWithPointValuesPresent",
                test_doc_values_rewrite_with_point_values_present,
            ),
            test(
                "testDocValuesRewriteWithDocValuesSkipperPresent",
                test_doc_values_rewrite_with_doc_values_skipper_present,
            ),
            test("testDocValuesNoRewrite", test_doc_values_no_rewrite),
            test(
                "testDocValuesNoRewriteWithDocValues",
                test_doc_values_no_rewrite_with_doc_values,
            ),
            test("testDocValuesRandom", test_doc_values_random),
            test("testDocValuesApproximation", test_doc_values_approximation),
            test("testDocValuesScore", test_doc_values_score),
            test("testDocValuesMissingField", test_doc_values_missing_field),
            test(
                "testDocValuesAllDocsHaveField",
                test_doc_values_all_docs_have_field,
            ),
            test(
                "testDocValuesFieldExistsButNoDocsHaveField",
                test_doc_values_field_exists_but_no_docs_have_field,
            ),
            test(
                "testDocValuesQueryMatchesCount",
                test_doc_values_query_matches_count,
            ),
            test("testNormsRandom", test_norms_random),
            test("testNormsApproximation", test_norms_approximation),
            test("testNormsScore", test_norms_score),
            test("testNormsMissingField", test_norms_missing_field),
            test("testNormsAllDocsHaveField", test_norms_all_docs_have_field),
            test(
                "testNormsFieldExistsButNoDocsHaveField",
                test_norms_field_exists_but_no_docs_have_field,
            ),
            test("testNormsQueryMatchesCount", test_norms_query_matches_count),
            test("testKnnVectorRandom", test_knn_vector_random),
            test("testKnnVectorMissingField", test_knn_vector_missing_field),
            test(
                "testKnnVectorAllDocsHaveField",
                test_knn_vector_all_docs_have_field,
            ),
            test("testDeleteKnnVector", test_delete_knn_vector),
            test("testKnnVectorConjunction", test_knn_vector_conjunction),
            test(
                "testKnnVectorFieldExistsButNoDocsHaveField",
                test_knn_vector_field_exists_but_no_docs_have_field,
            ),
            test("testDeleteDocValues", test_delete_doc_values),
            test("testDeleteAllPointDocs", test_delete_all_point_docs),
            test("testDeleteAllTermDocs", test_delete_all_term_docs),
            test(
                "testDocValuesCountWithDeletionsAllDocsHaveField",
                test_doc_values_count_with_deletions_all_docs_have_field,
            ),
            test(
                "testVectorCountWithDeletionsAllDocsHaveField",
                test_vector_count_with_deletions_all_docs_have_field,
            ),
        ],
    )
}

fn fe(field: &str) -> Arc<dyn Query> {
    Arc::new(FieldExistsQuery::new(field))
}

fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(field, text)))
}

/// `new StringField(name, value, Store.NO)`.
fn string_field(name: &str, value: &str) -> Result<Box<StringField>> {
    Ok(Box::new(StringField::new(
        name,
        value.to_string(),
        Store::NO,
    )?))
}

/// `new TextField(name, value, Store.NO)`.
fn text_field(name: &str, value: &str) -> Result<Box<TextField>> {
    Ok(Box::new(TextField::new(
        name,
        value.to_string(),
        Store::NO,
    )?))
}

/// `new BinaryPoint("dim", new byte[4], new byte[4])`.
fn binary_point() -> Result<Box<BinaryPoint>> {
    Ok(Box::new(BinaryPoint::new(
        "dim",
        &[BytesRef::new(vec![0; 4]), BytesRef::new(vec![0; 4])],
    )?))
}

/// `query.rewrite(searcher) instanceof MatchAllDocsQuery`.
fn rewrites_to_match_all(query: &Arc<dyn Query>, searcher: &IndexSearcher) -> Result<bool> {
    let rewritten = query
        .rewrite(searcher)?
        .unwrap_or_else(|| Arc::clone(query));
    Ok(rewritten.as_any().is::<MatchAllDocsQuery>())
}

/// `new RandomIndexWriter(random(), dir)`.
fn riw(dir: &TestDirectory) -> Result<RandomIndexWriter> {
    RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())
}

fn test_doc_values_rewrite_with_terms_present() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = riw(&dir)?;
    let num_docs = LuceneTestCase::at_least(100);
    for _ in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(DoubleDocValuesField::new("f", 2.0)));
        doc.add(string_field(
            "f",
            if random().next_boolean() { "yes" } else { "no" },
        )?);
        iw.add_document(&doc)?;
    }
    iw.commit()?;
    let reader = iw.get_reader()?;
    iw.close()?;

    Assert::assert_true(rewrites_to_match_all(
        &fe("f"),
        &LuceneTestCase::new_searcher(Arc::clone(&reader)),
    )?);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_doc_values_rewrite_with_point_values_present() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = riw(&dir)?;
    let num_docs = LuceneTestCase::at_least(100);
    for _ in 0..num_docs {
        let mut doc = Document::new();
        doc.add(binary_point()?);
        doc.add(Box::new(DoubleDocValuesField::new("dim", 2.0)));
        iw.add_document(&doc)?;
    }
    iw.commit()?;
    let reader = iw.get_reader()?;
    iw.close()?;

    Assert::assert_true(rewrites_to_match_all(
        &fe("dim"),
        &LuceneTestCase::new_searcher(Arc::clone(&reader)),
    )?);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_doc_values_rewrite_with_doc_values_skipper_present() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = riw(&dir)?;
    let num_docs = LuceneTestCase::at_least(100);
    for _ in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(NumericDocValuesField::indexed_field("dim", 2)));
        iw.add_document(&doc)?;
    }
    iw.commit()?;
    let reader = iw.get_reader()?;
    iw.close()?;

    let query = fe("dim");
    let rewritten = query
        .rewrite(&LuceneTestCase::new_searcher(Arc::clone(&reader)))?
        .unwrap_or_else(|| Arc::clone(&query));
    crate::core_search::test_boost_query::assert_query_equals(
        &(Arc::new(MatchAllDocsQuery::instance()) as Arc<dyn Query>),
        &rewritten,
    );
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_doc_values_no_rewrite() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = riw(&dir)?;
    let num_docs = LuceneTestCase::at_least(100);
    for _ in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(DoubleDocValuesField::new("dim", 2.0)));
        doc.add(binary_point()?);
        iw.add_document(&doc)?;
    }
    for _ in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(DoubleDocValuesField::new("f", 2.0)));
        doc.add(string_field(
            "f",
            if random().next_boolean() { "yes" } else { "no" },
        )?);
        iw.add_document(&doc)?;
    }
    iw.commit()?;
    let reader = iw.get_reader()?;
    iw.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    Assert::assert_false(rewrites_to_match_all(&fe("dim"), &searcher)?);
    Assert::assert_false(rewrites_to_match_all(&fe("f"), &searcher)?);
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_doc_values_no_rewrite_with_doc_values() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = riw(&dir)?;
    let num_docs = LuceneTestCase::at_least(100);
    for _ in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(NumericDocValuesField::new("dv1", 1)));
        doc.add(Box::new(SortedNumericDocValuesField::new("dv2", 1)));
        doc.add(Box::new(SortedNumericDocValuesField::new("dv2", 2)));
        iw.add_document(&doc)?;
    }
    iw.commit()?;
    let reader = iw.get_reader()?;
    iw.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    Assert::assert_false(rewrites_to_match_all(&fe("dv1"), &searcher)?);
    Assert::assert_false(rewrites_to_match_all(&fe("dv2"), &searcher)?);
    Assert::assert_false(rewrites_to_match_all(&fe("dv3"), &searcher)?);
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// The index the `testDocValues{Random,Approximation,Score}` iterations
/// build: the writer has been closed and the searcher opened.
fn doc_values_index(dir: &TestDirectory) -> Result<(Arc<dyn DirectoryReader>, IndexSearcher)> {
    let iw = riw(dir)?;
    let num_docs = LuceneTestCase::at_least(100);
    for _ in 0..num_docs {
        let mut doc = Document::new();
        let has_value = random().next_boolean();
        if has_value {
            doc.add(Box::new(NumericDocValuesField::new("dv1", 1)));
            doc.add(Box::new(SortedNumericDocValuesField::new("dv2", 1)));
            doc.add(Box::new(SortedNumericDocValuesField::new("dv2", 2)));
            doc.add(string_field("has_value", "yes")?);
        }
        doc.add(string_field(
            "f",
            if random().next_boolean() { "yes" } else { "no" },
        )?);
        iw.add_document(&doc)?;
    }
    if random().next_boolean() {
        iw.delete_documents_by_query(tq("f", "no"))?;
    }
    iw.commit()?;
    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;
    Ok((reader, searcher))
}

fn test_doc_values_random() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _ in 0..iters {
        let mut dir = LuceneTestCase::new_directory();
        let (reader, searcher) = doc_values_index(&dir)?;

        assert_same_matches(&searcher, tq("has_value", "yes"), fe("dv1"), false)?;
        assert_same_matches(&searcher, tq("has_value", "yes"), fe("dv2"), false)?;

        drop(searcher);
        reader.close()?;
        dir.close()?;
    }
    Ok(())
}

fn test_doc_values_approximation() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _ in 0..iters {
        let mut dir = LuceneTestCase::new_directory();
        let (reader, searcher) = doc_values_index(&dir)?;

        let mut r#ref = BooleanQueryBuilder::new();
        r#ref.add(tq("f", "yes"), Occur::MUST)?;
        r#ref.add(tq("has_value", "yes"), Occur::FILTER)?;

        let mut bq1 = BooleanQueryBuilder::new();
        bq1.add(tq("f", "yes"), Occur::MUST)?;
        bq1.add(fe("dv1"), Occur::FILTER)?;
        assert_same_matches(
            &searcher,
            Arc::new(r#ref.build()),
            Arc::new(bq1.build()),
            true,
        )?;

        let mut bq2 = BooleanQueryBuilder::new();
        bq2.add(tq("f", "yes"), Occur::MUST)?;
        bq2.add(fe("dv2"), Occur::FILTER)?;
        assert_same_matches(
            &searcher,
            Arc::new(r#ref.build()),
            Arc::new(bq2.build()),
            true,
        )?;

        drop(searcher);
        reader.close()?;
        dir.close()?;
    }
    Ok(())
}

/// `new BoostQuery(new ConstantScoreQuery(new TermQuery(new Term("has_value",
/// "yes"))), boost)`.
fn boosted_has_value(boost: f32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(BoostQuery::new(
        Arc::new(ConstantScoreQuery::new(tq("has_value", "yes"))),
        boost,
    )?))
}

fn test_doc_values_score() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _ in 0..iters {
        let mut dir = LuceneTestCase::new_directory();
        let (reader, searcher) = doc_values_index(&dir)?;

        let boost = random().next_float() * 10.0;
        let r#ref = boosted_has_value(boost)?;

        let q1: Arc<dyn Query> = Arc::new(BoostQuery::new(fe("dv1"), boost)?);
        assert_same_matches(&searcher, Arc::clone(&r#ref), q1, true)?;

        let q2: Arc<dyn Query> = Arc::new(BoostQuery::new(fe("dv2"), boost)?);
        assert_same_matches(&searcher, Arc::clone(&r#ref), q2, true)?;

        drop(searcher);
        reader.close()?;
        dir.close()?;
    }
    Ok(())
}

/// The body of the `*MissingField` tests: one empty document.
fn missing_field_count() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = riw(&dir)?;
    iw.add_document(&Document::new())?;
    iw.commit()?;
    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;
    assert_equals(&0, &searcher.count(fe("f"))?);
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_doc_values_missing_field() -> Result<()> {
    missing_field_count()
}

fn test_doc_values_all_docs_have_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = riw(&dir)?;
    let mut doc = Document::new();
    doc.add(Box::new(NumericDocValuesField::new("f", 1)));
    iw.add_document(&doc)?;
    iw.commit()?;
    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;
    assert_equals(&1, &searcher.count(fe("f"))?);
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_doc_values_field_exists_but_no_docs_have_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = riw(&dir)?;
    // 1st segment has the field, but 2nd one does not
    let mut doc = Document::new();
    doc.add(Box::new(NumericDocValuesField::new("f", 1)));
    iw.add_document(&doc)?;
    iw.commit()?;
    iw.add_document(&Document::new())?;
    iw.commit()?;
    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;
    assert_equals(&1, &searcher.count(fe("f"))?);
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_doc_values_query_matches_count() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = riw(&dir)?;

    let random_num_docs = TestUtil::next_int(&*random(), 11, 100);
    let mut num_matching_docs = 0;

    for i in 0..random_num_docs {
        let mut doc = Document::new();
        // We select most documents randomly but keep two documents:
        //  * #0 ensures we will delete at least one document (with long between 0 and 9)
        //  * #10 ensures we will keep at least one document (with long greater than 9)
        if i == 0 || i == 10 || random().next_boolean() {
            doc.add(Box::new(LongPoint::new("long", &[i64::from(i)])?));
            doc.add(Box::new(NumericDocValuesField::new("long", i64::from(i))));
            doc.add(string_field("string", "value")?);
            doc.add(Box::new(SortedDocValuesField::new(
                "string",
                BytesRef::from_char_sequence("value"),
            )));
            num_matching_docs += 1;
        }
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;

    let reader = w.get_reader()?;
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;

    assert_same_count(&reader, &searcher, "long", num_matching_docs)?;
    assert_same_count(&reader, &searcher, "string", num_matching_docs)?;
    assert_same_count(&reader, &searcher, "doesNotExist", 0)?;

    // Test that we can't count in O(1) when there are deleted documents
    w.w()
        .get_config()
        .set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    w.delete_documents_by_query(LongPoint::new_range_query("long", &[0], &[9])?)?;
    let reader2 = w.get_reader()?;
    let searcher2 = IndexSearcher::new(Arc::clone(&reader2) as Arc<dyn IndexReader>)?;
    let test_query = fe("long");
    let weight2 = searcher2.create_weight(test_query, ScoreMode::COMPLETE, 1.0)?;
    let leaves2 = (Arc::clone(&reader2) as Arc<dyn IndexReader>).leaves()?;
    assert_equals(&weight2.count(&leaves2[0])?, &-1);

    drop((weight2, searcher, searcher2));
    reader.close()?;
    reader2.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// The index the `testNorms{Random,Approximation,Score}` iterations build.
fn norms_index(dir: &TestDirectory) -> Result<(Arc<dyn DirectoryReader>, IndexSearcher)> {
    let iw = riw(dir)?;
    let num_docs = LuceneTestCase::at_least(100);
    for _ in 0..num_docs {
        let mut doc = Document::new();
        let has_value = random().next_boolean();
        if has_value {
            doc.add(text_field("text1", "value")?);
            doc.add(string_field("has_value", "yes")?);
        }
        doc.add(string_field(
            "f",
            if random().next_boolean() { "yes" } else { "no" },
        )?);
        iw.add_document(&doc)?;
    }
    if random().next_boolean() {
        iw.delete_documents_by_query(tq("f", "no"))?;
    }
    iw.commit()?;
    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;
    Ok((reader, searcher))
}

fn test_norms_random() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _ in 0..iters {
        let mut dir = LuceneTestCase::new_directory();
        let (reader, searcher) = norms_index(&dir)?;

        assert_same_matches(&searcher, tq("has_value", "yes"), fe("text1"), false)?;

        drop(searcher);
        reader.close()?;
        dir.close()?;
    }
    Ok(())
}

fn test_norms_approximation() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _ in 0..iters {
        let mut dir = LuceneTestCase::new_directory();
        let (reader, searcher) = norms_index(&dir)?;

        let mut r#ref = BooleanQueryBuilder::new();
        r#ref.add(tq("f", "yes"), Occur::MUST)?;
        r#ref.add(tq("has_value", "yes"), Occur::FILTER)?;

        let mut bq1 = BooleanQueryBuilder::new();
        bq1.add(tq("f", "yes"), Occur::MUST)?;
        bq1.add(fe("text1"), Occur::FILTER)?;
        assert_same_matches(
            &searcher,
            Arc::new(r#ref.build()),
            Arc::new(bq1.build()),
            true,
        )?;

        drop(searcher);
        reader.close()?;
        dir.close()?;
    }
    Ok(())
}

fn test_norms_score() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _ in 0..iters {
        let mut dir = LuceneTestCase::new_directory();
        let (reader, searcher) = norms_index(&dir)?;

        let boost = random().next_float() * 10.0;
        let r#ref = boosted_has_value(boost)?;

        let q1: Arc<dyn Query> = Arc::new(BoostQuery::new(fe("text1"), boost)?);
        assert_same_matches(&searcher, r#ref, q1, true)?;

        drop(searcher);
        reader.close()?;
        dir.close()?;
    }
    Ok(())
}

fn test_norms_missing_field() -> Result<()> {
    missing_field_count()
}

fn test_norms_all_docs_have_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = riw(&dir)?;
    let mut doc = Document::new();
    doc.add(text_field("f", "value")?);
    iw.add_document(&doc)?;
    iw.commit()?;
    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;
    assert_equals(&1, &searcher.count(fe("f"))?);
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_norms_field_exists_but_no_docs_have_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = riw(&dir)?;
    // 1st segment has the field, but 2nd one does not
    let mut doc = Document::new();
    doc.add(text_field("f", "value")?);
    iw.add_document(&doc)?;
    iw.commit()?;
    iw.add_document(&Document::new())?;
    iw.commit()?;
    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;
    assert_equals(&1, &searcher.count(fe("f"))?);
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_norms_query_matches_count() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = riw(&dir)?;

    let random_num_docs = TestUtil::next_int(&*random(), 10, 100);

    let no_norms_field_type = FieldType::new();
    no_norms_field_type.set_omit_norms(true)?;
    no_norms_field_type.set_index_options(IndexOptions::DOCS)?;

    let mut doc = Document::new();
    doc.add(text_field("text", "always here")?);
    doc.add(text_field("text_s", "")?);
    doc.add(Box::new(Field::new(
        "text_n",
        "always here".to_string(),
        no_norms_field_type.clone(),
    )?));
    w.add_document(&doc)?;

    for _ in 1..random_num_docs {
        doc.clear();
        doc.add(text_field("text", "some text")?);
        doc.add(text_field("text_s", "some text")?);
        doc.add(Box::new(Field::new(
            "text_n",
            "some here".to_string(),
            no_norms_field_type.clone(),
        )?));
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;

    let reader = w.get_reader()?;
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;

    assert_norms_count_with_shortcut(&searcher, "text", random_num_docs)?;
    assert_norms_count_with_shortcut(&searcher, "doesNotExist", 0)?;
    expect_throws("java.lang.IllegalStateException", || {
        searcher.count(fe("text_n"))
    });

    // docs that have a text field that analyzes to an empty token
    // stream still have a recorded norm value but don't show up in
    // Reader.getDocCount(field), so we can't use the shortcut for
    // these fields
    assert_norms_count_without_shortcut(&searcher, "text_s", random_num_docs)?;

    // We can still shortcut with deleted docs
    w.w()
        .get_config()
        .set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    w.delete_documents(term("text", "text"))?; // deletes all but the first doc
    let reader2 = w.get_reader()?;
    let searcher2 = IndexSearcher::new(Arc::clone(&reader2) as Arc<dyn IndexReader>)?;
    assert_norms_count_with_shortcut(&searcher2, "text", 1)?;

    drop((searcher, searcher2));
    reader.close()?;
    reader2.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

fn assert_norms_count_without_shortcut(
    searcher: &IndexSearcher,
    field: &str,
    expected_count: i32,
) -> Result<()> {
    let q = fe(field);
    let weight = searcher.create_weight(Arc::clone(&q), ScoreMode::COMPLETE, 1.0)?;
    let leaves = Arc::clone(searcher.get_index_reader()).leaves()?;
    assert_equals(&-1, &weight.count(&leaves[0])?);
    assert_equals(&expected_count, &searcher.count(q)?);
    Ok(())
}

fn assert_norms_count_with_shortcut(
    searcher: &IndexSearcher,
    field: &str,
    num_matching_docs: i32,
) -> Result<()> {
    let test_query = fe(field);
    assert_equals(
        &num_matching_docs,
        &searcher.count(Arc::clone(&test_query))?,
    );
    let weight = searcher.create_weight(test_query, ScoreMode::COMPLETE, 1.0)?;
    let leaves = Arc::clone(searcher.get_index_reader()).leaves()?;
    assert_equals(&num_matching_docs, &weight.count(&leaves[0])?);
    Ok(())
}

/// `new KnnFloatVectorField(name, vector)`.
fn knn(name: &str, vector: &[f32]) -> Result<Box<KnnFloatVectorField>> {
    Ok(Box::new(KnnFloatVectorField::with_euclidean(name, vector)?))
}

fn test_knn_vector_random() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _ in 0..iters {
        let mut dir = LuceneTestCase::new_directory();
        {
            let iw = riw(&dir)?;
            let num_docs = LuceneTestCase::at_least(100);
            for _ in 0..num_docs {
                let mut doc = Document::new();
                let has_value = random().next_boolean();
                if has_value {
                    doc.add(knn("vector", &random_vector(5)?)?);
                    doc.add(string_field("has_value", "yes")?);
                }
                doc.add(string_field("field", "value")?);
                iw.add_document(&doc)?;
            }
            if random().next_boolean() {
                iw.delete_documents_by_query(tq("f", "no"))?;
            }
            iw.commit()?;

            {
                let reader = iw.get_reader()?;
                let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

                assert_same_matches(&searcher, tq("has_value", "yes"), fe("vector"), false)?;

                let boost = random().next_float() * 10.0;
                assert_same_matches(
                    &searcher,
                    boosted_has_value(boost)?,
                    Arc::new(BoostQuery::new(fe("vector"), boost)?),
                    true,
                )?;
                drop(searcher);
                reader.close()?;
            }
            iw.close()?;
        }
        dir.close()?;
    }
    Ok(())
}

fn test_knn_vector_missing_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    {
        let iw = riw(&dir)?;
        iw.add_document(&Document::new())?;
        iw.commit()?;
        {
            let reader = iw.get_reader()?;
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
            assert_equals(&0, &searcher.count(fe("f"))?);
            drop(searcher);
            reader.close()?;
        }
        iw.close()?;
    }
    dir.close()?;
    Ok(())
}

fn test_knn_vector_all_docs_have_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    {
        let iw = riw(&dir)?;
        for _ in 0..100 {
            let mut doc = Document::new();
            doc.add(knn("vector", &random_vector(5)?)?);
            iw.add_document(&doc)?;
        }
        iw.commit()?;

        {
            let reader = iw.get_reader()?;
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
            let query = fe("vector");
            Assert::assert_true(
                searcher
                    .rewrite(Arc::clone(&query))?
                    .as_any()
                    .is::<MatchAllDocsQuery>(),
            );
            assert_equals(&100, &searcher.count(query)?);
            drop(searcher);
            reader.close()?;
        }
        iw.close()?;
    }
    dir.close()?;
    Ok(())
}

fn test_delete_knn_vector() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    {
        let iw = riw(&dir)?;
        let num_docs = LuceneTestCase::at_least(100);

        let all_docs_have_vector = random().next_boolean();
        let mut doc_with_vector = FixedBitSet::new(num_docs as usize);
        for i in 0..num_docs {
            let mut doc = Document::new();
            if all_docs_have_vector || random().next_boolean() {
                doc.add(knn("vector", &random_vector(5)?)?);
                doc_with_vector.set(i as usize);
            }
            doc.add(string_field("id", &i.to_string())?);
            iw.add_document(&doc)?;
        }
        if random().next_boolean() {
            let num_deleted = random().next_int_bound(num_docs) + 1;
            for i in 0..num_deleted {
                iw.delete_documents(term("id", &i.to_string()))?;
                doc_with_vector.clear(i as usize);
            }
        }

        {
            let reader = iw.get_reader()?;
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

            let count = searcher.count(fe("vector"))?;
            assert_equals(&(BitSet::cardinality(&doc_with_vector) as i32), &count);
            drop(searcher);
            reader.close()?;
        }
        iw.close()?;
    }
    dir.close()?;
    Ok(())
}

fn test_knn_vector_conjunction() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    {
        let iw = riw(&dir)?;
        let num_docs = LuceneTestCase::at_least(100);
        let mut num_vectors = 0;

        let all_docs_have_vector = random().next_boolean();
        for i in 0..num_docs {
            let mut doc = Document::new();
            if all_docs_have_vector || random().next_boolean() {
                doc.add(knn("vector", &random_vector(5)?)?);
                num_vectors += 1;
            }
            doc.add(string_field("field", &format!("value{}", i % 2))?);
            iw.add_document(&doc)?;
        }
        {
            let reader = iw.get_reader()?;
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
            let occur = if random().next_boolean() {
                Occur::MUST
            } else {
                Occur::FILTER
            };
            let mut b = BooleanQueryBuilder::new();
            b.add(tq("field", "value1"), occur)?;
            b.add(fe("vector"), Occur::FILTER)?;
            let boolean_query = b.build();

            let count = searcher.count(Arc::new(boolean_query))?;
            Assert::assert_true(count <= num_vectors);
            if all_docs_have_vector {
                assert_equals(&(num_docs / 2), &count);
            }
            drop(searcher);
            reader.close()?;
        }
        iw.close()?;
    }
    dir.close()?;
    Ok(())
}

fn test_knn_vector_field_exists_but_no_docs_have_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    {
        let iw = riw(&dir)?;
        // 1st segment has the field, but 2nd one does not
        let mut doc = Document::new();
        doc.add(knn("vector", &random_vector(3)?)?);
        iw.add_document(&doc)?;
        iw.commit()?;
        iw.add_document(&Document::new())?;
        iw.commit()?;
        {
            let reader = iw.get_reader()?;
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
            assert_equals(&1, &searcher.count(fe("vector"))?);
            drop(searcher);
            reader.close()?;
        }
        iw.close()?;
    }
    dir.close()?;
    Ok(())
}

fn random_vector(dim: usize) -> Result<Vec<f32>> {
    let mut v = vec![0f32; dim];
    for x in v.iter_mut() {
        *x = random().next_float();
    }
    vector_util::l2normalize(&mut v, true)?;
    Ok(v)
}

fn test_delete_doc_values() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    {
        let iw = riw(&dir)?;
        let num_docs = LuceneTestCase::at_least(100);

        let all_docs_have_value = random().next_boolean();
        let mut doc_with_value = FixedBitSet::new(num_docs as usize);
        for i in 0..num_docs {
            let mut doc = Document::new();
            if all_docs_have_value || random().next_boolean() {
                doc.add(Box::new(NumericDocValuesField::indexed_field(
                    "num",
                    i64::from(i),
                )));
                doc_with_value.set(i as usize);
            }
            doc.add(string_field("id", &i.to_string())?);
            iw.add_document(&doc)?;
        }

        if random().next_boolean() {
            let num_deleted = random().next_int_bound(num_docs) + 1;
            for _ in 0..num_deleted {
                let id = random().next_int_bound(num_docs);
                iw.delete_documents(term("id", &id.to_string()))?;
                doc_with_value.clear(id as usize);
            }
        }

        {
            let reader = iw.get_reader()?;
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

            let count = searcher.count(fe("num"))?;
            assert_equals(&(BitSet::cardinality(&doc_with_value) as i32), &count);
            drop(searcher);
            reader.close()?;
        }
        iw.close()?;
    }
    dir.close()?;
    Ok(())
}

fn test_delete_all_point_docs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    {
        let iw = riw(&dir)?;

        let mut doc = Document::new();
        doc.add(string_field("id", "0")?);
        doc.add(Box::new(LongPoint::new("long", &[17])?));
        doc.add(Box::new(NumericDocValuesField::new("long", 17)));
        iw.add_document(&doc)?;
        // add another document before the flush, otherwise the segment only has the document that
        // we are going to delete and the merge simply ignores the segment without carrying over its
        // field infos
        iw.add_document(&Document::new())?;
        // make sure there are two segments or force merge will be a no-op
        iw.flush()?;
        iw.add_document(&Document::new())?;
        iw.commit()?;

        iw.delete_documents(term("id", "0"))?;
        iw.force_merge(1)?;

        {
            let reader = iw.get_reader()?;
            let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
            Assert::assert_true(leaves.len() == 1 && !reader.has_deletions());
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
            assert_equals(&0, &searcher.count(fe("long"))?);
            drop(searcher);
            reader.close()?;
        }
        iw.close()?;
    }
    dir.close()?;
    Ok(())
}

fn test_delete_all_term_docs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    {
        let iw = riw(&dir)?;

        let mut doc = Document::new();
        doc.add(string_field("id", "0")?);
        doc.add(string_field("str", "foo")?);
        doc.add(Box::new(SortedDocValuesField::new(
            "str",
            BytesRef::from_char_sequence("foo"),
        )));
        iw.add_document(&doc)?;
        // add another document before the flush, otherwise the segment only has the document that
        // we are going to delete and the merge simply ignores the segment without carrying over its
        // field infos
        iw.add_document(&Document::new())?;
        // make sure there are two segments or force merge will be a no-op
        iw.flush()?;
        iw.add_document(&Document::new())?;
        iw.commit()?;

        iw.delete_documents(term("id", "0"))?;
        iw.force_merge(1)?;

        {
            let reader = iw.get_reader()?;
            let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
            Assert::assert_true(leaves.len() == 1 && !reader.has_deletions());
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
            assert_equals(&0, &searcher.count(fe("str"))?);
            drop(searcher);
            reader.close()?;
        }
        iw.close()?;
    }
    dir.close()?;
    Ok(())
}

fn assert_same_matches(
    searcher: &IndexSearcher,
    q1: Arc<dyn Query>,
    q2: Arc<dyn Query>,
    scores: bool,
) -> Result<()> {
    let max_doc = searcher.get_index_reader().max_doc();
    // `scores ? Sort.RELEVANCE : Sort.INDEXORDER`.
    let sort = if scores {
        Sort::new()
    } else {
        Sort::new_fields(vec![SortField::FIELD_DOC.clone()])?
    };
    let td1 = searcher.search_with_sort(q1, max_doc, &sort)?;
    let td2 = searcher.search_with_sort(q2, max_doc, &sort)?;
    assert_equals(&td1.total_hits.value(), &td2.total_hits.value());
    let (sd1, sd2) = (
        score_docs_of(&td1.score_docs),
        score_docs_of(&td2.score_docs),
    );
    for i in 0..sd1.len() {
        assert_equals(&sd1[i].doc, &sd2[i].doc);
        if scores {
            assert_equals_delta(f64::from(sd1[i].score), f64::from(sd2[i].score), 10e-7);
        }
    }
    Ok(())
}

fn assert_same_count(
    reader: &Arc<dyn DirectoryReader>,
    searcher: &IndexSearcher,
    field: &str,
    num_matching_docs: i32,
) -> Result<()> {
    let test_query = fe(field);
    assert_equals(
        &searcher.count(Arc::clone(&test_query))?,
        &num_matching_docs,
    );
    let weight = searcher.create_weight(test_query, ScoreMode::COMPLETE, 1.0)?;
    let leaves = (Arc::clone(reader) as Arc<dyn IndexReader>).leaves()?;
    assert_equals(&weight.count(&leaves[0])?, &num_matching_docs);
    Ok(())
}

/// When every document in a segment has a doc values field and some
/// documents are deleted, count() should still return the number of live
/// docs in O(1) instead of falling back to iteration.
fn test_doc_values_count_with_deletions_all_docs_have_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = riw(&dir)?;
    let num_docs = LuceneTestCase::at_least(20);
    for i in 0..num_docs {
        let mut doc = Document::new();
        // Use indexedField to ensure a DocValuesSkipper is available, which provides
        // an exact doc count for doc values that we can use even with deletions.
        doc.add(Box::new(NumericDocValuesField::indexed_field(
            "dv",
            i64::from(i),
        )));
        doc.add(string_field("id", &i.to_string())?);
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    // Delete some documents, but every remaining doc still has the field
    w.w()
        .get_config()
        .set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    w.delete_documents(term("id", "0"))?;
    w.delete_documents(term("id", "1"))?;

    let reader = w.get_reader()?;
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    Assert::assert_true_msg("Expected deletions in the segment", reader.has_deletions());
    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
    assert_equals(&1, &leaves.len());

    let query = fe("dv");
    let weight = searcher.create_weight(query, ScoreMode::COMPLETE, 1.0)?;
    // All docs had the field, so even with deletions we can count in O(1)
    let count = weight.count(&leaves[0])?;
    assert_equals(&reader.num_docs(), &count);
    assert_equals(&(num_docs - 2), &count);

    drop((weight, searcher, leaves));
    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// When every document in a segment has a vector field and some documents
/// are deleted, count() should still return the number of live docs in O(1)
/// instead of falling back to iteration.
fn test_vector_count_with_deletions_all_docs_have_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = riw(&dir)?;
    let num_docs = LuceneTestCase::at_least(20);
    for i in 0..num_docs {
        let mut doc = Document::new();
        let mut vector = [
            (i + 1) as f32,
            (i + 2) as f32,
            (i + 3) as f32,
            (i + 4) as f32,
        ];
        vector_util::l2normalize(&mut vector, true)?;
        doc.add(knn("vector", &vector)?);
        doc.add(string_field("id", &i.to_string())?);
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    w.w()
        .get_config()
        .set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    w.delete_documents(term("id", "0"))?;

    let reader = w.get_reader()?;
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    Assert::assert_true_msg("Expected deletions in the segment", reader.has_deletions());
    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
    assert_equals(&1, &leaves.len());

    let query = fe("vector");
    let weight = searcher.create_weight(query, ScoreMode::COMPLETE, 1.0)?;
    // All docs had the vector field, so even with deletions we can count in O(1)
    let count = weight.count(&leaves[0])?;
    assert_equals(&reader.num_docs(), &count);
    assert_equals(&(num_docs - 1), &count);

    drop((weight, searcher, leaves));
    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}
