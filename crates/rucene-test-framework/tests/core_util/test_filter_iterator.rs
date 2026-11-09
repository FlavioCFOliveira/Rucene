//! Port of `lucene/core/src/test/org/apache/lucene/util/TestFilterIterator.java`.
//!
//! Each anonymous `FilterIterator` subclass is a [`FilterIterator`] over the
//! closure of its `predicateFunction`. Rust's `Iterator::next` returns `None`
//! when exhausted where Java's throws `NoSuchElementException`;
//! [`it_next`] restores Java's contract.

use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::FilterIterator;
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_support::{assert_equals, class, expect_throws_panic, test};

/// `set`: `Collections.unmodifiableSet(new TreeSet<>(Arrays.asList("a", "b",
/// "c")))`.
static SET: LazyLock<BTreeSet<&'static str>> =
    LazyLock::new(|| ["a", "b", "c"].into_iter().collect());

type It =
    FilterIterator<std::collections::btree_set::Iter<'static, &'static str>, fn(&&&str) -> bool>;

/// `new FilterIterator<String, String>(set.iterator()) { predicateFunction }`.
fn filter(predicate: fn(&&&str) -> bool) -> It {
    FilterIterator::new(SET.iter(), predicate)
}

/// Java's `Iterator.next()`: the next element, or `NoSuchElementException`.
fn it_next(it: &mut It) -> &'static str {
    match it.next() {
        Some(s) => s,
        None => Throwable::new("java.util.NoSuchElementException", "").throw(),
    }
}

/// `TestFilterIterator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestFilterIterator",
        vec![
            test("testEmpty", test_empty),
            test("testA1", test_a1),
            test("testA2", test_a2),
            test("testB1", test_b1),
            test("testB2", test_b2),
            test("testAll1", test_all1),
            test("testAll2", test_all2),
            test("testUnmodifiable", test_unmodifiable),
        ],
    )
}

/// `assertNoMore(it)`.
fn assert_no_more(it: &mut It) {
    Assert::assert_false(it.has_next());
    expect_throws_panic("java.util.NoSuchElementException", || {
        it_next(it);
    });
    Assert::assert_false(it.has_next());
}

fn test_empty() -> Result<()> {
    let mut it = filter(|_s| false);
    assert_no_more(&mut it);
    Ok(())
}

fn test_a1() -> Result<()> {
    let mut it = filter(|s| "a" == **s);
    Assert::assert_true(it.has_next());
    assert_equals("a", it_next(&mut it));
    assert_no_more(&mut it);
    Ok(())
}

fn test_a2() -> Result<()> {
    let mut it = filter(|s| "a" == **s);
    // this time without check: assertTrue(it.hasNext());
    assert_equals("a", it_next(&mut it));
    assert_no_more(&mut it);
    Ok(())
}

fn test_b1() -> Result<()> {
    let mut it = filter(|s| "b" == **s);
    Assert::assert_true(it.has_next());
    assert_equals("b", it_next(&mut it));
    assert_no_more(&mut it);
    Ok(())
}

fn test_b2() -> Result<()> {
    let mut it = filter(|s| "b" == **s);
    // this time without check: assertTrue(it.hasNext());
    assert_equals("b", it_next(&mut it));
    assert_no_more(&mut it);
    Ok(())
}

fn test_all1() -> Result<()> {
    let mut it = filter(|_s| true);
    Assert::assert_true(it.has_next());
    assert_equals("a", it_next(&mut it));
    Assert::assert_true(it.has_next());
    assert_equals("b", it_next(&mut it));
    Assert::assert_true(it.has_next());
    assert_equals("c", it_next(&mut it));
    assert_no_more(&mut it);
    Ok(())
}

fn test_all2() -> Result<()> {
    let mut it = filter(|_s| true);
    assert_equals("a", it_next(&mut it));
    assert_equals("b", it_next(&mut it));
    assert_equals("c", it_next(&mut it));
    assert_no_more(&mut it);
    Ok(())
}

fn test_unmodifiable() -> Result<()> {
    let mut it = filter(|_s| true);
    assert_equals("a", it_next(&mut it));
    expect_throws("java.lang.UnsupportedOperationException", || it.remove());
    Ok(())
}
