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

use randomizedtesting::jdk::exception::Throwable;
use rucene::index::doc_values::{MonotonicDeltaLongValues, OrdinalMap, PackedDeltaLongValues};
use rucene::index::{CacheKey, PrefixCodedTerms, Term};
use rucene::search::multi_term_query::MultiTermQuery;
use rucene::search::term_in_set_query::TermInSetQuery;
use rucene::search::{
    BooleanClause, BooleanQuery, CacheAndCount, DisjunctionMaxQuery, LRUQueryCache, Occur,
    PhraseQuery, Query, TermQuery,
};
use rucene::util::automaton::byte_run_automaton::ByteRunAutomaton;
use rucene::util::automaton::compiled_automaton::{AutomatonType, CompiledAutomaton};
use rucene::util::automaton::nfa_run_automaton::DStateView;
use rucene::util::automaton::{Automaton, NFARunAutomaton};
use rucene::util::extra::{IdentityLongValues, LongValues, ZeroesLongValues};
use rucene::util::hnsw::{NeighborArray, OnHeapHnswGraph};
use rucene::util::packed::{
    AbstractPagedMutable, AbstractPagedMutableOps, DeltaPackedLongValues,
    DeltaPackedLongValuesBuilder, Format, GrowableWriter, MonotonicLongValues,
    MonotonicLongValuesBuilder, NullReader, Packed64, Packed64SingleBlock, PackedIntsMutable,
    PackedIntsReader, PackedLongValues, PackedLongValuesAccess, PackedLongValuesBuilder,
    PackedLongValuesBuilderOps, PagedGrowableWriter, PagedMutable,
};
use rucene::util::ring_buffer::IntBag;
use rucene::util::{
    ArrayUtil, ByteBlockPool, BytesRef, BytesRefHash, FixedBitSet, FrequencyTrackingRingBuffer,
    PagedBytes, PagedBytesReader, RamUsageEstimator,
};

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
    /// A JDK object outside the shapes above with a reference field the
    /// module system does not open to reflection (no `--add-opens`):
    /// `createCacheEntry` fails on `f.setAccessible(true)` and throws
    /// `RuntimeException("Can't access field '<field>' of class '<class>' for
    /// RAM estimation.")`, as `ReentrantReadWriteLock.ReadLock` and
    /// `WriteLock` do on their field `sync`.
    Inaccessible {
        /// The first reference field reflection refuses to open.
        field: &'static str,
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

    /// `ob instanceof T`, for the Java type named `java_type`, as a custom
    /// [`Accumulator`] asks it: by default, only the object's own class.
    fn instance_of(&self, java_type: &str) -> bool {
        self.java_class_name() == java_type
    }

    /// `((Accountable) ob).ramBytesUsed()`, for an object whose Java class is
    /// an `Accountable` a custom [`Accumulator`] measures that way; `None`
    /// otherwise.
    fn accountable_ram_bytes_used(&self) -> Option<i64> {
        None
    }
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
        // `createCacheEntry`: `f.setAccessible(true)` throws, rethrown as a
        // `RuntimeException` naming the field and the class.
        JavaObject::Inaccessible { field } => Throwable::new(
            "java.lang.RuntimeException",
            format!(
                "Can't access field '{field}' of class '{}' for RAM estimation.",
                ob.java_class_name()
            ),
        )
        .throw(),
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
    /// See [`JavaObject::Ignorable`].
    Ignorable,
    /// See [`JavaObject::Enum`].
    Enum {
        /// `RamUsageEstimator.shallowSizeOf(ob)`.
        shallow_size: i64,
    },
    /// See [`JavaObject::ByteBuffer`].
    ByteBuffer {
        /// `capacity()`.
        capacity: i32,
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
    /// See [`JavaObject::Inaccessible`].
    Inaccessible {
        /// The first reference field reflection refuses to open.
        field: &'static str,
    },
}

/// One Java object of a declared graph.
pub struct JavaNode {
    class_name: &'static str,
    shape: NodeShape,
    /// The Java supertypes `instanceof` answers for, besides the class.
    supertypes: &'static [&'static str],
    /// `ramBytesUsed()` of an `Accountable` object an accumulator measures.
    ram_bytes_used: Option<i64>,
}

impl JavaNode {
    /// Creates a node of the Java class `class_name`.
    pub fn new(class_name: &'static str, shape: NodeShape) -> Rc<Self> {
        Self::typed(class_name, shape, &[], None)
    }

    /// Creates a node of the Java class `class_name` that is also an instance
    /// of `supertypes` and, when `ram_bytes_used` is given, an `Accountable`
    /// reporting it (see [`RamObject::instance_of`] and
    /// [`RamObject::accountable_ram_bytes_used`]).
    pub fn typed(
        class_name: &'static str,
        shape: NodeShape,
        supertypes: &'static [&'static str],
        ram_bytes_used: Option<i64>,
    ) -> Rc<Self> {
        Rc::new(Self {
            class_name,
            shape,
            supertypes,
            ram_bytes_used,
        })
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

    fn instance_of(&self, java_type: &str) -> bool {
        self.class_name == java_type || self.supertypes.contains(&java_type)
    }

    fn accountable_ram_bytes_used(&self) -> Option<i64> {
        self.ram_bytes_used
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
            NodeShape::Ignorable => JavaObject::Ignorable,
            NodeShape::Enum { shallow_size } => JavaObject::Enum {
                shallow_size: *shallow_size,
            },
            NodeShape::ByteBuffer { capacity } => JavaObject::ByteBuffer {
                capacity: *capacity,
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
            NodeShape::Inaccessible { field } => JavaObject::Inaccessible { field },
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

    /// [`shared`](Self::shared) for a node that is also an instance of
    /// `supertypes` (see [`JavaNode::typed`]).
    pub fn shared_typed(
        &mut self,
        address: *const (),
        class_name: &'static str,
        supertypes: &'static [&'static str],
        build: impl FnOnce(&mut Self) -> NodeShape,
    ) -> Rc<JavaNode> {
        let key = (address as usize, class_name);
        if let Some(node) = self.seen.get(&key) {
            return Rc::clone(node);
        }
        let shape = build(self);
        let node = JavaNode::typed(class_name, shape, supertypes, None);
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
            return self.shared_typed(
                address,
                "org.apache.lucene.search.TermQuery",
                QUERY_TYPES,
                |b| {
                    let term = b.term(q.get_term());
                    let states = match q.get_term_states() {
                        // `TermStates`: the fields `topReaderContextIdentity`,
                        // `states`, `docFreq` and `totalTermFreq`.
                        Some(states) => NodeChild::Node(b.shared(
                            Arc::as_ptr(states) as *const (),
                            "org.apache.lucene.index.TermStates",
                            |_| NodeShape::Object {
                                aligned_shallow_size: aligned_shallow_instance_size(&[
                                    REF, REF, 4, 8,
                                ]),
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
                },
            );
        }
        if let Some(q) = any.downcast_ref::<PhraseQuery>() {
            return self.shared_typed(
                address,
                "org.apache.lucene.search.PhraseQuery",
                QUERY_TYPES,
                |b| {
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
                },
            );
        }
        if let Some(q) = any.downcast_ref::<DisjunctionMaxQuery>() {
            return self.shared_typed(
                address,
                "org.apache.lucene.search.DisjunctionMaxQuery",
                QUERY_TYPES,
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
            return self.shared_typed(
                address,
                "org.apache.lucene.search.BooleanQuery",
                QUERY_TYPES,
                |b| {
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
                },
            );
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

    /// `org.apache.lucene.index.PrefixCodedTerms`: `List<ByteBuffer> content`
    /// (the `ArrayList` of `ByteBuffersDataOutput.toBufferList()`), `long
    /// size`, `long delGen` and `int lazyHash` (`PrefixCodedTerms.java:37-40`).
    pub fn prefix_coded_terms(&mut self, terms: &PrefixCodedTerms) -> Rc<JavaNode> {
        self.shared(
            terms as *const PrefixCodedTerms as *const (),
            "org.apache.lucene.index.PrefixCodedTerms",
            |_| {
                let buffers: Vec<NodeChild> = terms
                    .content()
                    .iter()
                    .map(|block| {
                        NodeChild::Node(JavaNode::new(
                            "java.nio.HeapByteBufferR",
                            NodeShape::ByteBuffer {
                                capacity: i32::try_from(block.capacity()).unwrap_or(i32::MAX),
                            },
                        ))
                    })
                    .collect();
                let content = JavaNode::new(
                    "java.util.ArrayList",
                    NodeShape::JavaIterable {
                        shallow_size: ARRAY_LIST_SHALLOW_SIZE,
                        elements: buffers,
                    },
                );
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[REF, 8, 8, 4]),
                    fields: vec![("content", NodeChild::Node(content))],
                }
            },
        )
    }

    /// `org.apache.lucene.search.TermInSetQuery`: `Query.CLASS_NAME_HASH`,
    /// `MultiTermQuery.field` and `rewriteMethod`, and its own `field` (the
    /// same `String`), `termData` and `termDataHashCode`
    /// (`TermInSetQuery.java:71-73`). The default rewrite method is the
    /// anonymous `CONSTANT_SCORE_BLENDED_REWRITE`, a `RewriteMethod` with no
    /// field.
    pub fn term_in_set_query(&mut self, query: &TermInSetQuery) -> Rc<JavaNode> {
        self.shared_typed(
            query as *const TermInSetQuery as *const (),
            "org.apache.lucene.search.TermInSetQuery",
            &[
                "org.apache.lucene.search.MultiTermQuery",
                "org.apache.lucene.search.Query",
            ],
            |b| {
                let field = b.string(query.get_field());
                let rewrite_method = query.get_rewrite_method();
                let rewrite_method = b.shared(
                    Arc::as_ptr(&rewrite_method) as *const (),
                    "org.apache.lucene.search.MultiTermQuery$RewriteMethod",
                    |_| NodeShape::Object {
                        aligned_shallow_size: aligned_shallow_instance_size(&[]),
                        fields: Vec::new(),
                    },
                );
                let term_data = b.prefix_coded_terms(query.term_data());
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[
                        4, REF, REF, REF, REF, 4,
                    ]),
                    fields: vec![
                        ("field", NodeChild::Node(Rc::clone(&field))),
                        ("rewriteMethod", NodeChild::Node(rewrite_method)),
                        ("field", NodeChild::Node(field)),
                        ("termData", NodeChild::Node(term_data)),
                    ],
                }
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

/// The supertypes of a `Query` subclass `instanceof` answers for.
const QUERY_TYPES: &[&str] = &["org.apache.lucene.search.Query"];

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

// ---------------------------------------------------------------------------
// OrdinalMap, OnHeapHnswGraph and LRUQueryCache
// ---------------------------------------------------------------------------

/// `RamUsageEstimator.shallowSizeOf` of a `java.util.concurrent.atomic.LongAdder`
/// on JDK 25 (`Striped64`'s `cells`, `base` and `cellsBusy`): 32 bytes, as
/// printed against `lucene-core-10.5.0.jar`.
const LONG_ADDER_SHALLOW_SIZE: i64 = 32;

/// `RamUsageEstimator.shallowSizeOf` of a `java.util.Collections$SynchronizedMap`
/// (`m`, `mutex`, `keySet`, `entrySet`, `values`) on JDK 25: 32 bytes.
const SYNCHRONIZED_MAP_SHALLOW_SIZE: i64 = 32;

/// `RamUsageEstimator.shallowSizeOf` of a `java.util.Collections$SynchronizedSet`
/// (`c`, `mutex`) on JDK 25: 24 bytes.
const SYNCHRONIZED_SET_SHALLOW_SIZE: i64 = 24;

/// `RamUsageEstimator.shallowSizeOf` of a `java.util.IdentityHashMap` on
/// JDK 25: 40 bytes.
const IDENTITY_HASH_MAP_SHALLOW_SIZE: i64 = 40;

/// The supertypes of a `DocIdSet` subclass `instanceof` answers for.
const DOC_ID_SET_TYPES: &[&str] = &["org.apache.lucene.search.DocIdSet"];

/// The Java class of a `Packed64SingleBlock` of `bits_per_value` bits: the
/// generated subclass `Packed64SingleBlock.create` picks.
fn packed64_single_block_class(bits_per_value: i32) -> &'static str {
    match bits_per_value {
        1 => "org.apache.lucene.util.packed.Packed64SingleBlock$Packed64SingleBlock1",
        2 => "org.apache.lucene.util.packed.Packed64SingleBlock$Packed64SingleBlock2",
        3 => "org.apache.lucene.util.packed.Packed64SingleBlock$Packed64SingleBlock3",
        4 => "org.apache.lucene.util.packed.Packed64SingleBlock$Packed64SingleBlock4",
        5 => "org.apache.lucene.util.packed.Packed64SingleBlock$Packed64SingleBlock5",
        6 => "org.apache.lucene.util.packed.Packed64SingleBlock$Packed64SingleBlock6",
        7 => "org.apache.lucene.util.packed.Packed64SingleBlock$Packed64SingleBlock7",
        8 => "org.apache.lucene.util.packed.Packed64SingleBlock$Packed64SingleBlock8",
        9 => "org.apache.lucene.util.packed.Packed64SingleBlock$Packed64SingleBlock9",
        10 => "org.apache.lucene.util.packed.Packed64SingleBlock$Packed64SingleBlock10",
        12 => "org.apache.lucene.util.packed.Packed64SingleBlock$Packed64SingleBlock12",
        16 => "org.apache.lucene.util.packed.Packed64SingleBlock$Packed64SingleBlock16",
        21 => "org.apache.lucene.util.packed.Packed64SingleBlock$Packed64SingleBlock21",
        32 => "org.apache.lucene.util.packed.Packed64SingleBlock$Packed64SingleBlock32",
        other => unmodelled(&format!("Packed64SingleBlock of {other} bits per value")),
    }
}

/// A Rucene object the declared graphs do not model yet: the measurement
/// cannot be made, so it fails with an `UnsupportedOperationException`.
fn unmodelled(what: &str) -> ! {
    Throwable::new(
        "java.lang.UnsupportedOperationException",
        format!("RamUsageTester.ramUsed: no RamObject model of {what}"),
    )
    .throw()
}

/// `n` as a [`NodeChild`].
fn node(n: Rc<JavaNode>) -> NodeChild {
    NodeChild::Node(n)
}

impl ModelBuilder {
    /// A primitive array over `values`; an empty one is a fresh object (all
    /// empty Rust slices may share one dangling address, Java arrays never
    /// share an identity).
    fn slice_array<T>(
        &mut self,
        values: &[T],
        class_name: &'static str,
        element_bytes: i64,
    ) -> Rc<JavaNode> {
        if values.is_empty() {
            JavaNode::new(
                class_name,
                NodeShape::Array {
                    shallow_size: array_shallow_size(0, element_bytes),
                    elements: None,
                },
            )
        } else {
            self.primitive_array(
                values.as_ptr() as *const (),
                class_name,
                values.len(),
                element_bytes,
            )
        }
    }

    /// `LongValues.IDENTITY`, the anonymous `LongValues$1` with no field: one
    /// shared node.
    pub fn long_values_identity(&mut self) -> Rc<JavaNode> {
        self.shared(
            std::ptr::dangling::<()>(),
            "org.apache.lucene.util.LongValues$1",
            |_| NodeShape::Object {
                aligned_shallow_size: aligned_shallow_instance_size(&[]),
                fields: Vec::new(),
            },
        )
    }

    /// `LongValues.ZEROES`, the anonymous `LongValues$2` with no field: one
    /// shared node.
    pub fn long_values_zeroes(&mut self) -> Rc<JavaNode> {
        self.shared(
            2 as *const (),
            "org.apache.lucene.util.LongValues$2",
            |_| NodeShape::Object {
                aligned_shallow_size: aligned_shallow_instance_size(&[]),
                fields: Vec::new(),
            },
        )
    }

    /// `PackedInts.NullReader.forCount(value_count)`: the field `int
    /// valueCount`; the shared `DEFAULT_PACKED_LONG_VALUES_PAGE_SIZE` instance
    /// for `PackedLongValues.DEFAULT_PAGE_SIZE`, a fresh object otherwise
    /// (`PackedInts.java:557-562`).
    pub fn null_reader(&mut self, value_count: i32) -> Rc<JavaNode> {
        let shape = || NodeShape::Object {
            aligned_shallow_size: aligned_shallow_instance_size(&[4]),
            fields: Vec::new(),
        };
        if value_count == PackedLongValues::DEFAULT_PAGE_SIZE {
            self.shared(
                3 as *const (),
                "org.apache.lucene.util.packed.PackedInts$NullReader",
                |_| shape(),
            )
        } else {
            JavaNode::new(
                "org.apache.lucene.util.packed.PackedInts$NullReader",
                shape(),
            )
        }
    }

    /// A `PackedInts.Reader`, by its Java class: `NullReader`; `Packed64`
    /// (`long[] blocks`, `long maskRight`, `int bpvMinusBlockSize` and
    /// `MutableImpl`'s `int valueCount` and `int bitsPerValue`); a
    /// `Packed64SingleBlock` subclass (`long[] blocks`, `valueCount`,
    /// `bitsPerValue`).
    pub fn packed_ints_reader(&mut self, reader: &dyn PackedIntsReader) -> Rc<JavaNode> {
        let address = reader as *const dyn PackedIntsReader as *const ();
        let any = reader.as_any();
        if any.is_some_and(|a| a.is::<NullReader>()) {
            return self.null_reader(reader.size());
        }
        if let Some(packed) = any.and_then(|a| a.downcast_ref::<Packed64>()) {
            return self.shared(address, "org.apache.lucene.util.packed.Packed64", |b| {
                let blocks = b.slice_array(packed.blocks(), "[J", 8);
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[REF, 8, 4, 4, 4]),
                    fields: vec![("blocks", node(blocks))],
                }
            });
        }
        if let Some(packed) = any.and_then(|a| a.downcast_ref::<Packed64SingleBlock>()) {
            let class_name = packed64_single_block_class(packed.bits_per_value());
            return self.shared(address, class_name, |b| {
                let blocks = b.slice_array(packed.blocks(), "[J", 8);
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[REF, 4, 4]),
                    fields: vec![("blocks", node(blocks))],
                }
            });
        }
        if let Some(writer) = any.and_then(|a| a.downcast_ref::<GrowableWriter>()) {
            return self.growable_writer(writer);
        }
        unmodelled("this PackedInts.Reader")
    }

    /// The `PackedInts.Reader[] values` array of a `PackedLongValues`.
    fn packed_pages(&mut self, values: &PackedLongValues) -> Rc<JavaNode> {
        let pages = values.values();
        let elements: Vec<NodeChild> = pages
            .iter()
            .map(|page| node(self.packed_ints_reader(page.as_ref())))
            .collect();
        let shape = NodeShape::Array {
            shallow_size: array_shallow_size(elements.len(), REF),
            elements: Some(elements),
        };
        let class_name = "[Lorg.apache.lucene.util.packed.PackedInts$Reader;";
        if pages.is_empty() {
            JavaNode::new(class_name, shape)
        } else {
            self.shared(pages.as_ptr() as *const (), class_name, |_| shape)
        }
    }

    /// `org.apache.lucene.util.packed.PackedLongValues`: `Reader[] values`,
    /// `int pageShift`, `int pageMask`, `long size`, `long ramBytesUsed`.
    pub fn packed_long_values(&mut self, values: &PackedLongValues) -> Rc<JavaNode> {
        self.shared(
            values as *const PackedLongValues as *const (),
            "org.apache.lucene.util.packed.PackedLongValues",
            |b| {
                let pages = b.packed_pages(values);
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[REF, 4, 4, 8, 8]),
                    fields: vec![("values", node(pages))],
                }
            },
        )
    }

    /// `org.apache.lucene.util.packed.DeltaPackedLongValues`: `long[] mins`
    /// and the fields of `PackedLongValues`.
    pub fn delta_packed_long_values(&mut self, values: &DeltaPackedLongValues) -> Rc<JavaNode> {
        self.shared(
            values as *const DeltaPackedLongValues as *const (),
            "org.apache.lucene.util.packed.DeltaPackedLongValues",
            |b| {
                let mins = b.slice_array(values.mins(), "[J", 8);
                let pages = b.packed_pages(PackedLongValuesAccess::base(values));
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[REF, REF, 4, 4, 8, 8]),
                    fields: vec![("mins", node(mins)), ("values", node(pages))],
                }
            },
        )
    }

    /// `org.apache.lucene.util.packed.MonotonicLongValues`: `float[]
    /// averages` and the fields of `DeltaPackedLongValues`.
    pub fn monotonic_long_values(&mut self, values: &MonotonicLongValues) -> Rc<JavaNode> {
        self.shared(
            values as *const MonotonicLongValues as *const (),
            "org.apache.lucene.util.packed.MonotonicLongValues",
            |b| {
                let averages = b.slice_array(values.averages(), "[F", 4);
                let mins = b.slice_array(values.mins(), "[J", 8);
                let pages = b.packed_pages(PackedLongValuesAccess::base(values));
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[
                        REF, REF, REF, 4, 4, 8, 8,
                    ]),
                    fields: vec![
                        ("averages", node(averages)),
                        ("mins", node(mins)),
                        ("values", node(pages)),
                    ],
                }
            },
        )
    }

    /// An `org.apache.lucene.util.LongValues`, by its Java class: the
    /// `IDENTITY` and `ZEROES` singletons, the three `PackedLongValues`
    /// classes, and `OrdinalMap`'s anonymous `OrdinalMap$1` (`val$newDeltas`,
    /// a `PackedInts.Mutable`) and `OrdinalMap$2` (`val$deltas`, a
    /// `PackedLongValues`) (`OrdinalMap.java:376-391`).
    pub fn long_values(&mut self, values: &dyn LongValues) -> Rc<JavaNode> {
        let address = values as *const dyn LongValues as *const ();
        let Some(any) = values.as_any() else {
            unmodelled("this LongValues")
        };
        if any.is::<IdentityLongValues>() {
            return self.long_values_identity();
        }
        if any.is::<ZeroesLongValues>() {
            return self.long_values_zeroes();
        }
        if let Some(v) = any.downcast_ref::<PackedLongValues>() {
            return self.packed_long_values(v);
        }
        if let Some(v) = any.downcast_ref::<DeltaPackedLongValues>() {
            return self.delta_packed_long_values(v);
        }
        if let Some(v) = any.downcast_ref::<MonotonicLongValues>() {
            return self.monotonic_long_values(v);
        }
        if let Some(v) = any.downcast_ref::<PackedDeltaLongValues>() {
            return self.shared(address, "org.apache.lucene.index.OrdinalMap$1", |b| {
                let deltas = b.packed_ints_reader(v.deltas().as_packed_ints_reader());
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[REF]),
                    fields: vec![("val$newDeltas", node(deltas))],
                }
            });
        }
        if let Some(v) = any.downcast_ref::<MonotonicDeltaLongValues>() {
            return self.shared(address, "org.apache.lucene.index.OrdinalMap$2", |b| {
                let deltas = b.monotonic_long_values(v.deltas());
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[REF]),
                    fields: vec![("val$deltas", node(deltas))],
                }
            });
        }
        unmodelled("this LongValues")
    }

    /// `org.apache.lucene.index.IndexReader$CacheKey`, a class with no field:
    /// one node per key.
    pub fn cache_key(&mut self, key: &CacheKey) -> Rc<JavaNode> {
        self.shared(
            key.identity() as *const (),
            "org.apache.lucene.index.IndexReader$CacheKey",
            |_| NodeShape::Object {
                aligned_shallow_size: aligned_shallow_instance_size(&[]),
                fields: Vec::new(),
            },
        )
    }

    /// `org.apache.lucene.index.OrdinalMap` (`OrdinalMap.java:183-197`):
    /// `CacheKey owner`, `long valueCount`, `LongValues globalOrdDeltas`,
    /// `LongValues firstSegments`, `LongValues[] segmentToGlobalOrds`,
    /// `SegmentMap segmentMap` (`int[] newToOld`, `int[] oldToNew`) and `long
    /// ramBytesUsed`. The reference fields are listed in the iteration order
    /// of Java's `HashMap<Field, Object>` of them.
    pub fn ordinal_map(&mut self, map: &OrdinalMap) -> Rc<JavaNode> {
        self.shared(
            map as *const OrdinalMap as *const (),
            "org.apache.lucene.index.OrdinalMap",
            |b| {
                let owner = match &map.owner {
                    Some(key) => node(b.cache_key(key)),
                    None => NodeChild::Null,
                };
                let first_segments = b.long_values(map.first_segments());
                let new_to_old = b.slice_array(map.segment_map_new_to_old(), "[I", 4);
                let old_to_new = b.slice_array(map.segment_map_old_to_new(), "[I", 4);
                let segment_map = JavaNode::new(
                    "org.apache.lucene.index.OrdinalMap$SegmentMap",
                    NodeShape::Object {
                        aligned_shallow_size: aligned_shallow_instance_size(&[REF, REF]),
                        fields: vec![
                            ("newToOld", node(new_to_old)),
                            ("oldToNew", node(old_to_new)),
                        ],
                    },
                );
                let global_ord_deltas = b.long_values(map.global_ord_deltas());
                let ords = map.segment_to_global_ords();
                let elements: Vec<NodeChild> = ords
                    .iter()
                    .map(|values| node(b.long_values(values.as_ref())))
                    .collect();
                let segment_to_global_ords = JavaNode::new(
                    "[Lorg.apache.lucene.util.LongValues;",
                    NodeShape::Array {
                        shallow_size: array_shallow_size(elements.len(), REF),
                        elements: Some(elements),
                    },
                );
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[
                        REF, 8, REF, REF, REF, REF, 8,
                    ]),
                    fields: vec![
                        ("owner", owner),
                        ("firstSegments", node(first_segments)),
                        ("segmentMap", node(segment_map)),
                        ("globalOrdDeltas", node(global_ord_deltas)),
                        ("segmentToGlobalOrds", node(segment_to_global_ords)),
                    ],
                }
            },
        )
    }

    /// `org.apache.lucene.util.hnsw.NeighborArray` (`NeighborArray.java:39-46`):
    /// `boolean scoresDescOrder`, `int size`, `int maxSize`,
    /// `MaxSizedFloatArrayList scores`, `MaxSizedIntArrayList nodes`, `int
    /// sortedNodeSize`, `long ramBytesUsed` and `LongConsumer
    /// onHeapMemoryUsageListener`. Each list is `int maxSize` and
    /// `FloatArrayList`'s (`IntArrayList`'s) `float[] buffer` (`int[]`) and
    /// `int elementsCount`, its buffer of the length Java's has grown to.
    ///
    /// The listener `OnHeapHnswGraph.addNode` installs is a capturing lambda,
    /// a fresh object per array whose one field `arg$1` is the graph; that
    /// graph is the root of the walk, already seen when the lambda is reached,
    /// so the field is declared `null`, which adds the same nothing.
    pub fn neighbor_array(&mut self, array: &NeighborArray) -> Rc<JavaNode> {
        self.shared(
            array as *const NeighborArray as *const (),
            "org.apache.lucene.util.hnsw.NeighborArray",
            |_| {
                let length = array.buffer_length();
                let list = |list_class: &'static str, buffer_class: &'static str| {
                    let buffer = JavaNode::new(
                        buffer_class,
                        NodeShape::Array {
                            shallow_size: array_shallow_size(length, 4),
                            elements: None,
                        },
                    );
                    JavaNode::new(
                        list_class,
                        NodeShape::Object {
                            aligned_shallow_size: aligned_shallow_instance_size(&[4, REF, 4]),
                            fields: vec![("buffer", node(buffer))],
                        },
                    )
                };
                let scores = list(
                    "org.apache.lucene.internal.hppc.MaxSizedFloatArrayList",
                    "[F",
                );
                let nodes = list("org.apache.lucene.internal.hppc.MaxSizedIntArrayList", "[I");
                let listener = if array.has_on_heap_memory_usage_listener() {
                    node(JavaNode::new(
                        "org.apache.lucene.util.hnsw.OnHeapHnswGraph$$Lambda",
                        NodeShape::Object {
                            aligned_shallow_size: aligned_shallow_instance_size(&[REF]),
                            fields: vec![("arg$1", NodeChild::Null)],
                        },
                    ))
                } else {
                    NodeChild::Null
                };
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[
                        1, 4, 4, REF, REF, 4, 8, REF,
                    ]),
                    fields: vec![
                        ("scores", node(scores)),
                        ("nodes", node(nodes)),
                        ("onHeapMemoryUsageListener", listener),
                    ],
                }
            },
        )
    }

    /// `org.apache.lucene.util.hnsw.OnHeapHnswGraph`
    /// (`OnHeapHnswGraph.java:42-67`): `AtomicReference entryNode` (ignored by
    /// the walk), `NeighborArray[][] graph`, `IntArrayList[] levelToNodes`,
    /// `int lastFreezeSize`, `AtomicInteger size`, `AtomicInteger maxNodeId`,
    /// `int nsize`, `int nsize0`, `boolean noGrowth`, `int upto`,
    /// `NeighborArray cur` and `LongAdder graphRamBytesUsed` (one of the
    /// `SIMPLE_TYPES`, estimated at 0).
    ///
    /// `levelToNodes` is `null`: Java fills it only when `getNodesOnLevel` is
    /// called on a non-zero level, and the port does not cache it (see
    /// `OnHeapHnswGraph::build_level_to_nodes`). `cur` is `null` or one of the
    /// arrays of `graph` (`seek` sets it from `getNeighbors`), which the walk
    /// counts through `graph`; the port's cursor holds a copy, so the field is
    /// declared `null`, which adds the same nothing.
    pub fn on_heap_hnsw_graph(&mut self, graph: &OnHeapHnswGraph) -> Rc<JavaNode> {
        self.shared(
            graph as *const OnHeapHnswGraph as *const (),
            "org.apache.lucene.util.hnsw.OnHeapHnswGraph",
            |b| {
                let mut nodes = Vec::with_capacity(graph.graph_length());
                for n in 0..graph.graph_length() {
                    let child = match graph.graph_node(n) {
                        None => NodeChild::Null,
                        Some(levels) => {
                            let elements: Vec<NodeChild> = levels
                                .iter()
                                .map(|slot| match slot.get() {
                                    Some(array) => node(b.neighbor_array(array)),
                                    None => NodeChild::Null,
                                })
                                .collect();
                            node(JavaNode::new(
                                "[Lorg.apache.lucene.util.hnsw.NeighborArray;",
                                NodeShape::Array {
                                    shallow_size: array_shallow_size(elements.len(), REF),
                                    elements: Some(elements),
                                },
                            ))
                        }
                    };
                    nodes.push(child);
                }
                let outer = JavaNode::new(
                    "[[Lorg.apache.lucene.util.hnsw.NeighborArray;",
                    NodeShape::Array {
                        shallow_size: array_shallow_size(nodes.len(), REF),
                        elements: Some(nodes),
                    },
                );
                let atomic_integer = || {
                    JavaNode::new(
                        "java.util.concurrent.atomic.AtomicInteger",
                        NodeShape::Object {
                            aligned_shallow_size: aligned_shallow_instance_size(&[4]),
                            fields: Vec::new(),
                        },
                    )
                };
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[
                        REF, REF, REF, 4, REF, REF, 4, 4, 1, 4, REF, REF,
                    ]),
                    fields: vec![
                        (
                            "entryNode",
                            node(ignorable("java.util.concurrent.atomic.AtomicReference")),
                        ),
                        ("graph", node(outer)),
                        ("levelToNodes", NodeChild::Null),
                        ("graphRamBytesUsed", node(long_adder())),
                        ("maxNodeId", node(atomic_integer())),
                        ("size", node(atomic_integer())),
                        ("cur", NodeChild::Null),
                    ],
                }
            },
        )
    }

    /// A cached doc id set: a `DocIdSet` the LRU accumulators measure by its
    /// `ramBytesUsed()` without following it. `CacheAndCount.EMPTY`'s
    /// `DocIdSet.EMPTY` is the anonymous `DocIdSet$1`, one shared node.
    ///
    /// **Divergence.** The fields of the concrete set (`RoaringDocIdSet`,
    /// `BitDocIdSet`, ...) are not declared: no walk of an `LRUQueryCache`
    /// reaches them, since every one throws at the cache's `writeLock` (see
    /// [`lru_query_cache`](Self::lru_query_cache)), and both of Java's
    /// accumulators stop at a `DocIdSet`.
    fn cached_doc_id_set(&mut self, value: &CacheAndCount) -> Rc<JavaNode> {
        let set = value.cache();
        let ram_bytes_used = set.ram_bytes_used();
        let (address, class_name) = if value.is_empty_marker() {
            (4 as *const (), "org.apache.lucene.search.DocIdSet$1")
        } else {
            (
                Arc::as_ptr(set) as *const (),
                "org.apache.lucene.search.DocIdSet",
            )
        };
        let key = (address as usize, class_name);
        if let Some(n) = self.seen.get(&key) {
            return Rc::clone(n);
        }
        let n = JavaNode::typed(
            class_name,
            NodeShape::Object {
                aligned_shallow_size: aligned_shallow_instance_size(&[]),
                fields: Vec::new(),
            },
            DOC_ID_SET_TYPES,
            Some(ram_bytes_used),
        );
        self.seen.insert(key, Rc::clone(&n));
        n
    }

    /// `LRUQueryCache.CacheAndCount`: `DocIdSet cache` and `int count`;
    /// `CacheAndCount.EMPTY` is one shared node.
    fn cache_and_count(&mut self, value: &CacheAndCount) -> Rc<JavaNode> {
        let shape = |b: &mut Self| {
            let set = b.cached_doc_id_set(value);
            NodeShape::Object {
                aligned_shallow_size: aligned_shallow_instance_size(&[REF, 4]),
                fields: vec![("cache", node(set))],
            }
        };
        let class_name = "org.apache.lucene.search.LRUQueryCache$CacheAndCount";
        if value.is_empty_marker() {
            self.shared(5 as *const (), class_name, shape)
        } else {
            let shape = shape(self);
            JavaNode::new(class_name, shape)
        }
    }

    /// `org.apache.lucene.search.LRUQueryCache` (`LRUQueryCache.java:87-107`):
    /// `int maxSize`, `long maxRamBytesUsed`, `Predicate leavesToCache`, `Map
    /// uniqueQueries` (a `Collections.synchronizedMap` from each query to
    /// itself), `Set mostRecentlyUsedQueries` (its key set, a
    /// `Collections$SynchronizedSet`), `Map cache` (an `IdentityHashMap` from
    /// each core key to its `LeafCache`: `Object key`, `Map cache` from query
    /// to `CacheAndCount`, `long ramBytesUsed` and `this$0`), `ReadLock
    /// readLock`, `WriteLock writeLock`, `float skipCacheFactor`, `LongAdder
    /// hitCount`, `LongAdder missCount`, `long ramBytesUsed`, `long
    /// cacheCount` and `long cacheSize`.
    ///
    /// The reference fields are listed in the iteration order of Java's
    /// `HashMap<Field, Object>` of them, so the walk pops `cache` first, then
    /// `hitCount`, then `writeLock`, whose field `sync` the module system does
    /// not open: every `RamUsageTester.ramUsed` of an `LRUQueryCache` throws
    /// `RuntimeException("Can't access field 'sync' of class
    /// 'java.util.concurrent.locks.ReentrantReadWriteLock$WriteLock' for RAM
    /// estimation.")`, as printed against `lucene-test-framework-10.5.0.jar`
    /// on JDK 25.
    ///
    /// `leavesToCache` is a Rust closure; it is declared as an object with no
    /// field (16 bytes), as the tests' non-capturing lambdas and
    /// `MinSegmentSizePredicate` (one `int`) both are. `LeafCache.this$0` is
    /// the root of the walk, already seen when it is reached, so it is
    /// declared `null`, which adds the same nothing.
    pub fn lru_query_cache(&mut self, cache: &LRUQueryCache) -> Rc<JavaNode> {
        self.shared(
            cache as *const LRUQueryCache as *const (),
            "org.apache.lucene.search.LRUQueryCache",
            |b| {
                let queries = cache.cached_queries();
                let query_nodes: Vec<Rc<JavaNode>> =
                    queries.iter().map(|q| b.query(q.as_ref())).collect();
                let most_recently_used = JavaNode::new(
                    "java.util.Collections$SynchronizedSet",
                    NodeShape::JavaIterable {
                        shallow_size: SYNCHRONIZED_SET_SHALLOW_SIZE,
                        elements: query_nodes.iter().map(|q| node(Rc::clone(q))).collect(),
                    },
                );
                let unique_queries = JavaNode::new(
                    "java.util.Collections$SynchronizedMap",
                    NodeShape::JavaMap {
                        shallow_size: SYNCHRONIZED_MAP_SHALLOW_SIZE,
                        entries: query_nodes
                            .iter()
                            .map(|q| (node(Rc::clone(q)), node(Rc::clone(q))))
                            .collect(),
                    },
                );
                let mut leaves = Vec::new();
                for (key, entries) in cache.leaf_caches() {
                    let key = b.cache_key(&key);
                    let entries: Vec<(NodeChild, NodeChild)> = entries
                        .iter()
                        .map(|(query, value)| {
                            (
                                node(b.query(query.as_ref())),
                                node(b.cache_and_count(value)),
                            )
                        })
                        .collect();
                    let leaf = JavaNode::new(
                        "org.apache.lucene.search.LRUQueryCache$LeafCache",
                        NodeShape::Object {
                            aligned_shallow_size: aligned_shallow_instance_size(&[
                                REF, REF, 8, REF,
                            ]),
                            fields: vec![
                                ("this$0", NodeChild::Null),
                                ("cache", node(b.hash_map(entries))),
                                ("key", node(Rc::clone(&key))),
                            ],
                        },
                    );
                    leaves.push((node(key), node(leaf)));
                }
                let leaf_caches = JavaNode::new(
                    "java.util.IdentityHashMap",
                    NodeShape::JavaMap {
                        shallow_size: IDENTITY_HASH_MAP_SHALLOW_SIZE,
                        entries: leaves,
                    },
                );
                let leaves_to_cache = JavaNode::new(
                    "java.util.function.Predicate",
                    NodeShape::Object {
                        aligned_shallow_size: aligned_shallow_instance_size(&[]),
                        fields: Vec::new(),
                    },
                );
                let lock = |class_name: &'static str| {
                    JavaNode::new(class_name, NodeShape::Inaccessible { field: "sync" })
                };
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[
                        4, 8, REF, REF, REF, REF, REF, REF, 4, REF, REF, 8, 8, 8,
                    ]),
                    fields: vec![
                        ("missCount", node(long_adder())),
                        ("mostRecentlyUsedQueries", node(most_recently_used)),
                        (
                            "readLock",
                            node(lock(
                                "java.util.concurrent.locks.ReentrantReadWriteLock$ReadLock",
                            )),
                        ),
                        ("uniqueQueries", node(unique_queries)),
                        ("leavesToCache", node(leaves_to_cache)),
                        (
                            "writeLock",
                            node(lock(
                                "java.util.concurrent.locks.ReentrantReadWriteLock$WriteLock",
                            )),
                        ),
                        ("hitCount", node(long_adder())),
                        ("cache", node(leaf_caches)),
                    ],
                }
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Packed writers and builders, ring buffer, paged bytes, automata
// ---------------------------------------------------------------------------

/// `RamUsageEstimator.shallowSizeOf` of a `PackedInts.Format` constant on
/// JDK 25 (`id` and `Enum`'s `name`, `ordinal` and `hash`): 32 bytes, as
/// printed against `lucene-core-10.5.0.jar`.
const PACKED_INTS_FORMAT_SHALLOW_SIZE: i64 = aligned_shallow_instance_size_const(4);

/// `RamUsageEstimator.shallowSizeOf` of a `CompiledAutomaton.AUTOMATON_TYPE`
/// constant on JDK 25 (`Enum`'s `name`, `ordinal` and `hash`): 24 bytes.
const AUTOMATON_TYPE_SHALLOW_SIZE: i64 = aligned_shallow_instance_size_const(3);

/// `RamUsageEstimator.shallowSizeOf` of a `java.util.BitSet` (`long[] words`,
/// `int wordsInUse`, `boolean sizeIsSticky`): 24 bytes.
const BIT_SET_SHALLOW_SIZE: i64 = 24;

/// The length of the `keys` and `values` arrays of a fresh hppc
/// `IntIntHashMap(5)` and of a fresh `IntObjectHashMap()`: a buffer of 8 slots
/// plus the empty-key slot, 9, as printed against `lucene-core-10.5.0.jar`.
const FRESH_HPPC_MAP_SLOTS: usize = 9;

/// `Operations.PointTransitionSet`'s initial `new PointTransitions[5]`
/// (`Operations.java:570`).
const POINT_TRANSITION_SET_INITIAL_POINTS: usize = 5;

/// `NFARunAutomaton`'s initial `dStates = new DState[10]`
/// (`NFARunAutomaton.java:78`).
const NFA_D_STATES_INITIAL_LENGTH: usize = 10;

impl ModelBuilder {
    /// `org.apache.lucene.util.packed.GrowableWriter`
    /// (`GrowableWriter.java:32-34`): `long currentMask`, `PackedInts.Mutable
    /// current` and `float acceptableOverheadRatio`.
    pub fn growable_writer(&mut self, writer: &GrowableWriter) -> Rc<JavaNode> {
        self.shared(
            writer as *const GrowableWriter as *const (),
            "org.apache.lucene.util.packed.GrowableWriter",
            |b| {
                let current = b.packed_ints_reader(writer.mutable().as_packed_ints_reader());
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[8, REF, 4]),
                    fields: vec![("current", node(current))],
                }
            },
        )
    }

    /// A `PackedInts.Format` constant: an `Enum`, measured by its shallow size
    /// alone. Each constant has a body (`PackedInts.java:63-139`), so its class
    /// is the anonymous `PackedInts$Format$1` (`PACKED`) or `$2`
    /// (`PACKED_SINGLE_BLOCK`); one node per constant.
    pub fn packed_ints_format(&mut self, format: Format) -> Rc<JavaNode> {
        let (address, class_name) = match format {
            Format::Packed => (1usize, "org.apache.lucene.util.packed.PackedInts$Format$1"),
            Format::PackedSingleBlock => {
                (2usize, "org.apache.lucene.util.packed.PackedInts$Format$2")
            }
        };
        self.shared(address as *const (), class_name, |_| NodeShape::Enum {
            shallow_size: PACKED_INTS_FORMAT_SHALLOW_SIZE,
        })
    }

    /// The `PackedInts.Mutable[] subMutables` array of an
    /// `AbstractPagedMutable`, a `null` slot for a page not allocated.
    fn sub_mutables(&mut self, base: &AbstractPagedMutable) -> Rc<JavaNode> {
        let elements: Vec<NodeChild> = (0..base.page_count())
            .map(|i| match base.page(i) {
                Some(page) => node(self.packed_ints_reader(page.as_packed_ints_reader())),
                None => NodeChild::Null,
            })
            .collect();
        JavaNode::new(
            "[Lorg.apache.lucene.util.packed.PackedInts$Mutable;",
            NodeShape::Array {
                shallow_size: array_shallow_size(elements.len(), REF),
                elements: Some(elements),
            },
        )
    }

    /// `org.apache.lucene.util.packed.PagedMutable` (`PagedMutable.java:31`,
    /// `AbstractPagedMutable.java:38-42`): `PackedInts.Format format` and
    /// `AbstractPagedMutable`'s `long size`, `int pageShift`, `int pageMask`,
    /// `PackedInts.Mutable[] subMutables` and `int bitsPerValue`.
    pub fn paged_mutable(&mut self, mutable: &PagedMutable) -> Rc<JavaNode> {
        self.shared(
            mutable as *const PagedMutable as *const (),
            "org.apache.lucene.util.packed.PagedMutable",
            |b| {
                let format = b.packed_ints_format(mutable.format());
                let sub_mutables = b.sub_mutables(mutable.base());
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[REF, 8, 4, 4, REF, 4]),
                    fields: vec![
                        ("format", node(format)),
                        ("subMutables", node(sub_mutables)),
                    ],
                }
            },
        )
    }

    /// `org.apache.lucene.util.packed.PagedGrowableWriter`
    /// (`PagedGrowableWriter.java:32`): `float acceptableOverheadRatio` and
    /// the fields of `AbstractPagedMutable`; each page is a `GrowableWriter`.
    pub fn paged_growable_writer(&mut self, writer: &PagedGrowableWriter) -> Rc<JavaNode> {
        self.shared(
            writer as *const PagedGrowableWriter as *const (),
            "org.apache.lucene.util.packed.PagedGrowableWriter",
            |b| {
                let sub_mutables = b.sub_mutables(writer.base());
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[4, 8, 4, 4, REF, 4]),
                    fields: vec![("subMutables", node(sub_mutables))],
                }
            },
        )
    }

    /// A `PackedLongValues.Builder`, by its Java class
    /// (`PackedLongValues.java:176-196`, `DeltaPackedLongValues.java:80`,
    /// `MonotonicLongValues.java:84`): `int pageShift`, `int pageMask`, `float
    /// acceptableOverheadRatio`, `long[] pending` (`null` after `build()`),
    /// `long size`, `PackedInts.Reader[] values` (a `null` slot for a page not
    /// packed yet), `long ramBytesUsed`, `int valuesOff`, `int pendingOff`;
    /// `DeltaPackedLongValues.Builder` adds `long[] mins`, and
    /// `MonotonicLongValues.Builder` adds `float[] averages` to those.
    pub fn packed_long_values_builder<B>(&mut self, builder: &B) -> Rc<JavaNode>
    where
        B: PackedLongValuesBuilderOps + 'static,
    {
        let any: &dyn std::any::Any = builder;
        let address = builder as *const B as *const ();
        let state = builder.state();
        let base_fields: &[i64] = &[4, 4, 4, REF, 8, REF, 8, 4, 4];
        let build_base = |b: &mut Self| {
            let pending = match state.pending() {
                Some(pending) => node(b.slice_array(pending, "[J", 8)),
                None => NodeChild::Null,
            };
            let slots = state.values();
            let elements: Vec<NodeChild> = slots
                .iter()
                .map(|slot| match slot {
                    Some(page) => node(b.packed_ints_reader(page.as_ref())),
                    None => NodeChild::Null,
                })
                .collect();
            let values = JavaNode::new(
                "[Lorg.apache.lucene.util.packed.PackedInts$Reader;",
                NodeShape::Array {
                    shallow_size: array_shallow_size(elements.len(), REF),
                    elements: Some(elements),
                },
            );
            (pending, node(values))
        };
        if any.is::<PackedLongValuesBuilder>() {
            return self.shared(
                address,
                "org.apache.lucene.util.packed.PackedLongValues$Builder",
                |b| {
                    let (pending, values) = build_base(b);
                    NodeShape::Object {
                        aligned_shallow_size: aligned_shallow_instance_size(base_fields),
                        fields: vec![("pending", pending), ("values", values)],
                    }
                },
            );
        }
        if let Some(delta) = any.downcast_ref::<DeltaPackedLongValuesBuilder>() {
            return self.shared(
                address,
                "org.apache.lucene.util.packed.DeltaPackedLongValues$Builder",
                |b| {
                    let (pending, values) = build_base(b);
                    let mins = b.slice_array(delta.mins(), "[J", 8);
                    let mut field_sizes = vec![REF];
                    field_sizes.extend_from_slice(base_fields);
                    NodeShape::Object {
                        aligned_shallow_size: aligned_shallow_instance_size(&field_sizes),
                        fields: vec![
                            ("pending", pending),
                            ("values", values),
                            ("mins", node(mins)),
                        ],
                    }
                },
            );
        }
        if let Some(monotonic) = any.downcast_ref::<MonotonicLongValuesBuilder>() {
            return self.shared(
                address,
                "org.apache.lucene.util.packed.MonotonicLongValues$Builder",
                |b| {
                    let (pending, values) = build_base(b);
                    let mins = b.slice_array(monotonic.mins(), "[J", 8);
                    let averages = b.slice_array(monotonic.averages(), "[F", 4);
                    let mut field_sizes = vec![REF, REF];
                    field_sizes.extend_from_slice(base_fields);
                    NodeShape::Object {
                        aligned_shallow_size: aligned_shallow_instance_size(&field_sizes),
                        fields: vec![
                            ("pending", pending),
                            ("values", values),
                            ("mins", node(mins)),
                            ("averages", node(averages)),
                        ],
                    }
                },
            );
        }
        unmodelled("this PackedLongValues.Builder")
    }

    /// `org.apache.lucene.util.FrequencyTrackingRingBuffer`
    /// (`FrequencyTrackingRingBuffer.java:38-41`): `int maxSize`, `int[]
    /// buffer`, `int position` and `IntBag frequencies` (`int[] keys`, `int[]
    /// freqs`, `int mask`).
    pub fn frequency_tracking_ring_buffer(
        &mut self,
        buffer: &FrequencyTrackingRingBuffer,
    ) -> Rc<JavaNode> {
        self.shared(
            buffer as *const FrequencyTrackingRingBuffer as *const (),
            "org.apache.lucene.util.FrequencyTrackingRingBuffer",
            |b| {
                let ring = b.slice_array(buffer.buffer(), "[I", 4);
                let bag: &IntBag = buffer.frequencies();
                let freqs = b.slice_array(bag.freqs(), "[I", 4);
                let keys = b.slice_array(bag.keys(), "[I", 4);
                let frequencies = JavaNode::new(
                    "org.apache.lucene.util.FrequencyTrackingRingBuffer$IntBag",
                    NodeShape::Object {
                        aligned_shallow_size: aligned_shallow_instance_size(&[REF, REF, 4]),
                        fields: vec![("freqs", node(freqs)), ("keys", node(keys))],
                    },
                );
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[4, REF, 4, REF]),
                    fields: vec![("buffer", node(ring)), ("frequencies", node(frequencies))],
                }
            },
        )
    }

    /// A page of a `PagedBytes`: a `byte[]` shared, as Java shares it, by the
    /// `PagedBytes` and every `PagedBytes.Reader` frozen from it.
    fn paged_bytes_block(&mut self, block: &Arc<Vec<u8>>) -> Rc<JavaNode> {
        self.primitive_array(Arc::as_ptr(block) as *const (), "[B", block.len(), 1)
    }

    /// A `byte[][] blocks` array of `length` slots holding `blocks` first.
    fn paged_bytes_blocks(&mut self, blocks: &[Arc<Vec<u8>>], length: usize) -> Rc<JavaNode> {
        let mut elements: Vec<NodeChild> = blocks
            .iter()
            .map(|block| node(self.paged_bytes_block(block)))
            .collect();
        elements.resize_with(length.max(blocks.len()), || NodeChild::Null);
        JavaNode::new(
            "[[B",
            NodeShape::Array {
                shallow_size: array_shallow_size(elements.len(), REF),
                elements: Some(elements),
            },
        )
    }

    /// `org.apache.lucene.util.PagedBytes` (`PagedBytes.java:35-45`): `byte[][]
    /// blocks` (an array that grows through `ArrayUtil.grow`, its first
    /// `numBlocks` slots used), `int numBlocks`, `int blockSize`, `int
    /// blockBits`, `int blockMask`, `boolean didSkipBytes`, `boolean frozen`,
    /// `int upto`, `byte[] currentBlock` and `long bytesUsedPerBlock`.
    pub fn paged_bytes(&mut self, bytes: &PagedBytes) -> Rc<JavaNode> {
        self.shared(
            bytes as *const PagedBytes as *const (),
            "org.apache.lucene.util.PagedBytes",
            |b| {
                let current_block = match bytes.current_block() {
                    Some(block) => node(b.slice_array(block, "[B", 1)),
                    None => NodeChild::Null,
                };
                let blocks = b.paged_bytes_blocks(bytes.blocks(), bytes.blocks_length());
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[
                        REF, 4, 4, 4, 4, 1, 1, 4, REF, 8,
                    ]),
                    fields: vec![("currentBlock", current_block), ("blocks", node(blocks))],
                }
            },
        )
    }

    /// `org.apache.lucene.util.PagedBytes$Reader` (`PagedBytes.java:56-60`):
    /// `byte[][] blocks` (a copy of the `numBlocks` pages), `int blockBits`,
    /// `int blockMask`, `int blockSize` and `long bytesUsedPerBlock`.
    pub fn paged_bytes_reader(&mut self, reader: &PagedBytesReader) -> Rc<JavaNode> {
        self.shared(
            reader as *const PagedBytesReader as *const (),
            "org.apache.lucene.util.PagedBytes$Reader",
            |b| {
                let blocks = b.paged_bytes_blocks(reader.blocks(), reader.blocks().len());
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[REF, 4, 4, 4, 8]),
                    fields: vec![("blocks", node(blocks))],
                }
            },
        )
    }

    /// A fresh `org.apache.lucene.util.automaton.Transition`
    /// (`Transition.java:30-42`): five `int`s, no reference.
    pub fn transition(&mut self) -> Rc<JavaNode> {
        JavaNode::new(
            "org.apache.lucene.util.automaton.Transition",
            NodeShape::Object {
                aligned_shallow_size: aligned_shallow_instance_size(&[4, 4, 4, 4, 4]),
                fields: Vec::new(),
            },
        )
    }

    /// `org.apache.lucene.util.FixedBitSet`: `long[] bits`, `int numBits`,
    /// `int numWords`.
    pub fn fixed_bit_set(&mut self, bits: &FixedBitSet) -> Rc<JavaNode> {
        self.shared(
            bits as *const FixedBitSet as *const (),
            "org.apache.lucene.util.FixedBitSet",
            |b| {
                let words = b.slice_array(bits.get_bits(), "[J", 8);
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[REF, 4, 4]),
                    fields: vec![("bits", node(words))],
                }
            },
        )
    }

    /// `org.apache.lucene.util.automaton.Automaton` (`Automaton.java:50-80,
    /// 369-371, 424-426`): `int nextState`, `int nextTransition`, `int
    /// curState`, `int[] states`, `BitSet isAccept`, `int[] transitions`,
    /// `boolean deterministic` and the two anonymous `InPlaceMergeSorter`s
    /// `destMinMaxSorter` (`Automaton$1`) and `minMaxDestSorter`
    /// (`Automaton$2`), each with `Sorter`'s `int pivotIndex` and the outer
    /// reference `this$0`.
    ///
    /// **Divergence.** The arrays have the lengths Rucene's `Automaton` holds
    /// — the used entries of `states` and `transitions`, and one `BitSet` word
    /// per 64 states (at least one) — where Java's have grown through
    /// `ArrayUtil.grow`; Rucene's automaton does not record Java's
    /// capacities, and its `ramBytesUsed` charges the same lengths.
    ///
    /// A sorter is reachable only through its automaton, so the walk has seen
    /// the automaton when it reaches `this$0`; the field is declared `null`,
    /// which adds the same nothing.
    pub fn automaton(&mut self, automaton: &Automaton) -> Rc<JavaNode> {
        self.shared(
            automaton as *const Automaton as *const (),
            "org.apache.lucene.util.automaton.Automaton",
            |b| {
                let words = automaton.get_accept_states().len().div_ceil(64).max(1) as i64;
                let is_accept = JavaNode::new(
                    "java.util.BitSet",
                    NodeShape::Simple {
                        shallow_size: BIT_SET_SHALLOW_SIZE,
                        // `BitSet.size() / Byte.SIZE`: 64 bits per word.
                        estimate: words * 64 / 8,
                    },
                );
                let sorter = |class_name: &'static str| {
                    JavaNode::new(
                        class_name,
                        NodeShape::Object {
                            aligned_shallow_size: aligned_shallow_instance_size(&[REF, 4]),
                            fields: vec![("this$0", NodeChild::Null)],
                        },
                    )
                };
                let dest_min_max_sorter = sorter("org.apache.lucene.util.automaton.Automaton$1");
                let min_max_dest_sorter = sorter("org.apache.lucene.util.automaton.Automaton$2");
                let states = b.slice_array(automaton.states_array(), "[I", 4);
                let transitions = b.slice_array(automaton.transitions_array(), "[I", 4);
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[
                        4, 4, 4, REF, REF, REF, 1, REF, REF,
                    ]),
                    fields: vec![
                        ("isAccept", node(is_accept)),
                        ("destMinMaxSorter", node(dest_min_max_sorter)),
                        ("states", node(states)),
                        ("transitions", node(transitions)),
                        ("minMaxDestSorter", node(min_max_dest_sorter)),
                    ],
                }
            },
        )
    }

    /// `org.apache.lucene.util.automaton.ByteRunAutomaton`, the fields of
    /// `RunAutomaton` (`RunAutomaton.java:46-53`): `Automaton automaton`, `int
    /// alphabetSize`, `int size`, `FixedBitSet accept`, `int[] transitions`,
    /// `int[] points` and `int[] classmap`.
    pub fn byte_run_automaton(&mut self, run_automaton: &ByteRunAutomaton) -> Rc<JavaNode> {
        self.shared(
            run_automaton as *const ByteRunAutomaton as *const (),
            "org.apache.lucene.util.automaton.ByteRunAutomaton",
            |b| {
                let inner = run_automaton.run_automaton();
                let automaton = b.automaton(inner.automaton());
                let classmap = b.slice_array(inner.classmap(), "[I", 4);
                let accept = b.fixed_bit_set(inner.accept());
                let points = b.slice_array(inner.get_char_intervals(), "[I", 4);
                let transitions = b.slice_array(inner.transitions(), "[I", 4);
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[
                        REF, 4, 4, REF, REF, REF, REF,
                    ]),
                    fields: vec![
                        ("automaton", node(automaton)),
                        ("classmap", node(classmap)),
                        ("accept", node(accept)),
                        ("points", node(points)),
                        ("transitions", node(transitions)),
                    ],
                }
            },
        )
    }

    /// One `NFARunAutomaton.DState` (`NFARunAutomaton.java:245-253`): `int[]
    /// nfaStates`, `int[] transitions` (`null` until first used), `int
    /// hashCode`, `boolean isAccept`, `Transition stepTransition`, `Transition
    /// minimalTransition`, `int computedTransitions`, `int
    /// outgoingTransitions` and the outer reference `this$0`. A state is
    /// reachable only through its automaton, so `this$0` is declared `null`,
    /// which adds the same nothing.
    fn nfa_d_state(&mut self, state: &DStateView) -> Rc<JavaNode> {
        let minimal_transition = if state.has_minimal_transition {
            node(self.transition())
        } else {
            NodeChild::Null
        };
        let transitions = match &state.transitions {
            Some(transitions) => node(self.slice_array(transitions, "[I", 4)),
            None => NodeChild::Null,
        };
        let step_transition = self.transition();
        let nfa_states = JavaNode::new(
            "[I",
            NodeShape::Array {
                shallow_size: array_shallow_size(state.nfa_states.len(), 4),
                elements: None,
            },
        );
        JavaNode::new(
            "org.apache.lucene.util.automaton.NFARunAutomaton$DState",
            NodeShape::Object {
                aligned_shallow_size: aligned_shallow_instance_size(&[
                    REF, REF, 4, 1, REF, REF, 4, 4, REF,
                ]),
                fields: vec![
                    ("minimalTransition", minimal_transition),
                    ("transitions", transitions),
                    ("stepTransition", node(step_transition)),
                    ("this$0", NodeChild::Null),
                    ("nfaStates", node(nfa_states)),
                ],
            },
        )
    }

    /// `org.apache.lucene.util.automaton.NFARunAutomaton`
    /// (`NFARunAutomaton.java:46-55`): `Automaton automaton`, `int[] points`,
    /// `Map<DState, Integer> dStateToOrd` (a `HashMap` from each state to its
    /// ordinal), `DState[] dStates` (`new DState[10]`, grown through
    /// `ArrayUtil.grow`), `int alphabetSize`, `int[] classmap`, and the
    /// reusable scratch `Operations.PointTransitionSet transitionSet` (`int
    /// count`, `PointTransitions[] points = new PointTransitions[5]`,
    /// `IntObjectHashMap map`, `boolean useHash`) and `StateSet statesSet =
    /// new StateSet(5)` (`IntIntHashMap inner`, `long hashCode`, two
    /// `boolean`s, `int[] arrayCache = new int[0]`).
    ///
    /// **Divergence.** The scratch is modelled as Java holds it before any
    /// state is determinized (every `DState.transitions` still `null`): Rucene
    /// keeps it in std collections that do not record the capacities Java's
    /// hppc maps and grown arrays reach. Once a state has been determinized,
    /// the measurement fails with an `UnsupportedOperationException`, rather
    /// than report sizes Java would not.
    pub fn nfa_run_automaton(&mut self, nfa: &NFARunAutomaton) -> Rc<JavaNode> {
        self.shared(
            nfa as *const NFARunAutomaton as *const (),
            "org.apache.lucene.util.automaton.NFARunAutomaton",
            |b| {
                let views = nfa.d_state_views();
                if views
                    .iter()
                    .any(|v| v.transitions.is_some() || v.has_minimal_transition)
                {
                    unmodelled("an NFARunAutomaton whose DFA states were determinized");
                }

                // transitionSet
                let points_slots = JavaNode::new(
                    "[Lorg.apache.lucene.util.automaton.Operations$PointTransitions;",
                    NodeShape::Array {
                        shallow_size: array_shallow_size(POINT_TRANSITION_SET_INITIAL_POINTS, REF),
                        elements: Some(
                            (0..POINT_TRANSITION_SET_INITIAL_POINTS)
                                .map(|_| NodeChild::Null)
                                .collect(),
                        ),
                    },
                );
                let map_keys = JavaNode::new(
                    "[I",
                    NodeShape::Array {
                        shallow_size: array_shallow_size(FRESH_HPPC_MAP_SLOTS, 4),
                        elements: None,
                    },
                );
                let map_values = JavaNode::new(
                    "[Ljava.lang.Object;",
                    NodeShape::Array {
                        shallow_size: array_shallow_size(FRESH_HPPC_MAP_SLOTS, REF),
                        elements: Some(
                            (0..FRESH_HPPC_MAP_SLOTS).map(|_| NodeChild::Null).collect(),
                        ),
                    },
                );
                let map = JavaNode::new(
                    "org.apache.lucene.internal.hppc.IntObjectHashMap",
                    NodeShape::Object {
                        aligned_shallow_size: aligned_shallow_instance_size(&[
                            REF, REF, 4, 4, 4, 1, 8, 4,
                        ]),
                        fields: vec![("keys", node(map_keys)), ("values", node(map_values))],
                    },
                );
                let transition_set = JavaNode::new(
                    "org.apache.lucene.util.automaton.Operations$PointTransitionSet",
                    NodeShape::Object {
                        aligned_shallow_size: aligned_shallow_instance_size(&[4, REF, REF, 1]),
                        fields: vec![("map", node(map)), ("points", node(points_slots))],
                    },
                );

                // dStates and dStateToOrd: the same DState objects.
                let states: Vec<Rc<JavaNode>> = views.iter().map(|v| b.nfa_d_state(v)).collect();
                let mut length = NFA_D_STATES_INITIAL_LENGTH;
                for ord in 0..states.len() {
                    if ord >= length {
                        // `ArrayUtil.grow(dStates, ord + 1)`.
                        length = ArrayUtil::oversize(ord + 1, REF as usize);
                    }
                }
                let mut elements: Vec<NodeChild> =
                    states.iter().map(|s| node(Rc::clone(s))).collect();
                elements.resize_with(length, || NodeChild::Null);
                let d_states = JavaNode::new(
                    "[Lorg.apache.lucene.util.automaton.NFARunAutomaton$DState;",
                    NodeShape::Array {
                        shallow_size: array_shallow_size(length, REF),
                        elements: Some(elements),
                    },
                );
                let entries: Vec<(NodeChild, NodeChild)> = states
                    .iter()
                    .enumerate()
                    .map(|(ord, s)| {
                        let ord = i32::try_from(ord).unwrap_or(i32::MAX);
                        (node(Rc::clone(s)), node(b.integer(ord)))
                    })
                    .collect();
                let d_state_to_ord = b.hash_map(entries);

                let classmap = b.slice_array(nfa.classmap(), "[I", 4);
                let automaton = b.automaton(nfa.automaton());
                let points = b.slice_array(nfa.points(), "[I", 4);

                // statesSet
                let int_array = |len: usize| {
                    JavaNode::new(
                        "[I",
                        NodeShape::Array {
                            shallow_size: array_shallow_size(len, 4),
                            elements: None,
                        },
                    )
                };
                let inner = JavaNode::new(
                    "org.apache.lucene.internal.hppc.IntIntHashMap",
                    NodeShape::Object {
                        aligned_shallow_size: aligned_shallow_instance_size(&[
                            REF, REF, 4, 4, 4, 1, 8, 4,
                        ]),
                        fields: vec![
                            ("values", node(int_array(FRESH_HPPC_MAP_SLOTS))),
                            ("keys", node(int_array(FRESH_HPPC_MAP_SLOTS))),
                        ],
                    },
                );
                let states_set = JavaNode::new(
                    "org.apache.lucene.util.automaton.StateSet",
                    NodeShape::Object {
                        aligned_shallow_size: aligned_shallow_instance_size(&[REF, 8, 1, 1, REF]),
                        fields: vec![("inner", node(inner)), ("arrayCache", node(int_array(0)))],
                    },
                );

                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[
                        REF, REF, REF, REF, 4, REF, REF, REF,
                    ]),
                    fields: vec![
                        ("transitionSet", node(transition_set)),
                        ("dStates", node(d_states)),
                        ("dStateToOrd", node(d_state_to_ord)),
                        ("classmap", node(classmap)),
                        ("automaton", node(automaton)),
                        ("points", node(points)),
                        ("statesSet", node(states_set)),
                    ],
                }
            },
        )
    }

    /// A `CompiledAutomaton.AUTOMATON_TYPE` constant: an `Enum`, measured by
    /// its shallow size alone; one node per constant.
    pub fn automaton_type(&mut self, automaton_type: AutomatonType) -> Rc<JavaNode> {
        let ordinal: usize = match automaton_type {
            AutomatonType::None => 0,
            AutomatonType::All => 1,
            AutomatonType::Single => 2,
            AutomatonType::Normal => 3,
        };
        self.shared(
            (ordinal + 1) as *const (),
            "org.apache.lucene.util.automaton.CompiledAutomaton$AUTOMATON_TYPE",
            |_| NodeShape::Enum {
                shallow_size: AUTOMATON_TYPE_SHALLOW_SIZE,
            },
        )
    }

    /// `org.apache.lucene.util.automaton.CompiledAutomaton`
    /// (`CompiledAutomaton.java:53-99, 273`): `AUTOMATON_TYPE type`, `BytesRef
    /// term`, `ByteRunAutomaton runAutomaton`, `Automaton automaton`,
    /// `NFARunAutomaton nfaRunAutomaton`, `BytesRef commonSuffixRef`, `boolean
    /// finite`, `int sinkState` and the scratch `Transition transition = new
    /// Transition()`.
    ///
    /// Java's `automaton` is `runAutomaton.automaton`, the same object
    /// (`CompiledAutomaton.java:263`), so the field refers to the run
    /// automaton's node. Rucene creates the scratch `transition` where it is
    /// used rather than storing it; the model holds the fresh `Transition`
    /// Java holds.
    pub fn compiled_automaton(&mut self, compiled: &CompiledAutomaton) -> Rc<JavaNode> {
        self.shared(
            compiled as *const CompiledAutomaton as *const (),
            "org.apache.lucene.util.automaton.CompiledAutomaton",
            |b| {
                let transition = b.transition();
                let automaton_type = b.automaton_type(compiled.automaton_type);
                let common_suffix_ref = match &compiled.common_suffix_ref {
                    Some(suffix) => node(b.bytes_ref(suffix)),
                    None => NodeChild::Null,
                };
                let term = match &compiled.term {
                    Some(term) => node(b.bytes_ref(term)),
                    None => NodeChild::Null,
                };
                let nfa_run_automaton = match &compiled.nfa_run_automaton {
                    Some(nfa) => node(b.nfa_run_automaton(nfa)),
                    None => NodeChild::Null,
                };
                let (run_automaton, automaton) = match &compiled.run_automaton {
                    Some(run_automaton) => {
                        let run = b.byte_run_automaton(run_automaton);
                        let automaton = b.automaton(run_automaton.automaton());
                        (node(run), node(automaton))
                    }
                    None => (
                        NodeChild::Null,
                        match &compiled.automaton {
                            Some(automaton) => node(b.automaton(automaton)),
                            None => NodeChild::Null,
                        },
                    ),
                };
                NodeShape::Object {
                    aligned_shallow_size: aligned_shallow_instance_size(&[
                        REF, REF, REF, REF, REF, REF, 1, 4, REF,
                    ]),
                    fields: vec![
                        ("transition", node(transition)),
                        ("type", node(automaton_type)),
                        ("commonSuffixRef", common_suffix_ref),
                        ("term", term),
                        ("nfaRunAutomaton", nfa_run_automaton),
                        ("automaton", automaton),
                        ("runAutomaton", run_automaton),
                    ],
                }
            },
        )
    }
}

/// A `java.util.concurrent.atomic.LongAdder`, one of the `SIMPLE_TYPES`,
/// estimated at 0 past its shallow size.
fn long_adder() -> Rc<JavaNode> {
    JavaNode::new(
        "java.util.concurrent.atomic.LongAdder",
        NodeShape::Simple {
            shallow_size: LONG_ADDER_SHALLOW_SIZE,
            estimate: 0,
        },
    )
}

/// An object `handleOther` ignores (`CharsetEncoder`, `CharsetDecoder`,
/// `ReentrantReadWriteLock`, `AtomicReference`).
fn ignorable(class_name: &'static str) -> Rc<JavaNode> {
    JavaNode::new(class_name, NodeShape::Ignorable)
}
