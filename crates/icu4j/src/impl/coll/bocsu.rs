// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.BOCSU`.

use crate::jdk::{char_count, code_point_at};
use crate::util::byte_array_wrapper::ByteArrayWrapper;

/// Binary Ordered Compression Scheme for Unicode, used for the identical
/// level of sort keys.
///
/// Equivalent to `com.ibm.icu.impl.coll.BOCSU`.
#[derive(Debug)]
pub struct BOCSU;

const SLOPE_MIN_: i32 = 3;
const SLOPE_MAX_: i32 = 0xff;
const SLOPE_MIDDLE_: i32 = 0x81;
const SLOPE_TAIL_COUNT_: i32 = SLOPE_MAX_ - SLOPE_MIN_ + 1;
const SLOPE_MAX_BYTES_: i32 = 4;
const SLOPE_SINGLE_: i32 = 80;
const SLOPE_LEAD_2_: i32 = 42;
const SLOPE_LEAD_3_: i32 = 3;
const SLOPE_REACH_POS_1_: i32 = SLOPE_SINGLE_;
const SLOPE_REACH_NEG_1_: i32 = -SLOPE_SINGLE_;
const SLOPE_REACH_POS_2_: i32 = SLOPE_LEAD_2_ * SLOPE_TAIL_COUNT_ + SLOPE_LEAD_2_ - 1;
const SLOPE_REACH_NEG_2_: i32 = -SLOPE_REACH_POS_2_ - 1;
const SLOPE_REACH_POS_3_: i32 =
    SLOPE_LEAD_3_ * SLOPE_TAIL_COUNT_ * SLOPE_TAIL_COUNT_ + (SLOPE_LEAD_3_ - 1) * SLOPE_TAIL_COUNT_ + (SLOPE_TAIL_COUNT_ - 1);
const SLOPE_REACH_NEG_3_: i32 = -SLOPE_REACH_POS_3_ - 1;
const SLOPE_START_POS_2_: i32 = SLOPE_MIDDLE_ + SLOPE_SINGLE_ + 1;
const SLOPE_START_POS_3_: i32 = SLOPE_START_POS_2_ + SLOPE_LEAD_2_;
const SLOPE_START_NEG_2_: i32 = SLOPE_MIDDLE_ + SLOPE_REACH_NEG_1_;
const SLOPE_START_NEG_3_: i32 = SLOPE_START_NEG_2_ - SLOPE_LEAD_2_;

impl BOCSU {
    /// Encode the code points of a string as a sequence of byte-encoded
    /// differences (slope detection), preserving lexical order.
    ///
    /// Equivalent to `writeIdenticalLevelRun(int, CharSequence, int, int,
    /// ByteArrayWrapper)`; returns the new "prev" code point.
    pub fn write_identical_level_run(mut prev: i32, s: &[u16], mut i: i32, length: i32, sink: &mut ByteArrayWrapper) -> i32 {
        while i < length {
            // We must have capacity>=SLOPE_MAX_BYTES in case writeDiff() writes that much,
            // but we do not want to force the sink to allocate
            // for a large min_capacity because we might actually only write one byte.
            ensure_append_capacity(sink, 16, s.len() as i32 * 2);
            let size = sink.size;
            let buffer = sink.bytes.as_mut().expect("ensureAppendCapacity allocated the buffer");
            let capacity = buffer.len() as i32;
            let mut p = size;
            let last_safe = capacity - SLOPE_MAX_BYTES_;
            while i < length && p <= last_safe {
                if !(0x4e00..0xa000).contains(&prev) {
                    prev = (prev & !0x7f) - SLOPE_REACH_NEG_1_;
                } else {
                    // Unihan U+4e00..U+9fa5:
                    // double-bytes down from the upper end
                    prev = 0x9fff - SLOPE_REACH_POS_2_;
                }

                let c = code_point_at(s, i);
                i += char_count(c);
                if c == 0xfffe {
                    buffer[p as usize] = 2; // merge separator
                    p += 1;
                    prev = 0;
                } else {
                    p = write_diff(c - prev, buffer, p);
                    prev = c;
                }
            }
            sink.size = p;
        }
        prev
    }
}

fn ensure_append_capacity(sink: &mut ByteArrayWrapper, min_capacity: i32, mut desired_capacity: i32) {
    let len = sink.bytes.as_ref().map_or(0, |b| b.len() as i32);
    let remaining_capacity = len - sink.size;
    if remaining_capacity >= min_capacity {
        return;
    }
    if desired_capacity < min_capacity {
        desired_capacity = min_capacity;
    }
    let size = sink.size;
    sink.ensure_capacity(size + desired_capacity);
}

/// Integer division and modulo with negative numerators yields negative
/// modulo results and quotients that are one more than what we need here.
fn get_neg_div_mod(number: i32, factor: i32) -> i64 {
    let mut modulo = number % factor;
    let mut result = (number / factor) as i64;
    if modulo < 0 {
        result -= 1;
        modulo += factor;
    }
    (result << 32) | modulo as i64
}

/// Encode one difference value -0x10ffff..+0x10ffff in 1..4 bytes,
/// preserving lexical order.
fn write_diff(mut diff: i32, buffer: &mut [i8], mut offset: i32) -> i32 {
    let o = |offset: i32| offset as usize;
    if diff >= SLOPE_REACH_NEG_1_ {
        if diff <= SLOPE_REACH_POS_1_ {
            buffer[o(offset)] = (SLOPE_MIDDLE_ + diff) as i8;
            offset += 1;
        } else if diff <= SLOPE_REACH_POS_2_ {
            buffer[o(offset)] = (SLOPE_START_POS_2_ + (diff / SLOPE_TAIL_COUNT_)) as i8;
            offset += 1;
            buffer[o(offset)] = (SLOPE_MIN_ + (diff % SLOPE_TAIL_COUNT_)) as i8;
            offset += 1;
        } else if diff <= SLOPE_REACH_POS_3_ {
            buffer[o(offset + 2)] = (SLOPE_MIN_ + (diff % SLOPE_TAIL_COUNT_)) as i8;
            diff /= SLOPE_TAIL_COUNT_;
            buffer[o(offset + 1)] = (SLOPE_MIN_ + (diff % SLOPE_TAIL_COUNT_)) as i8;
            buffer[o(offset)] = (SLOPE_START_POS_3_ + (diff / SLOPE_TAIL_COUNT_)) as i8;
            offset += 3;
        } else {
            buffer[o(offset + 3)] = (SLOPE_MIN_ + diff % SLOPE_TAIL_COUNT_) as i8;
            diff /= SLOPE_TAIL_COUNT_;
            buffer[o(offset + 2)] = (SLOPE_MIN_ + diff % SLOPE_TAIL_COUNT_) as i8;
            diff /= SLOPE_TAIL_COUNT_;
            buffer[o(offset + 1)] = (SLOPE_MIN_ + diff % SLOPE_TAIL_COUNT_) as i8;
            buffer[o(offset)] = SLOPE_MAX_ as i8;
            offset += 4;
        }
    } else {
        let mut division = get_neg_div_mod(diff, SLOPE_TAIL_COUNT_);
        let mut modulo = division as i32;
        if diff >= SLOPE_REACH_NEG_2_ {
            diff = (division >> 32) as i32;
            buffer[o(offset)] = (SLOPE_START_NEG_2_ + diff) as i8;
            offset += 1;
            buffer[o(offset)] = (SLOPE_MIN_ + modulo) as i8;
            offset += 1;
        } else if diff >= SLOPE_REACH_NEG_3_ {
            buffer[o(offset + 2)] = (SLOPE_MIN_ + modulo) as i8;
            diff = (division >> 32) as i32;
            division = get_neg_div_mod(diff, SLOPE_TAIL_COUNT_);
            modulo = division as i32;
            diff = (division >> 32) as i32;
            buffer[o(offset + 1)] = (SLOPE_MIN_ + modulo) as i8;
            buffer[o(offset)] = (SLOPE_START_NEG_3_ + diff) as i8;
            offset += 3;
        } else {
            buffer[o(offset + 3)] = (SLOPE_MIN_ + modulo) as i8;
            diff = (division >> 32) as i32;
            division = get_neg_div_mod(diff, SLOPE_TAIL_COUNT_);
            modulo = division as i32;
            diff = (division >> 32) as i32;
            buffer[o(offset + 2)] = (SLOPE_MIN_ + modulo) as i8;
            division = get_neg_div_mod(diff, SLOPE_TAIL_COUNT_);
            modulo = division as i32;
            buffer[o(offset + 1)] = (SLOPE_MIN_ + modulo) as i8;
            buffer[o(offset)] = SLOPE_MIN_ as i8;
            offset += 4;
        }
    }
    offset
}
