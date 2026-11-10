//! Port of `lucene/core/src/test/org/apache/lucene/search/comparators/TestTermOrdValComparatorAdaptiveSkipping.java`.
//!
//! Tests that the adaptive skipping logic in TermOrdValComparator's
//! SkipperBasedCompetitiveState preserves correctness across data
//! distributions, and that it disables skipping when the skip index is
//! ineffective.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, SortedDocValuesField, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::IndexReader;
use rucene::search::comparators::term_ord_val_comparator::TermOrdValComparator;
use rucene::search::field_comparator::{
    FieldComparator, SharedFieldComparator, SharedLeafComparator,
};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::pruning::Pruning;
use rucene::search::scorable::Scorable;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::top_field_collector_manager::TopFieldCollectorManager;
use rucene::search::{TopFieldDocs, NO_MORE_DOCS};
use rucene::util::BytesRef;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{close_dir, tq};
use crate::core_support::{assert_equals, assert_equals_msg, class, random, test};

const SORT_FIELD_SKIP: &str = "sort.skip";
const SORT_FIELD_NO_SKIP: &str = "sort.noskip";
const QUERY_FIELD: &str = "match";
const QUERY_VALUE: &str = "yes";
const NUM_DOCS: i32 = 200_000;
const NUM_HITS: i32 = 10;
const TOTAL_HITS_THRESHOLD: i32 = 1;

/// `TestTermOrdValComparatorAdaptiveSkipping`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.comparators.TestTermOrdValComparatorAdaptiveSkipping",
        vec![
            test(
                "testResultsCorrectForInterleavedData",
                test_results_correct_for_interleaved_data,
            ),
            test(
                "testSkippingEffectiveForClusteredData",
                test_skipping_effective_for_clustered_data,
            ),
            test(
                "testResultsCorrectForRandomData",
                test_results_correct_for_random_data,
            ),
            test(
                "testAdaptiveDisablingFiresForInterleavedData",
                test_adaptive_disabling_fires_for_interleaved_data,
            ),
            test(
                "testAdaptiveDisablingDoesNotFireForClusteredData",
                test_adaptive_disabling_does_not_fire_for_clustered_data,
            ),
        ],
    )
}

fn br(s: &str) -> BytesRef {
    BytesRef::from_char_sequence(s)
}

/// `try (Directory dir = newDirectory()) { body }`.
fn with_dir(body: impl FnOnce(&TestDirectory) -> Result<()>) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let result = body(&dir);
    result.and(close_dir(Some(dir)))
}

/// `try (IndexReader reader = DirectoryReader.open(dir)) { body }`.
fn with_reader(
    dir: &TestDirectory,
    body: impl FnOnce(&Arc<dyn IndexReader>) -> Result<()>,
) -> Result<()> {
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
    let result = body(&reader);
    result.and(reader.close())
}

/// For interleaved data (alternating values uncorrelated with doc order),
/// the adaptive logic may disable skipping. Verify that top-N results are
/// identical to the non-skip baseline.
fn test_results_correct_for_interleaved_data() -> Result<()> {
    let a = br("a");
    let z = br("z");
    with_dir(|dir| {
        build_index(dir, |i| if (i & 1) == 0 { a.clone() } else { z.clone() })?;
        with_reader(dir, |reader| {
            assert_top_n_match(&IndexSearcher::new(Arc::clone(reader))?)
        })
    })
}

/// For clustered data (values sorted with doc order), skipping should remain
/// active and prune non-competitive blocks. Verify results are correct AND
/// that totalHits is significantly lower with skipping, showing that the
/// adaptive logic did not incorrectly disable.
fn test_skipping_effective_for_clustered_data() -> Result<()> {
    let a = br("a");
    let z = br("z");
    with_dir(|dir| {
        build_index(dir, |i| {
            if i < NUM_DOCS / 2 {
                a.clone()
            } else {
                z.clone()
            }
        })?;
        with_reader(dir, |reader| {
            let searcher = IndexSearcher::new(Arc::clone(reader))?;

            assert_top_n_match(&searcher)?;

            let with_skip = search_sort(&searcher, SORT_FIELD_SKIP)?;
            let without_skip = search_sort(&searcher, SORT_FIELD_NO_SKIP)?;

            Assert::assert_true_msg(
                &format!(
                    "Expected skip-enabled totalHits ({}) to be significantly lower than non-skip ({}), proving skipping was not incorrectly disabled",
                    with_skip.total_hits.value(),
                    without_skip.total_hits.value()
                ),
                with_skip.total_hits.value() < without_skip.total_hits.value() * 3 / 4,
            );
            Ok(())
        })
    })
}

/// For random data, results must still be correct regardless of whether
/// skipping is active or adaptively disabled.
fn test_results_correct_for_random_data() -> Result<()> {
    let values: Vec<BytesRef> = (0..26u8)
        .map(|i| br(&((b'a' + i) as char).to_string()))
        .collect();
    with_dir(|dir| {
        build_index(dir, |_i| {
            values[random().next_int_bound(values.len() as i32) as usize].clone()
        })?;
        with_reader(dir, |reader| {
            assert_top_n_match(&IndexSearcher::new(Arc::clone(reader))?)
        })
    })
}

/// `leafComp.copy(0, 0)`: the Rust `copy` also takes the scorable the
/// collector holds, which the term comparator does not read.
struct NoScorable;

impl Scorable for NoScorable {
    fn score(&mut self) -> Result<f32> {
        Ok(0.0)
    }
}

/// The body both `testAdaptiveDisabling*` methods share; `expect_disabled`
/// is the final assertion.
fn adaptive_disabling(value_of: impl Fn(i32) -> BytesRef, expect_disabled: bool) -> Result<()> {
    with_dir(|dir| {
        build_index(dir, &value_of)?;
        with_reader(dir, |reader| {
            let leaves = Arc::clone(reader).leaves()?;
            assert_equals(&1, &leaves.len());
            let leaf_context = &leaves[0];
            let max_doc = leaf_context.leaf_reader().max_doc();

            // `new TermOrdValComparator(...)`, the `FieldComparator` whose
            // slot state the shared comparator holds.
            let mut comparator = SharedFieldComparator::new(TermOrdValComparator::new(
                1,
                SORT_FIELD_SKIP,
                false,
                false,
                Pruning::GREATER_THAN,
            ));
            let mut leaf_comp = comparator.get_leaf_comparator(leaf_context)?;
            Assert::assert_true(
                leaf_comp
                    .as_any()
                    .is_some_and(|a| a.is::<SharedLeafComparator<TermOrdValComparator>>()),
            );
            let is_skipping_disabled =
                |leaf_comp: &dyn rucene::search::leaf_field_comparator::LeafFieldComparator| {
                    leaf_comp
                        .as_any()
                        .and_then(|a| {
                            a.downcast_ref::<SharedLeafComparator<TermOrdValComparator>>()
                        })
                        .expect("TermOrdValLeafComparator")
                        .leaf()
                        .is_skipping_disabled()
                };

            leaf_comp.copy(0, 0, &mut NoScorable)?;
            leaf_comp.set_bottom(0)?;
            leaf_comp.set_hits_threshold_reached()?;

            let competitive_iter = leaf_comp.competitive_iterator()?;
            LuceneTestCase::assume_true(
                "Competitive iterator not available (codec may not support skip index)",
                competitive_iter.is_some(),
            );
            let mut competitive_iter = competitive_iter.expect("assumed");

            if expect_disabled {
                Assert::assert_false_msg(
                    "Skipping should not be disabled before iteration",
                    is_skipping_disabled(&*leaf_comp),
                );
            }

            let mut doc = 0;
            while doc < max_doc {
                let next = competitive_iter.advance(doc)?;
                if next == NO_MORE_DOCS {
                    break;
                }
                doc = next + 1;
            }

            if expect_disabled {
                Assert::assert_true_msg(
                    "Skipping should be adaptively disabled after traversing interleaved data",
                    is_skipping_disabled(&*leaf_comp),
                );
            } else {
                Assert::assert_false_msg(
                    "Skipping should remain active for clustered data",
                    is_skipping_disabled(&*leaf_comp),
                );
            }
            Ok(())
        })
    })
}

/// Directly exercises the comparator with interleaved data and verifies that
/// adaptive disabling fires via
/// `TermOrdValComparator.TermOrdValLeafComparator#isSkippingDisabled()`.
fn test_adaptive_disabling_fires_for_interleaved_data() -> Result<()> {
    let a = br("a");
    let z = br("z");
    adaptive_disabling(|i| if (i & 1) == 0 { a.clone() } else { z.clone() }, true)
}

/// Directly exercises the comparator with clustered data and verifies that
/// adaptive disabling does NOT fire, because the skip iterator successfully
/// skips non-competitive blocks.
fn test_adaptive_disabling_does_not_fire_for_clustered_data() -> Result<()> {
    let a = br("a");
    let z = br("z");
    adaptive_disabling(
        |i| {
            if i < NUM_DOCS / 2 {
                a.clone()
            } else {
                z.clone()
            }
        },
        false,
    )
}

fn build_index(dir: &TestDirectory, value_supplier: impl Fn(i32) -> BytesRef) -> Result<()> {
    let mut config = LuceneTestCase::new_index_writer_config();
    config.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
    let writer = IndexWriter::new(dir.as_directory(), config)?;
    let result = (|| -> Result<()> {
        for i in 0..NUM_DOCS {
            let mut doc = Document::new();
            doc.add(Box::new(StringField::new(
                QUERY_FIELD,
                QUERY_VALUE.to_string(),
                Store::NO,
            )?));
            let value = value_supplier(i);
            doc.add(Box::new(SortedDocValuesField::new(
                SORT_FIELD_NO_SKIP,
                value.clone(),
            )));
            doc.add(Box::new(SortedDocValuesField::indexed_field(
                SORT_FIELD_SKIP,
                value,
            )));
            writer.add_document(&doc)?;
        }
        writer.force_merge(1)
    })();
    result.and(writer.close())
}

fn assert_top_n_match(searcher: &IndexSearcher) -> Result<()> {
    let with_skip = search_sort(searcher, SORT_FIELD_SKIP)?;
    let without_skip = search_sort(searcher, SORT_FIELD_NO_SKIP)?;

    assert_equals_msg(
        Some("Top-N result count should match"),
        &with_skip.score_docs.len(),
        &without_skip.score_docs.len(),
    );
    for i in 0..with_skip.score_docs.len() {
        let skip_doc = &with_skip.score_docs[i];
        let no_skip_doc = &without_skip.score_docs[i];
        assert_equals_msg(
            Some(&format!("Sort value at position {i} should match")),
            &skip_doc.fields.as_ref().map(|f| f[0].clone()),
            &no_skip_doc.fields.as_ref().map(|f| f[0].clone()),
        );
    }
    Ok(())
}

fn search_sort(searcher: &IndexSearcher, field: &str) -> Result<TopFieldDocs> {
    let query = tq(QUERY_FIELD, QUERY_VALUE);
    let sort_field = SortField::new(Some(field.to_string()), SortFieldType::String)?;
    let sort = Sort::new_fields(vec![sort_field, SortField::FIELD_SCORE])?;
    let manager = TopFieldCollectorManager::new(sort, NUM_HITS, None, TOTAL_HITS_THRESHOLD)?;
    searcher.search_with_manager(query, &manager)
}
