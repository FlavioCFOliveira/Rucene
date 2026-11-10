//! Port of `lucene/core/src/test/org/apache/lucene/search/TestTopFieldCollectorEarlyTermination.java`.
//!
//! The instance fields Java's `createRandomIndex` fills (`numDocs`, `terms`,
//! `dir`, `iw`, `reader`) and the `sort` field are an [`Instance`] that each
//! test method owns, as each Java test method runs on a fresh instance.
//! Java's `HashSet<String>` of random terms becomes a vector that keeps the
//! first occurrence of each term, so the draws are the same.

use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::random::JdkRandom;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, NumericDocValuesField, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::Term;
use rucene::search::field_doc::FieldDoc;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::term_query::TermQuery;
use rucene::search::top_field_collector::can_early_terminate;
use rucene::search::top_field_collector_manager::TopFieldCollectorManager;
use rucene::search::total_hits::TotalHitsRelation;
use rucene::search::{MatchAllDocsQuery, Query, ScoreDoc, TopFieldDocs};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::{MockRandomMergePolicy, RandomIndexWriter};
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_equals, class, random, test};

/// `TestTopFieldCollectorEarlyTermination`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestTopFieldCollectorEarlyTermination",
        vec![
            test("testEarlyTermination", test_early_termination),
            test(
                "testEarlyTerminationWhenPaging",
                test_early_termination_when_paging,
            ),
            test(
                "testCanEarlyTerminateOnDocId",
                test_can_early_terminate_on_doc_id,
            ),
            test(
                "testCanEarlyTerminateOnPrefix",
                test_can_early_terminate_on_prefix,
            ),
        ],
    )
}

const FORCE_MERGE_MAX_SEGMENT_COUNT: i32 = 5;

/// The instance state of `TestTopFieldCollectorEarlyTermination`.
struct Instance {
    num_docs: i32,
    terms: Vec<String>,
    dir: Option<TestDirectory>,
    /// `private final Sort sort = new Sort(new SortField("ndv1", SortField.Type.LONG))`.
    sort: Sort,
    iw: Option<RandomIndexWriter>,
    reader: Option<Arc<dyn DirectoryReader>>,
}

impl Instance {
    fn new() -> Result<Self> {
        Ok(Instance {
            num_docs: 0,
            terms: Vec::new(),
            dir: None,
            sort: Sort::new_fields(vec![SortField::new(
                Some("ndv1".to_string()),
                SortFieldType::Long,
            )?])?,
            iw: None,
            reader: None,
        })
    }

    /// `private Document randomDocument()`.
    fn random_document(&self) -> Result<Document> {
        let mut doc = Document::new();
        doc.add(Box::new(NumericDocValuesField::new(
            "ndv1",
            i64::from(random().next_int_bound(10)),
        )));
        doc.add(Box::new(NumericDocValuesField::new(
            "ndv2",
            i64::from(random().next_int_bound(10)),
        )));
        doc.add(Box::new(StringField::new(
            "s",
            RandomPicks::random_from_list(&*random(), &self.terms).clone(),
            Store::YES,
        )?));
        Ok(doc)
    }

    /// `private void createRandomIndex(boolean singleSortedSegment)`.
    fn create_random_index(&mut self, single_sorted_segment: bool) -> Result<()> {
        let dir = LuceneTestCase::new_directory();
        self.num_docs = LuceneTestCase::at_least(150);
        let num_terms = TestUtil::next_int(&*random(), 1, self.num_docs / 5);
        let mut random_terms: Vec<String> = Vec::new();
        while (random_terms.len() as i32) < num_terms {
            let term = TestUtil::random_simple_string(&*random());
            if !random_terms.contains(&term) {
                random_terms.push(term);
            }
        }
        self.terms = random_terms;
        let seed = random().next_long();
        let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&JdkRandom::new(seed)));
        let mut iwc = LuceneTestCase::new_index_writer_config_analyzer(analyzer);
        if iwc
            .merge_policy()
            .as_any()
            .is_some_and(|a| a.is::<MockRandomMergePolicy>())
        {
            // MockRandomMP randomly wraps the leaf readers which makes merging angry
            iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_tiered_merge_policy())))?;
        }
        iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?; // for reproducible tests
        iwc.set_index_sort(self.sort.clone())?;
        let iw =
            RandomIndexWriter::new_with_config(&JdkRandom::new(seed), dir.as_directory(), iwc)?;
        iw.set_do_random_force_merge(false); // don't do this, it may happen anyway with MockRandomMP
        for i in 0..self.num_docs {
            let doc = self.random_document()?;
            iw.add_document(&doc)?;
            if i == self.num_docs / 2 || (i != self.num_docs - 1 && random().next_int_bound(8) == 0)
            {
                iw.commit()?;
            }
            if random().next_int_bound(15) == 0 {
                let term = RandomPicks::random_from_list(&*random(), &self.terms).clone();
                iw.delete_documents(Term::from_text("s", &term))?;
            }
        }
        if single_sorted_segment {
            iw.force_merge(1)?;
        } else if random().next_boolean() {
            iw.force_merge(FORCE_MERGE_MAX_SEGMENT_COUNT)?;
        }
        let mut reader = iw.get_reader()?;
        if reader.num_docs() == 0 {
            iw.add_document(&Document::new())?;
            reader.close()?;
            reader = iw.get_reader()?;
        }
        self.dir = Some(dir);
        self.iw = Some(iw);
        self.reader = Some(reader);
        Ok(())
    }

    /// `private void closeIndex()`.
    fn close_index(&mut self) -> Result<()> {
        if let Some(reader) = self.reader.take() {
            reader.close()?;
        }
        if let Some(iw) = self.iw.take() {
            iw.close()?;
        }
        if let Some(mut dir) = self.dir.take() {
            dir.close()?;
        }
        Ok(())
    }
}

/// Port of `TestTopFieldCollectorEarlyTermination.testEarlyTermination`.
fn test_early_termination() -> Result<()> {
    do_test_early_termination(false)
}

/// Port of `TestTopFieldCollectorEarlyTermination.testEarlyTerminationWhenPaging`.
fn test_early_termination_when_paging() -> Result<()> {
    do_test_early_termination(true)
}

/// `TopFieldDocs.scoreDocs` as the `ScoreDoc[]` `CheckHits.checkEqual` reads.
fn score_docs(top_docs: &TopFieldDocs) -> Vec<ScoreDoc> {
    top_docs.score_docs.iter().map(|fd| fd.score_doc).collect()
}

/// `private void doTestEarlyTermination(boolean paging)`.
fn do_test_early_termination(paging: bool) -> Result<()> {
    let mut f = Instance::new()?;
    let iters = LuceneTestCase::at_least(1);
    for _i in 0..iters {
        f.create_random_index(false)?;
        let reader = Arc::clone(f.reader.as_ref().expect("createRandomIndex ran"));
        for _j in 0..iters {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
            let mut max_slice_size = 0;
            for slice in searcher.get_slices()? {
                let mut num_docs = 0; // number of live docs in the slice
                for partition in slice.partitions() {
                    let live_docs = partition.ctx.leaf_reader().get_live_docs();
                    let max_doc = partition
                        .max_doc_id
                        .min(partition.ctx.leaf_reader().max_doc());
                    for doc in partition.min_doc_id..max_doc {
                        if live_docs.as_ref().is_none_or(|l| l.get(doc as usize)) {
                            num_docs += 1;
                        }
                    }
                }
                max_slice_size = max_slice_size.max(num_docs);
            }
            let num_hits = TestUtil::next_int(&*random(), 1, f.num_docs);
            let after: Option<FieldDoc> = if paging {
                debug_assert!(searcher.get_index_reader().num_docs() > 0);
                let td = searcher.search_with_sort(
                    Arc::new(MatchAllDocsQuery::instance()),
                    10,
                    &f.sort,
                )?;
                Some(td.score_docs[td.score_docs.len() - 1].clone())
            } else {
                None
            };
            let manager1 =
                TopFieldCollectorManager::new(f.sort.clone(), num_hits, after.clone(), i32::MAX)?;
            let manager2 = TopFieldCollectorManager::new(f.sort.clone(), num_hits, after, 1)?;

            let query: Arc<dyn Query> = if random().next_boolean() {
                Arc::new(TermQuery::new(Term::from_text(
                    "s",
                    RandomPicks::random_from_list(&*random(), &f.terms),
                )))
            } else {
                Arc::new(MatchAllDocsQuery::instance())
            };
            let td1 = searcher.search_with_manager(Arc::clone(&query), &manager1)?;
            let td2 = searcher.search_with_manager(Arc::clone(&query), &manager2)?;

            assert_not_equals(
                &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
                &td1.total_hits.relation(),
            );
            if !paging && max_slice_size > num_hits && query.as_any().is::<MatchAllDocsQuery>() {
                // Make sure that we sometimes early terminate
                assert_equals(
                    &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
                    &td2.total_hits.relation(),
                );
            }
            if td2.total_hits.relation() == TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO {
                Assert::assert_true(td2.total_hits.value() >= td1.score_docs.len() as i64);
                Assert::assert_true(td2.total_hits.value() <= i64::from(reader.max_doc()));
            } else {
                assert_equals(&td2.total_hits.value(), &td1.total_hits.value());
            }
            CheckHits::check_equal(&*query, &score_docs(&td1), &score_docs(&td2));
        }
        drop(reader);
        f.close_index()?;
    }
    Ok(())
}

/// `new Sort(fields...)`.
fn sort_of(fields: Vec<SortField>) -> Sort {
    Sort::new_fields(fields).expect("non-empty sort")
}

/// `new SortField(field, type)`.
fn sf(field: &str, t: SortFieldType) -> SortField {
    SortField::new(Some(field.to_string()), t).expect("named sort field")
}

/// `new SortField(field, type, reverse)`.
fn sf_reverse(field: &str, t: SortFieldType, reverse: bool) -> SortField {
    SortField::new_reverse(Some(field.to_string()), t, reverse).expect("named sort field")
}

/// Port of `TestTopFieldCollectorEarlyTermination.testCanEarlyTerminateOnDocId`.
fn test_can_early_terminate_on_doc_id() -> Result<()> {
    Assert::assert_true(can_early_terminate(
        &sort_of(vec![SortField::FIELD_DOC]),
        Some(&sort_of(vec![SortField::FIELD_DOC])),
    ));

    Assert::assert_true(can_early_terminate(
        &sort_of(vec![SortField::FIELD_DOC]),
        None,
    ));

    Assert::assert_false(can_early_terminate(
        &sort_of(vec![sf("a", SortFieldType::Long)]),
        None,
    ));

    Assert::assert_false(can_early_terminate(
        &sort_of(vec![sf("a", SortFieldType::Long)]),
        Some(&sort_of(vec![sf("b", SortFieldType::Long)])),
    ));

    Assert::assert_true(can_early_terminate(
        &sort_of(vec![SortField::FIELD_DOC]),
        Some(&sort_of(vec![sf("b", SortFieldType::Long)])),
    ));

    Assert::assert_true(can_early_terminate(
        &sort_of(vec![SortField::FIELD_DOC]),
        Some(&sort_of(vec![
            sf("b", SortFieldType::Long),
            SortField::FIELD_DOC,
        ])),
    ));

    Assert::assert_false(can_early_terminate(
        &sort_of(vec![sf("a", SortFieldType::Long)]),
        Some(&sort_of(vec![SortField::FIELD_DOC])),
    ));

    Assert::assert_false(can_early_terminate(
        &sort_of(vec![sf("a", SortFieldType::Long), SortField::FIELD_DOC]),
        Some(&sort_of(vec![SortField::FIELD_DOC])),
    ));
    Ok(())
}

/// Port of `TestTopFieldCollectorEarlyTermination.testCanEarlyTerminateOnPrefix`.
fn test_can_early_terminate_on_prefix() -> Result<()> {
    Assert::assert_true(can_early_terminate(
        &sort_of(vec![sf("a", SortFieldType::Long)]),
        Some(&sort_of(vec![sf("a", SortFieldType::Long)])),
    ));

    Assert::assert_true(can_early_terminate(
        &sort_of(vec![
            sf("a", SortFieldType::Long),
            sf("b", SortFieldType::String),
        ]),
        Some(&sort_of(vec![
            sf("a", SortFieldType::Long),
            sf("b", SortFieldType::String),
        ])),
    ));

    Assert::assert_true(can_early_terminate(
        &sort_of(vec![sf("a", SortFieldType::Long)]),
        Some(&sort_of(vec![
            sf("a", SortFieldType::Long),
            sf("b", SortFieldType::String),
        ])),
    ));

    Assert::assert_false(can_early_terminate(
        &sort_of(vec![sf_reverse("a", SortFieldType::Long, true)]),
        None,
    ));

    Assert::assert_false(can_early_terminate(
        &sort_of(vec![sf_reverse("a", SortFieldType::Long, true)]),
        Some(&sort_of(vec![sf_reverse("a", SortFieldType::Long, false)])),
    ));

    Assert::assert_false(can_early_terminate(
        &sort_of(vec![
            sf("a", SortFieldType::Long),
            sf("b", SortFieldType::String),
        ]),
        Some(&sort_of(vec![sf("a", SortFieldType::Long)])),
    ));

    Assert::assert_false(can_early_terminate(
        &sort_of(vec![
            sf("a", SortFieldType::Long),
            sf("b", SortFieldType::String),
        ]),
        Some(&sort_of(vec![
            sf("a", SortFieldType::Long),
            sf("c", SortFieldType::String),
        ])),
    ));

    Assert::assert_false(can_early_terminate(
        &sort_of(vec![
            sf("a", SortFieldType::Long),
            sf("b", SortFieldType::String),
        ]),
        Some(&sort_of(vec![
            sf("c", SortFieldType::Long),
            sf("b", SortFieldType::String),
        ])),
    ));
    Ok(())
}
