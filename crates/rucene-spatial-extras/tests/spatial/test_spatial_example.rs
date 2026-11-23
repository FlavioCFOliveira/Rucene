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

//! Port of `TestSpatialExample.java` (package `org.apache.lucene.spatial`):
//! this class serves as example code to show how to use the Lucene spatial
//! module.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericDocValuesField, StoredField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::IndexReader;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::top_field_docs::TopFieldDocs;
use rucene::search::{
    double_values_source, IndexSearcher, MatchAllDocsQuery, Query, TotalHitsRelation,
};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene_spatial_extras::prefix::recursive_prefix_tree_strategy::RecursivePrefixTreeStrategy;
use rucene_spatial_extras::prefix::tree::geohash_prefix_tree::GeohashPrefixTree;
use rucene_spatial_extras::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use rucene_spatial_extras::query::spatial_args::SpatialArgs;
use rucene_spatial_extras::query::spatial_args_parser::SpatialArgsParser;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use rucene_spatial_extras::spatial_strategy::SpatialStrategy;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::distance::distance_utils::DistanceUtils;
use spatial4j::shape::shape::Shape;

use crate::core_support::{class, test};
use crate::spatial::spatial_test_case::java_assert;

/// `TestSpatialExample`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial.TestSpatialExample",
        vec![test("test", || TestSpatialExample::new().test())],
    )
}

/// `main(String[] args)`: `new TestSpatialExample().test()`.
///
/// Note: Test invoked via `TestTestFramework.spatialExample()`.
pub fn main() -> Result<()> {
    TestSpatialExample::new().test()
}

/// The `TestSpatialExample` instance.
struct TestSpatialExample {
    /// `ctx`: the Spatial4j SpatialContext is a sort of global-ish singleton
    /// needed by Lucene spatial. It's a facade to the rest of Spatial4j,
    /// acting as a factory for Shapes and provides access to reading and
    /// writing them from Strings.
    ctx: Option<Arc<SpatialContext>>, // "ctx" is the conventional variable name

    /// `strategy`: the Lucene spatial SpatialStrategy encapsulates an
    /// approach to indexing and searching shapes, and providing distance
    /// values for them. It's a simple API to unify different approaches.
    /// You might use more than one strategy for a shape as each strategy has
    /// its strengths and weaknesses.
    ///
    /// Note that these are initialized with a field name.
    strategy: Option<Arc<dyn SpatialStrategy>>,

    directory: Option<Arc<dyn Directory>>,
}

impl TestSpatialExample {
    fn new() -> Self {
        Self {
            ctx: None,
            strategy: None,
            directory: None,
        }
    }

    fn ctx(&self) -> &Arc<SpatialContext> {
        self.ctx.as_ref().expect("init() sets ctx")
    }

    fn strategy(&self) -> &Arc<dyn SpatialStrategy> {
        self.strategy.as_ref().expect("init() sets strategy")
    }

    /// `test()`.
    fn test(&mut self) -> Result<()> {
        self.init()?;
        self.index_points()?;
        self.search()
    }

    /// `init()`.
    fn init(&mut self) -> Result<()> {
        // Typical geospatial context
        //  These can also be constructed from SpatialContextFactory
        self.ctx = Some(SpatialContext::geo());

        let max_levels = 11; // results in sub-meter precision for geohash
                             // TODO demo lookup by detail distance
                             //  This can also be constructed from SpatialPrefixTreeFactory
        let grid: Arc<dyn SpatialPrefixTree> =
            GeohashPrefixTree::new(Arc::clone(self.ctx()), max_levels)?;

        self.strategy = Some(RecursivePrefixTreeStrategy::new(grid, "myGeoField")?);

        self.directory = Some(Arc::new(ByteBuffersDirectory::new()));
        Ok(())
    }

    /// `indexPoints()`.
    fn index_points(&self) -> Result<()> {
        let iw_config = IndexWriterConfig::with_analyzer(None);
        let index_writer = IndexWriter::new(
            Arc::clone(self.directory.as_ref().expect("init() sets directory")),
            iw_config,
        )?;

        let sf = self.ctx().get_shape_factory();
        // Spatial4j is x-y order for arguments
        index_writer.add_document(
            &self.new_sample_document(2, &[sf.point_xy(-80.93, 33.77)? as Arc<dyn Shape>])?,
        )?;

        // Spatial4j has a WKT parser which is also "x y" order
        index_writer.add_document(
            &self.new_sample_document(
                4,
                &[self
                    .ctx()
                    .read_shape_from_wkt("POINT(60.9289094 -50.7693246)")?],
            )?,
        )?;

        index_writer.add_document(&self.new_sample_document(
            20,
            &[
                sf.point_xy(0.1, 0.1)? as Arc<dyn Shape>,
                sf.point_xy(0.0, 0.0)?,
            ],
        )?)?;

        index_writer.close()?;
        Ok(())
    }

    /// `newSampleDocument(int id, Shape... shapes)`.
    fn new_sample_document(&self, id: i32, shapes: &[Arc<dyn Shape>]) -> Result<Document> {
        let mut doc = Document::new();
        doc.add(StoredField::new_number(
            "id",
            rucene::document::NumericValue::Int(id),
        )?);
        doc.add(NumericDocValuesField::new("id", i64::from(id)));
        // Potentially more than one shape in this field is supported by some
        // strategies; see the javadocs of the SpatialStrategy impl to see.
        for shape in shapes {
            for f in self.strategy().create_indexable_fields(shape)? {
                doc.add_shared(f);
            }
            // store it too; the format is up to you
            //  (assume point in this example)
            let pt = shape
                .as_point()
                .expect("java.lang.ClassCastException: the shape is a Point");
            doc.add(StoredField::new_string(
                self.strategy().get_field_name().to_string(),
                format!(
                    "{} {}",
                    rucene::util::java_fmt::double_to_string(pt.get_x()),
                    rucene::util::java_fmt::double_to_string(pt.get_y())
                ),
            )?);
        }

        Ok(doc)
    }

    /// `search()`.
    fn search(&self) -> Result<()> {
        let index_reader = directory_reader::open(Arc::clone(
            self.directory.as_ref().expect("init() sets directory"),
        ))?;
        let index_searcher = IndexSearcher::new(Arc::clone(&index_reader) as Arc<dyn IndexReader>)?;
        let id_sort = Sort::new_fields(vec![SortField::new(
            Some("id".to_string()),
            SortFieldType::Int,
        )?])?;

        // --Filter by circle (<= distance from a point)
        {
            // Search with circle
            // note: SpatialArgs can be parsed from a string
            let args = SpatialArgs::new(
                SpatialOperation::Intersects,
                self.ctx().get_shape_factory().circle(
                    -80.0,
                    33.0,
                    DistanceUtils::dist2_degrees(200.0, DistanceUtils::EARTH_MEAN_RADIUS_KM),
                )?,
            );
            let query = self.strategy().make_query(&args)?;
            let docs = index_searcher.search_with_sort(query, 10, &id_sort)?;
            assert_doc_matched_ids(&index_searcher, &docs, &[2])?;
            // Now, lets get the distance for the 1st doc via computing from stored point value:
            // (this computation is usually not redundant)
            let doc1 = index_searcher
                .stored_fields()?
                .document(docs.score_docs[0].score_doc.doc)?;
            let doc1_str = doc1
                .get_field(self.strategy().get_field_name())
                .and_then(|f| f.string_value())
                .expect("the stored point");
            // assume doc1Str is "x y" as written in newSampleDocument()
            let space_idx = doc1_str.find(' ').expect("x y");
            let x: f64 = crate::spatial_support::java_parse_double(&doc1_str[..space_idx])?;
            let y: f64 = crate::spatial_support::java_parse_double(&doc1_str[space_idx + 1..])?;
            let doc1_dist_deg = self
                .ctx()
                .calc_distance_xy(&*args.get_shape().get_center(), x, y);
            Assert::assert_equals_double_msg(
                None,
                121.6,
                DistanceUtils::degrees2_dist(doc1_dist_deg, DistanceUtils::EARTH_MEAN_RADIUS_KM),
                0.1,
            );
            // or more simply:
            Assert::assert_equals_double_msg(
                None,
                121.6,
                doc1_dist_deg * DistanceUtils::DEG_TO_KM,
                0.1,
            );
        }
        // --Match all, order by distance ascending
        {
            let pt = self.ctx().get_shape_factory().point_xy(60.0, -50.0)?;
            let value_source = self
                .strategy()
                .make_distance_value_source(&pt, DistanceUtils::DEG_TO_KM)?; // the distance (in km)
            let dist_sort = Sort::new_fields(vec![double_values_source::get_sort_field(
                value_source,
                false,
            )?])?
            .rewrite(&index_searcher)?
            .into_owned(); // false=asc dist
            let docs = index_searcher.search_with_sort(
                Arc::new(MatchAllDocsQuery::instance()) as Arc<dyn Query>,
                10,
                &dist_sort,
            )?;
            assert_doc_matched_ids(&index_searcher, &docs, &[4, 20, 2])?;
            // To get the distance, we could compute from stored values like earlier.
            // However in this example we sorted on it, and the distance will get
            // computed redundantly.  If the distance is only needed for the top-X
            // search results then that's not a big deal. Alternatively, try wrapping
            // the ValueSource with CachingDoubleValueSource then retrieve the value
            // from the ValueSource now. See LUCENE-4541 for an example.
        }
        // demo arg parsing
        {
            let args = SpatialArgs::new(
                SpatialOperation::Intersects,
                self.ctx().get_shape_factory().circle(-80.0, 33.0, 1.0)?,
            );
            let args2 = SpatialArgsParser::new()
                .parse("Intersects(BUFFER(POINT(-80 33),1))", self.ctx())?;
            Assert::assert_equals(args.to_string(), args2.to_string());
        }

        index_reader.close()?;
        Ok(())
    }
}

/// `assertDocMatchedIds(IndexSearcher indexSearcher, TopDocs docs, int... ids)`.
fn assert_doc_matched_ids(
    index_searcher: &IndexSearcher,
    docs: &TopFieldDocs,
    ids: &[i32],
) -> Result<()> {
    java_assert(docs.total_hits.relation() == TotalHitsRelation::EQUAL_TO);
    let mut got_ids = vec![0i32; usize::try_from(docs.total_hits.value()).expect("toIntExact")];
    for i in 0..got_ids.len() {
        got_ids[i] = index_searcher
            .stored_fields()?
            .document(docs.score_docs[i].score_doc.doc)?
            .get_field("id")
            .and_then(|f| f.numeric_value())
            .map(crate::spatial_support::number_int_value)
            .expect("the stored id");
    }
    crate::core_support::assert_array_equals(ids, &got_ids);
    Ok(())
}
