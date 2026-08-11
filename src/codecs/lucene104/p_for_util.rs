//! Patched frame-of-reference (PFor) utility for Lucene 10.4 postings.
//!
//! Port of `org.apache.lucene.codecs.lucene104.PForUtil`. Encodes/decodes
//! blocks of 256 positive integers, optionally patching up to seven outliers
//! that exceed the bit width chosen for the bulk of the block.

use crate::error::{LuceneError, Result};
use crate::store::{DataInput, DataOutput};

use super::for_util::{ForUtil, BLOCK_SIZE};
use super::posting_decoding_util::PostingDecodingUtil;

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
            if v < 0 {
                return Err(LuceneError::IllegalArgument(
                    "PForUtil cannot encode negative values".to_string(),
                ));
            }
            let bits = 32 - (v as u32).leading_zeros() as i32;
            let bits = if v == 0 { 0 } else { bits };
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
            for i in 0..BLOCK_SIZE {
                if ints[i] as i64 > max_unpatched_value {
                    exceptions[exception_count * 2] = i as u8;
                    exceptions[exception_count * 2 + 1] =
                        ((ints[i] as u32) >> patched_bits_required) as u8;
                    ints[i] &= max_unpatched_value as i32;
                    exception_count += 1;
                }
            }
            assert_eq!(exception_count, num_exceptions as usize);
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
        let token = pdu.input.read_byte()? as i32;
        let bits_per_value = token & 0x1f;
        if bits_per_value == 0 {
            let value = pdu.input.read_v_int()?;
            ints.fill(value);
        } else {
            self.for_util.decode(bits_per_value, pdu, ints)?;
        }
        let num_exceptions = (token as u32 >> 5) as i32;
        for _ in 0..num_exceptions {
            let idx = pdu.input.read_byte()? as usize;
            let high = pdu.input.read_byte()? as i32;
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
    use crate::store::{ByteArrayDataOutput, IndexInput, MockIndexInput};

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
        let mut out = ByteArrayDataOutput::new();
        pfor.encode(&mut ints, &mut out).unwrap();

        let data = out.into_inner();
        let mut input = MockIndexInput::new(data, "pfor-roundtrip");
        let mut pdu = PostingDecodingUtil::new(&mut input);
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
        let mut out = ByteArrayDataOutput::new();
        pfor.encode(&mut original, &mut out).unwrap();

        let data = out.into_inner();
        let mut input = MockIndexInput::new(data, "pfor-all-equal");
        let mut pdu = PostingDecodingUtil::new(&mut input);
        let mut decoded = [0i32; BLOCK_SIZE];
        pfor.decode(&mut pdu, &mut decoded).unwrap();

        assert!(decoded.iter().all(|&v| v == 42));
    }

    #[test]
    fn skip_block() {
        let mut pfor = PForUtil::new();
        let mut original = [0i32; BLOCK_SIZE];
        for (i, v) in original.iter_mut().enumerate() {
            *v = (i % 100) as i32;
        }
        let mut out = ByteArrayDataOutput::new();
        pfor.encode(&mut original, &mut out).unwrap();

        // append some trailing bytes
        out.write_byte(0xAA).unwrap();
        out.write_byte(0xBB).unwrap();

        let data = out.into_inner();
        let trailing_offset = (data.len() - 2) as i64;
        let mut input = MockIndexInput::new(data, "pfor-skip");
        PForUtil::skip(&mut input).unwrap();
        assert_eq!(input.file_pointer(), trailing_offset);
        assert_eq!(input.read_byte().unwrap(), 0xAA);
        assert_eq!(input.read_byte().unwrap(), 0xBB);
    }
}
