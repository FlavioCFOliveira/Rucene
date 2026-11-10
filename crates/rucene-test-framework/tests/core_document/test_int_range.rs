//! Port of `lucene/core/src/test/org/apache/lucene/document/TestIntRange.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::range_fields::IntRange;
use rucene::error::Result;

use crate::core_support::{assert_equals, class, test};

/// `TestIntRange`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestIntRange",
        vec![test("testToString", test_to_string)],
    )
}

/// Port of `TestIntRange.testToString`.
fn test_to_string() -> Result<()> {
    let range = IntRange::new("foo", &[1, 11, 21, 31], &[2, 12, 22, 32])?;
    assert_equals(
        "IntRange <foo: [1 : 2] [11 : 12] [21 : 22] [31 : 32]>",
        range.to_string().as_str(),
    );
    Ok(())
}
