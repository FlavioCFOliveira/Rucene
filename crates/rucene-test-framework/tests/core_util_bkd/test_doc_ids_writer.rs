//! Port of `lucene/core/src/test/org/apache/lucene/util/bkd/TestDocIdsWriter.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use rucene::document::{Document, IntPoint, SortedNumericDocValuesField};
use rucene::error::{LuceneError, Result};
use rucene::index::index_writer::IndexWriter;
use rucene::index::point_values::{IntersectVisitor, Relation};
use rucene::store::{DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT};
use rucene::util::bkd::{BKDConfig, BKDWriter, DocIdsWriter};
use rucene::util::Constants;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_array_equals, assert_equals, class, random, test, JavaHashSet};

/// `TestDocIdsWriter`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testRandom", test_random),
        test("testSorted", test_sorted),
        test("testCluster", test_cluster),
        test("testBitSet", test_bit_set),
        test("testContinuousIds", test_continuous_ids),
        test("testCrash", test_crash),
    ];
    class("org.apache.lucene.util.bkd.TestDocIdsWriter", methods)
}

/// `VERSIONS`.
const VERSIONS: [i32; 2] = [BKDWriter::VERSION_META_FILE, BKDWriter::VERSION_CURRENT];

/// `try (Directory dir = newDirectory()) { body }`.
fn with_directory(body: impl FnOnce(&TestDirectory) -> Result<()>) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let result = body(&dir);
    let closed = dir.close();
    result.and(closed)
}

fn test_random() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(100);
    with_directory(|dir| {
        for _ in 0..num_iters {
            let count = if random().next_boolean() {
                1 + random().next_int_bound(5000)
            } else {
                BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE
            };
            let mut doc_ids = vec![0i32; count as usize];
            let bpv = TestUtil::next_int(&*random(), 1, 32);
            for d in doc_ids.iter_mut() {
                *d = TestUtil::next_int(
                    &*random(),
                    0,
                    1i32.wrapping_shl(bpv as u32).wrapping_sub(1),
                );
            }
            test_ints(dir, &doc_ids)?;
        }
        Ok(())
    })
}

fn test_sorted() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(100);
    with_directory(|dir| {
        for _ in 0..num_iters {
            let mut doc_ids = vec![0i32; 1 + random().next_int_bound(5000) as usize];
            let bpv = TestUtil::next_int(&*random(), 1, 32);
            for d in doc_ids.iter_mut() {
                *d = TestUtil::next_int(
                    &*random(),
                    0,
                    1i32.wrapping_shl(bpv as u32).wrapping_sub(1),
                );
            }
            doc_ids.sort();
            test_ints(dir, &doc_ids)?;
        }
        Ok(())
    })
}

fn test_cluster() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(100);
    with_directory(|dir| {
        for _ in 0..num_iters {
            let count = if random().next_boolean() {
                1 + random().next_int_bound(5000)
            } else {
                BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE
            };
            let mut doc_ids = vec![0i32; count as usize];
            let min = random().next_int_bound(1000);
            let bpv = TestUtil::next_int(&*random(), 1, 16);
            for d in doc_ids.iter_mut() {
                *d = min + TestUtil::next_int(&*random(), 0, (1 << bpv) - 1);
            }
            test_ints(dir, &doc_ids)?;
        }
        Ok(())
    })
}

fn test_bit_set() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(100);
    with_directory(|dir| {
        for _ in 0..num_iters {
            let size = 1 + random().next_int_bound(5000);
            // `HashSet.newHashSet(size)`: a set sized for `size` mappings.
            let mut set: JavaHashSet<i32> =
                JavaHashSet::with_capacity(((size as f64) / 0.75f64).ceil() as usize);
            let small = random().next_int_bound(1000);
            while set.size() < size as usize {
                set.add(small + random().next_int_bound(size * 16));
            }
            let mut doc_ids = set.to_vec();
            doc_ids.sort();
            test_ints(dir, &doc_ids)?;
        }
        Ok(())
    })
}

fn test_continuous_ids() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(100);
    with_directory(|dir| {
        for _ in 0..num_iters {
            let size = 1 + random().next_int_bound(5000);
            let mut doc_ids = vec![0i32; size as usize];
            let start = random().next_int_bound(1000000);
            for (i, d) in doc_ids.iter_mut().enumerate() {
                *d = start + i as i32;
            }
            test_ints(dir, &doc_ids)?;
        }
        Ok(())
    })
}

/// The anonymous `IntersectVisitor` of `test`: records each visited doc ID.
struct RecordingVisitor<'a> {
    read: &'a mut [i32],
    i: usize,
}

impl IntersectVisitor for RecordingVisitor<'_> {
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        self.read[self.i] = doc_id;
        self.i += 1;
        Ok(())
    }

    fn visit_with_value(&mut self, _doc_id: i32, _packed_value: &[u8]) -> Result<()> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn compare(&self, _min_packed_value: &[u8], _max_packed_value: &[u8]) -> Result<Relation> {
        Err(LuceneError::unsupported_operation(String::new()))
    }
}

/// `test(dir, ints)`.
fn test_ints(dir: &TestDirectory, ints: &[i32]) -> Result<()> {
    let dir = dir.as_directory();
    // It is hard to get BPV24-encoded docs in TextLuceneXXPointsFormat, test bwc here as well.
    let version = VERSIONS[random().next_int_bound(VERSIONS.len() as i32) as usize];
    // `new DocIdsWriter(ints.length, version)`: Rucene's `DocIdsWriter` is
    // stateless, the version passed to each call.
    let len;
    {
        let mut out = dir.create_output("tmp", &*DEFAULT_IO_CONTEXT)?;
        DocIdsWriter::write_doc_ids(out.as_mut(), ints, version)?;
        len = out.file_pointer();
        if random().next_boolean() {
            out.write_long(0)?; // garbage
        }
        out.close()?;
    }
    {
        let mut input = dir.open_input("tmp", &*READONCE_IO_CONTEXT)?;
        let mut read = vec![0i32; ints.len()];
        DocIdsWriter::read_doc_ids(input.as_mut(), ints.len(), &mut read, version)?;
        assert_array_equals(ints, &read);
        assert_equals(&len, &input.file_pointer());
        input.close()?;
    }
    {
        let mut input = dir.open_input("tmp", &*READONCE_IO_CONTEXT)?;
        let mut read = vec![0i32; ints.len()];
        let mut buffer = vec![0i32; ints.len()];
        {
            let mut visitor = RecordingVisitor {
                read: &mut read,
                i: 0,
            };
            DocIdsWriter::read_doc_ids_visitor(
                input.as_mut(),
                ints.len(),
                &mut visitor,
                &mut buffer,
                version,
            )?;
        }
        assert_array_equals(ints, &read);
        assert_equals(&len, &input.file_pointer());
        input.close()?;
    }
    dir.delete_file("tmp")
}

/// `testCrash()`: this simple test tickles a JVM C2 JIT crash on JDK's less
/// than 21.0.1. Crashes only when run with HotSpot C2. Regardless of whether
/// C2 is enabled or not, the test should never fail.
fn test_crash() -> Result<()> {
    LuceneTestCase::assume_true(
        "Requires HotSpot C2 compiler (won't work on client VM).",
        Constants::is_hotspot_vm() && !Constants::is_client_vm(),
    );
    let itrs = LuceneTestCase::at_least(100);
    for _ in 0..itrs {
        let mut dir = LuceneTestCase::new_directory();
        let iw = IndexWriter::new(
            dir.as_directory(),
            LuceneTestCase::new_index_writer_config_analyzer(None),
        )?;
        for _ in 0..20_000 {
            let mut doc = Document::new();
            doc.add(Box::new(IntPoint::new("foo", &[0])?));
            doc.add(Box::new(SortedNumericDocValuesField::new("bar", 0)));
            iw.add_document(&doc)?;
        }
        iw.close()?;
        dir.close()?;
    }
    Ok(())
}
