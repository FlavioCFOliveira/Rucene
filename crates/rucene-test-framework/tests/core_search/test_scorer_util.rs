//! Port of `lucene/core/src/test/org/apache/lucene/search/TestScorerUtil.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::feature_field::FeatureField;
use rucene::document::{Document, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::postings_enum::POSTINGS_ENUM_FREQS;
use rucene::index::{IndexReader, Term};
use rucene::search::abstract_doc_id_set_iterator::FilterDocIdSetIterator;
use rucene::search::doc_id_set_iterator::{self, DocIdSetIterator};
use rucene::search::scorer_util::{LikelyLiveDocs, ScorerUtil};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::math_util::MathUtil;
use rucene::util::{Bits, BytesRef, MatchAllBits, SparseFixedBitSet};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{class, random, test};

/// `TestScorerUtil`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestScorerUtil",
        vec![
            test("testLikelyFixedBits", test_likely_fixed_bits),
            test("testLikelyImpactsEnum", test_likely_impacts_enum),
            test("testMinRequiredScore", test_min_required_score),
        ],
    )
}

/// Whether `ScorerUtil.likelyLiveDocs(bits)` returned `bits` itself
/// (`assertSame`), rather than a wrapper.
fn is_same(bits: &dyn Bits, likely: &Option<LikelyLiveDocs<'_>>) -> bool {
    match likely {
        Some(likely) => {
            std::ptr::addr_eq(likely.as_bits() as *const dyn Bits, bits as *const dyn Bits)
        }
        None => false,
    }
}

/// `ScorerUtil.likelyLiveDocs(bits).getClass()`: `true` for the class of
/// the bits themselves, `false` for `FilterBits`.
fn class_of(likely: &Option<LikelyLiveDocs<'_>>) -> Option<bool> {
    likely
        .as_ref()
        .map(|l| matches!(l, LikelyLiveDocs::Same(_)))
}

/// Port of `TestScorerUtil.testLikelyFixedBits`.
fn test_likely_fixed_bits() -> Result<()> {
    Assert::assert_true(ScorerUtil::likely_live_docs(None).is_none());

    let bits1 = SparseFixedBitSet::new(10)?;
    Assert::assert_false(is_same(&bits1, &ScorerUtil::likely_live_docs(Some(&bits1))));
    let bits2 = MatchAllBits::new(10);
    Assert::assert_false(is_same(&bits2, &ScorerUtil::likely_live_docs(Some(&bits2))));
    Assert::assert_true(
        class_of(&ScorerUtil::likely_live_docs(Some(&bits1)))
            == class_of(&ScorerUtil::likely_live_docs(Some(&bits2))),
    );

    let mut dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
    let iwc = IndexWriterConfig::new();
    iwc.set_codec(TestUtil::get_default_codec())?;
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    let w = IndexWriter::new(Arc::clone(&dir), iwc)?;
    let result = (|| -> Result<()> {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            "1".to_string(),
            Store::NO,
        )?));
        w.add_document(&doc)?;
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            "2".to_string(),
            Store::NO,
        )?));
        w.add_document(&doc)?;
        w.delete_documents_by_terms(vec![Term::from_text("id", "1")])?;
        let reader = directory_reader::open_with_writer(&*w)?;
        let r = (|| -> Result<()> {
            let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
            let leaf_reader = leaves[0].leaf_reader();
            let accept_docs = leaf_reader.get_live_docs();
            Assert::assert_true(accept_docs.is_some());
            let accept_docs = accept_docs.expect("assertNotNull");
            Assert::assert_true(is_same(
                &*accept_docs,
                &ScorerUtil::likely_live_docs(Some(&*accept_docs)),
            ));
            Ok(())
        })();
        reader.close()?;
        r
    })();
    let closed = w.close();
    drop(w);
    result.and(closed)?;
    // `dir.close()`: the writer and reader that shared it are closed.
    if let Some(dir) = Arc::get_mut(&mut dir) {
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestScorerUtil.testLikelyImpactsEnum`.
fn test_likely_impacts_enum() -> Result<()> {
    let iterator: Box<dyn DocIdSetIterator> = Box::new(doc_id_set_iterator::all(10)?);
    Assert::assert_true(
        ScorerUtil::likely_impacts_enum(iterator)
            .disi_as_any()
            .is_some_and(|a| a.is::<FilterDocIdSetIterator>()),
    );

    let mut dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
    let iwc = IndexWriterConfig::new();
    iwc.set_codec(TestUtil::get_default_codec())?;
    let w = IndexWriter::new(Arc::clone(&dir), iwc)?;
    let result = (|| -> Result<()> {
        let mut doc = Document::new();
        doc.add(Box::new(FeatureField::new("field", "value", 1.0)?));
        w.add_document(&doc)?;
        let reader = directory_reader::open_with_writer(&*w)?;
        let r = (|| -> Result<()> {
            let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
            let leaf_reader = leaves[0].leaf_reader();
            let mut te = leaf_reader
                .terms("field")?
                .unwrap_or_else(|| {
                    randomizedtesting::jdk::exception::throw_without_message(
                        "java.lang.NullPointerException",
                    )
                })
                .iterator()?;
            Assert::assert_true(te.seek_exact(&BytesRef::from_char_sequence("value"))?);
            let ie = te.impacts(POSTINGS_ENUM_FREQS)?;
            let ie: Box<dyn DocIdSetIterator> = ie;
            let before = &*ie as *const dyn DocIdSetIterator;
            let after = ScorerUtil::likely_impacts_enum(ie);
            Assert::assert_true(std::ptr::addr_eq(
                before,
                &*after as *const dyn DocIdSetIterator,
            ));
            Ok(())
        })();
        reader.close()?;
        r
    })();
    let closed = w.close();
    drop(w);
    result.and(closed)?;
    // `dir.close()`: the writer and reader that shared it are closed.
    if let Some(dir) = Arc::get_mut(&mut dir) {
        dir.close()?;
    }
    Ok(())
}

/// `Math.ulp(float)`.
fn float_ulp(f: f32) -> f32 {
    let f = f.abs();
    if f.is_nan() {
        return f32::NAN;
    }
    if f.is_infinite() {
        return f32::INFINITY;
    }
    if f == f32::MAX {
        return f32::from_bits(0x7380_0000); // 2^104
    }
    f.next_up() - f
}

/// Port of `TestScorerUtil.testMinRequiredScore`.
fn test_min_required_score() -> Result<()> {
    let iters = LuceneTestCase::at_least(10000);
    for _iter in 0..iters {
        let max_remaining_score = random().next_double();
        let min_competitive_score = random().next_float();
        let num_scorers = random().next_int_range(1, 1000);

        let min_required_score =
            ScorerUtil::min_required_score(max_remaining_score, min_competitive_score, num_scorers);
        if f64::from(min_competitive_score) < max_remaining_score {
            Assert::assert_true(min_required_score <= 0.0);
        } else {
            // The value before minRequiredScore must not be able to produce a score >=
            // minCompetitiveScore.
            Assert::assert_false(
                MathUtil::sum_upper_bound(
                    min_required_score.next_down() + max_remaining_score,
                    num_scorers,
                ) as f32
                    >= min_competitive_score,
            );
        }

        // NOTE: we need to assert the internal while loop ends within an acceptable iterations. But
        // it seems there is no easy way to do this assertion, so the assertion below relies on the
        // internal implementation detail of ScorerUtil.minRequiredScore
        let mut initial_min_required_score = f64::from(min_competitive_score) - max_remaining_score;
        let subtraction = f64::from(float_ulp(min_competitive_score));
        let expect_converge = 10;
        for _i in 0..expect_converge {
            initial_min_required_score -= subtraction;
        }
        Assert::assert_true(initial_min_required_score <= min_required_score);
    }
    Ok(())
}
