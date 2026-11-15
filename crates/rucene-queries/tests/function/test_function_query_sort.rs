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

//! Port of `TestFunctionQuerySort.java`: test that functionquery's
//! getSortField() actually works.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericDocValuesField, NumericValue, StoredField};
use rucene::error::Result;
use rucene::index::IndexReader;
use rucene::search::field_comparator::SortValue;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::query::Query;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene_queries::function::value_source::ValueSource;
use rucene_queries::function::valuesource::double_const_value_source::DoubleConstValueSource;
use rucene_queries::function::valuesource::double_field_source::DoubleFieldSource;
use rucene_queries::function::valuesource::float_field_source::FloatFieldSource;
use rucene_queries::function::valuesource::int_field_source::IntFieldSource;
use rucene_queries::function::valuesource::long_field_source::LongFieldSource;
use rucene_queries::function::valuesource::sum_float_function::SumFloatFunction;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::{class, random, test};
use crate::queries_support::search_support::field_mut_as;

/// `TestFunctionQuerySort`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.function.TestFunctionQuerySort",
        vec![
            test(
                "testOptimizedFieldSourceFunctionSorting",
                test_optimized_field_source_function_sorting,
            ),
            test(
                "testSearchAfterWhenSortingByFunctionValues",
                test_search_after_when_sorting_by_function_values,
            ),
        ],
    )
}

fn test_optimized_field_source_function_sorting() -> Result<()> {
    // index contents don't matter for this test.
    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(None);
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    let reader = writer.get_reader()?;
    writer.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let reverse = random().next_boolean();
    let cases: [(Arc<dyn ValueSource>, &str, SortFieldType); 4] = [
        (
            Arc::new(IntFieldSource::new("int_field")),
            "int_field",
            SortFieldType::Int,
        ),
        (
            Arc::new(FloatFieldSource::new("float_field")),
            "float_field",
            SortFieldType::Float,
        ),
        (
            Arc::new(DoubleFieldSource::new("double_field")),
            "double_field",
            SortFieldType::Double,
        ),
        (
            Arc::new(LongFieldSource::new("long_field")),
            "long_field",
            SortFieldType::Long,
        ),
    ];
    for (vs, field, type_) in cases {
        let sf = SortField::new_reverse(Some(field.to_string()), type_, reverse)?;
        let vssf = vs.get_sort_field(reverse)?;
        Assert::assert_equals(&sf, &vssf);
        let sf = sf.rewrite(&searcher)?.into_owned();
        let vssf = vssf.rewrite(&searcher)?.into_owned();
        Assert::assert_equals(&sf, &vssf);
    }

    reader.close()?;
    dir.close()
}

fn test_search_after_when_sorting_by_function_values() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(None);
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?; // depends on docid order
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    let mut doc = Document::new();
    doc.add(Box::new(StoredField::new_number(
        "value",
        NumericValue::Int(0),
    )?));
    doc.add(Box::new(NumericDocValuesField::new("value", 0)));

    // Save docs unsorted (decreasing value n, n-1, ...)
    const NUM_VALS: i32 = 5;
    let mut val = NUM_VALS;
    while val > 0 {
        field_mut_as::<StoredField>(&mut doc, "value").set_int_value(val)?;
        field_mut_as::<NumericDocValuesField>(&mut doc, "value").set_value(i64::from(val));
        writer.add_document(&doc)?;
        val -= 1;
    }

    // Open index
    let reader = writer.get_reader()?;
    writer.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // Trivial ValueSource function that bypasses single field ValueSource sort optimization
    let src: Arc<dyn ValueSource> = Arc::new(SumFloatFunction::new(vec![
        Arc::new(IntFieldSource::new("value")),
        Arc::new(DoubleConstValueSource::new(1.0)),
    ]));
    // ...and make it a sort criterion
    let sf = src.get_sort_field(false)?.rewrite(&searcher)?.into_owned();
    let order_by = Sort::new_fields(vec![sf])?;

    // Get hits sorted by our FunctionValues (ascending values)
    let q: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
    let max_doc = (Arc::clone(&reader) as Arc<dyn IndexReader>).max_doc();
    let hits = searcher.search_with_sort(Arc::clone(&q), max_doc, &order_by)?;
    Assert::assert_equals(NUM_VALS as usize, hits.score_docs.len());
    // Verify that sorting works in general
    let stored_value = |doc: i32| -> Result<i32> {
        let v = (Arc::clone(&reader) as Arc<dyn IndexReader>)
            .stored_fields()?
            .document(doc)?
            .get("value")
            .unwrap_or_default();
        Ok(v.parse::<i32>().unwrap_or_else(|_| {
            randomizedtesting::jdk::exception::Throwable::new(
                "java.lang.NumberFormatException",
                format!("For input string: \"{v}\""),
            )
            .throw()
        }))
    };
    let mut i = 0;
    for hit in &hits.score_docs {
        let value_from_doc = stored_value(hit.score_doc.doc)?;
        i += 1;
        Assert::assert_equals(i, value_from_doc);
    }

    // Now get hits after hit #2 using IS.searchAfter()
    let after_idx = 1usize;
    let after_hit = hits.score_docs[after_idx].clone();
    let hits = searcher.search_after_with_sort(Some(after_hit.clone()), q, max_doc, &order_by)?;

    // Expected # of hits: NUM_VALS - 2
    Assert::assert_equals(NUM_VALS as usize - (after_idx + 1), hits.score_docs.len());

    // Verify that hits are actually "after"
    let after_value = match after_hit.fields.as_ref().map(|f| &f[0]) {
        Some(SortValue::Double(d)) => *d as i32,
        other => randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.ClassCastException",
            format!("{other:?} is not a java.lang.Double"),
        )
        .throw(),
    };
    for hit in &hits.score_docs {
        let val = stored_value(hit.score_doc.doc)?;
        Assert::assert_true(after_value <= val);
        Assert::assert_false(hit.score_doc.doc == after_hit.score_doc.doc);
    }
    reader.close()?;
    dir.close()
}
