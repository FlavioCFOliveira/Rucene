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

//! Port of `StrategyTestCase.java` (package `org.apache.lucene.spatial`):
//! the methods no subclass overrides are [`StrategyTestCaseOps`]; the
//! overridable ones are on [`SpatialCase`].

use std::sync::Arc;

use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, StoredField, StringField};
use rucene::error::{LuceneError, Result};
use rucene::index::terms::Term;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::{Query, TermQuery};
use rucene_spatial_extras::query::spatial_args::SpatialArgs;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::shape::shape::Shape;

use crate::core_support::JavaHashSet;
use crate::spatial::spatial_match_concern::SpatialMatchConcern;
use crate::spatial::spatial_test_case::{
    java_assert_msg, SearchResults, SpatialCase, SpatialFixture, SpatialTestCaseOps,
};
use crate::spatial::spatial_test_data::SpatialTestData;
use crate::spatial::spatial_test_query::SpatialTestQuery;

/// `DATA_SIMPLE_BBOX`.
pub const DATA_SIMPLE_BBOX: &str = "simple-bbox.txt";
/// `DATA_STATES_POLY`.
pub const DATA_STATES_POLY: &str = "states-poly.txt";
/// `DATA_STATES_BBOX`.
pub const DATA_STATES_BBOX: &str = "states-bbox.txt";
/// `DATA_COUNTRIES_POLY`.
pub const DATA_COUNTRIES_POLY: &str = "countries-poly.txt";
/// `DATA_COUNTRIES_BBOX`.
pub const DATA_COUNTRIES_BBOX: &str = "countries-bbox.txt";
/// `DATA_WORLD_CITIES_POINTS`.
pub const DATA_WORLD_CITIES_POINTS: &str = "world-cities-points.txt";

/// `QTEST_States_IsWithin_BBox`.
pub const QTEST_STATES_IS_WITHIN_BBOX: &str = "states-IsWithin-BBox.txt";
/// `QTEST_States_Intersects_BBox`.
pub const QTEST_STATES_INTERSECTS_BBOX: &str = "states-Intersects-BBox.txt";
/// `QTEST_Cities_Intersects_BBox`.
pub const QTEST_CITIES_INTERSECTS_BBOX: &str = "cities-Intersects-BBox.txt";
/// `QTEST_Simple_Queries_BBox`.
pub const QTEST_SIMPLE_QUERIES_BBOX: &str = "simple-Queries-BBox.txt";

/// `getClass().getClassLoader().getResourceAsStream(path)`: the test
/// resources of `lucene/spatial-extras/src/test-files`, shipped in
/// `tests/test-files`; `None` for a missing resource.
pub fn get_resource_as_stream(path: &str) -> Option<Vec<u8>> {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("test-files")
        .join(path);
    std::fs::read(file).ok()
}

/// `StrategyTestCase.newDoc(String id, Shape shape)` (the body a subclass
/// reaches with `super`, and the default of [`SpatialCase::new_doc`]).
///
/// # Errors
///
/// The errors of `strategy.createIndexableFields`.
pub fn strategy_new_doc(
    fx: &SpatialFixture,
    id: &str,
    shape: Option<&Arc<dyn Shape>>,
) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(StringField::new("id", id.to_string(), Store::YES)?);
    if let Some(shape) = shape {
        let strategy = fx.strategy();
        for f in strategy.create_indexable_fields(shape)? {
            doc.add_shared(f);
        }
        if fx.store_shape {
            doc.add(StoredField::new_string(
                strategy.get_field_name().to_string(),
                shape.to_string(),
            )?); // not to be parsed; just for debug
        }
    }
    Ok(doc)
}

/// The methods of `StrategyTestCase` no subclass overrides.
pub trait StrategyTestCaseOps: SpatialCase {
    /// `executeQueries(SpatialMatchConcern concern, String... testQueryFile)`.
    ///
    /// # Errors
    ///
    /// The errors of reading the queries and of searching.
    fn execute_queries(
        &mut self,
        concern: SpatialMatchConcern,
        test_query_file: &[&str],
    ) -> Result<()> {
        self.fx().log.info(&format!(
            "testing queried for strategy {}",
            self.fx().strategy()
        )); // nowarn
        for path in test_query_file {
            let test_queries = self.get_test_queries(path, &self.fx().ctx())?;
            self.run_test_queries(test_queries, concern)?;
        }
        Ok(())
    }

    /// `getAddAndVerifyIndexedDocuments(String testDataFile)`.
    ///
    /// # Errors
    ///
    /// The errors of reading the data and of indexing.
    fn get_add_and_verify_indexed_documents(&mut self, test_data_file: &str) -> Result<()> {
        let test_documents = self.get_documents(test_data_file)?;
        self.add_documents_and_commit(&test_documents)?;
        self.verify_documents_indexed(test_documents.len());
        Ok(())
    }

    /// `getDocuments(String testDataFile)`.
    ///
    /// # Errors
    ///
    /// The errors of reading the data and of the strategy.
    fn get_documents(&mut self, test_data_file: &str) -> Result<Vec<Document>> {
        let sample_data = self.get_sample_data(test_data_file)?;
        self.get_documents_from(sample_data)
    }

    /// `getDocuments(Iterator<SpatialTestData> sampleData)`.
    ///
    /// # Errors
    ///
    /// The errors of the strategy.
    fn get_documents_from(&mut self, sample_data: Vec<SpatialTestData>) -> Result<Vec<Document>> {
        let mut documents = Vec::new();
        for data in sample_data {
            let mut document = Document::new();
            document.add(StringField::new("id", data.id.clone(), Store::YES)?);
            document.add(StringField::new("name", data.name.clone(), Store::YES)?);
            let shape = self.convert_shape_from_get_documents(data.shape);
            if let Some(shape) = shape {
                let strategy = self.fx().strategy();
                for f in strategy.create_indexable_fields(&shape)? {
                    document.add_shared(f);
                }
                if self.fx().store_shape {
                    // just for diagnostics
                    document.add(StoredField::new_string(
                        strategy.get_field_name().to_string(),
                        shape.to_string(),
                    )?);
                }
            }

            documents.push(document);
        }
        Ok(documents)
    }

    /// `getSampleData(String testDataFile)`.
    ///
    /// # Errors
    ///
    /// Java's `FileNotFoundException("classpath resource not found: …")`;
    /// the errors of `SpatialTestData.getTestData`.
    fn get_sample_data(&self, test_data_file: &str) -> Result<Vec<SpatialTestData>> {
        let path = format!("data/{test_data_file}");
        let stream = get_resource_as_stream(&path).ok_or_else(|| {
            LuceneError::from(rucene::error::LuceneErrorKind::NotFound(format!(
                "classpath resource not found: {path}"
            )))
        })?;
        SpatialTestData::get_test_data(&stream, &self.fx().ctx()) // closes the InputStream
    }

    /// `getTestQueries(String testQueryFile, SpatialContext ctx)`.
    ///
    /// # Errors
    ///
    /// The errors of `SpatialTestQuery.getTestQueries` (a missing resource
    /// is Java's `NullPointerException` there).
    fn get_test_queries(
        &self,
        test_query_file: &str,
        ctx: &SpatialContext,
    ) -> Result<Vec<SpatialTestQuery>> {
        let input =
            get_resource_as_stream(test_query_file).ok_or_else(LuceneError::null_pointer)?;
        SpatialTestQuery::get_test_queries(&self.fx().args_parser, ctx, test_query_file, &input)
        // closes the InputStream
    }

    /// `runTestQueries(Iterator<SpatialTestQuery> queries, SpatialMatchConcern concern)`.
    ///
    /// # Errors
    ///
    /// The errors of `runTestQuery`.
    fn run_test_queries(
        &mut self,
        queries: Vec<SpatialTestQuery>,
        concern: SpatialMatchConcern,
    ) -> Result<()> {
        for mut q in queries {
            self.run_test_query(concern, &mut q)?;
        }
        Ok(())
    }

    /// `runTestQuery(SpatialMatchConcern concern, SpatialTestQuery q)`.
    ///
    /// # Errors
    ///
    /// The errors of `makeQuery` and of searching.
    fn run_test_query(
        &mut self,
        concern: SpatialMatchConcern,
        q: &mut SpatialTestQuery,
    ) -> Result<()> {
        let msg = q.to_string(); // "Query: " + q.args.toString(ctx);
        let got = self.execute_query(self.make_query(q)?, 100.max(q.ids.len() as i32 + 1))?;
        if self.fx().store_shape && got.num_found > 0 {
            // check stored value is there
            if got.results[0]
                .document
                .get(self.fx().strategy().get_field_name())
                .is_none()
            {
                Assert::fail_without_message();
            }
        }
        if concern.order_is_important {
            let mut ids = q.ids.iter();
            for r in &got.results {
                let id = r.document.get("id");
                let Some(next) = ids.next() else {
                    Assert::fail(&format!(
                        "{msg} :: Did not get enough results. Expected [{}], got: {}",
                        q.ids.join(", "),
                        got.to_debug_string()
                    ));
                };
                Assert::assert_equals_str(
                    Some(&format!("out of order: {msg}")),
                    Some(next),
                    id.as_deref(),
                );
            }
            if let Some(next) = ids.next() {
                Assert::fail(&format!("{msg} :: expect more results then we got: {next}"));
            }
        } else {
            // We are looking at how the results overlap
            if concern.results_are_superset {
                let mut found: JavaHashSet<String> = JavaHashSet::new();
                for r in &got.results {
                    found.add(r.document.get("id").unwrap_or_else(|| "null".to_string()));
                }
                for s in &q.ids {
                    if !found.contains(s) {
                        Assert::fail(&format!(
                            "Results are missing id: {s} :: [{}]",
                            found.to_vec().join(", ")
                        ));
                    }
                }
            } else {
                let mut found: Vec<String> = Vec::new();
                for r in &got.results {
                    found.push(r.document.get("id").unwrap_or_else(|| "null".to_string()));
                }

                // sort both so that the order is not important
                crate::spatial_support::java_string_sort(&mut q.ids);
                crate::spatial_support::java_string_sort(&mut found);
                Assert::assert_equals_str(
                    Some(&msg),
                    Some(&format!("[{}]", q.ids.join(", "))),
                    Some(&format!("[{}]", found.join(", "))),
                );
            }
        }
        Ok(())
    }

    /// `adoc(String id, String shapeStr)`; `None` stands for `null`.
    ///
    /// # Errors
    ///
    /// The `ParseException` of reading the WKT; the errors of indexing.
    fn adoc_wkt(&mut self, id: &str, shape_str: Option<&str>) -> Result<()> {
        let shape = match shape_str {
            None => None,
            Some(s) => Some(self.fx().ctx().read_shape_from_wkt(s)?),
        };
        let doc = self.new_doc(id, shape.as_ref())?;
        self.add_document(&doc)
    }

    /// `adoc(String id, Shape shape)`; `None` stands for `null`.
    ///
    /// # Errors
    ///
    /// The errors of indexing.
    fn adoc(&mut self, id: &str, shape: Option<Arc<dyn Shape>>) -> Result<()> {
        let doc = self.new_doc(id, shape.as_ref())?;
        self.add_document(&doc)
    }

    /// `deleteDoc(String id)`.
    ///
    /// # Errors
    ///
    /// The errors of `indexWriter.deleteDocuments`.
    fn delete_doc(&mut self, id: &str) -> Result<()> {
        let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("id", id)));
        self.fx().index_writer().delete_documents_by_query(query)?;
        Ok(())
    }

    /// `checkValueSource(DoubleValuesSource vs, float[] scores, float delta)`:
    /// `scores[]` are in docId order.
    ///
    /// # Errors
    ///
    /// The errors of reading the values.
    fn check_value_source(
        &self,
        vs: Arc<dyn DoubleValuesSource>,
        scores: &[f32],
        delta: f32,
    ) -> Result<()> {
        for ctx in Arc::clone(self.fx().index_searcher().get_top_reader_context()).leaves() {
            let mut v = Arc::clone(&vs).get_values(&ctx, None)?;
            let count = ctx.leaf_reader().max_doc();
            for i in 0..count {
                Assert::assert_true(v.advance_exact(i)?);
                let doc = i + ctx.doc_base();
                Assert::assert_equals_double_msg(
                    Some(&format!("Not equal for doc {doc}")),
                    v.double_value()?,
                    f64::from(scores[doc as usize]),
                    f64::from(delta),
                );
            }
        }
        Ok(())
    }

    /// `testOperation(Shape indexedShape, SpatialOperation operation, Shape
    /// queryShape, boolean match)`.
    ///
    /// # Errors
    ///
    /// The errors of indexing and searching.
    fn test_operation(
        &mut self,
        indexed_shape: Arc<dyn Shape>,
        operation: SpatialOperation,
        query_shape: Arc<dyn Shape>,
        match_: bool,
    ) -> Result<()> {
        Assert::assert_true_msg(
            "Faulty test",
            operation.evaluate(&*indexed_shape, &*query_shape) == match_
                || (indexed_shape.equals(&*query_shape)
                    && (operation == SpatialOperation::Contains
                        || operation == SpatialOperation::IsWithin)),
        );
        self.adoc("0", Some(Arc::clone(&indexed_shape)))?;
        self.commit()?;
        let query = self
            .fx()
            .strategy()
            .make_query(&SpatialArgs::new(operation, Arc::clone(&query_shape)))?;
        let got: SearchResults = self.execute_query(query, 1)?;
        java_assert_msg(got.num_found <= 1, "unclean test env");
        if (got.num_found == 1) != match_ {
            Assert::fail(&format!("{operation} I:{indexed_shape} Q:{query_shape}"));
        }
        self.delete_all() // clean up after ourselves
    }
}

impl<T: SpatialCase> StrategyTestCaseOps for T {}
