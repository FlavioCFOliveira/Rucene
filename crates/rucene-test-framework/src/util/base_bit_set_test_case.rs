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

//! Port of `org.apache.lucene.tests.util.BaseBitSetTestCase`: base test case
//! for `BitSet`s.
//!
//! The class is declared by [`class()`]; its members are the trait
//! [`BaseBitSetTestCase`], whose associated type is Java's type parameter
//! `T extends BitSet`. A subclass implements the trait (its `copyOf` and the
//! hooks it overrides) and builds its instances with
//! [`BitSetTestInstance::new`].
//!
//! **Divergences** (declared, each confined to the member named):
//! * `@Ignore` on the class: JUnit's `@Ignore` is not `@Inherited`, so it
//!   only stops the abstract class itself from running; the declaration
//!   carries the annotation for the same effect.
//! * `randomCopy` case 2 (`RoaringDocIdSet`): Java skips the `or` when the
//!   set's `iterator()` is `null` (an empty `RoaringDocIdSet`); the port's
//!   `DocIdSet.iterator()` returns an exhausted iterator instead, whose `or`
//!   leaves both sets unchanged. Neither path draws randomness.

use std::fmt;
use std::sync::{Arc, LazyLock};

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::doc_id_set::DocIdSet;
use rucene::search::NO_MORE_DOCS;
use rucene::util::bit_set::{BitSet, BitSetIterator};
use rucene::util::bit_sets::{RoaringDocIdSet, SparseFixedBitSet};
use rucene::util::doc_id_set::BitDocIdSet;
use rucene::util::{Accountable, Bits, FixedBitSet};

use crate::index::base_index_file_format_test_case::rethrow;
use crate::jdk::bit_set::JavaBitSet;
use crate::util::LuceneTestCase;

/// The declaration of `org.apache.lucene.tests.util.BaseBitSetTestCase`
/// (`@Ignore public abstract class BaseBitSetTestCase<T extends BitSet>
/// extends LuceneTestCase`) with its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        let t = |name: &str, body: fn(&dyn BitSetTests) -> Result<()>| {
            Method::new(name).throws(&["java.io.IOException"]).body(
                move |inst: &mut BitSetTestInstance| {
                    let case = Arc::clone(&inst.case);
                    body(&*case).unwrap_or_else(|e| rethrow(e))
                },
            )
        };
        Class::builder("org.apache.lucene.tests.util.BaseBitSetTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .annotate(randomizedtesting::junit::Ignore::default())
            .extends(&LuceneTestCase::class())
            .method(t("testCardinality", |c| c.test_cardinality()))
            .method(t("testPrevSetBit", |c| c.test_prev_set_bit()))
            .method(t("testNextSetBit", |c| c.test_next_set_bit()))
            .method(t("testNextSetBitInRange", |c| {
                c.test_next_set_bit_in_range()
            }))
            .method(t("testSet", |c| c.test_set()))
            .method(t("testGetAndSet", |c| c.test_get_and_set()))
            .method(t("testClear", |c| c.test_clear()))
            .method(t("testClearRange", |c| c.test_clear_range()))
            .method(t("testClearAll", |c| c.test_clear_all()))
            .method(t("testOrSparse", |c| c.test_or_sparse()))
            .method(t("testOrDense", |c| c.test_or_dense()))
            .method(t("testOrRandom", |c| c.test_or_random()))
            .build()
    });
    Arc::clone(&CLASS)
}

/// The instance of a `BaseBitSetTestCase` subclass the runner creates.
pub struct BitSetTestInstance {
    /// The subclass, as the tests it runs.
    pub case: Arc<dyn BitSetTests>,
}

impl BitSetTestInstance {
    /// The instance of a subclass.
    pub fn new<C: BaseBitSetTestCase>(case: C) -> Self {
        BitSetTestInstance {
            case: Arc::new(case),
        }
    }
}

/// The tests of `BaseBitSetTestCase`, object-safe: what the runner calls on
/// an instance, whatever its `T`.
pub trait BitSetTests: Send + Sync {
    /// `testCardinality()`.
    fn test_cardinality(&self) -> Result<()>;
    /// `testPrevSetBit()`.
    fn test_prev_set_bit(&self) -> Result<()>;
    /// `testNextSetBit()`.
    fn test_next_set_bit(&self) -> Result<()>;
    /// `testNextSetBitInRange()`.
    fn test_next_set_bit_in_range(&self) -> Result<()>;
    /// `testSet()`.
    fn test_set(&self) -> Result<()>;
    /// `testGetAndSet()`.
    fn test_get_and_set(&self) -> Result<()>;
    /// `testClear()`.
    fn test_clear(&self) -> Result<()>;
    /// `testClearRange()`.
    fn test_clear_range(&self) -> Result<()>;
    /// `testClearAll()`.
    fn test_clear_all(&self) -> Result<()>;
    /// `testOrSparse()`.
    fn test_or_sparse(&self) -> Result<()>;
    /// `testOrDense()`.
    fn test_or_dense(&self) -> Result<()>;
    /// `testOrRandom()`.
    fn test_or_random(&self) -> Result<()>;
}

impl<C: BaseBitSetTestCase> BitSetTests for C {
    fn test_cardinality(&self) -> Result<()> {
        BaseBitSetTestCase::test_cardinality(self)
    }
    fn test_prev_set_bit(&self) -> Result<()> {
        BaseBitSetTestCase::test_prev_set_bit(self)
    }
    fn test_next_set_bit(&self) -> Result<()> {
        BaseBitSetTestCase::test_next_set_bit(self)
    }
    fn test_next_set_bit_in_range(&self) -> Result<()> {
        BaseBitSetTestCase::test_next_set_bit_in_range(self)
    }
    fn test_set(&self) -> Result<()> {
        BaseBitSetTestCase::test_set(self)
    }
    fn test_get_and_set(&self) -> Result<()> {
        BaseBitSetTestCase::test_get_and_set(self)
    }
    fn test_clear(&self) -> Result<()> {
        BaseBitSetTestCase::test_clear(self)
    }
    fn test_clear_range(&self) -> Result<()> {
        BaseBitSetTestCase::test_clear_range(self)
    }
    fn test_clear_all(&self) -> Result<()> {
        BaseBitSetTestCase::test_clear_all(self)
    }
    fn test_or_sparse(&self) -> Result<()> {
        BaseBitSetTestCase::test_or_sparse(self)
    }
    fn test_or_dense(&self) -> Result<()> {
        BaseBitSetTestCase::test_or_dense(self)
    }
    fn test_or_random(&self) -> Result<()> {
        BaseBitSetTestCase::test_or_random(self)
    }
}

/// `randomSet(numBits, numBitsSet)` (package-private `static`): creates a
/// random set which has `numBitsSet` of its `numBits` bits set.
pub fn random_set(num_bits: i32, num_bits_set: i32) -> JavaBitSet {
    assert!(num_bits_set <= num_bits);
    let mut set = JavaBitSet::with_capacity(num_bits as usize);
    if num_bits_set == num_bits {
        set.set_range(0, num_bits as usize);
    } else {
        let random = LuceneTestCase::random();
        for _ in 0..num_bits_set {
            loop {
                let o = random.next_int_bound(num_bits) as usize;
                if !set.get(o) {
                    set.set(o);
                    break;
                }
            }
        }
    }
    set
}

/// `randomSet(numBits, percentSet)`: same as [`random_set`] but given a load
/// factor.
pub fn random_set_load(num_bits: i32, percent_set: f32) -> JavaBitSet {
    random_set(num_bits, (percent_set * num_bits as f32) as i32)
}

/// The load factors most tests iterate over.
const PERCENTS: [f32; 7] = [0.0, 0.01, 0.1, 0.5, 0.9, 0.99, 1.0];

/// The members of `BaseBitSetTestCase<T>`: the abstract `copyOf`, the
/// overridable `assertEquals(BitSet, T, int)` and the tests (a subclass may
/// override a test, as `TestFixedBitSet` does `testPrevSetBit`).
pub trait BaseBitSetTestCase: Send + Sync + 'static {
    /// `T extends BitSet`.
    type T: BitSet + 'static;

    /// `copyOf(BitSet bs, int length)`: create a copy of the given `BitSet`
    /// which has `length` bits.
    fn copy_of(&self, bs: &dyn BitSet, length: usize) -> Result<Self::T>;

    /// `assertEquals(BitSet set1, T set2, int maxDoc)` (`protected`).
    fn assert_equals_sets(&self, set1: &dyn BitSet, set2: &Self::T, max_doc: usize) {
        base_assert_equals_sets(set1, set2, max_doc);
    }

    /// `testCardinality()`: test the `BitSet#cardinality()` method.
    fn test_cardinality(&self) -> Result<()> {
        let num_bits = 1 + LuceneTestCase::random().next_int_bound(100000);
        for percent_set in PERCENTS {
            let set1 = JavaUtilBitSet::new(random_set_load(num_bits, percent_set), num_bits);
            let set2 = self.copy_of(&set1, num_bits as usize)?;
            Assert::assert_equals(set1.cardinality(), set2.cardinality());
        }
        Ok(())
    }

    /// `testPrevSetBit()`: test `BitSet#prevSetBit(int)`.
    fn test_prev_set_bit(&self) -> Result<()> {
        let num_bits = 1 + LuceneTestCase::random().next_int_bound(100000);
        for percent_set in PERCENTS {
            let set1 = JavaUtilBitSet::new(random_set_load(num_bits, percent_set), num_bits);
            let set2 = self.copy_of(&set1, num_bits as usize)?;
            for i in 0..num_bits {
                let (a, b) = (set1.prev_set_bit(i), set2.prev_set_bit(i));
                if a != b {
                    Assert::assert_equals_msg(Some(&i.to_string()), a, b);
                }
            }
        }
        Ok(())
    }

    /// `testNextSetBit()`: test `BitSet#nextSetBit(int)`.
    fn test_next_set_bit(&self) -> Result<()> {
        let num_bits = 1 + LuceneTestCase::random().next_int_bound(100000);
        for percent_set in PERCENTS {
            let set1 = JavaUtilBitSet::new(random_set_load(num_bits, percent_set), num_bits);
            let set2 = self.copy_of(&set1, num_bits as usize)?;
            for i in 0..num_bits {
                Assert::assert_equals(set1.next_set_bit(i), set2.next_set_bit(i));
            }
        }
        Ok(())
    }

    /// `testNextSetBitInRange()`: test `BitSet#nextSetBit(int, int)`.
    fn test_next_set_bit_in_range(&self) -> Result<()> {
        let random = LuceneTestCase::random();
        let num_bits = 1 + LuceneTestCase::random().next_int_bound(100000);
        for percent_set in PERCENTS {
            let set1 = JavaUtilBitSet::new(random_set_load(num_bits, percent_set), num_bits);
            let set2 = self.copy_of(&set1, num_bits as usize)?;
            for start in 0..num_bits {
                let end = RandomNumbers::random_int_between(&*random, start + 1, num_bits);
                let (a, b) = (
                    set1.next_set_bit_in_range(start, end),
                    set2.next_set_bit_in_range(start, end),
                );
                if a != b {
                    Assert::assert_equals_msg(
                        Some(&format!("start={start}, end={end}, numBits={num_bits}")),
                        a,
                        b,
                    );
                }
            }
        }
        Ok(())
    }

    /// `testSet()`: test the `BitSet#set` method.
    fn test_set(&self) -> Result<()> {
        let random = LuceneTestCase::random();
        let num_bits = 1 + random.next_int_bound(100000);
        let mut set1 = JavaUtilBitSet::new(random_set(num_bits, 0), num_bits);
        let mut set2 = self.copy_of(&set1, num_bits as usize)?;
        let iters = 10000 + random.next_int_bound(10000);
        for _ in 0..iters {
            let index = random.next_int_bound(num_bits) as usize;
            set1.set(index);
            set2.set(index);
        }
        self.assert_equals_sets(&set1, &set2, num_bits as usize);
        Ok(())
    }

    /// `testGetAndSet()`: test the `BitSet#getAndSet` method.
    fn test_get_and_set(&self) -> Result<()> {
        let random = LuceneTestCase::random();
        let num_bits = 1 + random.next_int_bound(100000);
        let mut set1 = JavaUtilBitSet::new(random_set(num_bits, 0), num_bits);
        let mut set2 = self.copy_of(&set1, num_bits as usize)?;
        let iters = 10000 + random.next_int_bound(10000);
        for _ in 0..iters {
            let index = random.next_int_bound(num_bits) as usize;
            let v1 = set1.get_and_set(index);
            let v2 = set2.get_and_set(index);
            Assert::assert_equals(v1, v2);
        }
        self.assert_equals_sets(&set1, &set2, num_bits as usize);
        Ok(())
    }

    /// `testClear()`: test the `BitSet#clear(int)` method.
    fn test_clear(&self) -> Result<()> {
        let random = LuceneTestCase::random();
        let num_bits = 1 + random.next_int_bound(100000);
        for percent_set in PERCENTS {
            let mut set1 = JavaUtilBitSet::new(random_set_load(num_bits, percent_set), num_bits);
            let mut set2 = self.copy_of(&set1, num_bits as usize)?;
            let iters = 1 + random.next_int_bound(num_bits * 2);
            for _ in 0..iters {
                let index = random.next_int_bound(num_bits) as usize;
                set1.clear(index);
                set2.clear(index);
            }
            self.assert_equals_sets(&set1, &set2, num_bits as usize);
        }
        Ok(())
    }

    /// `testClearRange()`: test the `BitSet#clear(int,int)` method.
    fn test_clear_range(&self) -> Result<()> {
        let random = LuceneTestCase::random();
        let num_bits = 1 + random.next_int_bound(100000);
        for percent_set in PERCENTS {
            let mut set1 = JavaUtilBitSet::new(random_set_load(num_bits, percent_set), num_bits);
            let mut set2 = self.copy_of(&set1, num_bits as usize)?;
            let iters = LuceneTestCase::at_least_with(&*random, 10);
            for _ in 0..iters {
                let from = random.next_int_bound(num_bits) as usize;
                let to = random.next_int_bound(num_bits + 1) as usize;
                set1.clear_range(from, to);
                set2.clear_range(from, to);
                self.assert_equals_sets(&set1, &set2, num_bits as usize);
            }
        }
        Ok(())
    }

    /// `testClearAll()`: test the `BitSet#clear()` method.
    fn test_clear_all(&self) -> Result<()> {
        let random = LuceneTestCase::random();
        let num_bits = 1 + random.next_int_bound(100000);
        for percent_set in PERCENTS {
            let mut set1 = JavaUtilBitSet::new(random_set_load(num_bits, percent_set), num_bits);
            let mut set2 = self.copy_of(&set1, num_bits as usize)?;
            let iters = LuceneTestCase::at_least_with(&*random, 10);
            for _ in 0..iters {
                set1.clear_all();
                set2.clear_all();
                self.assert_equals_sets(&set1, &set2, num_bits as usize);
            }
        }
        Ok(())
    }

    /// `randomCopy(BitSet set, int numBits)` (`private`).
    fn random_copy(&self, set: Arc<JavaUtilBitSet>, num_bits: usize) -> Result<Box<dyn DocIdSet>> {
        Ok(match LuceneTestCase::random().next_int_bound(5) {
            0 => {
                let cardinality = set.cardinality() as i64;
                Box::new(BitDocIdSet::new(set, cardinality)?)
            }
            1 => {
                let cardinality = set.cardinality() as i64;
                Box::new(BitDocIdSet::new(
                    Arc::new(self.copy_of(&*set, num_bits)?),
                    cardinality,
                )?)
            }
            2 => {
                let mut builder = RoaringDocIdSet::builder(num_bits);
                let mut i = set.next_set_bit(0);
                while i != NO_MORE_DOCS {
                    builder.add(i as usize)?;
                    i = if i as usize + 1 >= num_bits {
                        NO_MORE_DOCS
                    } else {
                        set.next_set_bit(i + 1)
                    };
                }
                Box::new(builder.build())
            }
            3 => {
                let mut fbs = FixedBitSet::new(num_bits);
                fbs.or(&mut BitSetIterator::new(set, 0)?)?;
                Box::new(BitDocIdSet::from_set(Arc::new(fbs))?)
            }
            4 => {
                let mut sfbs = SparseFixedBitSet::new(num_bits)?;
                sfbs.or(&mut BitSetIterator::new(set, 0)?)?;
                Box::new(BitDocIdSet::from_set(Arc::new(sfbs))?)
            }
            _ => Assert::fail_without_message(),
        })
    }

    /// `testOr(float load)` (`private`).
    fn test_or(&self, load: f32) -> Result<()> {
        let num_bits = 1 + LuceneTestCase::random().next_int_bound(100000);
        let mut set1 = JavaUtilBitSet::new(random_set(num_bits, 0), num_bits); // empty
        let mut set2 = self.copy_of(&set1, num_bits as usize)?;
        let iterations = LuceneTestCase::at_least(10);
        for _ in 0..iterations {
            let other_set = self.random_copy(
                Arc::new(JavaUtilBitSet::new(
                    random_set_load(num_bits, load),
                    num_bits,
                )),
                num_bits as usize,
            )?;
            let mut other_iterator = other_set.iterator()?;
            set1.or(&mut *other_iterator)?;
            set2.or(&mut *other_set.iterator()?)?;
            self.assert_equals_sets(&set1, &set2, num_bits as usize);
        }
        Ok(())
    }

    /// `testOrSparse()`: test `BitSet#or(DocIdSetIterator)` on sparse sets.
    fn test_or_sparse(&self) -> Result<()> {
        self.test_or(0.001)
    }

    /// `testOrDense()`: test `BitSet#or(DocIdSetIterator)` on dense sets.
    fn test_or_dense(&self) -> Result<()> {
        self.test_or(0.5)
    }

    /// `testOrRandom()`: test `BitSet#or(DocIdSetIterator)` on a random
    /// density.
    fn test_or_random(&self) -> Result<()> {
        let load = LuceneTestCase::random().next_float();
        self.test_or(load)
    }
}

/// `BaseBitSetTestCase.assertEquals(BitSet, T, int)`, the body an override
/// calls as `super.assertEquals`.
pub fn base_assert_equals_sets(set1: &dyn BitSet, set2: &dyn BitSet, max_doc: usize) {
    for i in 0..max_doc {
        let (a, b) = (set1.get(i), set2.get(i));
        if a != b {
            Assert::assert_equals_msg(Some(&format!("Different at {i}")), a, b);
        }
    }
}

/// `JavaUtilBitSet` (`private static class`): a Lucene `BitSet` over a
/// `java.util.BitSet` of `numBits` bits.
#[derive(Debug)]
pub struct JavaUtilBitSet {
    bit_set: JavaBitSet,
    num_bits: i32,
}

impl JavaUtilBitSet {
    /// `JavaUtilBitSet(java.util.BitSet bitSet, int numBits)`.
    pub fn new(bit_set: JavaBitSet, num_bits: i32) -> Self {
        JavaUtilBitSet { bit_set, num_bits }
    }
}

impl fmt::Display for JavaUtilBitSet {
    /// `Object.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "org.apache.lucene.tests.util.BaseBitSetTestCase$JavaUtilBitSet@{:x}",
            self as *const Self as usize as u32
        )
    }
}

impl Bits for JavaUtilBitSet {
    fn get(&self, index: usize) -> bool {
        self.bit_set.get(index)
    }

    fn length(&self) -> usize {
        self.num_bits as usize
    }
}

impl Accountable for JavaUtilBitSet {
    fn ram_bytes_used(&self) -> i64 {
        -1
    }
}

impl BitSet for JavaUtilBitSet {
    fn clear_all(&mut self) {
        self.bit_set.clear_all();
    }

    fn set(&mut self, i: usize) {
        self.bit_set.set(i);
    }

    fn get_and_set(&mut self, index: usize) -> bool {
        let v = self.get(index);
        self.set(index);
        v
    }

    fn clear(&mut self, index: usize) {
        self.bit_set.clear(index);
    }

    fn clear_range(&mut self, start_index: usize, end_index: usize) {
        if start_index >= end_index {
            return;
        }
        self.bit_set.clear_range(start_index, end_index);
    }

    fn cardinality(&self) -> usize {
        self.bit_set.cardinality()
    }

    fn approximate_cardinality(&self) -> usize {
        self.bit_set.cardinality()
    }

    fn prev_set_bit(&self, index: i32) -> i32 {
        self.bit_set.previous_set_bit(index)
    }

    fn next_set_bit_in_range(&self, start: i32, upper_bound: i32) -> i32 {
        let mut next = self.bit_set.next_set_bit(start as usize);
        if next == -1 || next >= upper_bound {
            next = NO_MORE_DOCS;
        }
        next
    }

    fn next_clear_bit_in_range(&self, start: i32, upper_bound: i32) -> i32 {
        let mut next = self.bit_set.next_clear_bit(start as usize);
        if next == -1 || next >= upper_bound {
            next = NO_MORE_DOCS;
        }
        next
    }
}
