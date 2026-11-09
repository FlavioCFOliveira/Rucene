//! The `lucene/codecs` test class that extends `BasePointsFormatTestCase`
//! (rmp #360, the #354 slice), ported in full with the cases it inherits:
//! `org.apache.lucene.codecs.simpletext.TestSimpleTextPointsFormat`, whose
//! only member is `private final Codec codec = new SimpleTextCodec()`.
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
use rucene_test_framework::index::base_points_format_test_case::{self, BasePointsFormatTestCase};

#[path = "../../rucene-test-framework/tests/suite_seeds/mod.rs"]
mod suite_seeds;
use suite_seeds::run_suite;

/// `TestSimpleTextPointsFormat`.
struct TestSimpleTextPointsFormat {
    codec: Arc<dyn Codec>,
}

impl BaseIndexFileFormatTestCase for TestSimpleTextPointsFormat {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.points_add_random_fields(doc);
    }

    fn merge_is_stable(&self) -> bool {
        self.points_merge_is_stable()
    }

    fn as_points(&self) -> Option<&dyn BasePointsFormatTestCase> {
        Some(self)
    }
}

impl BasePointsFormatTestCase for TestSimpleTextPointsFormat {}

fn simple_text_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.simpletext.TestSimpleTextPointsFormat")
        .extends(&base_points_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestSimpleTextPointsFormat {
                codec: Arc::new(SimpleTextCodec::new()),
            }))
        }))
        .build()
}

#[test]
fn test_simple_text_points_format() {
    run_suite(simple_text_class);
}
