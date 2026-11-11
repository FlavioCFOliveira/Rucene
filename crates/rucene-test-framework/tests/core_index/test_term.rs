//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTerm.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::terms::Term;

use crate::core_support::{assert_equals, class, test};

/// `TestTerm`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestTerm",
        vec![test("testEquals", test_equals)],
    )
}

/// Port of `TestTerm.testEquals`.
///
/// **Divergence.** Java's `base.equals(differentType)` compares a `Term`
/// with a `String` (`TestTerm.java:32`); Rust's `PartialEq` is typed, so the
/// comparison with an object of another type is the `Any` downcast that
/// `Term.equals` performs first (`obj.getClass() != getClass()`), which fails.
fn test_equals() -> Result<()> {
    let base = Term::from_text("same", "same");
    let same = Term::from_text("same", "same");
    let different_field = Term::from_text("different", "same");
    let different_text = Term::from_text("same", "different");
    let different_type: &dyn std::any::Any = &"AString";
    assert_equals(&base, &base);
    assert_equals(&base, &same);
    Assert::assert_false(base == different_field);
    Assert::assert_false(base == different_text);
    Assert::assert_false(different_type.downcast_ref::<Term>() == Some(&base));
    Ok(())
}
