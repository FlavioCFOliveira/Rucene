//! Patched frame-of-reference (PFor) utility for Lucene 10.4 postings.
//!
//! Port of `org.apache.lucene.codecs.lucene104.PForUtil`. Encodes/decodes
//! blocks of 256 positive integers, optionally patching up to seven outliers
//! that exceed the bit width chosen for the bulk of the block.

use crate::error::Result;
use crate::store::{DataInput, DataOutput};
use crate::util::packed::PackedInts;

use super::for_util::{ForUtil, BLOCK_SIZE};
use crate::internal::vectorization::PostingDecodingUtil;

// `PForUtil.java:41`: Java asserts this in its static initializer; it is a
// property of a constant, checked here at compile time.
const _: () = assert!(BLOCK_SIZE <= 256, "blocksize must fit in one byte");

/// Maximum number of values that can be patched as exceptions in a block.
const MAX_EXCEPTIONS: usize = 7;

/// Patched FOR encoder/decoder.
pub struct PForUtil {
    for_util: ForUtil,
}

impl Default for PForUtil {
    fn default() -> Self {
        Self::new()
    }
}

impl PForUtil {
    /// Creates a new PFOR utility.
    pub fn new() -> Self {
        Self {
            for_util: ForUtil::new(),
        }
    }

    /// Encodes 256 integers from `ints` into `out`.
    ///
    /// `ints` is mutated in place (outliers are masked down to the packed bit
    /// width). Callers that need the original values must save them.
    pub fn encode(&mut self, ints: &mut [i32; BLOCK_SIZE], out: &mut dyn DataOutput) -> Result<()> {
        let mut histogram = [0i32; 32];
        let mut max_bits_required = 0;
        for &v in ints.iter() {
            // `PForUtil.java:57`: `PackedInts.bitsRequired(v)` rejects a negative
            // value and returns at least 1, so a zero lands in bucket 1 and
            // `patchedBitsRequired` never drops to 0 (`PackedInts.java:803-820`).
            let bits = PackedInts::bits_required_i32(v)?;
            histogram[bits as usize] += 1;
            max_bits_required = max_bits_required.max(bits);
        }

        // Patch offset is stored on a byte, so we cannot reduce the width by
        // more than 8 bits.
        let min_bits = (max_bits_required - 8).max(0);
        let mut cumulative_exceptions = 0;
        let mut patched_bits_required = max_bits_required;
        let mut num_exceptions = 0;

        for b in (min_bits..=max_bits_required).rev() {
            if cumulative_exceptions > MAX_EXCEPTIONS as i32 {
                break;
            }
            patched_bits_required = b;
            num_exceptions = cumulative_exceptions;
            cumulative_exceptions += histogram[b as usize];
        }

        let max_unpatched_value = (1i64 << patched_bits_required) - 1;
        let mut exceptions = vec![0u8; (num_exceptions * 2) as usize];
        if num_exceptions > 0 {
            let mut exception_count = 0;
            for (i, int) in ints[..BLOCK_SIZE].iter_mut().enumerate() {
                if *int as i64 > max_unpatched_value {
                    exceptions[exception_count * 2] = i as u8;
                    exceptions[exception_count * 2 + 1] =
                        ((*int as u32) >> patched_bits_required) as u8;
                    *int &= max_unpatched_value as i32;
                    exception_count += 1;
                }
            }
            debug_assert_eq!(
                exception_count, num_exceptions as usize,
                "{exception_count} {num_exceptions}"
            );
        }

        if all_equal(ints) && max_bits_required <= 8 {
            for i in 0..num_exceptions as usize {
                exceptions[2 * i + 1] =
                    ((exceptions[2 * i + 1] as u32) << patched_bits_required) as u8;
            }
            let token = num_exceptions << 5;
            out.write_byte(token as u8)?;
            out.write_v_int(ints[0])?;
        } else {
            let token = (num_exceptions << 5) | patched_bits_required;
            out.write_byte(token as u8)?;
            self.for_util.encode(ints, patched_bits_required, out)?;
        }
        out.write_bytes(&exceptions, 0, exceptions.len())
    }

    /// Decodes 256 integers into `ints`.
    pub fn decode(
        &mut self,
        pdu: &mut PostingDecodingUtil,
        ints: &mut [i32; BLOCK_SIZE],
    ) -> Result<()> {
        let token = pdu.r#in.read_byte()? as i32;
        let bits_per_value = token & 0x1f;
        if bits_per_value == 0 {
            let value = pdu.r#in.read_v_int()?;
            ints.fill(value);
        } else {
            self.for_util.decode(bits_per_value, pdu, ints)?;
        }
        let num_exceptions = (token as u32 >> 5) as i32;
        for _ in 0..num_exceptions {
            let idx = pdu.r#in.read_byte()? as usize;
            let high = pdu.r#in.read_byte()? as i32;
            ints[idx] |= high << bits_per_value;
        }
        Ok(())
    }

    /// Skips a single PFor-encoded block.
    pub fn skip(in_: &mut dyn DataInput) -> Result<()> {
        let token = in_.read_byte()? as i32;
        let bits_per_value = token & 0x1f;
        let num_exceptions = (token as u32 >> 5) as i32;
        if bits_per_value == 0 {
            in_.read_v_long()?;
            in_.skip_bytes((num_exceptions << 1) as i64)?;
        } else {
            in_.skip_bytes(
                (super::for_util::num_bytes(bits_per_value) + (num_exceptions << 1) as usize)
                    as i64,
            )?;
        }
        Ok(())
    }
}

fn all_equal(l: &[i32; BLOCK_SIZE]) -> bool {
    for i in 1..BLOCK_SIZE {
        if l[i] != l[0] {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{IndexInput, MockIndexInput, VecDataOutput};

    #[test]
    fn round_trip_random_values() {
        let mut pfor = PForUtil::new();
        let mut original = [0i32; BLOCK_SIZE];
        for (i, v) in original.iter_mut().enumerate() {
            // deterministic distribution with some large outliers
            let x = (i * 0x9E37_79B9 + 0x5F37_5F37) & 0x7FFF_FFFF;
            *v = (x % 100_000) as i32;
        }
        // Force one outlier that needs more than 16 bits.
        original[7] = 70000;
        original[200] = 0;

        let mut ints = original;
        let mut out = VecDataOutput::new();
        pfor.encode(&mut ints, &mut out).unwrap();

        let data = out.into_inner();
        let input = MockIndexInput::new(data, "pfor-roundtrip");
        let mut pdu = PostingDecodingUtil::new(Box::new(input));
        let mut decoded = [0i32; BLOCK_SIZE];
        pfor.decode(&mut pdu, &mut decoded).unwrap();

        assert_eq!(
            &decoded[..],
            &original[..],
            "decoded values must match original PFor block"
        );
    }

    #[test]
    fn round_trip_all_equal() {
        let mut pfor = PForUtil::new();
        let mut original = [42i32; BLOCK_SIZE];
        let mut out = VecDataOutput::new();
        pfor.encode(&mut original, &mut out).unwrap();

        let data = out.into_inner();
        let input = MockIndexInput::new(data, "pfor-all-equal");
        let mut pdu = PostingDecodingUtil::new(Box::new(input));
        let mut decoded = [0i32; BLOCK_SIZE];
        pfor.decode(&mut pdu, &mut decoded).unwrap();

        assert!(decoded.iter().all(|&v| v == 42));
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Portability regression for the `.pos` byte divergence: the expected
    /// bytes were printed by Lucene 10.5.0's own `PForUtil.encode` (the
    /// `PForProbe` oracle, run against `lucene-core-10.5.0.jar`). Each case
    /// sets the listed `(index, value)` pairs in an otherwise all-zero block.
    ///
    /// Before the fix the port treated `bitsRequired(0)` as 0 and chose the
    /// all-equal encoding where Lucene writes a 1-bit block; for example the
    /// first case encoded as `20 00 79 35`.
    #[test]
    fn encode_matches_lucene_bytes() {
        let cases: &[(&[(usize, i32)], &str)] = &[
            (
                &[(121, 53)],
                "210000000000000100000000000000000000000000000000000000000000000000791a",
            ),
            (&[], "0000"),
            (
                &[(3, 1), (50, 1)],
                "010000000000000000000000020000008000000000000000000000000000000000",
            ),
            (
                &[(0, 7), (255, 200), (17, 1)],
                "4100000080000000200000000000000000000000000000000000000000000000000003ff64",
            ),
            (
                &[(121, 53), (122, 9), (7, 255)],
                "610000000000000100000001000000000000000000000000000000000000000080077f791a7a04",
            ),
        ];
        let mut pfor = PForUtil::new();
        for (set, expected) in cases {
            let mut original = [0i32; BLOCK_SIZE];
            for &(i, v) in set.iter() {
                original[i] = v;
            }
            let mut ints = original;
            let mut out = VecDataOutput::new();
            pfor.encode(&mut ints, &mut out).unwrap();
            let data = out.into_inner();
            assert_eq!(hex(&data), *expected, "block {set:?}");

            // The bytes Lucene writes decode back to the original values.
            let input = MockIndexInput::new(data, "pfor-lucene");
            let mut pdu = PostingDecodingUtil::new(Box::new(input));
            let mut decoded = [0i32; BLOCK_SIZE];
            pfor.decode(&mut pdu, &mut decoded).unwrap();
            assert_eq!(&decoded[..], &original[..], "block {set:?}");
        }
    }

    /// Lucene's decoder reads an all-equal block with exceptions (token bits
    /// 0) by OR-ing each exception unshifted (`PForUtil.java:108-121`), so the
    /// port's decoder keeps accepting that form, which earlier rucene writers
    /// produced for the first case above.
    #[test]
    fn decode_accepts_all_equal_block_with_exceptions() {
        let input = MockIndexInput::new(vec![0x20, 0x00, 0x79, 0x35], "pfor-old");
        let mut pdu = PostingDecodingUtil::new(Box::new(input));
        let mut decoded = [0i32; BLOCK_SIZE];
        PForUtil::new().decode(&mut pdu, &mut decoded).unwrap();
        let mut expected = [0i32; BLOCK_SIZE];
        expected[121] = 53;
        assert_eq!(&decoded[..], &expected[..]);
    }

    #[test]
    fn skip_block() {
        let mut pfor = PForUtil::new();
        let mut original = [0i32; BLOCK_SIZE];
        for (i, v) in original.iter_mut().enumerate() {
            *v = (i % 100) as i32;
        }
        let mut out = VecDataOutput::new();
        pfor.encode(&mut original, &mut out).unwrap();

        // append some trailing bytes
        out.write_byte(0xAA).unwrap();
        out.write_byte(0xBB).unwrap();

        let data = out.into_inner();
        let trailing_offset = (data.len() - 2) as i64;
        let mut input = MockIndexInput::new(data, "pfor-skip");
        PForUtil::skip(&mut input).unwrap();
        assert_eq!(input.file_pointer().unwrap(), trailing_offset);
        assert_eq!(input.read_byte().unwrap(), 0xAA);
        assert_eq!(input.read_byte().unwrap(), 0xBB);
    }
}
