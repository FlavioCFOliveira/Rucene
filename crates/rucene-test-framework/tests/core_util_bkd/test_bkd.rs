//! Port of `lucene/core/src/test/org/apache/lucene/util/bkd/TestBKD.java`.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::{LuceneError, Result};
use rucene::index::merge::DocMap;
use rucene::index::point_values::{
    IntersectVisitor, MutablePointTree, PointTree, PointValues, Relation, MAX_DIMENSIONS,
    MAX_INDEX_DIMENSIONS, MAX_NUM_BYTES,
};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::store::{
    BufferedChecksumIndexInput, Directory, IOContext, IndexInput, IndexOutput, Lock,
    DEFAULT_IO_CONTEXT,
};
use rucene::util::bkd::{BKDConfig, BKDReader, BKDWriter};
use rucene::util::{BytesRef, NumericUtils};
use rucene_test_framework::jdk::bit_set::JavaBitSet;
use rucene_test_framework::mockfile::extras_fs::ExtrasFS;
use rucene_test_framework::store::CorruptingIndexOutput;
use rucene_test_framework::util::java_exceptions::{expect_throws, is_instance, java_throwable};
use rucene_test_framework::util::lucene_test_case::{Nightly, TestDirectory};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::big_integer::BigInteger;
use crate::core_support::{
    arrays_to_string, assert_array_equals, assert_equals, assert_equals_msg, class, random, test,
    verbose,
};
use crate::shared_io::{SharedIndexInput, SharedIndexOutput};

/// `TestBKD`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testBasicInts1D", test_basic_ints_1d),
        test("testRandomIntsNDims", test_random_ints_n_dims),
        test("testBigIntNDims", test_big_int_n_dims),
        test("testWithExceptions", test_with_exceptions),
        test("testRandomBinaryTiny", test_random_binary_tiny),
        test("testRandomBinaryMedium", test_random_binary_medium),
        test("testRandomBinaryBig", test_random_binary_big).annotate(Nightly),
        test("testTooLittleHeap", test_too_little_heap),
        test("testAllEqual", test_all_equal),
        test(
            "testIndexDimEqualDataDimDifferent",
            test_index_dim_equal_data_dim_different,
        ),
        test("testOneDimEqual", test_one_dim_equal),
        test("testOneDimLowCard", test_one_dim_low_card),
        test("testOneDimTwoValues", test_one_dim_two_values),
        test(
            "testRandomFewDifferentValues",
            test_random_few_different_values,
        ),
        test("testMultiValued", test_multi_valued),
        test("testBitFlippedOnPartition1", test_bit_flipped_on_partition1),
        test("testBitFlippedOnPartition2", test_bit_flipped_on_partition2),
        test("testTieBreakOrder", test_tie_break_order),
        test(
            "testCheckDataDimOptimalOrder",
            test_check_data_dim_optimal_order,
        ),
        test("test2DLongOrdsOffline", test_2d_long_ords_offline),
        test("testWastedLeadingBytes", test_wasted_leading_bytes),
        test("testEstimatePointCount", test_estimate_point_count),
        test(
            "testTotalPointCountValidation",
            test_total_point_count_validation,
        ),
        test("testTooManyPoints", test_too_many_points),
        test("testTooManyPoints1D", test_too_many_points_1d),
    ];
    class("org.apache.lucene.util.bkd.TestBKD", methods)
}

/// `BKDWriter.DEFAULT_MAX_MB_SORT_IN_HEAP`.
const DEFAULT_MAX_MB_SORT_IN_HEAP: f64 = 16.0;

/// `getPointValues(in)`: `new BKDReader(in, in, in)`, the three roles on the
/// one input.
fn get_point_values(input: &SharedIndexInput) -> Result<BKDReader> {
    BKDReader::new(
        &mut input.handle(),
        &mut input.handle(),
        &mut input.handle(),
    )
}

/// `IORunnable finalizer = w.finish(out, out, out); long fp =
/// out.getFilePointer(); finalizer.run();`: the file pointer between the
/// data and the index.
fn finish(w: &mut BKDWriter, out: &SharedIndexOutput) -> Result<i64> {
    let finalizer = w.finish(&mut out.handle())?;
    let fp = out.get_file_pointer();
    finalizer
        .expect("the writer holds points")
        .run(&mut out.handle(), &mut out.handle())?;
    Ok(fp)
}

/// `dir.createOutput(name, IOContext.DEFAULT)`, as one stream for every role.
fn create_output(dir: &Arc<dyn Directory>, name: &str) -> Result<SharedIndexOutput> {
    Ok(SharedIndexOutput::new(
        dir.create_output(name, &*DEFAULT_IO_CONTEXT)?,
    ))
}

/// `dir.openInput(name, IOContext.DEFAULT)`, as one stream for every role.
fn open_input(dir: &Arc<dyn Directory>, name: &str) -> Result<SharedIndexInput> {
    Ok(SharedIndexInput::new(
        dir.open_input(name, &*DEFAULT_IO_CONTEXT)?,
    ))
}

/// `new byte[n]`.
fn bytes(n: i32) -> Vec<u8> {
    vec![0u8; n as usize]
}

/// `try (Directory dir = getDirectory(numPoints)) { body }`.
fn with_directory(
    num_points: i32,
    body: impl FnOnce(&Arc<dyn Directory>) -> Result<()>,
) -> Result<()> {
    let mut dir = get_directory(num_points);
    let result = body(&dir.as_directory());
    let closed = dir.close();
    result.and(closed)
}

fn test_basic_ints_1d() -> Result<()> {
    let config = BKDConfig::new(1, 1, 4, 2)?;
    with_directory(100, |dir| {
        let mut w = BKDWriter::new_default(100, Arc::clone(dir), "tmp", config.clone(), 1.0, 100)?;
        let mut scratch = [0u8; 4];
        for doc_id in 0..100 {
            NumericUtils::int_to_sortable_bytes(doc_id, &mut scratch, 0);
            w.add(&scratch, doc_id)?;
        }

        let index_fp;
        {
            let out = create_output(dir, "bkd")?;
            index_fp = finish(&mut w, &out)?;
            out.close_stream()?;
        }

        {
            let input = open_input(dir, "bkd")?;
            input.handle().seek(index_fp)?;
            let r = get_point_values(&input)?;

            // Simple 1D range query:
            let mut query_min = vec![vec![0u8; 4]];
            NumericUtils::int_to_sortable_bytes(42, &mut query_min[0], 0);
            let mut query_max = vec![vec![0u8; 4]];
            NumericUtils::int_to_sortable_bytes(87, &mut query_max[0], 0);

            let hits = Mutex::new(JavaBitSet::new());
            r.intersect(&mut get_intersect_visitor(
                &hits, &query_min, &query_max, &config,
            ))?;
            let hits = hits.into_inner().unwrap();

            for doc_id in 0..100 {
                let expected = (42..=87).contains(&doc_id);
                let actual = hits.get(doc_id as usize);
                assert_equals_msg(Some(&format!("docID={doc_id}")), &expected, &actual);
            }
            input.close_stream()?;
        }
        Ok(())
    })
}

fn test_random_ints_n_dims() -> Result<()> {
    let num_docs = LuceneTestCase::at_least(1000);
    with_directory(num_docs, |dir| {
        let num_dims = TestUtil::next_int(&*random(), 1, 5);
        let num_index_dims = TestUtil::next_int(&*random(), 1, num_dims);
        let max_points_in_leaf_node = TestUtil::next_int(&*random(), 50, 100);
        let max_mb = 3.0f32 + (3.0 * random().next_float());
        let config = BKDConfig::new(num_dims, num_index_dims, 4, max_points_in_leaf_node)?;
        let mut w = BKDWriter::new_default(
            num_docs,
            Arc::clone(dir),
            "tmp",
            config.clone(),
            max_mb as f64,
            num_docs as i64,
        )?;

        if verbose() {
            println!("TEST: numDims={num_dims} numIndexDims={num_index_dims} numDocs={num_docs}");
        }
        let mut docs: Vec<Vec<i32>> = Vec::with_capacity(num_docs as usize);
        let mut scratch = bytes(4 * num_dims);
        let mut min_value = vec![i32::MAX; num_dims as usize];
        let mut max_value = vec![i32::MIN; num_dims as usize];
        for doc_id in 0..num_docs {
            let mut values = vec![0i32; num_dims as usize];
            if verbose() {
                println!("  docID={doc_id}");
            }
            for dim in 0..num_dims as usize {
                values[dim] = random().next_int();
                if values[dim] < min_value[dim] {
                    min_value[dim] = values[dim];
                }
                if values[dim] > max_value[dim] {
                    max_value[dim] = values[dim];
                }
                NumericUtils::int_to_sortable_bytes(values[dim], &mut scratch, dim * 4);
                if verbose() {
                    println!("    {dim} -> {}", values[dim]);
                }
            }
            docs.push(values);
            w.add(&scratch, doc_id)?;
        }

        let index_fp;
        {
            let out = create_output(dir, "bkd")?;
            index_fp = finish(&mut w, &out)?;
            out.close_stream()?;
        }

        {
            let input = open_input(dir, "bkd")?;
            input.handle().seek(index_fp)?;
            let r = get_point_values(&input)?;

            let min_packed_value = PointValues::min_packed_value(&r)?.expect("points were indexed");
            let max_packed_value = PointValues::max_packed_value(&r)?.expect("points were indexed");
            for dim in 0..num_index_dims as usize {
                assert_equals(
                    &min_value[dim],
                    &NumericUtils::sortable_bytes_to_int(&min_packed_value, dim * 4),
                );
                assert_equals(
                    &max_value[dim],
                    &NumericUtils::sortable_bytes_to_int(&max_packed_value, dim * 4),
                );
            }

            let iters = LuceneTestCase::at_least(100);
            for iter in 0..iters {
                if verbose() {
                    println!("\nTEST: iter={iter}");
                }

                // Random N dims rect query:
                let mut query_min = vec![0i32; num_dims as usize];
                let mut query_min_bytes = vec![vec![0u8; 4]; num_dims as usize];
                let mut query_max = vec![0i32; num_dims as usize];
                let mut query_max_bytes = vec![vec![0u8; 4]; num_dims as usize];
                for dim in 0..num_index_dims as usize {
                    query_min[dim] = random().next_int();
                    query_max[dim] = random().next_int();
                    if query_min[dim] > query_max[dim] {
                        std::mem::swap(&mut query_min[dim], &mut query_max[dim]);
                    }
                    NumericUtils::int_to_sortable_bytes(
                        query_min[dim],
                        &mut query_min_bytes[dim],
                        0,
                    );
                    NumericUtils::int_to_sortable_bytes(
                        query_max[dim],
                        &mut query_max_bytes[dim],
                        0,
                    );
                }

                let hits = Mutex::new(JavaBitSet::new());
                r.intersect(&mut get_intersect_visitor(
                    &hits,
                    &query_min_bytes,
                    &query_max_bytes,
                    &config,
                ))?;
                let hits = hits.into_inner().unwrap();

                for doc_id in 0..num_docs as usize {
                    let doc_values = &docs[doc_id];
                    let mut expected = true;
                    for dim in 0..num_index_dims as usize {
                        let x = doc_values[dim];
                        if x < query_min[dim] || x > query_max[dim] {
                            expected = false;
                            break;
                        }
                    }
                    let actual = hits.get(doc_id);
                    assert_equals_msg(Some(&format!("docID={doc_id}")), &expected, &actual);
                }
            }
            input.close_stream()?;
        }
        Ok(())
    })
}

/// `testBigIntNDims()`: tests on N-dimensional points where each dimension
/// is a `BigInteger`.
fn test_big_int_n_dims() -> Result<()> {
    let num_docs = LuceneTestCase::at_least(1000);
    with_directory(num_docs, |dir| {
        let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, 30);
        let num_dims = TestUtil::next_int(&*random(), 1, 5);
        let max_points_in_leaf_node = TestUtil::next_int(&*random(), 50, 100);
        let max_mb = 3.0f32 + (3.0 * random().next_float());
        let config = BKDConfig::new(
            num_dims,
            num_dims,
            num_bytes_per_dim,
            max_points_in_leaf_node,
        )?;
        let mut w = BKDWriter::new_default(
            num_docs,
            Arc::clone(dir),
            "tmp",
            config.clone(),
            max_mb as f64,
            num_docs as i64,
        )?;
        let nbpd = num_bytes_per_dim as usize;
        let mut docs: Vec<Vec<BigInteger>> = Vec::with_capacity(num_docs as usize);
        let mut scratch = bytes(num_bytes_per_dim * num_dims);
        for doc_id in 0..num_docs {
            let mut values = Vec::with_capacity(num_dims as usize);
            if verbose() {
                println!("  docID={doc_id}");
            }
            for dim in 0..num_dims as usize {
                values.push(random_big_int(num_bytes_per_dim));
                NumericUtils::big_int_to_sortable_bytes(
                    &values[dim].to_byte_array(),
                    nbpd,
                    &mut scratch,
                    dim * nbpd,
                );
                if verbose() {
                    println!("    {dim} -> {}", values[dim]);
                }
            }
            docs.push(values);
            w.add(&scratch, doc_id)?;
        }

        let index_fp;
        {
            let out = create_output(dir, "bkd")?;
            index_fp = finish(&mut w, &out)?;
            out.close_stream()?;
        }

        {
            let input = open_input(dir, "bkd")?;
            input.handle().seek(index_fp)?;
            let point_values = get_point_values(&input)?;

            let iters = LuceneTestCase::at_least(100);
            for iter in 0..iters {
                if verbose() {
                    println!("\nTEST: iter={iter}");
                }

                // Random N dims rect query:
                let mut query_min: Vec<BigInteger> = Vec::new();
                let mut query_min_bytes = vec![vec![0u8; nbpd]; num_dims as usize];
                let mut query_max: Vec<BigInteger> = Vec::new();
                let mut query_max_bytes = vec![vec![0u8; nbpd]; num_dims as usize];
                for dim in 0..num_dims as usize {
                    query_min.push(random_big_int(num_bytes_per_dim));
                    query_max.push(random_big_int(num_bytes_per_dim));
                    if query_min[dim] > query_max[dim] {
                        let x = query_min[dim].clone();
                        query_min[dim] = query_max[dim].clone();
                        query_max[dim] = x;
                    }
                    NumericUtils::big_int_to_sortable_bytes(
                        &query_min[dim].to_byte_array(),
                        nbpd,
                        &mut query_min_bytes[dim],
                        0,
                    );
                    NumericUtils::big_int_to_sortable_bytes(
                        &query_max[dim].to_byte_array(),
                        nbpd,
                        &mut query_max_bytes[dim],
                        0,
                    );
                }

                let hits = Mutex::new(JavaBitSet::new());
                point_values.intersect(&mut get_intersect_visitor(
                    &hits,
                    &query_min_bytes,
                    &query_max_bytes,
                    &config,
                ))?;
                let hits = hits.into_inner().unwrap();

                for doc_id in 0..num_docs as usize {
                    let doc_values = &docs[doc_id];
                    let mut expected = true;
                    for dim in 0..num_dims as usize {
                        let x = &doc_values[dim];
                        if *x < query_min[dim] || *x > query_max[dim] {
                            expected = false;
                            break;
                        }
                    }
                    let actual = hits.get(doc_id);
                    assert_equals_msg(Some(&format!("docID={doc_id}")), &expected, &actual);
                }
            }
            input.close_stream()?;
        }
        Ok(())
    })
}

/// `new byte[numDocs][numDataDims][numBytesPerDim]`, each value drawn with
/// `random().nextBytes`.
fn random_doc_values(
    num_docs: i32,
    num_data_dims: i32,
    num_bytes_per_dim: i32,
) -> Vec<Vec<Vec<u8>>> {
    let mut doc_values = Vec::with_capacity(num_docs as usize);
    for _ in 0..num_docs {
        let mut values = Vec::with_capacity(num_data_dims as usize);
        for _ in 0..num_data_dims {
            let mut v = bytes(num_bytes_per_dim);
            random().next_bytes(&mut v);
            values.push(v);
        }
        doc_values.push(values);
    }
    doc_values
}

/// `testWithExceptions()`: make sure we close open files, delete temp
/// files, etc., on exception.
fn test_with_exceptions() -> Result<()> {
    let num_docs = LuceneTestCase::at_least(10000);
    let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, 30);
    let num_data_dims = TestUtil::next_int(&*random(), 1, MAX_DIMENSIONS);
    let num_index_dims = TestUtil::next_int(&*random(), 1, num_data_dims).min(MAX_INDEX_DIMENSIONS);

    let doc_values = random_doc_values(num_docs, num_data_dims, num_bytes_per_dim);

    let mut max_mb_heap = 0.05;
    // Keep retrying until we 1) we allow a big enough heap, and 2) we hit a random IOExc from MDW:
    let mut done = false;
    while !done {
        let mut dir = LuceneTestCase::new_mock_fs_directory(&LuceneTestCase::create_temp_dir());
        dir.set_random_io_exception_rate(0.05);
        dir.set_random_io_exception_rate_on_open(0.05);
        let result = verify_in(
            &dir.as_directory(),
            &doc_values,
            None,
            num_data_dims,
            num_index_dims,
            num_bytes_per_dim,
            50,
            max_mb_heap,
        );
        if let Err(e) = result {
            if is_instance(&e, "java.lang.IllegalArgumentException") {
                // This just means we got a too-small maxMB for the maxPointsInLeafNode; just retry w/ more
                // heap
                Assert::assert_true(
                    java_throwable(&e).message().unwrap_or("").contains(
                        "either increase maxMBSortInHeap or decrease maxPointsInLeafNode",
                    ),
                );
                max_mb_heap *= 1.25;
            } else if is_instance(&e, "java.io.IOException") {
                if java_throwable(&e)
                    .message()
                    .unwrap_or("")
                    .contains("a random IOException")
                {
                    // BKDWriter should fully clean up after itself:
                    done = true;
                } else {
                    return Err(e);
                }
            } else {
                return Err(e);
            }
        }

        let files: Vec<String> = dir
            .list_all()?
            .into_iter()
            .filter(|file| !ExtrasFS::is_extra(file))
            .collect();
        Assert::assert_true_msg(
            &format!("files={}", arrays_to_string(&files)),
            files.is_empty(),
        );
        dir.close()?;
    }
    Ok(())
}

fn test_random_binary_tiny() -> Result<()> {
    do_test_random_binary(10)
}

fn test_random_binary_medium() -> Result<()> {
    do_test_random_binary(10000)
}

fn test_random_binary_big() -> Result<()> {
    do_test_random_binary(200000)
}

fn test_too_little_heap() -> Result<()> {
    with_directory(0, |dir| {
        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            BKDWriter::new_default(
                1,
                Arc::clone(dir),
                "bkd",
                BKDConfig::new(1, 1, 16, 1000000)?,
                0.001,
                0,
            )
        });
        Assert::assert_true(
            expected
                .message()
                .unwrap_or("")
                .contains("either increase maxMBSortInHeap or decrease maxPointsInLeafNode"),
        );
        Ok(())
    })
}

/// `doTestRandomBinary(count)`.
fn do_test_random_binary(count: i32) -> Result<()> {
    let num_docs = TestUtil::next_int(&*random(), count, count * 2);
    let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, 30);
    let num_data_dims = TestUtil::next_int(&*random(), 1, MAX_DIMENSIONS);
    let num_index_dims = TestUtil::next_int(&*random(), 1, num_data_dims).min(MAX_INDEX_DIMENSIONS);

    let doc_values = random_doc_values(num_docs, num_data_dims, num_bytes_per_dim);

    verify(
        &doc_values,
        None,
        num_data_dims,
        num_index_dims,
        num_bytes_per_dim,
    )
}

fn test_all_equal() -> Result<()> {
    let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, 30);
    let num_data_dims = TestUtil::next_int(&*random(), 1, MAX_DIMENSIONS);
    let num_index_dims = TestUtil::next_int(&*random(), 1, num_data_dims).min(MAX_INDEX_DIMENSIONS);

    let num_docs = LuceneTestCase::at_least(1000);
    let mut doc_values: Vec<Vec<Vec<u8>>> = Vec::with_capacity(num_docs as usize);
    for doc_id in 0..num_docs {
        if doc_id == 0 {
            let mut values = Vec::with_capacity(num_data_dims as usize);
            for _ in 0..num_data_dims {
                let mut v = bytes(num_bytes_per_dim);
                random().next_bytes(&mut v);
                values.push(v);
            }
            doc_values.push(values);
        } else {
            doc_values.push(doc_values[0].clone());
        }
    }

    verify(
        &doc_values,
        None,
        num_data_dims,
        num_index_dims,
        num_bytes_per_dim,
    )
}

fn test_index_dim_equal_data_dim_different() -> Result<()> {
    let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, 30);
    let num_data_dims = TestUtil::next_int(&*random(), 2, MAX_DIMENSIONS);
    let num_index_dims =
        TestUtil::next_int(&*random(), 1, num_data_dims - 1).min(MAX_INDEX_DIMENSIONS);

    let num_docs = LuceneTestCase::at_least(1000);
    let mut index_dimensions: Vec<Vec<u8>> = Vec::new();
    for _ in 0..num_index_dims {
        let mut v = bytes(num_bytes_per_dim);
        random().next_bytes(&mut v);
        index_dimensions.push(v);
    }

    let mut doc_values = Vec::with_capacity(num_docs as usize);
    for _ in 0..num_docs {
        let mut values: Vec<Vec<u8>> = Vec::with_capacity(num_data_dims as usize);
        for dim in 0..num_index_dims as usize {
            values.push(index_dimensions[dim].clone());
        }
        for _ in num_index_dims..num_data_dims {
            let mut v = bytes(num_bytes_per_dim);
            random().next_bytes(&mut v);
            values.push(v);
        }
        doc_values.push(values);
    }

    verify(
        &doc_values,
        None,
        num_data_dims,
        num_index_dims,
        num_bytes_per_dim,
    )
}

fn test_one_dim_equal() -> Result<()> {
    let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, 30);
    let num_data_dims = TestUtil::next_int(&*random(), 1, MAX_DIMENSIONS);
    let num_index_dims = TestUtil::next_int(&*random(), 1, num_data_dims).min(MAX_INDEX_DIMENSIONS);

    let num_docs = LuceneTestCase::at_least(1000);
    let the_equal_dim = random().next_int_bound(num_data_dims) as usize;
    let mut doc_values: Vec<Vec<Vec<u8>>> = Vec::with_capacity(num_docs as usize);
    for doc_id in 0..num_docs {
        let mut values = Vec::with_capacity(num_data_dims as usize);
        for _ in 0..num_data_dims {
            let mut v = bytes(num_bytes_per_dim);
            random().next_bytes(&mut v);
            values.push(v);
        }
        doc_values.push(values);
        if doc_id > 0 {
            let first = doc_values[0][the_equal_dim].clone();
            doc_values[doc_id as usize][the_equal_dim] = first;
        }
    }

    // Use a small number of points in leaf blocks to trigger a lot of splitting
    let max_points_in_leaf_node = TestUtil::next_int(&*random(), 20, 50);
    verify_leaf(
        &doc_values,
        None,
        num_data_dims,
        num_index_dims,
        num_bytes_per_dim,
        max_points_in_leaf_node,
    )
}

/// `testOneDimLowCard()`: this triggers the logic that makes sure all
/// dimensions get indexed by looking at how many times each dim has been
/// split.
fn test_one_dim_low_card() -> Result<()> {
    let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, 30);
    let num_data_dims = TestUtil::next_int(&*random(), 2, MAX_DIMENSIONS);
    let num_index_dims = TestUtil::next_int(&*random(), 2, num_data_dims).min(MAX_INDEX_DIMENSIONS);

    let num_docs = LuceneTestCase::at_least(10000);
    let the_low_card_dim = random().next_int_bound(num_data_dims);

    let mut value1 = bytes(num_bytes_per_dim);
    random().next_bytes(&mut value1);
    let mut value2 = value1.clone();
    let last = num_bytes_per_dim as usize - 1;
    if value2[last] == 0 || random().next_boolean() {
        value2[last] = value2[last].wrapping_add(1);
    } else {
        value2[last] = value2[last].wrapping_sub(1);
    }

    let mut doc_values = Vec::with_capacity(num_docs as usize);
    for _ in 0..num_docs {
        let mut values = Vec::with_capacity(num_data_dims as usize);
        for dim in 0..num_data_dims {
            if dim == the_low_card_dim {
                values.push(if random().next_boolean() {
                    value1.clone()
                } else {
                    value2.clone()
                });
            } else {
                let mut v = bytes(num_bytes_per_dim);
                random().next_bytes(&mut v);
                values.push(v);
            }
        }
        doc_values.push(values);
    }

    // Use a small number of points in leaf blocks to trigger a lot of splitting
    let max_points_in_leaf_node = TestUtil::next_int(&*random(), 20, 50);
    verify_leaf(
        &doc_values,
        None,
        num_data_dims,
        num_index_dims,
        num_bytes_per_dim,
        max_points_in_leaf_node,
    )
}

/// `testOneDimTwoValues()`: this should trigger run-length compression with
/// lengths that are greater than 255.
fn test_one_dim_two_values() -> Result<()> {
    let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, 30);
    let num_data_dims = TestUtil::next_int(&*random(), 1, MAX_DIMENSIONS);
    let num_index_dims = TestUtil::next_int(&*random(), 1, num_data_dims).min(MAX_INDEX_DIMENSIONS);

    let num_docs = LuceneTestCase::at_least(1000);
    let the_dim = random().next_int_bound(num_data_dims);
    let mut value1 = bytes(num_bytes_per_dim);
    random().next_bytes(&mut value1);
    let mut value2 = bytes(num_bytes_per_dim);
    random().next_bytes(&mut value2);
    let mut doc_values = Vec::with_capacity(num_docs as usize);

    for _ in 0..num_docs {
        let mut values = Vec::with_capacity(num_data_dims as usize);
        for dim in 0..num_data_dims {
            if dim == the_dim {
                values.push(if random().next_boolean() {
                    value1.clone()
                } else {
                    value2.clone()
                });
            } else {
                let mut v = bytes(num_bytes_per_dim);
                random().next_bytes(&mut v);
                values.push(v);
            }
        }
        doc_values.push(values);
    }

    verify(
        &doc_values,
        None,
        num_data_dims,
        num_index_dims,
        num_bytes_per_dim,
    )
}

/// `testRandomFewDifferentValues()`: this should trigger low cardinality
/// leaves.
fn test_random_few_different_values() -> Result<()> {
    let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, 30);
    let num_data_dims = TestUtil::next_int(&*random(), 1, MAX_DIMENSIONS);
    let num_index_dims = TestUtil::next_int(&*random(), 1, num_data_dims).min(MAX_INDEX_DIMENSIONS);

    let num_docs = LuceneTestCase::at_least(10000);
    let cardinality = TestUtil::next_int(&*random(), 2, 100);
    let mut values = vec![
        vec![vec![0u8; num_bytes_per_dim as usize]; num_data_dims as usize];
        cardinality as usize
    ];
    for v in values.iter_mut() {
        for d in v.iter_mut() {
            random().next_bytes(d);
        }
    }

    let mut doc_values = Vec::with_capacity(num_docs as usize);
    for _ in 0..num_docs {
        doc_values.push(values[random().next_int_bound(cardinality) as usize].clone());
    }

    verify(
        &doc_values,
        None,
        num_data_dims,
        num_index_dims,
        num_bytes_per_dim,
    )
}

fn test_multi_valued() -> Result<()> {
    let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, 30);
    let num_data_dims = TestUtil::next_int(&*random(), 1, MAX_DIMENSIONS);
    let num_index_dims = TestUtil::next_int(&*random(), 1, num_data_dims).min(MAX_INDEX_DIMENSIONS);

    let num_docs = LuceneTestCase::at_least(1000);
    let mut doc_values: Vec<Vec<Vec<u8>>> = Vec::new();
    let mut doc_ids: Vec<i32> = Vec::new();

    for doc_id in 0..num_docs {
        let num_values_in_doc = TestUtil::next_int(&*random(), 1, 5);
        for _ in 0..num_values_in_doc {
            doc_ids.push(doc_id);
            let mut values = Vec::with_capacity(num_data_dims as usize);
            for _ in 0..num_data_dims {
                let mut v = bytes(num_bytes_per_dim);
                random().next_bytes(&mut v);
                values.push(v);
            }
            doc_values.push(values);
        }
    }

    verify(
        &doc_values,
        Some(&doc_ids),
        num_data_dims,
        num_index_dims,
        num_bytes_per_dim,
    )
}

/// `verify(docValues, docIDs, numDataDims, numIndexDims, numBytesPerDim)`:
/// `docIDs` can be null, for the single valued case, else it maps value to
/// docID.
fn verify(
    doc_values: &[Vec<Vec<u8>>],
    doc_ids: Option<&[i32]>,
    num_data_dims: i32,
    num_index_dims: i32,
    num_bytes_per_dim: i32,
) -> Result<()> {
    let max_points_in_leaf_node = TestUtil::next_int(&*random(), 50, 1000);
    verify_leaf(
        doc_values,
        doc_ids,
        num_data_dims,
        num_index_dims,
        num_bytes_per_dim,
        max_points_in_leaf_node,
    )
}

/// `verify(docValues, docIDs, numDataDims, numIndexDims, numBytesPerDim,
/// maxPointsInLeafNode)`.
fn verify_leaf(
    doc_values: &[Vec<Vec<u8>>],
    doc_ids: Option<&[i32]>,
    num_data_dims: i32,
    num_index_dims: i32,
    num_bytes_per_dim: i32,
    max_points_in_leaf_node: i32,
) -> Result<()> {
    with_directory(doc_values.len() as i32, |dir| {
        let max_mb = (3.0f32 as f64) + (3.0 * random().next_double());
        verify_in(
            dir,
            doc_values,
            doc_ids,
            num_data_dims,
            num_index_dims,
            num_bytes_per_dim,
            max_points_in_leaf_node,
            max_mb,
        )
    })
}

/// `verify(dir, docValues, docIDs, numDataDims, numIndexDims,
/// numBytesPerDim, maxPointsInLeafNode, maxMB)`.
#[allow(clippy::too_many_arguments)]
fn verify_in(
    dir: &Arc<dyn Directory>,
    doc_values: &[Vec<Vec<u8>>],
    doc_ids: Option<&[i32]>,
    num_data_dims: i32,
    num_index_dims: i32,
    num_bytes_per_dim: i32,
    mut max_points_in_leaf_node: i32,
    mut max_mb: f64,
) -> Result<()> {
    let num_values = doc_values.len() as i32;
    if verbose() {
        println!(
            "TEST: numValues={num_values} numDataDims={num_data_dims} numIndexDims={num_index_dims} numBytesPerDim={num_bytes_per_dim} maxPointsInLeafNode={max_points_in_leaf_node} maxMB={max_mb}"
        );
    }

    let mut to_merge: Option<Vec<i64>> = None;
    let mut doc_maps: Option<Vec<DocMap>> = None;
    let mut seg = 0;
    // we force sometimes to provide a bigger  point count
    let mut max_docs = i64::MIN;
    if random().next_boolean() {
        max_docs = doc_values.len() as i64;
    } else {
        while max_docs < doc_values.len() as i64 {
            max_docs = random().next_long();
        }
    }
    let mut w = BKDWriter::new_default(
        num_values,
        Arc::clone(dir),
        &format!("_{seg}"),
        BKDConfig::new(
            num_data_dims,
            num_index_dims,
            num_bytes_per_dim,
            max_points_in_leaf_node,
        )?,
        max_mb,
        max_docs,
    )?;
    let mut out = Some(create_output(dir, "bkd")?);
    let mut input: Option<SharedIndexInput> = None;

    let result = (|| -> Result<()> {
        let nbpd = num_bytes_per_dim as usize;
        let mut scratch = bytes(num_bytes_per_dim * num_data_dims);
        let mut last_doc_id_base = 0;
        let use_merge = num_data_dims == 1 && num_values >= 10 && random().next_boolean();
        let mut values_in_this_seg = if use_merge {
            // Sometimes we will call merge with a single segment:
            TestUtil::next_int(&*random(), num_values / 10, num_values)
        } else {
            0
        };
        let mut seg_count = 0;

        for ord in 0..num_values as usize {
            let doc_id = match doc_ids {
                None => ord as i32,
                Some(ids) => ids[ord],
            };
            if verbose() {
                println!("  ord={ord} docID={doc_id} lastDocIDBase={last_doc_id_base}");
            }
            for dim in 0..num_data_dims as usize {
                if verbose() {
                    println!(
                        "    {dim} -> {}",
                        BytesRef::new(doc_values[ord][dim].clone())
                    );
                }
                scratch[dim * nbpd..(dim + 1) * nbpd]
                    .copy_from_slice(&doc_values[ord][dim][..nbpd]);
            }
            w.add(&scratch, doc_id - last_doc_id_base)?;
            seg_count += 1;

            if use_merge && seg_count == values_in_this_seg {
                if to_merge.is_none() {
                    to_merge = Some(Vec::new());
                    doc_maps = Some(Vec::new());
                }
                let cur_doc_id_base = last_doc_id_base;
                doc_maps
                    .as_mut()
                    .unwrap()
                    .push(Arc::new(move |doc_id1| cur_doc_id_base + doc_id1));
                let o = out.as_ref().unwrap();
                let fp = finish(&mut w, o)?;
                let _ = fp;
                // `toMerge.add(out.getFilePointer())` is read between
                // `finish` and `finalizer.run()`, which `finish` returns.
                to_merge.as_mut().unwrap().push(fp);
                values_in_this_seg =
                    TestUtil::next_int(&*random(), num_values / 10, num_values / 2);
                seg_count = 0;

                seg += 1;
                max_points_in_leaf_node = TestUtil::next_int(&*random(), 50, 1000);
                max_mb = (3.0f32 as f64) + (3.0 * random().next_double());
                w = BKDWriter::new_default(
                    num_values,
                    Arc::clone(dir),
                    &format!("_{seg}"),
                    BKDConfig::new(
                        num_data_dims,
                        num_index_dims,
                        num_bytes_per_dim,
                        max_points_in_leaf_node,
                    )?,
                    max_mb,
                    doc_values.len() as i64,
                )?;
                last_doc_id_base = doc_id;
            }
        }

        let index_fp;

        if let Some(to_merge_fps) = to_merge.as_mut() {
            if seg_count > 0 {
                let fp = finish(&mut w, out.as_ref().unwrap())?;
                to_merge_fps.push(fp);
                let cur_doc_id_base = last_doc_id_base;
                doc_maps
                    .as_mut()
                    .unwrap()
                    .push(Arc::new(move |doc_id| cur_doc_id_base + doc_id));
            }
            out.take().unwrap().close_stream()?;
            input = Some(open_input(dir, "bkd")?);
            seg += 1;
            w = BKDWriter::new_default(
                num_values,
                Arc::clone(dir),
                &format!("_{seg}"),
                BKDConfig::new(
                    num_data_dims,
                    num_index_dims,
                    num_bytes_per_dim,
                    max_points_in_leaf_node,
                )?,
                max_mb,
                doc_values.len() as i64,
            )?;
            let mut readers: Vec<BKDReader> = Vec::new();
            let shared = input.as_ref().unwrap();
            for &fp in to_merge_fps.iter() {
                shared.handle().seek(fp)?;
                readers.push(get_point_values(shared)?);
            }
            let reader_refs: Vec<&dyn PointValues> =
                readers.iter().map(|r| r as &dyn PointValues).collect();
            out = Some(create_output(dir, "bkd2")?);
            let o = out.as_ref().unwrap();
            let finalizer = w.merge(&mut o.handle(), doc_maps.as_deref(), &reader_refs)?;
            index_fp = o.get_file_pointer();
            finalizer
                .expect("the merged segments hold points")
                .run(&mut o.handle(), &mut o.handle())?;
            out.take().unwrap().close_stream()?;
            input.take().unwrap().close_stream()?;
            input = Some(open_input(dir, "bkd2")?);
        } else {
            index_fp = finish(&mut w, out.as_ref().unwrap())?;
            out.take().unwrap().close_stream()?;
            input = Some(open_input(dir, "bkd")?);
        }

        let shared = input.as_ref().unwrap();
        shared.handle().seek(index_fp)?;
        let point_values = get_point_values(shared)?;

        assert_size(point_values.point_tree()?)?;

        let iters = LuceneTestCase::at_least(100);
        for iter in 0..iters {
            if verbose() {
                println!("\nTEST: iter={iter}");
            }

            // Random N dims rect query:
            let mut query_min: Vec<Vec<u8>> = Vec::with_capacity(num_data_dims as usize);
            let mut query_max: Vec<Vec<u8>> = Vec::with_capacity(num_data_dims as usize);
            for _ in 0..num_data_dims {
                let mut min = bytes(num_bytes_per_dim);
                random().next_bytes(&mut min);
                let mut max = bytes(num_bytes_per_dim);
                random().next_bytes(&mut max);
                if min[..nbpd].cmp(&max[..nbpd]).is_gt() {
                    std::mem::swap(&mut min, &mut max);
                }
                query_min.push(min);
                query_max.push(max);
            }

            let mut expected = JavaBitSet::new();
            for ord in 0..num_values as usize {
                let mut matches = true;
                for dim in 0..num_index_dims as usize {
                    let x = &doc_values[ord][dim];
                    if x[..nbpd].cmp(&query_min[dim][..nbpd]).is_lt()
                        || x[..nbpd].cmp(&query_max[dim][..nbpd]).is_gt()
                    {
                        matches = false;
                        break;
                    }
                }

                if matches {
                    let doc_id = match doc_ids {
                        None => ord as i32,
                        Some(ids) => ids[ord],
                    };
                    expected.set(doc_id as usize);
                }
            }

            let config = BKDConfig::new(
                num_data_dims,
                num_index_dims,
                num_bytes_per_dim,
                max_points_in_leaf_node,
            )?;
            let hits = Mutex::new(JavaBitSet::new());
            point_values.intersect(&mut get_intersect_visitor(
                &hits, &query_min, &query_max, &config,
            ))?;
            assert_hits(&hits.lock().unwrap(), &expected);

            hits.lock().unwrap().clear_all();
            point_values
                .point_tree()?
                .visit_doc_values(&mut get_intersect_visitor(
                    &hits, &query_min, &query_max, &config,
                ))?;
            assert_hits(&hits.lock().unwrap(), &expected);
        }
        input.take().unwrap().close_stream()?;
        dir.delete_file("bkd")?;
        if to_merge.is_some() {
            dir.delete_file("bkd2")?;
        }
        Ok(())
    })();
    if result.is_err() {
        // IOUtils.closeWhileHandlingException(w, in, out);
        let _ = w.close();
        if let Some(i) = input.take() {
            let _ = i.close_stream();
        }
        if let Some(o) = out.take() {
            let _ = o.close_stream();
        }
        // IOUtils.deleteFilesIgnoringExceptions(dir, "bkd", "bkd2");
        let _ = dir.delete_file("bkd");
        let _ = dir.delete_file("bkd2");
    }
    result
}

/// The anonymous `IntersectVisitor` of `assertSize`: counts the doc IDs and
/// the doc values it visits.
struct SizeVisitor {
    visit_doc_id_size: i64,
    visit_doc_values_size: i64,
}

impl IntersectVisitor for SizeVisitor {
    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        self.visit_doc_id_size += 1;
        Ok(())
    }

    fn visit_with_value(&mut self, _doc_id: i32, _packed_value: &[u8]) -> Result<()> {
        self.visit_doc_values_size += 1;
        Ok(())
    }

    fn compare(&self, _min_packed_value: &[u8], _max_packed_value: &[u8]) -> Result<Relation> {
        Ok(Relation::CellCrossesQuery)
    }
}

/// `assertSize(tree)`.
fn assert_size(tree: Box<dyn PointTree>) -> Result<()> {
    let mut tree = tree;
    assert_size_ref(&mut tree)
}

fn assert_size_ref(tree: &mut Box<dyn PointTree>) -> Result<()> {
    let clone = tree.clone_tree()?;
    assert_equals(&clone.size()?, &tree.size()?);
    // rarely continue with the clone tree
    let mut clone_slot = clone;
    let use_clone = LuceneTestCase::rarely();
    let tree: &mut Box<dyn PointTree> = if use_clone { &mut clone_slot } else { tree };
    let mut visitor = SizeVisitor {
        visit_doc_id_size: 0,
        visit_doc_values_size: 0,
    };
    if random().next_boolean() {
        tree.visit_doc_ids(&mut visitor)?;
        tree.visit_doc_values(&mut visitor)?;
    } else {
        tree.visit_doc_values(&mut visitor)?;
        tree.visit_doc_ids(&mut visitor)?;
    }
    assert_equals(&visitor.visit_doc_id_size, &visitor.visit_doc_values_size);
    assert_equals(&visitor.visit_doc_id_size, &tree.size()?);
    if tree.move_to_child()? {
        loop {
            random_point_tree_navigation(tree.as_mut())?;
            assert_size_ref(tree)?;
            if !tree.move_to_sibling()? {
                break;
            }
        }
        tree.move_to_parent()?;
    }
    Ok(())
}

/// `randomPointTreeNavigation(tree)`.
fn random_point_tree_navigation(tree: &mut dyn PointTree) -> Result<()> {
    let min_packed_value = tree.min_packed_value()?.to_vec();
    let max_packed_value = tree.max_packed_value()?.to_vec();
    let size = tree.size()?;
    if random().next_boolean() && tree.move_to_child()? {
        random_point_tree_navigation(tree)?;
        if random().next_boolean() && tree.move_to_sibling()? {
            random_point_tree_navigation(tree)?;
        }
        tree.move_to_parent()?;
    }
    // we always finish on the same node we started
    assert_array_equals(&min_packed_value, tree.min_packed_value()?);
    assert_array_equals(&max_packed_value, tree.max_packed_value()?);
    assert_equals(&size, &tree.size()?);
    Ok(())
}

/// `BitSet.length()`: the index of the highest set bit plus one.
fn bit_set_length(bits: &JavaBitSet) -> usize {
    if bits.size() == 0 {
        return 0;
    }
    (bits.previous_set_bit(bits.size() as i32 - 1) + 1) as usize
}

/// `assertHits(hits, expected)`.
fn assert_hits(hits: &JavaBitSet, expected: &JavaBitSet) {
    let limit = bit_set_length(expected).max(bit_set_length(hits));
    for doc_id in 0..limit {
        assert_equals_msg(
            Some(&format!("docID={doc_id}")),
            &expected.get(doc_id),
            &hits.get(doc_id),
        );
    }
}

/// The anonymous `IntersectVisitor` of `getIntersectVisitor`.
struct RangeVisitor<'a> {
    hits: &'a Mutex<JavaBitSet>,
    query_min: &'a [Vec<u8>],
    query_max: &'a [Vec<u8>],
    config: &'a BKDConfig,
}

/// `getIntersectVisitor(hits, queryMin, queryMax, config)`.
fn get_intersect_visitor<'a>(
    hits: &'a Mutex<JavaBitSet>,
    query_min: &'a [Vec<u8>],
    query_max: &'a [Vec<u8>],
    config: &'a BKDConfig,
) -> RangeVisitor<'a> {
    RangeVisitor {
        hits,
        query_min,
        query_max,
        config,
    }
}

impl IntersectVisitor for RangeVisitor<'_> {
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        self.hits.lock().unwrap().set(doc_id as usize);
        // System.out.println("visit docID=" + docID);
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        // System.out.println("visit check docID=" + docID);
        let bpd = self.config.bytes_per_dim as usize;
        for dim in 0..self.config.num_index_dims as usize {
            let v = &packed_value[dim * bpd..dim * bpd + bpd];
            if v.cmp(&self.query_min[dim][..bpd]).is_lt()
                || v.cmp(&self.query_max[dim][..bpd]).is_gt()
            {
                // System.out.println("  no");
                return Ok(());
            }
        }

        // System.out.println("  yes");
        self.hits.lock().unwrap().set(doc_id as usize);
        Ok(())
    }

    fn visit_iterator_with_value(
        &mut self,
        iterator: &mut dyn DocIdSetIterator,
        packed_value: &[u8],
    ) -> Result<()> {
        if random().next_boolean() {
            // check the default method is correct (`IntersectVisitor.super.visit(iterator,
            // packedValue)`, whose body this is)
            loop {
                let doc_id = iterator.next_doc()?;
                if doc_id == NO_MORE_DOCS {
                    break;
                }
                self.visit_with_value(doc_id, packed_value)?;
            }
        } else {
            assert_equals(&iterator.doc_id(), &-1);
            let cost = i32::try_from(iterator.cost()).expect("Math.toIntExact");
            let mut number_of_points = 0;
            loop {
                let doc_id = iterator.next_doc()?;
                if doc_id == NO_MORE_DOCS {
                    break;
                }
                assert_equals(&iterator.doc_id(), &doc_id);
                self.visit_with_value(doc_id, packed_value)?;
                number_of_points += 1;
            }
            assert_equals(&cost, &number_of_points);
            assert_equals(&iterator.doc_id(), &NO_MORE_DOCS);
            assert_equals(&iterator.next_doc()?, &NO_MORE_DOCS);
            assert_equals(&iterator.doc_id(), &NO_MORE_DOCS);
        }
        Ok(())
    }

    fn compare(&self, min_packed: &[u8], max_packed: &[u8]) -> Result<Relation> {
        let bpd = self.config.bytes_per_dim as usize;
        let mut crosses = false;
        for dim in 0..self.config.num_index_dims as usize {
            let max = &max_packed[dim * bpd..dim * bpd + bpd];
            let min = &min_packed[dim * bpd..dim * bpd + bpd];
            if max.cmp(&self.query_min[dim][..bpd]).is_lt()
                || min.cmp(&self.query_max[dim][..bpd]).is_gt()
            {
                return Ok(Relation::CellOutsideQuery);
            } else if min.cmp(&self.query_min[dim][..bpd]).is_lt()
                || max.cmp(&self.query_max[dim][..bpd]).is_gt()
            {
                crosses = true;
            }
        }

        if crosses {
            Ok(Relation::CellCrossesQuery)
        } else {
            Ok(Relation::CellInsideQuery)
        }
    }
}

/// `randomBigInt(numBytes)`.
fn random_big_int(num_bytes: i32) -> BigInteger {
    let r = random();
    let mut x = BigInteger::random(num_bytes * 8 - 1, &*r);
    if random().next_boolean() {
        x = x.negate();
    }
    x
}

/// `getDirectory(numPoints)`.
fn get_directory(num_points: i32) -> TestDirectory {
    if num_points > 100000 {
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("TestBKDTree"))
    } else {
        LuceneTestCase::new_directory()
    }
}

/// The fixed data set of `testBitFlippedOnPartition1`/`2`.
fn fixed_doc_values(num_docs: i32, num_dims: i32, num_bytes_per_dim: i32) -> Vec<Vec<Vec<u8>>> {
    let mut doc_values = Vec::with_capacity(num_docs as usize);
    let mut counter: u8 = 0;
    for _ in 0..num_docs {
        let mut values = Vec::with_capacity(num_dims as usize);
        for _ in 0..num_dims {
            let mut v = bytes(num_bytes_per_dim);
            for b in v.iter_mut() {
                *b = counter;
                counter = counter.wrapping_add(1);
            }
            values.push(v);
        }
        doc_values.push(values);
    }
    doc_values
}

/// The anonymous `FilterDirectory` of `testBitFlippedOnPartition1`/`2`:
/// corrupts the first temporary output its predicate selects.
struct CorruptingDirectory {
    r#in: Arc<dyn Directory>,
    corrupted: Mutex<bool>,
    corrupt: fn(&str, &str) -> bool,
    byte_to_corrupt: i64,
}

impl Directory for CorruptingDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.r#in.list_all()
    }
    fn delete_file(&self, name: &str) -> Result<()> {
        self.r#in.delete_file(name)
    }
    fn file_length(&self, name: &str) -> Result<i64> {
        self.r#in.file_length(name)
    }
    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.r#in.create_output(name, context)
    }
    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        let out = self.r#in.create_temp_output(prefix, suffix, context)?;
        let mut corrupted = self.corrupted.lock().unwrap();
        if !*corrupted && (self.corrupt)(prefix, suffix) {
            *corrupted = true;
            Ok(Box::new(CorruptingIndexOutput::new(
                Arc::clone(&self.r#in),
                self.byte_to_corrupt,
                out,
            )))
        } else {
            Ok(out)
        }
    }
    fn sync(&self, names: &[String]) -> Result<()> {
        self.r#in.sync(names)
    }
    fn sync_metadata(&self) -> Result<()> {
        self.r#in.sync_metadata()
    }
    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.r#in.rename(source, dest)
    }
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.r#in.open_input(name, context)
    }
    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        self.r#in.open_checksum_input(name)
    }
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.r#in.obtain_lock(name)
    }
    /// `FilterDirectory.close()`: closes the wrapped directory, which the
    /// test's try-with-resources closes itself.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.r#in.get_pending_deletions()
    }
    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        Some(self.r#in.as_ref())
    }
}

/// `testBitFlippedOnPartition1()`: make sure corruption on an input sort
/// file is caught, even if BKDWriter doesn't get angry.
fn test_bit_flipped_on_partition1() -> Result<()> {
    // Generate fixed data set:
    let num_docs = LuceneTestCase::at_least(10000);
    let num_bytes_per_dim = 4;
    let num_dims = 3;
    let doc_values = fixed_doc_values(num_docs, num_dims, num_bytes_per_dim);

    let mut dir0 = LuceneTestCase::new_mock_directory();
    let dir: Arc<dyn Directory> = Arc::new(CorruptingDirectory {
        r#in: dir0.as_directory(),
        corrupted: Mutex::new(false),
        corrupt: |prefix, suffix| prefix == "_0" && suffix == "bkd_left0",
        byte_to_corrupt: 22,
    });

    let e = expect_throws("org.apache.lucene.index.CorruptIndexException", || {
        verify_in(
            &dir,
            &doc_values,
            None,
            num_dims,
            num_dims,
            num_bytes_per_dim,
            50,
            0.1,
        )
    });
    Assert::assert_true(
        e.message()
            .unwrap_or("")
            .contains("checksum failed (hardware problem?)"),
    );
    dir0.close()
}

/// `testBitFlippedOnPartition2()`: make sure corruption on a recursed
/// partition is caught, when BKDWriter does get angry.
fn test_bit_flipped_on_partition2() -> Result<()> {
    // Generate fixed data set:
    let num_docs = LuceneTestCase::at_least(10000);
    let num_bytes_per_dim = 4;
    let num_dims = 3;
    let doc_values = fixed_doc_values(num_docs, num_dims, num_bytes_per_dim);

    let mut dir0 = LuceneTestCase::new_mock_directory();
    let dir: Arc<dyn Directory> = Arc::new(CorruptingDirectory {
        r#in: dir0.as_directory(),
        corrupted: Mutex::new(false),
        // System.out.println("prefix=" + prefix + " suffix=" + suffix);
        corrupt: |_prefix, suffix| suffix == "bkd_left0",
        byte_to_corrupt: 22072,
    });

    let t = expect_throws("org.apache.lucene.index.CorruptIndexException", || {
        verify_in(
            &dir,
            &doc_values,
            None,
            num_dims,
            num_dims,
            num_bytes_per_dim,
            50,
            0.1,
        )
    });
    assert_corruption_detected(&t);
    dir0.close()
}

/// `assertCorruptionDetected(t)`.
fn assert_corruption_detected(t: &Throwable) {
    if t.is_instance("org.apache.lucene.index.CorruptIndexException")
        && t.message()
            .unwrap_or("")
            .contains("checksum failed (hardware problem?)")
    {
        return;
    }
    for suppressed in t.suppressed() {
        if suppressed.is_instance("org.apache.lucene.index.CorruptIndexException")
            && suppressed
                .message()
                .unwrap_or("")
                .contains("checksum failed (hardware problem?)")
        {
            return;
        }
    }
    Assert::fail("did not see a suppressed CorruptIndexException");
}

/// The anonymous `IntersectVisitor` of `testTieBreakOrder`.
struct TieBreakVisitor {
    last_doc_id: i32,
}

impl IntersectVisitor for TieBreakVisitor {
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        Assert::assert_true_msg(
            &format!("lastDocID={} docID={doc_id}", self.last_doc_id),
            doc_id > self.last_doc_id,
        );
        self.last_doc_id = doc_id;
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, _packed_value: &[u8]) -> Result<()> {
        self.visit(doc_id)
    }

    fn compare(&self, _min_packed: &[u8], _max_packed: &[u8]) -> Result<Relation> {
        Ok(Relation::CellCrossesQuery)
    }
}

fn test_tie_break_order() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let num_docs = 10000;
    let mut w = BKDWriter::new_default(
        num_docs + 1,
        Arc::clone(&d),
        "tmp",
        BKDConfig::new(1, 1, 4, 2)?,
        0.01f32 as f64,
        num_docs as i64,
    )?;
    for i in 0..num_docs {
        w.add(&[0u8; 4], i)?;
    }

    let out = create_output(&d, "bkd")?;
    let fp = finish(&mut w, &out)?;
    out.close_stream()?;

    let input = open_input(&d, "bkd")?;
    input.handle().seek(fp)?;
    let r = get_point_values(&input)?;
    r.intersect(&mut TieBreakVisitor { last_doc_id: -1 })?;
    input.close_stream()?;
    dir.close()
}

/// The anonymous `IntersectVisitor` of `testCheckDataDimOptimalOrder`.
struct OptimalOrderVisitor {
    previous: Option<Vec<u8>>,
    has_changed: bool,
    length: usize,
}

impl IntersectVisitor for OptimalOrderVisitor {
    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn visit_with_value(&mut self, _doc_id: i32, packed_value: &[u8]) -> Result<()> {
        match &mut self.previous {
            None => {
                self.previous = Some(packed_value[..self.length].to_vec());
            }
            Some(previous) => {
                if packed_value != previous.as_slice() {
                    if !self.has_changed {
                        self.has_changed = true;
                        previous.copy_from_slice(&packed_value[..self.length]);
                    } else {
                        Assert::fail("Points are not in optimal order");
                    }
                }
            }
        }
        Ok(())
    }

    fn compare(&self, _min_packed_value: &[u8], _max_packed_value: &[u8]) -> Result<Relation> {
        Ok(Relation::CellCrossesQuery)
    }
}

fn test_check_data_dim_optimal_order() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let num_values = LuceneTestCase::at_least(5000);
    let max_points_in_leaf_node = TestUtil::next_int(&*random(), 50, 500);
    let num_bytes_per_dim = TestUtil::next_int(&*random(), 1, 4);
    let max_mb = (3.0f32 as f64) + (3.0 * random().next_double());
    let num_index_dims = TestUtil::next_int(&*random(), 1, 8);
    let num_data_dims = TestUtil::next_int(&*random(), num_index_dims, 8);
    let mut point_value1 = bytes(num_data_dims * num_bytes_per_dim);
    let mut point_value2 = bytes(num_data_dims * num_bytes_per_dim);
    random().next_bytes(&mut point_value1);
    random().next_bytes(&mut point_value2);
    // equal index dimensions but different data dimensions
    let nbpd = num_bytes_per_dim as usize;
    for i in 0..num_index_dims as usize {
        point_value2[i * nbpd..(i + 1) * nbpd]
            .copy_from_slice(&point_value1[i * nbpd..(i + 1) * nbpd]);
    }

    let mut w = BKDWriter::new_default(
        2 * num_values,
        Arc::clone(&d),
        "_temp",
        BKDConfig::new(
            num_data_dims,
            num_index_dims,
            num_bytes_per_dim,
            max_points_in_leaf_node,
        )?,
        max_mb,
        2 * num_values as i64,
    )?;
    for i in 0..num_values {
        w.add(&point_value1, i)?;
        w.add(&point_value2, i)?;
    }
    let index_fp;
    {
        let out = create_output(&d, "bkd")?;
        index_fp = finish(&mut w, &out)?;
        w.close()?;
        out.close_stream()?;
    }

    let points_in = open_input(&d, "bkd")?;
    points_in.handle().seek(index_fp)?;
    let points = get_point_values(&points_in)?;

    points.intersect(&mut OptimalOrderVisitor {
        previous: None,
        has_changed: false,
        length: (num_data_dims * num_bytes_per_dim) as usize,
    })?;

    points_in.close_stream()?;
    dir.close()
}

/// The anonymous `IntersectVisitor` of `test2DLongOrdsOffline` and
/// `testWastedLeadingBytes`: counts every doc, and sometimes crosses.
struct CountingVisitor {
    count: i32,
    lengths: Option<(usize, usize)>,
}

impl IntersectVisitor for CountingVisitor {
    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        self.count += 1;
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if let Some((data_len, _)) = self.lengths {
            assert!(packed_value.len() == data_len);
        }
        self.visit(doc_id)
    }

    fn compare(&self, min_packed: &[u8], max_packed: &[u8]) -> Result<Relation> {
        if let Some((_, index_len)) = self.lengths {
            assert!(min_packed.len() == index_len);
            assert!(max_packed.len() == index_len);
        }
        if random().next_int_bound(7) == 1 {
            Ok(Relation::CellCrossesQuery)
        } else {
            Ok(Relation::CellInsideQuery)
        }
    }
}

fn test_2d_long_ords_offline() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let num_docs = 100000;
    let mut w = BKDWriter::new_default(
        num_docs + 1,
        Arc::clone(&d),
        "tmp",
        BKDConfig::new(2, 2, 4, 2)?,
        0.01f32 as f64,
        num_docs as i64,
    )?;
    let mut buffer = [0u8; 2 * 4];
    for i in 0..num_docs {
        random().next_bytes(&mut buffer);
        w.add(&buffer, i)?;
    }

    let out = create_output(&d, "bkd")?;
    let fp = finish(&mut w, &out)?;
    out.close_stream()?;

    let input = open_input(&d, "bkd")?;
    input.handle().seek(fp)?;
    let r = get_point_values(&input)?;
    let mut visitor = CountingVisitor {
        count: 0,
        lengths: None,
    };
    r.intersect(&mut visitor)?;
    assert_equals(&num_docs, &visitor.count);
    input.close_stream()?;
    dir.close()
}

/// `testWastedLeadingBytes()`: claims 16 bytes per dim, but only use the
/// bottom N 1-3 bytes; this would happen e.g. if a user indexes what are
/// actually just short values as a LongPoint.
fn test_wasted_leading_bytes() -> Result<()> {
    let random = random();
    let num_dims = TestUtil::next_int(&*random, 1, MAX_INDEX_DIMENSIONS);
    let num_index_dims = TestUtil::next_int(&*random, 1, num_dims);
    let bytes_per_dim = MAX_NUM_BYTES;
    let bytes_used = TestUtil::next_int(&*random, 1, 3);

    let mut dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir());
    let d = dir.as_directory();
    let num_docs = LuceneTestCase::at_least(10000);
    let mut w = BKDWriter::new_default(
        num_docs + 1,
        Arc::clone(&d),
        "tmp",
        BKDConfig::new(num_dims, num_index_dims, bytes_per_dim, 32)?,
        1.0,
        num_docs as i64,
    )?;
    let mut tmp = bytes(bytes_used);
    let mut buffer = bytes(num_dims * bytes_per_dim);
    let bpd = bytes_per_dim as usize;
    let used = bytes_used as usize;
    for i in 0..num_docs {
        for dim in 0..num_dims as usize {
            random.next_bytes(&mut tmp);
            let start = dim * bpd + (bpd - used);
            buffer[start..start + used].copy_from_slice(&tmp);
        }
        w.add(&buffer, i)?;
    }

    let out = create_output(&d, "bkd")?;
    let fp = finish(&mut w, &out)?;
    out.close_stream()?;

    let input = open_input(&d, "bkd")?;
    input.handle().seek(fp)?;
    let r = get_point_values(&input)?;
    let mut visitor = CountingVisitor {
        count: 0,
        lengths: Some((
            (num_dims * bytes_per_dim) as usize,
            (num_index_dims * bytes_per_dim) as usize,
        )),
    };
    r.intersect(&mut visitor)?;
    assert_equals(&num_docs, &visitor.count);
    input.close_stream()?;
    dir.close()
}

/// The anonymous `IntersectVisitor`s of `testEstimatePointCount`, which
/// visit nothing and answer `compare` with `relation`.
struct EstimateVisitor<F: Fn(&[u8], &[u8]) -> Relation> {
    relation: F,
}

impl<F: Fn(&[u8], &[u8]) -> Relation> IntersectVisitor for EstimateVisitor<F> {
    fn visit_with_value(&mut self, _doc_id: i32, _packed_value: &[u8]) -> Result<()> {
        Ok(())
    }

    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Result<Relation> {
        Ok((self.relation)(min_packed_value, max_packed_value))
    }
}

fn test_estimate_point_count() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let num_values = LuceneTestCase::at_least(10000); // make sure to have multiple leaves
    let max_points_in_leaf_node = TestUtil::next_int(&*random(), 50, 500);
    let num_bytes_per_dim = TestUtil::next_int(&*random(), 1, 4);
    let mut point_value = bytes(num_bytes_per_dim);
    let mut unique_point_value = bytes(num_bytes_per_dim);
    random().next_bytes(&mut unique_point_value);

    let mut w = BKDWriter::new_default(
        num_values,
        Arc::clone(&d),
        "_temp",
        BKDConfig::new(1, 1, num_bytes_per_dim, max_points_in_leaf_node)?,
        DEFAULT_MAX_MB_SORT_IN_HEAP,
        num_values as i64,
    )?;
    for i in 0..num_values {
        if i == num_values / 2 {
            w.add(&unique_point_value, i)?;
        } else {
            loop {
                random().next_bytes(&mut point_value);
                if point_value != unique_point_value {
                    break;
                }
            }
            w.add(&point_value, i)?;
        }
    }
    let index_fp;
    {
        let out = create_output(&d, "bkd")?;
        index_fp = finish(&mut w, &out)?;
        w.close()?;
        out.close_stream()?;
    }

    let points_in = open_input(&d, "bkd")?;
    points_in.handle().seek(index_fp)?;
    let points = get_point_values(&points_in)?;

    // If all points match, then the point count is numValues
    assert_equals(
        &(num_values as i64),
        &points.estimate_point_count(&mut EstimateVisitor {
            relation: |_, _| Relation::CellInsideQuery,
        })?,
    );

    // Return 0 if no points match
    assert_equals(
        &0,
        &points.estimate_point_count(&mut EstimateVisitor {
            relation: |_, _| Relation::CellOutsideQuery,
        })?,
    );

    // If only one point matches, then the point count is (actualMaxPointsInLeafNode + 1) / 2
    // in general, or maybe 2x that if the point is a split value
    let nbpd = num_bytes_per_dim as usize;
    let unique = unique_point_value.clone();
    let point_count = points.estimate_point_count(&mut EstimateVisitor {
        relation: move |min_packed_value: &[u8], max_packed_value: &[u8]| {
            if unique[..nbpd].cmp(&max_packed_value[..nbpd]).is_gt()
                || unique[..nbpd].cmp(&min_packed_value[..nbpd]).is_lt()
            {
                return Relation::CellOutsideQuery;
            }
            Relation::CellCrossesQuery
        },
    })?;

    let max_points_in_leaf_node = max_points_in_leaf_node as i64;
    let last_node_point_count = num_values as i64 % max_points_in_leaf_node;
    Assert::assert_true_msg(
        &point_count.to_string(),
        point_count == (max_points_in_leaf_node + 1) / 2 // common case
            || point_count == (last_node_point_count + 1) / 2 // not fully populated leaf
            || point_count == 2 * ((max_points_in_leaf_node + 1) / 2) // if the point is a split value
            || point_count
                == ((max_points_in_leaf_node + 1) / 2) + ((last_node_point_count + 1) / 2),
    ); // if the point is a split value and one leaf is not fully populated

    points_in.close_stream()?;
    dir.close()
}

/// The anonymous `MutablePointTree` of `testTotalPointCountValidation`.
struct ConstantPointTree {
    point_value: Vec<u8>,
    num_points_added: i64,
}

fn unsupported<T>() -> Result<T> {
    Err(LuceneError::unsupported_operation(String::new()))
}

impl PointTree for ConstantPointTree {
    fn clone_tree(&self) -> Result<Box<dyn PointTree>> {
        unsupported()
    }
    fn move_to_child(&mut self) -> Result<bool> {
        unsupported()
    }
    fn move_to_sibling(&mut self) -> Result<bool> {
        unsupported()
    }
    fn move_to_parent(&mut self) -> Result<bool> {
        unsupported()
    }
    fn min_packed_value(&self) -> Result<&[u8]> {
        unsupported()
    }
    fn max_packed_value(&self) -> Result<&[u8]> {
        unsupported()
    }
    fn size(&self) -> Result<i64> {
        Ok(self.num_points_added)
    }
    fn visit_doc_ids(&mut self, _visitor: &mut dyn IntersectVisitor) -> Result<()> {
        unsupported()
    }
    fn visit_doc_values(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        for _ in 0..self.num_points_added {
            visitor.visit_with_value(0, &self.point_value)?;
        }
        Ok(())
    }
    fn as_mutable(&mut self) -> Option<&mut dyn MutablePointTree> {
        Some(self)
    }
}

impl MutablePointTree for ConstantPointTree {
    fn value(&self, _i: i32) -> std::borrow::Cow<'_, [u8]> {
        std::borrow::Cow::Borrowed(&self.point_value)
    }
    fn byte_at(&self, i: i32, k: i32) -> u8 {
        self.value(i)[k as usize]
    }
    fn doc_id(&self, _i: i32) -> i32 {
        0
    }
    fn swap(&mut self, _i: i32, _j: i32) {
        // do nothing
    }
    fn save(&mut self, _i: i32, _j: i32) {
        // do nothing
    }
    fn restore(&mut self, _i: i32, _j: i32) {
        // do nothing
    }
}

fn test_total_point_count_validation() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let num_values = 10;
    let num_points_added = 50; // exceeds totalPointCount
    let num_bytes_per_dim = TestUtil::next_int(&*random(), 1, 4);
    let mut point_value = bytes(num_bytes_per_dim);
    random().next_bytes(&mut point_value);
    let mut reader = ConstantPointTree {
        point_value,
        num_points_added,
    };
    let mut w = BKDWriter::new_default(
        num_values,
        Arc::clone(&d),
        "_temp",
        BKDConfig::new(
            1,
            1,
            num_bytes_per_dim,
            BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE,
        )?,
        DEFAULT_MAX_MB_SORT_IN_HEAP,
        num_values as i64,
    )?;
    expect_throws("java.lang.IllegalStateException", || -> Result<()> {
        let out = create_output(&d, "bkd")?;
        // `w.writeField(out, out, out, "test_field_name", reader)`: the port
        // takes the data output and returns the finalizer.
        let result = w.write_field(&mut out.handle(), &mut reader);
        let finalized = match result {
            Ok(Some(finalizer)) => finalizer.run(&mut out.handle(), &mut out.handle()),
            Ok(None) => Ok(()),
            Err(e) => Err(e),
        };
        let closed_out = out.close_stream();
        let closed_w = w.close();
        let closed_dir = dir.close();
        finalized.and(closed_out).and(closed_w).and(closed_dir)
    });
    Ok(())
}

fn test_too_many_points() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let num_values = 10;
    let num_bytes_per_dim = TestUtil::next_int(&*random(), 1, 4);
    let mut point_value = bytes(num_bytes_per_dim);
    let mut w = BKDWriter::new_default(
        num_values,
        Arc::clone(&d),
        "_temp",
        BKDConfig::new(1, 1, num_bytes_per_dim, 2)?,
        DEFAULT_MAX_MB_SORT_IN_HEAP,
        num_values as i64,
    )?;
    for i in 0..num_values {
        random().next_bytes(&mut point_value);
        w.add(&point_value, i)?;
    }
    random().next_bytes(&mut point_value);
    let ex = expect_throws("java.lang.IllegalStateException", || {
        w.add(&point_value, num_values)
    });
    assert_equals(
        &Some("totalPointCount=10 was passed when we were created, but we just hit 11 values"),
        &ex.message(),
    );
    w.close()?;
    dir.close()
}

/// The anonymous `MutablePointTree` of `testTooManyPoints1D`.
struct ElevenPointTree {
    point_value: Vec<Vec<u8>>,
    doc_id: Vec<i32>,
    tmp_values: Vec<Vec<u8>>,
    tmp_docs: Vec<i32>,
}

impl PointTree for ElevenPointTree {
    fn clone_tree(&self) -> Result<Box<dyn PointTree>> {
        unsupported()
    }
    fn move_to_child(&mut self) -> Result<bool> {
        unsupported()
    }
    fn move_to_sibling(&mut self) -> Result<bool> {
        unsupported()
    }
    fn move_to_parent(&mut self) -> Result<bool> {
        unsupported()
    }
    fn min_packed_value(&self) -> Result<&[u8]> {
        unsupported()
    }
    fn max_packed_value(&self) -> Result<&[u8]> {
        unsupported()
    }
    fn size(&self) -> Result<i64> {
        Ok(11)
    }
    fn visit_doc_ids(&mut self, _visitor: &mut dyn IntersectVisitor) -> Result<()> {
        unsupported()
    }
    fn visit_doc_values(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        for i in 0..self.size()? as usize {
            visitor.visit_with_value(self.doc_id[i], &self.point_value[i])?;
        }
        Ok(())
    }
    fn as_mutable(&mut self) -> Option<&mut dyn MutablePointTree> {
        Some(self)
    }
}

impl MutablePointTree for ElevenPointTree {
    fn value(&self, i: i32) -> std::borrow::Cow<'_, [u8]> {
        std::borrow::Cow::Borrowed(&self.point_value[i as usize])
    }
    fn byte_at(&self, i: i32, k: i32) -> u8 {
        self.point_value[i as usize][k as usize]
    }
    fn doc_id(&self, i: i32) -> i32 {
        self.doc_id[i as usize]
    }
    fn swap(&mut self, i: i32, j: i32) {
        self.point_value.swap(i as usize, j as usize);
        self.doc_id.swap(i as usize, j as usize);
    }
    fn save(&mut self, i: i32, j: i32) {
        self.tmp_values[j as usize] = self.point_value[i as usize].clone();
        self.tmp_docs[j as usize] = self.doc_id[i as usize];
    }
    fn restore(&mut self, i: i32, j: i32) {
        for k in i as usize..j as usize {
            self.point_value[k] = self.tmp_values[k].clone();
            self.doc_id[k] = self.tmp_docs[k];
        }
    }
}

fn test_too_many_points_1d() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let num_values = 10;
    let num_bytes_per_dim = TestUtil::next_int(&*random(), 1, 4);
    let mut point_value = vec![bytes(num_bytes_per_dim); 11];
    let mut doc_id = vec![0i32; 11];
    let mut w = BKDWriter::new_default(
        num_values + 1,
        Arc::clone(&d),
        "_temp",
        BKDConfig::new(1, 1, num_bytes_per_dim, 2)?,
        DEFAULT_MAX_MB_SORT_IN_HEAP,
        num_values as i64,
    )?;
    for i in 0..(num_values + 1) as usize {
        random().next_bytes(&mut point_value[i]);
        doc_id[i] = i as i32;
    }
    let mut val = ElevenPointTree {
        point_value,
        doc_id,
        tmp_values: vec![Vec::new(); num_values as usize],
        tmp_docs: vec![0; num_values as usize],
    };
    {
        let out = create_output(&d, "bkd")?;
        let ex = expect_throws("java.lang.IllegalStateException", || {
            // `w.writeField(out, out, out, "", val)`.
            match w.write_field(&mut out.handle(), &mut val)? {
                Some(finalizer) => finalizer.run(&mut out.handle(), &mut out.handle()),
                None => Ok(()),
            }
        });
        assert_equals(
            &Some("totalPointCount=10 was passed when we were created, but we just hit 11 values"),
            &ex.message(),
        );
        w.close()?;
        out.close_stream()?;
    }
    dir.close()
}
