//! Port of `lucene/core/src/test/org/apache/lucene/search/TestScoreCachingWrappingScorer.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::scorable::Scorable;
use rucene::search::score_caching_wrapping_scorer::ScoreCachingWrappingScorer;
use rucene::search::scorer::Scorer;
use rucene::search::{
    CollectionResult, Collector, DocIdSetIterator, LeafCollector, ScoreMode, NO_MORE_DOCS,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::null_leaf_context;
use crate::core_support::{class, random, test};

/// `TestScoreCachingWrappingScorer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestScoreCachingWrappingScorer",
        vec![test("testGetScores", test_get_scores)],
    )
}

/// The anonymous `DocIdSetIterator` of `SimpleScorer.iterator()`, which reads
/// and writes the scorer's `doc`.
#[derive(Debug)]
struct SimpleScorerIterator {
    doc: i32,
}

impl DocIdSetIterator for SimpleScorerIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc += 1;
        Ok(if (self.doc as usize) < SCORES.len() {
            self.doc
        } else {
            NO_MORE_DOCS
        })
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc = target;
        Ok(if (self.doc as usize) < SCORES.len() {
            self.doc
        } else {
            NO_MORE_DOCS
        })
    }

    fn cost(&self) -> i64 {
        SCORES.len() as i64
    }
}

/// `private static final class SimpleScorer extends Scorer`.
///
/// Java's `iterator()` returns a new anonymous iterator over the scorer's
/// `doc` field on each call; the one iterator here holds that `doc`.
#[derive(Debug)]
struct SimpleScorer {
    idx: usize,
    iterator: SimpleScorerIterator,
}

impl SimpleScorer {
    fn new() -> Self {
        SimpleScorer {
            idx: 0,
            iterator: SimpleScorerIterator { doc: -1 },
        }
    }
}

impl Scorable for SimpleScorer {
    fn score(&mut self) -> Result<f32> {
        // advance idx on purpose, so that consecutive calls to score will get
        // different results. This is to emulate computation of a score. If
        // ScoreCachingWrappingScorer is used, this should not be called more than
        // once per document.
        Ok(if self.idx == SCORES.len() {
            f32::NAN
        } else {
            let s = SCORES[self.idx];
            self.idx += 1;
            s
        })
    }
}

impl Scorer for SimpleScorer {
    fn doc_id(&self) -> i32 {
        self.iterator.doc
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.iterator
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(f32::INFINITY)
    }
}

/// The anonymous `LeafCollector` of `ScoreCachingCollector.getLeafCollector`.
struct RecordingLeafCollector<'a> {
    idx: &'a mut usize,
    mscores: &'a mut Vec<f32>,
}

impl LeafCollector for RecordingLeafCollector<'_> {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        // just a sanity check to avoid IOOB.
        if *self.idx == self.mscores.len() {
            return Ok(());
        }

        // just call score() a couple of times and record the score.
        self.mscores[*self.idx] = scorer.score()?;
        self.mscores[*self.idx] = scorer.score()?;
        self.mscores[*self.idx] = scorer.score()?;
        *self.idx += 1;
        Ok(())
    }
}

/// `private static final class ScoreCachingCollector implements Collector`.
struct ScoreCachingCollector {
    idx: usize,
    mscores: Vec<f32>,
}

impl ScoreCachingCollector {
    fn new(num_to_collect: usize) -> Self {
        ScoreCachingCollector {
            idx: 0,
            mscores: vec![0.0; num_to_collect],
        }
    }
}

impl Collector for ScoreCachingCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        _context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        Ok(Box::new(ScoreCachingWrappingScorer::wrap(
            RecordingLeafCollector {
                idx: &mut self.idx,
                mscores: &mut self.mscores,
            },
        )))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }
}

const SCORES: [f32; 13] = [
    0.7767749, 1.7839992, 8.9925785, 7.9608946, 0.07948637, 2.6356435, 7.4950366, 7.1490803,
    8.108544, 4.961808, 2.2423935, 7.285586, 4.6699767,
];

/// Port of `TestScoreCachingWrappingScorer.testGetScores`.
fn test_get_scores() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
    writer.commit()?;
    let ir = writer.get_reader()?;
    writer.close()?;
    let mut s = SimpleScorer::new();
    let mut scc = ScoreCachingCollector::new(SCORES.len());
    {
        // `getLeafCollector(null)`.
        let (_null_reader, null_leaves) = null_leaf_context()?;
        let mut lc = scc
            .get_leaf_collector(&null_leaves[0])
            .map_err(|e| e.into_lucene_error())?;
        lc.set_scorer(&mut s)?;

        // We need to iterate on the scorer so that its doc() advances.
        loop {
            let doc = s.iterator().next_doc()?;
            if doc == NO_MORE_DOCS {
                break;
            }
            lc.collect(doc, &mut s).map_err(|e| e.into_lucene_error())?;
        }
        drop(lc);
        _null_reader.close()?;
    }

    for i in 0..SCORES.len() {
        Assert::assert_equals_float_msg(None, SCORES[i], scc.mscores[i], 0.0);
    }
    ir.close()?;
    directory.close()?;
    Ok(())
}
