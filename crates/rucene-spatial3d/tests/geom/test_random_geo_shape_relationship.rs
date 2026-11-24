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

//! Port of `TestRandomGeoShapeRelationship.java`.
//!
//! Java's `shape instanceof GeoArea` / `(GeoArea) shape` over the shapes
//! `RandomGeo3dShapeGenerator` builds is the cast to `GeoAreaShape`
//! ([`SerializableObject::into_geo_area_shape`]): every generated shape class
//! is a `GeoAreaShape`, and the `GeoArea`s that are not (the `XYZSolid`s) are
//! never generated.

use rucene_spatial_test_fixtures::random_geo3d_shape_generator::Constraints;
use rucene_test_framework::util::lucene_test_case::Nightly;

use crate::spatial3d_support::prelude::*;

type G = RandomGeo3dShapeGenerator;

/// `TestRandomGeoShapeRelationship`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial3d.geom.TestRandomGeoShapeRelationship",
        vec![
            test("testRandomPointWithin", test_random_point_within).annotate(Test::new()),
            test("testRandomPointNotWithin", test_random_point_not_within),
            test("testRandomDisjoint", test_random_disjoint).annotate(Test::new()),
            test("testRandomWithIn", test_random_with_in).annotate(Test::new()),
            test("testRandomContains", test_random_contains)
                .annotate(Test::new())
                .annotate(Nightly),
            test("testRandomOverlaps", test_random_overlaps).annotate(Test::new()),
        ],
    )
}

/// `randomGeoAreaShape(shapeType, planetModel, constraints)`.
fn area_shape(
    shape_type: i32,
    planet_model: &Arc<PlanetModel>,
    constraints: &Constraints,
) -> Result<Option<Arc<dyn GeoAreaShape>>> {
    G::random_geo_area_shape_with_constraints(shape_type, planet_model, constraints)
}

/// Test for WITHIN points. We build a WITHIN shape with respect the
/// geoAreaShape and create a point WITHIN the shape. The resulting shape
/// should be WITHIN the original shape.
fn test_random_point_within() -> Result<()> {
    let reference_shape_type = G::CONVEX_POLYGON;
    let planet_model = G::random_planet_model();
    let mut shape_type = G::random_shape_type();
    while shape_type == G::POINT || shape_type == G::LINE {
        shape_type = G::random_shape_type();
    }
    let mut shape: Option<Arc<dyn GeoAreaShape>> = None;
    let mut point: Option<GeoPoint> = None;
    while point.is_none() {
        shape = Some(G::random_geo_area_shape(shape_type, &planet_model)?);
        let mut constraints = G::get_empty_constraint();
        constraints.put(npe(shape.clone())?, WITHIN);
        let reference = area_shape(reference_shape_type, &planet_model, &constraints)?;
        if let Some(reference) = reference {
            constraints = Constraints::new();
            constraints.put(reference, WITHIN);
            point = G::random_geo_point_with_constraints(&planet_model, &constraints)?;
        }
    }
    let (shape, point) = (npe(shape)?, npe(point)?);
    let mut b = String::new();
    b.push_str(&format!("shape: {shape}\n"));
    b.push_str(&format!("point: {point}"));
    Assert::assert_true_msg(&b, shape.is_within(&point));
    Ok(())
}

/// Test for NOT WITHIN points. We build a DIJOINT shape with respect the
/// geoAreaShape and create a point WITHIN that shape. The resulting shape
/// should not be WITHIN the original shape.
fn test_random_point_not_within() -> Result<()> {
    let reference_shape_type = G::CONVEX_POLYGON;
    let planet_model = G::random_planet_model();
    let shape_type = G::random_shape_type();
    let mut shape: Option<Arc<dyn GeoAreaShape>> = None;
    let mut point: Option<GeoPoint> = None;
    while point.is_none() {
        shape = Some(G::random_geo_area_shape(shape_type, &planet_model)?);
        let mut constraints = G::get_empty_constraint();
        constraints.put(npe(shape.clone())?, DISJOINT);
        let reference = area_shape(reference_shape_type, &planet_model, &constraints)?;
        if let Some(reference) = reference {
            constraints = Constraints::new();
            constraints.put(reference, WITHIN);
            point = G::random_geo_point_with_constraints(&planet_model, &constraints)?;
        }
    }
    let (shape, point) = (npe(shape)?, npe(point)?);
    let mut b = String::new();
    b.push_str(&format!("shape: {shape}\n"));
    b.push_str(&format!("point: {point}"));
    Assert::assert_false_msg(&b, shape.is_within(&point));
    Ok(())
}

/// The two relationship assertions every shape test ends with:
/// `geoAreaShape.getRelationship(shape)` and, when the shape is a
/// `GeoArea`, its relationship to `geoAreaShape`.
fn check_relationships(
    geo_area_shape: &Arc<dyn GeoAreaShape>,
    shape: &Arc<dyn GeoShape>,
    expected: i32,
    expected_reverse: i32,
) -> Result<()> {
    let mut b = String::new();
    b.push_str(&format!("geoAreaShape: {geo_area_shape}\n"));
    b.push_str(&format!("shape: {shape}"));
    let mut rel = geo_area_shape.get_relationship(&**shape)?;
    Assert::assert_equals_msg(Some(&b), expected, rel);
    if let Some(area) = shape.clone().into_geo_area_shape() {
        rel = area.get_relationship(&**geo_area_shape)?;
        Assert::assert_equals_msg(Some(&b), expected_reverse, rel);
    }
    Ok(())
}

/// Test for disjoint shapes. We build a DISJOINT shape with respect the
/// geoAreaShape and create shapes WITHIN that shapes. The resulting shape
/// should be DISJOINT to the geoAreaShape.
///
/// Note that both shapes cannot be concave.
fn test_random_disjoint() -> Result<()> {
    let reference_shape_type = G::CONVEX_SIMPLE_POLYGON;
    let planet_model = G::random_planet_model();
    let geo_area_shape_type = G::random_geo_area_shape_type();
    let shape_type = G::random_convex_shape_type();

    let mut shape: Option<Arc<dyn GeoShape>> = None;
    let mut geo_area_shape: Option<Arc<dyn GeoAreaShape>> = None;
    while shape.is_none() {
        geo_area_shape = Some(G::random_geo_area_shape(
            geo_area_shape_type,
            &planet_model,
        )?);
        let mut constraints = Constraints::new();
        constraints.put(npe(geo_area_shape.clone())?, DISJOINT);
        let reference = area_shape(reference_shape_type, &planet_model, &constraints)?;
        if let Some(reference) = reference {
            constraints = G::get_empty_constraint();
            constraints.put(reference, WITHIN);
            shape = G::random_geo_shape_with_constraints(shape_type, &planet_model, &constraints)?;
        }
    }
    check_relationships(&npe(geo_area_shape)?, &npe(shape)?, DISJOINT, DISJOINT)
}

/// Test for within shapes. We build a shape WITHIN the geoAreaShape and create
/// shapes WITHIN that shape. The resulting shape should be WITHIN to the
/// geoAreaShape.
///
/// Note that if the geoAreaShape is not concave the other shape must be not
/// concave.
fn test_random_with_in() -> Result<()> {
    let planet_model = G::random_planet_model();
    let mut geo_area_shape_type = G::random_geo_area_shape_type();
    // shapes cannot be point or line -- no area!
    while geo_area_shape_type == G::POINT || geo_area_shape_type == G::LINE {
        geo_area_shape_type = G::random_geo_area_shape_type();
    }
    let mut shape_type = G::LINE; // randomShapeType();
    let mut reference_shape_type = G::CONVEX_SIMPLE_POLYGON;
    if !G::is_concave(geo_area_shape_type) {
        shape_type = G::random_convex_shape_type();
    }
    if G::is_concave(shape_type) {
        // both concave
        reference_shape_type = G::CONCAVE_SIMPLE_POLYGON;
    }
    let mut shape: Option<Arc<dyn GeoShape>> = None;
    let mut geo_area_shape: Option<Arc<dyn GeoAreaShape>> = None;
    while shape.is_none() {
        geo_area_shape = Some(G::random_geo_area_shape(
            geo_area_shape_type,
            &planet_model,
        )?);
        let mut constraints = Constraints::new();
        constraints.put(npe(geo_area_shape.clone())?, WITHIN);
        let reference = area_shape(reference_shape_type, &planet_model, &constraints)?;
        if let Some(reference) = reference {
            constraints = Constraints::new();
            constraints.put(reference, WITHIN);
            shape = G::random_geo_shape_with_constraints(shape_type, &planet_model, &constraints)?;
        }
    }
    check_relationships(&npe(geo_area_shape)?, &npe(shape)?, WITHIN, CONTAINS)
}

/// Test for contains shapes. We build a shape containing the geoAreaShape and
/// create shapes WITHIN that shape. The resulting shape should CONTAIN the
/// geoAreaShape.
///
/// Note that if the geoAreaShape is concave the other shape must be concave.
/// If shape is concave, the shape for reference should be concave as well.
// TODO: this test seems to hit pathological cases that cause it to run for many minutes?!
fn test_random_contains() -> Result<()> {
    let mut reference_shape_type = G::CONVEX_SIMPLE_POLYGON;
    let planet_model = G::random_planet_model();
    let mut geo_area_shape_type = G::random_geo_area_shape_type();
    while geo_area_shape_type == G::COLLECTION {
        geo_area_shape_type = G::random_geo_area_shape_type();
    }
    let mut shape_type = G::random_shape_type();
    while shape_type == G::POINT || shape_type == G::LINE {
        shape_type = G::random_shape_type();
    }
    if G::is_concave(geo_area_shape_type) {
        shape_type = G::random_concave_shape_type();
    }
    if G::is_concave(shape_type) {
        reference_shape_type = G::CONCAVE_SIMPLE_POLYGON;
    }
    let mut shape: Option<Arc<dyn GeoShape>> = None;
    let mut geo_area_shape: Option<Arc<dyn GeoAreaShape>> = None;
    while shape.is_none() {
        geo_area_shape = Some(G::random_geo_area_shape(
            geo_area_shape_type,
            &planet_model,
        )?);
        let mut constraints = G::get_empty_constraint();
        constraints.put(npe(geo_area_shape.clone())?, CONTAINS);
        // `(GeoPolygon) randomGeoAreaShape(...)`: the simple polygon types
        // always build a `GeoPolygon`.
        let reference = match area_shape(reference_shape_type, &planet_model, &constraints)? {
            None => None,
            Some(r) => Some(class_cast_polygon(r)?),
        };
        if let Some(reference) = reference {
            constraints = G::get_empty_constraint();
            constraints.put(reference, CONTAINS);
            shape = G::random_geo_shape_with_constraints(shape_type, &planet_model, &constraints)?;
        }
    }
    check_relationships(&npe(geo_area_shape)?, &npe(shape)?, CONTAINS, WITHIN)
}

/// `(GeoPolygon) shape`, as an area shape again for the constraints.
fn class_cast_polygon(shape: Arc<dyn GeoAreaShape>) -> Result<Arc<dyn GeoAreaShape>> {
    let class_name = shape.class_name();
    match shape.into_geo_polygon() {
        Some(p) => Ok(p),
        None => Err(LuceneError::unchecked(
            "java.lang.ClassCastException",
            format!(
                "class {class_name} cannot be cast to class org.apache.lucene.spatial3d.geom.GeoPolygon"
            ),
        )),
    }
}

/// Test for overlapping shapes. We build a shape that contains part of the
/// geoAreaShape, is disjoint to other part and contains a disjoint shape. We
/// create shapes according the criteria. The resulting shape should OVERLAP
/// the geoAreaShape.
fn test_random_overlaps() -> Result<()> {
    let planet_model = G::random_planet_model();
    let mut geo_area_shape_type = G::random_geo_area_shape_type();
    while geo_area_shape_type == G::POINT || geo_area_shape_type == G::LINE {
        geo_area_shape_type = G::random_geo_area_shape_type();
    }
    let mut shape_type = G::random_shape_type();
    while shape_type == G::POINT || shape_type == G::LINE {
        shape_type = G::random_shape_type();
    }
    let mut shape: Option<Arc<dyn GeoShape>> = None;
    let mut geo_area_shape: Option<Arc<dyn GeoAreaShape>> = None;
    while shape.is_none() {
        let gas = G::random_geo_area_shape(geo_area_shape_type, &planet_model)?;
        geo_area_shape = Some(gas.clone());
        let mut constraints = G::get_empty_constraint();
        constraints.put(gas.clone(), WITHIN);
        let Some(reference1) = area_shape(G::CONVEX_SIMPLE_POLYGON, &planet_model, &constraints)?
        else {
            continue;
        };
        constraints = G::get_empty_constraint();
        constraints.put(gas.clone(), WITHIN);
        constraints.put(reference1.clone(), DISJOINT);
        let Some(reference2) = area_shape(G::CONVEX_SIMPLE_POLYGON, &planet_model, &constraints)?
        else {
            continue;
        };
        constraints = G::get_empty_constraint();
        constraints.put(gas.clone(), DISJOINT);
        let reference3 = area_shape(G::CONVEX_SIMPLE_POLYGON, &planet_model, &constraints)?;
        if let Some(reference3) = reference3 {
            constraints = Constraints::new();
            constraints.put(reference1, DISJOINT);
            constraints.put(reference2, CONTAINS);
            constraints.put(reference3, CONTAINS);
            shape = G::random_geo_shape_with_constraints(shape_type, &planet_model, &constraints)?;
        }
    }
    check_relationships(&npe(geo_area_shape)?, &npe(shape)?, OVERLAPS, OVERLAPS)
}
