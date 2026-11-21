//! Port of `lucene/core/src/test/org/apache/lucene/codecs/lucene104/TestPostingsUtil.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::codecs::lucene104::{read_v_int_block, write_v_int_block, FOR_BLOCK_SIZE};
use rucene::error::Result;
use rucene::store::DEFAULT_IO_CONTEXT;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestPostingsUtil`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.codecs.lucene104.TestPostingsUtil",
        vec![test("testIntegerOverflow", test_integer_overflow)],
    )
}

/// checks for bug described in https://github.com/apache/lucene/issues/13373
///
/// Port of `TestPostingsUtil.testIntegerOverflow`.
fn test_integer_overflow() -> Result<()> {
    // Size that writes the first value as a regular vint
    let random_size1 = random().next_int_range(1, 3);
    // Size that writes the first value as a group vint
    let random_size2 = random().next_int_range(4, FOR_BLOCK_SIZE as i32);
    do_test_integer_overflow(random_size1 as usize)?;
    do_test_integer_overflow(random_size2 as usize)
}

/// `private void doTestIntegerOverflow(int size)`.
fn do_test_integer_overflow(size: usize) -> Result<()> {
    let mut doc_delta_buffer = vec![0i32; size];
    let freq_buffer = vec![0i32; size];
    let delta = 1 << 30;
    doc_delta_buffer[0] = delta;
    let dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        {
            let mut out = dir
                .as_directory()
                .create_output("test", &*DEFAULT_IO_CONTEXT)?;
            // In old implementation, this would cause integer overflow exception.
            let r = write_v_int_block(&mut *out, &mut doc_delta_buffer, &freq_buffer, size, true);
            r.and(out.close())?;
        }
        let mut restored_docs = vec![0i32; size];
        let mut restored_freqs = vec![0i32; size];
        {
            let mut r#in = dir
                .as_directory()
                .open_input("test", &*DEFAULT_IO_CONTEXT)?;
            let r = read_v_int_block(
                &mut *r#in,
                &mut restored_docs,
                &mut restored_freqs,
                size,
                true,
                true,
            );
            r.and(r#in.close())?;
        }
        assert_equals(&delta, &restored_docs[0]);
        Ok(())
    })();
    result.and(dir.close())
}
