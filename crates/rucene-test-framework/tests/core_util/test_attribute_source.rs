//! Port of `lucene/core/src/test/org/apache/lucene/util/TestAttributeSource.java`.
//!
//! Java keeps references to attribute instances (`termAtt`, `typeAtt`, …);
//! Rucene's `AttributeSource` lends them through guards, so each access goes
//! through the source the Java reference was taken from. `Class` objects are
//! `TypeId`s.

use std::any::TypeId;
use std::rc::Rc;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    default_token_attribute_factory, CharTermAttribute, CharTermAttributeImpl, FlagsAttribute,
    FlagsAttributeImpl, OffsetAttribute, OffsetAttributeImpl, PayloadAttribute,
    PayloadAttributeImpl, PositionIncrementAttribute, PositionIncrementAttributeImpl,
    TypeAttribute, TypeAttributeImpl,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeCell;
use rucene::util::{AttributeImpl, AttributeSource, BytesRef};
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_support::{assert_equals, assert_equals_msg, assert_not_null, class, test};

const IAE: &str = "java.lang.IllegalArgumentException";

/// `TestAttributeSource`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestAttributeSource",
        vec![
            test("testCaptureState", test_capture_state),
            test("testCloneAttributes", test_clone_attributes),
            test(
                "testDefaultAttributeFactory",
                test_default_attribute_factory,
            ),
            test("testInvalidArguments", test_invalid_arguments),
            test("testLUCENE_3042", test_lucene_3042),
            test("testClonePayloadAttribute", test_clone_payload_attribute),
            test("testRemoveAllAttributes", test_remove_all_attributes),
        ],
    )
}

/// `termAtt.toString()` of the source's `CharTermAttribute`.
fn term(src: &AttributeSource) -> String {
    let term_att = src
        .get_attribute_interface::<dyn CharTermAttribute>()
        .expect("CharTermAttribute");
    term_att.buffer()[..term_att.length()].iter().collect()
}

/// `typeAtt.type()` of the source's `TypeAttribute`.
fn type_of(src: &AttributeSource) -> String {
    src.get_attribute_interface::<dyn TypeAttribute>()
        .expect("TypeAttribute")
        .type_value()
        .to_string()
}

/// `typeAtt.setType(type)` on the source's `TypeAttribute`.
fn set_type(src: &AttributeSource, t: &str) {
    src.get_attribute_interface_mut::<dyn TypeAttribute>()
        .expect("TypeAttribute")
        .set_type(t.to_string());
}

/// `flagsAtt.getFlags()` of the source's `FlagsAttribute`.
fn flags_of(src: &AttributeSource) -> i32 {
    src.get_attribute_interface::<dyn FlagsAttribute>()
        .expect("FlagsAttribute")
        .get_flags()
}

/// `flagsAtt.setFlags(flags)` on the source's `FlagsAttribute`.
fn set_flags(src: &AttributeSource, flags: i32) {
    src.get_attribute_interface_mut::<dyn FlagsAttribute>()
        .expect("FlagsAttribute")
        .set_flags(flags);
}

/// `assertNotSame(message, unexpected, actual)` over attribute instances.
fn assert_not_same(message: &str, unexpected: &AttributeCell, actual: &AttributeCell) {
    if Rc::ptr_eq(unexpected, actual) {
        Assert::fail(&format!("{message} expected not same"))
    }
}

/// `assertEquals(message, expected, actual)` over attribute instances
/// (`AttributeImpl.equals`).
fn assert_impl_equals(message: &str, expected: &AttributeCell, actual: &AttributeCell) {
    let e = expected.borrow();
    let a = actual.borrow();
    if !e.equals(&**a) {
        Assert::fail(&format!("{message} expected:<{e:?}> but was:<{a:?}>"))
    }
}

fn test_capture_state() -> Result<()> {
    // init a first instance
    let mut src = AttributeSource::new();
    src.add_attribute_interface::<dyn CharTermAttribute>()?;
    src.add_attribute_interface::<dyn TypeAttribute>()?;
    src.get_attribute_interface_mut::<dyn CharTermAttribute>()
        .expect("CharTermAttribute")
        .append_string("TestTerm");
    set_type(&src, "TestType");
    let hash_code = src.hash_code();

    let state = src.capture_state().expect("state");

    // modify the attributes
    {
        let mut term_att = src
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("CharTermAttribute");
        term_att.set_empty();
        term_att.append_string("AnotherTestTerm");
    }
    set_type(&src, "AnotherTestType");
    Assert::assert_true_msg(
        "Hash code should be different",
        hash_code != src.hash_code(),
    );

    src.restore_state(&state)?;
    assert_equals("TestTerm", term(&src).as_str());
    assert_equals("TestType", type_of(&src).as_str());
    assert_equals_msg(
        Some("Hash code should be equal after restore"),
        &hash_code,
        &src.hash_code(),
    );

    // restore into an exact configured copy
    let mut copy = AttributeSource::new();
    copy.add_attribute_interface::<dyn CharTermAttribute>()?;
    copy.add_attribute_interface::<dyn TypeAttribute>()?;
    copy.restore_state(&state)?;
    assert_equals_msg(
        Some("Both AttributeSources should have same hashCode after restore"),
        &src.hash_code(),
        &copy.hash_code(),
    );
    assert_equals_msg(
        Some("Both AttributeSources should be equal after restore"),
        &src,
        &copy,
    );

    // init a second instance (with attributes in different order and one additional attribute)
    let mut src2 = AttributeSource::new();
    src2.add_attribute_interface::<dyn TypeAttribute>()?;
    src2.add_attribute_interface::<dyn FlagsAttribute>()?;
    src2.add_attribute_interface::<dyn CharTermAttribute>()?;
    set_flags(&src2, 12345);

    src2.restore_state(&state)?;
    assert_equals("TestTerm", term(&src2).as_str());
    assert_equals("TestType", type_of(&src2).as_str());
    assert_equals_msg(
        Some("FlagsAttribute should not be touched"),
        &12345,
        &flags_of(&src2),
    );

    // init a third instance missing one Attribute
    let mut src3 = AttributeSource::new();
    src3.add_attribute_interface::<dyn CharTermAttribute>()?;
    // The third instance is missing the TypeAttribute, so restoreState() should throw
    // IllegalArgumentException
    expect_throws(IAE, || src3.restore_state(&state));
    Ok(())
}

fn test_clone_attributes() -> Result<()> {
    let mut src = AttributeSource::new();
    let flags_att = src.add_attribute_by_id(TypeId::of::<dyn FlagsAttribute>())?;
    let type_att = src.add_attribute_by_id(TypeId::of::<dyn TypeAttribute>())?;
    set_flags(&src, 1234);
    set_type(&src, "TestType");

    let clone = src.clone_attributes();
    let mut it = clone.attribute_classes();
    assert_equals_msg(
        Some("FlagsAttribute must be the first attribute"),
        &Some(TypeId::of::<dyn FlagsAttribute>()),
        &it.next(),
    );
    assert_equals_msg(
        Some("TypeAttribute must be the second attribute"),
        &Some(TypeId::of::<dyn TypeAttribute>()),
        &it.next(),
    );
    Assert::assert_false_msg("No more attributes", it.next().is_some());

    let flags_att2 = clone.get_attribute_by_id(TypeId::of::<dyn FlagsAttribute>());
    assert_not_null(&flags_att2);
    let type_att2 = clone.get_attribute_by_id(TypeId::of::<dyn TypeAttribute>());
    assert_not_null(&type_att2);
    let flags_att2 = flags_att2.unwrap();
    let type_att2 = type_att2.unwrap();
    assert_not_same(
        "FlagsAttribute of original and clone must be different instances",
        &flags_att2,
        &flags_att,
    );
    assert_not_same(
        "TypeAttribute of original and clone must be different instances",
        &type_att2,
        &type_att,
    );
    assert_impl_equals(
        "FlagsAttribute of original and clone must be equal",
        &flags_att2,
        &flags_att,
    );
    assert_impl_equals(
        "TypeAttribute of original and clone must be equal",
        &type_att2,
        &type_att,
    );

    // test copy back
    set_flags(&clone, 4711);
    set_type(&clone, "OtherType");
    clone.copy_to(&src)?;
    assert_equals_msg(
        Some("FlagsAttribute of original must now contain updated term"),
        &4711,
        &flags_of(&src),
    );
    assert_equals_msg(
        Some("TypeAttribute of original must now contain updated type"),
        "OtherType",
        type_of(&src).as_str(),
    );
    // verify again:
    assert_not_same(
        "FlagsAttribute of original and clone must be different instances",
        &flags_att2,
        &flags_att,
    );
    assert_not_same(
        "TypeAttribute of original and clone must be different instances",
        &type_att2,
        &type_att,
    );
    assert_impl_equals(
        "FlagsAttribute of original and clone must be equal",
        &flags_att2,
        &flags_att,
    );
    assert_impl_equals(
        "TypeAttribute of original and clone must be equal",
        &type_att2,
        &type_att,
    );
    Ok(())
}

/// `src.addAttribute(I.class) instanceof Impl`.
fn added_is<Impl: 'static>(src: &mut AttributeSource, id: TypeId) -> Result<bool> {
    let cell = src.add_attribute_by_id(id)?;
    let is = cell.borrow().as_any().is::<Impl>();
    Ok(is)
}

fn test_default_attribute_factory() -> Result<()> {
    let mut src = AttributeSource::new();

    Assert::assert_true_msg(
        "CharTermAttribute is not implemented by CharTermAttributeImpl",
        added_is::<CharTermAttributeImpl>(&mut src, TypeId::of::<dyn CharTermAttribute>())?,
    );
    Assert::assert_true_msg(
        "OffsetAttribute is not implemented by OffsetAttributeImpl",
        added_is::<OffsetAttributeImpl>(&mut src, TypeId::of::<dyn OffsetAttribute>())?,
    );
    Assert::assert_true_msg(
        "FlagsAttribute is not implemented by FlagsAttributeImpl",
        added_is::<FlagsAttributeImpl>(&mut src, TypeId::of::<dyn FlagsAttribute>())?,
    );
    Assert::assert_true_msg(
        "PayloadAttribute is not implemented by PayloadAttributeImpl",
        added_is::<PayloadAttributeImpl>(&mut src, TypeId::of::<dyn PayloadAttribute>())?,
    );
    Assert::assert_true_msg(
        "PositionIncrementAttribute is not implemented by PositionIncrementAttributeImpl",
        added_is::<PositionIncrementAttributeImpl>(
            &mut src,
            TypeId::of::<dyn PositionIncrementAttribute>(),
        )?,
    );
    Assert::assert_true_msg(
        "TypeAttribute is not implemented by TypeAttributeImpl",
        added_is::<TypeAttributeImpl>(&mut src, TypeId::of::<dyn TypeAttribute>())?,
    );
    Ok(())
}

fn test_invalid_arguments() -> Result<()> {
    expect_throws(IAE, || {
        let mut src = AttributeSource::new();
        src.add_attribute_by_id(TypeId::of::<Token>())
    });

    expect_throws(IAE, || {
        let mut src = AttributeSource::new_with_factory(Token::token_attribute_factory());
        src.add_attribute_by_id(TypeId::of::<Token>())
    });

    expect_throws(IAE, || {
        let mut src = AttributeSource::new();
        // break this by unsafe cast
        src.add_attribute_by_id(TypeId::of::<dyn Iterator<Item = ()>>())
    });
    Ok(())
}

fn test_lucene_3042() -> Result<()> {
    let mut src1 = AttributeSource::new();
    src1.add_attribute_interface::<dyn CharTermAttribute>()?;
    src1.get_attribute_interface_mut::<dyn CharTermAttribute>()
        .expect("CharTermAttribute")
        .append_string("foo");
    let hash1 = src1.hash_code(); // this triggers a cached state
    let mut src2 = AttributeSource::new_from(&src1);
    src2.add_attribute_interface::<dyn TypeAttribute>()?;
    set_type(&src2, "bar");
    Assert::assert_true_msg(
        "The hashCode is identical, so the captured state was preserved.",
        hash1 != src1.hash_code(),
    );
    assert_equals(&src2.hash_code(), &src1.hash_code());
    Ok(())
}

/// `payloadAtt.getPayload().bytes[0] = 10`: a Rust `BytesRef` cannot be
/// shared, so the payload is taken, modified and set back.
fn set_first_payload_byte(att: &mut PayloadAttributeImpl, b: u8) {
    let mut payload = att.get_payload().cloned().expect("payload");
    payload.bytes[0] = b;
    att.set_payload(Some(payload));
}

fn test_clone_payload_attribute() -> Result<()> {
    // LUCENE-6055: verify that PayloadAttribute.clone() does deep cloning.
    let src = PayloadAttributeImpl::with_payload(BytesRef::new(vec![1, 2, 3]));

    // test clone()
    let mut clone_box = src.clone_box();
    let clone = clone_box
        .as_any_mut()
        .downcast_mut::<PayloadAttributeImpl>()
        .expect("PayloadAttributeImpl");
    set_first_payload_byte(clone, 10); // modify one byte, srcBytes shouldn't change
    assert_equals_msg(
        Some("clone() wasn't deep"),
        &1,
        &src.get_payload().expect("payload").bytes[0],
    );

    // test copyTo()
    let mut clone = PayloadAttributeImpl::new();
    src.copy_to(&mut clone);
    set_first_payload_byte(&mut clone, 10); // modify one byte, srcBytes shouldn't change
    assert_equals_msg(
        Some("clone() wasn't deep"),
        &1,
        &src.get_payload().expect("payload").bytes[0],
    );
    Ok(())
}

fn test_remove_all_attributes() -> Result<()> {
    let attr_classes: Vec<(TypeId, &str)> = vec![
        (TypeId::of::<dyn CharTermAttribute>(), "CharTermAttribute"),
        (TypeId::of::<dyn OffsetAttribute>(), "OffsetAttribute"),
        (TypeId::of::<dyn FlagsAttribute>(), "FlagsAttribute"),
        (TypeId::of::<dyn PayloadAttribute>(), "PayloadAttribute"),
        (
            TypeId::of::<dyn PositionIncrementAttribute>(),
            "PositionIncrementAttribute",
        ),
        (TypeId::of::<dyn TypeAttribute>(), "TypeAttribute"),
    ];

    // Add attributes with the default factory, then try to remove all of them
    let mut default_factory_attribute_source = AttributeSource::new();

    Assert::assert_false(default_factory_attribute_source.has_attributes());

    for (attr_class, simple_name) in &attr_classes {
        default_factory_attribute_source.add_attribute_by_id(*attr_class)?;
        Assert::assert_true_msg(
            &format!("Missing added attribute {simple_name}"),
            default_factory_attribute_source.has_attribute_by_id(*attr_class),
        );
    }

    default_factory_attribute_source.remove_all_attributes();

    for (attr_class, simple_name) in &attr_classes {
        Assert::assert_false_msg(
            &format!("Didn't remove attribute {simple_name}"),
            default_factory_attribute_source.has_attribute_by_id(*attr_class),
        );
    }
    Assert::assert_false(default_factory_attribute_source.has_attributes());

    // Add attributes with the packed implementations factory, then try to remove all of them
    let mut packed_impls_attribute_source =
        AttributeSource::new_with_factory(default_token_attribute_factory());
    Assert::assert_false(packed_impls_attribute_source.has_attributes());

    for (attr_class, simple_name) in &attr_classes {
        packed_impls_attribute_source.add_attribute_by_id(*attr_class)?;
        Assert::assert_true_msg(
            &format!("Missing added attribute {simple_name}"),
            packed_impls_attribute_source.has_attribute_by_id(*attr_class),
        );
    }

    packed_impls_attribute_source.remove_all_attributes();

    for (attr_class, simple_name) in &attr_classes {
        Assert::assert_false_msg(
            &format!("Didn't remove attribute {simple_name}"),
            packed_impls_attribute_source.has_attribute_by_id(*attr_class),
        );
    }
    Assert::assert_false(packed_impls_attribute_source.has_attributes());
    Ok(())
}
