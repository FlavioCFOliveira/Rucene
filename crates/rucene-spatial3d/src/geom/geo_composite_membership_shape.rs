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

//! The types declared in `GeoCompositeMembershipShape.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::fmt;
use std::io::{Read, Write};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use rucene::error::Result;

use super::bounded::Bounded;
use super::bounds::Bounds;
use super::distance_style::DistanceStyle;
#[allow(unused_imports)]
use super::geo_area::GeoArea;
#[allow(unused_imports)]
use super::geo_area_shape::GeoAreaShape;
#[allow(unused_imports)]
use super::geo_base_composite_area_shape::GeoBaseCompositeAreaShape;
use super::geo_base_composite_membership_shape::GeoBaseCompositeMembershipShape;
use super::geo_base_composite_shape::GeoBaseCompositeShape;
use super::geo_membership_shape::GeoMembershipShape;
use super::geo_outside_distance::GeoOutsideDistance;
use super::geo_point::GeoPoint;
#[allow(unused_imports)]
use super::geo_polygon::GeoPolygon;
use super::geo_shape::GeoShape;
use super::list_to_string;
use super::membership::Membership;
use super::plane::Plane;
use super::planet_model::PlanetModel;
use super::serializable_object::SerializableObject;

/// GeoComposite is a set of GeoMembershipShape's, treated as a unit.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoCompositeMembershipShape`. Java's `extends
/// GeoBaseCompositeShape<GeoMembershipShape>` is a [`Deref`] to the composite.
pub struct GeoCompositeMembershipShape {
    base: GeoBaseCompositeShape<dyn GeoMembershipShape>,
}

impl Deref for GeoCompositeMembershipShape {
    type Target = GeoBaseCompositeShape<dyn GeoMembershipShape>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl DerefMut for GeoCompositeMembershipShape {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

impl GeoCompositeMembershipShape {
    /// `new GeoCompositeMembershipShape(PlanetModel)`: an empty composite.
    pub fn new(planet_model: &Arc<PlanetModel>) -> Self {
        Self {
            base: GeoBaseCompositeShape::new(planet_model),
        }
    }

    /// `new GeoCompositeMembershipShape(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream; `ClassCastException` for a member of
    /// the wrong type.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            base: GeoBaseCompositeShape::from_stream(
                planet_model,
                input,
                "org.apache.lucene.spatial3d.geom.GeoMembershipShape",
                |o: Arc<dyn SerializableObject>| o.into_geo_membership_shape(),
            )?,
        })
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        self.base.write(output)
    }

    fn equals_impl(&self, other: &Self) -> bool {
        self.base.equals(&other.base)
    }

    fn hash_code_impl(&self) -> i32 {
        self.base.hash_code()
    }
}

impl_serializable_object!(
    GeoCompositeMembershipShape,
    "org.apache.lucene.spatial3d.geom.GeoCompositeMembershipShape",
    planet_object,
    geo_membership_shape
);

impl super::planet_object::PlanetObject for GeoCompositeMembershipShape {
    fn get_planet_model(&self) -> &Arc<PlanetModel> {
        &self.base.planet_model
    }
}

impl_geo_bounds!(GeoCompositeMembershipShape);

impl Membership for GeoCompositeMembershipShape {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.base.is_within_xyz(x, y, z)
    }
}

impl Bounded for GeoCompositeMembershipShape {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        self.base.get_bounds(bounds);
    }
}

impl GeoShape for GeoCompositeMembershipShape {
    fn get_edge_points(&self) -> Vec<GeoPoint> {
        self.base.get_edge_points()
    }

    fn intersects(
        &self,
        p: &Plane,
        notable_points: &[GeoPoint],
        bounds: &[&dyn Membership],
    ) -> bool {
        self.base.intersects(p, notable_points, bounds)
    }
}

impl GeoOutsideDistance for GeoCompositeMembershipShape {
    fn compute_outside_distance_xyz(
        &self,
        distance_style: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<f64> {
        GeoBaseCompositeMembershipShape::compute_outside_distance_xyz(
            &self.base,
            distance_style,
            x,
            y,
            z,
        )
    }
}

impl GeoMembershipShape for GeoCompositeMembershipShape {}

impl fmt::Display for GeoCompositeMembershipShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GeoCompositeMembershipShape: {{{}}}",
            list_to_string(&self.base.shapes)
        )
    }
}
