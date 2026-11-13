//! Variable-length postings-block utilities.
//!
//! Port of `org.apache.lucene.codecs.lucene104.PostingsUtil`. Reads and writes
//! blocks of doc ids (and optionally frequencies) using group-varint encoding.

use crate::error::Result;
use crate::store::{DataInput, DataOutput};
use crate::util::GroupVIntUtil;

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
    GroupVIntUtil::read_group_vints_input(doc_in, doc_buffer, num)?;
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
        for doc in &mut doc_buffer[..num] {
            *doc = (*doc as u32 >> 1) as i32;
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
    doc_out.write_group_vints(doc_buffer, num)?;
    if write_freqs {
        for &freq in &freq_buffer[..num] {
            if freq != 1 {
                doc_out.write_v_int(freq)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{MockIndexInput, VecDataOutput};

    #[test]
    fn group_v_int_round_trip() {
        let values = [0, 1, 128, 0x1000, 0x1_0000, 0x100_0000, 0x7FFF_FFFF, 42];
        let mut out = VecDataOutput::new();
        out.write_group_vints(&values, values.len()).unwrap();

        let data = out.into_inner();
        let mut input = MockIndexInput::new(data, "group-vint");
        let mut decoded = [0i32; 8];
        GroupVIntUtil::read_group_vints_input(&mut input, &mut decoded, 8).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn v_int_block_round_trip_with_freqs() {
        let mut docs = [10i32, 20, 30, 40, 50];
        let freqs = [1i32, 3, 1, 1, 7];
        let mut out = VecDataOutput::new();
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
        let mut out = VecDataOutput::new();
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
