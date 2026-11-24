//! Port of the `lucene/core/src/test/org/apache/lucene/analysis/tokenattributes/`
//! test classes `TestSimpleAttributeImpl`, `TestBytesRefAttImpl`,
//! `TestCharTermAttributeImpl` and `TestPackedTokenAttributeImpl` of Lucene
//! 10.5.0, each run as a suite by the ported runner. The last three are in
//! `core_analysis_tokenattributes/`.

mod core_analysis_tokenattributes;
mod lucene_core_shims;
mod suite_seeds;

use std::collections::HashMap;
use std::sync::Arc;

use lucene_core_shims::{lucene_class, run, t};
use randomizedtesting::jdk::reflect::Class;
use rucene::analysis::tokenattributes::{
    FlagsAttributeImpl, KeywordAttributeImpl, OffsetAttributeImpl, PayloadAttributeImpl,
    PositionIncrementAttributeImpl, PositionLengthAttributeImpl, TypeAttributeImpl, DEFAULT_TYPE,
};
use rucene::error::Result;
use rucene_test_framework::util::TestUtil;

/// `Collections.singletonMap(Attribute.class.getName() + "#" + key, value)`.
///
/// `attribute` is the interface's simple name in
/// `org.apache.lucene.analysis.tokenattributes.`.
///
/// **Divergence from Lucene 10.5.0.** Java's map holds the value objects;
/// `TestUtil.assertAttributeReflection` compares each value's `Debug`
/// rendering, so the expected map is valued the same way.
fn single(attribute: &str, key: &str, value: impl std::fmt::Debug) -> HashMap<String, String> {
    HashMap::from([(
        format!("org.apache.lucene.analysis.tokenattributes.{attribute}#{key}"),
        format!("{value:?}"),
    )])
}

/// `TestSimpleAttributeImpl.testAttributes()`: checks using reflection API
/// if the defaults are correct.
fn simple_attribute_impl_test_attributes() -> Result<()> {
    TestUtil::assert_attribute_reflection(
        &PositionIncrementAttributeImpl::default(),
        &single("PositionIncrementAttribute", "positionIncrement", 1),
    );
    TestUtil::assert_attribute_reflection(
        &PositionLengthAttributeImpl::default(),
        &single("PositionLengthAttribute", "positionLength", 1),
    );
    TestUtil::assert_attribute_reflection(
        &FlagsAttributeImpl::default(),
        &single("FlagsAttribute", "flags", 0),
    );
    TestUtil::assert_attribute_reflection(
        &TypeAttributeImpl::default(),
        &single("TypeAttribute", "type", DEFAULT_TYPE),
    );
    TestUtil::assert_attribute_reflection(
        &PayloadAttributeImpl::default(),
        &single("PayloadAttribute", "payload", None::<()>),
    );
    TestUtil::assert_attribute_reflection(
        &KeywordAttributeImpl::default(),
        &single("KeywordAttribute", "keyword", false),
    );
    let offset = "org.apache.lucene.analysis.tokenattributes.OffsetAttribute";
    TestUtil::assert_attribute_reflection(
        &OffsetAttributeImpl::default(),
        &HashMap::from([
            (format!("{offset}#startOffset"), "0".to_string()),
            (format!("{offset}#endOffset"), "0".to_string()),
        ]),
    );
    Ok(())
}

fn simple_attribute_impl_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.tokenattributes.TestSimpleAttributeImpl")
        .method(t("testAttributes", simple_attribute_impl_test_attributes))
        .build()
}

#[test]
fn test_simple_attribute_impl() {
    run(simple_attribute_impl_class);
}

#[test]
fn test_bytes_ref_att_impl() {
    run(core_analysis_tokenattributes::test_bytes_ref_att_impl::class);
}

#[test]
fn test_char_term_attribute_impl() {
    run(core_analysis_tokenattributes::test_char_term_attribute_impl::class);
}

#[test]
fn test_packed_token_attribute_impl() {
    run(core_analysis_tokenattributes::test_packed_token_attribute_impl::class);
}
