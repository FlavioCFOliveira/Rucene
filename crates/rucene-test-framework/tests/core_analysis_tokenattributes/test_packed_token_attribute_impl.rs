//! Port of `org.apache.lucene.analysis.tokenattributes.TestPackedTokenAttributeImpl`.
//!
//! Java's `PackedTokenAttributeImpl.toString()`, inherited from
//! `CharTermAttributeImpl`, is the term: `term()` here.

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::tokenattributes::{
    CharTermAttribute, CharTermAttributeImpl, FlagsAttributeImpl, OffsetAttribute,
    PackedTokenAttributeImpl, PositionIncrementAttribute, PositionLengthAttribute,
    TermFrequencyAttribute, TypeAttribute,
};
use rucene::analysis::{default_token_attribute_factory, StringCharReader};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::attributes::{
    char_term_id, flags_id, offset_id, pos_inc_id, type_id, Att,
};
use rucene_test_framework::analysis::mock_tokenizer::{DEFAULT_MAX_TOKEN_LENGTH, WHITESPACE};
use rucene_test_framework::analysis::MockTokenizer;
use rucene_test_framework::util::TestUtil;

use super::test_char_term_attribute_impl::{assert_clone_is_equal, assert_copy_is_equal};
use crate::lucene_core_shims::{lucene_class, t};

/* the CharTermAttributeStuff is tested by TestCharTermAttributeImpl */

fn test_clone() -> Result<()> {
    let mut t = PackedTokenAttributeImpl::new();
    t.set_offset(0, 5);
    let content: Vec<char> = "hello".chars().collect();
    t.copy_buffer(&content, 0, 5);
    let buf = t.buffer().as_ptr();
    let copy = assert_clone_is_equal(&t);
    Assert::assert_equals_str(None, Some(&t.term()), Some(&copy.term()));
    // `assertNotSame(buf, copy.buffer())`.
    Assert::assert_true(buf != copy.buffer().as_ptr());
    Ok(())
}

fn test_copy_to() -> Result<()> {
    let t = PackedTokenAttributeImpl::new();
    let copy = assert_copy_is_equal(&t);
    Assert::assert_equals_str(None, Some(""), Some(&t.term()));
    Assert::assert_equals_str(None, Some(""), Some(&copy.term()));

    let mut t = PackedTokenAttributeImpl::new();
    t.set_offset(0, 5);
    let content: Vec<char> = "hello".chars().collect();
    t.copy_buffer(&content, 0, 5);
    let buf = t.buffer().as_ptr();
    let copy = assert_copy_is_equal(&t);
    Assert::assert_equals_str(None, Some(&t.term()), Some(&copy.term()));
    // `assertNotSame(buf, copy.buffer())`.
    Assert::assert_true(buf != copy.buffer().as_ptr());
    Ok(())
}

/// `ts.addAttribute(X.class) instanceof Impl`.
fn is<T: 'static>(att: &Att) -> bool {
    att.implementation().as_any().is::<T>()
}

fn test_packed_token_attribute_factory() -> Result<()> {
    let mut ts = MockTokenizer::with_factory(
        default_token_attribute_factory(),
        Arc::clone(&WHITESPACE),
        false,
        DEFAULT_MAX_TOKEN_LENGTH,
    )?;
    ts.set_reader(Box::new(StringCharReader::new("foo bar")))?;
    let source: &mut AttributeSource = rucene::analysis::TokenStream::attribute_source_mut(&mut ts);

    Assert::assert_true_msg(
        "CharTermAttribute is not implemented by Token",
        is::<PackedTokenAttributeImpl>(&Att::add(source, char_term_id())?),
    );
    Assert::assert_true_msg(
        "OffsetAttribute is not implemented by Token",
        is::<PackedTokenAttributeImpl>(&Att::add(source, offset_id())?),
    );
    Assert::assert_true_msg(
        "PositionIncrementAttribute is not implemented by Token",
        is::<PackedTokenAttributeImpl>(&Att::add(source, pos_inc_id())?),
    );
    Assert::assert_true_msg(
        "TypeAttribute is not implemented by Token",
        is::<PackedTokenAttributeImpl>(&Att::add(source, type_id())?),
    );

    Assert::assert_true_msg(
        "FlagsAttribute is not implemented by FlagsAttributeImpl",
        is::<FlagsAttributeImpl>(&Att::add(source, flags_id())?),
    );
    Ok(())
}

/// The reflected map is keyed and valued as Rucene's `AttributeReflector`
/// reports them (see `TestUtil.assertAttributeReflection`): the
/// `CharTermAttribute` and `TermToBytesRefAttribute` keys under
/// `CharTermAttributeImpl`'s type name, the others under
/// `PackedTokenAttributeImpl`'s.
fn test_attribute_reflection() -> Result<()> {
    let mut t = PackedTokenAttributeImpl::new();
    t.append_string(Some("foobar"));
    t.set_offset(6, 22);
    t.set_position_increment(3);
    t.set_position_length(11);
    t.set_type("foobar".to_string());
    t.set_term_frequency(42);
    let packed = std::any::type_name::<PackedTokenAttributeImpl>();
    let term = std::any::type_name::<CharTermAttributeImpl>();
    let mut expected: HashMap<String, String> = HashMap::new();
    expected.insert(format!("{term}#term"), format!("{:?}", "foobar"));
    expected.insert(
        format!("{term}#bytes"),
        format!("{:?}", BytesRef::new(b"foobar".to_vec())),
    );
    expected.insert(format!("{packed}#startOffset"), "6".to_string());
    expected.insert(format!("{packed}#endOffset"), "22".to_string());
    expected.insert(format!("{packed}#positionIncrement"), "3".to_string());
    expected.insert(format!("{packed}#positionLength"), "11".to_string());
    expected.insert(format!("{packed}#type"), format!("{:?}", "foobar"));
    expected.insert(format!("{packed}#termFrequency"), "42".to_string());
    TestUtil::assert_attribute_reflection(&t, &expected);
    Ok(())
}

/// The class `org.apache.lucene.analysis.tokenattributes.TestPackedTokenAttributeImpl`.
pub fn class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.tokenattributes.TestPackedTokenAttributeImpl")
        .method(t("testClone", test_clone))
        .method(t("testCopyTo", test_copy_to))
        .method(t(
            "testPackedTokenAttributeFactory",
            test_packed_token_attribute_factory,
        ))
        .method(t("testAttributeReflection", test_attribute_reflection))
        .build()
}
