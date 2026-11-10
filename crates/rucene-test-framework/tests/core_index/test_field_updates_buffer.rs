//! Port of `lucene/core/src/test/org/apache/lucene/index/TestFieldUpdatesBuffer.java`.
//!
//! * `new FieldUpdatesBuffer(counter, update, docUpTo)` is
//!   `FieldUpdatesBuffer::new(update, docUpTo)`: Rucene's buffer owns its
//!   `Counter` (a pre-existing divergence of `src/index/field_updates_buffer.rs`);
//!   the tests only create the counter.
//! * `getMaxNumeric()`/`getMinNumeric()` are
//!   `max_numeric_value()`/`min_numeric_value()` with Java's `0` for `None`.
//! * `NumericDocValuesUpdate`/`BinaryDocValuesUpdate` are `DocValuesUpdate`
//!   values; `prepareForApply(docIdUpTo)` is [`prepare_for_apply`].

use std::collections::BTreeMap;
use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::doc_values_update::{DocValuesUpdate, DocValuesUpdateValue, MAX_DOC_ID_UP_TO};
use rucene::index::field_updates_buffer::FieldUpdatesBuffer;
use rucene::index::terms::Term;
use rucene::index::DocValuesType;
use rucene::util::concurrent::Counter;
use rucene::util::BytesRef;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_null, assert_null, class, random, test,
};

/// `TestFieldUpdatesBuffer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestFieldUpdatesBuffer",
        vec![
            test("testBasics", test_basics),
            test("testUpdateShareValues", test_update_share_values),
            test(
                "testUpdateShareValuesBinary",
                test_update_share_values_binary,
            ),
            test("testBinaryRandom", test_binary_random),
            test("testNumericRandom", test_numeric_random),
            test("testNoNumericValue", test_no_numeric_value),
            test(
                "testSortAndDedupNumericUpdatesByTerms",
                test_sort_and_dedup_numeric_updates_by_terms,
            ),
        ],
    )
}

/// `new DocValuesUpdate.NumericDocValuesUpdate(term, field, value)`.
fn numeric_update(term: Term, field: &str, value: Option<i64>) -> DocValuesUpdate {
    DocValuesUpdate::new(
        DocValuesType::NUMERIC,
        term,
        field,
        MAX_DOC_ID_UP_TO,
        value.map_or(DocValuesUpdateValue::None, DocValuesUpdateValue::Numeric),
    )
}

/// `new DocValuesUpdate.BinaryDocValuesUpdate(term, field, value)`.
fn binary_update(term: Term, field: &str, value: Option<BytesRef>) -> DocValuesUpdate {
    DocValuesUpdate::new(
        DocValuesType::BINARY,
        term,
        field,
        MAX_DOC_ID_UP_TO,
        value.map_or(DocValuesUpdateValue::None, DocValuesUpdateValue::Binary),
    )
}

/// `update.prepareForApply(docIdUpTo)`: the same update, applying up to
/// `doc_id_up_to` (`DocValuesUpdate.java`, `prepareForApply`).
fn prepare_for_apply(update: DocValuesUpdate, doc_id_up_to: i32) -> DocValuesUpdate {
    if update.doc_id_up_to == doc_id_up_to {
        return update;
    }
    DocValuesUpdate::new(
        update.doc_values_type,
        update.term,
        update.field,
        doc_id_up_to,
        update.value,
    )
}

/// `NumericDocValuesUpdate.getValue()`.
fn numeric_value(update: &DocValuesUpdate) -> i64 {
    match update.value {
        DocValuesUpdateValue::Numeric(v) => v,
        _ => 0,
    }
}

/// `BinaryDocValuesUpdate.getValue()`.
fn binary_value(update: &DocValuesUpdate) -> Option<BytesRef> {
    match &update.value {
        DocValuesUpdateValue::Binary(v) => Some(v.clone()),
        _ => None,
    }
}

/// `buffer.getMaxNumeric()`.
fn max_numeric(buffer: &FieldUpdatesBuffer) -> i64 {
    buffer.max_numeric_value().unwrap_or(0)
}

/// `buffer.getMinNumeric()`.
fn min_numeric(buffer: &FieldUpdatesBuffer) -> i64 {
    buffer.min_numeric_value().unwrap_or(0)
}

/// `new BytesRef("")`.
fn empty() -> BytesRef {
    BytesRef::from_char_sequence("")
}

/// Port of `TestFieldUpdatesBuffer.testBasics`.
fn test_basics() -> Result<()> {
    let _counter = Counter::new_counter();
    let update = numeric_update(Term::from_text("id", "1"), "age", Some(6));
    let mut buffer = FieldUpdatesBuffer::new(update, 15)?;
    buffer.add_numeric_update(&Term::from_text("id", "10"), 6, 15)?;
    Assert::assert_true(buffer.has_single_value());
    buffer.add_numeric_update(&Term::from_text("id", "8"), 12, 15)?;
    Assert::assert_false(buffer.has_single_value());
    buffer.add_numeric_update(&Term::from_text("some_other_field", "8"), 13, 17)?;
    Assert::assert_false(buffer.has_single_value());
    buffer.add_numeric_update(&Term::from_text("id", "8"), 12, 16)?;
    Assert::assert_false(buffer.has_single_value());
    Assert::assert_true(buffer.is_numeric());
    assert_equals(&13, &max_numeric(&buffer));
    assert_equals(&6, &min_numeric(&buffer));
    buffer.finish()?;
    let mut iterator = buffer.iterator()?;
    let expected: [(&str, &str, i64, i32); 5] = [
        ("id", "1", 6, 15),
        ("id", "10", 6, 15),
        ("id", "8", 12, 15),
        ("some_other_field", "8", 13, 17),
        ("id", "8", 12, 16),
    ];
    for (field, term, numeric, doc_up_to) in expected {
        let value = iterator.next()?;
        assert_not_null(&value);
        let value = value.unwrap_or_else(|| crate::support::npe("value"));
        assert_equals(field, value.term_field.as_str());
        assert_equals(term, value.term_value.utf8_to_string()?.as_str());
        assert_equals(&numeric, &value.numeric_value);
        assert_equals(&doc_up_to, &value.doc_up_to);
    }
    assert_null(&iterator.next()?.map(|_| ()));
    Ok(())
}

/// Port of `TestFieldUpdatesBuffer.testUpdateShareValues`.
fn test_update_share_values() -> Result<()> {
    let _counter = Counter::new_counter();
    let int_value = random().next_int();
    let value_for_three = random().next_boolean();
    let update = numeric_update(
        Term::from_text("id", "0"),
        "enabled",
        Some(int_value as i64),
    );
    let mut buffer = FieldUpdatesBuffer::new(update, i32::MAX)?;
    buffer.add_numeric_update(&Term::from_text("id", "1"), int_value as i64, i32::MAX)?;
    buffer.add_numeric_update(&Term::from_text("id", "2"), int_value as i64, i32::MAX)?;
    if value_for_three {
        buffer.add_numeric_update(&Term::from_text("id", "3"), int_value as i64, i32::MAX)?;
    } else {
        buffer.add_no_value(&Term::from_text("id", "3"), i32::MAX)?;
    }
    buffer.add_numeric_update(&Term::from_text("id", "4"), int_value as i64, i32::MAX)?;
    buffer.finish()?;
    let mut iterator = buffer.iterator()?;
    let mut count = 0;
    while let Some(value) = iterator.next()? {
        let has_value = count != 3 || value_for_three;
        assert_equals(
            count.to_string().as_str(),
            value.term_value.utf8_to_string()?.as_str(),
        );
        count += 1;
        assert_equals("id", value.term_field.as_str());
        assert_equals(&has_value, &value.has_value);
        if has_value {
            assert_equals(&(int_value as i64), &value.numeric_value);
        } else {
            assert_equals(&0, &value.numeric_value);
        }
        assert_equals(&i32::MAX, &value.doc_up_to);
    }
    Assert::assert_true(buffer.is_numeric());
    Ok(())
}

/// Port of `TestFieldUpdatesBuffer.testUpdateShareValuesBinary`.
fn test_update_share_values_binary() -> Result<()> {
    let _counter = Counter::new_counter();
    let value_for_three = random().next_boolean();
    let update = binary_update(Term::from_text("id", "0"), "enabled", Some(empty()));
    let mut buffer = FieldUpdatesBuffer::new(update, i32::MAX)?;
    buffer.add_binary_update(&Term::from_text("id", "1"), empty(), i32::MAX)?;
    buffer.add_binary_update(&Term::from_text("id", "2"), empty(), i32::MAX)?;
    if value_for_three {
        buffer.add_binary_update(&Term::from_text("id", "3"), empty(), i32::MAX)?;
    } else {
        buffer.add_no_value(&Term::from_text("id", "3"), i32::MAX)?;
    }
    buffer.add_binary_update(&Term::from_text("id", "4"), empty(), i32::MAX)?;
    buffer.finish()?;
    let mut iterator = buffer.iterator()?;
    let mut count = 0;
    while let Some(value) = iterator.next()? {
        let has_value = count != 3 || value_for_three;
        assert_equals(
            count.to_string().as_str(),
            value.term_value.utf8_to_string()?.as_str(),
        );
        count += 1;
        assert_equals("id", value.term_field.as_str());
        assert_equals(&has_value, &value.has_value);
        if has_value {
            assert_equals(&Some(empty()), &value.binary_value);
        } else {
            assert_null(&value.binary_value);
        }
        assert_equals(&i32::MAX, &value.doc_up_to);
    }
    Assert::assert_false(buffer.is_numeric());
    Ok(())
}

const TERM_FIELDS: [&str; 3] = ["id", "_id", "some_other_field"];

/// `getRandomBinaryUpdate(docIdUpTo)`.
fn get_random_binary_update(doc_id_up_to: i32) -> DocValuesUpdate {
    let term_field = *RandomPicks::random_from(&*random(), &TERM_FIELDS);
    let doc_id = random().next_int_bound(10).to_string();
    let value = binary_update(
        Term::from_text(term_field, &doc_id),
        "binary",
        if LuceneTestCase::rarely() {
            None
        } else {
            Some(BytesRef::from_char_sequence(
                &TestUtil::random_realistic_unicode_string(&*random()),
            ))
        },
    );
    if LuceneTestCase::rarely() {
        prepare_for_apply(value, doc_id_up_to)
    } else {
        value
    }
}

/// `getRandomNumericUpdate(docIdUpTo)`.
fn get_random_numeric_update(doc_id_up_to: i32) -> DocValuesUpdate {
    let term_field = *RandomPicks::random_from(&*random(), &TERM_FIELDS);
    let doc_id = random().next_int_bound(10).to_string();
    let value = numeric_update(
        Term::from_text(term_field, &doc_id),
        "numeric",
        if LuceneTestCase::rarely() {
            None
        } else {
            Some(random().next_int_bound(100) as i64)
        },
    );
    if LuceneTestCase::rarely() {
        prepare_for_apply(value, doc_id_up_to)
    } else {
        value
    }
}

/// Port of `TestFieldUpdatesBuffer.testBinaryRandom`.
fn test_binary_random() -> Result<()> {
    let mut updates: Vec<DocValuesUpdate> = Vec::new();
    let num_updates = 1 + random().next_int_bound(1000);
    let _counter = Counter::new_counter();
    let random_update = get_random_binary_update(0);
    let doc_id_up_to = random_update.doc_id_up_to;
    updates.push(random_update.clone());
    let mut buffer = FieldUpdatesBuffer::new(random_update, doc_id_up_to)?;
    for i in 0..num_updates {
        let random_update = get_random_binary_update(i + 1);
        updates.push(random_update.clone());
        if random_update.has_value() {
            buffer.add_binary_update(
                &random_update.term,
                binary_value(&random_update).unwrap_or_else(|| crate::support::npe("value")),
                random_update.doc_id_up_to,
            )?;
        } else {
            buffer.add_no_value(&random_update.term, random_update.doc_id_up_to)?;
        }
    }
    buffer.finish()?;
    let mut iterator = buffer.iterator()?;

    let mut count = 0;
    while let Some(value) = iterator.next()? {
        let random_update = &updates[count];
        count += 1;
        assert_equals(
            random_update.term.bytes().utf8_to_string()?.as_str(),
            value.term_value.utf8_to_string()?.as_str(),
        );
        assert_equals(random_update.term.field(), value.term_field.as_str());
        assert_equals_msg(
            Some(&format!("count: {count}")),
            &random_update.has_value(),
            &value.has_value,
        );
        if random_update.has_value() {
            assert_equals(&binary_value(random_update), &value.binary_value);
        } else {
            assert_null(&value.binary_value);
        }
        assert_equals(&random_update.doc_id_up_to, &value.doc_up_to);
    }
    assert_equals(&count, &updates.len());
    Ok(())
}

/// Port of `TestFieldUpdatesBuffer.testNumericRandom`.
fn test_numeric_random() -> Result<()> {
    let mut updates: Vec<DocValuesUpdate> = Vec::new();
    let num_updates = 1 + random().next_int_bound(1000);
    let _counter = Counter::new_counter();
    let mut random_update = get_random_numeric_update(0);
    let doc_id_up_to = random_update.doc_id_up_to;
    updates.push(random_update.clone());
    let mut buffer = FieldUpdatesBuffer::new(random_update.clone(), doc_id_up_to)?;
    for i in 0..num_updates {
        random_update = get_random_numeric_update(i + 1);
        updates.push(random_update.clone());
        if random_update.has_value() {
            buffer.add_numeric_update(
                &random_update.term,
                numeric_value(&random_update),
                random_update.doc_id_up_to,
            )?;
        } else {
            buffer.add_no_value(&random_update.term, random_update.doc_id_up_to)?;
        }
    }
    buffer.finish()?;
    let last_update = &random_update;
    let terms_sorted = last_update.has_value()
        && updates.iter().all(|update| {
            update.field == last_update.field
                && update.has_value()
                && numeric_value(update) == numeric_value(last_update)
        });
    assert_buffer_updates(&buffer, updates, terms_sorted)
}

/// Port of `TestFieldUpdatesBuffer.testNoNumericValue`.
fn test_no_numeric_value() -> Result<()> {
    let update = numeric_update(Term::from_text("id", "1"), "age", None);
    let doc_id_up_to = update.doc_id_up_to;
    let buffer = FieldUpdatesBuffer::new(update, doc_id_up_to)?;
    assert_equals(&0, &min_numeric(&buffer));
    assert_equals(&0, &max_numeric(&buffer));
    Ok(())
}

/// Port of `TestFieldUpdatesBuffer.testSortAndDedupNumericUpdatesByTerms`.
fn test_sort_and_dedup_numeric_updates_by_terms() -> Result<()> {
    let mut updates: Vec<DocValuesUpdate> = Vec::new();
    let num_updates = 1 + random().next_int_bound(1000);
    let _counter = Counter::new_counter();
    let term_field = *RandomPicks::random_from(&*random(), &TERM_FIELDS);
    let doc_value = 1 + random().next_int_bound(1000) as i64;
    let random_update = numeric_update(
        Term::from_text(term_field, &random().next_int_bound(1000).to_string()),
        "numeric",
        Some(doc_value),
    );
    let random_update = prepare_for_apply(random_update, 0);
    updates.push(random_update.clone());
    let doc_id_up_to = random_update.doc_id_up_to;
    let mut buffer = FieldUpdatesBuffer::new(random_update, doc_id_up_to)?;
    for i in 0..num_updates {
        let random_update = numeric_update(
            Term::from_text(term_field, &random().next_int_bound(1000).to_string()),
            "numeric",
            Some(doc_value),
        );
        let random_update = prepare_for_apply(random_update, i + 1);
        updates.push(random_update.clone());
        buffer.add_numeric_update(
            &random_update.term,
            numeric_value(&random_update),
            random_update.doc_id_up_to,
        )?;
    }
    buffer.finish()?;
    assert_buffer_updates(&buffer, updates, true)
}

/// `assertBufferUpdates(buffer, updates, termSorted)`.
fn assert_buffer_updates(
    buffer: &FieldUpdatesBuffer,
    mut updates: Vec<DocValuesUpdate>,
    term_sorted: bool,
) -> Result<()> {
    if term_sorted {
        updates.sort_by(|a, b| a.term.bytes().cmp(b.term.bytes()));
        let mut by_terms: BTreeMap<BytesRef, DocValuesUpdate> = BTreeMap::new();
        for update in updates {
            let key = update.term.bytes().clone();
            let keep_old = by_terms
                .get(&key)
                .is_some_and(|v| v.doc_id_up_to >= update.doc_id_up_to);
            if !keep_old {
                by_terms.insert(key, update);
            }
        }
        updates = by_terms.into_values().collect();
    }
    let mut iterator = buffer.iterator()?;

    let mut count = 0;
    let mut min = i64::MAX;
    let mut max = i64::MIN;
    let mut has_at_least_one_value = false;
    while let Some(value) = iterator.next()? {
        let v = buffer.get_numeric_value(count);
        let expected_update = &updates[count];
        count += 1;
        assert_equals(
            expected_update.term.bytes().utf8_to_string()?.as_str(),
            value.term_value.utf8_to_string()?.as_str(),
        );
        assert_equals(expected_update.term.field(), value.term_field.as_str());
        assert_equals(&expected_update.has_value(), &value.has_value);
        if expected_update.has_value() {
            assert_equals(&numeric_value(expected_update), &value.numeric_value);
            assert_equals(&v, &value.numeric_value);
            min = min.min(v);
            max = max.max(v);
            has_at_least_one_value = true;
        } else {
            assert_equals(&0, &value.numeric_value);
            assert_equals(&0, &v);
        }
        assert_equals(&expected_update.doc_id_up_to, &value.doc_up_to);
    }
    if has_at_least_one_value {
        assert_equals(&max, &max_numeric(buffer));
        assert_equals(&min, &min_numeric(buffer));
    } else {
        assert_equals(&0, &max_numeric(buffer));
        assert_equals(&0, &min_numeric(buffer));
    }
    assert_equals(&count, &updates.len());
    Ok(())
}
