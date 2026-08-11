//! Frame-of-Reference (FOR) bit-packing utilities for Lucene 10.4 postings.
//!
//! This is a scalar Rust port of `org.apache.lucene.codecs.lucene104.ForUtil`.
//! It packs 256 integers per block using 1..32 bits per value, matching the
//! on-disk layout of Lucene Core 10.5.0.

use crate::error::Result;
use crate::store::DataOutput;

use super::posting_decoding_util::PostingDecodingUtil;

/// Number of integers in a single FOR block.
pub const BLOCK_SIZE: usize = 256;
/// `log2(BLOCK_SIZE)`.
pub const BLOCK_SIZE_LOG2: i32 = 8;

/// FOR encoder/decoder.
///
/// Equivalent to `org.apache.lucene.codecs.lucene104.ForUtil`.
pub struct ForUtil {
    tmp: [i32; BLOCK_SIZE],
}

impl Default for ForUtil {
    fn default() -> Self {
        Self::new()
    }
}

impl ForUtil {
    /// Creates a new FOR utility.
    pub fn new() -> Self {
        Self {
            tmp: [0i32; BLOCK_SIZE],
        }
    }

    /// Encodes 256 integers from `ints` into `out` using `bits_per_value` bits
    /// per value.
    ///
    /// `ints` is mutated in place as a temporary workspace and is restored
    /// before returning only for the packed primitive sizes (8/16/32 bits).
    /// Callers that need the original values after encoding must save them.
    pub fn encode(
        &mut self,
        ints: &mut [i32; BLOCK_SIZE],
        bits_per_value: i32,
        out: &mut dyn DataOutput,
    ) -> Result<()> {
        let next_primitive = if bits_per_value <= 8 {
            collapse8(ints);
            8
        } else if bits_per_value <= 16 {
            collapse16(ints);
            16
        } else {
            32
        };
        encode_inner(ints, bits_per_value, next_primitive, out, &mut self.tmp)
    }

    /// Decodes 256 integers into `ints`.
    pub fn decode(
        &mut self,
        bits_per_value: i32,
        pdu: &mut PostingDecodingUtil,
        ints: &mut [i32; BLOCK_SIZE],
    ) -> Result<()> {
        match bits_per_value {
            1 => {
                decode1(pdu, ints)?;
                expand8(ints);
            }
            2 => {
                decode2(pdu, ints)?;
                expand8(ints);
            }
            3 => {
                decode3(pdu, &mut self.tmp, ints)?;
                expand8(ints);
            }
            4 => {
                decode4(pdu, ints)?;
                expand8(ints);
            }
            5 => {
                decode5(pdu, &mut self.tmp, ints)?;
                expand8(ints);
            }
            6 => {
                decode6(pdu, &mut self.tmp, ints)?;
                expand8(ints);
            }
            7 => {
                decode7(pdu, &mut self.tmp, ints)?;
                expand8(ints);
            }
            8 => {
                decode8(pdu, ints)?;
                expand8(ints);
            }
            9 => {
                decode9(pdu, &mut self.tmp, ints)?;
                expand16(ints);
            }
            10 => {
                decode10(pdu, &mut self.tmp, ints)?;
                expand16(ints);
            }
            11 => {
                decode11(pdu, &mut self.tmp, ints)?;
                expand16(ints);
            }
            12 => {
                decode12(pdu, &mut self.tmp, ints)?;
                expand16(ints);
            }
            13 => {
                decode13(pdu, &mut self.tmp, ints)?;
                expand16(ints);
            }
            14 => {
                decode14(pdu, &mut self.tmp, ints)?;
                expand16(ints);
            }
            15 => {
                decode15(pdu, &mut self.tmp, ints)?;
                expand16(ints);
            }
            16 => {
                decode16(pdu, ints)?;
                expand16(ints);
            }
            _ => {
                decode_slow(bits_per_value, pdu, &mut self.tmp, ints)?;
            }
        }
        Ok(())
    }
}

/// Number of bytes required to encode 256 integers of `bits_per_value` bits
/// per value.
pub fn num_bytes(bits_per_value: i32) -> usize {
    (bits_per_value << BLOCK_SIZE_LOG2) as usize >> 3
}

fn encode_inner(
    ints: &mut [i32; BLOCK_SIZE],
    bits_per_value: i32,
    primitive_size: i32,
    out: &mut dyn DataOutput,
    tmp: &mut [i32; BLOCK_SIZE],
) -> Result<()> {
    let num_ints = (BLOCK_SIZE as i32 * primitive_size) >> 5;
    let num_ints_per_shift = bits_per_value << 3;

    tmp[..num_ints_per_shift as usize].fill(0);

    let mut idx: usize = 0;
    let mut shift = primitive_size - bits_per_value;
    for i in 0..num_ints_per_shift as usize {
        tmp[i] = ints[idx] << shift;
        idx += 1;
    }
    shift -= bits_per_value;
    while shift >= 0 {
        for i in 0..num_ints_per_shift as usize {
            tmp[i] |= ints[idx] << shift;
            idx += 1;
        }
        shift -= bits_per_value;
    }

    let remaining_bits_per_int = shift + bits_per_value;
    let mask_remaining_bits_per_int = match primitive_size {
        8 => MASKS8[remaining_bits_per_int as usize],
        16 => MASKS16[remaining_bits_per_int as usize],
        _ => MASKS32[remaining_bits_per_int as usize],
    };

    let mut tmp_idx: usize = 0;
    let mut remaining_bits_per_value = bits_per_value;
    while idx < num_ints as usize {
        if remaining_bits_per_value >= remaining_bits_per_int {
            remaining_bits_per_value -= remaining_bits_per_int;
            let v = (ints[idx] as u32) >> remaining_bits_per_value;
            tmp[tmp_idx] |= (v as i32) & mask_remaining_bits_per_int;
            tmp_idx += 1;
            if remaining_bits_per_value == 0 {
                idx += 1;
                remaining_bits_per_value = bits_per_value;
            }
        } else {
            let (mask1, mask2) = match primitive_size {
                8 => (
                    MASKS8[remaining_bits_per_value as usize],
                    MASKS8[(remaining_bits_per_int - remaining_bits_per_value) as usize],
                ),
                16 => (
                    MASKS16[remaining_bits_per_value as usize],
                    MASKS16[(remaining_bits_per_int - remaining_bits_per_value) as usize],
                ),
                _ => (
                    MASKS32[remaining_bits_per_value as usize],
                    MASKS32[(remaining_bits_per_int - remaining_bits_per_value) as usize],
                ),
            };
            tmp[tmp_idx] |=
                (ints[idx] & mask1) << (remaining_bits_per_int - remaining_bits_per_value);
            idx += 1;
            remaining_bits_per_value =
                bits_per_value - remaining_bits_per_int + remaining_bits_per_value;
            let v = (ints[idx] as u32) >> remaining_bits_per_value;
            tmp[tmp_idx] |= (v as i32) & mask2;
            tmp_idx += 1;
        }
    }

    out.write_ints(tmp, 0, num_ints_per_shift as usize)
}

const fn expand_mask16(mask16: i32) -> i32 {
    mask16 | (mask16 << 16)
}

const fn expand_mask8(mask8: i32) -> i32 {
    expand_mask16(mask8 | (mask8 << 8))
}

const fn mask32(bits_per_value: i32) -> i32 {
    ((1u32 << (bits_per_value as u32)) - 1) as i32
}

const fn mask16(bits_per_value: i32) -> i32 {
    expand_mask16(((1u16 << (bits_per_value as u32)) - 1) as i32)
}

const fn mask8(bits_per_value: i32) -> i32 {
    expand_mask8(((1u8 << (bits_per_value as u32)) - 1) as i32)
}

fn expand8(arr: &mut [i32; BLOCK_SIZE]) {
    for i in 0..64 {
        let l = arr[i] as u32;
        arr[i] = (l >> 24) as i32 & 0xFF;
        arr[64 + i] = (l >> 16) as i32 & 0xFF;
        arr[128 + i] = (l >> 8) as i32 & 0xFF;
        arr[192 + i] = l as i32 & 0xFF;
    }
}

fn collapse8(arr: &mut [i32; BLOCK_SIZE]) {
    for i in 0..64 {
        let a = arr[i] & 0xFF;
        let b = arr[64 + i] & 0xFF;
        let c = arr[128 + i] & 0xFF;
        let d = arr[192 + i] & 0xFF;
        arr[i] = (a << 24) | (b << 16) | (c << 8) | d;
    }
}

fn expand16(arr: &mut [i32; BLOCK_SIZE]) {
    for i in 0..128 {
        let l = arr[i] as u32;
        arr[i] = (l >> 16) as i32 & 0xFFFF;
        arr[128 + i] = l as i32 & 0xFFFF;
    }
}

fn collapse16(arr: &mut [i32; BLOCK_SIZE]) {
    for i in 0..128 {
        let a = arr[i] & 0xFFFF;
        let b = arr[128 + i] & 0xFFFF;
        arr[i] = (a << 16) | b;
    }
}

fn decode_slow(
    bits_per_value: i32,
    pdu: &mut PostingDecodingUtil,
    tmp: &mut [i32; BLOCK_SIZE],
    ints: &mut [i32; BLOCK_SIZE],
) -> Result<()> {
    let num_ints = (bits_per_value << 3) as usize;
    let mask = MASKS32[bits_per_value as usize];
    pdu.split_ints(
        num_ints as i32,
        ints,
        32 - bits_per_value,
        32,
        mask,
        tmp,
        0,
        -1,
    )?;

    let remaining_bits_per_int = 32 - bits_per_value;
    let mask_remaining_bits_per_int = MASKS32[remaining_bits_per_int as usize];

    let mut tmp_idx: usize = 0;
    let mut remaining_bits = remaining_bits_per_int;
    for ints_idx in num_ints..BLOCK_SIZE {
        let mut b = bits_per_value - remaining_bits;
        let v = (tmp[tmp_idx] & MASKS32[remaining_bits as usize]) as u32;
        let mut l = (v << b) as i32;
        tmp_idx += 1;
        while b >= remaining_bits_per_int {
            b -= remaining_bits_per_int;
            let v2 = (tmp[tmp_idx] & mask_remaining_bits_per_int) as u32;
            l |= (v2 << b) as i32;
            tmp_idx += 1;
        }
        if b > 0 {
            let v3 = (tmp[tmp_idx] as u32) >> (remaining_bits_per_int - b);
            l |= (v3 & ((1u32 << b) - 1)) as i32;
            remaining_bits = remaining_bits_per_int - b;
        } else {
            remaining_bits = remaining_bits_per_int;
        }
        ints[ints_idx] = l;
    }
    Ok(())
}

static MASKS8: [i32; 8] = {
    let mut arr = [0i32; 8];
    let mut i = 0;
    while i < 8 {
        arr[i] = mask8(i as i32);
        i += 1;
    }
    arr
};

static MASKS16: [i32; 16] = {
    let mut arr = [0i32; 16];
    let mut i = 0;
    while i < 16 {
        arr[i] = mask16(i as i32);
        i += 1;
    }
    arr
};

static MASKS32: [i32; 32] = {
    let mut arr = [0i32; 32];
    let mut i = 0;
    while i < 32 {
        arr[i] = mask32(i as i32);
        i += 1;
    }
    arr
};

macro_rules! mask_consts8 {
    ($($name:ident = $idx:literal),* $(,)?) => {
        $(
            const $name: i32 = MASKS8[$idx];
        )*
    };
}

macro_rules! mask_consts16 {
    ($($name:ident = $idx:literal),* $(,)?) => {
        $(
            const $name: i32 = MASKS16[$idx];
        )*
    };
}

macro_rules! mask_consts32 {
    ($($name:ident = $idx:literal),* $(,)?) => {
        $(
            const $name: i32 = MASKS32[$idx];
        )*
    };
}

mask_consts8! {
    MASK8_1 = 1, MASK8_2 = 2, MASK8_3 = 3, MASK8_4 = 4,
    MASK8_5 = 5, MASK8_6 = 6, MASK8_7 = 7,
}

mask_consts16! {
    MASK16_1 = 1, MASK16_2 = 2, MASK16_3 = 3, MASK16_4 = 4,
    MASK16_5 = 5, MASK16_6 = 6, MASK16_7 = 7, MASK16_8 = 8,
    MASK16_9 = 9, MASK16_10 = 10, MASK16_11 = 11, MASK16_12 = 12,
    MASK16_13 = 13, MASK16_14 = 14, MASK16_15 = 15,
}

mask_consts32! {
    MASK32_1 = 1, MASK32_2 = 2, MASK32_3 = 3, MASK32_4 = 4,
    MASK32_5 = 5, MASK32_6 = 6, MASK32_7 = 7, MASK32_8 = 8,
    MASK32_9 = 9, MASK32_10 = 10, MASK32_11 = 11, MASK32_12 = 12,
    MASK32_13 = 13, MASK32_14 = 14, MASK32_15 = 15, MASK32_16 = 16,
}

fn decode1(pdu: &mut PostingDecodingUtil, ints: &mut [i32; BLOCK_SIZE]) -> Result<()> {
    let (b, c) = ints.split_at_mut(56);
    pdu.split_ints(8, b, 7, 1, MASK8_1, c, 0, MASK8_1)
}

fn decode2(pdu: &mut PostingDecodingUtil, ints: &mut [i32; BLOCK_SIZE]) -> Result<()> {
    let (b, c) = ints.split_at_mut(48);
    pdu.split_ints(16, b, 6, 2, MASK8_2, c, 0, MASK8_2)
}

fn decode3(
    pdu: &mut PostingDecodingUtil,
    tmp: &mut [i32; BLOCK_SIZE],
    ints: &mut [i32; BLOCK_SIZE],
) -> Result<()> {
    pdu.split_ints(24, ints, 5, 3, MASK8_3, tmp, 0, MASK8_2)?;
    let mut tmp_idx = 0;
    let mut ints_idx = 48;
    for _ in 0..8 {
        let mut l0 = tmp[tmp_idx] << 1;
        l0 |= (tmp[tmp_idx + 1] as u32 >> 1) as i32 & MASK8_1;
        ints[ints_idx] = l0;
        let mut l1 = (tmp[tmp_idx + 1] & MASK8_1) << 2;
        l1 |= tmp[tmp_idx + 2];
        ints[ints_idx + 1] = l1;
        tmp_idx += 3;
        ints_idx += 2;
    }
    Ok(())
}

fn decode4(pdu: &mut PostingDecodingUtil, ints: &mut [i32; BLOCK_SIZE]) -> Result<()> {
    let (b, c) = ints.split_at_mut(32);
    pdu.split_ints(32, b, 4, 4, MASK8_4, c, 0, MASK8_4)
}

fn decode5(
    pdu: &mut PostingDecodingUtil,
    tmp: &mut [i32; BLOCK_SIZE],
    ints: &mut [i32; BLOCK_SIZE],
) -> Result<()> {
    pdu.split_ints(40, ints, 3, 5, MASK8_5, tmp, 0, MASK8_3)?;
    let mut tmp_idx = 0;
    let mut ints_idx = 40;
    for _ in 0..8 {
        let mut l0 = tmp[tmp_idx] << 2;
        l0 |= (tmp[tmp_idx + 1] as u32 >> 1) as i32 & MASK8_2;
        ints[ints_idx] = l0;
        let mut l1 = (tmp[tmp_idx + 1] & MASK8_1) << 4;
        l1 |= tmp[tmp_idx + 2] << 1;
        l1 |= (tmp[tmp_idx + 3] as u32 >> 2) as i32 & MASK8_1;
        ints[ints_idx + 1] = l1;
        let mut l2 = (tmp[tmp_idx + 3] & MASK8_2) << 3;
        l2 |= tmp[tmp_idx + 4];
        ints[ints_idx + 2] = l2;
        tmp_idx += 5;
        ints_idx += 3;
    }
    Ok(())
}

fn decode6(
    pdu: &mut PostingDecodingUtil,
    tmp: &mut [i32; BLOCK_SIZE],
    ints: &mut [i32; BLOCK_SIZE],
) -> Result<()> {
    pdu.split_ints(48, ints, 2, 6, MASK8_6, tmp, 0, MASK8_2)?;
    let mut tmp_idx = 0;
    let mut ints_idx = 48;
    for _ in 0..16 {
        let mut l0 = tmp[tmp_idx] << 4;
        l0 |= tmp[tmp_idx + 1] << 2;
        l0 |= tmp[tmp_idx + 2];
        ints[ints_idx] = l0;
        tmp_idx += 3;
        ints_idx += 1;
    }
    Ok(())
}

fn decode7(
    pdu: &mut PostingDecodingUtil,
    tmp: &mut [i32; BLOCK_SIZE],
    ints: &mut [i32; BLOCK_SIZE],
) -> Result<()> {
    pdu.split_ints(56, ints, 1, 7, MASK8_7, tmp, 0, MASK8_1)?;
    let mut tmp_idx = 0;
    let mut ints_idx = 56;
    for _ in 0..8 {
        let mut l0 = tmp[tmp_idx] << 6;
        l0 |= tmp[tmp_idx + 1] << 5;
        l0 |= tmp[tmp_idx + 2] << 4;
        l0 |= tmp[tmp_idx + 3] << 3;
        l0 |= tmp[tmp_idx + 4] << 2;
        l0 |= tmp[tmp_idx + 5] << 1;
        l0 |= tmp[tmp_idx + 6];
        ints[ints_idx] = l0;
        tmp_idx += 7;
        ints_idx += 1;
    }
    Ok(())
}

fn decode8(pdu: &mut PostingDecodingUtil, ints: &mut [i32; BLOCK_SIZE]) -> Result<()> {
    pdu.read_ints(ints, 0, 64)
}

fn decode9(
    pdu: &mut PostingDecodingUtil,
    tmp: &mut [i32; BLOCK_SIZE],
    ints: &mut [i32; BLOCK_SIZE],
) -> Result<()> {
    pdu.split_ints(72, ints, 7, 9, MASK16_9, tmp, 0, MASK16_7)?;
    let mut tmp_idx = 0;
    let mut ints_idx = 72;
    for _ in 0..8 {
        let mut l0 = tmp[tmp_idx] << 2;
        l0 |= (tmp[tmp_idx + 1] as u32 >> 5) as i32 & MASK16_2;
        ints[ints_idx] = l0;
        let mut l1 = (tmp[tmp_idx + 1] & MASK16_5) << 4;
        l1 |= (tmp[tmp_idx + 2] as u32 >> 3) as i32 & MASK16_4;
        ints[ints_idx + 1] = l1;
        let mut l2 = (tmp[tmp_idx + 2] & MASK16_3) << 6;
        l2 |= (tmp[tmp_idx + 3] as u32 >> 1) as i32 & MASK16_6;
        ints[ints_idx + 2] = l2;
        let mut l3 = (tmp[tmp_idx + 3] & MASK16_1) << 8;
        l3 |= tmp[tmp_idx + 4] << 1;
        l3 |= (tmp[tmp_idx + 5] as u32 >> 6) as i32 & MASK16_1;
        ints[ints_idx + 3] = l3;
        let mut l4 = (tmp[tmp_idx + 5] & MASK16_6) << 3;
        l4 |= (tmp[tmp_idx + 6] as u32 >> 4) as i32 & MASK16_3;
        ints[ints_idx + 4] = l4;
        let mut l5 = (tmp[tmp_idx + 6] & MASK16_4) << 5;
        l5 |= (tmp[tmp_idx + 7] as u32 >> 2) as i32 & MASK16_5;
        ints[ints_idx + 5] = l5;
        let mut l6 = (tmp[tmp_idx + 7] & MASK16_2) << 7;
        l6 |= tmp[tmp_idx + 8];
        ints[ints_idx + 6] = l6;
        tmp_idx += 9;
        ints_idx += 7;
    }
    Ok(())
}

fn decode10(
    pdu: &mut PostingDecodingUtil,
    tmp: &mut [i32; BLOCK_SIZE],
    ints: &mut [i32; BLOCK_SIZE],
) -> Result<()> {
    pdu.split_ints(80, ints, 6, 10, MASK16_10, tmp, 0, MASK16_6)?;
    let mut tmp_idx = 0;
    let mut ints_idx = 80;
    for _ in 0..16 {
        let mut l0 = tmp[tmp_idx] << 4;
        l0 |= (tmp[tmp_idx + 1] as u32 >> 2) as i32 & MASK16_4;
        ints[ints_idx] = l0;
        let mut l1 = (tmp[tmp_idx + 1] & MASK16_2) << 8;
        l1 |= tmp[tmp_idx + 2] << 2;
        l1 |= (tmp[tmp_idx + 3] as u32 >> 4) as i32 & MASK16_2;
        ints[ints_idx + 1] = l1;
        let mut l2 = (tmp[tmp_idx + 3] & MASK16_4) << 6;
        l2 |= tmp[tmp_idx + 4];
        ints[ints_idx + 2] = l2;
        tmp_idx += 5;
        ints_idx += 3;
    }
    Ok(())
}

fn decode11(
    pdu: &mut PostingDecodingUtil,
    tmp: &mut [i32; BLOCK_SIZE],
    ints: &mut [i32; BLOCK_SIZE],
) -> Result<()> {
    pdu.split_ints(88, ints, 5, 11, MASK16_11, tmp, 0, MASK16_5)?;
    let mut tmp_idx = 0;
    let mut ints_idx = 88;
    for _ in 0..8 {
        let mut l0 = tmp[tmp_idx] << 6;
        l0 |= tmp[tmp_idx + 1] << 1;
        l0 |= (tmp[tmp_idx + 2] as u32 >> 4) as i32 & MASK16_1;
        ints[ints_idx] = l0;
        let mut l1 = (tmp[tmp_idx + 2] & MASK16_4) << 7;
        l1 |= tmp[tmp_idx + 3] << 2;
        l1 |= (tmp[tmp_idx + 4] as u32 >> 3) as i32 & MASK16_2;
        ints[ints_idx + 1] = l1;
        let mut l2 = (tmp[tmp_idx + 4] & MASK16_3) << 8;
        l2 |= tmp[tmp_idx + 5] << 3;
        l2 |= (tmp[tmp_idx + 6] as u32 >> 2) as i32 & MASK16_3;
        ints[ints_idx + 2] = l2;
        let mut l3 = (tmp[tmp_idx + 6] & MASK16_2) << 9;
        l3 |= tmp[tmp_idx + 7] << 4;
        l3 |= (tmp[tmp_idx + 8] as u32 >> 1) as i32 & MASK16_4;
        ints[ints_idx + 3] = l3;
        let mut l4 = (tmp[tmp_idx + 8] & MASK16_1) << 10;
        l4 |= tmp[tmp_idx + 9] << 5;
        l4 |= tmp[tmp_idx + 10];
        ints[ints_idx + 4] = l4;
        tmp_idx += 11;
        ints_idx += 5;
    }
    Ok(())
}

fn decode12(
    pdu: &mut PostingDecodingUtil,
    tmp: &mut [i32; BLOCK_SIZE],
    ints: &mut [i32; BLOCK_SIZE],
) -> Result<()> {
    pdu.split_ints(96, ints, 4, 12, MASK16_12, tmp, 0, MASK16_4)?;
    let mut tmp_idx = 0;
    let mut ints_idx = 96;
    for _ in 0..32 {
        let mut l0 = tmp[tmp_idx] << 8;
        l0 |= tmp[tmp_idx + 1] << 4;
        l0 |= tmp[tmp_idx + 2];
        ints[ints_idx] = l0;
        tmp_idx += 3;
        ints_idx += 1;
    }
    Ok(())
}

fn decode13(
    pdu: &mut PostingDecodingUtil,
    tmp: &mut [i32; BLOCK_SIZE],
    ints: &mut [i32; BLOCK_SIZE],
) -> Result<()> {
    pdu.split_ints(104, ints, 3, 13, MASK16_13, tmp, 0, MASK16_3)?;
    let mut tmp_idx = 0;
    let mut ints_idx = 104;
    for _ in 0..8 {
        let mut l0 = tmp[tmp_idx] << 10;
        l0 |= tmp[tmp_idx + 1] << 7;
        l0 |= tmp[tmp_idx + 2] << 4;
        l0 |= tmp[tmp_idx + 3] << 1;
        l0 |= (tmp[tmp_idx + 4] as u32 >> 2) as i32 & MASK16_1;
        ints[ints_idx] = l0;
        let mut l1 = (tmp[tmp_idx + 4] & MASK16_2) << 11;
        l1 |= tmp[tmp_idx + 5] << 8;
        l1 |= tmp[tmp_idx + 6] << 5;
        l1 |= tmp[tmp_idx + 7] << 2;
        l1 |= (tmp[tmp_idx + 8] as u32 >> 1) as i32 & MASK16_2;
        ints[ints_idx + 1] = l1;
        let mut l2 = (tmp[tmp_idx + 8] & MASK16_1) << 12;
        l2 |= tmp[tmp_idx + 9] << 9;
        l2 |= tmp[tmp_idx + 10] << 6;
        l2 |= tmp[tmp_idx + 11] << 3;
        l2 |= tmp[tmp_idx + 12];
        ints[ints_idx + 2] = l2;
        tmp_idx += 13;
        ints_idx += 3;
    }
    Ok(())
}

fn decode14(
    pdu: &mut PostingDecodingUtil,
    tmp: &mut [i32; BLOCK_SIZE],
    ints: &mut [i32; BLOCK_SIZE],
) -> Result<()> {
    pdu.split_ints(112, ints, 2, 14, MASK16_14, tmp, 0, MASK16_2)?;
    let mut tmp_idx = 0;
    let mut ints_idx = 112;
    for _ in 0..16 {
        let mut l0 = tmp[tmp_idx] << 12;
        l0 |= tmp[tmp_idx + 1] << 10;
        l0 |= tmp[tmp_idx + 2] << 8;
        l0 |= tmp[tmp_idx + 3] << 6;
        l0 |= tmp[tmp_idx + 4] << 4;
        l0 |= tmp[tmp_idx + 5] << 2;
        l0 |= tmp[tmp_idx + 6];
        ints[ints_idx] = l0;
        tmp_idx += 7;
        ints_idx += 1;
    }
    Ok(())
}

fn decode15(
    pdu: &mut PostingDecodingUtil,
    tmp: &mut [i32; BLOCK_SIZE],
    ints: &mut [i32; BLOCK_SIZE],
) -> Result<()> {
    pdu.split_ints(120, ints, 1, 15, MASK16_15, tmp, 0, MASK16_1)?;
    let mut tmp_idx = 0;
    let mut ints_idx = 120;
    for _ in 0..8 {
        let mut l0 = tmp[tmp_idx] << 14;
        l0 |= tmp[tmp_idx + 1] << 13;
        l0 |= tmp[tmp_idx + 2] << 12;
        l0 |= tmp[tmp_idx + 3] << 11;
        l0 |= tmp[tmp_idx + 4] << 10;
        l0 |= tmp[tmp_idx + 5] << 9;
        l0 |= tmp[tmp_idx + 6] << 8;
        l0 |= tmp[tmp_idx + 7] << 7;
        l0 |= tmp[tmp_idx + 8] << 6;
        l0 |= tmp[tmp_idx + 9] << 5;
        l0 |= tmp[tmp_idx + 10] << 4;
        l0 |= tmp[tmp_idx + 11] << 3;
        l0 |= tmp[tmp_idx + 12] << 2;
        l0 |= tmp[tmp_idx + 13] << 1;
        l0 |= tmp[tmp_idx + 14];
        ints[ints_idx] = l0;
        tmp_idx += 15;
        ints_idx += 1;
    }
    Ok(())
}

fn decode16(pdu: &mut PostingDecodingUtil, ints: &mut [i32; BLOCK_SIZE]) -> Result<()> {
    pdu.read_ints(ints, 0, 128)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ByteArrayDataOutput, MockIndexInput};

    fn round_trip_bits(bits_per_value: i32) {
        let mut for_util = ForUtil::new();
        let mut out = ByteArrayDataOutput::new();

        let max_value: i32 = ((1i64 << bits_per_value) - 1) as i32;
        let mut original = [0i32; BLOCK_SIZE];
        for (i, v) in original.iter_mut().enumerate() {
            // deterministic pseudo-random values that fit in the bit width
            let mut x = ((i * 31 + 7) & 0x7FFF_FFFF) as i64;
            if bits_per_value < 31 {
                x %= 1i64 << bits_per_value;
            }
            *v = x.min(max_value as i64) as i32;
        }
        // Ensure the max value is represented at least once for the edge case.
        original[0] = max_value;

        let mut encoded = original;
        for_util
            .encode(&mut encoded, bits_per_value, &mut out)
            .unwrap();

        let data = out.into_inner();
        assert_eq!(data.len(), num_bytes(bits_per_value));

        let mut input = MockIndexInput::new(data, "for-util-roundtrip");
        let mut pdu = PostingDecodingUtil::new(&mut input);
        let mut decoded = [0i32; BLOCK_SIZE];
        for_util
            .decode(bits_per_value, &mut pdu, &mut decoded)
            .unwrap();

        assert_eq!(
            &decoded[..],
            &original[..],
            "round-trip failed for {bits_per_value} bits"
        );
    }

    #[test]
    fn round_trip_all_bits() {
        for bits in 1..=31 {
            round_trip_bits(bits);
        }
    }

    #[test]
    fn num_bytes_matches_layout() {
        assert_eq!(num_bytes(1), 32);
        assert_eq!(num_bytes(8), 256);
        assert_eq!(num_bytes(16), 512);
        assert_eq!(num_bytes(32), 1024);
    }
}
