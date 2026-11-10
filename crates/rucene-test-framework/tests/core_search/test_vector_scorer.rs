//! Port of `lucene/core/src/test/org/apache/lucene/search/TestVectorScorer.java`.

use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, KnnByteVectorField, KnnFloatVectorField, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::{IndexReader, VectorEncoding, VectorSimilarityFunction};
use rucene::search::vector_scorer::VectorScorer;
use rucene::search::NO_MORE_DOCS;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestVectorScorer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestVectorScorer",
        vec![test("testFindAll", test_find_all)],
    )
}

/// Port of `TestVectorScorer.testFindAll`.
fn test_find_all() -> Result<()> {
    // `VectorEncoding.values()`.
    let encoding =
        *RandomPicks::random_from(&*random(), &[VectorEncoding::BYTE, VectorEncoding::FLOAT32]);
    let mut index_store =
        get_index_store("field", encoding, &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]])?;
    let result = (|| -> Result<()> {
        let reader: Arc<dyn IndexReader> = directory_reader::open(index_store.as_directory())?;
        let result = (|| -> Result<()> {
            let leaves = Arc::clone(&reader).leaves()?;
            debug_assert!(leaves.len() == 1);
            let context = &leaves[0];
            let mut vector_scorer: Box<dyn VectorScorer> = match encoding {
                VectorEncoding::BYTE => context
                    .leaf_reader()
                    .get_byte_vector_values("field")?
                    .expect("byte vector values")
                    .scorer(&[1, 2])?
                    .expect("vector scorer"),
                VectorEncoding::FLOAT32 => context
                    .leaf_reader()
                    .get_float_vector_values("field")?
                    .expect("float vector values")
                    .scorer(&[1.0, 2.0])?
                    .expect("vector scorer"),
            };

            let iterator = vector_scorer.iterator();
            let mut num_docs = 0;
            while iterator.next_doc()? != NO_MORE_DOCS {
                num_docs += 1;
            }
            assert_equals(&3, &num_docs);
            Ok(())
        })();
        result.and(reader.close())
    })();
    result.and(index_store.close())
}

/// Creates a new directory and adds documents with the given vectors as kNN
/// vector fields
///
/// `private Directory getIndexStore(String field, VectorEncoding encoding, float[]... contents)`.
fn get_index_store(
    field: &str,
    encoding: VectorEncoding,
    contents: &[&[f32]],
) -> Result<TestDirectory> {
    let index_store = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), index_store.as_directory())?;
    for i in 0..contents.len() {
        let mut doc = Document::new();
        if encoding == VectorEncoding::BYTE {
            let mut v = vec![0u8; contents[i].len()];
            for j in 0..v.len() {
                v[j] = contents[i][j] as i8 as u8;
            }
            doc.add(Box::new(KnnByteVectorField::new(
                field,
                &v,
                VectorSimilarityFunction::EUCLIDEAN,
            )?));
        } else {
            doc.add(Box::new(KnnFloatVectorField::with_euclidean(
                field,
                contents[i],
            )?));
        }
        doc.add(Box::new(StringField::new(
            "id",
            format!("id{i}"),
            Store::YES,
        )?));
        writer.add_document(&doc)?;
    }
    // Add some documents without a vector
    for _i in 0..5 {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "other",
            "value".to_string(),
            Store::NO,
        )?));
        writer.add_document(&doc)?;
    }
    writer.force_merge(1)?;
    writer.close()?;
    Ok(index_store)
}
