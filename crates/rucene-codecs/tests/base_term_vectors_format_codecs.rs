//! The `lucene/codecs` test class that extends `BaseTermVectorsFormatTestCase`
//! (rmp #360, the #352 slice), ported in full with the cases it inherits:
//! `org.apache.lucene.codecs.simpletext.TestSimpleTextTermVectorsFormat`:
//! `getCodec()` returns `new SimpleTextCodec()` and
//! `getReadPastLastPositionExceptionClass()` returns `AssertionError.class`.
//!
//! The suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene_codecs::simpletext::codec::SimpleTextCodec;
use rucene_test_framework::index::base_index_file_format_test_case::{
    BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_term_vectors_format_test_case::{
    self, BaseTermVectorsFormatTestCase,
};

#[path = "../../rucene-test-framework/tests/suite_seeds/mod.rs"]
mod suite_seeds;
use suite_seeds::run_suite;

/// `TestSimpleTextTermVectorsFormat`.
struct TestSimpleTextTermVectorsFormat;

impl BaseIndexFileFormatTestCase for TestSimpleTextTermVectorsFormat {
    /// `getCodec()`: `new SimpleTextCodec()`.
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::new(SimpleTextCodec::new())
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.term_vectors_add_random_fields(doc);
    }

    fn as_term_vectors(&self) -> Option<&dyn BaseTermVectorsFormatTestCase> {
        Some(self)
    }
}

impl BaseTermVectorsFormatTestCase for TestSimpleTextTermVectorsFormat {
    fn get_read_past_last_position_exception_class(&self) -> &'static str {
        "java.lang.AssertionError"
    }
}

fn simple_text_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.simpletext.TestSimpleTextTermVectorsFormat")
        .extends(&base_term_vectors_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestSimpleTextTermVectorsFormat))
        }))
        .build()
}

#[test]
fn test_simple_text_term_vectors_format() {
    run_suite(simple_text_class);
}
