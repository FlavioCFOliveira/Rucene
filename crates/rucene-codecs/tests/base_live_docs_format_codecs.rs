//! The `lucene/codecs` test class that extends `BaseLiveDocsFormatTestCase`
//! (rmp #360, the #352 slice), ported in full with the cases it inherits:
//! `org.apache.lucene.codecs.simpletext.TestSimpleTextLiveDocsFormat`, whose
//! only member is `private final Codec codec = new SimpleTextCodec()`.
//!
//! The suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use rucene::codecs::Codec;
use rucene_codecs::simpletext::codec::SimpleTextCodec;
use rucene_test_framework::index::base_live_docs_format_test_case::{
    self, BaseLiveDocsFormatTestCase, LiveDocsTestInstance,
};

#[path = "../../rucene-test-framework/tests/suite_seeds/mod.rs"]
mod suite_seeds;
use suite_seeds::run_suite;

/// `TestSimpleTextLiveDocsFormat`.
struct TestSimpleTextLiveDocsFormat {
    codec: Arc<dyn Codec>,
}

impl BaseLiveDocsFormatTestCase for TestSimpleTextLiveDocsFormat {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }
}

fn simple_text_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.simpletext.TestSimpleTextLiveDocsFormat")
        .extends(&base_live_docs_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            LiveDocsTestInstance::new(Arc::new(TestSimpleTextLiveDocsFormat {
                codec: Arc::new(SimpleTextCodec::new()),
            }))
        }))
        .build()
}

#[test]
fn test_simple_text_live_docs_format() {
    run_suite(simple_text_class);
}
