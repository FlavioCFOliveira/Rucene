//! Port of `lucene/core/src/test/org/apache/lucene/codecs/lucene90/TestIndexedDISI.java`.
//!
//! Copied from the lucene70 package for separation of codec-code.
//!
//! Rucene's `BitSetIterator` shares its set (`Arc<dyn BitSet>`); the tests
//! that change a set after iterating it reach it through `Arc::get_mut`
//! once every iterator over it is gone ([`bits`]).

use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::codecs::lucene90::indexed_disi::{
    create_block_slice, create_jump_table, write_bit_set, Method, MAX_ARRAY_LENGTH,
};
use rucene::codecs::lucene90::IndexedDISI;
use rucene::error::Result;
use rucene::index::doc_values::DocValuesIterator;
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::store::{Directory, DEFAULT_IO_CONTEXT};
use rucene::util::{bit_set_of, BitSet, BitSetIterator, Bits, FixedBitSet, SparseFixedBitSet};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::Nightly;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_equals, class, random, test,
};

/// `TestIndexedDISI`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.codecs.lucene90.TestIndexedDISI",
        vec![
            test("testEmpty", test_empty),
            test("testEmptyBlocks", test_empty_blocks).annotate(Nightly),
            test("testLastEmptyBlocks", test_last_empty_blocks),
            test("testRandomBlocks", test_random_blocks).annotate(Nightly),
            test("testPositionNotZero", test_position_not_zero),
            test("testOneDoc", test_one_doc),
            test("testTwoDocs", test_two_docs),
            test("testAllDocs", test_all_docs),
            test("testHalfFull", test_half_full),
            test("testDocRange", test_doc_range),
            test("testSparseDenseBoundary", test_sparse_dense_boundary),
            test("testOneDocMissing", test_one_doc_missing),
            test("testFewMissingDocs", test_few_missing_docs),
            test("testDenseMultiBlock", test_dense_multi_block),
            test(
                "testDenseBitSizeLessThanBlockSize",
                test_dense_bit_size_less_than_block_size,
            ),
            test("testIllegalDenseRankPower", test_illegal_dense_rank_power),
            test("testOneDocMissingFixed", test_one_doc_missing_fixed),
            test("testRandom", test_random),
        ],
    )
}

/// The set behind `set`, for a change: no `BitSetIterator` holds it then.
fn bits(set: &mut Arc<dyn BitSet>) -> &mut dyn BitSet {
    Arc::get_mut(set).expect("every BitSetIterator over the set is dropped")
}

/// `new SparseFixedBitSet(maxDoc)` as a `BitSet`.
fn sparse(max_doc: i32) -> Result<Arc<dyn BitSet>> {
    Ok(Arc::new(SparseFixedBitSet::new(max_doc as usize)?))
}

/// `new BitSetIterator(set, cost)`.
fn iterator(set: &Arc<dyn BitSet>, cost: i64) -> Result<BitSetIterator> {
    BitSetIterator::new(Arc::clone(set), cost)
}

/// `rarely() ? -1 : (byte) (random().nextInt(7) + 7)`: sane + chance of
/// disable.
fn random_dense_rank_power() -> i8 {
    if LuceneTestCase::rarely() {
        -1
    } else {
        (random().next_int_bound(7) + 7) as i8
    }
}

/// Port of `TestIndexedDISI.testEmpty`.
fn test_empty() -> Result<()> {
    let max_doc = TestUtil::next_int(&*random(), 1, 100000);
    let set = sparse(max_doc)?;
    let dir = LuceneTestCase::new_directory();
    do_test(&set, &*dir.as_directory())?;
    dir.close()
}

/// EMPTY blocks are special with regard to jumps as they have size 0
///
/// Port of `TestIndexedDISI.testEmptyBlocks`.
fn test_empty_blocks() -> Result<()> {
    const B: usize = 65536;
    let max_doc = B * 11;
    let mut set = sparse(max_doc as i32)?;
    {
        let s = bits(&mut set);
        // block 0: EMPTY
        s.set(B + 5); // block 1: SPARSE
                      // block 2: EMPTY
                      // block 3: EMPTY
        s.set(B * 4 + 5); // block 4: SPARSE

        for i in 0..B {
            s.set(B * 6 + i); // block 6: ALL
        }
        for i in (0..B).step_by(3) {
            s.set(B * 7 + i); // block 7: DENSE
        }
        for i in 0..B {
            if i != 32768 {
                s.set(B * 8 + i); // block 8: DENSE (all-1)
            }
        }
        // block 9-11: EMPTY
    }

    {
        let dir = LuceneTestCase::new_directory();
        do_test_all_single_jump(&set, &*dir.as_directory())?;
        dir.close()?;
    }

    // Change the first block to DENSE to see if jump-tables sets to position 0
    bits(&mut set).set(0);
    let dir = LuceneTestCase::new_directory();
    do_test_all_single_jump(&set, &*dir.as_directory())?;
    dir.close()
}

/// EMPTY blocks are special with regard to jumps as they have size 0
///
/// Port of `TestIndexedDISI.testLastEmptyBlocks`.
fn test_last_empty_blocks() -> Result<()> {
    const B: usize = 65536;
    let max_doc = B * 3;
    let mut set = sparse(max_doc as i32)?;
    {
        let s = bits(&mut set);
        for doc_id in 0..B * 2 {
            // first 2 blocks are ALL
            s.set(doc_id);
        }
    }
    // Last block is EMPTY

    let dir = LuceneTestCase::new_directory();
    {
        let d = dir.as_directory();
        do_test_all_single_jump(&set, &*d)?;
        assert_advance_beyond_end(&set, &*d)?;
    }
    dir.close()
}

/// Checks that advance after the end of the blocks has been reached has the correct behaviour
///
/// Port of `TestIndexedDISI.assertAdvanceBeyondEnd`.
fn assert_advance_beyond_end(set: &Arc<dyn BitSet>, dir: &dyn Directory) -> Result<()> {
    let cardinality = set.cardinality() as i32;
    let dense_rank_power: i8 = 9; // Not tested here so fixed to isolate factors
    let jump_tableentry_count;
    {
        let mut out = dir.create_output("bar", &*DEFAULT_IO_CONTEXT)?;
        jump_tableentry_count = write_bit_set(
            &mut iterator(set, cardinality as i64)?,
            &mut *out,
            dense_rank_power,
        )?;
        out.close()?;
    }

    let mut r#in = dir.open_input("bar", &*DEFAULT_IO_CONTEXT)?;
    {
        let mut disi2 = iterator(set, cardinality as i64)?;
        let mut doc = disi2.doc_id();
        let mut index = 0;
        while doc < cardinality {
            doc = disi2.next_doc()?;
            index += 1;
        }

        let mut disi = IndexedDISI::new(
            &*r#in,
            0,
            r#in.length(),
            jump_tableentry_count as i32,
            dense_rank_power,
            cardinality as i64,
        )?;
        // Advance 1 docID beyond end
        Assert::assert_false_msg(
            "There should be no set bit beyond the valid docID range",
            disi.advance_exact(Bits::length(&**set) as i32)?,
        );
        disi.advance(doc)?; // Should be the special docID signifyin NO_MORE_DOCS from the BitSetIterator
        assert_equals_msg(
            Some("The index when advancing beyond the last defined docID should be correct"),
            &index,
            &(disi.index() + 1),
        ); // disi.index()+1 as the while-loop also counts the NO_MORE_DOCS
    }
    r#in.close()
}

/// TODO: can this be toned down?
///
/// Port of `TestIndexedDISI.testRandomBlocks`.
fn test_random_blocks() -> Result<()> {
    const BLOCKS: i32 = 5;
    let set = create_set_with_random_blocks(BLOCKS)?;
    let dir = LuceneTestCase::new_directory();
    do_test_all_single_jump(&set, &*dir.as_directory())?;
    dir.close()
}

/// When doing merges in Lucene80NormsProducer, IndexedDISI are created from slices where the
/// offset is not 0
///
/// Port of `TestIndexedDISI.testPositionNotZero`.
fn test_position_not_zero() -> Result<()> {
    const BLOCKS: i32 = 10;
    let dense_rank_power = random_dense_rank_power();

    let set = create_set_with_random_blocks(BLOCKS)?;
    let dir = LuceneTestCase::new_directory();
    {
        let d = dir.as_directory();
        let cardinality = set.cardinality() as i64;
        let jump_table_entry_count;
        {
            let mut out = d.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
            jump_table_entry_count = write_bit_set(
                &mut iterator(&set, cardinality)?,
                &mut *out,
                dense_rank_power,
            )? as i32;
            out.close()?;
        }
        let mut full_input = d.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
        {
            let mut block_data = create_block_slice(
                &*full_input,
                "blocks",
                0,
                full_input.length(),
                jump_table_entry_count,
            )?;
            let seek_to = random().next_int_bound(block_data.length() as i32);
            block_data.seek(seek_to as i64)?;

            let jump_table =
                create_jump_table(&*full_input, 0, full_input.length(), jump_table_entry_count)?;
            let mut disi = IndexedDISI::with_slices(
                block_data,
                jump_table,
                jump_table_entry_count,
                dense_rank_power,
                cardinality,
            )?;
            // This failed at some point during LUCENE-8585 development as it did not reset the slice
            // position
            disi.advance_exact(BLOCKS * 65536 - 1)?;
        }
        full_input.close()?;
    }
    dir.close()
}

/// Port of `TestIndexedDISI.createSetWithRandomBlocks`.
fn create_set_with_random_blocks(block_count: i32) -> Result<Arc<dyn BitSet>> {
    const B: i32 = 65536;
    let mut set = sparse(block_count * B)?;
    {
        let s = bits(&mut set);
        for block in 0..block_count {
            match random().next_int_bound(4) {
                0 => {
                    // EMPTY
                }
                1 => {
                    // ALL
                    for doc_id in block * B..(block + 1) * B {
                        s.set(doc_id as usize);
                    }
                }
                2 => {
                    // SPARSE ( < 4096 )
                    for doc_id in (block * B..(block + 1) * B).step_by(101) {
                        s.set(doc_id as usize);
                    }
                }
                3 => {
                    // DENSE ( >= 4096 )
                    for doc_id in (block * B..(block + 1) * B).step_by(3) {
                        s.set(doc_id as usize);
                    }
                }
                _ => {
                    return Err(rucene::error::LuceneError::illegal_state(
                        "Modulo logic error: there should only be 4 possibilities".to_string(),
                    ))
                }
            }
        }
    }
    Ok(set)
}

/// Port of `TestIndexedDISI.doTestAllSingleJump`.
fn do_test_all_single_jump(set: &Arc<dyn BitSet>, dir: &dyn Directory) -> Result<()> {
    let cardinality = set.cardinality() as i64;
    let dense_rank_power = random_dense_rank_power();
    let length;
    let jump_tableentry_count;
    {
        let mut out = dir.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
        jump_tableentry_count = write_bit_set(
            &mut iterator(set, cardinality)?,
            &mut *out,
            dense_rank_power,
        )? as i32;
        length = out.file_pointer();
        out.close()?;
    }

    let mut r#in = dir.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
    for i in 0..Bits::length(&**set) {
        let i32_ = i as i32;
        let mut disi = IndexedDISI::new(
            &*r#in,
            0,
            length,
            jump_tableentry_count,
            dense_rank_power,
            cardinality,
        )?;
        assert_equals_msg(
            Some(&format!(
                "The bit at {i} should be correct with advanceExact"
            )),
            &Bits::get(&**set, i),
            &disi.advance_exact(i32_)?,
        );

        let mut disi2 = IndexedDISI::new(
            &*r#in,
            0,
            length,
            jump_tableentry_count,
            dense_rank_power,
            cardinality,
        )?;
        disi2.advance(i32_)?;
        // Proper sanity check with jump tables as an error could make them seek backwards
        Assert::assert_true_msg(
            &format!(
                "The docID should at least be {i} after advance({i}) but was {}",
                disi2.doc_id()
            ),
            i32_ <= disi2.doc_id(),
        );
        if Bits::get(&**set, i) {
            assert_equals_msg(
                Some("The docID should be present with advance"),
                &i32_,
                &disi2.doc_id(),
            );
        } else {
            // `assertNotSame(…, i, disi2.docID())` over two boxed integers.
            Assert::assert_true_msg(
                "The docID should not be present with advance",
                i32_ != disi2.doc_id(),
            );
        }
    }
    r#in.close()
}

/// Port of `TestIndexedDISI.testOneDoc`.
fn test_one_doc() -> Result<()> {
    let max_doc = TestUtil::next_int(&*random(), 1, 100000);
    let mut set = sparse(max_doc)?;
    bits(&mut set).set(random().next_int_bound(max_doc) as usize);
    let dir = LuceneTestCase::new_directory();
    do_test(&set, &*dir.as_directory())?;
    dir.close()
}

/// Port of `TestIndexedDISI.testTwoDocs`.
fn test_two_docs() -> Result<()> {
    let max_doc = TestUtil::next_int(&*random(), 1, 100000);
    let mut set = sparse(max_doc)?;
    bits(&mut set).set(random().next_int_bound(max_doc) as usize);
    bits(&mut set).set(random().next_int_bound(max_doc) as usize);
    let dir = LuceneTestCase::new_directory();
    do_test(&set, &*dir.as_directory())?;
    dir.close()
}

/// Port of `TestIndexedDISI.testAllDocs`.
fn test_all_docs() -> Result<()> {
    let max_doc = TestUtil::next_int(&*random(), 1, 100000);
    let mut set = FixedBitSet::new(max_doc as usize);
    set.set_range(1, max_doc as usize);
    let set: Arc<dyn BitSet> = Arc::new(set);
    let dir = LuceneTestCase::new_directory();
    do_test(&set, &*dir.as_directory())?;
    dir.close()
}

/// Port of `TestIndexedDISI.testHalfFull`.
fn test_half_full() -> Result<()> {
    let max_doc = TestUtil::next_int(&*random(), 1, 100000);
    let mut set = sparse(max_doc)?;
    {
        let s = bits(&mut set);
        let mut i = random().next_int_bound(2);
        while i < max_doc {
            s.set(i as usize);
            i += TestUtil::next_int(&*random(), 1, 3);
        }
    }
    let dir = LuceneTestCase::new_directory();
    do_test(&set, &*dir.as_directory())?;
    dir.close()
}

/// Port of `TestIndexedDISI.testDocRange`.
fn test_doc_range() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    {
        let d = dir.as_directory();
        for _ in 0..10 {
            let max_doc = TestUtil::next_int(&*random(), 1, 1000000);
            let mut set = FixedBitSet::new(max_doc as usize);
            let start = random().next_int_bound(max_doc);
            let end = TestUtil::next_int(&*random(), start + 1, max_doc);
            set.set_range(start as usize, end as usize);
            let set: Arc<dyn BitSet> = Arc::new(set);
            do_test(&set, &*d)?;
        }
    }
    dir.close()
}

/// Port of `TestIndexedDISI.testSparseDenseBoundary`.
fn test_sparse_dense_boundary() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    {
        let d = dir.as_directory();
        let mut fixed = FixedBitSet::new(200000);
        let start = 65536 + random().next_int_bound(100);
        let dense_rank_power = random_dense_rank_power();

        // we set MAX_ARRAY_LENGTH bits so the encoding will be sparse
        fixed.set_range(start as usize, start as usize + MAX_ARRAY_LENGTH);
        let mut set: Arc<dyn BitSet> = Arc::new(fixed);
        let mut length;
        let jump_table_entry_count;
        {
            let mut out = d.create_output("sparse", &*DEFAULT_IO_CONTEXT)?;
            jump_table_entry_count = write_bit_set(
                &mut iterator(&set, MAX_ARRAY_LENGTH as i64)?,
                &mut *out,
                dense_rank_power,
            )? as i32;
            length = out.file_pointer();
            out.close()?;
        }
        {
            let mut r#in = d.open_input("sparse", &*DEFAULT_IO_CONTEXT)?;
            {
                let mut disi = IndexedDISI::new(
                    &*r#in,
                    0,
                    length,
                    jump_table_entry_count,
                    dense_rank_power,
                    MAX_ARRAY_LENGTH as i64,
                )?;
                assert_equals(&start, &disi.next_doc()?);
                assert_equals(&Method::Sparse, &disi.method);
            }
            r#in.close()?;
        }
        do_test(&set, &*d)?;

        // now we set one more bit so the encoding will be dense
        let extra = start as usize + MAX_ARRAY_LENGTH + random().next_int_bound(100) as usize;
        bits(&mut set).set(extra);
        {
            let mut out = d.create_output("bar", &*DEFAULT_IO_CONTEXT)?;
            write_bit_set(
                &mut iterator(&set, MAX_ARRAY_LENGTH as i64 + 1)?,
                &mut *out,
                dense_rank_power,
            )?;
            length = out.file_pointer();
            out.close()?;
        }
        {
            let mut r#in = d.open_input("bar", &*DEFAULT_IO_CONTEXT)?;
            {
                let mut disi = IndexedDISI::new(
                    &*r#in,
                    0,
                    length,
                    jump_table_entry_count,
                    dense_rank_power,
                    MAX_ARRAY_LENGTH as i64 + 1,
                )?;
                assert_equals(&start, &disi.next_doc()?);
                assert_equals(&Method::Dense, &disi.method);
            }
            r#in.close()?;
        }
        do_test(&set, &*d)?;
    }
    dir.close()
}

/// Port of `TestIndexedDISI.testOneDocMissing`.
fn test_one_doc_missing() -> Result<()> {
    let max_doc = TestUtil::next_int(&*random(), 1, 1000000);
    let mut set = FixedBitSet::new(max_doc as usize);
    set.set_range(0, max_doc as usize);
    set.clear(random().next_int_bound(max_doc) as usize);
    let set: Arc<dyn BitSet> = Arc::new(set);
    let dir = LuceneTestCase::new_directory();
    do_test(&set, &*dir.as_directory())?;
    dir.close()
}

/// Port of `TestIndexedDISI.testFewMissingDocs`.
fn test_few_missing_docs() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    {
        let d = dir.as_directory();
        let num_iters = LuceneTestCase::at_least(10);
        for _ in 0..num_iters {
            let max_doc = TestUtil::next_int(&*random(), 1, 100000);
            let mut set = FixedBitSet::new(max_doc as usize);
            set.set_range(0, max_doc as usize);
            let num_missing_docs = TestUtil::next_int(&*random(), 2, 1000);
            for _ in 0..num_missing_docs {
                set.clear(random().next_int_bound(max_doc) as usize);
            }
            let set: Arc<dyn BitSet> = Arc::new(set);
            do_test(&set, &*d)?;
        }
    }
    dir.close()
}

/// Port of `TestIndexedDISI.testDenseMultiBlock`.
fn test_dense_multi_block() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    {
        let max_doc = 10 * 65536; // 10 blocks
        let mut set = FixedBitSet::new(max_doc);
        for i in (0..max_doc).step_by(2) {
            // Set every other to ensure dense
            set.set(i);
        }
        let set: Arc<dyn BitSet> = Arc::new(set);
        do_test(&set, &*dir.as_directory())?;
    }
    dir.close()
}

/// Port of `TestIndexedDISI.testDenseBitSizeLessThanBlockSize`.
fn test_dense_bit_size_less_than_block_size() -> Result<()> {
    let dense_rank_power = (random().next_int_bound(7) + 7) as i8;
    let dir = LuceneTestCase::new_directory();
    {
        let d = dir.as_directory();
        // initialize a maxDoc that is less than IndexedDISI.BLOCK_SIZE
        let max_doc = random().next_int_range(4096 * 2, 65536);
        let mut fixed = FixedBitSet::new(max_doc as usize);
        for i in (0..max_doc as usize).step_by(2) {
            // Set every other to ensure dense
            fixed.set(i);
        }
        let set: Arc<dyn BitSet> = Arc::new(fixed);
        let jump_table_entry_count; // this should always be 0 given that maxDoc < BLOCK_SIZE
        let length;
        {
            let mut out = d.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
            jump_table_entry_count = write_bit_set(
                &mut iterator(&set, set.cardinality() as i64)?,
                &mut *out,
                dense_rank_power,
            )? as i32;
            length = out.file_pointer();
            Assert::assert_true_msg(
                "jumpTableEntryCount should be 0 for dense bitsets with size < BLOCK_SIZE",
                0 == jump_table_entry_count,
            );
            out.close()?;
        }
        let mut r#in = d.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
        {
            let mut disi = IndexedDISI::new(
                &*r#in,
                0,
                length,
                jump_table_entry_count,
                dense_rank_power,
                set.cardinality() as i64,
            )?;
            let mut disi_set = FixedBitSet::new(max_doc as usize);
            // This would throw IOOB if bitset size is not handled correctly as per #14882
            BitSet::or(&mut disi_set, &mut disi)?;
        }
        r#in.close()?;
    }
    dir.close()
}

/// Port of `TestIndexedDISI.testIllegalDenseRankPower`.
fn test_illegal_dense_rank_power() -> Result<()> {
    // Legal values
    for dense_rank_power in [-1i8, 7, 8, 9, 10, 11, 12, 13, 14, 15] {
        create_and_open_disi(dense_rank_power, dense_rank_power)?;
    }

    // Illegal values
    for dense_rank_power in [-2i8, 0, 1, 6, 16] {
        expect_throws("java.lang.IllegalArgumentException", || {
            create_and_open_disi(dense_rank_power, 8) // Illegal write, legal read (should not reach read)
        });

        expect_throws("java.lang.IllegalArgumentException", || {
            create_and_open_disi(8, dense_rank_power) // Legal write, illegal read (should reach read)
        });
    }
    Ok(())
}

/// Port of `TestIndexedDISI.createAndOpenDISI`.
fn create_and_open_disi(dense_rank_power_write: i8, dense_rank_power_read: i8) -> Result<()> {
    let mut fixed = FixedBitSet::new(10);
    let len = fixed.length();
    fixed.set(len - 1);
    let set: Arc<dyn BitSet> = Arc::new(fixed);
    let dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let d = dir.as_directory();
        let length;
        let jump_table_entry_count;
        {
            let mut out = d.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
            let written = write_bit_set(
                &mut iterator(&set, set.cardinality() as i64)?,
                &mut *out,
                dense_rank_power_write,
            );
            length = out.file_pointer();
            let closed = out.close();
            jump_table_entry_count = written? as i32;
            closed?;
        }
        let mut r#in = d.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
        let opened = IndexedDISI::new(
            &*r#in,
            0,
            length,
            jump_table_entry_count,
            dense_rank_power_read,
            set.cardinality() as i64,
        );
        let closed = r#in.close();
        opened?;
        closed
        // This tests the legality of the denseRankPower only, so we don't do anything with the disi
    })();
    let closed = dir.close();
    result.and(closed)
}

/// Port of `TestIndexedDISI.testOneDocMissingFixed`.
fn test_one_doc_missing_fixed() -> Result<()> {
    let max_doc = 9699;
    let dense_rank_power = random_dense_rank_power();
    let mut fixed = FixedBitSet::new(max_doc);
    fixed.set_range(0, max_doc);
    fixed.clear(1345);
    let set: Arc<dyn BitSet> = Arc::new(fixed);
    let dir = LuceneTestCase::new_directory();
    {
        let d = dir.as_directory();
        let cardinality = set.cardinality() as i64;
        let length;
        let jump_tableentry_count;
        {
            let mut out = d.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
            jump_tableentry_count = write_bit_set(
                &mut iterator(&set, cardinality)?,
                &mut *out,
                dense_rank_power,
            )? as i32;
            length = out.file_pointer();
            out.close()?;
        }

        let step = 16000;
        let mut r#in = d.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
        {
            let mut disi = IndexedDISI::new(
                &*r#in,
                0,
                length,
                jump_tableentry_count,
                dense_rank_power,
                cardinality,
            )?;
            let mut disi2 = iterator(&set, cardinality)?;
            assert_advance_equality(&mut disi, &mut disi2, step)?;
        }
        r#in.close()?;
    }
    dir.close()
}

/// Port of `TestIndexedDISI.testRandom`.
fn test_random() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    {
        let d = dir.as_directory();
        let num_iters = LuceneTestCase::at_least(3);
        for _ in 0..num_iters {
            do_test_random(&*d)?;
        }
    }
    dir.close()
}

/// Port of `TestIndexedDISI.doTestRandom`.
fn do_test_random(dir: &dyn Directory) -> Result<()> {
    let random = random();
    let max_step = TestUtil::next_int(&*random, 1, 1 << TestUtil::next_int(&*random, 2, 20));
    let num_docs = TestUtil::next_int(&*random, 1, 100000.min((i32::MAX - 1) / max_step));
    let mut docs = sparse(num_docs * max_step + 1)?;
    let mut last_doc = -1;
    {
        let s = bits(&mut docs);
        let mut doc = -1;
        for _ in 0..num_docs {
            doc += TestUtil::next_int(&*random, 1, max_step);
            s.set(doc as usize);
            last_doc = doc;
        }
    }
    let max_doc = last_doc + TestUtil::next_int(&*random, 1, 100);

    let cost = docs.approximate_cardinality() as i64;
    let set: Arc<dyn BitSet> = Arc::from(bit_set_of(&mut iterator(&docs, cost)?, max_doc)?);
    do_test(&set, dir)
}

/// Port of `TestIndexedDISI.doTest`.
fn do_test(set: &Arc<dyn BitSet>, dir: &dyn Directory) -> Result<()> {
    let cardinality = set.cardinality() as i64;
    let dense_rank_power = random_dense_rank_power();
    let length;
    let jump_tableentry_count;
    {
        let mut out = dir.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
        jump_tableentry_count = write_bit_set(
            &mut iterator(set, cardinality)?,
            &mut *out,
            dense_rank_power,
        )? as i32;
        length = out.file_pointer();
        out.close()?;
    }

    let open = |r#in: &dyn rucene::store::IndexInput| {
        IndexedDISI::new(
            r#in,
            0,
            length,
            jump_tableentry_count,
            dense_rank_power,
            cardinality,
        )
    };

    {
        let mut r#in = dir.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
        {
            let mut disi = open(&*r#in)?;
            let mut disi2 = iterator(set, cardinality)?;
            assert_single_step_equality(&mut disi, &mut disi2)?;
        }
        r#in.close()?;
    }

    for step in [1, 10, 100, 1000, 10000, 100000] {
        let mut r#in = dir.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
        {
            let mut disi = open(&*r#in)?;
            let mut disi2 = iterator(set, cardinality)?;
            assert_advance_equality(&mut disi, &mut disi2, step)?;
        }
        r#in.close()?;
    }

    for step in [10, 100, 1000, 10000, 100000] {
        let mut r#in = dir.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
        {
            let mut disi = open(&*r#in)?;
            let mut disi2 = iterator(set, cardinality)?;
            let disi2length = Bits::length(&**set) as i32;
            assert_advance_exact_randomized(&mut disi, &mut disi2, disi2length, step)?;
        }
        r#in.close()?;
    }

    for step in [100, 1000, 10000, 100000] {
        let mut r#in = dir.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
        {
            let mut disi = open(&*r#in)?;
            let mut disi2 = iterator(set, cardinality)?;
            let disi2length = Bits::length(&**set) as i32;
            assert_into_bitset_randomized(&mut disi, &mut disi2, disi2length, step)?;
        }
        r#in.close()?;
    }

    for step in [100, 1000, 10000, 100000] {
        let mut r#in = dir.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
        {
            let mut disi = open(&*r#in)?;
            let mut disi2 = iterator(set, cardinality)?;
            let disi2length = Bits::length(&**set) as i32;
            assert_doc_id_run_end_randomized(&mut disi, &mut disi2, disi2length, step)?;
        }
        r#in.close()?;
    }

    dir.delete_file("foo")
}

/// Port of `TestIndexedDISI.assertAdvanceExactRandomized`.
fn assert_advance_exact_randomized(
    disi: &mut IndexedDISI,
    disi2: &mut BitSetIterator,
    disi2length: i32,
    step: i32,
) -> Result<()> {
    let mut index = -1;
    let random = random();
    let mut target = 0;
    while target < disi2length {
        target += TestUtil::next_int(&*random, 0, step);
        let mut doc = disi2.doc_id();
        while doc < target {
            doc = disi2.next_doc()?;
            index += 1;
        }

        let exists = disi.advance_exact(target)?;
        assert_equals(&(doc == target), &exists);
        if exists {
            assert_equals(&index, &disi.index());
        } else if random.next_boolean() {
            assert_equals(&doc, &disi.next_doc()?);
            // This is a bit strange when doc == NO_MORE_DOCS as the index overcounts in the disi2
            // while-loop
            assert_equals(&index, &disi.index());
            target = doc;
        }
    }
    Ok(())
}

/// Port of `TestIndexedDISI.assertIntoBitsetRandomized`.
fn assert_into_bitset_randomized(
    disi: &mut IndexedDISI,
    disi2: &mut BitSetIterator,
    disi2length: i32,
    step: i32,
) -> Result<()> {
    let mut index = -1;
    let mut set1 = FixedBitSet::new(step as usize);
    let mut set2 = FixedBitSet::new(step as usize);

    let mut up_to = 0;
    while up_to < disi2length {
        let last_up_to = up_to;
        up_to += TestUtil::next_int(&*random(), 0, step);
        let offset = TestUtil::next_int(&*random(), last_up_to, up_to);

        if disi.doc_id() < offset {
            disi.advance(offset)?;
        }
        let mut doc = disi2.doc_id();
        while doc < offset {
            index += 1;
            doc = disi2.next_doc()?;
        }
        while doc < up_to {
            set2.set((doc - offset) as usize);
            index += 1;
            doc = disi2.next_doc()?;
        }

        disi.into_bit_set(up_to, &mut set1, offset)?;
        assert_equals(&index, &disi.index());
        assert_equals(&disi2.doc_id(), &disi.doc_id());

        {
            let set1_cardinality = set1.cardinality() as i64;
            let set2_cardinality = set2.cardinality() as i64;
            let mut expected = BitSetIterator::new(Arc::new(set2.clone()), set2_cardinality)?;
            let mut actual = BitSetIterator::new(Arc::new(set1.clone()), set1_cardinality)?;
            let mut expected_doc = expected.next_doc()?;
            while expected_doc != NO_MORE_DOCS {
                let actual_doc = actual.next_doc()?;
                assert_equals(&(expected_doc + offset), &(actual_doc + offset)); // plus offset for better message.
                expected_doc = expected.next_doc()?;
            }
            assert_equals(&NO_MORE_DOCS, &actual.next_doc()?);
        }

        if disi2.doc_id() != NO_MORE_DOCS {
            assert_equals(&disi2.next_doc()?, &disi.next_doc()?);
            index += 1;
            assert_equals(&index, &disi.index());
        }

        set1.clear_all();
        set2.clear_all();
    }
    Ok(())
}

/// Port of `TestIndexedDISI.assertDocIDRunEndRandomized`.
fn assert_doc_id_run_end_randomized(
    disi: &mut IndexedDISI,
    disi2: &mut BitSetIterator,
    disi2length: i32,
    step: i32,
) -> Result<()> {
    let mut target = 0;
    while target < disi2length {
        target += TestUtil::next_int(&*random(), 0, step);
        if disi.doc_id() < target {
            disi.advance(target)?;
            disi2.advance(target)?;
            assert_equals(&disi2.doc_id(), &disi.doc_id());
            let end = disi.doc_id_run_end()?;
            assert_not_equals(&0, &end);
            let mut it = disi.doc_id();
            while it != NO_MORE_DOCS && it + 1 < end {
                assert_equals(&(it + 1), &disi.next_doc()?);
                assert_equals(&(it + 1), &disi2.next_doc()?);
                it += 1;
            }
        }
    }
    Ok(())
}

/// Port of `TestIndexedDISI.assertSingleStepEquality`.
fn assert_single_step_equality(disi: &mut IndexedDISI, disi2: &mut BitSetIterator) -> Result<()> {
    let mut i = 0;
    let mut doc = disi2.next_doc()?;
    while doc != NO_MORE_DOCS {
        assert_equals(&doc, &disi.next_doc()?);
        assert_equals(&i, &disi.index());
        i += 1;
        doc = disi2.next_doc()?;
    }
    assert_equals(&NO_MORE_DOCS, &disi.next_doc()?);
    Ok(())
}

/// Port of `TestIndexedDISI.assertAdvanceEquality`.
fn assert_advance_equality(
    disi: &mut IndexedDISI,
    disi2: &mut BitSetIterator,
    step: i32,
) -> Result<()> {
    let mut index = -1;
    loop {
        let target = disi2.doc_id().wrapping_add(step);
        let mut doc;
        loop {
            doc = disi2.next_doc()?;
            index += 1;
            if doc >= target {
                break;
            }
        }
        assert_equals(&doc, &disi.advance(target)?);
        if doc == NO_MORE_DOCS {
            break;
        }
        assert_equals_msg(
            Some(&format!(
                "Expected equality using step {step} at docID {doc}"
            )),
            &index,
            &disi.index(),
        );
    }
    Ok(())
}
