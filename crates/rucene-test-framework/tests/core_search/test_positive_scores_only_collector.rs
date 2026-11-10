//! Port of `lucene/core/src/test/org/apache/lucene/search/TestPositiveScoresOnlyCollector.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::IndexReader;
use rucene::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::search::positive_scores_only_collector::PositiveScoresOnlyCollector;
use rucene::search::scorable::Scorable;
use rucene::search::scorer::Scorer;
use rucene::search::top_docs_collector::TopDocsCollector;
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene::search::{Collector, CollectorManager};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestPositiveScoresOnlyCollector`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestPositiveScoresOnlyCollector",
        vec![test("testNegativeScores", test_negative_scores)],
    )
}

/// The iterator of `SimpleScorer`, which shares the scorer's `idx`.
struct SimpleScorerIterator {
    idx: i32,
}

impl DocIdSetIterator for SimpleScorerIterator {
    fn doc_id(&self) -> i32 {
        self.idx
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.idx += 1;
        Ok(if self.idx != SCORES.len() as i32 {
            self.idx
        } else {
            NO_MORE_DOCS
        })
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.idx = target;
        Ok(if self.idx < SCORES.len() as i32 {
            self.idx
        } else {
            NO_MORE_DOCS
        })
    }

    fn cost(&self) -> i64 {
        SCORES.len() as i64
    }
}

/// `private static final class SimpleScorer extends Scorer`.
struct SimpleScorer {
    it: SimpleScorerIterator,
}

impl Scorable for SimpleScorer {
    fn score(&mut self) -> Result<f32> {
        let idx = self.it.idx;
        Ok(if idx == SCORES.len() as i32 {
            f32::NAN
        } else {
            SCORES[idx as usize]
        })
    }
}

impl Scorer for SimpleScorer {
    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(f32::INFINITY)
    }

    fn doc_id(&self) -> i32 {
        self.it.idx
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.it
    }
}

// The scores must have positive as well as negative values
const SCORES: [f32; 13] = [
    0.7767749,
    -1.7839992,
    8.9925785,
    7.9608946,
    -0.07948637,
    2.6356435,
    7.4950366,
    7.1490803,
    -8.108544,
    4.961808,
    2.2423935,
    -7.285586,
    4.6699767,
];

/// Port of `TestPositiveScoresOnlyCollector.testNegativeScores`.
fn test_negative_scores() -> Result<()> {
    // The Top*Collectors previously filtered out documents with <= scores.
    // This behavior has changed. This test checks that if
    // PositiveOnlyScoresFilter wraps one of these collectors, documents with
    // <= 0 scores are indeed filtered.

    let mut num_positive_scores = 0i64;
    for score in SCORES {
        if score > 0.0 {
            num_positive_scores += 1;
        }
    }

    let mut directory = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
    writer.add_document(&Document::new())?;
    writer.commit()?;
    let ir = writer.get_reader()?;
    writer.close()?;
    let mut s = SimpleScorer {
        it: SimpleScorerIterator { idx: -1 },
    };
    let tdc =
        TopScoreDocCollectorManager::new(SCORES.len() as i32, None, i32::MAX)?.new_collector()?;
    let mut c = PositiveScoresOnlyCollector::new(tdc);
    {
        let leaves = (Arc::clone(&ir) as Arc<dyn IndexReader>).leaves()?;
        let mut ac = c
            .get_leaf_collector(&leaves[0])
            .map_err(|e| e.into_lucene_error())?;
        ac.set_scorer(&mut s)?;
        while s.iterator().next_doc()? != NO_MORE_DOCS {
            ac.collect(0, &mut s).map_err(|e| e.into_lucene_error())?;
        }
    }
    // `tdc.topDocs()`: Java keeps its reference to the wrapped collector.
    let mut tdc = c.into_inner();
    let td = tdc.top_docs()?;
    let sd = &td.score_docs;
    assert_equals(&num_positive_scores, &td.total_hits.value());
    for d in sd {
        Assert::assert_true_msg(
            &format!("only positive scores should return: {}", d.score),
            d.score > 0.0,
        );
    }
    ir.close()?;
    directory.close()?;
    Ok(())
}
