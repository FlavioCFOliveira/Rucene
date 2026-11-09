//! Port of
//! `lucene/core/src/test/org/apache/lucene/util/TestRamUsageEstimator.java`.
//!
//! Methods not declared (each carries a `PENDING` note at its place below):
//! `testSanity`, `testBytesRefHash`, `testMap`, `testCollection`, `testQuery`,
//! `testReferenceSize`, `testHotspotBean` and `testPrintValues`. They need
//! Lucene members Rucene lacks: `RamUsageEstimator.shallowSizeOfInstance(Class)`
//! over Java classes (Rucene's measures a Rust type), `sizeOfObject` over
//! `Map`, `Collection` and `Query`, the constants `COMPRESSED_REFS_ENABLED`,
//! `JVM_IS_HOTSPOT_64BIT` and `LONG_SIZE`, and `RamUsageTester` models of
//! `Holder`, `BytesRefHash`, `HashMap`, `ArrayList` and the queries.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::util::RamUsageEstimator;
use rucene_test_framework::util::ram_usage_tester::{JavaObjectArray, RamObject, RamUsageTester};

use crate::core_support::{assert_equals, class, random, test};

/// `TestRamUsageEstimator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestRamUsageEstimator",
        vec![
            test("testStaticOverloads", test_static_overloads),
            test("testStrings", test_strings),
        ],
    )
}

/// `static final String[] strings`.
const STRINGS: [&str; 3] = ["test string", "hollow", "catchmaster"];

fn ram_used(o: &dyn RamObject) -> i64 {
    RamUsageTester::ram_used(o)
}

// PENDING(member): `testSanity` needs `shallowSizeOfInstance(Class)` of
// `String`, `Holder`, `HolderSubclass` and `HolderSubclass2` (Java classes,
// measured by reflection) and `RamUsageTester` models of `Holder`.

fn test_static_overloads() -> Result<()> {
    let rnd = random();
    {
        let array = vec![0u8; rnd.next_int_bound(1024) as usize];
        assert_equals(&RamUsageEstimator::size_of(&array), &ram_used(&array));
        // `shallowSizeOf(Object)` over a primitive array.
        assert_equals(
            &RamUsageEstimator::shallow_size_of_array::<u8>(array.len()),
            &ram_used(&array),
        );
    }

    {
        let array = vec![false; rnd.next_int_bound(1024) as usize];
        assert_equals(&ram_used(&array), &RamUsageEstimator::size_of_bool(&array));
        assert_equals(
            &ram_used(&array),
            &RamUsageEstimator::shallow_size_of_array::<bool>(array.len()),
        );
    }

    {
        let array = vec![0u16; rnd.next_int_bound(1024) as usize];
        assert_equals(&ram_used(&array), &RamUsageEstimator::size_of_char(&array));
        assert_equals(
            &ram_used(&array),
            &RamUsageEstimator::shallow_size_of_array::<u16>(array.len()),
        );
    }

    {
        let array = vec![0i16; rnd.next_int_bound(1024) as usize];
        assert_equals(&ram_used(&array), &RamUsageEstimator::size_of_short(&array));
        assert_equals(
            &ram_used(&array),
            &RamUsageEstimator::shallow_size_of_array::<i16>(array.len()),
        );
    }

    {
        let array = vec![0i32; rnd.next_int_bound(1024) as usize];
        assert_equals(&ram_used(&array), &RamUsageEstimator::size_of_int(&array));
        assert_equals(
            &ram_used(&array),
            &RamUsageEstimator::shallow_size_of_array::<i32>(array.len()),
        );
    }

    {
        let array = vec![0f32; rnd.next_int_bound(1024) as usize];
        assert_equals(&ram_used(&array), &RamUsageEstimator::size_of_float(&array));
        assert_equals(
            &ram_used(&array),
            &RamUsageEstimator::shallow_size_of_array::<f32>(array.len()),
        );
    }

    {
        let array = vec![0i64; rnd.next_int_bound(1024) as usize];
        assert_equals(&ram_used(&array), &RamUsageEstimator::size_of_long(&array));
        assert_equals(
            &ram_used(&array),
            &RamUsageEstimator::shallow_size_of_array::<i64>(array.len()),
        );
    }

    {
        let array = vec![0f64; rnd.next_int_bound(1024) as usize];
        assert_equals(
            &ram_used(&array),
            &RamUsageEstimator::size_of_double(&array),
        );
        assert_equals(
            &ram_used(&array),
            &RamUsageEstimator::shallow_size_of_array::<f64>(array.len()),
        );
    }
    Ok(())
}

fn test_strings() -> Result<()> {
    let owned: Vec<String> = STRINGS.iter().map(|s| s.to_string()).collect();
    let array = JavaObjectArray(owned.iter().map(|s| Some(s as &dyn RamObject)).collect());
    let actual = ram_used(&array);
    let estimated = RamUsageEstimator::size_of_string_array(
        &STRINGS.iter().map(|s| Some(*s)).collect::<Vec<_>>(),
    );
    assert_equals(&actual, &estimated);
    Ok(())
}

// PENDING(member): `testBytesRefHash` needs a `RamUsageTester` model of
// `BytesRefHash` (`RamUsageTester.ramUsed(bytes)`).

// PENDING(member): `testMap`, `testCollection` and `testQuery` need
// `RamUsageEstimator.sizeOfObject(Object)` over `Map`, `Collection` and
// `Query` (`RamUsageEstimator.java`, `sizeOfObject(Object, int, long)`),
// absent from Rucene's `SizedObject`, the constant
// `COMPRESSED_REFS_ENABLED`, and `RamUsageTester` models of the map, list,
// `Term` and query objects.

// PENDING(member): `testReferenceSize`, `testHotspotBean` and
// `testPrintValues` read `COMPRESSED_REFS_ENABLED`, `JVM_IS_HOTSPOT_64BIT`
// and `LONG_SIZE`, which Rucene's `RamUsageEstimator` does not define (its
// object layout is fixed).
