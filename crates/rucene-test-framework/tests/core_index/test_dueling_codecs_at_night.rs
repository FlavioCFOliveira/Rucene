//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDuelingCodecsAtNight.java`.
//!
//! Just like TestDuelingCodecs, only with a lot more documents.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use rucene::error::Result;
use rucene::index::IndexReader;
use rucene_test_framework::util::lucene_test_case::{Nightly, SuppressCodecs};
use rucene_test_framework::util::LuceneTestCase;

use crate::test_dueling_codecs::{create_random_index, instance_test, DuelingCodecsInstance};

/// `TestDuelingCodecsAtNight` (`@Nightly`, `@SuppressCodecs({"Direct"})`, it
/// can be too much for these codecs), which extends `TestDuelingCodecs`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestDuelingCodecsAtNight")
        .extends(&crate::test_dueling_codecs::class_def())
        .annotate(Nightly)
        .annotate(SuppressCodecs {
            value: vec!["Direct".to_string()],
        })
        .constructor(Constructor::no_args(DuelingCodecsInstance::default))
        .method(instance_test("testBigEquals", test_big_equals))
        .build()
}

/// use a big number of documents
/// Port of `TestDuelingCodecsAtNight.testBigEquals`.
fn test_big_equals(inst: &mut DuelingCodecsInstance) -> Result<()> {
    let numdocs = LuceneTestCase::at_least(2000);
    create_random_index(numdocs, inst.left_writer(), inst.seed)?;
    create_random_index(numdocs, inst.right_writer(), inst.seed)?;

    inst.left_reader = Some(inst.left_writer().get_reader()? as Arc<dyn IndexReader>);
    inst.right_reader = Some(inst.right_writer().get_reader()? as Arc<dyn IndexReader>);

    LuceneTestCase::assert_reader_equals(&inst.info, inst.left_reader(), inst.right_reader());
    Ok(())
}
