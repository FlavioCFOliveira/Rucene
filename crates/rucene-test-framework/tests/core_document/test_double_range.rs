//! Port of `lucene/core/src/test/org/apache/lucene/document/TestDoubleRange.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::range_fields::DoubleRange;
use rucene::error::Result;

use crate::core_support::{assert_equals, class, test};

/// `TestDoubleRange`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestDoubleRange",
        vec![test("testToString", test_to_string)],
    )
}

/// Port of `TestDoubleRange.testToString`.
fn test_to_string() -> Result<()> {
    let range = DoubleRange::new("foo", &[0.1, 1.1, 2.1, 3.1], &[0.2, 1.2, 2.2, 3.2])?;
    assert_equals(
        "DoubleRange <foo: [0.1 : 0.2] [1.1 : 1.2] [2.1 : 2.2] [3.1 : 3.2]>",
        range.to_string().as_str(),
    );
    Ok(())
}
