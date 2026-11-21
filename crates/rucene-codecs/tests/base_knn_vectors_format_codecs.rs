//! The `lucene/codecs` test class that extends `BaseKnnVectorsFormatTestCase`
//! (rmp #360, the #354 slice), ported in full with the cases it inherits:
//! `org.apache.lucene.codecs.simpletext.TestSimpleTextKnnVectorsFormat`,
//! whose `getCodec()` returns `new SimpleTextCodec()`, whose
//! `randomVectorEncoding()` returns `FLOAT32` ("Should
//! SimpleTextKnnVectorsFormat support all encodings?"), which turns
//! `testRandomBytes` and `testSortedIndexBytes` into no-ops (unimplemented)
//! and does not support the float-vector fallback.
//!
//! The suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene::index::VectorEncoding;
use rucene_codecs::simpletext::codec::SimpleTextCodec;
use rucene_test_framework::index::base_index_file_format_test_case::{
    BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_knn_vectors_format_test_case::{
    self, BaseKnnVectorsFormatTestCase, KnnSuiteState,
};

#[path = "../../rucene-test-framework/tests/suite_seeds/mod.rs"]
mod suite_seeds;
use suite_seeds::run_suite;

/// `TestSimpleTextKnnVectorsFormat`.
#[derive(Default)]
struct TestSimpleTextKnnVectorsFormat {
    state: KnnSuiteState,
}

impl BaseIndexFileFormatTestCase for TestSimpleTextKnnVectorsFormat {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::new(SimpleTextCodec::new())
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.knn_add_random_fields(doc);
    }

    fn merge_is_stable(&self) -> bool {
        self.knn_merge_is_stable()
    }

    fn as_knn_vectors(&self) -> Option<&dyn BaseKnnVectorsFormatTestCase> {
        Some(self)
    }
}

impl BaseKnnVectorsFormatTestCase for TestSimpleTextKnnVectorsFormat {
    fn knn_state(&self) -> &KnnSuiteState {
        &self.state
    }

    fn supports_float_vector_fallback(&self) -> bool {
        false
    }

    fn random_vector_encoding(&self) -> VectorEncoding {
        // TODO: Should SimpleTextKnnVectorsFormat support all encodings?
        VectorEncoding::FLOAT32
    }
}

fn simple_text_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.simpletext.TestSimpleTextKnnVectorsFormat")
        .extends(&base_knn_vectors_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestSimpleTextKnnVectorsFormat::default()))
        }))
        .method(
            Method::new("testRandomBytes")
                .throws(&["java.lang.Exception"])
                .body(|_inst: &mut FormatTestInstance| {
                    // unimplemented
                }),
        )
        .method(
            Method::new("testSortedIndexBytes")
                .throws(&["java.lang.Exception"])
                .body(|_inst: &mut FormatTestInstance| {
                    // unimplemented
                }),
        )
        .build()
}

#[test]
fn test_simple_text_knn_vectors_format() {
    run_suite(simple_text_class);
}
