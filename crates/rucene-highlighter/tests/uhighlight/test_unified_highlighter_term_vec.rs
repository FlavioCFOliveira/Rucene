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

//! Port of `TestUnifiedHighlighterTermVec.java`.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, FieldType, TextField};
use rucene::error::Result;
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, FilterDirectoryReader, SubReaderWrapper,
};
use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::index_reader::CacheHelper;
use rucene::index::{DirectoryReader, Fields, LeafReader, TermVectors};
use rucene::search::boolean_clause::Occur;
use rucene_highlighter::uhighlight::unified_highlighter::{
    HighlightFlag, UnifiedHighlighter, UnifiedHighlighterOverrides,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::highlighter_support::core_support::random;
use crate::highlighter_support::ti;
use crate::uhighlight::test_unified_highlighter::{bq, builder};
use crate::uhighlight::unified_highlighter_test_base::{
    add, assert_snippets, field, get, random_field_type, random_type_constructor, s,
    search_index_order, tq, tv_type, uh_class, UHBase, UHTest,
};

type T = UHTest;

/// `TestUnifiedHighlighterTermVec`.
pub fn class_def() -> Arc<Class> {
    uh_class::<T>(
        "org.apache.lucene.search.uhighlight.TestUnifiedHighlighterTermVec",
        random_type_constructor(|ft| UHTest { b: UHBase::new(ft) }),
        vec![
            ti::<T>(
                "testTermVecButNoPositions1",
                test_term_vec_but_no_positions1,
            ),
            ti::<T>(
                "testTermVecButNoPositions2",
                test_term_vec_but_no_positions2,
            ),
            ti::<T>(
                "testTermVecButNoPositions3",
                test_term_vec_but_no_positions3,
            ),
            ti::<T>(
                "testTermVecButNoPositions4",
                test_term_vec_but_no_positions4,
            ),
            ti::<T>(
                "testFetchTermVecsOncePerDoc",
                test_fetch_term_vecs_once_per_doc,
            ),
            ti::<T>(
                "testUserFailedToIndexOffsets",
                test_user_failed_to_index_offsets,
            ),
        ],
    )
}

fn test_term_vec_but_no_positions1(t: &mut T) -> Result<()> {
    test_term_vec_but_no_positions(t, "x", "y", "y x", "<b>y</b> <b>x</b>")
}

fn test_term_vec_but_no_positions2(t: &mut T) -> Result<()> {
    test_term_vec_but_no_positions(t, "y", "x", "y x", "<b>y</b> <b>x</b>")
}

fn test_term_vec_but_no_positions3(t: &mut T) -> Result<()> {
    test_term_vec_but_no_positions(t, "zzz", "yyy", "zzz yyy", "<b>zzz</b> <b>yyy</b>")
}

fn test_term_vec_but_no_positions4(t: &mut T) -> Result<()> {
    test_term_vec_but_no_positions(t, "zzz", "yyy", "yyy zzz", "<b>yyy</b> <b>zzz</b>")
}

/// `testTermVecButNoPositions(String aaa, String bbb, String indexed, String
/// expected)`: public, but with parameters, so not a test method.
fn test_term_vec_but_no_positions(
    t: &UHTest,
    aaa: &str,
    bbb: &str,
    indexed: &str,
    expected: &str,
) -> Result<()> {
    let tv_no_pos_type = FieldType::new_from(TextField::type_stored());
    tv_no_pos_type.set_store_term_vectors(true)?;
    tv_no_pos_type.set_store_term_vector_offsets(true)?;
    tv_no_pos_type.freeze();
    let tv_no_pos_type = Arc::new(tv_no_pos_type);

    let iw = RandomIndexWriter::new(&*random(), t.dir().as_directory(), t.analyzer())?;
    let mut document = Document::new();
    add(&mut document, &field("body", indexed, &tv_no_pos_type)?);
    iw.add_document(&document)?;
    let ir = iw.get_reader()?;
    let result = (|| -> Result<()> {
        iw.close()?;
        let searcher = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&ir)));
        let query = bq(vec![
            (tq("body", aaa), Occur::MUST),
            (tq("body", bbb), Occur::MUST),
        ])?;
        let top_docs = searcher.search(Arc::clone(&query), 10)?;
        Assert::assert_equals(1, top_docs.total_hits.value());
        let highlighter = UnifiedHighlighter::builder(Arc::clone(&searcher), t.analyzer()).build();
        let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 2)?;
        Assert::assert_equals(1, snippets.len());
        Assert::assert_true_msg(s(&snippets[0]), s(&snippets[0]).contains(expected));
        Ok(())
    })();
    let closed = ir.close();
    result?;
    closed
}

fn test_fetch_term_vecs_once_per_doc(t: &mut T) -> Result<()> {
    let iw = RandomIndexWriter::new(&*random(), t.dir().as_directory(), t.analyzer())?;

    // Declare some number of fields with random field type; but at least one will have term
    // vectors.
    let num_tv_fields = 1 + random().next_int_bound(3);
    let mut fields: Vec<String> = Vec::with_capacity(num_tv_fields as usize);
    let mut field_types: Vec<Arc<FieldType>> = Vec::with_capacity(num_tv_fields as usize);
    for i in 0..num_tv_fields {
        fields.push(format!("body{i}"));
        field_types.push(random_field_type(&*random(), &[]));
    }
    // ensure at least one has TVs by setting one randomly to it:
    let at = random().next_int_bound(field_types.len() as i32) as usize;
    field_types[at] = tv_type();

    let num_docs = 1 + random().next_int_bound(3);
    for _ in 0..num_docs {
        let mut doc = Document::new();
        for f in &fields {
            add(&mut doc, &field(f, "some test text", &tv_type())?);
        }
        iw.add_document(&doc)?;
    }

    // Wrap the reader to ensure we only fetch TVs once per doc
    let original_reader = iw.get_reader()?;
    let ir = assert_once_term_vec_directory_reader(original_reader)?;
    iw.close()?;

    let searcher = Arc::new(LuceneTestCase::new_searcher_wrap(Arc::clone(&ir), false)); // wrapping the reader messes up our counting logic
    let highlighter = UnifiedHighlighter::builder(Arc::clone(&searcher), t.analyzer()).build();
    let query = bq(fields
        .iter()
        .map(|f| (tq(f, "test"), Occur::MUST))
        .collect())?;
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(num_docs as i64, top_docs.total_hits.value());
    let field_to_snippets = highlighter.highlight_fields(&fields, &query, &top_docs)?;
    let expected_snippets_by_doc = vec!["some <b>test</b> text"; num_docs as usize];
    for f in &fields {
        assert_snippets(&expected_snippets_by_doc, get(&field_to_snippets, f));
    }

    ir.stored_fields()?.document(0)?; // ensure this works because the ir hasn't been closed
    ir.close()
}

/// `new AssertOnceTermVecDirectoryReader(in)`.
fn assert_once_term_vec_directory_reader(
    r#in: Arc<dyn DirectoryReader>,
) -> Result<Arc<dyn DirectoryReader>> {
    Ok(Arc::new(FilterDirectoryReader::new(
        r#in,
        Arc::new(AssertOnceSubReaderWrapper),
        Arc::new(AssertOnceDirectoryReaderWrapperKind),
    )?))
}

/// `AssertOnceTermVecDirectoryReader.doWrapDirectoryReader` and its
/// `getReaderCacheHelper()`, which returns `null`.
struct AssertOnceDirectoryReaderWrapperKind;

impl DirectoryReaderWrapper for AssertOnceDirectoryReaderWrapperKind {
    fn wrap(&self, inner: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        assert_once_term_vec_directory_reader(inner)
    }

    fn subclass_simple_name(&self) -> &'static str {
        "AssertOnceTermVecDirectoryReader"
    }

    fn reader_cache_helper_is_null(&self) -> bool {
        true
    }
}

/// `AssertOnceTermVecDirectoryReader.SUB_READER_WRAPPER`.
struct AssertOnceSubReaderWrapper;

impl SubReaderWrapper for AssertOnceSubReaderWrapper {
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        Ok(Arc::new(AssertOnceLeafReader {
            inner: FilterLeafReader::new(reader)?,
            seen_doc_ids: Arc::new(Mutex::new(HashSet::new())),
        }))
    }
}

/// The anonymous `FilterLeafReader` of `SUB_READER_WRAPPER.wrap`.
struct AssertOnceLeafReader {
    inner: FilterLeafReader,
    /// `seenDocIDs` (a `java.util.BitSet`).
    seen_doc_ids: Arc<Mutex<HashSet<i32>>>,
}

impl fmt::Debug for AssertOnceLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssertOnceLeafReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for AssertOnceLeafReader {
    /// The inherited `FilterLeafReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl LeafReader for AssertOnceLeafReader {
    crate::forward_leaf_reader!(filter inner;
        core, filter_leaf_reader_delegate, do_close, num_docs, max_doc,
        stored_fields, terms, get_numeric_doc_values, get_binary_doc_values,
        get_sorted_doc_values, get_sorted_numeric_doc_values, get_sorted_set_doc_values,
        get_norm_values, get_doc_values_skipper, get_float_vector_values,
        get_byte_vector_values, search_nearest_vectors, search_nearest_vectors_byte,
        get_field_infos, get_live_docs, get_point_values, check_integrity, get_meta_data);

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        let orig = self.inner.get_delegate().term_vectors()?;
        Ok(Box::new(AssertOnceTermVectors {
            orig,
            seen_doc_ids: Arc::clone(&self.seen_doc_ids),
        }))
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
}

/// The anonymous `TermVectors` of the wrapped leaf's `termVectors()`.
#[derive(Debug)]
struct AssertOnceTermVectors {
    orig: Box<dyn TermVectors>,
    seen_doc_ids: Arc<Mutex<HashSet<i32>>>,
}

impl TermVectors for AssertOnceTermVectors {
    fn get(&self, doc_id: i32) -> Result<Option<Box<dyn Fields>>> {
        let mut seen = self.seen_doc_ids.lock().unwrap_or_else(|e| e.into_inner());
        Assert::assert_false_msg(
            "Should not request TVs for doc more than once.",
            seen.contains(&doc_id),
        );
        seen.insert(doc_id);
        drop(seen);
        self.orig.get(doc_id)
    }
}

/// The overrides of the anonymous highlighter of
/// `testUserFailedToIndexOffsets`.
#[derive(Debug)]
struct NoFlags;

impl UnifiedHighlighterOverrides for NoFlags {
    fn get_flags(&self, _this: &UnifiedHighlighter, _field: &str) -> HashSet<HighlightFlag> {
        HashSet::new() // no WEIGHT_MATCHES
    }
}

fn test_user_failed_to_index_offsets(t: &mut T) -> Result<()> {
    let field_type = FieldType::new_from(&*tv_type()); // note: it's indexed too
    field_type.set_store_term_vector_positions(random().next_boolean())?;
    field_type.set_store_term_vector_offsets(false)?;
    let field_type = Arc::new(field_type);

    let iw = RandomIndexWriter::new(&*random(), t.dir().as_directory(), t.analyzer())?;
    let mut doc = Document::new();
    add(&mut doc, &field("body", "term vectors", &field_type)?);
    iw.add_document(&doc)?;
    let ir = iw.get_reader()?;
    let result = (|| -> Result<()> {
        iw.close()?;
        let searcher = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&ir)));
        let uh_builder = builder(&searcher, t.analyzer());
        let mut highlighter = UnifiedHighlighter::from_builder(uh_builder);
        highlighter.set_overrides(Arc::new(NoFlags));
        let query = tq("body", "vectors");
        let top_docs = search_index_order(&searcher, &query, 10)?;
        expect_throws("java.lang.IllegalArgumentException", || {
            highlighter.highlight_max_passages("body", &query, &top_docs, 1)
        });
        Ok(())
    })();
    let closed = ir.close();
    result?;
    closed
}
