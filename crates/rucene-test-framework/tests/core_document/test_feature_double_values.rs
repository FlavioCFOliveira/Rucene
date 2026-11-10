//! Port of `lucene/core/src/test/org/apache/lucene/document/TestFeatureDoubleValues.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::feature_field::{FeatureDoubleValuesSource, FeatureField};
use rucene::document::Document;
use rucene::error::{LuceneError, Result};
use rucene::index::{IndexReader, LeafReaderContext};
use rucene::search::{DoubleValues, DoubleValuesSource, IndexSearcher, SegmentCacheable};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, assert_equals_delta, assert_not_equals, class, random, test,
};

/// `TestFeatureDoubleValues`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestFeatureDoubleValues",
        vec![
            test("testFeature", test_feature),
            test("testFeatureMissing", test_feature_missing),
            test(
                "testFeatureMissingFieldInSegment",
                test_feature_missing_field_in_segment,
            ),
            test(
                "testFeatureMissingFeatureNameInSegment",
                test_feature_missing_feature_name_in_segment,
            ),
            test("testFeatureMultipleMissing", test_feature_multiple_missing),
            test("testHashCodeAndEquals", test_hash_code_and_equals),
        ],
    )
}

/// `newIndexWriterConfig().setMergePolicy(newLogMergePolicy(random().nextBoolean()))`
/// and `new RandomIndexWriter(random(), dir, config)`.
fn writer(dir: &TestDirectory) -> Result<RandomIndexWriter> {
    let mut config = LuceneTestCase::new_index_writer_config();
    config.set_merge_policy(LuceneTestCase::new_log_merge_policy_use_cfs(
        random().next_boolean(),
    ))?;
    RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)
}

/// `new Document()` with an optional `new FeatureField("field", name, value)`.
fn feature_doc(feature: Option<(&str, f32)>) -> Result<Document> {
    let mut doc = Document::new();
    if let Some((name, value)) = feature {
        doc.add(Box::new(FeatureField::new("field", name, value)?));
    }
    Ok(doc)
}

/// Indexes `docs`, then `forceMerge(1)` (or `commit()` when `commit`),
/// `getReader()`, `writer.close()`; checks one leaf and returns
/// `FeatureField.newDoubleValues("field", "name").getValues(context, null)`
/// with the reader to close.
fn values_of(
    dir: &TestDirectory,
    docs: Vec<Option<(&str, f32)>>,
    commit: bool,
) -> Result<(Arc<dyn IndexReader>, Box<dyn DoubleValues>)> {
    let writer = writer(dir)?;
    for d in docs {
        writer.add_document(&feature_doc(d)?)?;
    }
    if commit {
        writer.commit()?;
    } else {
        writer.force_merge(1)?;
    }
    let ir: Arc<dyn IndexReader> = writer.get_reader()?;
    writer.close()?;

    let leaves = Arc::clone(&ir).leaves()?;
    assert_equals(&1, &leaves.len());
    let context = Arc::clone(&leaves[0]);
    let values_source = Arc::new(FeatureField::new_double_values("field", "name"));
    let values = values_source.get_values(&context, None)?;
    Ok((ir, values))
}

/// Port of `testFeature`.
fn test_feature() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let (ir, mut values) = values_of(
        &dir,
        vec![
            Some(("name", 30.0)),
            Some(("name", 1.0)),
            Some(("name", 4.0)),
        ],
        false,
    )?;
    Assert::assert_true(values.advance_exact(0)?);
    assert_equals_delta(30.0, values.double_value()?, 0.0);
    Assert::assert_true(values.advance_exact(1)?);
    assert_equals_delta(1.0, values.double_value()?, 0.0);
    Assert::assert_true(values.advance_exact(2)?);
    assert_equals_delta(4.0, values.double_value()?, 0.0);
    Assert::assert_false(values.advance_exact(3)?);
    drop(values);
    ir.close()?;
    dir.close()
}

/// Port of `testFeatureMissing`.
fn test_feature_missing() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let (ir, mut values) = values_of(
        &dir,
        vec![None, Some(("name", 1.0)), Some(("name", 4.0))],
        false,
    )?;
    Assert::assert_false(values.advance_exact(0)?);
    Assert::assert_true(values.advance_exact(1)?);
    assert_equals_delta(1.0, values.double_value()?, 0.0);
    Assert::assert_true(values.advance_exact(2)?);
    assert_equals_delta(4.0, values.double_value()?, 0.0);
    Assert::assert_false(values.advance_exact(3)?);
    drop(values);
    ir.close()?;
    dir.close()
}

/// Port of `testFeatureMissingFieldInSegment`.
fn test_feature_missing_field_in_segment() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let (ir, mut values) = values_of(&dir, vec![None], true)?;
    Assert::assert_false(values.advance_exact(0)?);
    Assert::assert_false(values.advance_exact(1)?);
    drop(values);
    ir.close()?;
    dir.close()
}

/// Port of `testFeatureMissingFeatureNameInSegment`.
fn test_feature_missing_feature_name_in_segment() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let (ir, mut values) = values_of(&dir, vec![Some(("different_name", 0.5))], true)?;
    Assert::assert_false(values.advance_exact(0)?);
    Assert::assert_false(values.advance_exact(1)?);
    drop(values);
    ir.close()?;
    dir.close()
}

/// Port of `testFeatureMultipleMissing`.
fn test_feature_multiple_missing() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let (ir, mut values) = values_of(
        &dir,
        vec![
            None,
            None,
            None,
            None,
            None,
            Some(("name", 1.0)),
            Some(("name", 4.0)),
        ],
        false,
    )?;
    for doc in 0..5 {
        Assert::assert_false(values.advance_exact(doc)?);
    }
    Assert::assert_true(values.advance_exact(5)?);
    assert_equals_delta(1.0, values.double_value()?, 0.0);
    Assert::assert_true(values.advance_exact(6)?);
    assert_equals_delta(4.0, values.double_value()?, 0.0);
    Assert::assert_false(values.advance_exact(7)?);
    drop(values);
    ir.close()?;
    dir.close()
}

/// The anonymous `DoubleValuesSource` of `testHashCodeAndEquals`.
#[derive(Debug)]
struct OtherImpl;

impl SegmentCacheable for OtherImpl {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl DoubleValuesSource for OtherImpl {
    /// Java returns `null`; the port's signature has no null, so the call
    /// (never made by the test) fails instead.
    fn get_values<'a>(
        self: Arc<Self>,
        _ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        Err(LuceneError::UnsupportedOperation(
            "getValues returns null".to_string(),
        ))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    /// Java returns `null`; see [`get_values`](Self::get_values).
    fn rewrite(self: Arc<Self>, _searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Err(LuceneError::UnsupportedOperation(
            "rewrite returns null".to_string(),
        ))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn source_eq(&self, _other: &dyn DoubleValuesSource) -> bool {
        false
    }

    fn source_hash(&self) -> u64 {
        0
    }

    /// Java's `toString()` returns `null`; the port's `String` cannot, so
    /// it returns `"null"`, what `String.valueOf` would print.
    fn to_source_string(&self) -> String {
        "null".to_string()
    }
}

/// Port of `testHashCodeAndEquals`.
fn test_hash_code_and_equals() -> Result<()> {
    let values_source = FeatureDoubleValuesSource::new("test_field", "test_feature");
    let equal = FeatureDoubleValuesSource::new("test_field", "test_feature");
    let different_field = FeatureDoubleValuesSource::new("other field", "test_feature");
    let different_feature = FeatureDoubleValuesSource::new("test_field", "other_feature");
    let other_impl = OtherImpl;

    Assert::assert_true(values_source.source_eq(&equal));
    assert_equals(&values_source.source_hash(), &equal.source_hash());
    // `valuesSource.equals(null)`: the port's `source_eq` takes no null; the
    // Java `equals` returns `false` for it (`obj == null` check), which the
    // `Option` below states.
    let null: Option<&dyn DoubleValuesSource> = None;
    Assert::assert_false(null.is_some_and(|o| values_source.source_eq(o)));
    Assert::assert_false(values_source.source_eq(&other_impl));
    assert_not_equals(&values_source.source_hash(), &other_impl.source_hash());
    Assert::assert_false(values_source.source_eq(&different_field));
    assert_not_equals(&values_source.source_hash(), &different_field.source_hash());
    Assert::assert_false(values_source.source_eq(&different_feature));
    assert_not_equals(
        &values_source.source_hash(),
        &different_feature.source_hash(),
    );
    Ok(())
}
