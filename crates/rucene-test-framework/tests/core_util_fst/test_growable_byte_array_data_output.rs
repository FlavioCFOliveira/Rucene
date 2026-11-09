//! Port of `lucene/core/src/test/org/apache/lucene/util/fst/TestGrowableByteArrayDataOutput.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use rucene::error::Result;
use rucene::store::{ByteArrayDataInput, DataOutput, VecDataOutput, DEFAULT_IO_CONTEXT};
use rucene::util::fst::GrowableByteArrayDataOutput;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    arrays_to_string, assert_array_equals, assert_equals, assert_equals_msg, class, random, test,
    verbose,
};

/// `TestGrowableByteArrayDataOutput`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testRandom", test_random),
        test("testCopyBytesOnByteStore", test_copy_bytes_on_byte_store),
    ];
    class(
        "org.apache.lucene.util.fst.TestGrowableByteArrayDataOutput",
        methods,
    )
}

fn test_random() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    let max_bytes = if *TEST_NIGHTLY { 200000 } else { 20000 };
    for iter in 0..iters {
        let num_bytes = TestUtil::next_int(&*random(), 1, max_bytes) as usize;
        let mut expected = vec![0u8; num_bytes];
        let mut bytes = GrowableByteArrayDataOutput::new();
        if verbose() {
            println!("TEST: iter={iter} numBytes={num_bytes}");
        }

        let mut pos = 0usize;
        while pos < num_bytes {
            let op = random().next_int_bound(2);
            if verbose() {
                println!("  cycle pos={pos}");
            }
            match op {
                0 => {
                    // write random byte
                    let b = random().next_int_bound(256) as u8;
                    if verbose() {
                        println!("    writeByte b={}", b as i8);
                    }

                    expected[pos] = b;
                    pos += 1;
                    bytes.write_byte(b)?;
                }
                1 => {
                    // write random byte[]
                    let len = random().next_int_bound(((num_bytes - pos) as i32).min(100)) as usize;
                    let mut temp = vec![0u8; len];
                    random().next_bytes(&mut temp);
                    if verbose() {
                        let signed: Vec<i8> = temp.iter().map(|&b| b as i8).collect();
                        println!(
                            "    writeBytes len={len} bytes={}",
                            arrays_to_string(&signed)
                        );
                    }
                    expected[pos..pos + len].copy_from_slice(&temp);
                    bytes.write_bytes(&temp, 0, temp.len())?;
                    pos += len;
                }
                _ => {}
            }

            assert_equals(&pos, &bytes.position());

            if pos > 0 && random().next_int_bound(50) == 17 {
                // truncate
                let len = TestUtil::next_int(&*random(), 1, (pos as i32).min(100)) as usize;
                bytes.set_position(pos - len);
                pos -= len;
                expected[pos..pos + len].fill(0);
                if verbose() {
                    println!("    truncate len={len} newPos={pos}");
                }
            }

            if pos > 0 && random().next_int_bound(200) == 17 {
                verify(&bytes, &expected, pos)?;
            }
        }

        let bytes_to_verify = if random().next_boolean() {
            if verbose() {
                println!("TEST: save/load final bytes");
            }
            let mut dir = LuceneTestCase::new_directory();
            let d = dir.as_directory();
            let mut out = d.create_output("bytes", &*DEFAULT_IO_CONTEXT)?;
            bytes.write_to(out.as_mut())?;
            out.close()?;
            let mut input = d.open_input("bytes", &*DEFAULT_IO_CONTEXT)?;
            let mut bytes_to_verify = GrowableByteArrayDataOutput::new();
            bytes_to_verify.copy_bytes(input.as_mut(), num_bytes as i64)?;
            input.close()?;
            dir.close()?;
            bytes_to_verify
        } else {
            bytes
        };

        verify(&bytes_to_verify, &expected, num_bytes)?;
    }
    Ok(())
}

fn test_copy_bytes_on_byte_store() -> Result<()> {
    let mut bytes = vec![0u8; 1024 * 8 + 10];
    let mut bytesout = vec![0u8; bytes.len()];
    random().next_bytes(&mut bytes);
    let offset = TestUtil::next_int(&*random(), 0, 100) as usize;
    let len = bytes.len() - offset;
    let mut input = ByteArrayDataInput::with_range(&bytes, offset, len);
    let mut o = GrowableByteArrayDataOutput::new();
    o.copy_bytes(&mut input, len as i64)?;
    o.copy_to_slice(0, &mut bytesout, 0, len);
    assert_array_equals(&bytesout[0..len], &bytes[offset..offset + len]);
    Ok(())
}

/// `verify(bytes, expected, totalLength)`.
fn verify(bytes: &GrowableByteArrayDataOutput, expected: &[u8], total_length: usize) -> Result<()> {
    assert_equals(&total_length, &bytes.position());
    if total_length == 0 {
        return Ok(());
    }
    if verbose() {
        println!("  verify...");
    }

    // `bytes.writeTo(new OutputStreamDataOutput(baos))`.
    let mut baos = VecDataOutput::new();
    bytes.write_to(&mut baos)?;
    let actual = baos.into_inner();

    assert_equals(&total_length, &actual.len());

    for i in 0..total_length {
        assert_equals_msg(
            Some(&format!("byte @ index={i}")),
            &(expected[i] as i8),
            &(actual[i] as i8),
        );
    }
    Ok(())
}
