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

//! The types declared in `StandardObjects.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, LazyLock, RwLock};

use rucene::error::{LuceneError, Result};

use super::d_x_yd_z_solid::dXYdZSolid;
use super::d_xd_yd_z_solid::dXdYdZSolid;
use super::d_xd_yz_solid::dXdYZSolid;
use super::d_xyz_solid::dXYZSolid;
use super::geo_complex_polygon::GeoComplexPolygon;
use super::geo_composite_area_shape::GeoCompositeAreaShape;
use super::geo_composite_membership_shape::GeoCompositeMembershipShape;
use super::geo_composite_polygon::GeoCompositePolygon;
use super::geo_concave_polygon::GeoConcavePolygon;
use super::geo_convex_polygon::GeoConvexPolygon;
use super::geo_degenerate_horizontal_line::GeoDegenerateHorizontalLine;
use super::geo_degenerate_latitude_zone::GeoDegenerateLatitudeZone;
use super::geo_degenerate_longitude_slice::GeoDegenerateLongitudeSlice;
use super::geo_degenerate_path::GeoDegeneratePath;
use super::geo_degenerate_point::GeoDegeneratePoint;
use super::geo_degenerate_vertical_line::GeoDegenerateVerticalLine;
use super::geo_exact_circle::GeoExactCircle;
use super::geo_latitude_zone::GeoLatitudeZone;
use super::geo_longitude_slice::GeoLongitudeSlice;
use super::geo_north_latitude_zone::GeoNorthLatitudeZone;
use super::geo_north_rectangle::GeoNorthRectangle;
use super::geo_point::GeoPoint;
use super::geo_rectangle::GeoRectangle;
use super::geo_s2_shape::GeoS2Shape;
use super::geo_south_latitude_zone::GeoSouthLatitudeZone;
use super::geo_south_rectangle::GeoSouthRectangle;
use super::geo_standard_circle::GeoStandardCircle;
use super::geo_standard_path::GeoStandardPath;
use super::geo_wide_degenerate_horizontal_line::GeoWideDegenerateHorizontalLine;
use super::geo_wide_longitude_slice::GeoWideLongitudeSlice;
use super::geo_wide_north_rectangle::GeoWideNorthRectangle;
use super::geo_wide_rectangle::GeoWideRectangle;
use super::geo_wide_south_rectangle::GeoWideSouthRectangle;
use super::geo_world::GeoWorld;
use super::planet_model::PlanetModel;
use super::serializable_object::SerializableObject;
use super::standard_xyz_solid::StandardXYZSolid;
use super::x_yd_z_solid::XYdZSolid;
use super::xd_yd_z_solid::XdYdZSolid;
use super::xd_yz_solid::XdYZSolid;

/// Lookup tables for classes that can be serialized using a code.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.StandardObjects`: the
/// class registry is the index of [`StandardObjects::CLASSES`], each class
/// named by its binary name.
pub(crate) struct StandardObjects;

impl StandardObjects {
    /// `CLASS_REGISTRY` / `CODE_REGISTRY`: the class of each code.
    pub(crate) const CLASSES: [&'static str; 39] = [
        "org.apache.lucene.spatial3d.geom.GeoPoint",
        "org.apache.lucene.spatial3d.geom.GeoRectangle",
        "org.apache.lucene.spatial3d.geom.GeoStandardCircle",
        "org.apache.lucene.spatial3d.geom.GeoStandardPath",
        "org.apache.lucene.spatial3d.geom.GeoConvexPolygon",
        "org.apache.lucene.spatial3d.geom.GeoConcavePolygon",
        "org.apache.lucene.spatial3d.geom.GeoComplexPolygon",
        "org.apache.lucene.spatial3d.geom.GeoCompositePolygon",
        "org.apache.lucene.spatial3d.geom.GeoCompositeMembershipShape",
        "org.apache.lucene.spatial3d.geom.GeoCompositeAreaShape",
        "org.apache.lucene.spatial3d.geom.GeoDegeneratePoint",
        "org.apache.lucene.spatial3d.geom.GeoDegenerateHorizontalLine",
        "org.apache.lucene.spatial3d.geom.GeoDegenerateLatitudeZone",
        "org.apache.lucene.spatial3d.geom.GeoDegenerateLongitudeSlice",
        "org.apache.lucene.spatial3d.geom.GeoDegenerateVerticalLine",
        "org.apache.lucene.spatial3d.geom.GeoLatitudeZone",
        "org.apache.lucene.spatial3d.geom.GeoLongitudeSlice",
        "org.apache.lucene.spatial3d.geom.GeoNorthLatitudeZone",
        "org.apache.lucene.spatial3d.geom.GeoNorthRectangle",
        "org.apache.lucene.spatial3d.geom.GeoSouthLatitudeZone",
        "org.apache.lucene.spatial3d.geom.GeoSouthRectangle",
        "org.apache.lucene.spatial3d.geom.GeoWideDegenerateHorizontalLine",
        "org.apache.lucene.spatial3d.geom.GeoWideLongitudeSlice",
        "org.apache.lucene.spatial3d.geom.GeoWideNorthRectangle",
        "org.apache.lucene.spatial3d.geom.GeoWideRectangle",
        "org.apache.lucene.spatial3d.geom.GeoWideSouthRectangle",
        "org.apache.lucene.spatial3d.geom.GeoWorld",
        "org.apache.lucene.spatial3d.geom.dXdYdZSolid",
        "org.apache.lucene.spatial3d.geom.dXdYZSolid",
        "org.apache.lucene.spatial3d.geom.dXYdZSolid",
        "org.apache.lucene.spatial3d.geom.dXYZSolid",
        "org.apache.lucene.spatial3d.geom.XdYdZSolid",
        "org.apache.lucene.spatial3d.geom.XdYZSolid",
        "org.apache.lucene.spatial3d.geom.XYdZSolid",
        "org.apache.lucene.spatial3d.geom.StandardXYZSolid",
        "org.apache.lucene.spatial3d.geom.PlanetModel",
        "org.apache.lucene.spatial3d.geom.GeoDegeneratePath",
        "org.apache.lucene.spatial3d.geom.GeoExactCircle",
        "org.apache.lucene.spatial3d.geom.GeoS2Shape",
    ];

    /// `CLASS_REGISTRY.get(clazz)`.
    pub(crate) fn code_of(clazz: &str) -> Option<i32> {
        Self::CLASSES
            .iter()
            .position(|c| *c == clazz)
            .map(|i| i as i32)
    }

    /// `CODE_REGISTRY.get(index)`.
    pub(crate) fn class_of(index: i32) -> Option<&'static str> {
        if index < 0 {
            return None;
        }
        Self::CLASSES.get(index as usize).copied()
    }
}

/// The `(InputStream)` constructor of a class registered with
/// [`register_class`].
pub type StreamConstructor =
    Arc<dyn Fn(&mut dyn Read) -> Result<Arc<dyn SerializableObject>> + Send + Sync>;

/// The `(PlanetModel, InputStream)` constructor of a class registered with
/// [`register_class`].
pub type PlanetModelStreamConstructor = Arc<
    dyn Fn(&Arc<PlanetModel>, &mut dyn Read) -> Result<Arc<dyn SerializableObject>> + Send + Sync,
>;

/// A class that is not one of the package's own serializable classes, as
/// `readObject` finds it by name on the class path.
#[derive(Clone)]
pub enum JavaClass {
    /// A class implementing `SerializableObject`, with the constructors it
    /// declares (`None` for one it lacks: `NoSuchMethodException`).
    Serializable {
        /// `(InputStream)`.
        from_stream: Option<StreamConstructor>,
        /// `(PlanetModel, InputStream)`.
        from_stream_with_model: Option<PlanetModelStreamConstructor>,
    },
    /// A class that does not implement `SerializableObject`: `readClass`
    /// rejects it before loading or instantiating it.
    NotSerializable,
}

/// The classes beyond the package's own that `readClass` can find by name.
static CLASS_PATH: LazyLock<RwLock<HashMap<String, JavaClass>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Makes the class `name` (Java's binary name, `getClass().getName()`)
/// findable by `SerializableObject.readObject`, as a class on Java's class
/// path is: Java's `readClass` loads any class by name through
/// `MethodHandles.lookup().findClass` and instantiates a `SerializableObject`
/// reflectively; Rust has no class path, so a class outside the package
/// (a user's `SerializableObject`, or any other class a stream may name) is
/// registered with its constructors once, before it is read.
pub fn register_class(name: impl Into<String>, class: JavaClass) {
    CLASS_PATH
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(name.into(), class);
}

/// The registered class `name`.
fn registered(name: &str) -> Option<JavaClass> {
    CLASS_PATH
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(name)
        .cloned()
}

/// What `MethodHandles.lookup().findClass(name).asSubclass(SerializableObject.class)`
/// finds for `name`.
pub(crate) enum ClassLookup {
    /// A `SerializableObject` class.
    Found,
    /// A class that is not a `SerializableObject`: `ClassCastException`.
    NotSerializable,
    /// No such class: `ClassNotFoundException`.
    NotFound,
}

/// `MethodHandles.lookup().findClass(name).asSubclass(SerializableObject.class)`.
pub(crate) fn find_class(name: &str) -> ClassLookup {
    if StandardObjects::code_of(name).is_some() {
        return ClassLookup::Found;
    }
    match registered(name) {
        Some(JavaClass::Serializable { .. }) => ClassLookup::Found,
        Some(JavaClass::NotSerializable) => ClassLookup::NotSerializable,
        None => ClassLookup::NotFound,
    }
}

/// A registered class's constructor invocation.
fn new_registered_instance(
    clazz: &str,
    invoke: impl FnOnce(&JavaClass) -> Option<Result<Arc<dyn SerializableObject>>>,
) -> Instantiation {
    match registered(clazz).as_ref().and_then(invoke) {
        Some(Ok(o)) => Instantiation::Created(o),
        Some(Err(e)) => Instantiation::Failed(e),
        None => Instantiation::NoSuchConstructor,
    }
}

/// The outcome of a reflective constructor invocation.
pub(crate) enum Instantiation {
    /// The constructed object.
    Created(Arc<dyn SerializableObject>),
    /// `NoSuchMethodException`: the class lacks the constructor.
    NoSuchConstructor,
    /// `InvocationTargetException`: the constructor threw (its cause kept,
    /// as Java's exception keeps it, though no caller reads it).
    Failed(#[allow(dead_code)] LuceneError),
}

fn wrap<T: SerializableObject>(r: Result<T>) -> Instantiation {
    match r {
        Ok(o) => Instantiation::Created(Arc::new(o)),
        Err(e) => Instantiation::Failed(e),
    }
}

/// `clazz.getDeclaredConstructor(PlanetModel.class, InputStream.class)
/// .newInstance(planetModel, inputStream)`.
pub(crate) fn new_instance_with_model(
    clazz: &str,
    pm: &Arc<PlanetModel>,
    input: &mut dyn Read,
) -> Instantiation {
    let Some(code) = StandardObjects::code_of(clazz) else {
        return new_registered_instance(clazz, |c| match c {
            JavaClass::Serializable {
                from_stream_with_model: Some(ctor),
                ..
            } => Some(ctor(pm, input)),
            _ => None,
        });
    };
    match code {
        0 => wrap(GeoPoint::from_stream_with_model(pm, input)),
        1 => wrap(GeoRectangle::from_stream(pm, input)),
        2 => wrap(GeoStandardCircle::from_stream(pm, input)),
        3 => wrap(GeoStandardPath::from_stream(pm, input)),
        4 => wrap(GeoConvexPolygon::from_stream(pm, input)),
        5 => wrap(GeoConcavePolygon::from_stream(pm, input)),
        6 => wrap(GeoComplexPolygon::from_stream(pm, input)),
        7 => wrap(GeoCompositePolygon::from_stream(pm, input)),
        8 => wrap(GeoCompositeMembershipShape::from_stream(pm, input)),
        9 => wrap(GeoCompositeAreaShape::from_stream(pm, input)),
        10 => wrap(GeoDegeneratePoint::from_stream(pm, input)),
        11 => wrap(GeoDegenerateHorizontalLine::from_stream(pm, input)),
        12 => wrap(GeoDegenerateLatitudeZone::from_stream(pm, input)),
        13 => wrap(GeoDegenerateLongitudeSlice::from_stream(pm, input)),
        14 => wrap(GeoDegenerateVerticalLine::from_stream(pm, input)),
        15 => wrap(GeoLatitudeZone::from_stream(pm, input)),
        16 => wrap(GeoLongitudeSlice::from_stream(pm, input)),
        17 => wrap(GeoNorthLatitudeZone::from_stream(pm, input)),
        18 => wrap(GeoNorthRectangle::from_stream(pm, input)),
        19 => wrap(GeoSouthLatitudeZone::from_stream(pm, input)),
        20 => wrap(GeoSouthRectangle::from_stream(pm, input)),
        21 => wrap(GeoWideDegenerateHorizontalLine::from_stream(pm, input)),
        22 => wrap(GeoWideLongitudeSlice::from_stream(pm, input)),
        23 => wrap(GeoWideNorthRectangle::from_stream(pm, input)),
        24 => wrap(GeoWideRectangle::from_stream(pm, input)),
        25 => wrap(GeoWideSouthRectangle::from_stream(pm, input)),
        26 => wrap(GeoWorld::from_stream(pm, input)),
        27 => wrap(dXdYdZSolid::from_stream(pm, input)),
        28 => wrap(dXdYZSolid::from_stream(pm, input)),
        29 => wrap(dXYdZSolid::from_stream(pm, input)),
        30 => wrap(dXYZSolid::from_stream(pm, input)),
        31 => wrap(XdYdZSolid::from_stream(pm, input)),
        32 => wrap(XdYZSolid::from_stream(pm, input)),
        33 => wrap(XYdZSolid::from_stream(pm, input)),
        34 => wrap(StandardXYZSolid::from_stream(pm, input)),
        35 => Instantiation::NoSuchConstructor,
        36 => wrap(GeoDegeneratePath::from_stream(pm, input)),
        37 => wrap(GeoExactCircle::from_stream(pm, input)),
        38 => wrap(GeoS2Shape::from_stream(pm, input)),
        _ => Instantiation::NoSuchConstructor,
    }
}

/// `clazz.getDeclaredConstructor(InputStream.class).newInstance(inputStream)`.
pub(crate) fn new_instance(clazz: &str, input: &mut dyn Read) -> Instantiation {
    match clazz {
        "org.apache.lucene.spatial3d.geom.GeoPoint" => wrap(GeoPoint::from_stream(input)),
        "org.apache.lucene.spatial3d.geom.PlanetModel" => wrap(PlanetModel::from_stream(input)),
        _ if StandardObjects::code_of(clazz).is_some() => Instantiation::NoSuchConstructor,
        _ => new_registered_instance(clazz, |c| match c {
            JavaClass::Serializable {
                from_stream: Some(ctor),
                ..
            } => Some(ctor(input)),
            _ => None,
        }),
    }
}
/// `RamUsageEstimator.shallowSizeOfInstance` of the package's classes: the
/// object header plus every instance field of the class and its
/// superclasses, aligned (counted from the Lucene 10.5.0 sources).
pub(crate) fn java_shallow_size_of_instance(class_name: &str) -> i64 {
    use rucene::util::RamUsageEstimator as R;
    let (refs, d8, d4, d2, d1): (i64, i64, i64, i64, i64) =
        match class_name.rsplit('.').next().unwrap_or(class_name) {
            "BasePlanetObject" => (1, 0, 0, 0, 0),
            "BaseXYZSolid" => (1, 0, 0, 0, 0),
            "GeoBaseAreaShape" => (1, 0, 0, 0, 0),
            "GeoBaseBBox" => (1, 0, 0, 0, 0),
            "GeoBaseBounds" => (1, 0, 0, 0, 0),
            "GeoBaseCircle" => (1, 0, 0, 0, 0),
            "GeoBaseCompositeAreaShape" => (2, 0, 0, 0, 0),
            "GeoBaseCompositeMembershipShape" => (2, 0, 0, 0, 0),
            "GeoBaseCompositeShape" => (2, 0, 0, 0, 0),
            "GeoBaseDistanceShape" => (1, 0, 0, 0, 0),
            "GeoBaseMembershipShape" => (1, 0, 0, 0, 0),
            "GeoBasePath" => (1, 0, 0, 0, 0),
            "GeoBasePolygon" => (1, 0, 0, 0, 0),
            "GeoBaseShape" => (1, 0, 0, 0, 0),
            "GeoComplexPolygon" => (17, 0, 0, 0, 1),
            "GeoCompositeAreaShape" => (2, 0, 0, 0, 0),
            "GeoCompositeMembershipShape" => (2, 0, 0, 0, 0),
            "GeoCompositePolygon" => (2, 0, 0, 0, 0),
            "GeoConcavePolygon" => (12, 0, 0, 0, 1),
            "GeoConvexPolygon" => (11, 0, 0, 0, 1),
            "GeoDegenerateHorizontalLine" => (9, 3, 0, 0, 0),
            "GeoDegenerateLatitudeZone" => (4, 2, 0, 0, 0),
            "GeoDegenerateLongitudeSlice" => (6, 1, 0, 0, 0),
            "GeoDegeneratePath" => (5, 0, 0, 0, 1),
            "GeoDegeneratePoint" => (2, 6, 0, 0, 0),
            "GeoDegenerateVerticalLine" => (10, 3, 0, 0, 0),
            "GeoExactCircle" => (4, 2, 0, 0, 0),
            "GeoLatitudeZone" => (7, 4, 0, 0, 0),
            "GeoLongitudeSlice" => (7, 2, 0, 0, 0),
            "GeoNorthLatitudeZone" => (5, 2, 0, 0, 0),
            "GeoNorthRectangle" => (12, 4, 0, 0, 0),
            "GeoPoint" => (0, 6, 0, 0, 0),
            "GeoRectangle" => (16, 5, 0, 0, 0),
            "GeoS2Shape" => (14, 0, 0, 0, 0),
            "GeoSouthLatitudeZone" => (5, 2, 0, 0, 0),
            "GeoSouthRectangle" => (12, 4, 0, 0, 0),
            "GeoStandardCircle" => (4, 1, 0, 0, 0),
            "GeoStandardPath" => (4, 3, 0, 0, 1),
            "GeoWideDegenerateHorizontalLine" => (10, 3, 0, 0, 0),
            "GeoWideLongitudeSlice" => (6, 2, 0, 0, 0),
            "GeoWideNorthRectangle" => (12, 4, 0, 0, 0),
            "GeoWideRectangle" => (16, 5, 0, 0, 0),
            "GeoWideSouthRectangle" => (12, 4, 0, 0, 0),
            "GeoWorld" => (2, 0, 0, 0, 0),
            "LatLonBounds" => (4, 0, 0, 0, 3),
            "Plane" => (0, 4, 0, 0, 0),
            "PlanetModel" => (7, 17, 2, 0, 0),
            "SidedPlane" => (0, 5, 0, 0, 0),
            "StandardXYZSolid" => (14, 6, 0, 0, 7),
            "Vector" => (0, 3, 0, 0, 0),
            "XYZBounds" => (6, 0, 0, 0, 0),
            "XYdZSolid" => (8, 5, 0, 0, 0),
            "XdYZSolid" => (8, 5, 0, 0, 0),
            "XdYdZSolid" => (2, 4, 0, 0, 0),
            "dXYZSolid" => (8, 5, 0, 0, 0),
            "dXYdZSolid" => (2, 4, 0, 0, 0),
            "dXdYZSolid" => (2, 4, 0, 0, 0),
            "dXdYdZSolid" => (3, 3, 0, 0, 1),
            _ => return R::UNKNOWN_DEFAULT_RAM_BYTES_USED,
        };
    R::align_object_size(
        R::NUM_BYTES_OBJECT_HEADER + refs * R::NUM_BYTES_OBJECT_REF + 8 * d8 + 4 * d4 + 2 * d2 + d1,
    )
}
