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

//! The types declared in `SerializableObject.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.
//!
//! The interface's static methods are the module's free functions. Java's
//! `InputStream` and `OutputStream` are [`Read`] and [`Write`]; a `Class`
//! object is its binary name, and the reflective constructor lookup of
//! `readObject` is the closed table of the package's serializable classes
//! (see `standard_objects::StandardObjects`).
//!
//! Java's `readClass` loads any class by name through
//! `MethodHandles.lookup().findClass` and checks it is a `SerializableObject`
//! (`SerializableObject.java` `readClass`), and `readObject` instantiates it
//! reflectively. Rust has no class path: a name resolves to the package's own
//! serializable classes or to a class registered with
//! [`register_class`](super::standard_objects::register_class) (the
//! port's class path), and any other name fails as Java's
//! `ClassNotFoundException` does. An `InvocationTargetException` is reported
//! with Java's message (`null` detail) but without its cause.

use std::any::Any;
use std::fmt;
use std::io::{Read, Write};
use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use super::geo_area_shape::GeoAreaShape;
use super::geo_membership_shape::GeoMembershipShape;
use super::geo_point::GeoPoint;
use super::geo_polygon::GeoPolygon;
use super::planet_model::PlanetModel;
use super::planet_object::PlanetObject;
use super::standard_objects::{self, StandardObjects};
use super::{ioe, JavaBitSet};

/// Indicates that a geo3d object can be serialized and deserialized.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.SerializableObject`.
///
/// Besides `write`, the trait carries the `java.lang.Object` behaviour the
/// package relies on: `getClass().getName()`, `equals(Object)` and
/// `hashCode()`; `toString()` is [`fmt::Display`]. The `into_*` methods are
/// Java's checked casts (`clazz.cast`, `instanceof`) of a deserialized object.
pub trait SerializableObject:
    SerializableObjectCasts + Send + Sync + fmt::Display + 'static
{
    /// `write(OutputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream; `UnsupportedOperationException` for an
    /// object that cannot be serialized.
    fn write(&self, output: &mut dyn Write) -> Result<()>;

    /// `getClass().getName()`.
    fn class_name(&self) -> &'static str;

    /// `equals(Object)`.
    fn java_equals(&self, o: &dyn Any) -> bool;

    /// `hashCode()`.
    fn java_hash_code(&self) -> i32;

    /// `(PlanetObject) this`, `None` when this is not a `PlanetObject`.
    fn into_planet_object(self: Arc<Self>) -> Option<Arc<dyn PlanetObject>> {
        None
    }

    /// `(GeoPolygon) this`, `None` when this is not a `GeoPolygon`.
    fn into_geo_polygon(self: Arc<Self>) -> Option<Arc<dyn GeoPolygon>> {
        None
    }

    /// `(GeoMembershipShape) this`, `None` when this is not one.
    fn into_geo_membership_shape(self: Arc<Self>) -> Option<Arc<dyn GeoMembershipShape>> {
        None
    }

    /// `(GeoAreaShape) this`, `None` when this is not one.
    fn into_geo_area_shape(self: Arc<Self>) -> Option<Arc<dyn GeoAreaShape>> {
        None
    }

    /// `(GeoBBox) this`, `None` when this is not a `GeoBBox`.
    fn into_geo_b_box(self: Arc<Self>) -> Option<Arc<dyn crate::geom::geo_b_box::GeoBBox>> {
        None
    }

    /// `(GeoCircle) this`, `None` when this is not a `GeoCircle`.
    fn into_geo_circle(self: Arc<Self>) -> Option<Arc<dyn crate::geom::geo_circle::GeoCircle>> {
        None
    }

    /// `(GeoPointShape) this`, `None` when this is not a `GeoPointShape`.
    fn into_geo_point_shape(
        self: Arc<Self>,
    ) -> Option<Arc<dyn crate::geom::geo_point_shape::GeoPointShape>> {
        None
    }
}

/// The casts every [`SerializableObject`] supports, implemented once for all
/// of them.
pub trait SerializableObjectCasts {
    /// The object as [`Any`], for `instanceof` checks.
    fn as_any(&self) -> &dyn Any;
    /// The shared object as [`Any`], for downcasts.
    fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
    /// The object as a [`SerializableObject`] trait object.
    fn as_serializable(&self) -> &dyn SerializableObject;
}

impl<T: SerializableObject> SerializableObjectCasts for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
    fn as_serializable(&self) -> &dyn SerializableObject {
        self
    }
}

/// `new ClassCastException` raised by `Class.cast`.
fn class_cast(from: &str, to: &str) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ClassCastException",
        format!("Cannot cast {} to {}", from, to),
    )
}

/// `writePlanetObject(OutputStream, PlanetObject)`.
///
/// # Errors
///
/// `IOException` from the stream.
pub fn write_planet_object(output: &mut dyn Write, object: &dyn PlanetObject) -> Result<()> {
    object.get_planet_model().write(output)?;
    write_object(output, object.as_serializable())
}

/// `readPlanetObject(InputStream)`.
///
/// # Errors
///
/// `IOException` from the stream or for a malformed one.
pub fn read_planet_object(input: &mut dyn Read) -> Result<Arc<dyn PlanetObject>> {
    let pm = Arc::new(PlanetModel::from_stream(input)?);
    let so = read_object_with_model(&pm, input)?;
    let class = so.class_name();
    match so.into_planet_object() {
        Some(po) => Ok(po),
        None => Err(ioe(format!(
            "Type of object is not expected PlanetObject: {}",
            class
        ))),
    }
}

/// `writeObject(OutputStream, SerializableObject)`.
///
/// # Errors
///
/// `IOException` from the stream.
pub fn write_object(output: &mut dyn Write, object: &dyn SerializableObject) -> Result<()> {
    write_class(output, object.class_name())?;
    object.write(output)
}

/// `readObject(PlanetModel, InputStream)`.
///
/// # Errors
///
/// `IOException` from the stream or for a malformed one.
pub fn read_object_with_model(
    planet_model: &Arc<PlanetModel>,
    input: &mut dyn Read,
) -> Result<Arc<dyn SerializableObject>> {
    let clazz = read_class(input)?;
    read_object_with_model_class(planet_model, input, &clazz)
}

/// `readObject(InputStream)`.
///
/// # Errors
///
/// `IOException` from the stream or for a malformed one.
pub fn read_object(input: &mut dyn Read) -> Result<Arc<dyn SerializableObject>> {
    let clazz = read_class(input)?;
    read_object_class(input, &clazz)
}

/// `readObject(PlanetModel, InputStream, Class)`: invokes the class's
/// `(PlanetModel, InputStream)` constructor.
///
/// # Errors
///
/// `IOException` wrapping what Java's reflection reports.
pub fn read_object_with_model_class(
    planet_model: &Arc<PlanetModel>,
    input: &mut dyn Read,
    clazz: &str,
) -> Result<Arc<dyn SerializableObject>> {
    match standard_objects::new_instance_with_model(clazz, planet_model, input) {
        standard_objects::Instantiation::Created(o) => Ok(o),
        standard_objects::Instantiation::NoSuchConstructor => Err(ioe(format!(
            "No such method exception for class {}: {}.<init>(org.apache.lucene.spatial3d.geom.PlanetModel,java.io.InputStream)",
            clazz, clazz
        ))),
        standard_objects::Instantiation::Failed(_) => Err(ioe(format!(
            "Exception instantiating class {}: null",
            clazz
        ))),
    }
}

/// `readObject(InputStream, Class)`: invokes the class's `(InputStream)`
/// constructor.
///
/// # Errors
///
/// `IOException` wrapping what Java's reflection reports.
pub fn read_object_class(input: &mut dyn Read, clazz: &str) -> Result<Arc<dyn SerializableObject>> {
    match standard_objects::new_instance(clazz, input) {
        standard_objects::Instantiation::Created(o) => Ok(o),
        standard_objects::Instantiation::NoSuchConstructor => Err(ioe(format!(
            "No such method exception for class {}: {}.<init>(java.io.InputStream)",
            clazz, clazz
        ))),
        standard_objects::Instantiation::Failed(_) => Err(ioe(format!(
            "Exception instantiating class {}: null",
            clazz
        ))),
    }
}

/// `writeClass(OutputStream, Class)`.
///
/// # Errors
///
/// `IOException` from the stream.
pub fn write_class(output: &mut dyn Write, clazz: &str) -> Result<()> {
    match StandardObjects::code_of(clazz) {
        None => {
            write_boolean(output, false)?;
            write_string(output, clazz)
        }
        Some(index) => {
            write_boolean(output, true)?;
            write_byte(output, index)
        }
    }
}

/// `readClass(InputStream)`.
fn read_class(input: &mut dyn Read) -> Result<String> {
    let standard = read_boolean(input)?;
    if standard {
        let index = read_byte(input)?;
        match StandardObjects::class_of(index) {
            Some(name) => Ok(name.to_string()),
            None => Err(ioe(format!(
                "No standard object found for index: {}",
                index
            ))),
        }
    } else {
        let class_name = read_string(input)?;
        // Load without initializing and confirm the named class is actually a
        // SerializableObject before it can be instantiated, so a crafted
        // stream cannot load arbitrary classes.
        let (class, message) = match standard_objects::find_class(&class_name) {
            standard_objects::ClassLookup::Found => return Ok(class_name),
            // `ClassNotFoundException.getMessage()` is the class name.
            standard_objects::ClassLookup::NotFound => {
                ("java.lang.ClassNotFoundException", class_name.clone())
            }
            // `Class.asSubclass` throws `new ClassCastException(this.toString())`.
            standard_objects::ClassLookup::NotSerializable => (
                "java.lang.ClassCastException",
                format!("class {}", class_name),
            ),
        };
        Err(LuceneError::from(
            rucene::store::java_nio_file::io_exception_with_cause(
                format!(
                    "Can't find or access class of correct type for deserialization: {}",
                    message
                ),
                Some(LuceneError::unchecked(class, message)),
            ),
        ))
    }
}

/// `writeString(OutputStream, String)`.
///
/// # Errors
///
/// `IOException` from the stream.
pub fn write_string(output: &mut dyn Write, value: &str) -> Result<()> {
    write_byte_array(output, value.as_bytes())
}

/// `readString(InputStream)`: malformed UTF-8 is replaced as `new
/// String(byte[], UTF_8)` does.
///
/// # Errors
///
/// `IOException` from the stream.
pub fn read_string(input: &mut dyn Read) -> Result<String> {
    Ok(rucene::util::java_fmt::java_utf8_vec_to_string(
        read_byte_array(input)?,
    ))
}

/// `writePointArray(OutputStream, GeoPoint[])` and `(OutputStream,
/// List<GeoPoint>)`.
///
/// # Errors
///
/// `IOException` from the stream.
pub fn write_point_array(output: &mut dyn Write, values: &[GeoPoint]) -> Result<()> {
    write_homogeneous_array(output, values)
}

/// `readPointArray(PlanetModel, InputStream)`.
///
/// # Errors
///
/// `IOException` from the stream or for a malformed one.
pub fn read_point_array(
    planet_model: &Arc<PlanetModel>,
    input: &mut dyn Read,
) -> Result<Vec<GeoPoint>> {
    read_homogeneous_array(
        planet_model,
        input,
        "org.apache.lucene.spatial3d.geom.GeoPoint",
        downcast_point,
    )
}

/// `(GeoPoint) o`.
pub(crate) fn downcast_point(o: Arc<dyn SerializableObject>) -> Result<GeoPoint> {
    let class = o.class_name();
    match o.into_any_arc().downcast::<GeoPoint>() {
        Ok(p) => Ok(Arc::unwrap_or_clone(p)),
        Err(_) => Err(class_cast(
            class,
            "org.apache.lucene.spatial3d.geom.GeoPoint",
        )),
    }
}

/// `writePolygonArray(OutputStream, GeoPolygon[])` and `(OutputStream,
/// List<GeoPolygon>)`.
///
/// # Errors
///
/// `IOException` from the stream.
pub fn write_polygon_array(output: &mut dyn Write, values: &[Arc<dyn GeoPolygon>]) -> Result<()> {
    write_int(output, values.len() as i32)?;
    for value in values {
        write_object(output, value.as_serializable())?;
    }
    Ok(())
}

/// `readPolygonArray(PlanetModel, InputStream)`.
///
/// # Errors
///
/// `IOException` from the stream or for a malformed one;
/// `ClassCastException` for an element that is not a `GeoPolygon`.
pub fn read_polygon_array(
    planet_model: &Arc<PlanetModel>,
    input: &mut dyn Read,
) -> Result<Vec<Arc<dyn GeoPolygon>>> {
    read_heterogeneous_array(
        planet_model,
        input,
        "org.apache.lucene.spatial3d.geom.GeoPolygon",
        |o| o.into_geo_polygon(),
    )
}

/// Write an array: the count, then each object's own `write` (no class
/// names).
///
/// Equivalent to `SerializableObject.writeHomogeneousArray(OutputStream, SerializableObject[])`
/// and its `(OutputStream, List<? extends SerializableObject>)` overload. A
/// Java `null` array writes the count 0, which is the empty slice here.
///
/// # Errors
///
/// `IOException` from the stream.
pub fn write_homogeneous_array<T: SerializableObject>(
    output: &mut dyn Write,
    values: &[T],
) -> Result<()> {
    write_int(output, values.len() as i32)?;
    for value in values {
        value.write(output)?;
    }
    Ok(())
}

/// Read an array: the count, then each object through `clazz`'s
/// `(PlanetModel, InputStream)` constructor; `cast` is `clazz.cast`.
///
/// Equivalent to `SerializableObject.readHomogeneousArray(PlanetModel, InputStream, Class)`.
///
/// # Errors
///
/// `IOException` from the stream or for a malformed one;
/// `ClassCastException` from `cast`.
pub fn read_homogeneous_array<T>(
    planet_model: &Arc<PlanetModel>,
    input: &mut dyn Read,
    clazz: &str,
    cast: impl Fn(Arc<dyn SerializableObject>) -> Result<T>,
) -> Result<Vec<T>> {
    let count = read_int(input)?;
    let mut rval = new_array(count)?;
    for _ in 0..count {
        rval.push(cast(read_object_with_model_class(
            planet_model,
            input,
            clazz,
        )?)?);
    }
    Ok(rval)
}

/// `writeHeterogeneousArray(OutputStream, SerializableObject[])`.
///
/// # Errors
///
/// `IOException` from the stream.
pub fn write_heterogeneous_array(
    output: &mut dyn Write,
    values: &[&dyn SerializableObject],
) -> Result<()> {
    write_int(output, values.len() as i32)?;
    for value in values {
        write_object(output, *value)?;
    }
    Ok(())
}

/// `readHeterogeneousArray(PlanetModel, InputStream, Class<T>)`: `cast` is
/// `clazz.cast`, `None` when the object is not a `T`.
///
/// # Errors
///
/// `IOException` from the stream or for a malformed one;
/// `ClassCastException` for an element that is not a `T`.
pub fn read_heterogeneous_array<T: ?Sized>(
    planet_model: &Arc<PlanetModel>,
    input: &mut dyn Read,
    clazz: &str,
    cast: impl Fn(Arc<dyn SerializableObject>) -> Option<Arc<T>>,
) -> Result<Vec<Arc<T>>> {
    let count = read_int(input)?;
    let mut rval = new_array(count)?;
    for _ in 0..count {
        let o = read_object_with_model(planet_model, input)?;
        let class = o.class_name();
        match cast(o) {
            Some(t) => rval.push(t),
            None => return Err(class_cast(class, clazz)),
        }
    }
    Ok(rval)
}

/// `Array.newInstance(clazz, count)`.
fn new_array<T>(count: i32) -> Result<Vec<T>> {
    if count < 0 {
        return Err(LuceneError::unchecked(
            "java.lang.NegativeArraySizeException",
            count.to_string(),
        ));
    }
    Ok(Vec::with_capacity((count as usize).min(1 << 16)))
}

/// `writeBitSet(OutputStream, BitSet)`.
pub(crate) fn write_bit_set(output: &mut dyn Write, bit_set: &JavaBitSet) -> Result<()> {
    write_byte_array(output, &bit_set.to_byte_array())
}

/// `readBitSet(InputStream)`.
pub(crate) fn read_bit_set(input: &mut dyn Read) -> Result<JavaBitSet> {
    Ok(JavaBitSet::value_of(&read_byte_array(input)?))
}

/// `writeByteArray(OutputStream, byte[])`.
///
/// # Errors
///
/// `IOException` from the stream.
pub fn write_byte_array(output: &mut dyn Write, bytes: &[u8]) -> Result<()> {
    write_int(output, bytes.len() as i32)?;
    output.write_all(bytes)?;
    Ok(())
}

/// `readByteArray(InputStream)`.
///
/// # Errors
///
/// `IOException` from the stream or at its premature end.
pub fn read_byte_array(input: &mut dyn Read) -> Result<Vec<u8>> {
    let mut string_length = read_int(input)?;
    if string_length < 0 {
        return Err(LuceneError::unchecked(
            "java.lang.NegativeArraySizeException",
            string_length.to_string(),
        ));
    }
    let mut string_offset = 0usize;
    let mut bytes = vec![0u8; string_length as usize];
    while string_length > 0 {
        let amt = input.read(&mut bytes[string_offset..string_offset + string_length as usize])?;
        if amt == 0 {
            return Err(ioe("Unexpected end of input stream"));
        }
        string_offset += amt;
        string_length -= amt as i32;
    }
    Ok(bytes)
}

/// `writeDouble(OutputStream, double)`.
///
/// # Errors
///
/// `IOException` from the stream.
pub fn write_double(output: &mut dyn Write, value: f64) -> Result<()> {
    // `Double.doubleToLongBits` collapses every NaN.
    let bits = if value.is_nan() {
        0x7ff8_0000_0000_0000u64 as i64
    } else {
        value.to_bits() as i64
    };
    write_long(output, bits)
}

/// `readDouble(InputStream)`.
///
/// # Errors
///
/// `IOException` from the stream.
pub fn read_double(input: &mut dyn Read) -> Result<f64> {
    Ok(f64::from_bits(read_long(input)? as u64))
}

/// `writeLong(OutputStream, long)`.
///
/// # Errors
///
/// `IOException` from the stream.
pub fn write_long(output: &mut dyn Write, value: i64) -> Result<()> {
    write_int(output, value as i32)?;
    write_int(output, (value >> 32) as i32)
}

/// `readLong(InputStream)`.
///
/// # Errors
///
/// `IOException` from the stream.
pub fn read_long(input: &mut dyn Read) -> Result<i64> {
    let lower = (read_int(input)? as i64) & 0x0000_0000_ffff_ffff;
    let upper = ((read_int(input)? as i64) << 32) & (0xffff_ffff_0000_0000u64 as i64);
    Ok(lower.wrapping_add(upper))
}

/// `writeInt(OutputStream, int)`.
///
/// # Errors
///
/// `IOException` from the stream.
pub fn write_int(output: &mut dyn Write, value: i32) -> Result<()> {
    write_byte(output, value)?;
    write_byte(output, value >> 8)?;
    write_byte(output, value >> 16)?;
    write_byte(output, value >> 24)
}

/// `readInt(InputStream)`: like Java, the end of the stream (`read()` of
/// `-1`) is not detected.
///
/// # Errors
///
/// `IOException` from the stream.
pub fn read_int(input: &mut dyn Read) -> Result<i32> {
    let l1 = read_byte(input)? & 0x0000_00ff;
    let l2 = (read_byte(input)? << 8) & 0x0000_ff00;
    let l3 = (read_byte(input)? << 16) & 0x00ff_0000;
    let l4 = (read_byte(input)? << 24) & (0xff00_0000u32 as i32);
    Ok(l1.wrapping_add(l2).wrapping_add(l3).wrapping_add(l4))
}

/// `writeBoolean(OutputStream, boolean)`.
///
/// # Errors
///
/// `IOException` from the stream.
pub fn write_boolean(output: &mut dyn Write, value: bool) -> Result<()> {
    write_byte(output, if value { 1 } else { 0 })
}

/// `readBoolean(InputStream)`.
///
/// # Errors
///
/// `IOException` from the stream or at its end.
pub fn read_boolean(input: &mut dyn Read) -> Result<bool> {
    let value_read = read_byte(input)?;
    if value_read == -1 {
        return Err(ioe("Unexpected end of input stream"));
    }
    Ok(value_read != 0)
}

/// `OutputStream.write(int)`: the low byte.
fn write_byte(output: &mut dyn Write, value: i32) -> Result<()> {
    output.write_all(&[value as u8])?;
    Ok(())
}

/// `InputStream.read()`: the next byte, `-1` at the end of the stream.
fn read_byte(input: &mut dyn Read) -> Result<i32> {
    let mut b = [0u8; 1];
    loop {
        match input.read(&mut b) {
            Ok(0) => return Ok(-1),
            Ok(_) => return Ok(b[0] as i32),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
}
