//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSegmentCacheables.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericDocValuesField, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::doc_values::DocValues;
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{IndexReader, LeafReaderContext, Term};
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class, test};

/// `TestSegmentCacheables`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestSegmentCacheables",
        vec![test(
            "testMultipleDocValuesDelegates",
            test_multiple_doc_values_delegates,
        )],
    )
}

/// A `SegmentCacheable` lambda.
struct Lambda<F: Fn(&LeafReaderContext) -> bool>(F);

impl<F: Fn(&LeafReaderContext) -> bool> SegmentCacheable for Lambda<F> {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        (self.0)(ctx)
    }
}

/// `private static boolean isCacheable(LeafReaderContext ctx, SegmentCacheable... ss)`.
fn is_cacheable(ctx: &LeafReaderContext, ss: &[&dyn SegmentCacheable]) -> bool {
    for s in ss {
        if !s.is_cacheable(ctx) {
            return false;
        }
    }
    true
}

/// Port of `TestSegmentCacheables.testMultipleDocValuesDelegates`.
fn test_multiple_doc_values_delegates() -> Result<()> {
    let seg = Lambda(|_ctx: &LeafReaderContext| true);
    let non = Lambda(|_ctx: &LeafReaderContext| false);
    let dv1 = Lambda(|ctx: &LeafReaderContext| DocValues::is_cacheable(ctx, &["field1"]));
    let dv2 = Lambda(|ctx: &LeafReaderContext| DocValues::is_cacheable(ctx, &["field2"]));
    let dv3 = Lambda(|ctx: &LeafReaderContext| DocValues::is_cacheable(ctx, &["field3"]));
    let dv34 =
        Lambda(|ctx: &LeafReaderContext| DocValues::is_cacheable(ctx, &["field3", "field4"]));
    let dv12 =
        Lambda(|ctx: &LeafReaderContext| DocValues::is_cacheable(ctx, &["field1", "field2"]));

    let seg_dv1 = Lambda(|ctx: &LeafReaderContext| is_cacheable(ctx, &[&seg, &dv1]));
    let dv2_dv34 = Lambda(|ctx: &LeafReaderContext| is_cacheable(ctx, &[&dv2, &dv34]));
    let dv2_non = Lambda(|ctx: &LeafReaderContext| is_cacheable(ctx, &[&dv2, &non]));

    let seg_dv1_dv2_dv34 =
        Lambda(|ctx: &LeafReaderContext| is_cacheable(ctx, &[&seg_dv1, &dv2_dv34]));

    let dv1_dv3 = Lambda(|ctx: &LeafReaderContext| is_cacheable(ctx, &[&dv1, &dv3]));
    let dv12_dv1_dv3 = Lambda(|ctx: &LeafReaderContext| is_cacheable(ctx, &[&dv12, &dv1_dv3]));

    let mut dir = LuceneTestCase::new_directory();
    let mut iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Arc::new(NoMergePolicy::new()))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(NumericDocValuesField::new("field3", 1)));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "text",
        "text",
        Store::NO,
    )));
    w.add_document(&doc)?;
    w.commit()?;
    let reader = directory_reader::open_with_writer(&*w)?;

    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
    let ctx = &leaves[0];

    Assert::assert_true(seg_dv1.is_cacheable(ctx));
    Assert::assert_true(dv2_dv34.is_cacheable(ctx));
    Assert::assert_true(seg_dv1_dv2_dv34.is_cacheable(ctx));
    Assert::assert_false(dv2_non.is_cacheable(ctx));

    w.update_numeric_doc_value(Term::from_text("text", "text"), "field3", 2)?;
    w.commit()?;
    drop(leaves);
    reader.close()?;
    let reader = directory_reader::open(dir.as_directory())?;

    // after field3 is updated, all composites referring to it should be uncacheable

    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
    let ctx = &leaves[0];
    Assert::assert_true(seg_dv1.is_cacheable(ctx));
    Assert::assert_false(dv34.is_cacheable(ctx));
    Assert::assert_false(dv2_dv34.is_cacheable(ctx));
    Assert::assert_false(dv1_dv3.is_cacheable(ctx));
    Assert::assert_false(seg_dv1_dv2_dv34.is_cacheable(ctx));
    Assert::assert_false(dv12_dv1_dv3.is_cacheable(ctx));

    drop(leaves);
    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}
