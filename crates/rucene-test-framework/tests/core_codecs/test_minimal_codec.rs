//! Port of `lucene/core/src/test/org/apache/lucene/codecs/TestMinimalCodec.java`.
//!
//! Tests to ensure that `Codec`s won't need to implement all formats in case where only a
//! small subset of Lucene's functionality is used.
//!
//! Rucene's `Codec` format accessors return the format rather than a
//! `Result`, so Java's `throw new UnsupportedOperationException()` is the
//! Java throwable the accessor panics with, as an unchecked exception
//! unwinds out of the Java call.

use std::sync::Arc;

use randomizedtesting::jdk::exception;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use rucene::codecs::{
    Codec, CompoundFormat, DocValuesFormat, FieldInfosFormat, KnnVectorsFormat, LiveDocsFormat,
    NormsFormat, PointsFormat, PostingsFormat, SegmentInfoFormat, StoredFieldsFormat,
    TermVectorsFormat,
};
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestMinimalCodec`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.codecs.TestMinimalCodec",
        vec![
            test("testMinimalCodec", test_minimal_codec),
            test("testMinimalCompoundCodec", test_minimal_compound_codec),
        ],
    )
}

/// Port of `TestMinimalCodec.testMinimalCodec`.
fn test_minimal_codec() -> Result<()> {
    run_minimal_codec_test(false)
}

/// Port of `TestMinimalCodec.testMinimalCompoundCodec`.
fn test_minimal_compound_codec() -> Result<()> {
    run_minimal_codec_test(true)
}

/// Port of `TestMinimalCodec.runMinimalCodecTest`.
fn run_minimal_codec_test(use_compound_file: bool) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let writer_config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
            MockAnalyzer::new(&*random()),
        )));
        writer_config.set_codec(if use_compound_file {
            Arc::new(MinimalCodec::compound())
        } else {
            Arc::new(MinimalCodec::new())
        })?;
        writer_config.set_use_compound_file(use_compound_file)?;
        if !use_compound_file {
            // Avoid using MockMP as it randomly enables compound file creation
            writer_config.set_merge_policy(Some(LuceneTestCase::new_merge_policy_include_mock(
                &*random(),
                false,
            )))?;
            writer_config.merge_policy().set_no_cfs_ratio(0.0)?;
            writer_config
                .merge_policy()
                .set_max_cfs_segment_size_mb(f64::INFINITY)?;
        }

        let writer = IndexWriter::new(dir.as_directory(), writer_config)?;
        let body = (|| -> Result<()> {
            writer.add_document(&basic_document())?;
            writer.flush()?;
            // create second segment
            writer.add_document(&basic_document())?;
            writer.force_merge(1)?; // test merges
            if random().next_boolean() {
                writer.commit()?;
            }

            let reader = directory_reader::open_with_writer(&*writer)?;
            assert_equals(&2, &reader.num_docs());
            reader.close()
        })();
        let closed = writer.close();
        body.and(closed)
    })();
    let closed = dir.close();
    result.and(closed)
}

/// returns a basic document with no indexed fields
///
/// Port of `TestMinimalCodec.basicDocument`.
fn basic_document() -> Document {
    Document::new()
}

/// Minimal codec implementation for working with the most basic documents
///
/// Port of `TestMinimalCodec.MinimalCodec` and, with `compound` set, of its
/// subclass `TestMinimalCodec.MinimalCompoundCodec` ("Minimal codec
/// implementation for working with the most basic documents, supporting
/// compound formats"), which overrides `compoundFormat()` only.
#[derive(Debug)]
struct MinimalCodec {
    name: &'static str,
    /// `wrappedCodec` (`protected final`): `TestUtil.getDefaultCodec()`.
    wrapped_codec: Arc<dyn Codec>,
    /// Whether this is a `MinimalCompoundCodec`.
    compound: bool,
}

impl MinimalCodec {
    /// `MinimalCodec()`: `this("MinimalCodec")`.
    fn new() -> Self {
        Self::with_name("MinimalCodec", false)
    }

    /// `MinimalCompoundCodec()`: `super("MinimalCompoundCodec")`.
    fn compound() -> Self {
        Self::with_name("MinimalCompoundCodec", true)
    }

    /// `MinimalCodec(String name)` (`protected`).
    fn with_name(name: &'static str, compound: bool) -> Self {
        Self {
            name,
            wrapped_codec: TestUtil::get_default_codec(),
            compound,
        }
    }
}

/// `throw new UnsupportedOperationException()`.
fn unsupported() -> ! {
    exception::throw_without_message("java.lang.UnsupportedOperationException")
}

impl Codec for MinimalCodec {
    fn name(&self) -> &str {
        self.name
    }

    fn field_infos_format(&self) -> &dyn FieldInfosFormat {
        self.wrapped_codec.field_infos_format()
    }

    fn segment_info_format(&self) -> &dyn SegmentInfoFormat {
        self.wrapped_codec.segment_info_format()
    }

    fn compound_format(&self) -> &dyn CompoundFormat {
        if self.compound {
            self.wrapped_codec.compound_format()
        } else {
            unsupported()
        }
    }

    fn live_docs_format(&self) -> &dyn LiveDocsFormat {
        unsupported()
    }

    fn stored_fields_format(&self) -> &dyn StoredFieldsFormat {
        // TODO: avoid calling this when no stored fields are written or read
        self.wrapped_codec.stored_fields_format()
    }

    fn postings_format(&self) -> &dyn PostingsFormat {
        unsupported()
    }

    fn doc_values_format(&self) -> &dyn DocValuesFormat {
        unsupported()
    }

    fn term_vectors_format(&self) -> &dyn TermVectorsFormat {
        unsupported()
    }

    fn norms_format(&self) -> &dyn NormsFormat {
        unsupported()
    }

    fn points_format(&self) -> &dyn PointsFormat {
        unsupported()
    }

    fn knn_vectors_format(&self) -> &dyn KnnVectorsFormat {
        unsupported()
    }
}
