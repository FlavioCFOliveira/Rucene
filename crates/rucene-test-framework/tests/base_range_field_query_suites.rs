//! The `BaseRangeFieldQueryTestCase` suites (rmp #356): the inherited cases
//! run, under the ported runner and `LuceneTestCase`'s rule chain, on the
//! range fields Lucene's own subclasses bind
//! (`lucene/core/src/test/org/apache/lucene/search/`):
//! `TestIntRangeFieldQueries`, `TestLongRangeFieldQueries`,
//! `TestFloatRangeFieldQueries`, `TestDoubleRangeFieldQueries` (each with its
//! `nextRange` and its `…TestRange` box) and `TestInetAddressRangeQueries`
//! (its `IpRange` box and its overrides of the inherited tests, which swap
//! `testMultiValued` and `testRandomMedium`).
//!
//! The classes' own extra test methods (`testBasics`) belong to the
//! core-test port (sprint 7) and are not declared here.
//!
//! `RUCENE_SUITE_SEEDS=<n>` runs each suite on the first `n` seeds of
//! `suite_seeds::SEEDS`; `test_range_field_suite_draws_match_lucene`
//! compares the seeded draws with `BaseSuitesDrawsOracle`'s.

use std::fmt;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use randomizedtesting::junit::{AfterClass, BeforeClass};
use rucene::document::geo_fields::{InetAddressPoint, InetAddressRange};
use rucene::document::range_fields::{DoubleRange, FloatRange, IntRange, LongRange};
use rucene::error::Result;
use rucene::index::IndexableField;
use rucene::search::query::Query;
use rucene_test_framework::search::base_range_field_query_test_case::{
    self, range_test, BaseRangeFieldQueryTestCase, Range, RangeTestInstance, RangeValue,
};
use rucene_test_framework::util::lucene_test_case::{LuceneTestCaseExecutor, Nightly};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod draw_hooks;
mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

/// A numeric type of the four numeric range bindings, with what each
/// binding draws and builds.
trait RangeNumber: Copy + PartialOrd + fmt::Display + Send + Sync + 'static {
    /// The binding's `FIELD_NAME`.
    const FIELD_NAME: &'static str;
    /// `next…Internal()`.
    fn next_internal() -> Self;
    /// `Math.min`.
    fn min(a: Self, b: Self) -> Self;
    /// `Math.max`.
    fn max(a: Self, b: Self) -> Self;
    /// `Arrays.equals` of two arrays (bit equality for floating point).
    fn arrays_equal(a: &[Self], b: &[Self]) -> bool;
    /// The value as a `RangeValue`.
    fn to_value(self) -> RangeValue;
    /// The `(Integer) val` cast.
    fn from_value(v: &RangeValue) -> Self;
    /// `new …Range(FIELD_NAME, min, max)`.
    fn new_field(min: &[Self], max: &[Self]) -> Result<Box<dyn IndexableField>>;
    /// `…Range.new<Relation>Query(FIELD_NAME, min, max)`.
    fn new_query(relation: usize, min: &[Self], max: &[Self]) -> Result<Arc<dyn Query>>;
}

/// The value `RangeValue` carries for `T`, or `ClassCastException`.
fn class_cast() -> ! {
    randomizedtesting::jdk::exception::throw_without_message("java.lang.ClassCastException")
}

macro_rules! range_queries {
    ($range:ident, $relation:expr, $min:expr, $max:expr) => {{
        let q: Arc<dyn Query> = match $relation {
            0 => Arc::new($range::new_intersects_query(Self::FIELD_NAME, $min, $max)?),
            1 => Arc::new($range::new_contains_query(Self::FIELD_NAME, $min, $max)?),
            2 => Arc::new($range::new_within_query(Self::FIELD_NAME, $min, $max)?),
            _ => Arc::new($range::new_crosses_query(Self::FIELD_NAME, $min, $max)?),
        };
        Ok(q)
    }};
}

impl RangeNumber for i32 {
    const FIELD_NAME: &'static str = "intRangeField";

    fn next_internal() -> Self {
        let random = LuceneTestCase::random();
        match random.next_int_bound(5) {
            0 => i32::MIN,
            1 => i32::MAX,
            _ => {
                let bpv = random.next_int_bound(32);
                match bpv {
                    32 => random.next_int(),
                    _ => {
                        let mut v = TestUtil::next_int(&*random, 0, ((1i64 << bpv) - 1) as i32);
                        if bpv > 0 {
                            // negative values sometimes
                            v -= 1 << (bpv - 1);
                        }
                        v
                    }
                }
            }
        }
    }
    fn min(a: Self, b: Self) -> Self {
        a.min(b)
    }
    fn max(a: Self, b: Self) -> Self {
        a.max(b)
    }
    fn arrays_equal(a: &[Self], b: &[Self]) -> bool {
        a == b
    }
    fn to_value(self) -> RangeValue {
        RangeValue::Int(self)
    }
    fn from_value(v: &RangeValue) -> Self {
        match v {
            RangeValue::Int(v) => *v,
            _ => class_cast(),
        }
    }
    fn new_field(min: &[Self], max: &[Self]) -> Result<Box<dyn IndexableField>> {
        Ok(Box::new(IntRange::new(Self::FIELD_NAME, min, max)?))
    }
    fn new_query(relation: usize, min: &[Self], max: &[Self]) -> Result<Arc<dyn Query>> {
        range_queries!(IntRange, relation, min, max)
    }
}

impl RangeNumber for i64 {
    const FIELD_NAME: &'static str = "longRangeField";

    fn next_internal() -> Self {
        let random = LuceneTestCase::random();
        match random.next_int_bound(5) {
            0 => i64::MIN,
            1 => i64::MAX,
            _ => {
                let bpv = random.next_int_bound(64);
                match bpv {
                    64 => random.next_long(),
                    _ => {
                        let mut v = TestUtil::next_long(&*random, 0, (1i64 << bpv).wrapping_sub(1));
                        if bpv > 0 {
                            // negative values sometimes
                            v -= 1i64 << (bpv - 1);
                        }
                        v
                    }
                }
            }
        }
    }
    fn min(a: Self, b: Self) -> Self {
        a.min(b)
    }
    fn max(a: Self, b: Self) -> Self {
        a.max(b)
    }
    fn arrays_equal(a: &[Self], b: &[Self]) -> bool {
        a == b
    }
    fn to_value(self) -> RangeValue {
        RangeValue::Long(self)
    }
    fn from_value(v: &RangeValue) -> Self {
        match v {
            RangeValue::Long(v) => *v,
            _ => class_cast(),
        }
    }
    fn new_field(min: &[Self], max: &[Self]) -> Result<Box<dyn IndexableField>> {
        Ok(Box::new(LongRange::new(Self::FIELD_NAME, min, max)?))
    }
    fn new_query(relation: usize, min: &[Self], max: &[Self]) -> Result<Arc<dyn Query>> {
        range_queries!(LongRange, relation, min, max)
    }
}

/// `Math.min(float, float)` / `Math.min(double, double)` for the values the
/// bindings draw (never NaN): `-0.0` is the smaller zero.
macro_rules! java_min_max {
    ($t:ty) => {
        fn min(a: Self, b: Self) -> Self {
            if a == b && a == 0.0 {
                if a.is_sign_negative() {
                    a
                } else {
                    b
                }
            } else if a <= b {
                a
            } else {
                b
            }
        }
        fn max(a: Self, b: Self) -> Self {
            if a == b && a == 0.0 {
                if a.is_sign_negative() {
                    b
                } else {
                    a
                }
            } else if a >= b {
                a
            } else {
                b
            }
        }
        fn arrays_equal(a: &[Self], b: &[Self]) -> bool {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
        }
    };
}

impl RangeNumber for f32 {
    const FIELD_NAME: &'static str = "floatRangeField";

    fn next_internal() -> Self {
        let random = LuceneTestCase::random();
        match random.next_int_bound(5) {
            0 => f32::NEG_INFINITY,
            1 => f32::INFINITY,
            _ => {
                if random.next_boolean() {
                    random.next_float()
                } else {
                    (random.next_int_bound(15) - 7) as f32 / 3.0
                }
            }
        }
    }
    java_min_max!(f32);
    fn to_value(self) -> RangeValue {
        RangeValue::Float(self)
    }
    fn from_value(v: &RangeValue) -> Self {
        match v {
            RangeValue::Float(v) => *v,
            _ => class_cast(),
        }
    }
    fn new_field(min: &[Self], max: &[Self]) -> Result<Box<dyn IndexableField>> {
        Ok(Box::new(FloatRange::new(Self::FIELD_NAME, min, max)?))
    }
    fn new_query(relation: usize, min: &[Self], max: &[Self]) -> Result<Arc<dyn Query>> {
        range_queries!(FloatRange, relation, min, max)
    }
}

impl RangeNumber for f64 {
    const FIELD_NAME: &'static str = "doubleRangeField";

    fn next_internal() -> Self {
        let random = LuceneTestCase::random();
        match random.next_int_bound(5) {
            0 => f64::NEG_INFINITY,
            1 => f64::INFINITY,
            _ => {
                if random.next_boolean() {
                    random.next_double()
                } else {
                    f64::from(random.next_int_bound(15) - 7) / 3.0
                }
            }
        }
    }
    java_min_max!(f64);
    fn to_value(self) -> RangeValue {
        RangeValue::Double(self)
    }
    fn from_value(v: &RangeValue) -> Self {
        match v {
            RangeValue::Double(v) => *v,
            _ => class_cast(),
        }
    }
    fn new_field(min: &[Self], max: &[Self]) -> Result<Box<dyn IndexableField>> {
        Ok(Box::new(DoubleRange::new(Self::FIELD_NAME, min, max)?))
    }
    fn new_query(relation: usize, min: &[Self], max: &[Self]) -> Result<Arc<dyn Query>> {
        range_queries!(DoubleRange, relation, min, max)
    }
}

/// `IntTestRange`, `LongTestRange`, `FloatTestRange`, `DoubleTestRange`.
struct NumTestRange<T: RangeNumber> {
    min: Vec<T>,
    max: Vec<T>,
}

impl<T: RangeNumber> NumTestRange<T> {
    fn new(min: Vec<T>, max: Vec<T>) -> Self {
        randomizedtesting::jdk::exception::java_assert(!min.is_empty() && !max.is_empty(), || {
            "test box: min/max cannot be null or empty".to_string()
        });
        randomizedtesting::jdk::exception::java_assert(min.len() == max.len(), || {
            "test box: min/max length do not agree".to_string()
        });
        NumTestRange { min, max }
    }

    fn cast(other: &dyn Range) -> &Self {
        other
            .as_any()
            .downcast_ref::<Self>()
            .unwrap_or_else(|| class_cast())
    }
}

impl<T: RangeNumber> fmt::Display for NumTestRange<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Box({} TO {}", self.min[0], self.max[0])?;
        for d in 1..self.min.len() {
            write!(f, ", {} TO {}", self.min[d], self.max[d])?;
        }
        write!(f, ")")
    }
}

impl<T: RangeNumber> Range for NumTestRange<T> {
    fn num_dimensions(&self) -> i32 {
        self.min.len() as i32
    }
    fn get_min(&self, dim: i32) -> RangeValue {
        self.min[dim as usize].to_value()
    }
    fn set_min(&mut self, dim: i32, val: RangeValue) {
        let v = T::from_value(&val);
        let dim = dim as usize;
        if self.min[dim] < v {
            self.max[dim] = v;
        } else {
            self.min[dim] = v;
        }
    }
    fn get_max(&self, dim: i32) -> RangeValue {
        self.max[dim as usize].to_value()
    }
    fn set_max(&mut self, dim: i32, val: RangeValue) {
        let v = T::from_value(&val);
        let dim = dim as usize;
        if self.max[dim] > v {
            self.min[dim] = v;
        } else {
            self.max[dim] = v;
        }
    }
    fn is_equal(&self, other: &dyn Range) -> bool {
        let o = Self::cast(other);
        T::arrays_equal(&self.min, &o.min) && T::arrays_equal(&self.max, &o.max)
    }
    fn is_disjoint(&self, o: &dyn Range) -> bool {
        let other = Self::cast(o);
        (0..self.min.len()).any(|d| self.min[d] > other.max[d] || self.max[d] < other.min[d])
    }
    fn is_within(&self, o: &dyn Range) -> bool {
        let other = Self::cast(o);
        (0..self.min.len()).all(|d| self.min[d] >= other.min[d] && self.max[d] <= other.max[d])
    }
    fn contains(&self, o: &dyn Range) -> bool {
        let other = Self::cast(o);
        (0..self.min.len()).all(|d| self.min[d] <= other.min[d] && self.max[d] >= other.max[d])
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// `Test{Int,Long,Float,Double}RangeFieldQueries`.
struct NumericRangeBinding<T: RangeNumber>(std::marker::PhantomData<T>);

impl<T: RangeNumber> NumericRangeBinding<T> {
    fn new() -> Self {
        NumericRangeBinding(std::marker::PhantomData)
    }

    fn query(&self, relation: usize, r: &dyn Range) -> Result<Arc<dyn Query>> {
        let r = NumTestRange::<T>::cast(r);
        T::new_query(relation, &r.min, &r.max)
    }
}

impl<T: RangeNumber> BaseRangeFieldQueryTestCase for NumericRangeBinding<T> {
    fn next_range(&self, dimensions: i32) -> Result<Box<dyn Range>> {
        let mut min = Vec::with_capacity(dimensions as usize);
        let mut max = Vec::with_capacity(dimensions as usize);
        for _ in 0..dimensions {
            let min_v = T::next_internal();
            let max_v = T::next_internal();
            min.push(T::min(min_v, max_v));
            max.push(T::max(min_v, max_v));
        }
        Ok(Box::new(NumTestRange::new(min, max)))
    }

    fn new_range_field(&self, r: &dyn Range) -> Result<Box<dyn IndexableField>> {
        let r = NumTestRange::<T>::cast(r);
        T::new_field(&r.min, &r.max)
    }

    fn new_intersects_query(&self, r: &dyn Range) -> Result<Arc<dyn Query>> {
        self.query(0, r)
    }

    fn new_contains_query(&self, r: &dyn Range) -> Result<Arc<dyn Query>> {
        self.query(1, r)
    }

    fn new_within_query(&self, r: &dyn Range) -> Result<Arc<dyn Query>> {
        self.query(2, r)
    }

    fn new_crosses_query(&self, r: &dyn Range) -> Result<Arc<dyn Query>> {
        self.query(3, r)
    }
}

// -----------------------------------------------------------------------------
// TestInetAddressRangeQueries
// -----------------------------------------------------------------------------

const IP_FIELD_NAME: &str = "ipRangeField";

/// `InetAddress.getByAddress(b).getAddress()`: an IPv4-mapped IPv6 address
/// (`::ffff:a.b.c.d`) becomes the `Inet4Address` `a.b.c.d`, as the JDK
/// converts it.
fn by_address(b: Vec<u8>) -> Vec<u8> {
    if b.len() == 16 && b[..10].iter().all(|&x| x == 0) && b[10] == 0xff && b[11] == 0xff {
        b[12..].to_vec()
    } else {
        b
    }
}

/// `InetAddress.getHostAddress()`.
fn host_address(a: &[u8]) -> String {
    if a.len() == 4 {
        format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3])
    } else {
        let parts: Vec<String> = a
            .chunks(2)
            .map(|c| format!("{:x}", (u16::from(c[0]) << 8) | u16::from(c[1])))
            .collect();
        parts.join(":")
    }
}

/// `InetAddressPoint.encode(address)`.
fn encode(address: &[u8]) -> Vec<u8> {
    InetAddressPoint::encode(address)
        .unwrap_or_else(|e| rucene_test_framework::util::java_exceptions::rethrow_java(e))
}

/// `IpRange` (`private class`): encapsulated IpRange for test validation.
struct IpRange {
    min_address: Vec<u8>,
    max_address: Vec<u8>,
    min: Vec<u8>,
    max: Vec<u8>,
}

impl IpRange {
    fn new(min: Vec<u8>, max: Vec<u8>) -> Self {
        IpRange {
            min: encode(&min),
            max: encode(&max),
            min_address: min,
            max_address: max,
        }
    }

    fn cast(other: &dyn Range) -> &Self {
        other
            .as_any()
            .downcast_ref::<Self>()
            .unwrap_or_else(|| class_cast())
    }
}

impl fmt::Display for IpRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Box({} TO {})",
            host_address(&self.min_address),
            host_address(&self.max_address)
        )
    }
}

impl Range for IpRange {
    fn num_dimensions(&self) -> i32 {
        1
    }
    fn get_min(&self, _dim: i32) -> RangeValue {
        RangeValue::Inet(self.min_address.clone())
    }
    fn set_min(&mut self, _dim: i32, val: RangeValue) {
        let RangeValue::Inet(v) = val else {
            class_cast()
        };
        let e = encode(&v);
        if self.min[..e.len()] < e[..] {
            self.max = e;
            self.max_address = v;
        } else {
            self.min = e;
            self.min_address = v;
        }
    }
    fn get_max(&self, _dim: i32) -> RangeValue {
        RangeValue::Inet(self.max_address.clone())
    }
    fn set_max(&mut self, _dim: i32, val: RangeValue) {
        let RangeValue::Inet(v) = val else {
            class_cast()
        };
        let e = encode(&v);
        if self.max[..e.len()] > e[..] {
            self.min = e;
            self.min_address = v;
        } else {
            self.max = e;
            self.max_address = v;
        }
    }
    fn is_equal(&self, o: &dyn Range) -> bool {
        let other = Self::cast(o);
        self.min == other.min && self.max == other.max
    }
    fn is_disjoint(&self, o: &dyn Range) -> bool {
        let other = Self::cast(o);
        self.min[..] > other.max[..self.min.len()] || self.max[..] < other.min[..self.max.len()]
    }
    fn is_within(&self, o: &dyn Range) -> bool {
        let other = Self::cast(o);
        self.min[..] >= other.min[..self.min.len()] && self.max[..] <= other.max[..self.max.len()]
    }
    fn contains(&self, o: &dyn Range) -> bool {
        let other = Self::cast(o);
        self.min[..] <= other.min[..self.min.len()] && self.max[..] >= other.max[..self.max.len()]
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// `TestInetAddressRangeQueries`.
struct TestInetAddressRangeQueries;

impl TestInetAddressRangeQueries {
    /// `nextInetaddress()`: return random IPv4 or IPv6 address.
    fn next_inetaddress() -> Vec<u8> {
        let random = LuceneTestCase::random();
        let mut b = if random.next_boolean() {
            vec![0u8; 4]
        } else {
            vec![0u8; 16]
        };
        match random.next_int_bound(5) {
            0 => by_address(b),
            1 => {
                b.fill(0xff);
                by_address(b)
            }
            2 => {
                b.fill(42);
                by_address(b)
            }
            _ => {
                random.next_bytes(&mut b);
                by_address(b)
            }
        }
    }

    fn query(&self, relation: usize, r: &dyn Range) -> Result<Arc<dyn Query>> {
        let r = IpRange::cast(r);
        let (min, max) = (&r.min_address[..], &r.max_address[..]);
        let q: Arc<dyn Query> = match relation {
            0 => Arc::new(InetAddressRange::new_intersects_query(
                IP_FIELD_NAME,
                min,
                max,
            )?),
            1 => Arc::new(InetAddressRange::new_contains_query(
                IP_FIELD_NAME,
                min,
                max,
            )?),
            2 => Arc::new(InetAddressRange::new_within_query(IP_FIELD_NAME, min, max)?),
            _ => Arc::new(InetAddressRange::new_crosses_query(
                IP_FIELD_NAME,
                min,
                max,
            )?),
        };
        Ok(q)
    }
}

impl BaseRangeFieldQueryTestCase for TestInetAddressRangeQueries {
    fn next_range(&self, _dimensions: i32) -> Result<Box<dyn Range>> {
        let min = Self::next_inetaddress();
        let b_min = encode(&min);
        let max = Self::next_inetaddress();
        let b_max = encode(&max);
        if b_min[..] > b_max[..b_min.len()] {
            return Ok(Box::new(IpRange::new(max, min)));
        }
        Ok(Box::new(IpRange::new(min, max)))
    }

    /// `newRangeField(Range r)`: return random range.
    fn new_range_field(&self, r: &dyn Range) -> Result<Box<dyn IndexableField>> {
        let r = IpRange::cast(r);
        Ok(Box::new(InetAddressRange::new(
            IP_FIELD_NAME,
            &r.min_address,
            &r.max_address,
        )?))
    }

    fn new_intersects_query(&self, r: &dyn Range) -> Result<Arc<dyn Query>> {
        self.query(0, r)
    }

    fn new_contains_query(&self, r: &dyn Range) -> Result<Arc<dyn Query>> {
        self.query(1, r)
    }

    fn new_within_query(&self, r: &dyn Range) -> Result<Arc<dyn Query>> {
        self.query(2, r)
    }

    fn new_crosses_query(&self, r: &dyn Range) -> Result<Arc<dyn Query>> {
        self.query(3, r)
    }
}

// -----------------------------------------------------------------------------
// The classes
// -----------------------------------------------------------------------------

fn numeric_class<T: RangeNumber>(name: &str) -> Arc<Class> {
    Class::builder(name)
        .extends(&base_range_field_query_test_case::class())
        .constructor(Constructor::no_args(|| {
            RangeTestInstance::new(NumericRangeBinding::<T>::new())
        }))
        .build()
}

fn inet_address_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.search.TestInetAddressRangeQueries")
        .extends(&base_range_field_query_test_case::class())
        .constructor(Constructor::no_args(|| {
            RangeTestInstance::new(TestInetAddressRangeQueries)
        }))
        .method(range_test("testRandomTiny", |c| c.test_random_tiny()))
        // `testMultiValued()` runs `super.testRandomMedium()`, and the converse.
        .method(range_test("testMultiValued", |c| c.test_random_medium()))
        .method(range_test("testRandomMedium", |c| c.test_multi_valued()))
        .method(range_test("testRandomBig", |c| c.test_random_big()).annotate(Nightly))
        .build()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestRangeFieldSuiteDraws
// -----------------------------------------------------------------------------

static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The shared executor `searchOnTestThread` sets aside, restored by
/// `recordClassDraw`.
static SHARED_EXECUTOR: Mutex<Option<Arc<LuceneTestCaseExecutor>>> = Mutex::new(None);

/// `TestRangeFieldSuiteDraws` of `BaseSuitesDrawsOracle`: the cases on
/// `TestIntRangeFieldQueries`' ranges. As the Java class does, it runs with
/// `LuceneTestCase`'s shared executor set to `null` (its `@BeforeClass
/// searchOnTestThread`), so that every `MAYBE_CACHE_POLICY` draw of a
/// search happens on the test thread: with the executor, the thread that
/// runs each slice, and so the randomness drawn, is a race in Java.
fn draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.search.TestRangeFieldSuiteDraws")
        .extends(&base_range_field_query_test_case::class())
        .constructor(Constructor::no_args(|| {
            RangeTestInstance::new(NumericRangeBinding::<i32>::new())
        }))
        .method(
            Method::new("searchOnTestThread")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .annotate(BeforeClass)
                .static_body(|| {
                    *SHARED_EXECUTOR.lock().unwrap() = LuceneTestCase::replace_executor(None);
                }),
        )
        .method(draw_hooks::record_tear_down(
            &DRAWS,
            draw_hooks::lucene_tear_down::<RangeTestInstance>,
        ))
        .method(
            Method::new("recordClassDraw")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .annotate(AfterClass)
                .static_body(|| {
                    DRAWS.lock().unwrap().push(format!(
                        "#class\tnext\t{}",
                        LuceneTestCase::random().next_long()
                    ));
                    LuceneTestCase::replace_executor(SHARED_EXECUTOR.lock().unwrap().take());
                }),
        )
        .build()
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

#[test]
fn test_range_field_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match("base_range_field_draws.tsv", draws_class, &DRAWS);
}

#[test]
fn test_int_range_field_queries() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(|| numeric_class::<i32>("org.apache.lucene.search.TestIntRangeFieldQueries"));
}

#[test]
fn test_long_range_field_queries() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(|| numeric_class::<i64>("org.apache.lucene.search.TestLongRangeFieldQueries"));
}

#[test]
fn test_float_range_field_queries() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(|| numeric_class::<f32>("org.apache.lucene.search.TestFloatRangeFieldQueries"));
}

#[test]
fn test_double_range_field_queries() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(|| numeric_class::<f64>("org.apache.lucene.search.TestDoubleRangeFieldQueries"));
}

#[test]
fn test_inet_address_range_queries() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(inet_address_class);
}
