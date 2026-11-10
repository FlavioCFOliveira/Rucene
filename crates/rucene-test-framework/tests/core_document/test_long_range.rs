//! Port of `lucene/core/src/test/org/apache/lucene/document/TestLongRange.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::range_fields::LongRange;
use rucene::error::Result;

use crate::core_support::{assert_equals, class, test};

/// `TestLongRange`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestLongRange",
        vec![test("testToString", test_to_string)],
    )
}

/// Port of `TestLongRange.testToString`.
fn test_to_string() -> Result<()> {
    let range = LongRange::new("foo", &[1, 11, 21, 31], &[2, 12, 22, 32])?;
    assert_equals(
        "LongRange <foo: [1 : 2] [11 : 12] [21 : 22] [31 : 32]>",
        range.to_string().as_str(),
    );
    Ok(())
}
