//! Port of `lucene/core/src/test/org/apache/lucene/search/TestQueryRescorer.java`.
//!
//! Java's anonymous `QueryRescorer` subclasses override `combine`; each is a
//! [`QueryRescorerImpl`] whose `combiner_name` is the anonymous class's name
//! (`TestQueryRescorer$1`, `$2`, `$3`, in source order), which `explain`
//! renders as Java renders `getClass()`.

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericDocValuesField, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::LeafReaderContext;
use rucene::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query_rescorer::{self, QueryRescorer, QueryRescorerImpl};
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::rescorer::Rescorer;
use rucene::search::scorable::Scorable;
use rucene::search::scorer::Scorer;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::{BM25Similarity, ClassicSimilarity, Explanation};
use rucene::search::weight::{DefaultScorerSupplier, Weight};
use rucene::search::{BooleanQueryBuilder, Occur, Query, ScoreMode, TopDocs};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::tq;
use crate::core_support::{assert_equals, assert_equals_delta, class, random, test};

/// `TestQueryRescorer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestQueryRescorer",
        vec![
            test(
                "testRescoreOfASubsetOfHits",
                test_rescore_of_a_subset_of_hits,
            ),
            test("testRescoreIsIdempotent", test_rescore_is_idempotent),
            test("testBasic", test_basic),
            test("testNullScorerTermQuery", test_null_scorer_term_query),
            test("testCustomCombine", test_custom_combine),
            test("testExplain", test_explain),
            test("testMissingSecondPassScore", test_missing_second_pass_score),
            test("testRandom", test_random),
        ],
    )
}

fn get_searcher(r: &Arc<dyn DirectoryReader>) -> IndexSearcher {
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(r));
    // We rely on more tokens = lower score:
    searcher.set_similarity(Arc::new(ClassicSimilarity::new()));
    searcher
}

/// `public static IndexWriterConfig newIndexWriterConfig()`.
fn new_index_writer_config() -> Result<IndexWriterConfig> {
    // We rely on more tokens = lower score:
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_similarity(Some(Arc::new(ClassicSimilarity::new())))?;
    Ok(iwc)
}

const DICTIONARY: [&str; 7] = ["river", "quick", "brown", "fox", "jumped", "lazy", "fence"];

fn random_sentence() -> String {
    let length = random().next_int_bound(10);
    let mut sentence = format!("{} ", DICTIONARY[0]);
    for _ in 0..length {
        sentence.push_str(&format!(
            "{} ",
            DICTIONARY[random().next_int_bound(DICTIONARY.len() as i32 - 1) as usize]
        ));
    }
    sentence
}

fn publish_docs(
    num_docs: i32,
    field_name: &str,
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<Arc<dyn DirectoryReader>> {
    let w = RandomIndexWriter::new_with_config(
        &*random(),
        dir.as_directory(),
        new_index_writer_config()?,
    )?;
    for i in 0..num_docs {
        let mut d = Document::new();
        d.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::YES,
        )));
        d.add(Box::new(LuceneTestCase::new_text_field(
            field_name,
            &random_sentence(),
            Store::NO,
        )));
        w.add_document(&d)?;
    }

    let reader = w.get_reader()?;
    w.close()?;
    Ok(reader)
}

/// Port of `TestQueryRescorer.testRescoreOfASubsetOfHits`.
fn test_rescore_of_a_subset_of_hits() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let num_docs = 100;
    let field_name = "field";
    let reader = publish_docs(num_docs, field_name, &dir)?;

    // Construct a query that will get numDocs hits.
    let word_one = DICTIONARY[0];
    let term_query = tq(field_name, word_one);
    let mut searcher = get_searcher(&reader);
    searcher.set_similarity(Arc::new(BM25Similarity::new()));
    let hits = searcher.search(term_query, num_docs)?;

    // Next, use a more specific phrase query that will return different scores
    // from the above term query
    let word_two = *RandomPicks::random_from(&*random(), &DICTIONARY);
    let phrase_query: Arc<dyn Query> = Arc::new(PhraseQuery::with_slop(
        1,
        field_name,
        &[word_one, word_two],
    )?);

    // rescore, requesting a smaller topN
    let top_n = random().next_int_bound(num_docs - 1);
    let phrase_query_hits = query_rescorer::rescore(&searcher, &hits, phrase_query, 2.0, top_n)?;
    assert_equals(&(top_n as usize), &phrase_query_hits.score_docs.len());

    for i in 1..phrase_query_hits.score_docs.len() {
        Assert::assert_true(
            phrase_query_hits.score_docs[i].score <= phrase_query_hits.score_docs[i - 1].score,
        );
    }
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestQueryRescorer.testRescoreIsIdempotent`.
fn test_rescore_is_idempotent() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let num_docs = 100;
    let field_name = "field";
    let reader = publish_docs(num_docs, field_name, &dir)?;

    // Construct a query that will get numDocs hits.
    let word_one = DICTIONARY[0];
    let term_query = tq(field_name, word_one);
    let mut searcher = get_searcher(&reader);
    searcher.set_similarity(Arc::new(BM25Similarity::new()));
    let hits1 = searcher.search(Arc::clone(&term_query), num_docs)?;
    let hits2 = searcher.search(term_query, num_docs)?;

    // Next, use a more specific phrase query that will return different scores
    // from the above term query
    let word_two = *RandomPicks::random_from(&*random(), &DICTIONARY);
    let phrase_query: Arc<dyn Query> = Arc::new(PhraseQuery::with_slop(
        1,
        field_name,
        &[word_one, word_two],
    )?);

    // rescore, requesting the same hits as topN
    let top_n = num_docs;
    let first_rescore_hits =
        query_rescorer::rescore(&searcher, &hits1, Arc::clone(&phrase_query), 2.0, top_n)?;

    // now rescore again, where topN is less than numDocs
    let top_n = random().next_int_bound(num_docs - 1);
    let second_rescore_hits =
        query_rescorer::rescore(&searcher, &hits2, Arc::clone(&phrase_query), 2.0, top_n)?
            .score_docs;
    let expected_top_n_score_docs = &first_rescore_hits.score_docs[0..top_n as usize];
    CheckHits::check_equal(
        &*phrase_query,
        expected_top_n_score_docs,
        &second_rescore_hits,
    );

    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// The two-document index the fixed-order tests share.
fn wizard_index(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<Arc<dyn DirectoryReader>> {
    let w = RandomIndexWriter::new_with_config(
        &*random(),
        dir.as_directory(),
        new_index_writer_config()?,
    )?;

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "0",
        Store::YES,
    )));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "wizard the the the the the oz",
        Store::NO,
    )));
    w.add_document(&doc)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "1",
        Store::YES,
    )));
    // 1 extra token, but wizard and oz are close;
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "wizard oz the the the the the the",
        Store::NO,
    )));
    w.add_document(&doc)?;
    let r = w.get_reader()?;
    w.close()?;
    Ok(r)
}

/// `+field:wizard +field:oz` as SHOULD clauses.
fn wizard_or_oz() -> Result<Arc<dyn Query>> {
    let mut bq = BooleanQueryBuilder::new();
    bq.add(tq("field", "wizard"), Occur::SHOULD)?;
    bq.add(tq("field", "oz"), Occur::SHOULD)?;
    Ok(Arc::new(bq.build()))
}

/// `searcher.storedFields().document(doc).get("id")`.
fn id_of(searcher: &IndexSearcher, doc: i32) -> Result<String> {
    Ok(searcher
        .stored_fields()?
        .document(doc)?
        .get("id")
        .unwrap_or_default())
}

/// The first-pass checks every fixed-order test makes.
fn assert_first_pass(searcher: &IndexSearcher, hits: &TopDocs) -> Result<()> {
    assert_equals(&2, &hits.total_hits.value());
    assert_equals(&"0".to_string(), &id_of(searcher, hits.score_docs[0].doc)?);
    assert_equals(&"1".to_string(), &id_of(searcher, hits.score_docs[1].doc)?);
    Ok(())
}

/// Port of `TestQueryRescorer.testBasic`.
fn test_basic() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let r = wizard_index(&dir)?;

    // Do ordinary BooleanQuery:
    let bq = wizard_or_oz()?;
    let mut searcher = get_searcher(&r);
    searcher.set_similarity(Arc::new(ClassicSimilarity::new()));

    let hits = searcher.search(bq, 10)?;
    assert_first_pass(&searcher, &hits)?;

    // Now, resort using PhraseQuery:
    let pq: Arc<dyn Query> = Arc::new(PhraseQuery::with_slop(5, "field", &["wizard", "oz"])?);

    let hits2 = query_rescorer::rescore(&searcher, &hits, pq, 2.0, 10)?;

    // Resorting changed the order:
    assert_equals(&2, &hits2.total_hits.value());
    assert_equals(
        &"1".to_string(),
        &id_of(&searcher, hits2.score_docs[0].doc)?,
    );
    assert_equals(
        &"0".to_string(),
        &id_of(&searcher, hits2.score_docs[1].doc)?,
    );

    drop(searcher);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Test LUCENE-5682
fn test_null_scorer_term_query() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let r = wizard_index(&dir)?;

    // Do ordinary BooleanQuery:
    let bq = wizard_or_oz()?;
    let mut searcher = get_searcher(&r);
    searcher.set_similarity(Arc::new(ClassicSimilarity::new()));

    let hits = searcher.search(bq, 10)?;
    assert_first_pass(&searcher, &hits)?;

    // Now, resort using TermQuery on term that does not exist.
    let tq = tq("field", "gold");
    let hits2 = query_rescorer::rescore(&searcher, &hits, tq, 2.0, 10)?;

    // Just testing that null scorer is handled.
    assert_equals(&2, &hits2.total_hits.value());

    drop(searcher);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// An anonymous `QueryRescorer` subclass: `combine` as `f`, named `name`.
struct Combiner {
    name: &'static str,
    f: fn(f32, bool, f32) -> f32,
}

impl QueryRescorerImpl for Combiner {
    fn combine(
        &self,
        first_pass_score: f32,
        second_pass_matches: bool,
        second_pass_score: f32,
    ) -> f32 {
        (self.f)(first_pass_score, second_pass_matches, second_pass_score)
    }

    fn combiner_name(&self) -> String {
        self.name.to_string()
    }
}

/// Port of `TestQueryRescorer.testCustomCombine`.
fn test_custom_combine() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let r = wizard_index(&dir)?;

    // Do ordinary BooleanQuery:
    let bq = wizard_or_oz()?;
    let searcher = get_searcher(&r);

    let hits = searcher.search(bq, 10)?;
    assert_first_pass(&searcher, &hits)?;

    // Now, resort using PhraseQuery, but with an
    // opposite-world combine:
    let pq: Arc<dyn Query> = Arc::new(PhraseQuery::with_slop(5, "field", &["wizard", "oz"])?);

    let hits2 = QueryRescorer::new(
        pq,
        Combiner {
            name: "org.apache.lucene.search.TestQueryRescorer$1",
            f: |first_pass_score, second_pass_matches, second_pass_score| {
                let mut score = first_pass_score;
                if second_pass_matches {
                    score = (score as f64 - 2.0 * second_pass_score as f64) as f32;
                }
                score
            },
        },
    )
    .rescore(&searcher, &hits, 10)?;

    // Resorting didn't change the order:
    assert_equals(&2, &hits2.total_hits.value());
    assert_equals(
        &"0".to_string(),
        &id_of(&searcher, hits2.score_docs[0].doc)?,
    );
    assert_equals(
        &"1".to_string(),
        &id_of(&searcher, hits2.score_docs[1].doc)?,
    );

    drop(searcher);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestQueryRescorer.testExplain`.
fn test_explain() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let r = wizard_index(&dir)?;

    // Do ordinary BooleanQuery:
    let bq = wizard_or_oz()?;
    let searcher = get_searcher(&r);

    let hits = searcher.search(Arc::clone(&bq), 10)?;
    assert_first_pass(&searcher, &hits)?;

    // Now, resort using PhraseQuery:
    let pq: Arc<dyn Query> = Arc::new(PhraseQuery::from_strings("field", &["wizard", "oz"])?);

    let rescorer = QueryRescorer::new(
        pq,
        Combiner {
            name: "org.apache.lucene.search.TestQueryRescorer$2",
            f: |first_pass_score, second_pass_matches, second_pass_score| {
                let mut score = first_pass_score;
                if second_pass_matches {
                    score = (score as f64 + 2.0 * second_pass_score as f64) as f32;
                }
                score
            },
        },
    );

    let hits2 = rescorer.rescore(&searcher, &hits, 10)?;

    // Resorting changed the order:
    assert_equals(&2, &hits2.total_hits.value());
    assert_equals(
        &"1".to_string(),
        &id_of(&searcher, hits2.score_docs[0].doc)?,
    );
    assert_equals(
        &"0".to_string(),
        &id_of(&searcher, hits2.score_docs[1].doc)?,
    );

    let doc_id = hits2.score_docs[0].doc;
    let explain = rescorer.explain(
        &searcher,
        &searcher.explain(Arc::clone(&bq), doc_id)?,
        doc_id,
    )?;
    let s = explain.to_string();
    Assert::assert_true(s.contains("TestQueryRescorer$"));
    Assert::assert_true(s.contains("combined first and second pass score"));
    Assert::assert_true(s.contains("first pass score"));
    Assert::assert_true(s.contains("= second pass score"));
    assert_equals_delta(
        hits2.score_docs[0].score as f64,
        explain.value().double_value(),
        0.0,
    );

    let doc_id = hits2.score_docs[1].doc;
    let explain = rescorer.explain(
        &searcher,
        &searcher.explain(Arc::clone(&bq), doc_id)?,
        doc_id,
    )?;
    let s = explain.to_string();
    Assert::assert_true(s.contains("TestQueryRescorer$"));
    Assert::assert_true(s.contains("combined first and second pass score"));
    Assert::assert_true(s.contains("first pass score"));
    Assert::assert_true(s.contains("no second pass score"));
    Assert::assert_false(s.contains("= second pass score"));
    assert_equals_delta(
        hits2.score_docs[1].score as f64,
        explain.value().double_value(),
        0.0,
    );

    drop(searcher);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestQueryRescorer.testMissingSecondPassScore`.
fn test_missing_second_pass_score() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let r = wizard_index(&dir)?;

    // Do ordinary BooleanQuery:
    let bq = wizard_or_oz()?;
    let searcher = get_searcher(&r);

    let hits = searcher.search(bq, 10)?;
    assert_first_pass(&searcher, &hits)?;

    // Now, resort using PhraseQuery, no slop:
    let pq: Arc<dyn Query> = Arc::new(PhraseQuery::from_strings("field", &["wizard", "oz"])?);

    let hits2 = query_rescorer::rescore(&searcher, &hits, pq, 2.0, 10)?;

    // Resorting changed the order:
    assert_equals(&2, &hits2.total_hits.value());
    assert_equals(
        &"1".to_string(),
        &id_of(&searcher, hits2.score_docs[0].doc)?,
    );
    assert_equals(
        &"0".to_string(),
        &id_of(&searcher, hits2.score_docs[1].doc)?,
    );

    drop(searcher);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestQueryRescorer.testRandom`.
fn test_random() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let num_docs = LuceneTestCase::at_least(1000);
    let w = RandomIndexWriter::new_with_config(
        &*random(),
        dir.as_directory(),
        new_index_writer_config()?,
    )?;

    let mut id_to_num = vec![0i32; num_docs as usize];
    let max_value = TestUtil::next_int(&*random(), 10, 1000000);
    for i in 0..num_docs as usize {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::YES,
        )));
        let num_tokens = TestUtil::next_int(&*random(), 1, 10);
        let mut b = String::new();
        for _ in 0..num_tokens {
            b.push_str("a ");
        }
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "field",
            &b,
            Store::NO,
        )));
        id_to_num[i] = random().next_int_bound(max_value);
        doc.add(Box::new(NumericDocValuesField::new(
            "num",
            id_to_num[i] as i64,
        )));
        w.add_document(&doc)?;
    }
    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let num_hits = TestUtil::next_int(&*random(), 1, num_docs);
    let reverse = random().next_boolean();

    // System.out.println("numHits=" + numHits + " reverse=" + reverse);
    let hits = s.search(tq("field", "a"), num_hits)?;

    let id_to_num = Arc::new(id_to_num);
    let hits2 = QueryRescorer::new(
        Arc::new(FixedScoreQuery {
            id_to_num: Arc::clone(&id_to_num),
            reverse,
        }),
        Combiner {
            name: "org.apache.lucene.search.TestQueryRescorer$3",
            f: |_first_pass_score, _second_pass_matches, second_pass_score| second_pass_score,
        },
    )
    .rescore(&s, &hits, num_hits)?;

    let mut expected: Vec<i32> = (0..num_hits as usize)
        .map(|i| hits.score_docs[i].doc)
        .collect();

    let reverse_int: i32 = if reverse { -1 } else { 1 };

    let stored_fields = r.stored_fields()?;
    let value_of = |doc: i32| -> i32 {
        let id = crate::core_support::unchecked(stored_fields.document(doc))
            .get("id")
            .unwrap_or_default();
        id_to_num[id.parse::<usize>().expect("Integer.parseInt")]
    };
    expected.sort_by(|&a, &b| {
        let av = value_of(a);
        let bv = value_of(b);
        let c = if av < bv {
            -reverse_int
        } else if bv < av {
            reverse_int
        } else {
            // Tie break by docID, ascending
            a - b
        };
        c.cmp(&0)
    });

    let mut fail = false;
    for i in 0..num_hits as usize {
        // System.out.println("expected=" + expected[i] + " vs " + hits2.scoreDocs[i].doc + " v=" +
        // idToNum[Integer.parseInt(r.storedFields().document(expected[i]).get("id"))]);
        if expected[i] != hits2.score_docs[i].doc {
            // System.out.println("  diff!");
            fail = true;
        }
    }
    Assert::assert_false(fail);

    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Just assigns score == idToNum[doc("id")] for each doc.
///
/// `private static class FixedScoreQuery extends Query`.
#[derive(Debug, Clone)]
struct FixedScoreQuery {
    id_to_num: Arc<Vec<i32>>,
    reverse: bool,
}

/// The anonymous `Weight` of `FixedScoreQuery.createWeight`.
#[derive(Debug)]
struct FixedScoreWeight {
    query: FixedScoreQuery,
}

impl SegmentCacheable for FixedScoreWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl Weight for FixedScoreWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::new(self.query.clone())
    }

    /// Java returns `null`; the port's `explain` returns a value.
    fn explain(&self, _context: &LeafReaderContext, _doc: i32) -> Result<Explanation> {
        Ok(Explanation::no_match("FixedScoreQuery", Vec::new()))
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let scorer = FixedScorer {
            it: FixedIterator {
                doc_id: -1,
                max_doc: context.leaf_reader().max_doc(),
            },
            stored_fields: context.leaf_reader().stored_fields()?,
            id_to_num: Arc::clone(&self.query.id_to_num),
            reverse: self.query.reverse,
        };
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }
}

/// The iterator of the anonymous scorer, which shares the scorer's `docID`.
struct FixedIterator {
    doc_id: i32,
    max_doc: i32,
}

impl DocIdSetIterator for FixedIterator {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn cost(&self) -> i64 {
        1
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc_id += 1;
        if self.doc_id >= self.max_doc {
            return Ok(NO_MORE_DOCS);
        }
        Ok(self.doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc_id = target;
        Ok(self.doc_id)
    }
}

/// The anonymous `Scorer` of `FixedScoreQuery`'s weight.
struct FixedScorer {
    it: FixedIterator,
    /// `context.reader().storedFields()`, read when the scorer is built.
    stored_fields: Box<dyn rucene::index::StoredFields>,
    id_to_num: Arc<Vec<i32>>,
    reverse: bool,
}

impl Scorable for FixedScorer {
    fn score(&mut self) -> Result<f32> {
        let id = self
            .stored_fields
            .document(self.it.doc_id)?
            .get("id")
            .unwrap_or_default();
        let num = self.id_to_num[id.parse::<usize>().expect("Integer.parseInt")];
        if self.reverse {
            // System.out.println("score doc=" + docID + " num=" + num);
            Ok(num as f32)
        } else {
            // System.out.println("score doc=" + docID + " num=" + -num);
            Ok(1.0 / (1 + num) as f32)
        }
    }
}

impl Scorer for FixedScorer {
    fn doc_id(&self) -> i32 {
        self.it.doc_id
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.it
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(f32::INFINITY)
    }
}

impl Query for FixedScoreQuery {
    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        _score_mode: ScoreMode,
        _boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(FixedScoreWeight {
            query: self.clone(),
        }))
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn to_query_string(&self, _field: &str) -> String {
        format!(
            "FixedScoreQuery {} ids; reverse={}",
            self.id_to_num.len(),
            self.reverse
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<FixedScoreQuery>()
                .is_some_and(|o| self.reverse == o.reverse && self.id_to_num == o.id_to_num)
    }

    fn query_hash(&self) -> u64 {
        let mut hash = self.class_hash() as u32;
        hash = 31u32
            .wrapping_mul(hash)
            .wrapping_add(if self.reverse { 0 } else { 1 });
        // `Arrays.hashCode(int[])`
        let mut array_hash = 1u32;
        for v in self.id_to_num.iter() {
            array_hash = 31u32.wrapping_mul(array_hash).wrapping_add(*v as u32);
        }
        hash = 31u32.wrapping_mul(hash).wrapping_add(array_hash);
        hash as u64
    }
}
