//! Port of `lucene/core/src/test/org/apache/lucene/util/TestLiveDocs.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::util::{Bits, DenseLiveDocs, FixedBitSet, LiveDocs, SparseFixedBitSet, SparseLiveDocs};
use rucene_test_framework::util::test_util::TestUtil;

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test};

/// `TestLiveDocs`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestLiveDocs",
        vec![
            test("testSparseLiveDocsBasic", test_sparse_live_docs_basic),
            test("testDenseLiveDocsBasic", test_dense_live_docs_basic),
            test("testSparseLiveDocsIterator", test_sparse_live_docs_iterator),
            test("testDenseLiveDocsIterator", test_dense_live_docs_iterator),
            test("testSparseDenseEquivalence", test_sparse_dense_equivalence),
            test("testEmptyIterator", test_empty_iterator),
            test("testIteratorAdvance", test_iterator_advance),
            test("testRandomDeletions", test_random_deletions),
            test("testMemoryUsage", test_memory_usage),
            test(
                "testWrappingExistingBitSets",
                test_wrapping_existing_bit_sets,
            ),
            test(
                "testSparseLiveDocsLiveIterator",
                test_sparse_live_docs_live_iterator,
            ),
            test(
                "testDenseLiveDocsLiveIterator",
                test_dense_live_docs_live_iterator,
            ),
            test(
                "testLiveIteratorEquivalence",
                test_live_iterator_equivalence,
            ),
            test(
                "testLiveIteratorFullIteration",
                test_live_iterator_full_iteration,
            ),
            test("testLiveIteratorAdvance", test_live_iterator_advance),
            test(
                "testRandomDeletionsLiveIterator",
                test_random_deletions_live_iterator,
            ),
            test("testSingleDocumentSegment", test_single_document_segment),
            test("testAllDocumentsDeleted", test_all_documents_deleted),
            test("testLargeSegment", test_large_segment),
            test(
                "testFirstAndLastDocDeletion",
                test_first_and_last_doc_deletion,
            ),
        ],
    )
}

/// `String.format(Locale.ROOT, "%.2f%%", 100.0 * sparseBytes / denseBytes)`.
fn ratio(sparse_bytes: i64, dense_bytes: i64) -> String {
    format!("{:.2}%", 100.0 * sparse_bytes as f64 / dense_bytes as f64)
}

/// A `FixedBitSet` of `maxDoc` bits, all set (`fixedSet.set(0, maxDoc)`).
fn all_set(max_doc: usize) -> FixedBitSet {
    let mut fixed_set = FixedBitSet::new(max_doc);
    fixed_set.set_range(0, max_doc);
    fixed_set
}

fn test_sparse_live_docs_basic() -> Result<()> {
    // GIVEN
    let max_doc = 1000;
    let mut sparse_set = SparseFixedBitSet::new(max_doc)?;
    sparse_set.set(10);
    sparse_set.set(50);
    sparse_set.set(100);

    // WHEN
    let live_docs = SparseLiveDocs::builder(sparse_set, max_doc).build();

    // THEN
    assert_equals(&3, &live_docs.deleted_count());
    Assert::assert_false_msg("Doc 10 should be deleted", live_docs.get(10));
    Assert::assert_false_msg("Doc 50 should be deleted", live_docs.get(50));
    Assert::assert_false_msg("Doc 100 should be deleted", live_docs.get(100));
    Assert::assert_true_msg("Doc 11 should be live", live_docs.get(11));
    Assert::assert_true_msg("Doc 51 should be live", live_docs.get(51));
    Ok(())
}

fn test_dense_live_docs_basic() -> Result<()> {
    // GIVEN
    let max_doc = 1000;
    let mut fixed_set = all_set(max_doc);
    fixed_set.clear(10);
    fixed_set.clear(50);
    fixed_set.clear(100);

    // WHEN
    let live_docs = DenseLiveDocs::builder(fixed_set, max_doc).build();

    // THEN
    assert_equals(&3, &live_docs.deleted_count());
    Assert::assert_false_msg("Doc 10 should be deleted", live_docs.get(10));
    Assert::assert_false_msg("Doc 50 should be deleted", live_docs.get(50));
    Assert::assert_false_msg("Doc 100 should be deleted", live_docs.get(100));
    Assert::assert_true_msg("Doc 11 should be live", live_docs.get(11));
    Assert::assert_true_msg("Doc 51 should be live", live_docs.get(51));
    Ok(())
}

/// The `THEN` of `testSparseLiveDocsIterator` and
/// `testDenseLiveDocsIterator`.
fn check_deleted_iterator(it: &mut dyn DocIdSetIterator, deleted_docs: &[i32]) -> Result<()> {
    let iterated_docs = collect_docs(it)?;
    assert_equals_msg(
        Some("Iterator cost should match deleted count"),
        &(deleted_docs.len() as i64),
        &it.cost(),
    );
    assert_equals_msg(
        Some("Should iterate exact number of deleted docs"),
        &deleted_docs.len(),
        &iterated_docs.len(),
    );
    for i in 0..deleted_docs.len() {
        assert_equals_msg(
            Some(&format!("Deleted doc mismatch at position {i}")),
            &deleted_docs[i],
            &iterated_docs[i],
        );
    }
    Ok(())
}

fn test_sparse_live_docs_iterator() -> Result<()> {
    // GIVEN
    let max_doc = 1000;
    let mut sparse_set = SparseFixedBitSet::new(max_doc)?;
    let deleted_docs = [5, 50, 100, 150, 500, 999];
    for &doc in &deleted_docs {
        sparse_set.set(doc as usize);
    }
    let live_docs = SparseLiveDocs::builder(sparse_set, max_doc).build();

    // WHEN
    let mut it = live_docs.deleted_docs_iterator()?;

    // THEN
    check_deleted_iterator(&mut *it, &deleted_docs)
}

fn test_dense_live_docs_iterator() -> Result<()> {
    // GIVEN
    let max_doc = 1000;
    let mut fixed_set = all_set(max_doc);
    let deleted_docs = [5, 50, 100, 150, 500, 999];
    for &doc in &deleted_docs {
        fixed_set.clear(doc as usize);
    }
    let live_docs = DenseLiveDocs::builder(fixed_set, max_doc).build();

    // WHEN
    let mut it = live_docs.deleted_docs_iterator()?;

    // THEN
    check_deleted_iterator(&mut *it, &deleted_docs)
}

fn test_sparse_dense_equivalence() -> Result<()> {
    // GIVEN
    let max_doc = 1000;
    let mut sparse_set = SparseFixedBitSet::new(max_doc)?;
    let mut fixed_set = all_set(max_doc);
    let deleted_docs = [1, 10, 50, 100, 200, 500, 750, 999];
    for &doc in &deleted_docs {
        sparse_set.set(doc);
        fixed_set.clear(doc);
    }

    // WHEN
    let sparse = SparseLiveDocs::builder(sparse_set, max_doc).build();
    let dense = DenseLiveDocs::builder(fixed_set, max_doc).build();
    let mut sparse_it = sparse.deleted_docs_iterator()?;
    let mut dense_it = dense.deleted_docs_iterator()?;

    // THEN
    assert_equals_msg(
        Some("Deleted counts should match"),
        &sparse.deleted_count(),
        &dense.deleted_count(),
    );
    assert_equals_msg(
        Some("Lengths should match"),
        &sparse.length(),
        &dense.length(),
    );
    for i in 0..max_doc {
        assert_equals_msg(
            Some(&format!("get({i}) should match")),
            &sparse.get(i),
            &dense.get(i),
        );
    }
    loop {
        let sparse_doc = sparse_it.next_doc()?;
        let dense_doc = dense_it.next_doc()?;
        assert_equals_msg(
            Some("Iterators should return same documents"),
            &sparse_doc,
            &dense_doc,
        );
        if sparse_doc == NO_MORE_DOCS {
            break;
        }
    }
    Ok(())
}

fn test_empty_iterator() -> Result<()> {
    // GIVEN
    let max_doc = 1000;
    let sparse_set = SparseFixedBitSet::new(max_doc)?;
    let fixed_set = all_set(max_doc);
    let sparse = SparseLiveDocs::builder(sparse_set, max_doc).build();
    let dense = DenseLiveDocs::builder(fixed_set, max_doc).build();

    // WHEN
    let mut sparse_it = sparse.deleted_docs_iterator()?;
    let mut dense_it = dense.deleted_docs_iterator()?;

    // THEN
    assert_equals(&NO_MORE_DOCS, &sparse_it.next_doc()?);
    assert_equals(&NO_MORE_DOCS, &dense_it.next_doc()?);
    Ok(())
}

fn test_iterator_advance() -> Result<()> {
    // GIVEN
    let max_doc = 1000;
    let mut fixed_set = all_set(max_doc);
    let mut i = 10;
    while i <= 50 {
        fixed_set.clear(i);
        i += 10;
    }
    let live_docs = DenseLiveDocs::builder(fixed_set, max_doc).build();

    // WHEN
    let mut it = live_docs.deleted_docs_iterator()?;

    // THEN
    assert_equals(&20, &it.advance(15)?);
    assert_equals(&30, &it.next_doc()?);
    assert_equals(&50, &it.advance(45)?);
    assert_equals(&NO_MORE_DOCS, &it.advance(1000)?);
    Ok(())
}

fn test_random_deletions() -> Result<()> {
    // GIVEN
    let max_doc = TestUtil::next_int(&*random(), 1000, 10000) as usize;
    let delete_ratio = random().next_double() * 0.5;
    let mut sparse_set = SparseFixedBitSet::new(max_doc)?;
    let mut fixed_set = all_set(max_doc);
    let mut expected_deleted: Vec<i32> = Vec::new();
    for i in 0..max_doc {
        if random().next_double() < delete_ratio {
            sparse_set.set(i);
            fixed_set.clear(i);
            expected_deleted.push(i as i32);
        }
    }

    // WHEN
    let sparse = SparseLiveDocs::builder(sparse_set, max_doc).build();
    let dense = DenseLiveDocs::builder(fixed_set, max_doc).build();
    let sparse_deleted = collect_docs(&mut *sparse.deleted_docs_iterator()?)?;
    let dense_deleted = collect_docs(&mut *dense.deleted_docs_iterator()?)?;

    // THEN
    assert_equals_msg(
        Some("Deleted counts should match expected"),
        &expected_deleted.len(),
        &sparse.deleted_count(),
    );
    assert_equals_msg(
        Some("Sparse and dense counts should match"),
        &sparse.deleted_count(),
        &dense.deleted_count(),
    );
    for i in 0..max_doc {
        let expected_live = !expected_deleted.contains(&(i as i32));
        assert_equals_msg(
            Some(&format!("Sparse get({i}) mismatch")),
            &expected_live,
            &sparse.get(i),
        );
        assert_equals_msg(
            Some(&format!("Dense get({i}) mismatch")),
            &expected_live,
            &dense.get(i),
        );
    }
    assert_equals_msg(
        Some("Sparse iterator should return all deleted docs"),
        &expected_deleted,
        &sparse_deleted,
    );
    assert_equals_msg(
        Some("Dense iterator should return all deleted docs"),
        &expected_deleted,
        &dense_deleted,
    );
    Ok(())
}

fn test_memory_usage() -> Result<()> {
    // GIVEN
    let max_doc = 1000000;
    let mut sparse_set = SparseFixedBitSet::new(max_doc)?;
    let mut fixed_set = all_set(max_doc);
    for i in 0..max_doc / 1000 {
        sparse_set.set(i * 1000);
        fixed_set.clear(i * 1000);
    }

    // WHEN
    let sparse = SparseLiveDocs::builder(sparse_set, max_doc).build();
    let dense = DenseLiveDocs::builder(fixed_set, max_doc).build();
    let sparse_bytes = sparse.ram_bytes_used();
    let dense_bytes = dense.ram_bytes_used();

    // THEN
    Assert::assert_true_msg(
        &format!(
            "Sparse should use less memory than dense for 0.1% deletions (sparse={sparse_bytes}, dense={dense_bytes}, ratio={})",
            ratio(sparse_bytes, dense_bytes)
        ),
        sparse_bytes < dense_bytes,
    );
    Assert::assert_true_msg(
        &format!(
            "Sparse should use significantly less memory (< 50%) for very sparse deletions (ratio={})",
            ratio(sparse_bytes, dense_bytes)
        ),
        sparse_bytes < dense_bytes / 2,
    );
    Ok(())
}

fn test_wrapping_existing_bit_sets() -> Result<()> {
    // GIVEN
    let max_doc = 1000;
    let mut sparse_deleted = SparseFixedBitSet::new(max_doc)?;
    sparse_deleted.set(10);
    sparse_deleted.set(50);
    sparse_deleted.set(100);
    let mut live_set = all_set(max_doc);
    live_set.clear(10);
    live_set.clear(50);
    live_set.clear(100);

    // WHEN
    let sparse = SparseLiveDocs::builder(sparse_deleted, max_doc).build();
    let dense = DenseLiveDocs::builder(live_set, max_doc).build();

    // THEN
    assert_equals(&3, &sparse.deleted_count());
    Assert::assert_false(sparse.get(10));
    Assert::assert_false(sparse.get(50));
    Assert::assert_true(sparse.get(11));
    assert_equals(&3, &dense.deleted_count());
    Assert::assert_false(dense.get(10));
    Assert::assert_false(dense.get(50));
    Assert::assert_true(dense.get(11));
    Ok(())
}

/// The `THEN` of `testSparseLiveDocsLiveIterator` and
/// `testDenseLiveDocsLiveIterator`.
fn check_live_iterator(
    it: &mut dyn DocIdSetIterator,
    max_doc: usize,
    deleted_docs: &[i32],
) -> Result<()> {
    let iterated_docs = collect_docs(it)?;
    assert_equals_msg(
        Some("Iterator cost should match live count"),
        &((max_doc - deleted_docs.len()) as i64),
        &it.cost(),
    );
    assert_equals_msg(
        Some("Should iterate exact number of live docs"),
        &(max_doc - deleted_docs.len()),
        &iterated_docs.len(),
    );
    for deleted_doc in deleted_docs {
        Assert::assert_false_msg(
            &format!("Deleted doc {deleted_doc} should not be in live iterator"),
            iterated_docs.contains(deleted_doc),
        );
    }
    for i in 1..iterated_docs.len() {
        Assert::assert_true_msg(
            "Docs should be in ascending order",
            iterated_docs[i] > iterated_docs[i - 1],
        );
    }
    Ok(())
}

fn test_sparse_live_docs_live_iterator() -> Result<()> {
    // GIVEN
    let max_doc = 100;
    let mut sparse_set = SparseFixedBitSet::new(max_doc)?;
    let deleted_docs = [5, 10, 50, 99];
    for &doc in &deleted_docs {
        sparse_set.set(doc as usize);
    }
    let live_docs = SparseLiveDocs::builder(sparse_set, max_doc).build();

    // WHEN
    let mut it = live_docs.live_docs_iterator()?;

    // THEN
    check_live_iterator(&mut *it, max_doc, &deleted_docs)
}

fn test_dense_live_docs_live_iterator() -> Result<()> {
    // GIVEN
    let max_doc = 100;
    let mut fixed_set = all_set(max_doc);
    let deleted_docs = [5, 10, 50, 99];
    for &doc in &deleted_docs {
        fixed_set.clear(doc as usize);
    }
    let live_docs = DenseLiveDocs::builder(fixed_set, max_doc).build();

    // WHEN
    let mut it = live_docs.live_docs_iterator()?;

    // THEN
    check_live_iterator(&mut *it, max_doc, &deleted_docs)
}

fn test_live_iterator_equivalence() -> Result<()> {
    // GIVEN
    let max_doc = 1000;
    let mut sparse_set = SparseFixedBitSet::new(max_doc)?;
    let mut fixed_set = all_set(max_doc);
    let deleted_docs = [1, 10, 50, 100, 200, 500, 750, 999];
    for &doc in &deleted_docs {
        sparse_set.set(doc);
        fixed_set.clear(doc);
    }
    let sparse = SparseLiveDocs::builder(sparse_set, max_doc).build();
    let dense = DenseLiveDocs::builder(fixed_set, max_doc).build();

    // WHEN
    let mut sparse_it = sparse.live_docs_iterator()?;
    let mut dense_it = dense.live_docs_iterator()?;

    // THEN
    loop {
        let sparse_doc = sparse_it.next_doc()?;
        let dense_doc = dense_it.next_doc()?;
        assert_equals_msg(
            Some("Live iterators should return same documents"),
            &sparse_doc,
            &dense_doc,
        );
        if sparse_doc == NO_MORE_DOCS {
            break;
        }
    }
    Ok(())
}

fn test_live_iterator_full_iteration() -> Result<()> {
    // GIVEN
    let max_doc = 1000;
    let sparse_set = SparseFixedBitSet::new(max_doc)?;
    let fixed_set = all_set(max_doc);
    let sparse = SparseLiveDocs::builder(sparse_set, max_doc).build();
    let dense = DenseLiveDocs::builder(fixed_set, max_doc).build();

    // WHEN
    let mut sparse_it = sparse.live_docs_iterator()?;
    let mut dense_it = dense.live_docs_iterator()?;
    let sparse_docs = collect_docs(&mut *sparse_it)?;
    let dense_docs = collect_docs(&mut *dense_it)?;

    // THEN
    assert_equals_msg(
        Some("Sparse iterator should return all docs"),
        &max_doc,
        &sparse_docs.len(),
    );
    assert_equals_msg(
        Some("Dense iterator should return all docs"),
        &max_doc,
        &dense_docs.len(),
    );
    for i in 0..max_doc {
        assert_equals_msg(
            Some(&format!("Sparse doc at position {i}")),
            &(i as i32),
            &sparse_docs[i],
        );
        assert_equals_msg(
            Some(&format!("Dense doc at position {i}")),
            &(i as i32),
            &dense_docs[i],
        );
    }
    Ok(())
}

fn test_live_iterator_advance() -> Result<()> {
    // GIVEN
    let max_doc = 1000;
    let mut fixed_set = all_set(max_doc);
    let mut i = 10;
    while i <= 50 {
        fixed_set.clear(i);
        i += 10;
    }
    let live_docs = DenseLiveDocs::builder(fixed_set, max_doc).build();

    // WHEN
    let mut it = live_docs.live_docs_iterator()?;

    // THEN
    assert_equals(&11, &it.advance(10)?);
    assert_equals(&12, &it.next_doc()?);
    assert_equals(&21, &it.advance(20)?);
    assert_equals(&500, &it.advance(500)?);
    assert_equals(&NO_MORE_DOCS, &it.advance(1000)?);
    Ok(())
}

fn test_random_deletions_live_iterator() -> Result<()> {
    // GIVEN
    let max_doc = TestUtil::next_int(&*random(), 1000, 10000) as usize;
    let delete_ratio = random().next_double() * 0.5;
    let mut sparse_set = SparseFixedBitSet::new(max_doc)?;
    let mut fixed_set = all_set(max_doc);
    let mut expected_live: Vec<i32> = Vec::new();
    for i in 0..max_doc {
        if random().next_double() < delete_ratio {
            sparse_set.set(i);
            fixed_set.clear(i);
        } else {
            expected_live.push(i as i32);
        }
    }

    // WHEN
    let sparse = SparseLiveDocs::builder(sparse_set, max_doc).build();
    let dense = DenseLiveDocs::builder(fixed_set, max_doc).build();
    let sparse_live = collect_docs(&mut *sparse.live_docs_iterator()?)?;
    let dense_live = collect_docs(&mut *dense.live_docs_iterator()?)?;

    // THEN
    assert_equals_msg(
        Some("Sparse live iterator should return all live docs"),
        &expected_live,
        &sparse_live,
    );
    assert_equals_msg(
        Some("Dense live iterator should return all live docs"),
        &expected_live,
        &dense_live,
    );
    Ok(())
}

/// `collectDocs(it)`.
fn collect_docs(it: &mut dyn DocIdSetIterator) -> Result<Vec<i32>> {
    let mut docs = Vec::new();
    let mut doc = it.next_doc()?;
    while doc != NO_MORE_DOCS {
        docs.push(doc);
        doc = it.next_doc()?;
    }
    Ok(docs)
}

fn test_single_document_segment() -> Result<()> {
    // GIVEN
    let max_doc = 1;
    let mut sparse_set = SparseFixedBitSet::new(max_doc)?;
    let mut fixed_set = all_set(max_doc);
    sparse_set.set(0);
    fixed_set.clear(0);

    // WHEN
    let sparse = SparseLiveDocs::builder(sparse_set, max_doc).build();
    let dense = DenseLiveDocs::builder(fixed_set, max_doc).build();
    let sparse_deleted = collect_docs(&mut *sparse.deleted_docs_iterator()?)?;
    let dense_deleted = collect_docs(&mut *dense.deleted_docs_iterator()?)?;
    let sparse_live = collect_docs(&mut *sparse.live_docs_iterator()?)?;
    let dense_live = collect_docs(&mut *dense.live_docs_iterator()?)?;

    // THEN
    Assert::assert_false_msg("Doc 0 should be deleted", sparse.get(0));
    Assert::assert_false_msg("Doc 0 should be deleted", dense.get(0));
    assert_equals(&1, &sparse.deleted_count());
    assert_equals(&1, &dense.deleted_count());
    assert_equals(&vec![0], &sparse_deleted);
    assert_equals(&vec![0], &dense_deleted);
    assert_equals(&0, &sparse_live.len());
    assert_equals(&0, &dense_live.len());
    Ok(())
}

fn test_all_documents_deleted() -> Result<()> {
    // GIVEN
    let max_doc = 100;
    let mut sparse_set = SparseFixedBitSet::new(max_doc)?;
    let mut fixed_set = all_set(max_doc);
    for i in 0..max_doc {
        sparse_set.set(i);
        fixed_set.clear(i);
    }

    // WHEN
    let sparse = SparseLiveDocs::builder(sparse_set, max_doc).build();
    let dense = DenseLiveDocs::builder(fixed_set, max_doc).build();
    let sparse_deleted = collect_docs(&mut *sparse.deleted_docs_iterator()?)?;
    let dense_deleted = collect_docs(&mut *dense.deleted_docs_iterator()?)?;
    let sparse_live = collect_docs(&mut *sparse.live_docs_iterator()?)?;
    let dense_live = collect_docs(&mut *dense.live_docs_iterator()?)?;

    // THEN
    assert_equals(&max_doc, &sparse.deleted_count());
    assert_equals(&max_doc, &dense.deleted_count());
    for i in 0..max_doc {
        Assert::assert_false_msg(&format!("Doc {i} should be deleted"), sparse.get(i));
        Assert::assert_false_msg(&format!("Doc {i} should be deleted"), dense.get(i));
    }
    assert_equals(&max_doc, &sparse_deleted.len());
    assert_equals(&max_doc, &dense_deleted.len());
    for i in 0..max_doc {
        assert_equals(&(i as i32), &sparse_deleted[i]);
        assert_equals(&(i as i32), &dense_deleted[i]);
    }
    assert_equals(&0, &sparse_live.len());
    assert_equals(&0, &dense_live.len());
    Ok(())
}

fn test_large_segment() -> Result<()> {
    // GIVEN
    let max_doc = 10_000_000;
    let mut sparse_set = SparseFixedBitSet::new(max_doc)?;
    let mut fixed_set = all_set(max_doc);
    let mut i = 0;
    while i < max_doc {
        sparse_set.set(i);
        fixed_set.clear(i);
        i += 1000;
    }

    // WHEN
    let sparse = SparseLiveDocs::builder(sparse_set, max_doc).build();
    let dense = DenseLiveDocs::builder(fixed_set, max_doc).build();
    let mut sparse_it = sparse.deleted_docs_iterator()?;
    let sparse_bytes = sparse.ram_bytes_used();
    let dense_bytes = dense.ram_bytes_used();

    // THEN
    assert_equals(&10_000, &sparse.deleted_count());
    assert_equals(&10_000, &dense.deleted_count());
    assert_equals(&0, &sparse_it.next_doc()?);
    sparse_it.advance((max_doc - 1000) as i32)?;
    assert_equals(&((max_doc - 1000) as i32), &sparse_it.doc_id());
    Assert::assert_true_msg(
        &format!(
            "Sparse should use less memory than dense for 0.1% deletions (sparse={sparse_bytes}, dense={dense_bytes})"
        ),
        sparse_bytes < dense_bytes,
    );
    Assert::assert_true_msg(
        &format!(
            "Sparse should use significantly less memory (<50%) for very sparse deletions (ratio={})",
            ratio(sparse_bytes, dense_bytes)
        ),
        sparse_bytes < dense_bytes / 2,
    );
    Ok(())
}

fn test_first_and_last_doc_deletion() -> Result<()> {
    // GIVEN
    let max_doc = 1000;
    let mut sparse_set = SparseFixedBitSet::new(max_doc)?;
    let mut fixed_set = all_set(max_doc);
    sparse_set.set(0);
    sparse_set.set(max_doc - 1);
    fixed_set.clear(0);
    fixed_set.clear(max_doc - 1);

    // WHEN
    let sparse = SparseLiveDocs::builder(sparse_set, max_doc).build();
    let dense = DenseLiveDocs::builder(fixed_set, max_doc).build();
    let sparse_deleted = collect_docs(&mut *sparse.deleted_docs_iterator()?)?;
    let dense_deleted = collect_docs(&mut *dense.deleted_docs_iterator()?)?;
    let sparse_live = collect_docs(&mut *sparse.live_docs_iterator()?)?;
    let dense_live = collect_docs(&mut *dense.live_docs_iterator()?)?;

    // THEN
    Assert::assert_false_msg("First doc should be deleted", sparse.get(0));
    Assert::assert_false_msg("Last doc should be deleted", sparse.get(max_doc - 1));
    Assert::assert_false_msg("First doc should be deleted", dense.get(0));
    Assert::assert_false_msg("Last doc should be deleted", dense.get(max_doc - 1));
    assert_equals(&2, &sparse_deleted.len());
    assert_equals(&2, &dense_deleted.len());
    assert_equals(&0, &sparse_deleted[0]);
    assert_equals(&((max_doc - 1) as i32), &sparse_deleted[1]);
    assert_equals(&(max_doc - 2), &sparse_live.len());
    assert_equals(&(max_doc - 2), &dense_live.len());
    assert_equals(&1, &sparse_live[0]);
    assert_equals(&((max_doc - 2) as i32), &sparse_live[sparse_live.len() - 1]);
    Ok(())
}
