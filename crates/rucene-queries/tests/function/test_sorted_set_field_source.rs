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

//! Port of `TestSortedSetFieldSource.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, SortedSetDocValuesField, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::search::sorted_set_sort_field::SortedSetSortField;
use rucene::util::BytesRef;
use rucene_queries::function::value_source::{new_context, ValueSource};
use rucene_queries::function::valuesource::sorted_set_field_source::SortedSetFieldSource;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::{class, random, test};

/// `TestSortedSetFieldSource`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.function.TestSortedSetFieldSource",
        vec![test("testSimple", test_simple)],
    )
}

fn test_simple() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    conf.set_max_buffered_docs(2)?; // generate few segments
    conf.set_merge_policy(Some(NoMergePolicy::instance()))?; // prevent merges for this test
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    let mut doc = Document::new();
    doc.add(Box::new(SortedSetDocValuesField::new(
        "value",
        BytesRef::from_char_sequence("baz"),
    )));
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "2",
        Store::YES,
    )));
    writer.add_document(&doc)?;
    let mut doc = Document::new();
    doc.add(Box::new(SortedSetDocValuesField::new(
        "value",
        BytesRef::from_char_sequence("foo"),
    )));
    doc.add(Box::new(SortedSetDocValuesField::new(
        "value",
        BytesRef::from_char_sequence("bar"),
    )));
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "1",
        Store::YES,
    )));
    writer.add_document(&doc)?;
    writer.close()?;

    let ir = directory_reader::open(dir.as_directory())?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&ir));
    // `getOnlyLeafReader(ir).getContext()`: the only leaf, at doc base 0
    LuceneTestCase::get_only_leaf_reader(Arc::clone(&ir) as Arc<dyn rucene::index::IndexReader>)?;
    let ar_context =
        Arc::clone(&(Arc::clone(&ir) as Arc<dyn rucene::index::IndexReader>).leaves()?[0]);
    let vs: Arc<dyn ValueSource> = Arc::new(SortedSetFieldSource::new("value"));
    // Collections.emptyMap(): no searcher, no entries
    let empty = new_context(None);
    let mut values = Arc::clone(&vs).get_values(&empty, &ar_context)?;
    Assert::assert_equals_str(None, Some("baz"), values.str_val(0)?.as_deref());
    Assert::assert_equals_str(None, Some("bar"), values.str_val(1)?.as_deref());

    // test SortField optimization
    let reverse = random().next_boolean();
    let vssf = Arc::clone(&vs).get_sort_field(reverse)?;
    let sf = SortedSetSortField::new("value", reverse)?.into_sort_field();
    Assert::assert_equals(&sf, &vssf);
    let vssf = vssf.rewrite(&searcher)?.into_owned();
    let sf = sf.rewrite(&searcher)?.into_owned();
    Assert::assert_equals(&sf, &vssf);

    // test scorer
    let vs: Arc<dyn ValueSource> = Arc::new(SortedSetFieldSource::new("value"));
    let values = Arc::clone(&vs).get_values(&empty, &ar_context)?;
    // `values.getRangeScorer(...)`: the scorer owns the values the test reads
    // back (`ValueSourceScorer::values`); its iterator is the scorer itself.
    let mut vss = rucene_queries::function::function_values::get_range_scorer(
        values,
        &ar_context,
        Some("a"),
        Some("z"),
        true,
        true,
    )?;
    let d0 = rucene::search::scorer::Scorer::iterator(&mut vss).next_doc()?;
    Assert::assert_equals_str(None, Some("baz"), vss.values().str_val(d0)?.as_deref());
    let d1 = rucene::search::scorer::Scorer::iterator(&mut vss).next_doc()?;
    Assert::assert_equals_str(None, Some("bar"), vss.values().str_val(d1)?.as_deref());

    ir.close()?;
    dir.close()
}
