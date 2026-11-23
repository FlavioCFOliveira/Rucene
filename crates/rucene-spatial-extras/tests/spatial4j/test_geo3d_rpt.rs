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

//! Port of `TestGeo3dRpt.java` (package `org.apache.lucene.spatial.spatial4j`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Test;
use rucene::error::Result;
use rucene_spatial3d::geom::geo_area_shape::GeoAreaShape;
use rucene_spatial3d::geom::geo_path_factory::GeoPathFactory;
use rucene_spatial3d::geom::geo_point::GeoPoint;
use rucene_spatial3d::geom::geo_polygon_factory::GeoPolygonFactory;
use rucene_spatial3d::geom::planet_model::PlanetModel;
use rucene_spatial_extras::composite::composite_spatial_strategy::CompositeSpatialStrategy;
use rucene_spatial_extras::prefix::prefix_tree_strategy::PrefixTreeStrategy;
use rucene_spatial_extras::prefix::recursive_prefix_tree_strategy::{
    RecursivePrefixTreeStrategy, RecursivePrefixTreeStrategyOps,
};
use rucene_spatial_extras::prefix::tree::geohash_prefix_tree::GeohashPrefixTree;
use rucene_spatial_extras::prefix::tree::quad_prefix_tree::QuadPrefixTree;
use rucene_spatial_extras::prefix::tree::s2_prefix_tree::S2PrefixTree;
use rucene_spatial_extras::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use rucene_spatial_extras::serialized::serialized_dv_strategy::SerializedDVStrategy;
use rucene_spatial_extras::spatial4j::geo3d_point_shape::Geo3dPointShape;
use rucene_spatial_extras::spatial4j::geo3d_shape::Geo3dShape;
use rucene_spatial_extras::spatial4j::geo3d_spatial_context_factory::Geo3dSpatialContextFactory;
use rucene_spatial_test_fixtures::random_geo3d_shape_generator::RandomGeo3dShapeGenerator;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use spatial4j::context::spatial_context_factory::SpatialContextFactoryOps;
use spatial4j::distance::distance_utils::DistanceUtils;
use spatial4j::shape::shape::Shape;

use crate::prefix::random_spatial_op_strategy_test_case::RandomSpatialOpStrategyTestCaseOps;
use crate::spatial::spatial_test_case::{
    random_spatial_op_strategy_test_case_class, spatial_class, ti, SpatialCase, SpatialFixture,
};
use crate::spatial::strategy_test_case::StrategyTestCaseOps;
use crate::spatial_support::random;

type G = RandomGeo3dShapeGenerator;

const NAME: &str = "org.apache.lucene.spatial.spatial4j.TestGeo3dRpt";
/// `getClass().getSimpleName()`.
const SIMPLE_NAME: &str = "TestGeo3dRpt";

/// `DEGREES_TO_RADIANS` (static import of `DistanceUtils.DEGREES_TO_RADIANS`).
const DEGREES_TO_RADIANS: f64 = DistanceUtils::DEGREES_TO_RADIANS;

/// `TestGeo3dRpt`.
pub fn class_def() -> Arc<Class> {
    type T = TestGeo3dRpt;
    spatial_class::<T>(
        NAME,
        &random_spatial_op_strategy_test_case_class(),
        Constructor::no_args(|| TestGeo3dRpt {
            fx: SpatialFixture::new(NAME),
            planet_model: None,
            grid: None,
            rpt_strategy: None,
        }),
        vec![
            ti::<T>("testFailure1", T::test_failure1).annotate(Test::new()),
            ti::<T>("testFailureLucene6535", T::test_failure_lucene6535).annotate(Test::new()),
            ti::<T>("testOperations", T::test_operations).annotate(Test::new()),
            ti::<T>("testOperationsFromFile", T::test_operations_from_file).annotate(Test::new()),
        ],
    )
}

/// The `TestGeo3dRpt` instance.
pub struct TestGeo3dRpt {
    fx: SpatialFixture,
    /// `planetModel`.
    planet_model: Option<Arc<PlanetModel>>,
    /// `grid`.
    grid: Option<Arc<dyn SpatialPrefixTree>>,
    /// `rptStrategy`.
    rpt_strategy: Option<Arc<RecursivePrefixTreeStrategy>>,
}

impl SpatialCase for TestGeo3dRpt {
    fn fx(&self) -> &SpatialFixture {
        &self.fx
    }

    fn fx_mut(&mut self) -> &mut SpatialFixture {
        &mut self.fx
    }

    /// `randomIndexedShape()`.
    fn random_indexed_shape(&mut self) -> Result<Arc<dyn Shape>> {
        let shape_type = G::random_shape_type();
        let area_shape = G::random_geo_area_shape(shape_type, self.planet_model())?;
        if let Some(point_shape) = Arc::clone(&area_shape).into_geo_point_shape() {
            return Ok(Geo3dPointShape::new(point_shape, self.fx.ctx()));
        }
        Ok(Arc::new(Geo3dShape::new(area_shape, self.fx.ctx())))
    }

    /// `randomQueryShape()`.
    fn random_query_shape(&mut self) -> Result<Arc<dyn Shape>> {
        let shape_type = G::random_shape_type();
        let area_shape = G::random_geo_area_shape(shape_type, self.planet_model())?;
        Ok(Arc::new(Geo3dShape::new(area_shape, self.fx.ctx())))
    }
}

impl TestGeo3dRpt {
    fn planet_model(&self) -> &Arc<PlanetModel> {
        self.planet_model
            .as_ref()
            .expect("setupStrategy() sets planetModel")
    }

    /// `setupGrid()`.
    fn setup_grid(&mut self) -> Result<()> {
        let ctx = self.fx.ctx();
        let shape_type = random().next_int_bound(4);
        self.grid = Some(if shape_type == 0 {
            GeohashPrefixTree::new(ctx, 2)?
        } else if shape_type == 1 {
            QuadPrefixTree::new(ctx, 5)
        } else {
            let arity = random().next_int_bound(3) + 1;
            S2PrefixTree::with_arity(ctx, 5 - arity, arity)?
        });
        let rpt = self.new_rpt()?;
        rpt.set_prune_leafy_branches(random().next_boolean());
        self.rpt_strategy = Some(rpt);
        Ok(())
    }

    /// `newRPT()`.
    fn new_rpt(&self) -> Result<Arc<RecursivePrefixTreeStrategy>> {
        let rpt = RecursivePrefixTreeStrategy::new(
            Arc::clone(self.grid.as_ref().expect("grid")),
            &format!("{SIMPLE_NAME}_rpt"),
        )?;
        rpt.set_dist_err_pct(0.10); // not too many cells
        Ok(rpt)
    }

    /// `setupStrategy()`.
    fn setup_strategy(&mut self) -> Result<()> {
        let planet_model = G::random_planet_model();
        self.planet_model = Some(Arc::clone(&planet_model));
        let mut factory = Geo3dSpatialContextFactory::new();
        factory.planet_model = Some(planet_model);
        self.fx.ctx = Some(factory.new_spatial_context()?);

        self.setup_grid()?;

        let serialized_dv_strategy = Arc::new(SerializedDVStrategy::new(
            self.fx.ctx(),
            &format!("{SIMPLE_NAME}_sdv"),
        )?);
        self.fx.strategy = Some(Arc::new(CompositeSpatialStrategy::new(
            &format!("composite_{SIMPLE_NAME}"),
            self.rpt_strategy.clone().expect("rptStrategy"),
            serialized_dv_strategy,
        )?));
        Ok(())
    }

    /// The triangle of `testFailure1` and `testFailureLucene6535`.
    fn triangle_points(&self) -> Result<Vec<GeoPoint>> {
        let pm = self.planet_model();
        Ok(vec![
            GeoPoint::new_lat_lon(pm, 18.0 * DEGREES_TO_RADIANS, -27.0 * DEGREES_TO_RADIANS)?,
            GeoPoint::new_lat_lon(pm, -57.0 * DEGREES_TO_RADIANS, 146.0 * DEGREES_TO_RADIANS)?,
            GeoPoint::new_lat_lon(pm, 14.0 * DEGREES_TO_RADIANS, -180.0 * DEGREES_TO_RADIANS)?,
            GeoPoint::new_lat_lon(pm, -15.0 * DEGREES_TO_RADIANS, 153.0 * DEGREES_TO_RADIANS)?,
        ])
    }

    /// `testFailure1()`.
    fn test_failure1(&mut self) -> Result<()> {
        self.setup_strategy()?;
        let points = self.triangle_points()?;

        let polygon: Arc<dyn GeoAreaShape> =
            GeoPolygonFactory::make_geo_polygon(self.planet_model(), points)?
                .expect("java.lang.NullPointerException: makeGeoPolygon returned null");
        let triangle: Arc<dyn Shape> = Arc::new(Geo3dShape::new(polygon, self.fx.ctx()));
        let rect: Arc<dyn Shape> = self.fx.ctx().make_rectangle(-49.0, -45.0, 73.0, 86.0)?;
        self.test_operation(rect, SpatialOperation::Intersects, triangle, false)
    }

    /// `testFailureLucene6535()`.
    fn test_failure_lucene6535(&mut self) -> Result<()> {
        self.setup_strategy()?;

        let _points = self.triangle_points()?;
        let pm = Arc::clone(self.planet_model());
        let path_points = [
            GeoPoint::new_lat_lon(&pm, 55.0 * DEGREES_TO_RADIANS, -26.0 * DEGREES_TO_RADIANS)?,
            GeoPoint::new_lat_lon(&pm, -90.0 * DEGREES_TO_RADIANS, 0.0)?,
            GeoPoint::new_lat_lon(&pm, 54.0 * DEGREES_TO_RADIANS, 165.0 * DEGREES_TO_RADIANS)?,
            GeoPoint::new_lat_lon(&pm, -90.0 * DEGREES_TO_RADIANS, 0.0)?,
        ];
        let path: Arc<dyn GeoAreaShape> =
            GeoPathFactory::make_geo_path(&pm, 29.0 * DEGREES_TO_RADIANS, &path_points)?;
        let shape: Arc<dyn Shape> = Arc::new(Geo3dShape::new(path, self.fx.ctx()));
        let rect: Arc<dyn Shape> = self.fx.ctx().make_rectangle(131.0, 143.0, 39.0, 54.0)?;
        self.test_operation(rect, SpatialOperation::Intersects, shape, true)
    }

    /// `testOperations()`.
    fn test_operations(&mut self) -> Result<()> {
        self.setup_strategy()?;

        self.test_operation_random_shapes(SpatialOperation::Intersects)
    }

    /// `testOperationsFromFile()`.
    fn test_operations_from_file(&mut self) -> Result<()> {
        self.setup_strategy()?;
        let indexed_spatial_data = self.get_sample_data("states-poly.txt")?;
        let mut indexed_shapes: Vec<Option<Arc<dyn Shape>>> = Vec::new();
        for data in indexed_spatial_data {
            indexed_shapes.push(Some(data.shape));
        }
        let query_spatial_data = self.get_sample_data("states-bbox.txt")?;
        let mut query_shapes: Vec<Arc<dyn Shape>> = Vec::new();
        for data in query_spatial_data {
            query_shapes.push(data.shape);
            if *TEST_NIGHTLY {
                query_shapes.push(self.random_query_shape()?);
            }
        }
        query_shapes.push(self.random_query_shape()?);
        let havoc = random().next_boolean();
        self.test_operation_shapes(
            SpatialOperation::Intersects,
            indexed_shapes,
            query_shapes,
            havoc,
        )
    }
}
