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

//! The types declared in `SpatialFileQueryMaker.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene::search::query::Query;
use rucene_queries::function::function_score_query::FunctionScoreQuery;
use rucene_spatial_extras::query::spatial_args::SpatialArgs;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use rucene_spatial_extras::spatial_strategy::SpatialStrategy;
use spatial4j::shape::shape::Shape;

use crate::by_task::feeds::abstract_query_maker::{AbstractQueryMaker, AbstractQueryMakerBase};
use crate::by_task::feeds::content_items_source::ContentItemsSource;
use crate::by_task::feeds::content_source::ContentSource;
use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::feeds::line_doc_source::LineDocSource;
use crate::by_task::feeds::no_more_data_exception::NoMoreDataException;
use crate::by_task::feeds::spatial_doc_maker::{ShapeConverter, SpatialDocMaker};
use crate::by_task::utils::config::Config;
use crate::jdk;
use crate::jdk::properties::Properties;

/// The fields `setConfig` sets.
#[derive(Default)]
struct State {
    strategy: Option<Arc<dyn SpatialStrategy>>,
    /// NaN if not set
    dist_err_pct: f64,
    operation: Option<SpatialOperation>,
    score: bool,
    shape_converter: Option<Arc<dyn ShapeConverter>>,
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.SpatialFileQueryMaker`
/// (`public class SpatialFileQueryMaker extends AbstractQueryMaker`): reads
/// spatial data from the body field docs from an internally created
/// [`LineDocSource`]. It's parsed by `SpatialContext.readShapeFromWkt(String)`
/// and then further manipulated via a configurable [`ShapeConverter`]. When
/// using point data, it's likely you'll want to configure the shape converter
/// so that the query shapes actually cover a region. The queries are all
/// created and cached in advance. This query maker works in conjunction with
/// [`SpatialDocMaker`]. See spatial.alg for a listing of options, in
/// particular the options starting with "query.".
#[derive(Default)]
pub struct SpatialFileQueryMaker {
    base: AbstractQueryMakerBase,
    state: Mutex<State>,
}

impl SpatialFileQueryMaker {
    /// `new SpatialFileQueryMaker()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `makeQueryFromShape(Shape shape)`.
    ///
    /// # Errors
    ///
    /// The exception of the strategy.
    pub fn make_query_from_shape(&self, shape: Arc<dyn Shape>) -> Result<Arc<dyn Query>> {
        let (strategy, dist_err_pct, operation, score) = {
            let s = jdk::lock(&self.state);
            (s.strategy.clone(), s.dist_err_pct, s.operation, s.score)
        };
        let strategy = strategy.ok_or_else(LuceneError::null_pointer)?;
        let operation = operation.ok_or_else(LuceneError::null_pointer)?;
        let args = SpatialArgs::new(operation, Arc::clone(&shape));
        if !dist_err_pct.is_nan() {
            args.set_dist_err_pct(Some(dist_err_pct));
        }
        let filter_query = strategy.make_query(&args)?;
        if score {
            // wrap with distance computing query
            let value_source = strategy.make_distance_value_source_default(&shape.get_center())?;
            Ok(Arc::new(FunctionScoreQuery::new(
                filter_query,
                value_source,
            )))
        } else {
            Ok(filter_query) // assume constant scoring
        }
    }
}

impl AbstractQueryMaker for SpatialFileQueryMaker {
    fn aqm_base(&self) -> &AbstractQueryMakerBase {
        &self.base
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.benchmark.byTask.feeds.SpatialFileQueryMaker"
    }

    fn set_config(&self, config: Arc<Config>) -> Result<()> {
        let strategy = SpatialDocMaker::get_spatial_strategy(config.get_round_number())?;
        let shape_converter = SpatialDocMaker::make_shape_converter(
            Arc::clone(&strategy),
            &config,
            "query.spatial.",
        )?;
        let dist_err_pct = config.get_double("query.spatial.distErrPct", f64::NAN)?;
        let operation = SpatialOperation::get(
            &config
                .get_str("query.spatial.predicate", Some("Intersects"))?
                .unwrap_or_default(),
        )?;
        let score = config.get_bool("query.spatial.score", false)?;
        {
            let mut s = jdk::lock(&self.state);
            s.strategy = Some(strategy);
            s.shape_converter = Some(shape_converter);
            s.dist_err_pct = dist_err_pct;
            s.operation = Some(operation);
            s.score = score;
        }
        // super.setConfig(config): call last, will call prepareQueries()
        crate::by_task::feeds::abstract_query_maker::set_config_base(self, config)
    }

    fn prepare_queries(&self) -> Result<Vec<Arc<dyn Query>>> {
        let config = self.base.config().ok_or_else(LuceneError::null_pointer)?;
        let max_queries = config.get_int("query.file.maxQueries", 1000)?;
        let src_config = Arc::new(Config::from_properties(Properties::new()));
        let set = |name: &str, value: Option<String>| -> Result<()> {
            // `Properties.setProperty(name, null)` throws NullPointerException.
            let value = value.ok_or_else(LuceneError::null_pointer)?;
            src_config.set(name, &value)
        };
        set("docs.file", config.get_str("query.file", None)?)?;
        set(
            "line.parser",
            config.get_str("query.file.line.parser", None)?,
        )?;
        set("content.source.forever", Some("false".to_string()))?;

        let mut queries: Vec<Arc<dyn Query>> = Vec::new();
        let src = LineDocSource::new();
        let (strategy, shape_converter) = {
            let s = jdk::lock(&self.state);
            (s.strategy.clone(), s.shape_converter.clone())
        };
        let strategy = strategy.ok_or_else(LuceneError::null_pointer)?;
        let shape_converter = shape_converter.ok_or_else(LuceneError::null_pointer)?;
        let result = (|| -> Result<()> {
            src.set_config(Arc::clone(&src_config))?;
            src.reset_inputs()?;
            let mut doc_data = DocData::new();
            let mut i = 0;
            while i < max_queries {
                src.get_next_doc_data(&mut doc_data)?;
                match SpatialDocMaker::make_shape_from_string(
                    &*strategy,
                    doc_data.get_name(),
                    doc_data.get_body(),
                ) {
                    Some(shape) => {
                        let shape = shape_converter.convert(shape)?;
                        queries.push(self.make_query_from_shape(shape)?);
                    }
                    None => i -= 1, // skip
                }
                i += 1;
            }
            Ok(())
        })();
        let closed = src.close();
        match result {
            Ok(()) => {}
            Err(e) if NoMoreDataException::is(&e) => {
                // all-done
            }
            Err(e) => return Err(e),
        }
        closed?;
        Ok(queries)
    }
}
