//! Port of `org.apache.lucene.analysis.tokenattributes.TestBytesRefAttImpl`.
//!
//! Java's `getBytesRef()` returns the attribute's field, `null` while it is
//! unset; the port reads it through `BytesTermAttributeImpl::bytes_ref_field`
//! (`None` for `null`), whose references the identity check compares.

use std::any::TypeId;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::tokenattributes::{
    BytesTermAttribute, BytesTermAttributeImpl, TermToBytesRefAttribute,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeImpl;
use rucene::util::BytesRef;

use super::test_char_term_attribute_impl::assert_copy_is_equal;
use crate::lucene_core_shims::{lucene_class, t};

fn test_copy_to() -> Result<()> {
    let mut t = BytesTermAttributeImpl::new();
    let mut copy = assert_copy_is_equal(&t);

    // first do empty
    Assert::assert_true(t.bytes_ref_field() == copy.bytes_ref_field());
    Assert::assert_true(copy.bytes_ref_field().is_none());
    // now after setting it
    t.set_bytes_ref(BytesRef::new(b"hello".to_vec()));
    copy = assert_copy_is_equal(&t);
    Assert::assert_true(t.bytes_ref_field() == copy.bytes_ref_field());
    // `assertNotSame(t.getBytesRef(), copy.getBytesRef())`
    Assert::assert_false(std::ptr::eq(
        t.bytes_ref_field().expect("set"),
        copy.bytes_ref_field().expect("copied"),
    ));
    Ok(())
}

/// `testLucene9856()`.
///
/// Java's `BytesTermAttributeImpl.class.getInterfaces()` is the list of
/// interfaces the class declares; the port's counterpart is the attribute's
/// `attribute_interfaces()`.
fn test_lucene9856() -> Result<()> {
    Assert::assert_true_msg(
        "BytesTermAttributeImpl must explicitly declare to implement TermToBytesRefAttribute",
        BytesTermAttributeImpl::new()
            .attribute_interfaces()
            .contains(&TypeId::of::<dyn TermToBytesRefAttribute>()),
    );
    Ok(())
}

/// The class `org.apache.lucene.analysis.tokenattributes.TestBytesRefAttImpl`.
pub fn class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.tokenattributes.TestBytesRefAttImpl")
        .method(t("testCopyTo", test_copy_to))
        .method(t("testLucene9856", test_lucene9856))
        .build()
}
