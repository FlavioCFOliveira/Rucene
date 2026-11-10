//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDocValuesFieldUpdates.java`.
//!
//! Rucene's `DocValuesFieldUpdates` is one type for Java's
//! `NumericDocValuesFieldUpdates` (built with `DocValuesType::NUMERIC`) and
//! its `SingleValueNumericDocValuesFieldUpdates` subclass
//! (`new_single_value_numeric`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::doc_values_field_updates::{merged_iterator, DocValuesFieldUpdates};
use rucene::index::DocValuesType;
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestDocValuesFieldUpdates`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestDocValuesFieldUpdates",
        vec![
            test("testMergeIterator", test_merge_iterator),
            test("testUpdateAndResetSameDoc", test_update_and_reset_same_doc),
            test(
                "testUpdateAndResetUpdateSameDoc",
                test_update_and_reset_update_same_doc,
            ),
            test("testUpdatesAndResetRandom", test_updates_and_reset_random),
            test("testSharedValueUpdates", test_shared_value_updates),
        ],
    )
}

/// `new NumericDocValuesFieldUpdates(delGen, field, maxDoc)`.
fn numeric(del_gen: i64, field: &str, max_doc: i32) -> DocValuesFieldUpdates {
    DocValuesFieldUpdates::new(max_doc, del_gen, field, DocValuesType::NUMERIC)
}

/// Port of `TestDocValuesFieldUpdates.testMergeIterator`.
fn test_merge_iterator() -> Result<()> {
    let mut updates1 = numeric(0, "test", 6);
    let mut updates2 = numeric(1, "test", 6);
    let mut updates3 = numeric(2, "test", 6);
    let mut updates4 = numeric(2, "test", 6);

    updates1.add_long(0, 1);
    updates1.add_long(4, 0);
    updates1.add_long(1, 4);
    updates1.add_long(2, 5);
    updates1.add_long(4, 9);
    Assert::assert_true(updates1.any());

    updates2.add_long(0, 18);
    updates2.add_long(1, 7);
    updates2.add_long(2, 19);
    updates2.add_long(5, 24);
    Assert::assert_true(updates2.any());

    updates3.add_long(2, 42);
    Assert::assert_true(updates3.any());
    Assert::assert_false(updates4.any());
    updates1.finish()?;
    updates2.finish()?;
    updates3.finish()?;
    updates4.finish()?;
    let mut iterators = vec![
        updates1.iterator(),
        updates2.iterator(),
        updates3.iterator(),
        updates4.iterator(),
    ];
    shuffle(&mut iterators, &*random());
    let mut iterator = merged_iterator(iterators);
    for (doc, value) in [(0, 18), (1, 7), (2, 42), (4, 9), (5, 24)] {
        assert_equals(&doc, &iterator.next_doc()?);
        assert_equals(&(value as i64), &iterator.long_value());
    }
    assert_equals(&NO_MORE_DOCS, &iterator.next_doc()?);
    Ok(())
}

/// Port of `TestDocValuesFieldUpdates.testUpdateAndResetSameDoc`.
fn test_update_and_reset_same_doc() -> Result<()> {
    let mut updates = numeric(0, "test", 2);
    updates.add_long(0, 1);
    updates.reset(0);
    updates.finish()?;
    let mut iterator = updates.iterator();
    assert_equals(&0, &iterator.next_doc()?);
    Assert::assert_false(iterator.has_value());
    assert_equals(&NO_MORE_DOCS, &iterator.next_doc()?);
    Ok(())
}

/// Port of `TestDocValuesFieldUpdates.testUpdateAndResetUpdateSameDoc`.
fn test_update_and_reset_update_same_doc() -> Result<()> {
    let mut updates = numeric(0, "test", 3);
    updates.add_long(0, 1);
    updates.add_doc(0);
    updates.add_long(0, 2);
    updates.finish()?;
    let mut iterator = updates.iterator();
    assert_equals(&0, &iterator.next_doc()?);
    Assert::assert_true(iterator.has_value());
    assert_equals(&2, &iterator.long_value());
    assert_equals(&NO_MORE_DOCS, &iterator.next_doc()?);
    Ok(())
}

/// Port of `TestDocValuesFieldUpdates.testUpdatesAndResetRandom`.
fn test_updates_and_reset_random() -> Result<()> {
    let mut updates = numeric(0, "test", 10);
    let num_updates = 10 + random().next_int_bound(100);
    let mut values: [Option<i32>; 5] = [None; 5];
    for i in 0..5 {
        values[i] = if random().next_boolean() {
            None
        } else {
            Some(random().next_int_bound(100))
        };
        match values[i] {
            None => updates.reset(i as i32),
            Some(v) => updates.add_long(i as i32, i64::from(v)),
        }
    }
    for _ in 0..num_updates {
        let doc_id = random().next_int_bound(5) as usize;
        values[doc_id] = if random().next_boolean() {
            None
        } else {
            Some(random().next_int_bound(100))
        };
        match values[doc_id] {
            None => updates.reset(doc_id as i32),
            Some(v) => updates.add_long(doc_id as i32, i64::from(v)),
        }
    }

    updates.finish()?;
    let mut iterator = updates.iterator();
    let mut idx = 0usize;
    while iterator.next_doc()? != NO_MORE_DOCS {
        assert_equals(&(idx as i32), &iterator.doc_id());
        match values[idx] {
            None => Assert::assert_false(iterator.has_value()),
            Some(v) => {
                Assert::assert_true(iterator.has_value());
                assert_equals(&i64::from(v), &iterator.long_value());
            }
        }
        idx += 1;
    }
    Ok(())
}

/// Port of `TestDocValuesFieldUpdates.testSharedValueUpdates`.
fn test_shared_value_updates() -> Result<()> {
    let del_gen = random().next_int();
    let max_doc = 1 + random().next_int_bound(1000);
    let value = random().next_long();
    let mut update =
        DocValuesFieldUpdates::new_single_value_numeric(i64::from(del_gen), "foo", max_doc, value);
    assert_equals(
        &value,
        &update
            .as_single_value_numeric()
            .unwrap_or_else(|| crate::support::npe("SingleValueNumericDocValuesFieldUpdates"))
            .long_value(),
    );
    let mut values: Vec<Option<bool>> = vec![None; max_doc as usize];
    let mut any = false;
    let no_reset = random().next_boolean(); // sometimes don't reset
    for i in 0..max_doc as usize {
        if random().next_boolean() {
            values[i] = Some(true);
            any = true;
            update.add_long(i as i32, value);
        } else if random().next_boolean() && !no_reset {
            values[i] = None;
            any = true;
            update.reset(i as i32);
        } else {
            values[i] = Some(false);
        }
    }
    if !no_reset {
        for i in 0..values.len() {
            if LuceneTestCase::rarely() {
                match values[i] {
                    None => {
                        values[i] = Some(true);
                        update.add_long(i as i32, value);
                    }
                    Some(true) => {
                        values[i] = None;
                        update.reset(i as i32);
                    }
                    Some(false) => {}
                }
            }
        }
    }
    update.finish()?;
    let mut iterator = update.iterator();
    assert_equals(&any, &update.any());
    assert_equals(&i64::from(del_gen), &iterator.del_gen());
    let mut index = 0usize;
    while iterator.next_doc()? != NO_MORE_DOCS {
        let doc = iterator.doc_id() as usize;
        if index < doc {
            while index < doc {
                // `assertFalse(values[index])`: unboxing a `null` throws.
                Assert::assert_false(
                    values[index].unwrap_or_else(|| crate::support::npe("values[index]")),
                );
                index += 1;
            }
        }
        if index == doc {
            let v = values[index];
            index += 1;
            if v.is_none() {
                Assert::assert_false(iterator.has_value());
            } else {
                Assert::assert_true(iterator.has_value());
                assert_equals(&value, &iterator.long_value());
            }
        }
    }
    Ok(())
}
