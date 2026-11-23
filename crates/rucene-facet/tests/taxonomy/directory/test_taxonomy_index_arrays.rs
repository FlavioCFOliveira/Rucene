/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.facet.taxonomy.directory.TestTaxonomyIndexArrays`.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{directory_reader, IndexReader, TwoPhaseCommit};
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::directory::taxonomy_index_arrays::{
    ChunkedIntArray, TaxonomyIndexArrays, CHUNK_SIZE,
};
use rucene_facet::taxonomy::facet_label::FacetLabel;
use rucene_facet::taxonomy::parallel_taxonomy_arrays::IntArray;
use rucene_facet::taxonomy::taxonomy_reader::{INVALID_ORDINAL, ROOT_ORDINAL};
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::core_support::{assert_not_equals, class, test};

const CHUNK: i32 = CHUNK_SIZE as i32;

/// `TestTaxonomyIndexArrays` (extends `LuceneTestCase`).
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.facet.taxonomy.directory.TestTaxonomyIndexArrays",
        vec![
            test("testRandom", test_random),
            test("testMultiplesOfChunkSize", test_multiples_of_chunk_size),
            test(
                "testConstructFromEmptyIndex",
                test_construct_from_empty_index,
            ),
            test("testConstructFromIndex", test_construct_from_index),
            test("testRefresh", test_refresh),
        ],
    )
}

fn val(a: &ChunkedIntArray, chunk: usize, i: usize) -> i32 {
    a.values[chunk][i].load(Ordering::Relaxed)
}

fn check_int_arrays_equals(expected: &ChunkedIntArray, actual: &ChunkedIntArray) {
    for i in 0..expected.values.len() - 1 {
        // assertSame
        Assert::assert_true(Arc::ptr_eq(&expected.values[i], &actual.values[i]));
    }
    let last_old_chunk = expected.values.len() - 1;
    for i in 0..expected.values[last_old_chunk].len() {
        Assert::assert_equals(
            val(expected, last_old_chunk, i),
            val(actual, last_old_chunk, i),
        );
    }
}

fn check_invariants(old_array: &TaxonomyIndexArrays, new_array: &TaxonomyIndexArrays) {
    check_int_arrays_equals(old_array.parents_array(), new_array.parents_array());
    check_int_arrays_equals(old_array.siblings_array(), new_array.siblings_array());
}

fn test_random() -> Result<()> {
    let random = LuceneTestCase::random();
    let old_array = Arc::new(TaxonomyIndexArrays::from_int_arrays(&[&[INVALID_ORDINAL]]));
    let num_iterations = 100;
    let mut ordinal = 1;
    for _ in 0..num_iterations {
        let new_ordinal = ordinal + random.next_int_bound(CHUNK);
        let new_array = old_array.add(new_ordinal, ordinal);
        check_invariants(&old_array, &new_array);
        ordinal = new_ordinal;
    }
    Ok(())
}

fn test_multiples_of_chunk_size() -> Result<()> {
    let old_array = Arc::new(TaxonomyIndexArrays::from_int_arrays(&[&[INVALID_ORDINAL]]));
    let num_iterations = 20;
    let mut ordinal = CHUNK;
    for _ in 0..num_iterations {
        let new_ordinal = ordinal + CHUNK;
        let new_array = old_array.add(new_ordinal, ordinal);
        check_invariants(&old_array, &new_array);
        ordinal = new_ordinal;
    }
    Ok(())
}

fn open_reader(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<Arc<dyn IndexReader>> {
    Ok(directory_reader::open(dir.as_directory())? as Arc<dyn IndexReader>)
}

fn test_construct_from_empty_index() -> Result<()> {
    let dir = LuceneTestCase::new_directory();

    // Produce empty index
    IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(None),
    )?
    .close()?;

    let reader = open_reader(&dir)?;

    let tia = TaxonomyIndexArrays::new(&reader)?;
    Assert::assert_equals(0, tia.parents_array().length());

    let tia = TaxonomyIndexArrays::with_copy_from(&reader, &tia)?;
    Assert::assert_equals(0, tia.parents_array().length());

    reader.close()?;
    dir.close()
}

/// `try (IndexReader reader = DirectoryReader.open(dir)) { tia = make(reader); }`.
fn with_reader<T>(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
    make: impl FnOnce(&Arc<dyn IndexReader>) -> Result<T>,
) -> Result<T> {
    let reader = open_reader(dir)?;
    let result = make(&reader);
    let closed = reader.close();
    let value = result?;
    closed?;
    Ok(value)
}

fn label(components: &[&str]) -> Result<FacetLabel> {
    FacetLabel::new(components.iter().copied())
}

fn test_construct_from_index() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_writer = DirectoryTaxonomyWriter::new(dir.as_directory())?;

    // Test 1
    // Add one category. The first TIA will have a non-zero length incomplete chunk.
    taxo_writer.add_category(&label(&["a"])?)?;
    taxo_writer.commit()?;

    let old_tia = with_reader(&dir, TaxonomyIndexArrays::new)?;

    let old_parents = old_tia.parents_array();
    Assert::assert_equals(2, old_parents.length());
    Assert::assert_equals(INVALID_ORDINAL, old_parents.get(0));
    Assert::assert_equals(ROOT_ORDINAL, old_parents.get(1));

    // Test 2
    // Add enough categories to fill the first chunk.
    for i in 2..CHUNK {
        taxo_writer.add_category(&label(&["a", &i.to_string()])?)?;
    }
    taxo_writer.commit()?;

    let old_tia = with_reader(&dir, TaxonomyIndexArrays::new)?;

    let old_parents = old_tia.parents_array();
    Assert::assert_equals(CHUNK, old_parents.length());
    Assert::assert_equals(INVALID_ORDINAL, old_parents.get(0));
    Assert::assert_equals(ROOT_ORDINAL, old_parents.get(1));
    for i in 2..old_parents.length() {
        Assert::assert_equals(1, old_parents.get(i));
    }

    // Test 3
    // Both TIAs have the same parents and siblings arrays.
    old_tia.children_array(); // Initializes children

    let new_tia = with_reader(&dir, |r| TaxonomyIndexArrays::with_copy_from(r, &old_tia))?;

    check_invariants(&old_tia, &new_tia);

    // Test 4
    // Add one more category, which will start a new chunk on the new TIA.
    taxo_writer.add_category(&label(&["a", &CHUNK.to_string()])?)?;
    taxo_writer.commit()?;

    let new_tia = with_reader(&dir, |r| TaxonomyIndexArrays::with_copy_from(r, &old_tia))?;

    let new_parents = new_tia.parents_array();
    Assert::assert_equals(1 + CHUNK, new_parents.length());
    Assert::assert_equals(INVALID_ORDINAL, new_parents.get(0));
    Assert::assert_equals(ROOT_ORDINAL, new_parents.get(1));
    for i in 2..new_parents.length() {
        Assert::assert_equals(1, new_parents.get(i));
    }

    // Test 5
    // Fill the second chunk of the new TIA.
    for i in 1..CHUNK {
        taxo_writer.add_category(&label(&["a", &(i + CHUNK).to_string()])?)?;
    }
    taxo_writer.commit()?;

    let new_tia = with_reader(&dir, |r| TaxonomyIndexArrays::with_copy_from(r, &old_tia))?;

    let new_parents = new_tia.parents_array();
    Assert::assert_equals(2 * CHUNK, new_parents.length());
    Assert::assert_equals(INVALID_ORDINAL, new_parents.get(0));
    Assert::assert_equals(ROOT_ORDINAL, new_parents.get(1));
    for i in 2..new_parents.length() {
        Assert::assert_equals(1, new_parents.get(i));
    }

    taxo_writer.close()?;
    dir.close()
}

fn test_refresh() -> Result<()> {
    let dir = LuceneTestCase::new_directory();

    // Write two chunks worth of ordinals whose parents are ordinals 1 or 2
    let taxo_writer = DirectoryTaxonomyWriter::new(dir.as_directory())?;
    taxo_writer.add_category(&label(&["a"])?)?; // ordinal 1
    taxo_writer.add_category(&label(&["b"])?)?; // ordinal 2
    for i in 0..2 * CHUNK {
        if i % 2 == 0 {
            taxo_writer.add_category(&label(&["a", &i.to_string()])?)?;
        } else {
            taxo_writer.add_category(&label(&["b", &i.to_string()])?)?;
        }
    }
    taxo_writer.commit()?;

    // Initialize old taxonomy arrays
    let reader = open_reader(&dir)?;
    let old_tia = TaxonomyIndexArrays::new(&reader)?;
    reader.close()?;
    old_tia.children_array(); // Init the children

    // Write one more batch of ordinals whose parents are ordinals 1 or 2 again.
    for i in 2 * CHUNK..3 * CHUNK {
        if i % 2 == 0 {
            taxo_writer.add_category(&label(&["a", &i.to_string()])?)?;
        } else {
            taxo_writer.add_category(&label(&["b", &i.to_string()])?)?;
        }
    }
    taxo_writer.close()?;

    // Initialize new taxonomy arrays
    let reader = open_reader(&dir)?;
    let new_tia = TaxonomyIndexArrays::with_copy_from(&reader, &old_tia)?;
    reader.close()?;

    // Parents and siblings are unchanged in the old range, children will have changed
    check_invariants(&old_tia, &new_tia);

    let old_children = old_tia.children_array();
    let new_children = new_tia.children_array();
    // The first chunk had to be reallocated to rewrite the value for ordinals 1 and 2
    Assert::assert_false(Arc::ptr_eq(
        &old_children.values[0],
        &new_children.values[0],
    ));
    // The second chunk could stay the same, since none of these ordinals have children
    Assert::assert_true(Arc::ptr_eq(
        &old_children.values[1],
        &new_children.values[1],
    ));
    // The third chunk had to be reallocated to grow
    Assert::assert_false(Arc::ptr_eq(
        &old_children.values[2],
        &new_children.values[2],
    ));

    // Check contents of the first chunk
    for i in 0..CHUNK_SIZE {
        if i == 1 || i == 2 {
            assert_not_equals(&val(old_children, 0, i), &val(new_children, 0, i));
        } else {
            Assert::assert_equals(val(old_children, 0, i), val(new_children, 0, i));
        }
    }

    // Check contents of the third chunk
    for i in 0..old_children.values[2].len() {
        Assert::assert_equals(val(old_children, 2, i), val(new_children, 2, i));
    }

    dir.close()
}
