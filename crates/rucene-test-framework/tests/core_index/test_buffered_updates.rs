//! Port of `lucene/core/src/test/org/apache/lucene/index/TestBufferedUpdates.java`:
//! Unit test for `BufferedUpdates`.

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::documents_writer::{BufferedUpdates, DeletedTerms};
use rucene::index::terms::Term;
use rucene::search::TermQuery;
use rucene::util::{Accountable, BytesRef};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_array_equals, assert_equals, class, random, test};

/// `TestBufferedUpdates`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestBufferedUpdates",
        vec![
            test("testRamBytesUsed", test_ram_bytes_used),
            test("testDeletedTerms", test_deleted_terms),
        ],
    )
}

/// Port of `TestBufferedUpdates.testRamBytesUsed`.
fn test_ram_bytes_used() -> Result<()> {
    let mut bu = BufferedUpdates::new("seg1");
    assert_equals(&bu.ram_bytes_used(), &0i64);
    Assert::assert_false(bu.any());
    let queries = LuceneTestCase::at_least(1);
    for _ in 0..queries {
        let doc_id_upto = if random().next_boolean() {
            i32::MAX
        } else {
            random().next_int_bound(100000)
        };
        let term = Term::from_text("id", &random().next_int_bound(100).to_string());
        bu.add_query(Arc::new(TermQuery::new(term)), doc_id_upto);
    }

    let terms = LuceneTestCase::at_least(1);
    for _ in 0..terms {
        let doc_id_upto = if random().next_boolean() {
            i32::MAX
        } else {
            random().next_int_bound(100000)
        };
        let term = Term::from_text("id", &random().next_int_bound(100).to_string());
        bu.add_term(term, doc_id_upto)?;
    }
    Assert::assert_true_msg("we have added tons of docIds, terms and queries", bu.any());

    let total_used = bu.ram_bytes_used();
    Assert::assert_true(total_used > 0);

    bu.clear_delete_terms();
    Assert::assert_true_msg(
        "only terms and docIds are cleaned, the queries are still in memory",
        bu.any(),
    );
    Assert::assert_true_msg(
        "terms are cleaned, ram in used should decrease",
        total_used > bu.ram_bytes_used(),
    );

    bu.clear();
    Assert::assert_false(bu.any());
    assert_equals(&bu.ram_bytes_used(), &0i64);
    Ok(())
}

/// Port of `TestBufferedUpdates.testDeletedTerms`.
fn test_deleted_terms() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    let fields = ["a", "b", "c"];
    let mut actual = DeletedTerms::new();
    for _ in 0..iters {
        let mut expected: HashMap<Term, i32> = HashMap::new();
        Assert::assert_true(actual.is_empty());

        let term_count = LuceneTestCase::at_least(5000);
        let max_bytes_num = random().next_int_bound(3) + 1;
        for _ in 0..term_count {
            let byte_num = random().next_int_bound(max_bytes_num) + 1;
            let mut bytes = vec![0u8; byte_num as usize];
            random().next_bytes(&mut bytes);
            let term = Term::new(
                fields[random().next_int_bound(fields.len() as i32) as usize],
                BytesRef::new(bytes),
            );
            let value = random().next_int_bound(10000000);
            expected.insert(term.clone(), value);
            actual.put(&term, value)?;
        }

        assert_equals(&expected.len(), &actual.size());

        for (key, value) in &expected {
            assert_equals(value, &actual.get(key));
        }

        let mut expected_sorted: Vec<(Term, i32)> =
            expected.iter().map(|(k, v)| (k.clone(), *v)).collect();
        expected_sorted.sort_by(|a, b| a.0.cmp(&b.0));
        let mut actual_sorted: Vec<(Term, i32)> = Vec::new();
        actual.for_each_ordered(|term, doc_id| {
            let copy = Term::new(term.field(), BytesRef::deep_copy_of(term.bytes()));
            actual_sorted.push((copy, doc_id));
        });
        assert_array_equals(&expected_sorted, &actual_sorted);

        actual.clear();
        assert_equals(&0, &actual.size());
        assert_equals(&0, &actual.ram_bytes_used());
        // `assertNull(actual.getPool().buffer)`: a pool reset without reusing
        // its first block holds no head block.
        assert_equals(&-1, &actual.get_pool().buffer_upto());
    }
    Ok(())
}
