/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `GeodesicSphereDistCalc.java`, of the Java package
//! `org.locationtech.spatial4j.distance`.

use rucene::util::java_math;
use std::any::Any;
use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::util::java_collections::java_identity_hash_code_stand_in;

use crate::context::spatial_context::SpatialContext;
use crate::distance::distance_calculator::DistanceCalculator;
use crate::distance::distance_utils::DistanceUtils;
use crate::shape::circle::Circle;
use crate::shape::point::Point;
use crate::shape::rectangle::Rectangle;

/// `radiusDEG`: `DistanceUtils.toDegrees(1)`, the sphere's radius in degrees.
fn radius_deg() -> f64 {
    DistanceUtils::to_degrees(1.0)
}

/// Equivalent to `org.locationtech.spatial4j.distance.GeodesicSphereDistCalc`
/// (`public abstract class GeodesicSphereDistCalc extends
/// AbstractDistanceCalculator`): distances on a sphere, in degrees. The
/// subclasses implement [`distance_lat_lon_rad`](Self::distance_lat_lon_rad).
///
/// Equivalent to `GeodesicSphereDistCalc.area(Rectangle)` and to its
/// `area(Circle)` overload: the `area_rect` and `area_circle` that the
/// `geodesic_sphere_dist_calc!` macro implements for every subclass.
pub trait GeodesicSphereDistCalc: Send + Sync + fmt::Debug + Any {
    /// `distanceLatLonRAD(double lat1, double lon1, double lat2, double
    /// lon2)`.
    fn distance_lat_lon_rad(&self, lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64;

    /// `getClass().getSimpleName()`.
    fn simple_class_name(&self) -> &'static str;
}

/// The `GeodesicSphereDistCalc` methods every subclass inherits.
macro_rules! geodesic_sphere_dist_calc {
    ($ty:ty, $simple:literal) => {
        impl DistanceCalculator for $ty {
            /// `distance(Point, double, double)` (final in Java).
            fn distance_xy(&self, from: &dyn Point, to_x: f64, to_y: f64) -> f64 {
                DistanceUtils::to_degrees(self.distance_lat_lon_rad(
                    DistanceUtils::to_radians(from.get_y()),
                    DistanceUtils::to_radians(from.get_x()),
                    DistanceUtils::to_radians(to_y),
                    DistanceUtils::to_radians(to_x),
                ))
            }

            fn point_on_bearing(
                &self,
                from: &Arc<dyn Point>,
                dist_deg: f64,
                bearing_deg: f64,
                ctx: &SpatialContext,
                reuse: Option<Arc<dyn Point>>,
            ) -> Result<Arc<dyn Point>> {
                if dist_deg == 0.0 {
                    let Some(reuse) = reuse else {
                        return Ok(Arc::clone(from));
                    };
                    reuse.reset(from.get_x(), from.get_y());
                    return Ok(reuse);
                }
                let result = DistanceUtils::point_on_bearing_rad(
                    DistanceUtils::to_radians(from.get_y()),
                    DistanceUtils::to_radians(from.get_x()),
                    DistanceUtils::to_radians(dist_deg),
                    DistanceUtils::to_radians(bearing_deg),
                    ctx,
                    reuse,
                )?; //output result is in radians
                result.reset(
                    DistanceUtils::to_degrees(result.get_x()),
                    DistanceUtils::to_degrees(result.get_y()),
                );
                Ok(result)
            }

            fn calc_box_by_dist_from_pt(
                &self,
                from: &dyn Point,
                dist_deg: f64,
                ctx: &SpatialContext,
                reuse: Option<Arc<dyn Rectangle>>,
            ) -> Result<Arc<dyn Rectangle>> {
                DistanceUtils::calc_box_by_dist_from_pt_deg(
                    from.get_y(),
                    from.get_x(),
                    dist_deg,
                    ctx,
                    reuse,
                )
            }

            fn calc_box_by_dist_from_pt_y_horiz_axis_deg(
                &self,
                from: &dyn Point,
                dist_deg: f64,
                _ctx: &SpatialContext,
            ) -> f64 {
                DistanceUtils::calc_box_by_dist_from_pt_lat_horiz_axis_deg(
                    from.get_y(),
                    from.get_x(),
                    dist_deg,
                )
            }

            /// Equivalent to `GeodesicSphereDistCalc.area(Rectangle)`.
            fn area_rect(&self, rect: &dyn Rectangle) -> f64 {
                //From http://mathforum.org/library/drmath/view/63767.html
                let lat1 = DistanceUtils::to_radians(rect.get_min_y());
                let lat2 = DistanceUtils::to_radians(rect.get_max_y());
                std::f64::consts::PI / 180.0
                    * radius_deg()
                    * radius_deg()
                    * (java_math::sin(lat1) - java_math::sin(lat2)).abs()
                    * rect.get_width()
            }

            /// Equivalent to `GeodesicSphereDistCalc.area(Circle)`.
            fn area_circle(&self, circle: &dyn Circle) -> f64 {
                //formula is a simplified case of area(rect).
                let lat = DistanceUtils::to_radians(90.0 - circle.get_radius());
                2.0 * std::f64::consts::PI
                    * radius_deg()
                    * radius_deg()
                    * (1.0 - java_math::sin(lat))
            }

            /// `equals(Object)`: same class.
            fn equals(&self, other: &dyn DistanceCalculator) -> bool {
                other.as_any().type_id() == std::any::TypeId::of::<$ty>()
            }

            /// `hashCode()`: `getClass().hashCode()`, an identity hash.
            fn hash_code(&self) -> i32 {
                java_identity_hash_code_stand_in(concat!(
                    "org.locationtech.spatial4j.distance.GeodesicSphereDistCalc$",
                    $simple
                ))
            }

            fn as_any(&self) -> &dyn Any {
                self
            }
        }

        impl fmt::Display for $ty {
            /// `toString()`: `getClass().getSimpleName()`.
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str($simple)
            }
        }
    };
}

/// Equivalent to
/// `org.locationtech.spatial4j.distance.GeodesicSphereDistCalc.Haversine`
/// (`public static class Haversine extends GeodesicSphereDistCalc`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Haversine;

impl GeodesicSphereDistCalc for Haversine {
    fn distance_lat_lon_rad(&self, lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
        DistanceUtils::dist_haversine_rad(lat1, lon1, lat2, lon2)
    }

    fn simple_class_name(&self) -> &'static str {
        "Haversine"
    }
}

geodesic_sphere_dist_calc!(Haversine, "Haversine");

/// Equivalent to
/// `org.locationtech.spatial4j.distance.GeodesicSphereDistCalc.LawOfCosines`
/// (`public static class LawOfCosines extends GeodesicSphereDistCalc`).
#[derive(Clone, Copy, Debug, Default)]
pub struct LawOfCosines;

impl GeodesicSphereDistCalc for LawOfCosines {
    fn distance_lat_lon_rad(&self, lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
        DistanceUtils::dist_law_of_cosines_rad(lat1, lon1, lat2, lon2)
    }

    fn simple_class_name(&self) -> &'static str {
        "LawOfCosines"
    }
}

geodesic_sphere_dist_calc!(LawOfCosines, "LawOfCosines");

/// Equivalent to
/// `org.locationtech.spatial4j.distance.GeodesicSphereDistCalc.Vincenty`
/// (`public static class Vincenty extends GeodesicSphereDistCalc`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Vincenty;

impl GeodesicSphereDistCalc for Vincenty {
    fn distance_lat_lon_rad(&self, lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
        DistanceUtils::dist_vincenty_rad(lat1, lon1, lat2, lon2)
    }

    fn simple_class_name(&self) -> &'static str {
        "Vincenty"
    }
}

geodesic_sphere_dist_calc!(Vincenty, "Vincenty");
