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

//! The types declared in `SpatialDocMaker.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use rucene::document::Document;
use rucene::error::{LuceneError, Result};
use rucene::util::java_random::JavaRandom;
use rucene_spatial_extras::composite::composite_spatial_strategy::CompositeSpatialStrategy;
use rucene_spatial_extras::prefix::prefix_tree_strategy::PrefixTreeStrategy;
use rucene_spatial_extras::prefix::recursive_prefix_tree_strategy::{
    RecursivePrefixTreeStrategy, RecursivePrefixTreeStrategyOps,
};
use rucene_spatial_extras::prefix::tree::packed_quad_prefix_tree::PackedQuadPrefixTree;
use rucene_spatial_extras::prefix::tree::spatial_prefix_tree_factory;
use rucene_spatial_extras::serialized::serialized_dv_strategy::SerializedDVStrategy;
use rucene_spatial_extras::spatial_strategy::SpatialStrategy;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::context::spatial_context_factory::SpatialContextFactory;
use spatial4j::shape::shape::Shape;

use crate::by_task::feeds::content_source::ContentSource;
use crate::by_task::feeds::doc_maker::{DocMaker, DocMakerClass};
use crate::by_task::utils::config::Config;
use crate::jdk;

/// `spatialStrategyCache`: cache spatialStrategy by round number.
static SPATIAL_STRATEGY_CACHE: LazyLock<Mutex<HashMap<i32, Arc<dyn SpatialStrategy>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The `spatial.*` keys the spatial context factory and the prefix tree
/// factory read from the configuration map.
const SPATIAL_KEYS: [&str; 16] = [
    "spatialContextFactory",
    "geo",
    "shapeFactoryClass",
    "normWrapLongitude",
    "binaryCodecClass",
    "hasFormatConfig",
    "distCalculator",
    "worldBounds",
    "readers",
    "writers",
    "wktShapeParserClass",
    "planetModel",
    "prefixTree",
    "maxLevels",
    "maxDistErr",
    "version",
];

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.SpatialDocMaker.ShapeConverter`
/// (`public interface ShapeConverter`): converts one shape to another.
/// Created by [`SpatialDocMaker::make_shape_converter`].
pub trait ShapeConverter: Send + Sync {
    /// `convert(Shape shape)`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of building the new shape.
    fn convert(&self, shape: Arc<dyn Shape>) -> Result<Arc<dyn Shape>>;
}

/// The anonymous `ShapeConverter` of `makeShapeConverter`.
struct DefaultShapeConverter {
    spatial_strategy: Arc<dyn SpatialStrategy>,
    radius_degrees: f64,
    plus_minus: f64,
    bbox: bool,
}

impl ShapeConverter for DefaultShapeConverter {
    fn convert(&self, shape: Arc<dyn Shape>) -> Result<Arc<dyn Shape>> {
        let mut shape = shape;
        let point = shape.as_point().map(|p| (p.get_x(), p.get_y(), shape.hash_code()));
        if let Some((x, y, hash)) = point {
            if self.radius_degrees != 0.0 || self.plus_minus != 0.0 {
                let mut radius = self.radius_degrees;
                if self.plus_minus > 0.0 {
                    let mut random = JavaRandom::new(i64::from(hash)); // use hashCode so it's reproducibly random
                    radius += random.next_double() * 2.0 * self.plus_minus - self.plus_minus;
                    radius = radius.abs(); // can happen if configured plusMinus > radiusDegrees
                }
                shape = self.spatial_strategy.get_spatial_context().make_circle(x, y, radius)?;
            }
        }
        if self.bbox {
            shape = shape.get_bounding_box();
        }
        Ok(shape)
    }
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.SpatialDocMaker`
/// (`public class SpatialDocMaker extends DocMaker`): indexes spatial data
/// according to a configured `SpatialStrategy` with optional shape
/// transformation via a configured [`ShapeConverter`]. The converter can
/// turn points into circles and bounding boxes, in order to vary the type of
/// indexing performance tests. Unless it's subclass-ed to do otherwise, this
/// class configures a `SpatialContext`, `SpatialPrefixTree`, and
/// `RecursivePrefixTreeStrategy`. The Strategy is made available to a query
/// maker via the static method [`get_spatial_strategy`](Self::get_spatial_strategy).
/// See spatial.alg for a listing of spatial parameters, in particular those
/// starting with "spatial." and "doc.spatial".
///
/// **Divergence.** Java hands the spatial factories a lazy `Map` view of the
/// config, whose `get(key)` is `config.get("spatial." + key, null)`
/// (`SpatialDocMaker.java:72-85`). spatial4j's port takes a `HashMap`, so
/// the map is built from the `spatial.*` keys the factories read
/// ([`SPATIAL_KEYS`]); a key read by round is extracted when the map is
/// built rather than when a factory reads it.
pub struct SpatialDocMaker {
    inner: DocMaker,
    /// `(strategy, shapeConverter)`.
    state: Mutex<(Option<Arc<dyn SpatialStrategy>>, Option<Arc<dyn ShapeConverter>>)>,
}

impl Default for SpatialDocMaker {
    fn default() -> Self {
        Self::new()
    }
}

impl SpatialDocMaker {
    /// `SPATIAL_FIELD`.
    pub const SPATIAL_FIELD: &'static str = "spatial";

    /// `new SpatialDocMaker()`.
    pub fn new() -> Self {
        SpatialDocMaker { inner: DocMaker::new(), state: Mutex::new((None, None)) }
    }

    /// `getSpatialStrategy(int roundNumber)`: looks up the SpatialStrategy
    /// from the given round -- `Config.getRoundNumber()`. It's an error if it
    /// wasn't created already for this round -- when SpatialDocMaker is
    /// initialized.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("Strategy should have been init'ed by
    /// SpatialDocMaker by now")`.
    pub fn get_spatial_strategy(round_number: i32) -> Result<Arc<dyn SpatialStrategy>> {
        jdk::lock(&SPATIAL_STRATEGY_CACHE).get(&round_number).cloned().ok_or_else(|| {
            LuceneError::illegal_state("Strategy should have been init'ed by SpatialDocMaker by now")
        })
    }

    /// The configuration map: `config.get("spatial." + key, null)` for each
    /// key the factories read.
    fn config_map(config: &Config) -> Result<HashMap<String, String>> {
        let mut map = HashMap::new();
        for key in SPATIAL_KEYS {
            if let Some(value) = config.get_str(&format!("spatial.{key}"), None)? {
                map.insert(key.to_string(), value);
            }
        }
        Ok(map)
    }

    /// `makeSpatialStrategy(Config config)`: builds a SpatialStrategy from
    /// configuration options.
    ///
    /// # Errors
    ///
    /// The exception of the factories.
    pub fn make_spatial_strategy(&self, config: &Config) -> Result<Arc<dyn SpatialStrategy>> {
        // A Map view of Config that prefixes keys with "spatial."
        let config_map = Self::config_map(config)?;
        let ctx = SpatialContextFactory::make_spatial_context(&config_map)?;
        self.make_spatial_strategy_with(config, &config_map, ctx)
    }

    /// `makeSpatialStrategy(Config config, Map<String, String> configMap,
    /// SpatialContext ctx)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("Unknown spatial.strategy: ...")`.
    pub fn make_spatial_strategy_with(
        &self,
        config: &Config,
        config_map: &HashMap<String, String>,
        ctx: Arc<SpatialContext>,
    ) -> Result<Arc<dyn SpatialStrategy>> {
        // TODO once strategies have factories, we could use them here.
        let strategy_name = config.get_str("spatial.strategy", Some("rpt"))?.unwrap_or_default();
        match strategy_name.as_str() {
            "rpt" => Ok(self.make_rpt_strategy(Self::SPATIAL_FIELD, config, config_map, ctx)?),
            "composite" => self.make_composite_strategy(config, config_map, ctx),
            // TODO add more as-needed
            _ => Err(LuceneError::illegal_state(format!("Unknown spatial.strategy: {strategy_name}"))),
        }
    }

    /// `makeRPTStrategy(String spatialField, Config config, Map<String,
    /// String> configMap, SpatialContext ctx)`.
    ///
    /// # Errors
    ///
    /// The exception of the prefix tree factory.
    pub fn make_rpt_strategy(
        &self,
        spatial_field: &str,
        config: &Config,
        config_map: &HashMap<String, String>,
        ctx: Arc<SpatialContext>,
    ) -> Result<Arc<RecursivePrefixTreeStrategy>> {
        // A factory for the prefix tree grid
        let grid = spatial_prefix_tree_factory::make_spt(config_map, &ctx)?;

        let strategy = RecursivePrefixTreeStrategy::new(Arc::clone(&grid), spatial_field)?;
        strategy.set_points_only(config.get_bool("spatial.docPointsOnly", false)?);
        let prune_leafy_branches = config.get_bool("spatial.pruneLeafyBranches", true)?;
        if let Some(packed) = grid.as_any().downcast_ref::<PackedQuadPrefixTree>() {
            packed.set_prune_leafy_branches(prune_leafy_branches);
            strategy.set_prune_leafy_branches(false); // always leave it to packed grid, even though it isn't the same
        } else {
            strategy.set_prune_leafy_branches(prune_leafy_branches);
        }

        let mut prefix_grid_scan_level = config.get_int("query.spatial.prefixGridScanLevel", -4)?;
        if prefix_grid_scan_level < 0 {
            prefix_grid_scan_level += grid.get_max_levels();
        }
        strategy.set_prefix_grid_scan_level(prefix_grid_scan_level);

        let dist_err_pct = config.get_double("spatial.distErrPct", 0.025)?; // doc & query; a default
        strategy.set_dist_err_pct(dist_err_pct);
        Ok(strategy)
    }

    /// `makeSerializedDVStrategy(String spatialField, Config config,
    /// Map<String, String> configMap, SpatialContext ctx)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of an empty field name.
    pub fn make_serialized_dv_strategy(
        &self,
        spatial_field: &str,
        _config: &Config,
        _config_map: &HashMap<String, String>,
        ctx: Arc<SpatialContext>,
    ) -> Result<Arc<SerializedDVStrategy>> {
        Ok(Arc::new(SerializedDVStrategy::new(ctx, spatial_field)?))
    }

    /// `makeCompositeStrategy(Config config, Map<String, String> configMap,
    /// SpatialContext ctx)`.
    ///
    /// # Errors
    ///
    /// The exception of building the strategies.
    pub fn make_composite_strategy(
        &self,
        config: &Config,
        config_map: &HashMap<String, String>,
        ctx: Arc<SpatialContext>,
    ) -> Result<Arc<dyn SpatialStrategy>> {
        let rpt: Arc<dyn RecursivePrefixTreeStrategyOps> =
            self.make_rpt_strategy(&format!("{}_rpt", Self::SPATIAL_FIELD), config, config_map, Arc::clone(&ctx))?;
        let sdv = self.make_serialized_dv_strategy(&format!("{}_sdv", Self::SPATIAL_FIELD), config, config_map, ctx)?;
        let strategy = CompositeSpatialStrategy::new(Self::SPATIAL_FIELD, rpt, sdv)?;
        strategy.set_optimize_predicates(config.get_bool("query.spatial.composite.optimizePredicates", true)?);
        Ok(Arc::new(strategy))
    }

    /// `makeShapeConverter(SpatialStrategy spatialStrategy, Config config,
    /// String configKeyPrefix)`: optionally converts points to circles, and
    /// optionally bbox'es result.
    ///
    /// # Errors
    ///
    /// The exception of reading the configuration.
    pub fn make_shape_converter(
        spatial_strategy: Arc<dyn SpatialStrategy>,
        config: &Config,
        config_key_prefix: &str,
    ) -> Result<Arc<dyn ShapeConverter>> {
        // by default does no conversion
        let radius_degrees = config.get_double(&format!("{config_key_prefix}radiusDegrees"), 0.0)?;
        let plus_minus = config.get_double(&format!("{config_key_prefix}radiusDegreesRandPlusMinus"), 0.0)?;
        let bbox = config.get_bool(&format!("{config_key_prefix}bbox"), false)?;
        Ok(Arc::new(DefaultShapeConverter { spatial_strategy, radius_degrees, plus_minus, bbox }))
    }

    /// `makeShapeFromString(SpatialStrategy strategy, String name, String
    /// shapeStr)`: the WKT shape, or `None` (printing why) when it does not
    /// parse.
    pub fn make_shape_from_string(strategy: &dyn SpatialStrategy, name: Option<&str>, shape_str: Option<&str>) -> Option<Arc<dyn Shape>> {
        if let Some(shape_str) = shape_str.filter(|s| !s.is_empty()) {
            match strategy.get_spatial_context().read_shape_from_wkt(shape_str) {
                Ok(shape) => return Some(shape),
                Err(e) => {
                    // InvalidShapeException TODO
                    eprintln!("Shape {} wasn't parseable: {e}  (skipping it)", name.unwrap_or("null"));
                    return None;
                }
            }
        }
        None
    }
}

impl DocMakerClass for SpatialDocMaker {
    fn doc_maker(&self) -> &DocMaker {
        &self.inner
    }

    fn set_config(&self, config: Arc<Config>, source: Arc<dyn ContentSource>) -> Result<()> {
        self.inner.set_config(Arc::clone(&config), source)?;
        let round = config.get_round_number();
        let existing = jdk::lock(&SPATIAL_STRATEGY_CACHE).get(&round).cloned();
        if existing.is_none() {
            // new round; we need to re-initialize
            let strategy = self.make_spatial_strategy(&config)?;
            jdk::lock(&SPATIAL_STRATEGY_CACHE).insert(round, Arc::clone(&strategy));
            // TODO remove previous round config?
            let shape_converter = Self::make_shape_converter(Arc::clone(&strategy), &config, "doc.spatial.")?;
            println!("Spatial Strategy: {strategy}");
            *jdk::lock(&self.state) = (Some(strategy), Some(shape_converter));
        }
        Ok(())
    }

    fn make_document(&self) -> Result<Document> {
        let doc_state = self.inner.get_doc_state()?;
        let mut doc = self.inner.make_document()?;

        // Set SPATIAL_FIELD from body
        let name = jdk::lock(&doc_state).doc_data.get_name().map(str::to_string);
        //   makeDocument() resets docState.getBody() so we can't look there; look in Document
        let shape_str = doc
            .get_field(DocMaker::BODY_FIELD)
            .ok_or_else(LuceneError::null_pointer)?
            .string_value();
        let (strategy, shape_converter) = jdk::lock(&self.state).clone();
        let strategy = strategy.ok_or_else(LuceneError::null_pointer)?;
        if let Some(shape) = Self::make_shape_from_string(&*strategy, name.as_deref(), shape_str.as_deref()) {
            let shape = shape_converter.ok_or_else(LuceneError::null_pointer)?.convert(shape)?;
            // index
            for f in strategy.create_indexable_fields(&shape)? {
                doc.add_shared(f);
            }
        }
        Ok(doc)
    }

    fn make_document_of_size(&self, _size: i32) -> Result<Document> {
        // TODO consider abusing the 'size' notion to number of shapes per document
        Err(LuceneError::unchecked_with_message("java.lang.UnsupportedOperationException", None))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
