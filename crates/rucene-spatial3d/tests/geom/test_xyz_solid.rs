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

//! Port of `TestXYZSolid.java`.

use crate::spatial3d_support::prelude::*;

/// `TestXYZSolid`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial3d.geom.TestXYZSolid",
        vec![
            test(
                "testNonDegenerateRelationships",
                test_non_degenerate_relationships,
            )
            .annotate(Test::new()),
            test("testDegenerateRelationships", test_degenerate_relationships)
                .annotate(Test::new()),
            test("testLUCENE8457", test_lucene8457).annotate(Test::new()),
        ],
    )
}

fn test_non_degenerate_relationships() -> Result<()> {
    let mut s: Arc<dyn XYZSolid>;
    let mut shape: Arc<dyn GeoShape>;
    // Something bigger than the world
    s = Arc::new(StandardXYZSolid::new(
        &SPHERE, -2.0, 2.0, -2.0, 2.0, -2.0, 2.0,
    )?);
    // Any shape, except whole world, should be within.
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, 0.0, 0.1)?);
    Assert::assert_equals(WITHIN, s.get_relationship(&*shape)?);
    shape = Arc::new(GeoWorld::new(&SPHERE));
    // An XYZSolid represents a surface shape, which when larger than the world is in fact
    // the entire world, so it should overlap the world.
    Assert::assert_equals(OVERLAPS, s.get_relationship(&*shape)?);
    // Something overlapping the world on only one side
    s = Arc::new(StandardXYZSolid::new(
        &SPHERE, -2.0, 0.0, -2.0, 2.0, -2.0, 2.0,
    )?);
    // Some things should be disjoint...
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, 0.0, 0.1)?);
    Assert::assert_equals(DISJOINT, s.get_relationship(&*shape)?);
    // And, some things should be within...
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, PI, 0.1)?);
    Assert::assert_equals(WITHIN, s.get_relationship(&*shape)?);
    // And, some things should overlap.
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, PI * 0.5, 0.1)?);
    Assert::assert_equals(OVERLAPS, s.get_relationship(&*shape)?);
    // Partial world should be contained by GeoWorld object...
    shape = Arc::new(GeoWorld::new(&SPHERE));
    Assert::assert_equals(CONTAINS, s.get_relationship(&*shape)?);
    // Something inside the world
    s = Arc::new(StandardXYZSolid::new(
        &SPHERE, -0.1, 0.1, -0.1, 0.1, -0.1, 0.1,
    )?);
    // All shapes should be disjoint
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, 0.0, 0.1)?);
    Assert::assert_equals(DISJOINT, s.get_relationship(&*shape)?);
    shape = Arc::new(GeoWorld::new(&SPHERE));
    Assert::assert_equals(DISJOINT, s.get_relationship(&*shape)?);
    Ok(())
}

fn test_degenerate_relationships() -> Result<()> {
    let mut solid: Arc<dyn GeoArea>;
    let mut shape: Arc<dyn GeoShape>;
    // Basic test of the factory method - non-degenerate
    solid = GeoAreaFactory::make_geo_area_xyz(&SPHERE, -2.0, 2.0, -2.0, 2.0, -2.0, 2.0)?;
    // Any shape, except whole world, should be within.
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, 0.0, 0.1)?);
    Assert::assert_equals(WITHIN, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoWorld::new(&SPHERE));
    // An XYZSolid represents a surface shape, which when larger than the world is in fact
    // the entire world, so it should overlap the world.
    Assert::assert_equals(OVERLAPS, solid.get_relationship(&*shape)?);
    // Build a degenerate point, not on sphere
    solid = GeoAreaFactory::make_geo_area_xyz(&SPHERE, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)?;
    // disjoint with everything?
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, 0.0, 0.1)?);
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoWorld::new(&SPHERE));
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    // Build a degenerate point that IS on the sphere
    solid = GeoAreaFactory::make_geo_area_xyz(&SPHERE, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0)?;
    // inside everything that it touches?
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, 0.0, 0.1)?);
    Assert::assert_equals(CONTAINS, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoWorld::new(&SPHERE));
    Assert::assert_equals(CONTAINS, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, PI, 0.1)?);
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    // Build a shape degenerate in (x,y), which has no points on sphere
    solid = GeoAreaFactory::make_geo_area_xyz(&SPHERE, 0.0, 0.0, 0.0, 0.0, -0.1, 0.1)?;
    // disjoint with everything?
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, 0.0, 0.1)?);
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoWorld::new(&SPHERE));
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    // Build a shape degenerate in (x,y) which has one point on sphere
    solid = GeoAreaFactory::make_geo_area_xyz(&SPHERE, 0.0, 0.0, 0.0, 0.0, -0.1, 1.1)?;
    // inside everything that it touches?
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, 0.0, 0.1)?);
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoWorld::new(&SPHERE));
    Assert::assert_equals(CONTAINS, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, PI * 0.5, 0.0, 0.1)?);
    Assert::assert_equals(CONTAINS, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, (-PI) * 0.5, 0.0, 0.1)?);
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    // Build a shape degenerate in (x,y) which has two points on sphere
    solid = GeoAreaFactory::make_geo_area_xyz(&SPHERE, 0.0, 0.0, 0.0, 0.0, -1.1, 1.1)?;
    // inside everything that it touches?
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, 0.0, 0.1)?);
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoWorld::new(&SPHERE));
    Assert::assert_equals(CONTAINS, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, PI * 0.5, 0.0, 0.1)?);
    Assert::assert_equals(OVERLAPS, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, (-PI) * 0.5, 0.0, 0.1)?);
    Assert::assert_equals(OVERLAPS, solid.get_relationship(&*shape)?);
    // Build a shape degenerate in (x,z), which has no points on sphere
    solid = GeoAreaFactory::make_geo_area_xyz(&SPHERE, 0.0, 0.0, -0.1, 0.1, 0.0, 0.0)?;
    // disjoint with everything?
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, 0.0, 0.1)?);
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoWorld::new(&SPHERE));
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    // Build a shape degenerate in (x,z) which has one point on sphere
    solid = GeoAreaFactory::make_geo_area_xyz(&SPHERE, 0.0, 0.0, -0.1, 1.1, 0.0, 0.0)?;
    // inside everything that it touches?
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, 0.0, 0.1)?);
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoWorld::new(&SPHERE));
    Assert::assert_equals(CONTAINS, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, PI * 0.5, 0.1)?);
    Assert::assert_equals(CONTAINS, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, (-PI) * 0.5, 0.1)?);
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    // Build a shape degenerate in (x,y) which has two points on sphere
    solid = GeoAreaFactory::make_geo_area_xyz(&SPHERE, 0.0, 0.0, -1.1, 1.1, 0.0, 0.0)?;
    // inside everything that it touches?
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, 0.0, 0.1)?);
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoWorld::new(&SPHERE));
    Assert::assert_equals(CONTAINS, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, PI * 0.5, 0.1)?);
    Assert::assert_equals(OVERLAPS, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, (-PI) * 0.5, 0.1)?);
    Assert::assert_equals(OVERLAPS, solid.get_relationship(&*shape)?);
    // MHL for y-z check
    // Build a shape that is degenerate in x, which has zero points intersecting sphere
    solid = GeoAreaFactory::make_geo_area_xyz(&SPHERE, 0.0, 0.0, -0.1, 0.1, -0.1, 0.1)?;
    // disjoint with everything?
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, 0.0, 0.1)?);
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoWorld::new(&SPHERE));
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    // Build a shape that is degenerate in x, which has zero points intersecting sphere, second
    // variation
    solid = GeoAreaFactory::make_geo_area_xyz(&SPHERE, 0.0, 0.0, -0.1, 0.1, 1.1, 1.2)?;
    // disjoint with everything?
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, 0.0, 0.1)?);
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoWorld::new(&SPHERE));
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    // Build a shape that is disjoint in X but intersects sphere in a complete circle
    solid = GeoAreaFactory::make_geo_area_xyz(&SPHERE, 0.0, 0.0, -1.1, 1.1, -1.1, 1.1)?;
    // inside everything that it touches?
    shape = Arc::new(GeoWorld::new(&SPHERE));
    Assert::assert_equals(CONTAINS, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, 0.0, 0.1)?);
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, PI, 0.1)?);
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, PI * 0.5, 0.1)?);
    Assert::assert_equals(OVERLAPS, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, (-PI) * 0.5, 0.1)?);
    Assert::assert_equals(OVERLAPS, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, PI * 0.5, 0.0, 0.1)?);
    Assert::assert_equals(OVERLAPS, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, (-PI) * 0.5, 0.0, 0.1)?);
    Assert::assert_equals(OVERLAPS, solid.get_relationship(&*shape)?);
    // Build a shape that is disjoint in X but intersects sphere in a half circle in Y
    solid = GeoAreaFactory::make_geo_area_xyz(&SPHERE, 0.0, 0.0, 0.0, 1.1, -1.1, 1.1)?;
    // inside everything that it touches?
    shape = Arc::new(GeoWorld::new(&SPHERE));
    Assert::assert_equals(CONTAINS, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, 0.0, 0.1)?);
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, PI, 0.1)?);
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, PI * 0.5, 0.1)?);
    Assert::assert_equals(OVERLAPS, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, 0.0, (-PI) * 0.5, 0.1)?);
    Assert::assert_equals(DISJOINT, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, PI * 0.5, 0.0, 0.1)?);
    Assert::assert_equals(OVERLAPS, solid.get_relationship(&*shape)?);
    shape = Arc::new(GeoStandardCircle::new(&SPHERE, (-PI) * 0.5, 0.0, 0.1)?);
    Assert::assert_equals(OVERLAPS, solid.get_relationship(&*shape)?);
    // MHL for degenerate Y
    // MHL for degenerate Z
    Ok(())
}

fn test_lucene8457() -> Result<()> {
    let shape: Arc<dyn GeoShape> =
        GeoBBoxFactory::make_geo_bbox(&WGS84, PI, 1.2487354264870392, 0.0, 3.5181789305199657E-12)?;
    // System.out.println("shape = "+shape);
    let mut bounds = XYZBounds::new();
    shape.get_bounds(&mut bounds);
    let solid: Arc<dyn XYZSolid> = XYZSolidFactory::make_xyz_solid_from_bounds(&WGS84, &bounds)?;
    // System.out.println("solid = "+solid);
    let point = GeoPoint::new_lat_lon(&WGS84, 1.4812439919751819, -3.141592653589793)?;
    // System.out.println("point="+point);
    // if the point is within the shape, it must be within the solid
    if shape.is_within(&point) {
        Assert::assert_true(solid.is_within(&point));
    }
    Ok(())
}
