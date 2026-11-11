//! Port of `lucene/core/src/test/org/apache/lucene/codecs/lucene90/compressing/TestStoredFieldsInt.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::codecs::lucene90::stored_fields::StoredFieldsInts;
use rucene::error::Result;
use rucene::store::{Directory, DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_array_equals, assert_equals, class, random, test};

/// `TestStoredFieldsInt`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.codecs.lucene90.compressing.TestStoredFieldsInt",
        vec![
            test("testRandom", test_random),
            test("testAllEquals", test_all_equals),
        ],
    )
}

/// Port of `TestStoredFieldsInt.testRandom`.
fn test_random() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(100);
    let dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        for _iter in 0..num_iters {
            let mut values = vec![0i32; (random().next_int_bound(5000) + 1) as usize];
            let bpv = TestUtil::next_int(&*random(), 1, 31);
            for i in 0..values.len() {
                values[i] = TestUtil::next_int(&*random(), 0, ((1i64 << bpv) - 1) as i32);
            }
            test_ints(&*dir.as_directory(), &values)?;
        }
        Ok(())
    })();
    result.and(dir.close())
}

/// Port of `TestStoredFieldsInt.testAllEquals`.
fn test_all_equals() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let mut doc_ids = vec![0i32; (random().next_int_bound(5000) + 1) as usize];
        let bpv = TestUtil::next_int(&*random(), 1, 31);
        doc_ids.fill(TestUtil::next_int(
            &*random(),
            0,
            ((1i64 << bpv) - 1) as i32,
        ));
        test_ints(&*dir.as_directory(), &doc_ids)
    })();
    result.and(dir.close())
}

/// `private void test(Directory dir, int[] ints)`.
fn test_ints(dir: &dyn Directory, ints: &[i32]) -> Result<()> {
    let len;
    {
        let mut out = dir.create_output("tmp", &*DEFAULT_IO_CONTEXT)?;
        let result = (|| -> Result<i64> {
            StoredFieldsInts::write_ints(ints, 0, ints.len(), &mut *out)?;
            let len = out.file_pointer();
            if random().next_boolean() {
                out.write_long(0)?; // garbage
            }
            Ok(len)
        })();
        let closed = out.close();
        len = result?;
        closed?;
    }

    {
        let mut r#in = dir.open_input("tmp", &*READONCE_IO_CONTEXT)?;
        let result = (|| -> Result<()> {
            let offset = random().next_int_bound(5) as usize;
            let mut read = vec![0i64; ints.len() + offset];
            StoredFieldsInts::read_ints(&mut *r#in, ints.len(), &mut read, offset)?;
            let mut read_ints = vec![0i32; ints.len()];
            for i in 0..ints.len() {
                read_ints[i] = read[offset + i] as i32;
            }
            assert_array_equals(ints, &read_ints);
            assert_equals(&len, &r#in.file_pointer());
            Ok(())
        })();
        result.and(r#in.close())?;
    }
    dir.delete_file("tmp")
}
