//! The `lucene/codecs` test class that extends `BaseNormsFormatTestCase`
//! (rmp #360, the #352 slice), ported in full with the cases it inherits:
//! `org.apache.lucene.codecs.simpletext.TestSimpleTextNormsFormat`, whose
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
use rucene_test_framework::index::base_norms_format_test_case::{self, BaseNormsFormatTestCase};

#[path = "../../rucene-test-framework/tests/suite_seeds/mod.rs"]
mod suite_seeds;
use suite_seeds::run_suite;

/// `TestSimpleTextNormsFormat`.
struct TestSimpleTextNormsFormat {
    codec: Arc<dyn Codec>,
}

impl BaseIndexFileFormatTestCase for TestSimpleTextNormsFormat {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.norms_add_random_fields(doc);
    }

    fn as_norms(&self) -> Option<&dyn BaseNormsFormatTestCase> {
        Some(self)
    }
}

impl BaseNormsFormatTestCase for TestSimpleTextNormsFormat {}

fn simple_text_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.simpletext.TestSimpleTextNormsFormat")
        .extends(&base_norms_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestSimpleTextNormsFormat {
                codec: Arc::new(SimpleTextCodec::new()),
            }))
        }))
        .build()
}

#[test]
fn test_simple_text_norms_format() {
    run_suite(simple_text_class);
}
