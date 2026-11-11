//! Port of `lucene/core/src/test/org/apache/lucene/index/TestReaderUtil.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::multi_reader::reader_util;
use rucene::index::IndexReader;
use rucene::search::score_doc::ScoreDoc;
use rucene::util::IOUtils;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_array_equals, assert_equals, class, random, test, JavaHashSet};

/// `TestReaderUtil`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestReaderUtil",
        vec![
            test(
                "testPartitionByLeafEmptyDocIds",
                test_partition_by_leaf_empty_doc_ids,
            ),
            test(
                "testPartitionByLeafSingleSegment",
                test_partition_by_leaf_single_segment,
            ),
            test(
                "testPartitionByLeafMultipleSegments",
                test_partition_by_leaf_multiple_segments,
            ),
            test(
                "testPartitionByLeafSkipsSegmentsWithNoHits",
                test_partition_by_leaf_skips_segments_with_no_hits,
            ),
            test(
                "testPartitionByLeafRandomized",
                test_partition_by_leaf_randomized,
            ),
        ],
    )
}

/// `new IndexWriterConfig().setMergePolicy(NoMergePolicy.INSTANCE)`.
fn no_merge_iwc() -> Result<IndexWriterConfig> {
    let iwc = IndexWriterConfig::new();
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    Ok(iwc)
}

/// `try (Directory dir = newDirectory(); IndexWriter writer = new
/// IndexWriter(dir, iwc)) { body; try (DirectoryReader reader =
/// DirectoryReader.open(writer)) { check } }`: the resources close in reverse
/// order, the first failure winning.
fn with_index(
    iwc: IndexWriterConfig,
    body: impl FnOnce(&IndexWriter) -> Result<()>,
    check: impl FnOnce(&Arc<dyn IndexReader>) -> Result<()>,
) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let writer = IndexWriter::new(dir.as_directory(), iwc)?;
        let result = (|| -> Result<()> {
            body(&writer)?;
            let reader: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*writer)?;
            let result = check(&reader);
            let closed = reader.close();
            result.and(closed)
        })();
        let closed = writer.close();
        result.and(closed)
    })();
    let closed = IOUtils::close([IOUtils::closeable(|| dir.close())]);
    result.and(closed)
}

/// `new ScoreDoc(doc, 1f)` for each of `docs`.
fn hits(docs: &[i32]) -> Vec<ScoreDoc> {
    docs.iter().map(|&d| ScoreDoc::new(d, 1.0)).collect()
}

/// Port of `TestReaderUtil.testPartitionByLeafEmptyDocIds`.
fn test_partition_by_leaf_empty_doc_ids() -> Result<()> {
    with_index(
        IndexWriterConfig::new(),
        |writer| {
            writer.add_document(&Document::new())?;
            writer.add_document(&Document::new())?;
            Ok(())
        },
        |reader| {
            let leaves = Arc::clone(reader).leaves()?;
            let result = reader_util::partition_by_leaf(&[], &leaves);
            assert_equals(&leaves.len(), &result.len());
            for leaf in &result {
                assert_equals(&0, &leaf.len());
            }
            Ok(())
        },
    )
}

/// Port of `TestReaderUtil.testPartitionByLeafSingleSegment`.
fn test_partition_by_leaf_single_segment() -> Result<()> {
    with_index(
        IndexWriterConfig::new(),
        |writer| {
            for _i in 0..10 {
                writer.add_document(&Document::new())?;
            }
            Ok(())
        },
        |reader| {
            let leaves = Arc::clone(reader).leaves()?;
            assert_equals(&1, &leaves.len());

            let hits = hits(&[0, 3, 5, 9]);
            let result = reader_util::partition_by_leaf(&hits, &leaves);

            assert_equals(&1, &result.len());
            assert_array_equals(&[0, 3, 5, 9], &result[0]);
            Ok(())
        },
    )
}

/// Port of `TestReaderUtil.testPartitionByLeafMultipleSegments`.
fn test_partition_by_leaf_multiple_segments() -> Result<()> {
    with_index(
        no_merge_iwc()?,
        |writer| {
            for _i in 0..10 {
                writer.add_document(&Document::new())?;
            }
            writer.commit()?;

            // Create second segment
            for _i in 0..10 {
                writer.add_document(&Document::new())?;
            }
            writer.commit()?;
            Ok(())
        },
        |reader| {
            let leaves = Arc::clone(reader).leaves()?;
            assert_equals(&2, &leaves.len());

            // Hits in both segments
            let hits = hits(&[2, 9, 10, 18]);
            let result = reader_util::partition_by_leaf(&hits, &leaves);

            assert_equals(&2, &result.len());
            // First segment: docs 0-9
            assert_array_equals(&[2, 9], &result[0]);
            // Second segment: docs 10-19
            assert_array_equals(&[10, 18], &result[1]);
            Ok(())
        },
    )
}

/// Port of `TestReaderUtil.testPartitionByLeafSkipsSegmentsWithNoHits`.
fn test_partition_by_leaf_skips_segments_with_no_hits() -> Result<()> {
    with_index(
        no_merge_iwc()?,
        |writer| {
            // Create 3 segments
            for _seg in 0..3 {
                for _i in 0..10 {
                    writer.add_document(&Document::new())?;
                }
                writer.commit()?;
            }
            Ok(())
        },
        |reader| {
            let leaves = Arc::clone(reader).leaves()?;
            assert_equals(&3, &leaves.len());

            // Hits only in first and third segment (skip middle)
            let hits = hits(&[3, 25]);
            let result = reader_util::partition_by_leaf(&hits, &leaves);

            assert_equals(&3, &result.len());
            assert_array_equals(&[3], &result[0]);
            assert_equals(&0, &result[1].len()); // middle segment has no hits
            assert_array_equals(&[25], &result[2]);
            Ok(())
        },
    )
}

/// Port of `TestReaderUtil.testPartitionByLeafRandomized`.
fn test_partition_by_leaf_randomized() -> Result<()> {
    for _iter in 0..100 {
        let num_segments = random().next_int_bound(10) + 1;
        let mut total_docs = 0;
        let mut docs_per_segment = vec![0i32; num_segments as usize];
        for i in 0..num_segments as usize {
            docs_per_segment[i] = random().next_int_bound(100) + 1;
            total_docs += docs_per_segment[i];
        }

        with_index(
            no_merge_iwc()?,
            |writer| {
                for seg in 0..num_segments as usize {
                    for _i in 0..docs_per_segment[seg] {
                        writer.add_document(&Document::new())?;
                    }
                    writer.commit()?;
                }
                Ok(())
            },
            |reader| {
                let leaves = Arc::clone(reader).leaves()?;
                assert_equals(&(num_segments as usize), &leaves.len());

                // Generate random hits (0 to totalDocs inclusive - covers empty and all-match)
                let num_hits = random().next_int_bound(total_docs + 1) as usize;
                let mut hit_set: JavaHashSet<i32> = JavaHashSet::new();
                while hit_set.size() < num_hits {
                    hit_set.add(random().next_int_bound(total_docs));
                }
                let doc_ids: Vec<i32> = hit_set.iter().copied().collect();
                let hits = hits(&doc_ids);

                let result = reader_util::partition_by_leaf(&hits, &leaves);

                // Verify: result length matches leaves
                assert_equals(&(num_segments as usize), &result.len());

                // Verify: total hits preserved
                let total_result_docs: usize = result.iter().map(Vec::len).sum();
                assert_equals(&doc_ids.len(), &total_result_docs);

                // Verify: each doc in correct leaf and sorted
                for leaf_idx in 0..result.len() {
                    let leaf_docs = &result[leaf_idx];
                    let leaf = &leaves[leaf_idx];
                    let doc_base = leaf.doc_base();
                    let max_doc = leaf.leaf_reader().max_doc();
                    for i in 0..leaf_docs.len() {
                        let doc_id = leaf_docs[i];
                        Assert::assert_true(doc_id >= doc_base && doc_id < doc_base + max_doc);
                        if i > 0 {
                            Assert::assert_true(leaf_docs[i] > leaf_docs[i - 1]);
                        }
                    }
                }
                Ok(())
            },
        )?;
    }
    Ok(())
}
