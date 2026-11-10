//! Port of `lucene/core/src/test/org/apache/lucene/document/TestManyKnnDocs.java`
//! (`@TimeoutSuite(millis = 86_400_000)`, `@Monster("takes ~10 minutes and
//! needs extra heap, disk space, file handles")`).

use std::sync::Arc;

use randomizedtesting::annotations::TimeoutSuite;
use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, KnnFloatVectorField};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::TieredMergePolicy;
use rucene::index::{IndexReader, VectorSimilarityFunction};
use rucene::search::{IndexSearcher, KnnFloatVectorQuery};
use rucene::store::{Directory, FSDirectory};
use rucene_test_framework::codecs::vector::ConfigurableMCodec;
use rucene_test_framework::util::lucene_test_case::Monster;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class_annotated, test};

/// `TestManyKnnDocs`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.document.TestManyKnnDocs",
        vec![
            test(
                "testSameVectorIndexedMultipleTimes",
                test_same_vector_indexed_multiple_times,
            ),
            test("testLargeSegment", test_large_segment),
        ],
        |builder| {
            builder
                // 24 hour timeout
                .annotate(TimeoutSuite { millis: 86_400_000 })
                .annotate(Monster {
                    value: "takes ~10 minutes and needs extra heap, disk space, file handles"
                        .to_string(),
                })
        },
    )
}

/// Port of `testSameVectorIndexedMultipleTimes`.
fn test_same_vector_indexed_multiple_times() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();
    let vector = vec![0.5f32; 16];
    let outcome = (|| -> Result<()> {
        let w = IndexWriter::new(d.as_directory(), IndexWriterConfig::new())?;
        let written = (|| -> Result<()> {
            for j in 1..=100_000 {
                let mut doc = Document::new();
                doc.add(Box::new(KnnFloatVectorField::new(
                    "field",
                    &vector,
                    VectorSimilarityFunction::DOT_PRODUCT,
                )?));
                w.add_document(&doc)?;
                if j % 1000 == 0 {
                    w.flush()?;
                }
            }
            w.commit()?;
            Ok(())
        })();
        let closed = w.close();
        written?;
        closed
    })();
    let closed = d.close();
    outcome?;
    closed
}

/// Port of `testLargeSegment`.
///
/// **Divergence from Lucene 10.5.0.** Java adds one `Document` holding one
/// `KnnFloatVectorField` over `vector` and mutates `vector[0]` between
/// `addDocument` calls; the field copies its vector here, so each call
/// indexes a document built over the array's current value (same field,
/// same values reach the writer).
fn test_large_segment() -> Result<()> {
    let iwc = IndexWriterConfig::new();
    // Make sure to use the ConfigurableMCodec instead of a random one
    iwc.set_codec(Arc::new(ConfigurableMCodec::with_max_conn(128)))?;
    iwc.set_ram_buffer_size_mb(64.0)?; // Use a 64MB buffer to create larger initial segments
    let mp = TieredMergePolicy::new();
    mp.set_max_merge_at_once(256)?; // avoid intermediate merges (waste of time with HNSW?)
    mp.set_segments_per_tier(256.0)?; // only merge once at the end when we ask
    iwc.set_merge_policy(Some(Arc::new(mp)))?;
    let field_name = "field";
    let similarity_function = VectorSimilarityFunction::DOT_PRODUCT;

    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(
        &LuceneTestCase::create_temp_dir_prefix("ManyKnnVectorDocs"),
    )?);
    let iw = IndexWriter::new(Arc::clone(&dir), iwc)?;
    let outcome = (|| -> Result<()> {
        let num_vectors = 2088992;
        let mut vector = vec![0f32; 1];
        for i in 0..num_vectors {
            vector[0] = (i % 256) as f32;
            let mut doc = Document::new();
            doc.add(Box::new(KnnFloatVectorField::new(
                field_name,
                &vector,
                similarity_function,
            )?));
            iw.add_document(&doc)?;
        }

        // merge to single segment and then verify
        iw.force_merge(1)?;
        iw.commit()?;
        let reader: Arc<dyn IndexReader> = rucene::index::directory_reader::open(Arc::clone(&dir))?;
        let searcher = IndexSearcher::new(reader)?;
        let docs = searcher.search(
            Arc::new(KnnFloatVectorQuery::new("field", vec![120.0], 10)?),
            5,
        )?;
        assert_equals(&5, &docs.score_docs.len());
        Ok(())
    })();
    // try-with-resources: `iw`, then `dir`, closed in reverse order.
    let closed = iw.close();
    drop(iw);
    let dir_closed = rucene_test_framework::store::base_directory_test_case::close_shared(dir);
    outcome?;
    closed?;
    dir_closed
}
