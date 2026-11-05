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

//! Port of `org.apache.lucene.tests.util.RamUsageTester`: crawls an object
//! graph to collect its RAM usage, for testing.
//!
//! # The object model
//!
//! Java walks any object by reflection: its class (array or not), its
//! instance fields and their values. Rust has no reflection, so an object
//! the walker can measure describes itself through [`RamObject`]: what
//! `handleArray` and `handleOther` would find for its Java counterpart (a
//! [`JavaObject`]), which names the Java shape — a primitive or reference
//! array, one of the `SIMPLE_TYPES`, an enum constant, a `ByteBuffer`, a JDK
//! map or iterable, an ignorable JDK object, or a plain object with its
//! reference fields — and the Java layout sizes, computed from the
//! [`RamUsageEstimator`] model (compressed references, 12-byte object and
//! 16-byte array headers, 8-byte alignment). The walk itself — the explicit
//! stack, the identity set, the [`Accumulator`] — is Java's.
//!
//! **Divergence.** Every measured type must implement [`RamObject`]; the port
//! implements it for the primitive arrays (`Vec<u8>` … `Vec<f64>`, as
//! `byte[]` … `double[]`), `String` (`java.lang.String` on JDK 25: 24 bytes
//! plus `charArraySize(length())`, the `SIMPLE_TYPES` estimate), `BytesRef`,
//! and [`JavaObjectArray`] (`Object[]`). An object's identity is its address
//! together with its Java class name (a Rust struct and its first field share
//! an address; Java objects never do).

use std::collections::HashSet;

use rucene::util::{BytesRef, RamUsageEstimator};

/// A key and a value of a JDK map, `null` as `None`.
pub type MapEntry<'a> = (Option<&'a dyn RamObject>, Option<&'a dyn RamObject>);

/// What `RamUsageTester`'s walk sees of an object: its Java shape and sizes.
pub enum JavaObject<'a> {
    /// An array (`obClazz.isArray()`): its shallow size
    /// (`RamUsageEstimator.shallowSizeOf(array)`) and, for an array of
    /// references, its elements (`None` for a primitive array, whose values
    /// `handleArray` passes as an empty list).
    Array {
        /// `RamUsageEstimator.shallowSizeOf(ob)`.
        shallow_size: i64,
        /// The elements of a reference array, `null` as `None`.
        elements: Option<Vec<Option<&'a dyn RamObject>>>,
    },
    /// One of the `SIMPLE_TYPES` (`String`, `StringBuilder`, `BitSet`,
    /// `ByteArrayOutputStream`, `File`, `Path`, `ByteOrder`, `LongAdder`): the
    /// aligned shallow instance size and the estimate the type's function
    /// adds.
    Simple {
        /// `RamUsageEstimator.shallowSizeOf(ob)`.
        shallow_size: i64,
        /// `SIMPLE_TYPES.get(obClazz).applyAsLong(ob)`.
        estimate: i64,
    },
    /// A `CharsetEncoder`, `CharsetDecoder`, `ReentrantReadWriteLock` or
    /// `AtomicReference`: ignored, with a shallow size of 0.
    Ignorable,
    /// An `Enum` constant: its shallow size, not given to the accumulator.
    Enum {
        /// `RamUsageEstimator.shallowSizeOf(ob)`.
        shallow_size: i64,
    },
    /// A `ByteBuffer`, approximated by its underlying storage.
    ByteBuffer {
        /// `capacity()`.
        capacity: i32,
    },
    /// A `java.`* `Map`: its shallow size and its keys and values.
    JavaMap {
        /// `RamUsageEstimator.shallowSizeOf(ob)`.
        shallow_size: i64,
        /// Each entry's key and value, `null` as `None`.
        entries: Vec<MapEntry<'a>>,
    },
    /// A `java.`* `Iterable` (a JDK collection): its shallow size and its
    /// elements.
    JavaIterable {
        /// `RamUsageEstimator.shallowSizeOf(ob)`.
        shallow_size: i64,
        /// The elements, `null` as `None`.
        elements: Vec<Option<&'a dyn RamObject>>,
    },
    /// Any other object: its aligned shallow instance size (`ClassCache`'s,
    /// from every non-static field of the class and its superclasses) and the
    /// values of its reference fields.
    Object {
        /// `ClassCache.alignedShallowInstanceSize`.
        aligned_shallow_size: i64,
        /// Each reference field's name and value, `null` as `None`.
        fields: Vec<(&'static str, Option<&'a dyn RamObject>)>,
    },
}

/// An object `RamUsageTester` can measure: what Java's reflection would read
/// of its Java counterpart.
pub trait RamObject {
    /// `getClass().getName()` of the Java counterpart; with the address, the
    /// object's identity in the walk.
    fn java_class_name(&self) -> &'static str;

    /// The Java shape and sizes the walk reads.
    fn java_object(&self) -> JavaObject<'_>;
}

/// The value list `accumulateObject` and `accumulateArray` receive, and the
/// queue they push references to.
pub type Queue<'a> = Vec<Option<&'a dyn RamObject>>;

/// Port of `RamUsageTester.Accumulator`: an accumulator of object references,
/// which allows for customizing RAM usage estimation.
pub trait Accumulator {
    /// Accumulate transitive references for the provided fields of the given
    /// object into `queue` and return the shallow size of this object.
    fn accumulate_object<'a>(
        &mut self,
        o: &'a dyn RamObject,
        shallow_size: i64,
        field_values: &[(&'static str, Option<&'a dyn RamObject>)],
        queue: &mut Queue<'a>,
    ) -> i64 {
        let _ = o;
        queue.extend(field_values.iter().map(|(_, value)| *value));
        shallow_size
    }

    /// Accumulate transitive references for the provided values of the given
    /// array into `queue` and return the shallow size of this array.
    fn accumulate_array<'a>(
        &mut self,
        array: &'a dyn RamObject,
        shallow_size: i64,
        values: &[Option<&'a dyn RamObject>],
        queue: &mut Queue<'a>,
    ) -> i64 {
        let _ = array;
        queue.extend_from_slice(values);
        shallow_size
    }
}

/// `new RamUsageTester.Accumulator()`: the accumulator with the default
/// behaviour, which follows every reference.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultAccumulator;

impl Accumulator for DefaultAccumulator {}

/// Port of `org.apache.lucene.tests.util.RamUsageTester`.
pub struct RamUsageTester;

impl RamUsageTester {
    /// `ramUsed(obj, accumulator)`: estimates the RAM usage by the given
    /// object. It will walk the object tree and sum up all referenced
    /// objects.
    ///
    /// **Resource Usage:** This method internally uses a set of every object
    /// seen during traversals so it does allocate memory (it isn't side
    /// effect free).
    pub fn ram_used_with(obj: Option<&dyn RamObject>, accumulator: &mut dyn Accumulator) -> i64 {
        measure_object_size(obj, accumulator)
    }

    /// `ramUsed(obj)`: `ramUsed(obj, new Accumulator())`.
    pub fn ram_used(obj: &dyn RamObject) -> i64 {
        Self::ram_used_with(Some(obj), &mut DefaultAccumulator)
    }

    /// `humanSizeOf(object)`: a human-readable size of a given object
    /// (`RamUsageEstimator.humanReadableUnits(ramUsed(object))`).
    pub fn human_size_of(object: &dyn RamObject) -> String {
        RamUsageEstimator::human_readable_units(Self::ram_used(object))
    }
}

/// The identity of an object in the walk: its address and its Java class
/// (`IdentityHashMap`'s `==`).
fn identity(o: &dyn RamObject) -> (usize, &'static str) {
    (
        o as *const dyn RamObject as *const () as usize,
        o.java_class_name(),
    )
}

/// `measureObjectSize(root, accumulator)`: the non-recursive object descend,
/// with an explicit stack (recursion caused stack overflows on long chains).
fn measure_object_size(root: Option<&dyn RamObject>, accumulator: &mut dyn Accumulator) -> i64 {
    // Objects seen so far.
    let mut seen: HashSet<(usize, &'static str)> = HashSet::new();
    // Stack of objects pending traversal. Recursion caused stack overflows.
    let mut stack: Queue<'_> = vec![root];

    let mut total_size = 0i64;
    while let Some(ob) = stack.pop() {
        let Some(ob) = ob else {
            continue;
        };
        if !seen.insert(identity(ob)) {
            continue;
        }

        let ob_size = match ob.java_object() {
            JavaObject::Array {
                shallow_size,
                elements,
            } => handle_array(accumulator, &mut stack, ob, shallow_size, elements),
            other => handle_other(accumulator, &mut stack, ob, other),
        };
        total_size += ob_size;
    }
    total_size
}

/// `handleOther`: an object that is not an array.
fn handle_other<'a>(
    accumulator: &mut dyn Accumulator,
    stack: &mut Queue<'a>,
    ob: &'a dyn RamObject,
    shape: JavaObject<'a>,
) -> i64 {
    match shape {
        // Ignore JDK objects we can't access or handle properly.
        JavaObject::Ignorable => accumulator.accumulate_object(ob, 0, &[], stack),
        // some simple type like String where the size is easy to get from
        // public properties
        JavaObject::Simple {
            shallow_size,
            estimate,
        } => accumulator.accumulate_object(ob, shallow_size + estimate, &[], stack),
        JavaObject::Enum { shallow_size } => shallow_size,
        // Approximate ByteBuffers with their underlying storage (ignores
        // field overhead).
        JavaObject::ByteBuffer { capacity } => byte_array_size(capacity),
        JavaObject::JavaMap {
            shallow_size,
            entries,
        } => {
            let values: Vec<Option<&'a dyn RamObject>> =
                entries.into_iter().flat_map(|(k, v)| [k, v]).collect();
            accumulator.accumulate_array(
                ob,
                shallow_size + RamUsageEstimator::NUM_BYTES_ARRAY_HEADER,
                &values,
                stack,
            ) + RamUsageEstimator::NUM_BYTES_ARRAY_HEADER
        }
        JavaObject::JavaIterable {
            shallow_size,
            elements,
        } => {
            accumulator.accumulate_array(
                ob,
                shallow_size + RamUsageEstimator::NUM_BYTES_ARRAY_HEADER,
                &elements,
                stack,
            ) + RamUsageEstimator::NUM_BYTES_ARRAY_HEADER
        }
        JavaObject::Object {
            aligned_shallow_size,
            fields,
        } => accumulator.accumulate_object(ob, aligned_shallow_size, &fields, stack),
        JavaObject::Array { .. } => unreachable!("INVARIANT: arrays go to handle_array"),
    }
}

/// `handleArray`: an array, possibly of primitive types. Push any of its
/// references to the processing stack and accumulate this array's shallow
/// size.
fn handle_array<'a>(
    accumulator: &mut dyn Accumulator,
    stack: &mut Queue<'a>,
    ob: &'a dyn RamObject,
    shallow_size: i64,
    elements: Option<Vec<Option<&'a dyn RamObject>>>,
) -> i64 {
    let values = elements.unwrap_or_default();
    accumulator.accumulate_array(ob, shallow_size, &values, stack)
}

/// `byteArraySize(len)`.
fn byte_array_size(len: i32) -> i64 {
    RamUsageEstimator::align_object_size(RamUsageEstimator::NUM_BYTES_ARRAY_HEADER + i64::from(len))
}

/// `SIMPLE_TYPES`' `charArraySize(len)`.
pub fn char_array_size(len: i32) -> i64 {
    RamUsageEstimator::align_object_size(
        RamUsageEstimator::NUM_BYTES_ARRAY_HEADER + 2 * i64::from(len),
    )
}

/// `RamUsageEstimator.shallowSizeOf` of an array of `len` elements of
/// `element_bytes` each: `alignObjectSize(NUM_BYTES_ARRAY_HEADER + len *
/// element_bytes)`.
pub fn array_shallow_size(len: usize, element_bytes: i64) -> i64 {
    RamUsageEstimator::align_object_size(
        RamUsageEstimator::NUM_BYTES_ARRAY_HEADER + len as i64 * element_bytes,
    )
}

/// `ClassCache.alignedShallowInstanceSize` of a class whose non-static fields
/// (its own and its superclasses') have the given sizes:
/// `alignObjectSize(NUM_BYTES_OBJECT_HEADER + Σ adjustForField)`.
pub fn aligned_shallow_instance_size(field_sizes: &[i64]) -> i64 {
    RamUsageEstimator::align_object_size(
        field_sizes
            .iter()
            .fold(RamUsageEstimator::NUM_BYTES_OBJECT_HEADER, |size, field| {
                RamUsageEstimator::adjust_for_field(size, *field)
            }),
    )
}

macro_rules! primitive_array {
    ($t:ty, $class:literal, $bytes:expr) => {
        /// A Java primitive array of this element type.
        impl RamObject for Vec<$t> {
            fn java_class_name(&self) -> &'static str {
                $class
            }

            fn java_object(&self) -> JavaObject<'_> {
                JavaObject::Array {
                    shallow_size: array_shallow_size(self.len(), $bytes),
                    elements: None,
                }
            }
        }
    };
}

primitive_array!(u8, "[B", 1);
primitive_array!(i8, "[B", 1);
primitive_array!(bool, "[Z", 1);
primitive_array!(u16, "[C", 2);
primitive_array!(i16, "[S", 2);
primitive_array!(i32, "[I", 4);
primitive_array!(f32, "[F", 4);
primitive_array!(i64, "[J", 8);
primitive_array!(f64, "[D", 8);

/// `java.lang.String`, one of the `SIMPLE_TYPES`: its shallow size on JDK 25
/// (`value`, `coder`, `hash`, `hashIsZero`: 24 bytes) plus
/// `charArraySize(length())`, `length()` in UTF-16 code units ("may not be
/// correct with Java 9's compact strings!").
impl RamObject for String {
    fn java_class_name(&self) -> &'static str {
        "java.lang.String"
    }

    fn java_object(&self) -> JavaObject<'_> {
        let length = self.encode_utf16().count();
        JavaObject::Simple {
            shallow_size: RamUsageEstimator::STRING_SIZE,
            estimate: char_array_size(i32::try_from(length).unwrap_or(i32::MAX)),
        }
    }
}

/// `org.apache.lucene.util.BytesRef`: the fields `bytes` (a reference),
/// `offset` and `length` (`int`s), 24 bytes, and the `byte[]` it references.
impl RamObject for BytesRef {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.util.BytesRef"
    }

    fn java_object(&self) -> JavaObject<'_> {
        JavaObject::Object {
            aligned_shallow_size: aligned_shallow_instance_size(&[
                RamUsageEstimator::NUM_BYTES_OBJECT_REF,
                4,
                4,
            ]),
            fields: vec![("bytes", Some(&self.bytes as &dyn RamObject))],
        }
    }
}

/// A Java `Object[]`: references to other objects, `null` as `None`.
pub struct JavaObjectArray<'a>(pub Vec<Option<&'a dyn RamObject>>);

impl RamObject for JavaObjectArray<'_> {
    fn java_class_name(&self) -> &'static str {
        "[Ljava.lang.Object;"
    }

    fn java_object(&self) -> JavaObject<'_> {
        JavaObject::Array {
            shallow_size: array_shallow_size(self.0.len(), RamUsageEstimator::NUM_BYTES_OBJECT_REF),
            elements: Some(self.0.clone()),
        }
    }
}
