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

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use rucene::index::Term;
use rucene::search::{
    BooleanClause, BooleanQuery, DisjunctionMaxQuery, Occur, PhraseQuery, Query, TermQuery,
};
use rucene::util::{ByteBlockPool, BytesRef, BytesRefHash, RamUsageEstimator};

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

// ---------------------------------------------------------------------------
// Declared Java object graphs
// ---------------------------------------------------------------------------
//
// Java's walk reads any object by reflection, including the JDK containers a
// Lucene class holds (`BooleanQuery.clauseSets`, an `EnumMap` of `Multiset`s
// and `HashSet`s) and fields Rust keeps in another shape. A Rucene object
// whose Java counterpart cannot be read off its Rust fields is measured
// through a [`JavaNode`] graph instead: [`ModelBuilder`] declares, class by
// class, the fields Java's reflection would find (the declared list of
// `ClassCache.alignedShallowInstanceSize` and `referenceFields`), and builds
// one node per Java object. A Rust object reached twice yields the same node,
// so the walk's identity set sees one object, as Java's does.

/// A reference held by a [`JavaNode`].
pub enum NodeChild {
    /// A `null` reference.
    Null,
    /// A reference to another object.
    Node(Rc<JavaNode>),
    /// A reference from the object to itself (`map.put("self", map)`).
    This,
}

/// The Java shape of a [`JavaNode`]: the cases of [`JavaObject`], over
/// [`NodeChild`] references.
pub enum NodeShape {
    /// See [`JavaObject::Array`].
    Array {
        /// `RamUsageEstimator.shallowSizeOf(ob)`.
        shallow_size: i64,
        /// The elements of a reference array; `None` for a primitive array.
        elements: Option<Vec<NodeChild>>,
    },
    /// See [`JavaObject::Simple`].
    Simple {
        /// `RamUsageEstimator.shallowSizeOf(ob)`.
        shallow_size: i64,
        /// `SIMPLE_TYPES.get(obClazz).applyAsLong(ob)`.
        estimate: i64,
    },
    /// See [`JavaObject::Enum`].
    Enum {
        /// `RamUsageEstimator.shallowSizeOf(ob)`.
        shallow_size: i64,
    },
    /// See [`JavaObject::JavaMap`].
    JavaMap {
        /// `RamUsageEstimator.shallowSizeOf(ob)`.
        shallow_size: i64,
        /// Each entry's key and value.
        entries: Vec<(NodeChild, NodeChild)>,
    },
    /// See [`JavaObject::JavaIterable`].
    JavaIterable {
        /// `RamUsageEstimator.shallowSizeOf(ob)`.
        shallow_size: i64,
        /// The elements.
        elements: Vec<NodeChild>,
    },
    /// See [`JavaObject::Object`].
    Object {
        /// `ClassCache.alignedShallowInstanceSize`.
        aligned_shallow_size: i64,
        /// Each reference field's name and value.
        fields: Vec<(&'static str, NodeChild)>,
    },
}

/// One Java object of a declared graph.
pub struct JavaNode {
    class_name: &'static str,
    shape: NodeShape,
}

impl JavaNode {
    /// Creates a node of the Java class `class_name`.
    pub fn new(class_name: &'static str, shape: NodeShape) -> Rc<Self> {
        Rc::new(Self { class_name, shape })
    }
}

impl JavaNode {
    fn child<'a>(&'a self, child: &'a NodeChild) -> Option<&'a dyn RamObject> {
        match child {
            NodeChild::Null => None,
            NodeChild::Node(node) => Some(&**node as &dyn RamObject),
            NodeChild::This => Some(self as &dyn RamObject),
        }
    }
}

impl RamObject for JavaNode {
    fn java_class_name(&self) -> &'static str {
        self.class_name
    }

    fn java_object(&self) -> JavaObject<'_> {
        match &self.shape {
            NodeShape::Array {
                shallow_size,
                elements,
            } => JavaObject::Array {
                shallow_size: *shallow_size,
                elements: elements
                    .as_ref()
                    .map(|elements| elements.iter().map(|e| self.child(e)).collect()),
            },
            NodeShape::Simple {
                shallow_size,
                estimate,
            } => JavaObject::Simple {
                shallow_size: *shallow_size,
                estimate: *estimate,
            },
            NodeShape::Enum { shallow_size } => JavaObject::Enum {
                shallow_size: *shallow_size,
            },
            NodeShape::JavaMap {
                shallow_size,
                entries,
            } => JavaObject::JavaMap {
                shallow_size: *shallow_size,
                entries: entries
                    .iter()
                    .map(|(k, v)| (self.child(k), self.child(v)))
                    .collect(),
            },
            NodeShape::JavaIterable {
                shallow_size,
                elements,
            } => JavaObject::JavaIterable {
                shallow_size: *shallow_size,
                elements: elements.iter().map(|e| self.child(e)).collect(),
            },
            NodeShape::Object {
                aligned_shallow_size,
                fields,
            } => JavaObject::Object {
                aligned_shallow_size: *aligned_shallow_size,
                fields: fields
                    .iter()
                    .map(|(name, value)| (*name, self.child(value)))
                    .collect(),
            },
        }
    }
}

/// A reference field's width: `NUM_BYTES_OBJECT_REF`.
const REF: i64 = RamUsageEstimator::NUM_BYTES_OBJECT_REF;

/// Builds [`JavaNode`] graphs over Rucene objects, one node per Java object.
#[derive(Default)]
pub struct ModelBuilder {
    /// The node built for each Rust object, keyed by its address and Java
    /// class.
    seen: HashMap<(usize, &'static str), Rc<JavaNode>>,
}

impl ModelBuilder {
    /// Creates a builder with no node yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the node of the Rust object at `address`, building it with
    /// `build` the first time.
    pub fn shared(
        &mut self,
        address: *const (),
        class_name: &'static str,
        build: impl FnOnce(&mut Self) -> NodeShape,
    ) -> Rc<JavaNode> {
        let key = (address as usize, class_name);
        if let Some(node) = self.seen.get(&key) {
            return Rc::clone(node);
        }
        let shape = build(self);
        let node = JavaNode::new(class_name, shape);
        self.seen.insert(key, Rc::clone(&node));
        node
    }

    /// `java.lang.String`: see the [`RamObject`] implementation for `String`.
    pub fn string(&mut self, s: &str) -> Rc<JavaNode> {
        self.shared(s.as_ptr() as *const (), "java.lang.String", |_| {
            let length = s.encode_utf16().count();
            NodeShape::Simple {
                shallow_size: RamUsageEstimator::STRING_SIZE,
                estimate: char_array_size(i32::try_from(length).unwrap_or(i32::MAX)),
            }
        })
    }

    /// A primitive array of `len` elements of `element_bytes` each, held at
    /// `address`.
    pub fn primitive_array(
        &mut self,
        address: *const (),
        class_name: &'static str,
        len: usize,
        element_bytes: i64,
    ) -> Rc<JavaNode> {
        self.shared(address, class_name, |_| NodeShape::Array {
            shallow_size: array_shallow_size(len, element_bytes),
            elements: None,
        })
    }

    /// `java.lang.Integer`: the field `int value`, 16 bytes.
    /// `Integer.valueOf` hands out the cached instance for `-128..=127`, so
    /// those values share one node per value; any other is a fresh object.
    pub fn integer(&mut self, value: i32) -> Rc<JavaNode> {
        let shape = || NodeShape::Object {
            aligned_shallow_size: aligned_shallow_instance_size(&[4]),
            fields: Vec::new(),
        };
        if (-128..=127).contains(&value) {
            // The `IntegerCache` entry: one key per cached value.
            self.shared(
                (value as isize as usize) as *const (),
                "java.lang.Integer",
                |_| shape(),
            )
        } else {
            JavaNode::new("java.lang.Integer", shape())
        }
    }

    /// `java.lang.Long`: the field `long value`, 24 bytes, a fresh object
    /// outside the `LongCache` range `-128..=127`.
    pub fn long(&mut self, value: i64) -> Rc<JavaNode> {
        let shape = || NodeShape::Object {
            aligned_shallow_size: aligned_shallow_instance_size(&[8]),
            fields: Vec::new(),
        };
        if (-128..=127).contains(&value) {
            self.shared(
                (value as isize as usize) as *const (),
                "java.lang.Long",
                |_| shape(),
            )
        } else {
            JavaNode::new("java.lang.Long", shape())
        }
    }

    /// `org.apache.lucene.util.BytesRef`: see the [`RamObject`]
    /// implementation for `BytesRef`.
    pub fn bytes_ref(&mut self, bytes: &BytesRef) -> Rc<JavaNode> {
        self.shared(
            bytes as *const BytesRef as *const (),
            "org.apache.lucene.util.BytesRef",
            |b| {
                let array = b.primitive_array(
                    bytes.bytes.as_ptr() as *const (),
                    "[B",
                    bytes.bytes.len(),
                    1,
                );
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[REF, 4, 4]),
                    fields: vec![("bytes", NodeChild::Node(array))],
                }
            },
        )
    }

    /// `org.apache.lucene.index.Term`: the fields `String field` and
    /// `BytesRef bytes`.
    pub fn term(&mut self, term: &Term) -> Rc<JavaNode> {
        self.shared(
            term as *const Term as *const (),
            "org.apache.lucene.index.Term",
            |b| {
                let field = b.string(term.field());
                let bytes = b.bytes_ref(term.bytes());
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[REF, REF]),
                    fields: vec![
                        ("field", NodeChild::Node(field)),
                        ("bytes", NodeChild::Node(bytes)),
                    ],
                }
            },
        )
    }

    /// A `BooleanClause.Occur` constant: an `Enum` with the fields `String
    /// name` and `int ordinal`. Constants are singletons, one node each.
    pub fn occur(&mut self, occur: Occur) -> Rc<JavaNode> {
        self.shared(
            (occur as usize + 1) as *const (),
            "org.apache.lucene.search.BooleanClause$Occur",
            |_| NodeShape::Enum {
                shallow_size: aligned_shallow_instance_size(&[REF, 4]),
            },
        )
    }

    /// `java.util.HashSet` of `elements`: the field `HashMap map`; a JDK
    /// iterable, so the walk reads its elements, not its backing map.
    pub fn hash_set(&mut self, elements: Vec<NodeChild>) -> Rc<JavaNode> {
        JavaNode::new(
            "java.util.HashSet",
            NodeShape::JavaIterable {
                shallow_size: aligned_shallow_instance_size(&[REF]),
                elements,
            },
        )
    }

    /// `java.util.HashMap` of `entries`: the fields `table`, `entrySet`,
    /// `size`, `modCount`, `threshold`, `loadFactor` and `AbstractMap`'s
    /// `keySet` and `values`; a JDK map, so the walk reads its keys and
    /// values, not its nodes.
    pub fn hash_map(&mut self, entries: Vec<(NodeChild, NodeChild)>) -> Rc<JavaNode> {
        JavaNode::new(
            "java.util.HashMap",
            NodeShape::JavaMap {
                shallow_size: HASH_MAP_SHALLOW_SIZE,
                entries,
            },
        )
    }

    /// `org.apache.lucene.search.Multiset` of `queries`: the fields
    /// `Map<T, Integer> map` (a `HashMap` from each distinct element to its
    /// count) and `int size`.
    pub fn query_multiset(&mut self, queries: &[&Arc<dyn Query>]) -> Rc<JavaNode> {
        let mut counts: Vec<(&Arc<dyn Query>, i32)> = Vec::new();
        for query in queries {
            match counts.iter_mut().find(|(q, _)| q.query_eq(query.as_ref())) {
                Some((_, count)) => *count += 1,
                None => counts.push((query, 1)),
            }
        }
        let entries = counts
            .into_iter()
            .map(|(q, count)| {
                (
                    NodeChild::Node(self.query(q.as_ref())),
                    NodeChild::Node(self.integer(count)),
                )
            })
            .collect();
        let map = self.hash_map(entries);
        JavaNode::new(
            "org.apache.lucene.search.Multiset",
            NodeShape::Object {
                aligned_shallow_size: aligned_shallow_instance_size(&[REF, 4]),
                fields: vec![("map", NodeChild::Node(map))],
            },
        )
    }

    /// A `HashSet<Query>` of `queries`, deduplicated by `equals`.
    pub fn query_hash_set(&mut self, queries: &[&Arc<dyn Query>]) -> Rc<JavaNode> {
        let mut distinct: Vec<&Arc<dyn Query>> = Vec::new();
        for query in queries {
            if !distinct.iter().any(|q| q.query_eq(query.as_ref())) {
                distinct.push(query);
            }
        }
        let elements = distinct
            .into_iter()
            .map(|q| NodeChild::Node(self.query(q.as_ref())))
            .collect();
        self.hash_set(elements)
    }

    /// A query, by its Java class.
    ///
    /// Declares `TermQuery`, `PhraseQuery`, `BooleanQuery` and
    /// `DisjunctionMaxQuery`; any other query is measured by its own shallow
    /// size (`RamUsageEstimator.shallowSizeOf`), its references not followed.
    pub fn query(&mut self, query: &dyn Query) -> Rc<JavaNode> {
        let address = query as *const dyn Query as *const ();
        let any = query.as_any();
        if let Some(q) = any.downcast_ref::<TermQuery>() {
            return self.shared(address, "org.apache.lucene.search.TermQuery", |b| {
                let term = b.term(q.get_term());
                let states = match q.get_term_states() {
                    // `TermStates`: the fields `topReaderContextIdentity`,
                    // `states`, `docFreq` and `totalTermFreq`.
                    Some(states) => NodeChild::Node(b.shared(
                        Arc::as_ptr(states) as *const (),
                        "org.apache.lucene.index.TermStates",
                        |_| NodeShape::Object {
                            aligned_shallow_size: aligned_shallow_instance_size(&[REF, REF, 4, 8]),
                            fields: Vec::new(),
                        },
                    )),
                    None => NodeChild::Null,
                };
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[REF, REF]),
                    fields: vec![
                        ("term", NodeChild::Node(term)),
                        ("perReaderTermState", states),
                    ],
                }
            });
        }
        if let Some(q) = any.downcast_ref::<PhraseQuery>() {
            return self.shared(address, "org.apache.lucene.search.PhraseQuery", |b| {
                let field = match q.get_field() {
                    Some(field) => NodeChild::Node(b.string(field)),
                    None => NodeChild::Null,
                };
                let terms: Vec<NodeChild> = q
                    .get_terms()
                    .iter()
                    .map(|t| NodeChild::Node(b.term(t)))
                    .collect();
                let terms = b.shared(
                    q.get_terms().as_ptr() as *const (),
                    "[Lorg.apache.lucene.index.Term;",
                    |_| NodeShape::Array {
                        shallow_size: array_shallow_size(terms.len(), REF),
                        elements: Some(terms),
                    },
                );
                let positions = b.primitive_array(
                    q.get_positions().as_ptr() as *const (),
                    "[I",
                    q.get_positions().len(),
                    4,
                );
                // `int slop`, `String field`, `Term[] terms`, `int[] positions`.
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[4, REF, REF, REF]),
                    fields: vec![
                        ("field", field),
                        ("terms", NodeChild::Node(terms)),
                        ("positions", NodeChild::Node(positions)),
                    ],
                }
            });
        }
        if let Some(q) = any.downcast_ref::<DisjunctionMaxQuery>() {
            return self.shared(
                address,
                "org.apache.lucene.search.DisjunctionMaxQuery",
                |b| {
                    let disjuncts: Vec<&Arc<dyn Query>> = q.get_disjuncts().iter().collect();
                    let multiset = b.query_multiset(&disjuncts);
                    // `Multiset<Query> disjuncts`, `float tieBreakerMultiplier`.
                    NodeShape::Object {
                        aligned_shallow_size: aligned_shallow_instance_size(&[REF, 4]),
                        fields: vec![("disjuncts", NodeChild::Node(multiset))],
                    }
                },
            );
        }
        if let Some(q) = any.downcast_ref::<BooleanQuery>() {
            return self.shared(address, "org.apache.lucene.search.BooleanQuery", |b| {
                let clauses: Vec<NodeChild> = q
                    .clauses()
                    .iter()
                    .map(|c| NodeChild::Node(b.boolean_clause(c)))
                    .collect();
                // `Collections.unmodifiableList(Arrays.asList(clauses))`: the
                // fields `c` and `list`; a JDK iterable.
                let clauses = JavaNode::new(
                    "java.util.Collections$UnmodifiableRandomAccessList",
                    NodeShape::JavaIterable {
                        shallow_size: aligned_shallow_instance_size(&[REF, REF]),
                        elements: clauses,
                    },
                );
                // `clauseSets = new EnumMap<>(Occur.class)` holding, in key
                // order, a `Multiset` for SHOULD and MUST and a `HashSet` for
                // FILTER and MUST_NOT, each with the clauses' queries.
                let of = |occur: Occur| -> Vec<&Arc<dyn Query>> {
                    q.clauses()
                        .iter()
                        .filter(|c| c.occur() == occur)
                        .map(|c| c.query())
                        .collect()
                };
                let mut entries = Vec::new();
                for occur in [Occur::MUST, Occur::FILTER, Occur::SHOULD, Occur::MUST_NOT] {
                    let key = NodeChild::Node(b.occur(occur));
                    let value = match occur {
                        Occur::SHOULD | Occur::MUST => b.query_multiset(&of(occur)),
                        Occur::FILTER | Occur::MUST_NOT => b.query_hash_set(&of(occur)),
                    };
                    entries.push((key, NodeChild::Node(value)));
                }
                // `EnumMap`: the fields `keyType`, `keyUniverse`, `vals`,
                // `size`, `entrySet` and `AbstractMap`'s `keySet` and
                // `values`; a JDK map.
                let clause_sets = JavaNode::new(
                    "java.util.EnumMap",
                    NodeShape::JavaMap {
                        shallow_size: aligned_shallow_instance_size(&[
                            REF, REF, REF, 4, REF, REF, REF,
                        ]),
                        entries,
                    },
                );
                // `int minimumNumberShouldMatch`, `List clauses`, `Map
                // clauseSets`, `int hashCode`.
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[4, REF, REF, 4]),
                    fields: vec![
                        ("clauses", NodeChild::Node(clauses)),
                        ("clauseSets", NodeChild::Node(clause_sets)),
                    ],
                }
            });
        }
        self.shared(address, "org.apache.lucene.search.Query", |_| {
            NodeShape::Object {
                aligned_shallow_size: RamUsageEstimator::shallow_size_of_object(query),
                fields: Vec::new(),
            }
        })
    }

    /// `org.apache.lucene.search.BooleanClause`, a record of `Query query`
    /// and `Occur occur`.
    pub fn boolean_clause(&mut self, clause: &BooleanClause) -> Rc<JavaNode> {
        self.shared(
            clause as *const BooleanClause as *const (),
            "org.apache.lucene.search.BooleanClause",
            |b| {
                let query = b.query(clause.query().as_ref());
                let occur = b.occur(clause.occur());
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[REF, REF]),
                    fields: vec![
                        ("query", NodeChild::Node(query)),
                        ("occur", NodeChild::Node(occur)),
                    ],
                }
            },
        )
    }

    /// `new BytesRefHash()`: a `BytesRefHash` over its own `ByteBlockPool`
    /// with a `DirectAllocator`, a `DirectBytesStartArray` and that array's
    /// `Counter`.
    pub fn bytes_ref_hash(&mut self, hash: &BytesRefHash, pool: &ByteBlockPool) -> Rc<JavaNode> {
        let bytes_start = hash.bytes_start().map(|starts| {
            self.primitive_array(starts.as_ptr() as *const (), "[I", starts.len(), 4)
        });
        let ids = if hash.ids().is_empty() {
            NodeChild::Null
        } else {
            NodeChild::Node(self.primitive_array(
                hash.ids().as_ptr() as *const (),
                "[I",
                hash.ids().len(),
                4,
            ))
        };
        // `Counter.newCounter()`: a `SerialCounter` with `long count`.
        let counter = JavaNode::new(
            "org.apache.lucene.util.Counter$SerialCounter",
            NodeShape::Object {
                aligned_shallow_size: aligned_shallow_instance_size(&[8]),
                fields: Vec::new(),
            },
        );
        let as_child = |node: &Option<Rc<JavaNode>>| match node {
            Some(node) => NodeChild::Node(Rc::clone(node)),
            None => NodeChild::Null,
        };
        // `DirectBytesStartArray`: `int initSize`, `int[] bytesStart`,
        // `Counter bytesUsed`.
        let bytes_start_array = JavaNode::new(
            "org.apache.lucene.util.BytesRefHash$DirectBytesStartArray",
            NodeShape::Object {
                aligned_shallow_size: aligned_shallow_instance_size(&[4, REF, REF]),
                fields: vec![
                    ("bytesStart", as_child(&bytes_start)),
                    ("bytesUsed", NodeChild::Node(Rc::clone(&counter))),
                ],
            },
        );
        let pool = self.byte_block_pool(pool);
        // `BytesRefBlockPool`: `ByteBlockPool byteBlockPool`.
        let ref_pool = JavaNode::new(
            "org.apache.lucene.util.BytesRefBlockPool",
            NodeShape::Object {
                aligned_shallow_size: aligned_shallow_instance_size(&[REF]),
                fields: vec![("byteBlockPool", NodeChild::Node(pool))],
            },
        );
        // `BytesRefBlockPool pool`, `int[] bytesStart`, `hashSize`,
        // `hashHalfSize`, `hashMask`, `highMask`, `count`, `lastCount`,
        // `int[] ids`, `BytesStartArray bytesStartArray`, `Counter bytesUsed`.
        JavaNode::new(
            "org.apache.lucene.util.BytesRefHash",
            NodeShape::Object {
                aligned_shallow_size: aligned_shallow_instance_size(&[
                    REF, REF, 4, 4, 4, 4, 4, 4, REF, REF, REF,
                ]),
                fields: vec![
                    ("pool", NodeChild::Node(ref_pool)),
                    ("bytesStart", as_child(&bytes_start)),
                    ("ids", ids),
                    ("bytesStartArray", NodeChild::Node(bytes_start_array)),
                    ("bytesUsed", NodeChild::Node(counter)),
                ],
            },
        )
    }

    /// `org.apache.lucene.util.ByteBlockPool` with a `DirectAllocator`:
    /// `byte[][] buffers`, `int bufferUpto`, `int byteUpto`, `byte[] buffer`
    /// (the head block, one of `buffers`), `int byteOffset` and `Allocator
    /// allocator` (`int blockSize`).
    pub fn byte_block_pool(&mut self, pool: &ByteBlockPool) -> Rc<JavaNode> {
        let allocated = usize::try_from(pool.buffer_upto() + 1).unwrap_or(0);
        let blocks: Vec<Rc<JavaNode>> = (0..allocated)
            .map(|i| {
                let block = pool.buffer(i);
                self.primitive_array(block.as_ptr() as *const (), "[B", block.len(), 1)
            })
            .collect();
        let mut elements: Vec<NodeChild> = blocks
            .iter()
            .map(|b| NodeChild::Node(Rc::clone(b)))
            .collect();
        elements.resize_with(pool.buffers_length().max(allocated), || NodeChild::Null);
        let buffers = JavaNode::new(
            "[[B",
            NodeShape::Array {
                shallow_size: array_shallow_size(elements.len(), REF),
                elements: Some(elements),
            },
        );
        let buffer = match blocks.last() {
            Some(head) => NodeChild::Node(Rc::clone(head)),
            None => NodeChild::Null,
        };
        let allocator = JavaNode::new(
            "org.apache.lucene.util.ByteBlockPool$DirectAllocator",
            NodeShape::Object {
                aligned_shallow_size: aligned_shallow_instance_size(&[4]),
                fields: Vec::new(),
            },
        );
        JavaNode::new(
            "org.apache.lucene.util.ByteBlockPool",
            NodeShape::Object {
                aligned_shallow_size: aligned_shallow_instance_size(&[REF, 4, 4, REF, 4, REF]),
                fields: vec![
                    ("buffers", NodeChild::Node(buffers)),
                    ("buffer", buffer),
                    ("allocator", NodeChild::Node(allocator)),
                ],
            },
        )
    }
}

/// The shallow size of a `java.util.HashMap`: see [`ModelBuilder::hash_map`].
pub const HASH_MAP_SHALLOW_SIZE: i64 = aligned_shallow_instance_size_const(8);

/// The shallow size of a `java.util.ArrayList`: `Object[] elementData`, `int
/// size` and `AbstractList`'s `int modCount`.
pub const ARRAY_LIST_SHALLOW_SIZE: i64 = aligned_shallow_instance_size_const(3);

/// `alignObjectSize(NUM_BYTES_OBJECT_HEADER + 4 * fields)`: the shallow size
/// of a class whose `fields` fields are all four bytes wide.
const fn aligned_shallow_instance_size_const(fields: i64) -> i64 {
    RamUsageEstimator::align_object_size(RamUsageEstimator::NUM_BYTES_OBJECT_HEADER + 4 * fields)
}
