//! Port of `lucene/core/src/test/org/apache/lucene/util/TestWeakIdentityMap.java`.
//!
//! Java's keys are objects the garbage collector reclaims once unreachable;
//! Rucene's [`WeakIdentityMap`] keys are `Arc<K>` handles held weakly, so a
//! key is reclaimed when its last `Arc` drops. `key1 = key2 = key3 = null`
//! drops the test's handles; `System.runFinalization()` and `System.gc()`
//! have nothing left to do.
//!
//! A `null` key is `None`, as Rucene's map takes it.
//!
//! Rucene's map is not concurrent (`newConcurrentHashMap` builds the same
//! structure; its documentation says to wrap it in a lock), so
//! `testConcurrentHashMap` shares it behind a `Mutex`.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::WeakIdentityMap;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_null, assert_null, class, expect_throws_panic,
    random, test,
};

/// `TestWeakIdentityMap`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestWeakIdentityMap",
        vec![
            test("testSimpleHashMap", test_simple_hash_map),
            test("testConcurrentHashMap", test_concurrent_hash_map),
        ],
    )
}

/// `new String("foo")`: a distinct key object.
fn new_key() -> Arc<String> {
    Arc::new("foo".to_string())
}

/// `map.get(key)` as Java's `String` result (`null` is `None`).
fn get(map: &mut WeakIdentityMap<String, String>, key: Option<&Arc<String>>) -> Option<String> {
    map.get(key).cloned()
}

fn test_simple_hash_map() -> Result<()> {
    let mut map: WeakIdentityMap<String, String> =
        WeakIdentityMap::new_hash_map_with(random().next_boolean());
    // we keep strong references to the keys,
    // so WeakIdentityMap will not forget about them:
    let mut key1 = Some(new_key());
    let mut key2 = Some(new_key());
    let mut key3 = Some(new_key());
    {
        let (k1, k2, k3) = (
            key1.as_ref().unwrap(),
            key2.as_ref().unwrap(),
            key3.as_ref().unwrap(),
        );
        Assert::assert_false(Arc::ptr_eq(k1, k2)); // assertNotSame
        assert_equals(k1, k2);
        Assert::assert_false(Arc::ptr_eq(k1, k3));
        assert_equals(k1, k3);
        Assert::assert_false(Arc::ptr_eq(k2, k3));
        assert_equals(k2, k3);

        // try null key & check its iterator also return null:
        map.put(None, "null".to_string());
        {
            let mut it = map.key_iterator().peekable();
            Assert::assert_true(it.peek().is_some());
            assert_null(&it.next().expect("hasNext() was true"));
            Assert::assert_false(it.peek().is_some());
            Assert::assert_false(it.peek().is_some());
        }
        // 2 more keys:
        map.put(Some(k1), "bar1".to_string());
        map.put(Some(k2), "bar2".to_string());

        assert_equals(&3, &map.size());

        assert_equals(&Some("bar1".to_string()), &get(&mut map, Some(k1)));
        assert_equals(&Some("bar2".to_string()), &get(&mut map, Some(k2)));
        assert_equals(&None, &get(&mut map, Some(k3)));
        assert_equals(&Some("null".to_string()), &get(&mut map, None));

        Assert::assert_true(map.contains_key(Some(k1)));
        Assert::assert_true(map.contains_key(Some(k2)));
        Assert::assert_false(map.contains_key(Some(k3)));
        Assert::assert_true(map.contains_key(None));

        // repeat and check that we have no double entries
        map.put(Some(k1), "bar1".to_string());
        map.put(Some(k2), "bar2".to_string());
        map.put(None, "null".to_string());

        assert_equals(&3, &map.size());

        assert_equals(&Some("bar1".to_string()), &get(&mut map, Some(k1)));
        assert_equals(&Some("bar2".to_string()), &get(&mut map, Some(k2)));
        assert_equals(&None, &get(&mut map, Some(k3)));
        assert_equals(&Some("null".to_string()), &get(&mut map, None));

        Assert::assert_true(map.contains_key(Some(k1)));
        Assert::assert_true(map.contains_key(Some(k2)));
        Assert::assert_false(map.contains_key(Some(k3)));
        Assert::assert_true(map.contains_key(None));

        map.remove(None);
        assert_equals(&2, &map.size());
        map.remove(Some(k1));
        assert_equals(&1, &map.size());
        map.put(Some(k1), "bar1".to_string());
        map.put(Some(k2), "bar2".to_string());
        map.put(Some(k3), "bar3".to_string());
        assert_equals(&3, &map.size());

        let mut c = 0;
        let mut keys_assigned = 0;
        let mut it = map.key_iterator().peekable();
        while it.peek().is_some() {
            Assert::assert_true(it.peek().is_some()); // try again, should return same result!
            let k = it.next().expect("peeked key");
            let same = |key: &Arc<String>| k.as_ref().is_some_and(|k| Arc::ptr_eq(k, key));
            Assert::assert_true(same(k1) || same(k2) || same(k3));
            keys_assigned += if same(k1) {
                1
            } else if same(k2) {
                2
            } else {
                4
            };
            c += 1;
        }
        drop(it);
        assert_equals(&3, &c);
        assert_equals_msg(
            Some("all keys must have been seen"),
            &(1 + 2 + 4),
            &keys_assigned,
        );

        c = 0;
        for v in map.value_iterator() {
            Assert::assert_true(v.starts_with("bar"));
            c += 1;
        }
        assert_equals(&3, &c);
    }

    // clear strong refs
    key1 = None;
    key2 = None;
    key3 = None;
    let _ = (&key1, &key2, &key3);

    // check that GC does not cause problems in reap() method, wait 1 second and
    // let GC work:
    let mut size = map.size();
    let mut i = 0;
    while size > 0 && i < 10 {
        // runFinalization(); System.gc();
        let mut new_size = map.size();
        Assert::assert_true_msg(
            &format!("previousSize({size})>=newSize({new_size})"),
            size >= new_size,
        );
        size = new_size;
        if Thread::try_sleep(100).is_ok() {
            let mut c = 0;
            for k in map.key_iterator() {
                assert_not_null(&k);
                c += 1;
            }
            new_size = map.size();
            Assert::assert_true_msg(
                &format!("previousSize({size})>=iteratorSize({c})"),
                size >= c,
            );
            Assert::assert_true_msg(
                &format!("iteratorSize({c})>=newSize({new_size})"),
                c >= new_size,
            );
            size = new_size;
        }
        // catch (InterruptedException ie) {}
        i += 1;
    }

    map.clear();
    assert_equals(&0, &map.size());
    Assert::assert_true(map.is_empty());

    {
        let mut it = map.key_iterator().peekable();
        Assert::assert_false(it.peek().is_some());
        expect_throws_panic("java.util.NoSuchElementException", || {
            // Java's `Iterator.next()` past the end.
            if it.next().is_none() {
                Throwable::new("java.util.NoSuchElementException", "").throw()
            }
        });
    }

    let key1 = new_key();
    let key2 = new_key();
    map.put(Some(&key1), "bar1".to_string());
    map.put(Some(&key2), "bar2".to_string());
    assert_equals(&2, &map.size());

    map.clear();
    assert_equals(&0, &map.size());
    Assert::assert_true(map.is_empty());
    Ok(())
}

fn lock(
    m: &Mutex<WeakIdentityMap<u8, i32>>,
) -> std::sync::MutexGuard<'_, WeakIdentityMap<u8, i32>> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// `new Object()`: a key with its own identity.
type Object = Arc<u8>;

fn test_concurrent_hash_map() -> Result<()> {
    // don't make threadCount and keyCount random, otherwise easily OOMs or
    // fails otherwise:
    let thread_count = if *TEST_NIGHTLY { 8 } else { 2 };
    let key_count = 1024usize;
    let map: Arc<Mutex<WeakIdentityMap<u8, i32>>> = Arc::new(Mutex::new(
        WeakIdentityMap::new_concurrent_hash_map_with(random().next_boolean()),
    ));
    // we keep strong references to the keys,
    // so WeakIdentityMap will not forget about them:
    let keys: Arc<Vec<Mutex<Option<Object>>>> = Arc::new(
        (0..key_count)
            .map(|_| Mutex::new(Some(Arc::new(0u8))))
            .collect(),
    );
    let key_at = |keys: &Vec<Mutex<Option<Object>>>, j: usize| -> Object {
        keys[j]
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .expect("key set")
    };

    // `Executors.newFixedThreadPool(threadCount, new
    // NamedThreadFactory("testConcurrentHashMap"))`: one task per thread.
    let mut threads = Vec::with_capacity(thread_count);
    for t in 0..thread_count {
        let rnd = JdkRandom::new(random().next_long());
        let map = Arc::clone(&map);
        let keys = Arc::clone(&keys);
        threads.push(Thread::new(
            format!("testConcurrentHashMap-1-thread-{}", t + 1),
            move || {
                let count = LuceneTestCase::at_least_with(&rnd, 10000);
                for _ in 0..count {
                    let j = rnd.next_int_bound(key_count as i32) as usize;
                    match rnd.next_int_bound(5) {
                        0 => {
                            let k = key_at(&keys, j);
                            lock(&map).put(Some(&k), j as i32);
                        }
                        1 => {
                            let k = key_at(&keys, j);
                            let v = lock(&map).get(Some(&k)).copied();
                            if let Some(v) = v {
                                assert_equals(&(j as i32), &v);
                            }
                        }
                        2 => {
                            let k = key_at(&keys, j);
                            lock(&map).remove(Some(&k));
                        }
                        3 => {
                            *keys[j].lock().unwrap_or_else(|e| e.into_inner()) =
                                Some(Arc::new(0u8));
                        }
                        4 => {
                            let mut m = lock(&map);
                            for k in m.key_iterator() {
                                assert_not_null(&k);
                            }
                        }
                        _ => Assert::fail("Should not get here."),
                    }
                }
            },
        ));
    }
    for t in &threads {
        t.start();
    }
    // exec.shutdown(); while (!exec.awaitTermination(1000L, MILLISECONDS)) {}
    for t in &threads {
        let _ = t.join();
    }

    // clear strong refs
    for j in 0..key_count {
        *keys[j].lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    // check that GC does not cause problems in reap() method:
    let mut size = lock(&map).size();
    let mut i = 0;
    while size > 0 && i < 10 {
        // runFinalization(); System.gc();
        let mut new_size = lock(&map).size();
        Assert::assert_true_msg(
            &format!("previousSize({size})>=newSize({new_size})"),
            size >= new_size,
        );
        size = new_size;
        if Thread::try_sleep(100).is_ok() {
            let mut c = 0;
            {
                let mut m = lock(&map);
                for k in m.key_iterator() {
                    assert_not_null(&k);
                    c += 1;
                }
            }
            new_size = lock(&map).size();
            Assert::assert_true_msg(
                &format!("previousSize({size})>=iteratorSize({c})"),
                size >= c,
            );
            Assert::assert_true_msg(
                &format!("iteratorSize({c})>=newSize({new_size})"),
                c >= new_size,
            );
            size = new_size;
        }
        i += 1;
    }
    Ok(())
}
