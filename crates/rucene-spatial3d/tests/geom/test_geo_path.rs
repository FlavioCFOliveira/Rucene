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

//! Port of `TestGeoPath.java`.

use crate::spatial3d_support::prelude::*;

/// `TestGeoPath`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial3d.geom.TestGeoPath",
        vec![
            test("testPathDistance", test_path_distance).annotate(Test::new()),
            test("test11956", test11956).annotate(Test::new()),
            test("testPathPointWithin", test_path_point_within).annotate(Test::new()),
            test("testGetRelationship", test_get_relationship).annotate(Test::new()),
            test("testPathBounds", test_path_bounds).annotate(Test::new()),
            test("testCoLinear", test_co_linear).annotate(Test::new()),
            test("testFailure1", test_failure1).annotate(Test::new()),
            test("testInterpolation", test_interpolation).annotate(Test::new()),
            test("testInterpolation2", test_interpolation2).annotate(Test::new()),
            test("testIdenticalPoints", test_identical_points).annotate(Test::new()),
            test("testLUCENE8696", test_lucene8696).annotate(Test::new()),
        ],
    )
}

fn test_path_distance() -> Result<()> {
    // Start with a really simple case
    let mut p: GeoStandardPath;
    let mut gp: GeoPoint;
    p = GeoStandardPath::new_empty(&SPHERE, 0.1)?;
    p.add_point(0.0, 0.0)?;
    p.add_point(0.0, 0.1)?;
    p.add_point(0.0, 0.2)?;
    p.done()?;
    gp = GeoPoint::new_lat_lon(&SPHERE, PI * 0.5, 0.15)?;
    Assert::assert_equals_double_msg(None, f64::INFINITY, p.compute_distance(&ARC, &gp)?, 0.0);
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.05, 0.15)?;
    Assert::assert_equals_double_msg(None, 0.15 + 0.05, p.compute_distance(&ARC, &gp)?, 0.000001);
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, 0.12)?;
    Assert::assert_equals_double_msg(None, 0.12 + 0.0, p.compute_distance(&ARC, &gp)?, 0.000001);
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.15, 0.05)?;
    Assert::assert_equals_double_msg(
        None,
        f64::INFINITY,
        p.compute_distance(&ARC, &gp)?,
        0.000001,
    );
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, 0.25)?;
    Assert::assert_equals_double_msg(None, 0.20 + 0.05, p.compute_distance(&ARC, &gp)?, 0.000001);
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, -0.05)?;
    Assert::assert_equals_double_msg(None, 0.0 + 0.05, p.compute_distance(&ARC, &gp)?, 0.000001);
    // Compute path distances now
    p = GeoStandardPath::new_empty(&SPHERE, 0.1)?;
    p.add_point(0.0, 0.0)?;
    p.add_point(0.0, 0.1)?;
    p.add_point(0.0, 0.2)?;
    p.done()?;
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.05, 0.15)?;
    Assert::assert_equals_double_msg(None, 0.15 + 0.05, p.compute_distance(&ARC, &gp)?, 0.000001);
    Assert::assert_equals_double_msg(None, 0.15, p.compute_nearest_distance(&ARC, &gp)?, 0.000001);
    Assert::assert_equals_double_msg(None, 0.10, p.compute_delta_distance(&ARC, &gp)?, 0.000001);
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, 0.12)?;
    Assert::assert_equals_double_msg(None, 0.12, p.compute_distance(&ARC, &gp)?, 0.000001);
    Assert::assert_equals_double_msg(None, 0.12, p.compute_nearest_distance(&ARC, &gp)?, 0.000001);
    Assert::assert_equals_double_msg(None, 0.0, p.compute_delta_distance(&ARC, &gp)?, 0.000001);
    // Now try a vertical path, and make sure distances are as expected
    p = GeoStandardPath::new_empty(&SPHERE, 0.1)?;
    p.add_point((-PI) * 0.25, -0.5)?;
    p.add_point(PI * 0.25, -0.5)?;
    p.done()?;
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, 0.0)?;
    Assert::assert_equals_double_msg(None, f64::INFINITY, p.compute_distance(&ARC, &gp)?, 0.0);
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.1, -1.0)?;
    Assert::assert_equals_double_msg(None, f64::INFINITY, p.compute_distance(&ARC, &gp)?, 0.0);
    gp = GeoPoint::new_lat_lon(&SPHERE, PI * 0.25 + 0.05, -0.5)?;
    Assert::assert_equals_double_msg(
        None,
        PI * 0.5 + 0.05,
        p.compute_distance(&ARC, &gp)?,
        0.000001,
    );
    gp = GeoPoint::new_lat_lon(&SPHERE, (-PI) * 0.25 - 0.05, -0.5)?;
    Assert::assert_equals_double_msg(None, 0.0 + 0.05, p.compute_distance(&ARC, &gp)?, 0.000001);
    Ok(())
}

fn test11956() -> Result<()> {
    // Geo3D:GeoStandardPath: {planetmodel=PlanetModel.SPHERE, width=1.1344640137963142(65.0),
    //  points={[[lat=-1.289777264488089, lon=3.0020962766211765([X=-0.2746408902222561,
    // Y=0.0385618624109501, Z=-0.96077331571257])],
    // [lat=-1.50113114141284, lon=2.545709547022838([X=-0.057611985525967656,
    // Y=0.03906726187412952, Z=-0.997574362227405])],
    // [lat=1.079898704051346, lon=1.7302019835278628([X=-0.07482880413595766,
    // Y=0.46544097200827866, Z=0.8819100587064257])],
    // [lat=0.4651998030659944, lon=-1.731044309953635([X=-0.14260656697812318,
    // Y=-0.8822812296622808, Z=0.44860138078290357])],
    // [lat=-0.058395560871481914, lon=-1.467184843697817([X=0.10324990479626492,
    // Y=-0.9929417354486986, Z=-0.058362377981787186])]]}}
    // intersect Geo3D:GeoDegeneratePoint:
    //   {planetmodel=PlanetModel.SPHERE, lat=0.7332272528281016(42.01082701102197),
    // lon=0.7287424582438785(41.753867209362866)}
    // 1> XYZBounds of PathNode inconsistent with isWithin of children!
    // XYZBounds=XYZBounds: [xmin=-0.9626183326283182 xmax=0.8721428398024924
    // ymin=-0.8870855520255307 ymax=0.47448488489820667 zmin=0.06959538905950932 zmax=1.000000001]
    //  child1=CutoffDualCircleSegmentEndpoint: SegmentEndpoint ([lat=1.079898704051346,
    // lon=1.7302019835278628([X=-0.07482880413595766, Y=0.46544097200827866,
    // Z=0.8819100587064257])])
    //  child2=PathSegment ([X=0.8628106851303438, Y=0.11314342359669693, Z=0.4927030417216086],
    // [X=0.8341665648133144, Y=-0.4564285905826633, Z=0.30957888146040907], [X=-0.9547028437115205,
    // Y=-0.2893077287099765, Z=0.06959539006148753], [X=-0.9260587233944911, Y=0.28026428546938364,
    // Z=0.25271955032268706])
    //  Point=[0.5543009381999603, 0.49479972312729714, 0.6692710242523532]
    // 1> Child1: Bounds=XYZBounds: [xmin=-0.9260587243944911 xmax=0.8721428398024924
    // ymin=-0.024137541083565 ymax=0.4654409730082787 zmin=0.25271954932268703
    // zmax=0.8819100597064257] isWithin=true
    // 1> Child2: Bounds=XYZBounds: [xmin=-0.9626183326283182 xmax=0.862810686131634
    // ymin=-0.8870855520255307 ymax=0.47448488489820667 zmin=0.06959538905950932 zmax=1.000000001]
    // isWithin=false
    let mut p: GeoStandardPath;
    p = GeoStandardPath::new_empty(&SPHERE, 1.1344640137963142)?;
    p.add_point(-1.289777264488089, 3.0020962766211765)?;
    p.add_point(-1.50113114141284, 2.545709547022838)?;
    p.add_point(1.079898704051346, 1.7302019835278628)?;
    p.add_point(0.4651998030659944, -1.731044309953635)?;
    p.add_point(-0.058395560871481914, -1.467184843697817)?;
    p.done()?;
    let gp = GeoPoint::new(0.5543009381999603, 0.49479972312729714, 0.6692710242523532);
    Assert::assert_true(SPHERE.point_on_surface(&gp));
    Assert::assert_true(p.is_within(&gp));
    // XYZBounds bounds = new XYZBounds();
    // p.getBounds(bounds);
    // assertTrue(bounds.isWithin(gp));
    Ok(())
}

fn test_path_point_within() -> Result<()> {
    // Tests whether we can properly detect whether a point is within a path or not
    let mut p: GeoStandardPath;
    let mut gp: GeoPoint;
    p = GeoStandardPath::new_empty(&SPHERE, 0.1)?;
    // Build a diagonal path crossing the equator
    p.add_point(-0.2, -0.2)?;
    p.add_point(0.2, 0.2)?;
    p.done()?;
    // Test points on the path
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.2, -0.2)?;
    Assert::assert_true(p.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, 0.0)?;
    Assert::assert_true(p.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.1, 0.1)?;
    Assert::assert_true(p.is_within(&gp));
    // Test points off the path
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.2, 0.2)?;
    Assert::assert_false(p.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, (-PI) * 0.5, 0.0)?;
    Assert::assert_false(p.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.2, -0.2)?;
    Assert::assert_false(p.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, PI)?;
    Assert::assert_false(p.is_within(&gp));
    // Repeat the test, but across the terminator
    p = GeoStandardPath::new_empty(&SPHERE, 0.1)?;
    // Build a diagonal path crossing the equator
    p.add_point(-0.2, PI - 0.2)?;
    p.add_point(0.2, (-PI) + 0.2)?;
    p.done()?;
    // Test points on the path
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.2, PI - 0.2)?;
    Assert::assert_true(p.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, PI)?;
    Assert::assert_true(p.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.1, (-PI) + 0.1)?;
    Assert::assert_true(p.is_within(&gp));
    // Test points off the path
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.2, (-PI) + 0.2)?;
    Assert::assert_false(p.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, (-PI) * 0.5, 0.0)?;
    Assert::assert_false(p.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.2, PI - 0.2)?;
    Assert::assert_false(p.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, 0.0)?;
    Assert::assert_false(p.is_within(&gp));
    Ok(())
}

fn test_get_relationship() -> Result<()> {
    let mut rect: Arc<dyn GeoArea>;
    let mut p: GeoStandardPath;
    let mut c: GeoStandardPath;
    let point: GeoPoint;
    let planet_model: Arc<PlanetModel>;
    planet_model = Arc::new(PlanetModel::new(1.151145876105594, 0.8488541238944061));
    c = GeoStandardPath::new_empty(&planet_model, 0.008726646259971648)?;
    c.add_point(-0.6925658899376476, 0.6316613927914589)?;
    c.add_point(0.27828548161836364, 0.6785795524104564)?;
    c.done()?;
    point = GeoPoint::new_lat_lon(&planet_model, -0.49298555067758226, 0.9892440995026406)?;
    GeoAreaFactory::make_geo_area_xyz(
        &planet_model,
        0.49937141144985997,
        0.5161765426256085,
        0.3337218719537796,
        0.8544419570901649,
        -0.6347692823688085,
        0.3069696588119369,
    )?;
    Assert::assert_true(!c.is_within(&point));
    // Start by testing the basic kinds of relationship, increasing in order of difficulty.
    p = GeoStandardPath::new_empty(&SPHERE, 0.1)?;
    p.add_point(-0.3, -0.3)?;
    p.add_point(0.3, 0.3)?;
    p.done()?;
    // Easiest: The path is wholly contains the georect
    rect = Arc::new(GeoRectangle::new(&SPHERE, 0.05, -0.05, -0.05, 0.05)?);
    Assert::assert_equals(CONTAINS, rect.get_relationship(&p)?);
    // Next easiest: Some endpoints of the rectangle are inside, and some are outside.
    rect = Arc::new(GeoRectangle::new(&SPHERE, 0.05, -0.05, -0.05, 0.5)?);
    Assert::assert_equals(OVERLAPS, rect.get_relationship(&p)?);
    // Now, all points are outside, but the figures intersect
    rect = Arc::new(GeoRectangle::new(&SPHERE, 0.05, -0.05, -0.5, 0.5)?);
    Assert::assert_equals(OVERLAPS, rect.get_relationship(&p)?);
    // Finally, all points are outside, and the figures *do not* intersect
    rect = Arc::new(GeoRectangle::new(&SPHERE, 0.5, -0.5, -0.5, 0.5)?);
    Assert::assert_equals(WITHIN, rect.get_relationship(&p)?);
    // Check that segment edge overlap detection works
    rect = Arc::new(GeoRectangle::new(&SPHERE, 0.1, 0.0, -0.1, 0.0)?);
    Assert::assert_equals(OVERLAPS, rect.get_relationship(&p)?);
    rect = Arc::new(GeoRectangle::new(&SPHERE, 0.2, 0.1, -0.2, -0.1)?);
    Assert::assert_equals(DISJOINT, rect.get_relationship(&p)?);
    // Check if overlap at endpoints behaves as expected next
    rect = Arc::new(GeoRectangle::new(&SPHERE, 0.5, -0.5, -0.5, -0.35)?);
    Assert::assert_equals(OVERLAPS, rect.get_relationship(&p)?);
    rect = Arc::new(GeoRectangle::new(&SPHERE, 0.5, -0.5, -0.5, -0.45)?);
    Assert::assert_equals(DISJOINT, rect.get_relationship(&p)?);
    Ok(())
}

fn test_path_bounds() -> Result<()> {
    let mut c: GeoStandardPath;
    let mut b: LatLonBounds;
    let mut xyzb: XYZBounds;
    let mut point: GeoPoint;
    let mut relationship: i32;
    let mut area: Arc<dyn GeoArea>;
    let planet_model: Arc<PlanetModel>;
    planet_model = Arc::new(PlanetModel::new(0.751521665790406, 1.248478334209594));
    c = GeoStandardPath::new_empty(&planet_model, 0.7504915783575618)?;
    c.add_point(0.10869761172400265, 0.08895880215465272)?;
    c.add_point(0.22467878641991612, 0.10972973084229565)?;
    c.add_point(-0.7398772468744732, -0.4465812941383364)?;
    c.add_point(-0.18462055300079366, -0.6713857796763727)?;
    c.done()?;
    point = GeoPoint::new_lat_lon(&planet_model, -0.626645355125733, -1.409304625439381)?;
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    area = GeoAreaFactory::make_geo_area_xyz(
        &planet_model,
        unbox(xyzb.get_minimum_x()),
        unbox(xyzb.get_maximum_x()),
        unbox(xyzb.get_minimum_y()),
        unbox(xyzb.get_maximum_y()),
        unbox(xyzb.get_minimum_z()),
        unbox(xyzb.get_maximum_z()),
    )?;
    relationship = area.get_relationship(&c)?;
    Assert::assert_true(relationship == WITHIN || relationship == OVERLAPS);
    Assert::assert_true(area.is_within(&point));
    // No longer true due to fixed GeoStandardPath waypoints.
    // assertTrue(zScaling.isWithin(point));
    c = GeoStandardPath::new_empty(&WGS84, 0.6894050545377601)?;
    c.add_point(-0.0788176065762948, 0.9431251741731624)?;
    c.add_point(0.510387871458147, 0.5327078872484678)?;
    c.add_point(-0.5624521609859962, 1.5398841746888388)?;
    c.add_point(-0.5025171434638661, -0.5895998642788894)?;
    c.done()?;
    point = GeoPoint::new_lat_lon(&WGS84, 0.023652082107211682, 0.023131910152748437)?;
    // System.err.println("Point.x = "+point.x+"; point.y="+point.y+"; point.z="+point.z);
    Assert::assert_true(c.is_within(&point));
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    area = GeoAreaFactory::make_geo_area_xyz(
        &WGS84,
        unbox(xyzb.get_minimum_x()),
        unbox(xyzb.get_maximum_x()),
        unbox(xyzb.get_minimum_y()),
        unbox(xyzb.get_maximum_y()),
        unbox(xyzb.get_minimum_z()),
        unbox(xyzb.get_maximum_z()),
    )?;
    // System.err.println("minx="+xyzb.getMinimumX()+" maxx="+xyzb.getMaximumX()+"
    // miny="+xyzb.getMinimumY()+" maxy="+xyzb.getMaximumY()+" minz="+xyzb.getMinimumZ()+"
    // maxz="+xyzb.getMaximumZ());
    // System.err.println("point.x="+point.x+" point.y="+point.y+" point.z="+point.z);
    relationship = area.get_relationship(&c)?;
    Assert::assert_true(relationship == WITHIN || relationship == OVERLAPS);
    Assert::assert_true(area.is_within(&point));
    c = GeoStandardPath::new_empty(&WGS84, 0.7766715171374766)?;
    c.add_point(-0.2751718361148076, -0.7786721269011477)?;
    c.add_point(0.5728375851539309, -1.2700115736820465)?;
    c.done()?;
    point = GeoPoint::new_lat_lon(&WGS84, -0.01580760332365284, -0.03956004622490505)?;
    Assert::assert_true(c.is_within(&point));
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    area = GeoAreaFactory::make_geo_area_xyz(
        &WGS84,
        unbox(xyzb.get_minimum_x()),
        unbox(xyzb.get_maximum_x()),
        unbox(xyzb.get_minimum_y()),
        unbox(xyzb.get_maximum_y()),
        unbox(xyzb.get_minimum_z()),
        unbox(xyzb.get_maximum_z()),
    )?;
    // System.err.println("minx="+xyzb.getMinimumX()+" maxx="+xyzb.getMaximumX()+"
    // miny="+xyzb.getMinimumY()+" maxy="+xyzb.getMaximumY()+" minz="+xyzb.getMinimumZ()+"
    // maxz="+xyzb.getMaximumZ());
    // System.err.println("point.x="+point.x+" point.y="+point.y+" point.z="+point.z);
    relationship = area.get_relationship(&c)?;
    Assert::assert_true(relationship == WITHIN || relationship == OVERLAPS);
    Assert::assert_true(area.is_within(&point));
    c = GeoStandardPath::new_empty(&SPHERE, 0.1)?;
    c.add_point(-0.3, -0.3)?;
    c.add_point(0.3, 0.3)?;
    c.done()?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, -0.4046919, unbox(b.get_left_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.4046919, unbox(b.get_right_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.3999999, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.3999999, unbox(b.get_max_latitude()), 0.000001);
    Ok(())
}

fn test_co_linear() -> Result<()> {
    // p1: (12,-90), p2: (11, -55), (129, -90)
    let mut p = GeoStandardPath::new_empty(&SPHERE, 0.1)?;
    p.add_point(to_radians(-90.0), to_radians(12.0))?;
    // south pole
    p.add_point(to_radians(-55.0), to_radians(11.0))?;
    p.add_point(to_radians(-90.0), to_radians(129.0))?;
    // south pole again
    p.done()?;
    // at least test this doesn't bomb like it used too -- LUCENE-6520
    Ok(())
}

fn test_failure1() -> Result<()> {
    /*
    GeoStandardPath: {planetmodel=PlanetModel.WGS84, width=1.117010721276371(64.0), points={[
    [lat=2.18531083006635E-12, lon=-3.141592653589793([X=-1.0011188539924791, Y=-1.226017000107956E-16, Z=2.187755873813378E-12])],
    [lat=0.0, lon=-3.141592653589793([X=-1.0011188539924791, Y=-1.226017000107956E-16, Z=0.0])]]}}
     */
    let points = vec![
        GeoPoint::new_lat_lon(&WGS84, 2.18531083006635E-12, -3.141592653589793)?,
        GeoPoint::new_lat_lon(&WGS84, 0.0, -3.141592653589793)?,
    ];
    let path: Arc<dyn GeoPath>;
    match GeoPathFactory::make_geo_path(&WGS84, 1.117010721276371, &points) {
        Ok(p) => path = p,
        Err(e) if e.is_illegal_argument_exception() => return Ok(()),
        Err(e) => return Err(e),
    }
    Assert::assert_true(false);
    let point = GeoPoint::new_lat_lon(&WGS84, -2.848117399637174E-91, -1.1092122135274942)?;
    randomizedtesting::jdk::system::err_println(&format!("point = {}", point));
    let mut bounds = XYZBounds::new();
    path.get_bounds(&mut bounds);
    let solid: Arc<dyn XYZSolid> = XYZSolidFactory::make_xyz_solid(
        &WGS84,
        unbox(bounds.get_minimum_x()),
        unbox(bounds.get_maximum_x()),
        unbox(bounds.get_minimum_y()),
        unbox(bounds.get_maximum_y()),
        unbox(bounds.get_minimum_z()),
        unbox(bounds.get_maximum_z()),
    )?;
    Assert::assert_true(path.is_within(&point));
    Assert::assert_true(solid.is_within(&point));
    Ok(())
}

fn test_interpolation() -> Result<()> {
    let lat = 52.51607;
    let lon = 13.37698;
    let path_lats = vec![
        52.5355, 52.54, 52.5626, 52.5665, 52.6007, 52.6135, 52.6303, 52.6651, 52.7074,
    ];
    let path_lons = vec![
        13.3634, 13.3704, 13.3307, 13.3076, 13.2806, 13.2484, 13.2406, 13.241, 13.1926,
    ];
    // Set up a point in the right way
    let car_point = GeoPoint::new_lat_lon(&SPHERE, to_radians(lat), to_radians(lon))?;
    // Create the path, but use a tiny width (e.g. zero)
    // `new GeoPoint[pathLats.length]`, every element assigned in order.
    let mut path_points: Vec<GeoPoint> = Vec::with_capacity(path_lats.len());
    for i in 0..(path_lats.len() as i32) {
        path_points.push(GeoPoint::new_lat_lon(
            &SPHERE,
            to_radians(path_lats[i as usize]),
            to_radians(path_lons[i as usize]),
        )?);
    }
    // Construct a path with no width
    let this_path: Arc<dyn GeoPath> = GeoPathFactory::make_geo_path(&SPHERE, 0.0, &path_points)?;
    // Construct a path with a width
    let legacy_path: Arc<dyn GeoPath> = GeoPathFactory::make_geo_path(&SPHERE, 1e-6, &path_points)?;
    // Compute the inside distance to the atPoint using zero-width path
    let distance = this_path.compute_nearest_distance(&ARC, &car_point)?;
    // Compute the inside distance using legacy path
    let legacy_distance = legacy_path.compute_nearest_distance(&ARC, &car_point)?;
    // Compute the inside distance using the legacy formula
    let old_formula_distance = this_path.compute_distance(&ARC, &car_point)?;
    // Compute the inside distance using the legacy formula with the legacy shape
    let old_formula_legacy_distance = legacy_path.compute_distance(&ARC, &car_point)?;
    // These should be about the same, but something is wrong with GeoDegeneratePath and they
    // aren't.
    // More research needed.
    randomizedtesting::jdk::system::out_println(&format!(
        "Zero width path nearestDistance = {}, standard path nearestDistance = {}",
        d2s(distance),
        d2s(legacy_distance)
    ));
    Assert::assert_equals_double_msg(None, legacy_distance, distance, 1e-12);
    Assert::assert_equals_double_msg(
        None,
        old_formula_legacy_distance,
        old_formula_distance,
        1e-12,
    );
    // This isn't true because example search center is off of the path.
    // assertEquals(oldFormulaDistance, distance, 1e-12);
    Ok(())
}

fn test_interpolation2() -> Result<()> {
    let lat = 52.5665;
    let lon = 13.3076;
    let path_lats = vec![
        52.5355, 52.54, 52.5626, 52.5665, 52.6007, 52.6135, 52.6303, 52.6651, 52.7074,
    ];
    let path_lons = vec![
        13.3634, 13.3704, 13.3307, 13.3076, 13.2806, 13.2484, 13.2406, 13.241, 13.1926,
    ];
    let car_point = GeoPoint::new_lat_lon(&SPHERE, to_radians(lat), to_radians(lon))?;
    // `new GeoPoint[pathLats.length]`, every element assigned in order.
    let mut path_points: Vec<GeoPoint> = Vec::with_capacity(path_lats.len());
    for i in 0..(path_lats.len() as i32) {
        path_points.push(GeoPoint::new_lat_lon(
            &SPHERE,
            to_radians(path_lats[i as usize]),
            to_radians(path_lons[i as usize]),
        )?);
    }
    // Construct a path with no width
    let this_path: Arc<dyn GeoPath> = GeoPathFactory::make_geo_path(&SPHERE, 0.0, &path_points)?;
    // Construct a path with a width
    let legacy_path: Arc<dyn GeoPath> = GeoPathFactory::make_geo_path(&SPHERE, 1e-6, &path_points)?;
    // Compute the inside distance to the atPoint using zero-width path
    let distance = this_path.compute_nearest_distance(&ARC, &car_point)?;
    // Compute the inside distance using legacy path
    let legacy_distance = legacy_path.compute_nearest_distance(&ARC, &car_point)?;
    // Compute the inside distance using the legacy formula
    let old_formula_distance = this_path.compute_distance(&ARC, &car_point)?;
    // Compute the inside distance using the legacy formula with the legacy shape
    let old_formula_legacy_distance = legacy_path.compute_distance(&ARC, &car_point)?;
    // These should be about the same
    Assert::assert_equals_double_msg(None, legacy_distance, distance, 1e-12);
    Assert::assert_equals_double_msg(
        None,
        old_formula_legacy_distance,
        old_formula_distance,
        1e-12,
    );
    // Since the point we picked is actually on the path, this should also be true
    Assert::assert_equals_double_msg(None, old_formula_distance, distance, 1e-12);
    Ok(())
}

// The Java test assigns values it never reads.
#[allow(unused_assignments)]
fn test_identical_points() -> Result<()> {
    let planet_model: Arc<PlanetModel> = WGS84.clone();
    let point1 =
        GeoPoint::new_lat_lon(&planet_model, 1.5707963267948963, -2.4818290647609542E-148)?;
    let point2 = GeoPoint::new_lat_lon(&planet_model, 1.570796326794895, -3.5E-323)?;
    let point3 = GeoPoint::new_lat_lon(&planet_model, 4.4E-323, -3.1415926535897896)?;
    let mut path: Arc<dyn GeoPath> = GeoPathFactory::make_geo_path(
        &planet_model,
        0.0,
        &vec![point1.clone(), point2.clone(), point3.clone()],
    )?;
    let point = GeoPoint::new_lat_lon(&planet_model, -1.5707963267948952, 2.369064805649877E-284)?;
    // If not filtered the point is wrongly in set
    Assert::assert_false(path.is_within(&point));
    // If not filtered it throws error
    path = GeoPathFactory::make_geo_path(
        &planet_model,
        1e-6,
        &vec![point1.clone(), point2.clone(), point3.clone()],
    )?;
    Assert::assert_false(path.is_within(&point));
    let point4 = GeoPoint::new_lat_lon(&planet_model, 1.5, 0.0)?;
    let point5 = GeoPoint::new_lat_lon(&planet_model, 1.5, 0.0)?;
    let point6 = GeoPoint::new_lat_lon(&planet_model, 4.4E-323, -3.1415926535897896)?;
    // If not filtered creates a degenerated Vector
    path = GeoPathFactory::make_geo_path(
        &planet_model,
        0.0,
        &vec![point4.clone(), point5.clone(), point6.clone()],
    )?;
    path = GeoPathFactory::make_geo_path(
        &planet_model,
        0.5,
        &vec![point4.clone(), point5.clone(), point6.clone()],
    )?;
    Ok(())
}

fn test_lucene8696() -> Result<()> {
    // `new GeoPoint[4]`, every element assigned in order.
    let points = vec![
        GeoPoint::new_lat_lon(&WGS84, 2.4457272005608357E-47, 0.017453291479645996)?,
        GeoPoint::new_lat_lon(&WGS84, 2.4457272005608357E-47, 0.8952476719156919)?,
        GeoPoint::new_lat_lon(&WGS84, 2.4457272005608357E-47, 0.6491968536639036)?,
        GeoPoint::new_lat_lon(&WGS84, -0.7718789008737459, 0.9236607495528212)?,
    ];
    let path: Arc<dyn GeoPath> =
        GeoPathFactory::make_geo_path(&WGS84, 1.3439035240356338, &points)?;
    let check = GeoPoint::new(0.02071783020158524, 0.9523290535474472, 0.30699177256064203);
    // Map to surface point, to remove that source of confusion
    let surface_check = WGS84.create_surface_point(&check);
    /*
    [junit4]   1>   cycle: cell=12502 parentCellID=12500 x: -1658490249 TO 2147483041, y: 2042111310 TO 2147483041, z: -2140282940 TO 2140277970, splits: 1 queue.size()=1
    [junit4]   1>     minx=-0.7731590077686981 maxx=1.0011188539924791 miny=0.9519964046486451 maxy=1.0011188539924791 minz=-0.9977622932859775 maxz=0.9977599768255027
    [junit4]   1>     GeoArea.CONTAINS: now addAll
     */
    let solid: Arc<dyn XYZSolid> = XYZSolidFactory::make_xyz_solid(
        &WGS84,
        -0.7731590077686981,
        1.0011188539924791,
        0.9519964046486451,
        1.0011188539924791,
        -0.9977622932859775,
        0.9977599768255027,
    )?;
    // Verify that the point is within it
    Assert::assert_true(solid.is_within(&surface_check));
    // Check the (surface) relationship
    let relationship = solid.get_relationship(&*path)?;
    if relationship == CONTAINS {
        // If relationship is CONTAINS then any point in the solid must also be within the path
        // If point is within solid, it must be within shape
        Assert::assert_true(path.is_within(&surface_check));
    }
    Ok(())
}
