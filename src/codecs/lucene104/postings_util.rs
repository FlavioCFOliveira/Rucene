//! Variable-length postings-block utilities.
//!
//! Port of `org.apache.lucene.codecs.lucene104.PostingsUtil`. Reads and writes
//! blocks of doc ids (and optionally frequencies) using group-varint encoding.

use crate::error::Result;
use crate::store::{DataInput, DataOutput};

/// Reads a block of up to `num` doc ids using group-varint, optionally
/// decoding frequencies.
///
/// Equivalent to `PostingsUtil.readVIntBlock`.
pub fn read_v_int_block(
    doc_in: &mut dyn DataInput,
    doc_buffer: &mut [i32],
    freq_buffer: &mut [i32],
    num: usize,
    index_has_freq: bool,
    decode_freq: bool,
) -> Result<()> {
    read_group_vints(doc_in, doc_buffer, num)?;
    if index_has_freq && decode_freq {
        for i in 0..num {
            let mut freq = doc_buffer[i] & 0x01;
            doc_buffer[i] = (doc_buffer[i] as u32 >> 1) as i32;
            if freq == 0 {
                freq = doc_in.read_v_int()?;
            }
            freq_buffer[i] = freq;
        }
    } else if index_has_freq {
        for i in 0..num {
            doc_buffer[i] = (doc_buffer[i] as u32 >> 1) as i32;
        }
    }
    Ok(())
}

/// Writes a block of `num` doc ids using group-varint, optionally encoding
/// frequencies inline in the low bit.
///
/// Equivalent to `PostingsUtil.writeVIntBlock`.
pub fn write_v_int_block(
    doc_out: &mut dyn DataOutput,
    doc_buffer: &mut [i32],
    freq_buffer: &[i32],
    num: usize,
    write_freqs: bool,
) -> Result<()> {
    if write_freqs {
        for i in 0..num {
            let low_bit = if freq_buffer[i] == 1 { 1 } else { 0 };
            doc_buffer[i] = (doc_buffer[i] << 1) | low_bit;
        }
    }
    write_group_vints(doc_out, doc_buffer, num)?;
    if write_freqs {
        for i in 0..num {
            let freq = freq_buffer[i];
            if freq != 1 {
                doc_out.write_v_int(freq)?;
            }
        }
    }
    Ok(())
}

/// Reads `limit` group-varint encoded values from `input` into `dst`.
fn read_group_vints(input: &mut dyn DataInput, dst: &mut [i32], limit: usize) -> Result<()> {
    let limit = limit.min(dst.len());
    let mut i = 0;
    while i + 4 <= limit {
        let flag = input.read_byte()? as usize;
        let n1 = flag >> 6;
        let n2 = (flag >> 4) & 0x03;
        let n3 = (flag >> 2) & 0x03;
        let n4 = flag & 0x03;
        dst[i] = read_int_in_group(input, n1)?;
        dst[i + 1] = read_int_in_group(input, n2)?;
        dst[i + 2] = read_int_in_group(input, n3)?;
        dst[i + 3] = read_int_in_group(input, n4)?;
        i += 4;
    }
    while i < limit {
        dst[i] = input.read_v_int()?;
        i += 1;
    }
    Ok(())
}

/// Reads a single value whose width is encoded as `num_bytes - 1`.
fn read_int_in_group(input: &mut dyn DataInput, num_bytes_minus_one: usize) -> Result<i32> {
    match num_bytes_minus_one {
        0 => Ok(input.read_byte()? as i32),
        1 => {
            let b0 = input.read_byte()? as i32;
            let b1 = input.read_byte()? as i32;
            Ok(b0 | (b1 << 8))
        }
        2 => {
            let b0 = input.read_byte()? as i32;
            let b1 = input.read_byte()? as i32;
            let b2 = input.read_byte()? as i32;
            Ok(b0 | (b1 << 8) | (b2 << 16))
        }
        3 => input.read_int(),
        _ => Err(crate::error::LuceneError::IllegalArgument(
            "invalid group varint byte count".to_string(),
        )),
    }
}

/// Writes `limit` values from `src` using group-varint encoding.
fn write_group_vints(out: &mut dyn DataOutput, src: &[i32], limit: usize) -> Result<()> {
    let limit = limit.min(src.len());
    let mut read_pos = 0;
    let mut scratch = [0u8; 17];
    while limit - read_pos >= 4 {
        let n1 = num_bytes(src[read_pos]) - 1;
        let n2 = num_bytes(src[read_pos + 1]) - 1;
        let n3 = num_bytes(src[read_pos + 2]) - 1;
        let n4 = num_bytes(src[read_pos + 3]) - 1;
        let flag = (n1 << 6) | (n2 << 4) | (n3 << 2) | n4;
        let mut write_pos = 0;
        scratch[write_pos] = flag as u8;
        write_pos += 1;
        write_int_in_group(src[read_pos], n1, &mut scratch, &mut write_pos);
        read_pos += 1;
        write_int_in_group(src[read_pos], n2, &mut scratch, &mut write_pos);
        read_pos += 1;
        write_int_in_group(src[read_pos], n3, &mut scratch, &mut write_pos);
        read_pos += 1;
        write_int_in_group(src[read_pos], n4, &mut scratch, &mut write_pos);
        read_pos += 1;
        out.write_bytes(&scratch, 0, write_pos)?;
    }
    while read_pos < limit {
        out.write_v_int(src[read_pos])?;
        read_pos += 1;
    }
    Ok(())
}

fn write_int_in_group(
    value: i32,
    num_bytes_minus_one: usize,
    scratch: &mut [u8; 17],
    pos: &mut usize,
) {
    let n = num_bytes_minus_one + 1;
    scratch[*pos] = value as u8;
    *pos += 1;
    if n >= 2 {
        scratch[*pos] = (value >> 8) as u8;
        *pos += 1;
    }
    if n >= 3 {
        scratch[*pos] = (value >> 16) as u8;
        *pos += 1;
    }
    if n == 4 {
        scratch[*pos] = (value >> 24) as u8;
        *pos += 1;
    }
}

fn num_bytes(v: i32) -> usize {
    let uv = (v as u32) | 1;
    4 - (uv.leading_zeros() >> 3) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ByteArrayDataOutput, MockIndexInput};

    #[test]
    fn group_v_int_round_trip() {
        let values = [0, 1, 128, 0x1000, 0x1_0000, 0x100_0000, 0x7FFF_FFFF, 42];
        let mut out = ByteArrayDataOutput::new();
        write_group_vints(&mut out, &values, values.len()).unwrap();

        let data = out.into_inner();
        let mut input = MockIndexInput::new(data, "group-vint");
        let mut decoded = [0i32; 8];
        read_group_vints(&mut input, &mut decoded, 8).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn v_int_block_round_trip_with_freqs() {
        let mut docs = [10i32, 20, 30, 40, 50];
        let freqs = [1i32, 3, 1, 1, 7];
        let mut out = ByteArrayDataOutput::new();
        write_v_int_block(&mut out, &mut docs, &freqs, 5, true).unwrap();

        let data = out.into_inner();
        let mut input = MockIndexInput::new(data, "vint-block");
        let mut read_docs = [0i32; 5];
        let mut read_freqs = [0i32; 5];
        read_v_int_block(&mut input, &mut read_docs, &mut read_freqs, 5, true, true).unwrap();

        assert_eq!(read_docs, [10, 20, 30, 40, 50]);
        assert_eq!(read_freqs, [1, 3, 1, 1, 7]);
    }

    #[test]
    fn v_int_block_skip_freqs() {
        let mut docs = [10i32, 20, 30, 40, 50];
        let freqs = [1i32, 3, 1, 1, 7];
        let mut out = ByteArrayDataOutput::new();
        write_v_int_block(&mut out, &mut docs, &freqs, 5, true).unwrap();

        let data = out.into_inner();
        let mut input = MockIndexInput::new(data, "vint-block-skip");
        let mut read_docs = [0i32; 5];
        let mut read_freqs = [0i32; 5];
        read_v_int_block(&mut input, &mut read_docs, &mut read_freqs, 5, true, false).unwrap();

        assert_eq!(read_docs, [10, 20, 30, 40, 50]);
        assert!(read_freqs.iter().all(|&v| v == 0));
    }
}
