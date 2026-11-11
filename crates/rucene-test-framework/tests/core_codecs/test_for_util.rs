//! Port of `lucene/core/src/test/org/apache/lucene/codecs/lucene104/TestForUtil.java`.
//!
//! `Lucene104PostingsReader.VECTORIZATION_PROVIDER.newPostingDecodingUtil(in)`
//! is the scalar [`PostingDecodingUtil`] over `in`, the only implementation
//! the port has.

use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::reflect::Class;
use rucene::codecs::lucene104::for_util::num_bytes;
use rucene::codecs::lucene104::{ForUtil, PostingDecodingUtil, FOR_BLOCK_SIZE};
use rucene::error::Result;
use rucene::store::{ByteBuffersDirectory, Directory, DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT};
use rucene::util::packed::PackedInts;
use rucene_test_framework::util::TestUtil;

use crate::core_support::{
    arrays_to_string, assert_array_equals_msg, assert_equals, class, random, test,
};

/// `TestForUtil`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.codecs.lucene104.TestForUtil",
        vec![test("testEncodeDecode", test_encode_decode)],
    )
}

/// Port of `TestForUtil.testEncodeDecode`.
fn test_encode_decode() -> Result<()> {
    let iterations = RandomNumbers::random_int_between(&*random(), 50, 1000) as usize;
    let mut values = vec![0i32; iterations * FOR_BLOCK_SIZE];
    for i in 0..iterations {
        let bpv = TestUtil::next_int(&*random(), 1, 31);
        for j in 0..FOR_BLOCK_SIZE {
            values[i * FOR_BLOCK_SIZE + j] =
                RandomNumbers::random_int_between(&*random(), 0, PackedInts::max_value(bpv) as i32);
        }
    }
    let d = ByteBuffersDirectory::new();
    let end_pointer;
    {
        // encode
        let mut out = d.create_output("test.bin", &*DEFAULT_IO_CONTEXT)?;
        let mut for_util = ForUtil::new();
        for i in 0..iterations {
            let mut source = [0i32; FOR_BLOCK_SIZE];
            let mut or: i64 = 0;
            for j in 0..FOR_BLOCK_SIZE {
                source[j] = values[i * FOR_BLOCK_SIZE + j];
                or |= i64::from(source[j]);
            }
            let bpv = PackedInts::bits_required(or)?;
            out.write_byte(bpv as u8)?;
            for_util.encode(&mut source, bpv, &mut *out)?;
        }
        end_pointer = out.file_pointer();
        out.close()?;
    }
    {
        // decode
        let mut r#in = d.open_input("test.bin", &*READONCE_IO_CONTEXT)?;
        let mut pdu = PostingDecodingUtil::new(&mut *r#in);
        let mut for_util = ForUtil::new();
        for i in 0..iterations {
            let bits_per_value = pdu.input.read_byte()? as i8 as i32;
            let current_file_pointer = pdu.input.file_pointer();
            let mut restored = [0i32; FOR_BLOCK_SIZE];
            for_util.decode(bits_per_value, &mut pdu, &mut restored)?;
            let mut ints = [0i32; FOR_BLOCK_SIZE];
            ints[..FOR_BLOCK_SIZE].copy_from_slice(&restored[..FOR_BLOCK_SIZE]);
            assert_array_equals_msg(
                Some(&arrays_to_string(&ints)),
                &values[i * FOR_BLOCK_SIZE..(i + 1) * FOR_BLOCK_SIZE],
                &ints,
            );
            assert_equals(
                &(num_bytes(bits_per_value) as i64),
                &(pdu.input.file_pointer() - current_file_pointer),
            );
        }
        assert_equals(&end_pointer, &pdu.input.file_pointer());
        r#in.close()?;
    }
    d.close()
}
