//! Port of `lucene/core/src/test/org/apache/lucene/search/TestTopDocsMerge.java`.
//!
//! Java's `shardScoreDoc == scoreDoc` and `assertArrayEquals(ScoreDoc[],
//! ScoreDoc[])` compare the `ScoreDoc` objects by identity: `TopDocs.merge`
//! returns the input instances. The port's merge returns copies, so the
//! comparison is by value; the hits of the test have distinct `(doc, shard)`
//! pairs, so value equality holds exactly where identity does.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::document::{
    Document, FloatDocValuesField, NumericDocValuesField, SortedDocValuesField, Store,
};
use rucene::error::Result;
use rucene::index::multi_reader::reader_util;
use rucene::index::{IndexReaderContext, LeafReaderContext, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::term_query::TermQuery;
use rucene::search::top_field_collector_manager::TopFieldCollectorManager;
use rucene::search::total_hits::{TotalHits, TotalHitsRelation};
use rucene::search::weight::Weight;
use rucene::search::{
    Collector, CollectorManager, Query, ScoreDoc, ScoreMode, TopDocs, TopDocsCollector,
    TopFieldDocs, TopScoreDocCollectorManager, NO_MORE_DOCS,
};
use rucene::util::BytesRef;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::test_util::TopDocsView;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_array_equals, assert_equals, assert_equals_msg, assert_not_null, class, random, test,
    verbose, IAE,
};

/// `TestTopDocsMerge`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestTopDocsMerge",
        vec![
            test("testSort_1", test_sort_1),
            test("testSort_2", test_sort_2),
            test(
                "testInconsistentTopDocsFail",
                test_inconsistent_top_docs_fail,
            ),
            test("testPreAssignedShardIndex", test_pre_assigned_shard_index),
            test("testMergeTotalHitsRelation", test_merge_total_hits_relation),
        ],
    )
}

/// `private static class ShardSearcher extends IndexSearcher`.
struct ShardSearcher {
    searcher: IndexSearcher,
    ctx: Arc<LeafReaderContext>,
}

impl ShardSearcher {
    /// `ShardSearcher(LeafReaderContext ctx, IndexReaderContext parent)`.
    fn new(ctx: Arc<LeafReaderContext>, parent: Arc<dyn IndexReaderContext>) -> Result<Self> {
        Ok(ShardSearcher {
            searcher: IndexSearcher::from_context(parent, None)?,
            ctx,
        })
    }

    /// `public void search(Weight weight, Collector collector)`.
    fn search_collector(
        &self,
        weight: &Arc<dyn Weight>,
        collector: &mut dyn Collector,
    ) -> Result<()> {
        self.searcher
            .search_leaf(&self.ctx, 0, NO_MORE_DOCS, weight, collector)
    }

    /// `public TopDocs search(Weight weight, int topN)`.
    fn search_top_n(&self, weight: &Arc<dyn Weight>, top_n: i32) -> Result<TopDocs> {
        let mut collector =
            TopScoreDocCollectorManager::new(top_n, None, i32::MAX)?.new_collector()?;
        self.searcher
            .search_leaf(&self.ctx, 0, NO_MORE_DOCS, weight, &mut collector)?;
        collector.top_docs()
    }
}

impl fmt::Display for ShardSearcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ShardSearcher({})", self.ctx)
    }
}

/// Port of `TestTopDocsMerge.testSort_1`.
fn test_sort_1() -> Result<()> {
    sort_test(false)
}

/// Port of `TestTopDocsMerge.testSort_2`.
fn test_sort_2() -> Result<()> {
    sort_test(true)
}

/// Port of `TestTopDocsMerge.testInconsistentTopDocsFail`.
fn test_inconsistent_top_docs_fail() -> Result<()> {
    let mut top_docs = vec![
        TopDocs::new(
            TotalHits::new(1, TotalHitsRelation::EQUAL_TO)?,
            vec![ScoreDoc::with_shard_index(1, 1.0f32, 5)],
        ),
        TopDocs::new(
            TotalHits::new(1, TotalHitsRelation::EQUAL_TO)?,
            vec![ScoreDoc::with_shard_index(1, 1.0f32, -1)],
        ),
    ];
    if random().next_boolean() {
        top_docs.swap(0, 1);
    }
    expect_throws(IAE, || TopDocs::merge_from(0, 2, &top_docs));
    Ok(())
}

/// Port of `TestTopDocsMerge.testPreAssignedShardIndex`.
fn test_pre_assigned_shard_index() -> Result<()> {
    let use_constant_score = random().next_boolean();
    let num_top_docs = 2 + random().next_int_bound(10);
    let mut top_docs: Vec<TopDocs> = Vec::with_capacity(num_top_docs as usize);
    let mut shard_result_mapping: HashMap<i32, TopDocs> = HashMap::new();
    let mut num_hits_total = 0;
    for i in 0..num_top_docs {
        let num_hits = 1 + random().next_int_bound(10);
        num_hits_total += num_hits;
        let mut score_docs: Vec<ScoreDoc> = Vec::with_capacity(num_hits as usize);
        for j in 0..num_hits {
            let score = if use_constant_score {
                1.0f32
            } else {
                random().next_float()
            };
            // we set the shard index to index in the list here but shuffle the entire list below
            score_docs.push(ScoreDoc::with_shard_index((100 * i) + j, score, i));
        }
        top_docs.push(TopDocs::new(
            TotalHits::new(i64::from(num_hits), TotalHitsRelation::EQUAL_TO)?,
            score_docs,
        ));
        shard_result_mapping.insert(i, top_docs[i as usize].clone());
    }
    // shuffle the entire thing such that we don't get 1 to 1 mapping of shard index to index in the
    // array
    // -- well likely ;)
    shuffle(&mut top_docs, &*random());
    let from = random().next_int_bound(num_hits_total - 1);
    let size = 1 + random().next_int_bound(num_hits_total - from);

    // passing false here means TopDocs.merge uses the incoming ScoreDoc.shardIndex
    // that we already set, instead of the position of that TopDocs in the array:
    let merge = TopDocs::merge_from(from as usize, size as usize, &top_docs)?;

    Assert::assert_true(!merge.score_docs.is_empty());
    for score_doc in &merge.score_docs {
        Assert::assert_true(score_doc.shard_index != -1);
        let shard_top_docs = shard_result_mapping.get(&score_doc.shard_index);
        assert_not_null(&shard_top_docs);
        let mut found = false;
        for shard_score_doc in &shard_top_docs.expect("checked").score_docs {
            if shard_score_doc == score_doc {
                found = true;
                break;
            }
        }
        Assert::assert_true(found);
    }

    // now ensure merge is stable even if we use our own shard IDs
    shuffle(&mut top_docs, &*random());
    let merge2 = TopDocs::merge_from(from as usize, size as usize, &top_docs)?;
    assert_array_equals(&merge.score_docs, &merge2.score_docs);
    Ok(())
}

/// The whole-index hits of one iteration: a `TopDocs`, or the
/// `TopFieldDocs` of a sorted search.
enum Hits {
    Scores(TopDocs),
    Fields(TopFieldDocs),
}

impl Hits {
    fn view(&self) -> TopDocsView<'_> {
        match self {
            Hits::Scores(t) => TopDocsView::from(t),
            Hits::Fields(t) => TopDocsView::from(t),
        }
    }
}

/// `void testSort(boolean useFrom)`.
fn sort_test(use_from: bool) -> Result<()> {
    let num_docs = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(1000)
    } else {
        LuceneTestCase::at_least(100)
    };

    let tokens = ["a", "b", "c", "d", "e"];

    if verbose() {
        println!("TEST: make index");
    }

    let mut dir = LuceneTestCase::new_directory();
    let reader = {
        let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        // w.setDoRandomForceMerge(false);

        // w.w.getConfig().setMaxBufferedDocs(atLeast(100));

        let mut content: Vec<String> = vec![String::new(); LuceneTestCase::at_least(20) as usize];

        for content_idx in 0..content.len() {
            let mut sb = String::new();
            let num_tokens = TestUtil::next_int(&*random(), 1, 10);
            for _token_idx in 0..num_tokens {
                sb.push_str(tokens[random().next_int_bound(tokens.len() as i32) as usize]);
                sb.push(' ');
            }
            content[content_idx] = sb;
        }

        for _doc_idx in 0..num_docs {
            let mut doc = Document::new();
            doc.add(Box::new(SortedDocValuesField::new(
                "string",
                BytesRef::from_char_sequence(
                    &TestUtil::random_realistic_unicode_string(&*random()),
                ),
            )));
            doc.add(Box::new(LuceneTestCase::new_text_field(
                "text",
                &content[random().next_int_bound(content.len() as i32) as usize],
                Store::NO,
            )));
            doc.add(Box::new(FloatDocValuesField::new(
                "float",
                random().next_float(),
            )));
            let int_value = if random().next_int_bound(100) == 17 {
                i32::MIN
            } else if random().next_int_bound(100) == 17 {
                i32::MAX
            } else {
                random().next_int()
            };
            doc.add(Box::new(NumericDocValuesField::new(
                "int",
                i64::from(int_value),
            )));
            if verbose() {
                println!("  doc={doc:?}");
            }
            w.add_document(&doc)?;
        }

        let reader = w.get_reader()?;
        w.close()?;
        reader
    };

    // NOTE: sometimes reader has just one segment, which is
    // important to test
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let ctx = Arc::clone(searcher.get_top_reader_context());

    let mut sub_searchers: Vec<ShardSearcher> = Vec::new();
    let mut doc_starts: Vec<i32> = Vec::new();

    if ctx.is_leaf_context() {
        let leaf = Arc::clone(&ctx).as_leaf().expect("LeafReaderContext");
        sub_searchers.push(ShardSearcher::new(leaf, Arc::clone(&ctx))?);
        doc_starts.push(0);
    } else {
        let comp_ctx = Arc::clone(&ctx);
        let leaves = Arc::clone(&comp_ctx).leaves();
        let mut doc_base = 0;
        for leave in leaves {
            let max_doc = leave.leaf_reader().max_doc();
            sub_searchers.push(ShardSearcher::new(leave, Arc::clone(&comp_ctx))?);
            doc_starts.push(doc_base);
            doc_base += max_doc;
        }
    }

    let sort_fields: Vec<SortField> = vec![
        SortField::new_reverse(Some("string".to_string()), SortFieldType::String, true)?,
        SortField::new_reverse(Some("string".to_string()), SortFieldType::String, false)?,
        SortField::new_reverse(Some("int".to_string()), SortFieldType::Int, true)?,
        SortField::new_reverse(Some("int".to_string()), SortFieldType::Int, false)?,
        SortField::new_reverse(Some("float".to_string()), SortFieldType::Float, true)?,
        SortField::new_reverse(Some("float".to_string()), SortFieldType::Float, false)?,
        SortField::new_reverse(None, SortFieldType::Score, true)?,
        SortField::new_reverse(None, SortFieldType::Score, false)?,
        SortField::new_reverse(None, SortFieldType::Doc, true)?,
        SortField::new_reverse(None, SortFieldType::Doc, false)?,
    ];

    let num_iters = LuceneTestCase::at_least(300);
    for _iter in 0..num_iters {
        // TODO: custom FieldComp...
        let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text(
            "text",
            tokens[random().next_int_bound(tokens.len() as i32) as usize],
        )));

        let sort: Option<Sort> = if random().next_int_bound(10) == 4 {
            // Sort by score
            None
        } else {
            let mut random_sort_fields: Vec<SortField> = Vec::new();
            let n = TestUtil::next_int(&*random(), 1, 3);
            for _sort_idx in 0..n {
                random_sort_fields.push(
                    sort_fields[random().next_int_bound(sort_fields.len() as i32) as usize].clone(),
                );
            }
            Some(Sort::new_fields(random_sort_fields)?)
        };

        let num_hits = TestUtil::next_int(&*random(), 1, num_docs + 5);
        // final int numHits = 5;

        if verbose() {
            println!(
                "TEST: search query={} sort={} numHits={num_hits}",
                rucene::search::query_to_string(&*query),
                sort.as_ref().map_or("null".to_string(), |s| s.to_string())
            );
        }

        let mut from: i32 = -1;
        let mut size: i32 = -1;
        // First search on whole index:
        let top_hits: Hits = match &sort {
            None => {
                if use_from {
                    from = TestUtil::next_int(&*random(), 0, num_hits - 1);
                    size = num_hits - from;
                    let mut temp_top_hits = searcher.search_with_manager(
                        Arc::clone(&query),
                        &TopScoreDocCollectorManager::with_num_hits(num_hits, i32::MAX)?,
                    )?;
                    if (from as usize) < temp_top_hits.score_docs.len() {
                        // Can't use TopDocs#topDocs(start, howMany), since it has different behaviour when
                        // start >= hitCount
                        // than TopDocs#merge currently has
                        let len =
                            (size as usize).min(temp_top_hits.score_docs.len() - from as usize);
                        let new_score_docs: Vec<ScoreDoc> =
                            temp_top_hits.score_docs[from as usize..from as usize + len].to_vec();
                        temp_top_hits.score_docs = new_score_docs;
                        Hits::Scores(temp_top_hits)
                    } else {
                        Hits::Scores(TopDocs::new(temp_top_hits.total_hits, Vec::new()))
                    }
                } else {
                    Hits::Scores(searcher.search(Arc::clone(&query), num_hits)?)
                }
            }
            Some(sort) => {
                let mut top_field_docs = searcher.search_with_manager(
                    Arc::clone(&query),
                    &TopFieldCollectorManager::with_num_hits(sort.clone(), num_hits, i32::MAX)?,
                )?;
                if use_from {
                    from = TestUtil::next_int(&*random(), 0, num_hits - 1);
                    size = num_hits - from;
                    if (from as usize) < top_field_docs.score_docs.len() {
                        // Can't use TopDocs#topDocs(start, howMany), since it has different behaviour when
                        // start >= hitCount
                        // than TopDocs#merge currently has
                        let len =
                            (size as usize).min(top_field_docs.score_docs.len() - from as usize);
                        let new_score_docs =
                            top_field_docs.score_docs[from as usize..from as usize + len].to_vec();
                        top_field_docs.score_docs = new_score_docs;
                        Hits::Fields(top_field_docs)
                    } else {
                        Hits::Scores(TopDocs::new(top_field_docs.total_hits, Vec::new()))
                    }
                } else {
                    Hits::Fields(top_field_docs)
                }
            }
        };

        if verbose() {
            if use_from {
                println!("from={from} size={size}");
            }
            let docs: Vec<ScoreDoc> = match &top_hits {
                Hits::Scores(t) => t.score_docs.clone(),
                Hits::Fields(t) => t.score_docs.iter().map(|f| f.score_doc).collect(),
            };
            let total = match &top_hits {
                Hits::Scores(t) => t.total_hits.value(),
                Hits::Fields(t) => t.total_hits.value(),
            };
            println!("  top search: {total} totalHits; hits={}", docs.len());
            for sd in &docs {
                println!("    doc={} score={}", sd.doc, sd.score);
            }
        }

        // ... then all shards:
        let w = searcher.create_weight(
            searcher.rewrite(Arc::clone(&query))?,
            ScoreMode::COMPLETE,
            1.0,
        )?;

        let mut shard_hits: Vec<TopDocs> = Vec::new();
        let mut shard_field_hits: Vec<TopFieldDocs> = Vec::new();
        for (shard_idx, sub_searcher) in sub_searchers.iter().enumerate() {
            match &sort {
                None => {
                    let mut sub_hits = sub_searcher.search_top_n(&w, num_hits)?;
                    for i in 0..sub_hits.score_docs.len() {
                        sub_hits.score_docs[i].shard_index = shard_idx as i32;
                    }
                    if verbose() {
                        println!(
                            "  shard={shard_idx} {} totalHits hits={}",
                            sub_hits.total_hits.value(),
                            sub_hits.score_docs.len()
                        );
                        for sd in &sub_hits.score_docs {
                            println!("    doc={} score={}", sd.doc, sd.score);
                        }
                    }
                    shard_hits.push(sub_hits);
                }
                Some(sort) => {
                    let mut c =
                        TopFieldCollectorManager::new(sort.clone(), num_hits, None, i32::MAX)?
                            .new_collector()?;
                    sub_searcher.search_collector(&w, &mut c)?;
                    let mut sub_hits = c.top_docs_range(0, num_hits)?;
                    for i in 0..sub_hits.score_docs.len() {
                        sub_hits.score_docs[i].score_doc.shard_index = shard_idx as i32;
                    }
                    if verbose() {
                        println!(
                            "  shard={shard_idx} {} totalHits hits={}",
                            sub_hits.total_hits.value(),
                            sub_hits.score_docs.len()
                        );
                        for sd in &sub_hits.score_docs {
                            println!("    doc={} score={}", sd.doc(), sd.score());
                        }
                    }
                    shard_field_hits.push(sub_hits);
                }
            }
        }

        // Merge:
        let merged_hits: Hits = if use_from {
            match &sort {
                None => Hits::Scores(TopDocs::merge_from(
                    from as usize,
                    size as usize,
                    &shard_hits,
                )?),
                Some(sort) => Hits::Fields(TopDocs::merge_sorted_from(
                    sort,
                    from as usize,
                    size as usize,
                    &shard_field_hits,
                )?),
            }
        } else {
            match &sort {
                None => Hits::Scores(TopDocs::merge(num_hits as usize, &shard_hits)?),
                Some(sort) => Hits::Fields(TopDocs::merge_sorted(
                    sort,
                    num_hits as usize,
                    &shard_field_hits,
                )?),
            }
        };

        // Make sure the returned shards are correct:
        let merged_docs: Vec<ScoreDoc> = match &merged_hits {
            Hits::Scores(t) => t.score_docs.clone(),
            Hits::Fields(t) => t.score_docs.iter().map(|f| f.score_doc).collect(),
        };
        for sd in &merged_docs {
            assert_equals_msg(
                Some(&format!("doc={} wrong shard", sd.doc)),
                &reader_util::sub_index(sd.doc, &doc_starts),
                &sd.shard_index,
            );
        }

        TestUtil::assert_consistent(top_hits.view(), merged_hits.view());
    }
    drop(sub_searchers);
    drop(searcher);
    reader.close()?;
    dir.close()
}

/// Port of `TestTopDocsMerge.testMergeTotalHitsRelation`.
fn test_merge_total_hits_relation() -> Result<()> {
    let top_docs1 = TopDocs::new(
        TotalHits::new(2, TotalHitsRelation::EQUAL_TO)?,
        vec![ScoreDoc::with_shard_index(42, 2f32, 0)],
    );
    let top_docs2 = TopDocs::new(
        TotalHits::new(1, TotalHitsRelation::EQUAL_TO)?,
        vec![ScoreDoc::with_shard_index(42, 2f32, 1)],
    );
    let top_docs3 = TopDocs::new(
        TotalHits::new(1, TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO)?,
        vec![ScoreDoc::with_shard_index(42, 2f32, 2)],
    );
    let top_docs4 = TopDocs::new(
        TotalHits::new(3, TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO)?,
        vec![ScoreDoc::with_shard_index(42, 2f32, 3)],
    );

    let merged1 = TopDocs::merge(1, &[top_docs1.clone(), top_docs2.clone()])?;
    assert_equals(
        &TotalHits::new(3, TotalHitsRelation::EQUAL_TO)?,
        &merged1.total_hits,
    );

    let merged2 = TopDocs::merge(1, &[top_docs1, top_docs3.clone()])?;
    assert_equals(
        &TotalHits::new(3, TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO)?,
        &merged2.total_hits,
    );

    let merged3 = TopDocs::merge(1, &[top_docs3, top_docs4.clone()])?;
    assert_equals(
        &TotalHits::new(4, TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO)?,
        &merged3.total_hits,
    );

    let merged4 = TopDocs::merge(1, &[top_docs4, top_docs2])?;
    assert_equals(
        &TotalHits::new(4, TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO)?,
        &merged4.total_hits,
    );
    Ok(())
}
