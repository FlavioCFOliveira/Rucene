//! Port of `lucene/core/src/test/org/apache/lucene/codecs/lucene104/TestPForUtil.java`.
//!
//! `Lucene104PostingsReader.VECTORIZATION_PROVIDER.newPostingDecodingUtil(in)`
//! is the scalar [`PostingDecodingUtil`] over `in`, the only implementation
//! the port has; `new PForUtil(new ForUtil())` is `PForUtil::new()`, which
//! builds its own `ForUtil`.

use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::reflect::Class;
use rucene::codecs::lucene104::postings_reader::Lucene104PostingsReader;
use rucene::codecs::lucene104::{PForUtil, FOR_BLOCK_SIZE};
use rucene::error::Result;
use rucene::store::{ByteBuffersDirectory, Directory, DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT};
use rucene::util::packed::PackedInts;
use rucene_test_framework::util::TestUtil;

use crate::core_support::{
    arrays_to_string, assert_array_equals_msg, assert_equals, class, random, test,
};

/// `TestPForUtil`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.codecs.lucene104.TestPForUtil",
        vec![test("testEncodeDecode", test_encode_decode)],
    )
}

/// Port of `TestPForUtil.testEncodeDecode`.
fn test_encode_decode() -> Result<()> {
    let iterations = RandomNumbers::random_int_between(&*random(), 50, 1000) as usize;
    let values = create_test_data(iterations, 31);
    let d = ByteBuffersDirectory::new();
    let end_pointer = encode_test_data(iterations, &values, &d)?;
    let r#in = d.open_input("test.bin", &*READONCE_IO_CONTEXT)?;
    let mut pdu;
    {
        // `Lucene104PostingsReader.VECTORIZATION_PROVIDER.newPostingDecodingUtil(in)`.
        pdu = Lucene104PostingsReader::vectorization_provider().new_posting_decoding_util(r#in)?;
        let mut pfor_util = PForUtil::new();
        for i in 0..iterations {
            if random().next_int_bound(5) == 0 {
                PForUtil::skip(&mut *pdu.r#in)?;
                continue;
            }
            let mut restored = [0i32; FOR_BLOCK_SIZE];
            pfor_util.decode(&mut pdu, &mut restored)?;
            let mut ints = [0i32; FOR_BLOCK_SIZE];
            ints[..FOR_BLOCK_SIZE].copy_from_slice(&restored[..FOR_BLOCK_SIZE]);
            assert_array_equals_msg(
                Some(&arrays_to_string(&ints)),
                &values[i * FOR_BLOCK_SIZE..(i + 1) * FOR_BLOCK_SIZE],
                &ints,
            );
        }
        assert_equals(&end_pointer, &pdu.r#in.file_pointer()?);
    }
    pdu.r#in.close()?;
    d.close()
}

/// `private int[] createTestData(int iterations, int maxBpv)`.
fn create_test_data(iterations: usize, max_bpv: i32) -> Vec<i32> {
    let mut values = vec![0i32; iterations * FOR_BLOCK_SIZE];
    for i in 0..iterations {
        let bpv = TestUtil::next_int(&*random(), 0, max_bpv);
        for j in 0..FOR_BLOCK_SIZE {
            values[i * FOR_BLOCK_SIZE + j] =
                RandomNumbers::random_int_between(&*random(), 0, PackedInts::max_value(bpv) as i32);
            if random().next_int_bound(100) == 0 {
                let exception_bpv = if random().next_int_bound(10) == 0 {
                    (bpv + TestUtil::next_int(&*random(), 9, 16)).min(max_bpv)
                } else {
                    (bpv + TestUtil::next_int(&*random(), 1, 8)).min(max_bpv)
                };
                values[i * FOR_BLOCK_SIZE + j] |=
                    random().next_int_bound(1 << (exception_bpv - bpv)) << bpv;
            }
        }
    }
    values
}

/// `private long encodeTestData(int iterations, int[] values, Directory d)`.
fn encode_test_data(iterations: usize, values: &[i32], d: &dyn Directory) -> Result<i64> {
    let mut out = d.create_output("test.bin", &*DEFAULT_IO_CONTEXT)?;
    let mut pfor_util = PForUtil::new();
    for i in 0..iterations {
        let mut source = [0i32; FOR_BLOCK_SIZE];
        for j in 0..FOR_BLOCK_SIZE {
            source[j] = values[i * FOR_BLOCK_SIZE + j];
        }
        pfor_util.encode(&mut source, &mut *out)?;
    }
    let end_pointer = out.file_pointer();
    out.close()?;
    Ok(end_pointer)
}
