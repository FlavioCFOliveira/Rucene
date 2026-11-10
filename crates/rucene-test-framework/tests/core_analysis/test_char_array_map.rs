//! Port of `org.apache.lucene.analysis.TestCharArrayMap`.
//!
//! Java's `CharArrayMap<Integer>` is a `CharArrayMap<i32>` here, and its
//! `null` values are `None`. Java's `put(Object, V)`, `put(CharSequence, V)`
//! and `get(Object)` read a non-`char[]` key by its `toString()`; the port
//! passes that string.

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::CharArrayMap;
use rucene::error::Result;
use rucene::util::java_collections::JavaHashMap;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::LuceneTestCase;

use crate::lucene_core_shims::{lucene_class, t};

/// `UnsupportedOperationException.class`.
const UOE: &str = "java.lang.UnsupportedOperationException";

/// `assertEquals(Object expected, Object actual)` over two nullable
/// `Integer`s.
fn assert_equals_integer(expected: Option<i32>, actual: Option<i32>) {
    if expected != actual {
        let show = |v: Option<i32>| v.map_or_else(|| "null".to_string(), |v| v.to_string());
        Assert::fail(&format!(
            "expected:<{}> but was:<{}>",
            show(expected),
            show(actual)
        ))
    }
}

/// `assertNull(String message, Object object)` over a nullable `Integer`.
fn assert_null_msg(message: &str, actual: Option<&i32>) {
    if let Some(v) = actual {
        Assert::fail(&format!("{message} expected null, but was:<{v}>"))
    }
}

/// `Integer.intValue()`: unboxing `null` throws `NullPointerException`.
fn int_value(v: Option<&i32>) -> i32 {
    match v {
        Some(v) => *v,
        None => Throwable::new(
            "java.lang.NullPointerException",
            "Cannot invoke \"java.lang.Integer.intValue()\" because the value is null".to_string(),
        )
        .throw(),
    }
}

/// `doRandom(int iter, boolean ignoreCase)`.
fn do_random(iter: i32, ignore_case: bool) {
    let mut map: CharArrayMap<i32> = CharArrayMap::new(1, ignore_case);
    let mut hmap: JavaHashMap<i32> = JavaHashMap::new();

    let random = LuceneTestCase::random();
    for _ in 0..iter {
        let len = random.next_int_bound(5);
        let mut key = vec![0u16; len as usize];
        for c in key.iter_mut() {
            *c = random.next_int_bound(127) as u16;
        }
        let key_str: String = String::from_utf16_lossy(&key);
        let hmap_key = if ignore_case {
            key_str.to_lowercase()
        } else {
            key_str.clone()
        };

        let val = random.next_int();

        let key_len = key.len();
        let o1 = map.put_chars(&mut key, 0, key_len, val);
        let o2 = hmap.insert(hmap_key, val);
        assert_equals_integer(o1, o2);

        // add it again with the string method
        Assert::assert_equals(val, int_value(map.put(&key_str, val).as_ref()));

        Assert::assert_equals(val, int_value(map.get_chars(&key, 0, key.len())));
        // `map.get(key)`: `get(Object)` with a `char[]`.
        Assert::assert_equals(val, int_value(map.get_chars(&key, 0, key.len())));
        Assert::assert_equals(val, int_value(map.get(&key_str)));

        Assert::assert_equals(hmap.len(), map.size());
    }
}

fn test_char_array_map() -> Result<()> {
    let num = 5 * *RANDOM_MULTIPLIER;
    for _ in 0..num {
        // pump this up for more random testing
        do_random(1000, false);
        do_random(1000, true);
    }
    Ok(())
}

fn test_methods() -> Result<()> {
    let mut cm: CharArrayMap<i32> = CharArrayMap::new(2, false);
    let mut hm: JavaHashMap<i32> = JavaHashMap::new();
    hm.insert("foo".to_string(), 1);
    hm.insert("bar".to_string(), 2);
    cm.put_all(hm.iter().map(|(k, v)| (k.clone(), *v)));
    Assert::assert_equals(hm.len(), cm.size());
    hm.insert("baz".to_string(), 3);
    cm.put_all(hm.iter().map(|(k, v)| (k.clone(), *v)));
    Assert::assert_equals(hm.len(), cm.size());

    // `CharArraySet cs = cm.keySet();` Rust lets the live view borrow the
    // map only while it is used, so each use of `cs` below re-acquires it
    // with `cm.key_set()`. The keys are collected before the loop body reads
    // `cm`; nothing writes the map while Java iterates.
    let mut n = 0;
    let keys: Vec<Vec<u16>> = cm.key_set().iter().map(<[u16]>::to_vec).collect();
    for co in &keys {
        // `cm.containsKey(o)`: `containsKey(Object)` with a `char[]`.
        Assert::assert_true(cm.contains_key_chars(co, 0, co.len()));
        Assert::assert_true(cm.contains_key_chars(co, 0, co.len()));
        n += 1;
    }
    Assert::assert_equals(hm.len(), n);
    Assert::assert_equals(hm.len(), cm.key_set().size());
    Assert::assert_equals(cm.size(), cm.key_set().size());
    cm.key_set().clear();
    Assert::assert_equals(0usize, cm.key_set().size());
    Assert::assert_equals(0usize, cm.size());
    // keySet() should not allow adding new keys
    expect_throws(UOE, || cm.key_set().add("test"));

    cm.put_all(hm.iter().map(|(k, v)| (k.clone(), *v)));
    Assert::assert_equals(hm.len(), cm.key_set().size());
    Assert::assert_equals(cm.size(), cm.key_set().size());

    let mut iter1 = cm.entry_set().iterator();
    n = 0;
    while iter1.has_next(&cm) {
        let entry = iter1.next(&cm);
        let key = entry.get_key(&cm);
        let val = entry.get_value(&cm).copied();
        // `cm.get(key)`: `get(Object)` with a `char[]`.
        assert_equals_integer(cm.get_chars(&key, 0, key.len()).copied(), val);
        entry.set_value(&mut cm, int_value(val.as_ref()) * 100)?;
        Assert::assert_equals(
            int_value(val.as_ref()) * 100,
            int_value(cm.get_chars(&key, 0, key.len())),
        );
        n += 1;
    }
    Assert::assert_equals(hm.len(), n);
    cm.clear();
    cm.put_all(hm.iter().map(|(k, v)| (k.clone(), *v)));
    Assert::assert_equals(cm.size(), n);

    let mut iter2 = cm.entry_set().iterator();
    n = 0;
    while iter2.has_next(&cm) {
        let keyc = iter2.next_key(&cm).to_vec();
        let val = iter2.current_value(&cm).copied();
        let key_string: String = String::from_utf16_lossy(&keyc);
        assert_equals_integer(hm.get(&key_string).copied(), val);
        iter2.set_value(&mut cm, int_value(val.as_ref()) * 100)?;
        Assert::assert_equals(
            int_value(val.as_ref()) * 100,
            int_value(cm.get_chars(&keyc, 0, keyc.len())),
        );
        n += 1;
    }
    Assert::assert_equals(hm.len(), n);

    cm.entry_set().clear()?;
    Assert::assert_equals(0usize, cm.size());
    Assert::assert_equals(0usize, cm.entry_set().size());
    Assert::assert_true(cm.is_empty());
    Ok(())
}

// TODO: break this up into simpler test methods vs. "telling a story"
fn test_modify_on_unmodifiable() -> Result<()> {
    let mut map: CharArrayMap<i32> = CharArrayMap::new(2, false);
    map.put("foo", 1);
    map.put("bar", 2);
    let size = map.size();
    Assert::assert_equals(2usize, size);
    Assert::assert_true(map.contains_key("foo"));
    Assert::assert_equals(1, int_value(map.get("foo")));
    Assert::assert_true(map.contains_key("bar"));
    Assert::assert_equals(2, int_value(map.get("bar")));

    let mut unmodifiable_map = CharArrayMap::unmodifiable_map(map);
    Assert::assert_equals_msg(
        Some("Map size changed due to unmodifiableMap call"),
        size,
        unmodifiable_map.size(),
    );
    let not_in_map = "SirGallahad";
    Assert::assert_false_msg(
        "Test String already exists in map",
        unmodifiable_map.contains_key(not_in_map),
    );
    assert_null_msg(
        "Test String already exists in map",
        unmodifiable_map.get(not_in_map),
    );

    expect_throws(UOE, || {
        let mut chars: Vec<u16> = not_in_map.encode_utf16().collect();
        let len = chars.len();
        unmodifiable_map.put_chars(&mut chars, 0, len, 3);
        Ok(())
    });
    Assert::assert_false_msg(
        "Test String has been added to unmodifiable map",
        unmodifiable_map.contains_key(not_in_map),
    );
    assert_null_msg(
        "Test String has been added to unmodifiable map",
        unmodifiable_map.get(not_in_map),
    );
    Assert::assert_equals_msg(
        Some("Size of unmodifiable map has changed"),
        size,
        unmodifiable_map.size(),
    );

    expect_throws(UOE, || {
        unmodifiable_map.put(not_in_map, 3);
        Ok(())
    });
    Assert::assert_false_msg(
        "Test String has been added to unmodifiable map",
        unmodifiable_map.contains_key(not_in_map),
    );
    assert_null_msg(
        "Test String has been added to unmodifiable map",
        unmodifiable_map.get(not_in_map),
    );
    Assert::assert_equals_msg(
        Some("Size of unmodifiable map has changed"),
        size,
        unmodifiable_map.size(),
    );

    // `put(new StringBuilder(NOT_IN_MAP), 3)`: `put(CharSequence, V)`.
    expect_throws(UOE, || {
        unmodifiable_map.put(String::from(not_in_map), 3);
        Ok(())
    });
    Assert::assert_false_msg(
        "Test String has been added to unmodifiable map",
        unmodifiable_map.contains_key(not_in_map),
    );
    assert_null_msg(
        "Test String has been added to unmodifiable map",
        unmodifiable_map.get(not_in_map),
    );
    Assert::assert_equals_msg(
        Some("Size of unmodifiable map has changed"),
        size,
        unmodifiable_map.size(),
    );

    expect_throws(UOE, || {
        unmodifiable_map.clear();
        Ok(())
    });
    Assert::assert_equals_msg(
        Some("Size of unmodifiable map has changed"),
        size,
        unmodifiable_map.size(),
    );

    expect_throws(UOE, || unmodifiable_map.entry_set().clear());
    Assert::assert_equals_msg(
        Some("Size of unmodifiable map has changed"),
        size,
        unmodifiable_map.size(),
    );

    expect_throws(UOE, || {
        unmodifiable_map.key_set().clear();
        Ok(())
    });
    Assert::assert_equals_msg(
        Some("Size of unmodifiable map has changed"),
        size,
        unmodifiable_map.size(),
    );

    // `put((Object) NOT_IN_MAP, 3)`: `put(Object, V)` stores `toString()`.
    expect_throws(UOE, || {
        unmodifiable_map.put(not_in_map.to_string(), 3);
        Ok(())
    });
    Assert::assert_false_msg(
        "Test String has been added to unmodifiable map",
        unmodifiable_map.contains_key(not_in_map),
    );
    assert_null_msg(
        "Test String has been added to unmodifiable map",
        unmodifiable_map.get(not_in_map),
    );
    Assert::assert_equals_msg(
        Some("Size of unmodifiable map has changed"),
        size,
        unmodifiable_map.size(),
    );

    // `putAll(Collections.singletonMap(NOT_IN_MAP, 3))`.
    expect_throws(UOE, || {
        unmodifiable_map.put_all([(not_in_map, 3)]);
        Ok(())
    });
    Assert::assert_false_msg(
        "Test String has been added to unmodifiable map",
        unmodifiable_map.contains_key(not_in_map),
    );
    assert_null_msg(
        "Test String has been added to unmodifiable map",
        unmodifiable_map.get(not_in_map),
    );
    Assert::assert_equals_msg(
        Some("Size of unmodifiable map has changed"),
        size,
        unmodifiable_map.size(),
    );

    Assert::assert_true(unmodifiable_map.contains_key("foo"));
    Assert::assert_equals(1, int_value(unmodifiable_map.get("foo")));
    Assert::assert_true(unmodifiable_map.contains_key("bar"));
    Assert::assert_equals(2, int_value(unmodifiable_map.get("bar")));
    Ok(())
}

fn test_to_string() -> Result<()> {
    // `new CharArrayMap<>(Collections.singletonMap("test", 1), false)`.
    let mut cm: CharArrayMap<i32> = CharArrayMap::from_map([("test", 1)], false);
    Assert::assert_equals_str(None, Some("[test]"), Some(&cm.key_set().to_string()));
    Assert::assert_equals_str(None, Some("[1]"), Some(&cm.values().to_string()));
    Assert::assert_equals_str(None, Some("[test=1]"), Some(&cm.entry_set().to_string()));
    Assert::assert_equals_str(None, Some("{test=1}"), Some(&cm.to_string()));
    cm.put("test2", 2);
    Assert::assert_true(cm.key_set().to_string().contains(", "));
    Assert::assert_true(cm.values().to_string().contains(", "));
    Assert::assert_true(cm.entry_set().to_string().contains(", "));
    Assert::assert_true(cm.to_string().contains(", "));
    Ok(())
}

/// The class `org.apache.lucene.analysis.TestCharArrayMap`.
pub fn class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.TestCharArrayMap")
        .method(t("testCharArrayMap", test_char_array_map))
        .method(t("testMethods", test_methods))
        .method(t("testModifyOnUnmodifiable", test_modify_on_unmodifiable))
        .method(t("testToString", test_to_string))
        .build()
}
