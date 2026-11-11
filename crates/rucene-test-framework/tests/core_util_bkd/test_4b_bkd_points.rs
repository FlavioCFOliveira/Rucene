//! Port of `lucene/core/src/test/org/apache/lucene/util/bkd/Test4BBKDPoints.java`
//! (`@TimeoutSuite(millis = Integer.MAX_VALUE)`, `@Monster("takes at least 4
//! hours and consumes many GB of temp disk space")`).

use std::sync::Arc;

use randomizedtesting::annotations::TimeoutSuite;
use randomizedtesting::jdk::reflect::{Class, Method};
use rucene::error::Result;
use rucene::index::check_index::VerifyPointsVisitor;
use rucene::index::point_values::PointValues;
use rucene::store::{Directory, FSDirectory, IndexInput, DEFAULT_IO_CONTEXT};
use rucene::util::bkd::{BKDConfig, BKDReader, BKDWriter};
use rucene::util::NumericUtils;
use rucene_test_framework::util::lucene_test_case::Monster;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class_annotated, random, test, verbose};
use crate::shared_io::{SharedIndexInput, SharedIndexOutput};

/// `Test4BBKDPoints`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![test("test1D", test_1d), test("test2D", test_2d)];
    class_annotated(
        "org.apache.lucene.util.bkd.Test4BBKDPoints",
        methods,
        |builder| {
            builder
                // hopefully ~24 days is long enough ;)
                .annotate(TimeoutSuite { millis: i32::MAX })
                .annotate(Monster {
                    value: "takes at least 4 hours and consumes many GB of temp disk space"
                        .to_string(),
                })
        },
    )
}

/// `BKDWriter.DEFAULT_MAX_MB_SORT_IN_HEAP`.
const DEFAULT_MAX_MB_SORT_IN_HEAP: f64 = 16.0;

/// The writing and checking half shared by `test1D` and `test2D`: `name`
/// is the BKD file, `dims` the dimensions, `fill` the packed bytes of one
/// value.
fn run(
    temp_dir_prefix: &str,
    file: &str,
    field: &str,
    dims: i32,
    fill: fn(&mut [u8], i32),
) -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(
        LuceneTestCase::create_temp_dir_prefix(temp_dir_prefix),
    )?);
    let num_docs = (i32::MAX / 13) + 100;

    let mut w = BKDWriter::new_default(
        num_docs,
        Arc::clone(&dir),
        "_0",
        BKDConfig::new(dims, dims, 8, BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE)?,
        DEFAULT_MAX_MB_SORT_IN_HEAP,
        26i64 * num_docs as i64,
    )?;
    let mut counter = 0i32;
    let mut packed_bytes = vec![0u8; dims as usize * 8];
    for doc_id in 0..num_docs {
        for _ in 0..26 {
            fill(&mut packed_bytes, counter);
            w.add(&packed_bytes, doc_id)?;
            counter = counter.wrapping_add(1);
        }
        if verbose() && doc_id % 100000 == 0 {
            println!("{doc_id} of {num_docs}...");
        }
    }
    let out = SharedIndexOutput::new(dir.create_output(file, &*DEFAULT_IO_CONTEXT)?);
    let finalizer = w.finish(&mut out.handle())?;
    let index_fp = out.get_file_pointer();
    finalizer
        .expect("points were added")
        .run(&mut out.handle(), &mut out.handle())?;
    out.close_stream()?;

    let input = SharedIndexInput::new(dir.open_input(file, &*DEFAULT_IO_CONTEXT)?);
    input.handle().seek(index_fp)?;
    let r = BKDReader::new(
        &mut input.handle(),
        &mut input.handle(),
        &mut input.handle(),
    )?;
    let mut visitor = VerifyPointsVisitor::new(field, num_docs, &r)?;
    r.intersect(&mut visitor)?;
    assert_equals(&PointValues::size(&r)?, &visitor.get_point_count_seen());
    assert_equals(
        &(PointValues::doc_count(&r)? as i64),
        &visitor.get_doc_count_seen(),
    );
    input.close_stream()?;
    dir.close()
}

fn test_1d() -> Result<()> {
    run(
        "4BBKDPoints1D",
        "1d.bkd",
        "1d",
        1,
        |packed_bytes, counter| {
            // first a random int:
            NumericUtils::int_to_sortable_bytes(random().next_int(), packed_bytes, 0);
            // then our counter, which will overflow a bit in the end:
            NumericUtils::int_to_sortable_bytes(counter, packed_bytes, 4);
        },
    )
}

fn test_2d() -> Result<()> {
    run(
        "4BBKDPoints2D",
        "2d.bkd",
        "2d",
        2,
        |packed_bytes, counter| {
            // first a random int:
            NumericUtils::int_to_sortable_bytes(random().next_int(), packed_bytes, 0);
            // then our counter, which will overflow a bit in the end:
            NumericUtils::int_to_sortable_bytes(counter, packed_bytes, 4);
            // then two random ints for the 2nd dimension:
            NumericUtils::int_to_sortable_bytes(random().next_int(), packed_bytes, 8);
            NumericUtils::int_to_sortable_bytes(random().next_int(), packed_bytes, 8 + 4);
        },
    )
}
