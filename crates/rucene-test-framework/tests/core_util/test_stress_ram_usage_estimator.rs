//! Port of
//! `lucene/core/src/test/org/apache/lucene/util/TestStressRamUsageEstimator.java`
//! (both methods `@Nightly`).
//!
//! Not expressible in Rust (reported): `System.gc()` and
//! `Runtime.getRuntime().totalMemory()` (no managed heap; the `mx:` line
//! prints the JVM heap growth), and `catch (OutOfMemoryError e)` (a Rust
//! allocation failure aborts). The `Object[]` elements are `byte[]`s, so their
//! `shallowSizeOf(Object)` is `shallowSizeOfArray` over bytes.
//! `static class Entry` is unused by the test and is not translated.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::util::RamUsageEstimator;
use rucene_test_framework::util::lucene_test_case::Nightly;

use crate::core_support::{class, random, test};

/// `TestStressRamUsageEstimator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestStressRamUsageEstimator",
        vec![
            test("testLargeSetOfByteArrays", test_large_set_of_byte_arrays).annotate(Nightly),
            test("testSimpleByteArrays", test_simple_byte_arrays).annotate(Nightly),
        ],
    )
}

/// `volatile Object guard`.
static GUARD: Mutex<Option<Vec<Vec<u8>>>> = Mutex::new(None);

fn test_large_set_of_byte_arrays() -> Result<()> {
    // System.gc(); long before = Runtime.getRuntime().totalMemory();  (not expressible)
    let mut all: Vec<Vec<u8>> = Vec::with_capacity(1000000);
    for _ in 0..1000000 {
        all.push(vec![0u8; random().next_int_bound(3) as usize]);
    }
    // System.gc(); long after = Runtime.getRuntime().totalMemory();  (not expressible)
    // System.out.println("mx:  " + humanReadableUnits(after - before));  (not expressible)
    println!(
        "rue: {}",
        RamUsageEstimator::human_readable_units(shallow_size_of(&all))
    );

    *GUARD.lock().unwrap_or_else(|e| e.into_inner()) = Some(all);
    Ok(())
}

/// `shallowSizeOf(Object[] all)`.
fn shallow_size_of(all: &[Vec<u8>]) -> i64 {
    let mut s = RamUsageEstimator::shallow_size_of(all);
    for o in all {
        s += RamUsageEstimator::shallow_size_of_array::<u8>(o.len());
    }
    s
}

/// `shallowSizeOf(Object[][] all)`.
fn shallow_size_of_2d(all: &[Vec<Vec<u8>>]) -> i64 {
    let mut s = RamUsageEstimator::shallow_size_of(all);
    for o in all {
        s += RamUsageEstimator::shallow_size_of(o);
        for o2 in o {
            s += RamUsageEstimator::shallow_size_of_array::<u8>(o2.len());
        }
    }
    s
}

fn test_simple_byte_arrays() -> Result<()> {
    let mut all: Vec<Vec<Vec<u8>>> = Vec::new();
    // try { ... } catch (OutOfMemoryError e) { /* Release and quit. */ }
    // (not expressible: a Rust allocation failure aborts)
    loop {
        // System.gc();  (not expressible)
        let estimated = shallow_size_of_2d(&all);
        if estimated > 50 * RamUsageEstimator::ONE_MB {
            break;
        }

        let mut seg: Vec<Vec<u8>> = Vec::with_capacity(10000);
        for _ in 0..10000 {
            seg.push(vec![0u8; random().next_int_bound(7) as usize]);
        }
        // all = ArrayUtil.growExact(all, all.length + 1); all[all.length - 1] = seg;
        all.push(seg);
    }
    Ok(())
}
