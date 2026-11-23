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

//! Port of `TestPointVectorStrategy.java` (package
//! `org.apache.lucene.spatial.vector`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::{Assert, Test};
use rucene::document::{FieldType, NumericValue};
use rucene::error::Result;
use rucene::search::{MatchAllDocsQuery, Query};
use rucene_spatial_extras::query::spatial_args::SpatialArgs;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use rucene_spatial_extras::vector::point_vector_strategy::{
    PointVectorStrategy, DEFAULT_FIELDTYPE,
};
use rucene_test_framework::util::java_exceptions::expect_throws;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::shape::shape::Shape;

use crate::spatial::spatial_match_concern::SpatialMatchConcern;
use crate::spatial::spatial_test_case::{
    spatial_class, spatial_set_up, strategy_test_case_class, ti, SpatialCase, SpatialFixture,
    SpatialTestCaseOps,
};
use crate::spatial::strategy_test_case::{
    StrategyTestCaseOps, DATA_WORLD_CITIES_POINTS, QTEST_CITIES_INTERSECTS_BBOX,
};

const NAME: &str = "org.apache.lucene.spatial.vector.TestPointVectorStrategy";
/// `getClass().getSimpleName()`.
const SIMPLE_NAME: &str = "TestPointVectorStrategy";

/// `TestPointVectorStrategy`.
pub fn class_def() -> Arc<Class> {
    type T = TestPointVectorStrategy;
    spatial_class::<T>(
        NAME,
        &strategy_test_case_class(),
        Constructor::no_args(|| TestPointVectorStrategy {
            fx: SpatialFixture::new(NAME),
        }),
        vec![
            ti::<T>("testCircleShapeSupport", T::test_circle_shape_support).annotate(Test::new()),
            ti::<T>("testInvalidQueryShape", T::test_invalid_query_shape).annotate(Test::new()),
            ti::<T>("testCitiesIntersectsBBox", T::test_cities_intersects_bbox)
                .annotate(Test::new()),
            ti::<T>("testFieldOptions", T::test_field_options).annotate(Test::new()),
        ],
    )
}

/// The `TestPointVectorStrategy` instance.
pub struct TestPointVectorStrategy {
    fx: SpatialFixture,
}

impl SpatialCase for TestPointVectorStrategy {
    fn fx(&self) -> &SpatialFixture {
        &self.fx
    }

    fn fx_mut(&mut self) -> &mut SpatialFixture {
        &mut self.fx
    }

    /// `setUp()`.
    fn set_up(&mut self) -> Result<()> {
        spatial_set_up(&mut self.fx)?;
        self.fx.ctx = Some(SpatialContext::geo());
        Ok(())
    }
}

impl TestPointVectorStrategy {
    /// `testCircleShapeSupport()`.
    fn test_circle_shape_support(&mut self) -> Result<()> {
        let ctx = self.fx.ctx();
        self.fx.strategy = Some(PointVectorStrategy::new_instance(
            Arc::clone(&ctx),
            SIMPLE_NAME,
        )?);
        let circle: Arc<dyn Shape> = ctx.make_circle_at(ctx.make_point(0.0, 0.0)?, 10.0)?;
        let args = SpatialArgs::new(SpatialOperation::Intersects, circle);
        let _query = self.fx.strategy().make_query(&args)?;

        // `assertNotNull(query)`: a query is never null
        Ok(())
    }

    /// `testInvalidQueryShape()`.
    fn test_invalid_query_shape(&mut self) -> Result<()> {
        let ctx = self.fx.ctx();
        self.fx.strategy = Some(PointVectorStrategy::new_instance(
            Arc::clone(&ctx),
            SIMPLE_NAME,
        )?);
        let point: Arc<dyn Shape> = ctx.make_point(0.0, 0.0)?;
        let args = SpatialArgs::new(SpatialOperation::Intersects, point);
        expect_throws("java.lang.UnsupportedOperationException", || {
            self.fx.strategy().make_query(&args)
        });
        Ok(())
    }

    /// `testCitiesIntersectsBBox()`.
    fn test_cities_intersects_bbox(&mut self) -> Result<()> {
        // note: does not require docValues
        self.fx.strategy = Some(PointVectorStrategy::new_instance(
            self.fx.ctx(),
            SIMPLE_NAME,
        )?);
        self.get_add_and_verify_indexed_documents(DATA_WORLD_CITIES_POINTS)?;
        self.execute_queries(SpatialMatchConcern::FILTER, &[QTEST_CITIES_INTERSECTS_BBOX])
    }

    /// `testFieldOptions()`.
    fn test_field_options(&mut self) -> Result<()> {
        // It's not stored; test it isn't.
        self.fx.strategy = Some(PointVectorStrategy::new_instance(
            self.fx.ctx(),
            SIMPLE_NAME,
        )?);
        self.adoc_wkt("99", Some("POINT(-5.0 8.2)"))?;
        self.commit()?;
        let mut results =
            self.execute_query(Arc::new(MatchAllDocsQuery::instance()) as Arc<dyn Query>, 1)?;
        let field_name = self.fx.strategy().get_field_name().to_string();
        {
            let document = &results.results[0].document;
            Assert::assert_true_msg(
                "not stored",
                document
                    .get_field(&format!("{field_name}{}", PointVectorStrategy::SUFFIX_X))
                    .is_none(),
            );
            Assert::assert_true_msg(
                "not stored",
                document
                    .get_field(&format!("{field_name}{}", PointVectorStrategy::SUFFIX_Y))
                    .is_none(),
            );
        }
        self.delete_all()?;

        // Now we mark it stored.  We also disable pointvalues...
        let field_type = FieldType::new_from(&*DEFAULT_FIELDTYPE);
        field_type.set_stored(true)?;
        field_type.set_dimensions(0, 0)?; // disable point values
        self.fx.strategy = Some(PointVectorStrategy::new(
            self.fx.ctx(),
            SIMPLE_NAME,
            &field_type,
        )?);
        self.adoc_wkt("99", Some("POINT(-5.0 8.2)"))?;
        self.commit()?;
        results =
            self.execute_query(Arc::new(MatchAllDocsQuery::instance()) as Arc<dyn Query>, 1)?;
        let document = &results.results[0].document;
        let field_name = self.fx.strategy().get_field_name().to_string();
        let x = document
            .get_field(&format!("{field_name}{}", PointVectorStrategy::SUFFIX_X))
            .and_then(|f| f.numeric_value());
        assert_equals_number("stored", -5.0, x);
        let y = document
            .get_field(&format!("{field_name}{}", PointVectorStrategy::SUFFIX_Y))
            .and_then(|f| f.numeric_value());
        assert_equals_number("stored", 8.2, y);

        // Test a query fails without point values
        expect_throws("java.lang.UnsupportedOperationException", || {
            let args = SpatialArgs::new(
                SpatialOperation::Intersects,
                self.fx.ctx().make_rectangle(-10.0, 10.0, -5.0, 5.0)?,
            );
            self.fx.strategy().make_query(&args)
        });
        Ok(())
    }
}

/// `assertEquals(String message, Object expected, Object actual)` of a
/// boxed `Double` against a field's `Number` (`Double.equals`: a `Double`
/// with the same bits).
fn assert_equals_number(message: &str, expected: f64, actual: Option<NumericValue>) {
    let same = matches!(actual, Some(NumericValue::Double(d)) if d.to_bits() == expected.to_bits());
    if !same {
        let actual = match actual {
            None => "null".to_string(),
            Some(NumericValue::Int(i)) => i.to_string(),
            Some(NumericValue::Long(l)) => l.to_string(),
            Some(NumericValue::Float(f)) => rucene::util::java_fmt::float_to_string(f),
            Some(NumericValue::Double(d)) => rucene::util::java_fmt::double_to_string(d),
        };
        Assert::fail(&format!(
            "{message} expected:<{}> but was:<{actual}>",
            rucene::util::java_fmt::double_to_string(expected)
        ));
    }
}
