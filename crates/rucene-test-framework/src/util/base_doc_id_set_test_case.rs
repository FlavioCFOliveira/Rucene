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

//! Port of `org.apache.lucene.tests.util.BaseDocIdSetTestCase`: base test
//! class for `DocIdSet`s.
//!
//! The class is declared by [`class()`]; its members are the trait
//! [`BaseDocIdSetTestCase`], whose associated type is Java's type parameter
//! `T extends DocIdSet`.
//!
//! **Divergences** (declared, each confined to the member named):
//! * `iterator() == null`: the port's `DocIdSet.iterator()` never returns
//!   `null` (an empty set gives an exhausted iterator), so the `null`
//!   branches of `assertEquals` and `testIntoBitSet` are unreachable and
//!   not ported; the non-`null` branches draw the same randomness.
//! * `ramBytesUsed(DocIdSet, int)`: Java measures the copy with
//!   `RamUsageTester`'s reflective walk; the port's walk reads the Java
//!   layout each type describes through [`RamObject`]
//!   (`ram_usage_tester`'s declared divergence), so `T` is a [`RamObject`]
//!   too. [`JavaShape`] and [`JavaShaped`] let a subclass describe the Java
//!   layout of the set it builds.

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::doc_id_set::DocIdSet;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::NO_MORE_DOCS;
use rucene::util::{Accountable, Bits, FixedBitSet, RamUsageEstimator};

use crate::index::base_index_file_format_test_case::rethrow;
use crate::jdk::bit_set::JavaBitSet;
use crate::util::base_bit_set_test_case::random_set_load;
use crate::util::lucene_test_case::TEST_NIGHTLY;
use crate::util::ram_usage_tester::{
    aligned_shallow_instance_size, array_shallow_size, JavaObject, RamObject,
};
use crate::util::{LuceneTestCase, RamUsageTester, TestUtil};

/// The declaration of `org.apache.lucene.tests.util.BaseDocIdSetTestCase`
/// (`public abstract class BaseDocIdSetTestCase<T extends DocIdSet> extends
/// LuceneTestCase`) with its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        let t = |name: &str, body: fn(&dyn DocIdSetTests) -> Result<()>| {
            Method::new(name).throws(&["java.io.IOException"]).body(
                move |inst: &mut DocIdSetTestInstance| {
                    let case = Arc::clone(&inst.case);
                    body(&*case).unwrap_or_else(|e| rethrow(e))
                },
            )
        };
        Class::builder("org.apache.lucene.tests.util.BaseDocIdSetTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .method(t("testNoBit", |c| c.test_no_bit()))
            .method(t("test1Bit", |c| c.test_1_bit()))
            .method(t("test2Bits", |c| c.test_2_bits()))
            .method(t("testAgainstBitSet", |c| c.test_against_bit_set()))
            .method(t("testRamBytesUsed", |c| c.test_ram_bytes_used()))
            .method(t("testIntoBitSet", |c| c.test_into_bit_set()))
            .method(t("testIntoBitSetBoundChecks", |c| {
                c.test_into_bit_set_bound_checks()
            }))
            .build()
    });
    Arc::clone(&CLASS)
}

/// The instance of a `BaseDocIdSetTestCase` subclass the runner creates.
pub struct DocIdSetTestInstance {
    /// The subclass, as the tests it runs.
    pub case: Arc<dyn DocIdSetTests>,
}

impl DocIdSetTestInstance {
    /// The instance of a subclass.
    pub fn new<C: BaseDocIdSetTestCase>(case: C) -> Self {
        DocIdSetTestInstance {
            case: Arc::new(case),
        }
    }
}

/// The tests of `BaseDocIdSetTestCase`, object-safe.
pub trait DocIdSetTests: Send + Sync {
    /// `testNoBit()`.
    fn test_no_bit(&self) -> Result<()>;
    /// `test1Bit()`.
    fn test_1_bit(&self) -> Result<()>;
    /// `test2Bits()`.
    fn test_2_bits(&self) -> Result<()>;
    /// `testAgainstBitSet()`.
    fn test_against_bit_set(&self) -> Result<()>;
    /// `testRamBytesUsed()`.
    fn test_ram_bytes_used(&self) -> Result<()>;
    /// `testIntoBitSet()`.
    fn test_into_bit_set(&self) -> Result<()>;
    /// `testIntoBitSetBoundChecks()`.
    fn test_into_bit_set_bound_checks(&self) -> Result<()>;
}

impl<C: BaseDocIdSetTestCase> DocIdSetTests for C {
    fn test_no_bit(&self) -> Result<()> {
        BaseDocIdSetTestCase::test_no_bit(self)
    }
    fn test_1_bit(&self) -> Result<()> {
        BaseDocIdSetTestCase::test_1_bit(self)
    }
    fn test_2_bits(&self) -> Result<()> {
        BaseDocIdSetTestCase::test_2_bits(self)
    }
    fn test_against_bit_set(&self) -> Result<()> {
        BaseDocIdSetTestCase::test_against_bit_set(self)
    }
    fn test_ram_bytes_used(&self) -> Result<()> {
        BaseDocIdSetTestCase::test_ram_bytes_used(self)
    }
    fn test_into_bit_set(&self) -> Result<()> {
        BaseDocIdSetTestCase::test_into_bit_set(self)
    }
    fn test_into_bit_set_bound_checks(&self) -> Result<()> {
        BaseDocIdSetTestCase::test_into_bit_set_bound_checks(self)
    }
}

/// The members of `BaseDocIdSetTestCase<T>`.
pub trait BaseDocIdSetTestCase: Send + Sync + 'static {
    /// `T extends DocIdSet`; a [`RamObject`] for `ramBytesUsed`.
    type T: DocIdSet + RamObject + 'static;

    /// `copyOf(BitSet bs, int length)`: create a copy of the given `BitSet`
    /// which has `length` bits.
    fn copy_of(&self, bs: &JavaBitSet, length: i32) -> Result<Self::T>;

    /// `assertEquals(int numBits, BitSet ds1, T ds2)`: assert that the
    /// content of the `DocIdSet` is the same as the content of the `BitSet`.
    fn assert_equals_sets(&self, num_bits: i32, ds1: &JavaBitSet, ds2: &Self::T) -> Result<()> {
        base_assert_equals_sets(num_bits, ds1, ds2)
    }

    /// `testNoBit()`: test length=0.
    fn test_no_bit(&self) -> Result<()> {
        let bs = JavaBitSet::with_capacity(1);
        let copy = self.copy_of(&bs, 1)?;
        self.assert_equals_sets(1, &bs, &copy)
    }

    /// `test1Bit()`: test length=1.
    fn test_1_bit(&self) -> Result<()> {
        let mut bs = JavaBitSet::with_capacity(1);
        if LuceneTestCase::random().next_boolean() {
            bs.set(0);
        }
        let copy = self.copy_of(&bs, 1)?;
        self.assert_equals_sets(1, &bs, &copy)
    }

    /// `test2Bits()`: test length=2.
    fn test_2_bits(&self) -> Result<()> {
        let mut bs = JavaBitSet::with_capacity(2);
        if LuceneTestCase::random().next_boolean() {
            bs.set(0);
        }
        if LuceneTestCase::random().next_boolean() {
            bs.set(1);
        }
        let copy = self.copy_of(&bs, 2)?;
        self.assert_equals_sets(2, &bs, &copy)
    }

    /// `testAgainstBitSet()`: compare the content of the set against a
    /// `BitSet`.
    fn test_against_bit_set(&self) -> Result<()> {
        let random = LuceneTestCase::random();
        let num_bits = TestUtil::next_int(&*random, 100, 1 << 20);
        // test various random sets with various load factors
        let percents = [0f32, 0.0001, random.next_float(), 0.9, 1.0];
        for percent_set in percents {
            let set = random_set_load(num_bits, percent_set);
            let copy = self.copy_of(&set, num_bits)?;
            self.assert_equals_sets(num_bits, &set, &copy)?;
        }
        // test one doc
        let mut set = JavaBitSet::with_capacity(num_bits as usize);
        set.set(0); // 0 first
        let copy = self.copy_of(&set, num_bits)?;
        self.assert_equals_sets(num_bits, &set, &copy)?;
        set.clear(0);
        set.set(random.next_int_bound(num_bits) as usize);
        let copy = self.copy_of(&set, num_bits)?; // then random index
        self.assert_equals_sets(num_bits, &set, &copy)?;
        // test regular increments
        let max_iterations = if *TEST_NIGHTLY { i32::MAX } else { 10 };
        let mut iterations = 0;
        let mut inc = 2;
        while inc < 1000 {
            // don't let this test run too many times, even if it gets unlucky with "inc"
            if iterations >= max_iterations {
                break;
            }
            iterations += 1;
            let mut set = JavaBitSet::with_capacity(num_bits as usize);
            let mut d = random.next_int_bound(10);
            while d < num_bits {
                set.set(d as usize);
                d += inc;
            }
            let copy = self.copy_of(&set, num_bits)?;
            self.assert_equals_sets(num_bits, &set, &copy)?;
            inc += TestUtil::next_int(&*random, 1, 100);
        }
        Ok(())
    }

    /// `testRamBytesUsed()`: test ram usage estimation.
    fn test_ram_bytes_used(&self) -> Result<()> {
        let random = LuceneTestCase::random();
        let iters = 100;
        for _ in 0..iters {
            let pow = random.next_int_bound(20);
            let max_doc = TestUtil::next_int(&*random, 1, 1 << pow);
            let num_docs = TestUtil::next_int(
                &*random,
                0,
                max_doc.min(1 << TestUtil::next_int(&*random, 0, pow)),
            );
            let set = crate::util::base_bit_set_test_case::random_set(max_doc, num_docs);
            let copy = self.copy_of(&set, max_doc)?;
            let actual_bytes = self.ram_bytes_used(&copy, max_doc)?;
            let expected_bytes = copy.ram_bytes_used();
            Assert::assert_equals(expected_bytes, actual_bytes);
        }
        Ok(())
    }

    /// `ramBytesUsed(DocIdSet set, int length)` (`private`): same as
    /// `RamUsageTester.sizeOf` but tries to not take into account resources
    /// that might be shared across instances.
    fn ram_bytes_used(&self, set: &Self::T, length: i32) -> Result<i64> {
        let o1 = self.copy_of(&JavaBitSet::with_capacity(length as usize), length)?;
        let bytes1 = RamUsageTester::ram_used(&Dummy {
            o1: Some(&o1),
            o2: Some(set),
        });
        let bytes2 = RamUsageTester::ram_used(&Dummy {
            o1: Some(&o1),
            o2: None,
        });
        Ok(bytes1 - bytes2)
    }

    /// `testIntoBitSet()`.
    fn test_into_bit_set(&self) -> Result<()> {
        let random = LuceneTestCase::random();
        let num_bits = TestUtil::next_int(&*random, 100, 1 << 20);
        // test various random sets with various load factors
        let percents = [0f32, 0.0001, random.next_float(), 0.9, 1.0];
        for percent_set in percents {
            let set = random_set_load(num_bits, percent_set);
            let copy = self.copy_of(&set, num_bits)?;
            let from = TestUtil::next_int(&*LuceneTestCase::random(), 0, num_bits - 1);
            let to = TestUtil::next_int(&*LuceneTestCase::random(), from, num_bits + 5);
            let mut actual = FixedBitSet::new((to - from) as usize);
            let mut it1 = copy.iterator()?;
            let from_doc = it1.advance(from)?;
            // No docs to set
            it1.into_bit_set(from, &mut actual, from)?;
            Assert::assert_true(actual.scan_is_empty());
            Assert::assert_equals(from_doc, it1.doc_id());
            // Now actually set some bits
            it1.into_bit_set(to, &mut actual, from)?;
            let mut expected = FixedBitSet::new((to - from) as usize);
            let mut it2 = copy.iterator()?;
            let mut doc = it2.advance(from)?;
            while doc < to {
                expected.set((doc - from) as usize);
                doc = it2.next_doc()?;
            }
            if expected != actual {
                // assertEquals(Object, Object): FixedBitSet.equals
                Assert::fail(&format!("expected:<{expected}> but was:<{actual}>"));
            }
            // Check if docID() / nextDoc() return the same value after #intoBitSet has been called.
            Assert::assert_equals(it2.doc_id(), it1.doc_id());
            if it2.doc_id() != NO_MORE_DOCS {
                Assert::assert_equals(it2.next_doc()?, it1.next_doc()?);
            }
        }
        Ok(())
    }

    /// `testIntoBitSetBoundChecks()`.
    fn test_into_bit_set_bound_checks(&self) -> Result<()> {
        let mut set = JavaBitSet::new();
        set.set(20);
        set.set(42);
        let copy = self.copy_of(&set, 256)?;
        let from = TestUtil::next_int(&*LuceneTestCase::random(), 0, 20);
        let to = TestUtil::next_int(&*LuceneTestCase::random(), 43, 256);
        let offset = TestUtil::next_int(&*LuceneTestCase::random(), 0, from);
        let mut dest1 = FixedBitSet::new((42 - offset + 1) as usize);
        let mut it1 = copy.iterator()?;
        it1.advance(from)?;
        // This call is legal, since all "set" bits are in the range
        it1.into_bit_set(to, &mut dest1, offset)?;
        for i in 0..dest1.length() as i32 {
            Assert::assert_equals(offset + i == 20 || offset + i == 42, dest1.get(i as usize));
        }
        let mut dest2 = FixedBitSet::new((42 - offset) as usize);
        let mut it2 = copy.iterator()?;
        it2.advance(from)?;
        // This call is not legal, since there is one bit that is set beyond the end of the target bit
        // set
        LuceneTestCase::expect_throws("java.lang.Throwable", || {
            it2.into_bit_set(to, &mut dest2, offset)
                .map_err(|e| to_throwable(&e))
        });
        let mut dest3 = FixedBitSet::new((42 - offset + 1) as usize);
        let mut it3 = copy.iterator()?;
        it3.advance(from)?;
        // This call is not legal, since offset is greater than the current doc
        LuceneTestCase::expect_throws("java.lang.Throwable", || {
            it3.into_bit_set(to, &mut dest3, 21)
                .map_err(|e| to_throwable(&e))
        });
        Ok(())
    }
}

/// The Java throwable a Rucene error stands for.
fn to_throwable(e: &rucene::error::LuceneError) -> randomizedtesting::jdk::exception::Throwable {
    let (class, message) = crate::analysis::base_token_stream_test_case::java_class_and_message(e);
    randomizedtesting::jdk::exception::Throwable::new(class, message)
}

/// `BaseDocIdSetTestCase.assertEquals(int, BitSet, T)`, the body an override
/// calls as `super.assertEquals`.
pub fn base_assert_equals_sets(num_bits: i32, ds1: &JavaBitSet, ds2: &dyn DocIdSet) -> Result<()> {
    let random = LuceneTestCase::random();
    // nextDoc
    let mut it2 = ds2.iterator()?;
    Assert::assert_equals(-1, it2.doc_id());
    let mut doc = ds1.next_set_bit(0);
    while doc != -1 {
        Assert::assert_equals(doc, it2.next_doc()?);
        Assert::assert_equals(doc, it2.doc_id());
        doc = ds1.next_set_bit(doc as usize + 1);
    }
    Assert::assert_equals(NO_MORE_DOCS, it2.next_doc()?);
    Assert::assert_equals(NO_MORE_DOCS, it2.doc_id());

    // nextDoc / advance
    let mut it2 = ds2.iterator()?;
    let mut doc = -1;
    while doc != NO_MORE_DOCS {
        if random.next_boolean() {
            doc = ds1.next_set_bit((doc + 1) as usize);
            if doc == -1 {
                doc = NO_MORE_DOCS;
            }
            Assert::assert_equals(doc, it2.next_doc()?);
            Assert::assert_equals(doc, it2.doc_id());
        } else {
            let bound = if random.next_boolean() {
                64
            } else {
                (num_bits / 8).max(1)
            };
            let target = doc + 1 + random.next_int_bound(bound);
            doc = ds1.next_set_bit(target as usize);
            if doc == -1 {
                doc = NO_MORE_DOCS;
            }
            Assert::assert_equals(doc, it2.advance(target)?);
            Assert::assert_equals(doc, it2.doc_id());
        }
    }

    // bits()
    if let Some(bits) = ds2.bits()? {
        // test consistency between bits and iterator
        let mut it2 = ds2.iterator()?;
        let mut previous_doc = -1;
        let mut doc = it2.next_doc()?;
        loop {
            let max = if doc == NO_MORE_DOCS {
                bits.length() as i32
            } else {
                doc
            };
            for i in previous_doc + 1..max {
                Assert::assert_equals(false, bits.get(i as usize));
            }
            if doc == NO_MORE_DOCS {
                break;
            }
            Assert::assert_equals(true, bits.get(doc as usize));
            previous_doc = doc;
            doc = it2.next_doc()?;
        }
    }
    Ok(())
}

/// `Dummy` (`private static class`): two `Object` fields.
struct Dummy<'a> {
    o1: Option<&'a dyn RamObject>,
    o2: Option<&'a dyn RamObject>,
}

impl RamObject for Dummy<'_> {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.util.BaseDocIdSetTestCase$Dummy"
    }

    fn java_object(&self) -> JavaObject<'_> {
        JavaObject::Object {
            aligned_shallow_size: aligned_shallow_instance_size(&[
                RamUsageEstimator::NUM_BYTES_OBJECT_REF,
                RamUsageEstimator::NUM_BYTES_OBJECT_REF,
            ]),
            fields: vec![("o1", self.o1), ("o2", self.o2)],
        }
    }
}

// -----------------------------------------------------------------------------
// The Java layout of a set
// -----------------------------------------------------------------------------

/// The Java layout of an object graph, for [`RamUsageTester`]: what Java's
/// reflective walk would read. A shared [`Arc`] is one Java object (its
/// identity is its address), as a shared singleton is.
#[derive(Debug)]
pub enum JavaShape {
    /// A plain object: its class, its fields' sizes and its reference
    /// fields.
    Object {
        /// `getClass().getName()`.
        class: &'static str,
        /// The sizes of every non-static field (`NUM_BYTES_OBJECT_REF` for
        /// a reference).
        field_sizes: Vec<i64>,
        /// The reference fields, `null` as `None`.
        fields: Vec<(&'static str, Option<Arc<JavaShape>>)>,
    },
    /// A primitive array of `len` elements of `element_bytes` each.
    PrimitiveArray {
        /// The array class (`[J`, `[I`, `[S`).
        class: &'static str,
        /// The length.
        len: usize,
        /// The element size.
        element_bytes: i64,
    },
    /// An array of references.
    ObjectArray {
        /// The array class (`[Lorg.apache.lucene.search.DocIdSet;`).
        class: &'static str,
        /// The elements, `null` as `None`.
        elements: Vec<Option<Arc<JavaShape>>>,
    },
}

impl JavaShape {
    /// A `long[]` of `len` elements.
    pub fn longs(len: usize) -> Arc<JavaShape> {
        Arc::new(JavaShape::PrimitiveArray {
            class: "[J",
            len,
            element_bytes: 8,
        })
    }

    /// An `int[]` of `len` elements.
    pub fn ints(len: usize) -> Arc<JavaShape> {
        Arc::new(JavaShape::PrimitiveArray {
            class: "[I",
            len,
            element_bytes: 4,
        })
    }

    /// A `short[]` of `len` elements.
    pub fn shorts(len: usize) -> Arc<JavaShape> {
        Arc::new(JavaShape::PrimitiveArray {
            class: "[S",
            len,
            element_bytes: 2,
        })
    }

    /// `org.apache.lucene.util.FixedBitSet`: `long[] bits`, `int numBits`,
    /// `int numWords`.
    pub fn fixed_bit_set(words: usize) -> Arc<JavaShape> {
        Arc::new(JavaShape::Object {
            class: "org.apache.lucene.util.FixedBitSet",
            field_sizes: vec![RamUsageEstimator::NUM_BYTES_OBJECT_REF, 4, 4],
            fields: vec![("bits", Some(JavaShape::longs(words)))],
        })
    }

    /// `org.apache.lucene.util.BitDocIdSet`: `BitSet set`, `long cost`.
    pub fn bit_doc_id_set(set: Arc<JavaShape>) -> Arc<JavaShape> {
        Arc::new(JavaShape::Object {
            class: "org.apache.lucene.util.BitDocIdSet",
            field_sizes: vec![RamUsageEstimator::NUM_BYTES_OBJECT_REF, 8],
            fields: vec![("set", Some(set))],
        })
    }

    /// `org.apache.lucene.util.NotDocIdSet`: `int maxDoc`, `DocIdSet in`.
    pub fn not_doc_id_set(inner: Arc<JavaShape>) -> Arc<JavaShape> {
        Arc::new(JavaShape::Object {
            class: "org.apache.lucene.util.NotDocIdSet",
            field_sizes: vec![4, RamUsageEstimator::NUM_BYTES_OBJECT_REF],
            fields: vec![("in", Some(inner))],
        })
    }
}

impl RamObject for JavaShape {
    fn java_class_name(&self) -> &'static str {
        match self {
            JavaShape::Object { class, .. }
            | JavaShape::PrimitiveArray { class, .. }
            | JavaShape::ObjectArray { class, .. } => class,
        }
    }

    fn java_object(&self) -> JavaObject<'_> {
        match self {
            JavaShape::Object {
                field_sizes,
                fields,
                ..
            } => JavaObject::Object {
                aligned_shallow_size: aligned_shallow_instance_size(field_sizes),
                fields: fields
                    .iter()
                    .map(|(name, value)| (*name, value.as_deref().map(|v| v as &dyn RamObject)))
                    .collect(),
            },
            JavaShape::PrimitiveArray {
                len, element_bytes, ..
            } => JavaObject::Array {
                shallow_size: array_shallow_size(*len, *element_bytes),
                elements: None,
            },
            JavaShape::ObjectArray { elements, .. } => JavaObject::Array {
                shallow_size: array_shallow_size(
                    elements.len(),
                    RamUsageEstimator::NUM_BYTES_OBJECT_REF,
                ),
                elements: Some(
                    elements
                        .iter()
                        .map(|e| e.as_deref().map(|v| v as &dyn RamObject))
                        .collect(),
                ),
            },
        }
    }
}

/// A set together with the Java layout of its Java counterpart: the `T` of
/// a subclass whose set type has no [`RamObject`] of its own. It is the set
/// for every other purpose ([`std::ops::Deref`], [`DocIdSet`],
/// [`Accountable`]).
pub struct JavaShaped<D> {
    /// The set.
    pub set: D,
    /// Its Java layout.
    pub shape: Arc<JavaShape>,
}

impl<D> std::ops::Deref for JavaShaped<D> {
    type Target = D;

    fn deref(&self) -> &D {
        &self.set
    }
}

impl<D: Accountable> Accountable for JavaShaped<D> {
    fn ram_bytes_used(&self) -> i64 {
        self.set.ram_bytes_used()
    }
}

impl<D: DocIdSet> DocIdSet for JavaShaped<D> {
    fn iterator(&self) -> Result<Box<dyn DocIdSetIterator>> {
        self.set.iterator()
    }

    fn bits(&self) -> Result<Option<Box<dyn Bits>>> {
        self.set.bits()
    }
}

impl<D> RamObject for JavaShaped<D> {
    fn java_class_name(&self) -> &'static str {
        self.shape.java_class_name()
    }

    fn java_object(&self) -> JavaObject<'_> {
        self.shape.java_object()
    }
}
