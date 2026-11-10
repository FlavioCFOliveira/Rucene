/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.codecs.memory.FSTTermOutputs`.

use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{FieldInfo, IndexOptions};
use rucene::store::{DataInput, DataOutput};
use rucene::util::fst::Outputs;
use rucene::util::RamUsageEstimator;

/// Represents the metadata for one term. On an FST, only long[] part is
/// 'shared' and pushed towards root. byte[] and term stats will be kept on
/// deeper arcs.
///
/// Port of `FSTTermOutputs.TermData`. `bytes` is shared (`Arc`) where Java
/// shares the array between the copies `add` and `subtract` make.
#[derive(Clone, PartialEq, Eq)]
pub struct TermData {
    /// Port of `TermData.bytes`; `None` is Java's `null`.
    pub bytes: Option<Arc<[u8]>>,
    /// Port of `TermData.docFreq`.
    pub doc_freq: i32,
    /// Port of `TermData.totalTermFreq`.
    pub total_term_freq: i64,
}

/// Port of `RamUsageEstimator.shallowSizeOfInstance(TermData.class)`: the
/// object header, one reference, an `int` and a `long`, aligned.
const BASE_RAM_BYTES_USED: i64 = 32;

impl Default for TermData {
    /// Port of `TermData()`: `bytes = null; docFreq = 0; totalTermFreq = -1`,
    /// the value of `NO_OUTPUT`.
    fn default() -> Self {
        Self {
            bytes: None,
            doc_freq: 0,
            total_term_freq: -1,
        }
    }
}

impl TermData {
    /// Port of `TermData(byte[], int, long)`.
    pub fn new(bytes: Option<Arc<[u8]>>, doc_freq: i32, total_term_freq: i64) -> Self {
        Self {
            bytes,
            doc_freq,
            total_term_freq,
        }
    }

    /// Port of `TermData.ramBytesUsed()`.
    pub fn ram_bytes_used(&self) -> i64 {
        let mut ram_bytes_used = BASE_RAM_BYTES_USED;
        if let Some(bytes) = &self.bytes {
            ram_bytes_used += RamUsageEstimator::size_of(bytes);
        }
        ram_bytes_used
    }

    /// Port of `TermData.hashCode()`, in Java's `int` arithmetic.
    // NOTE: actually, FST nodes are seldom
    // identical when outputs on their arcs
    // aren't NO_OUTPUTs.
    pub fn hash_code(&self) -> i32 {
        let mut hash: i32 = 0;
        if let Some(bytes) = &self.bytes {
            for &b in bytes.iter() {
                hash = hash.wrapping_add(i32::from(b as i8));
            }
        }
        // `hash += docFreq + totalTermFreq`: an int plus a long, narrowed.
        hash = i64::from(hash)
            .wrapping_add(i64::from(self.doc_freq).wrapping_add(self.total_term_freq))
            as i32;
        hash
    }

    /// Whether this is `NO_OUTPUT`.
    ///
    /// Java compares with `NO_OUTPUT` by identity (`t1 == NO_OUTPUT`). The
    /// port compares values: `NO_OUTPUT` is the only `TermData` with
    /// `docFreq == 0`, no bytes and `totalTermFreq == -1` that these outputs
    /// ever handle, because every term's output has `docFreq >= 1` and
    /// `common`, `subtract`, `add` and `read` only ever return `NO_OUTPUT`
    /// itself or a copy of an operand that is not `NO_OUTPUT` (the test
    /// `no_output_identity_is_value_equality` exercises every case).
    fn is_no_output(&self) -> bool {
        self.bytes.is_none() && self.doc_freq == 0 && self.total_term_freq == -1
    }
}

impl fmt::Debug for TermData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for TermData {
    /// Port of `TermData.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes = match &self.bytes {
            None => "null".to_string(),
            Some(b) => format!(
                "[{}]",
                b.iter()
                    .map(|&v| (v as i8).to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        write!(
            f,
            "FSTTermOutputs$TermData bytes={bytes} docFreq={} totalTermFreq={}",
            self.doc_freq, self.total_term_freq
        )
    }
}

/// Port of `FSTTermOutputs.statsEqual`.
fn stats_equal(t1: &TermData, t2: &TermData) -> bool {
    t1.doc_freq == t2.doc_freq && t1.total_term_freq == t2.total_term_freq
}

/// Port of `FSTTermOutputs.bytesEqual`.
fn bytes_equal(t1: &TermData, t2: &TermData) -> bool {
    match (&t1.bytes, &t2.bytes) {
        (None, None) => true,
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// An FST `Outputs` implementation for
/// [`FSTTermsWriter`](super::FSTTermsWriter).
///
/// Port of the package-private `org.apache.lucene.codecs.memory.FSTTermOutputs`.
// NOTE: outputs should be per-field, since
// longsSize is fixed for each field
#[derive(Clone, Debug)]
pub struct FSTTermOutputs {
    has_pos: bool,
}

impl FSTTermOutputs {
    /// Port of `FSTTermOutputs(FieldInfo)`.
    pub fn new(field_info: &FieldInfo) -> Self {
        Self {
            has_pos: field_info.get_index_options() != IndexOptions::DOCS,
        }
    }
}

impl fmt::Display for FSTTermOutputs {
    /// `Object.toString()`, which `FSTTermOutputs` does not override.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rucene::index::index_reader::java_object_to_string(
            "org.apache.lucene.codecs.memory.FSTTermOutputs",
            self,
        ))
    }
}

impl Outputs for FSTTermOutputs {
    type Output = TermData;

    //
    // The return value will be the smaller one, when these two are
    // 'comparable', i.e.
    // 1. every value in t1 is not larger than in t2, or
    // 2. every value in t1 is not smaller than t2.
    //
    fn common(&self, t1: &TermData, t2: &TermData) -> TermData {
        if t1.is_no_output() || t2.is_no_output() {
            return TermData::default();
        }
        if stats_equal(t1, t2) && bytes_equal(t1, t2) {
            t1.clone()
        } else {
            TermData::default()
        }
    }

    fn subtract(&self, t1: &TermData, t2: &TermData) -> TermData {
        if t2.is_no_output() {
            return t1.clone();
        }
        if stats_equal(t1, t2) && bytes_equal(t1, t2) {
            TermData::default()
        } else {
            TermData::new(t1.bytes.clone(), t1.doc_freq, t1.total_term_freq)
        }
    }

    // TODO: if we refactor a 'addSelf(TermData other)',
    // we can gain about 5~7% for fuzzy queries, however this also
    // means we are putting too much stress on FST Outputs decoding?
    fn add(&self, t1: &TermData, t2: &TermData) -> TermData {
        if t1.is_no_output() {
            return t2.clone();
        } else if t2.is_no_output() {
            return t1.clone();
        }
        if t2.bytes.is_some() || t2.doc_freq > 0 {
            TermData::new(t2.bytes.clone(), t2.doc_freq, t2.total_term_freq)
        } else {
            TermData::new(t1.bytes.clone(), t1.doc_freq, t1.total_term_freq)
        }
    }

    fn write(&self, data: &TermData, out: &mut dyn DataOutput) -> Result<()> {
        debug_assert!(self.has_pos || data.total_term_freq == -1);
        let bytes_len = data.bytes.as_ref().map_or(0, |b| b.len());
        let bit0 = i32::from(bytes_len != 0);
        let bit1 = i32::from(data.doc_freq != 0) << 1;
        let mut bits = bit0 | bit1;
        if bit0 > 0 {
            // determine extra length
            if bytes_len < 32 {
                bits |= (bytes_len as i32) << 2;
                out.write_byte(bits as u8)?;
            } else {
                out.write_byte(bits as u8)?;
                out.write_v_int(bytes_len as i32)?;
            }
        } else {
            out.write_byte(bits as u8)?;
        }
        if bit0 > 0 {
            // bytes exists
            let bytes = data.bytes.as_ref().expect("INVARIANT: bit0 means bytes");
            out.write_bytes(bytes, 0, bytes.len())?;
        }
        if bit1 > 0 {
            // stats exist
            if self.has_pos {
                if i64::from(data.doc_freq) == data.total_term_freq {
                    out.write_v_int((data.doc_freq << 1) | 1)?;
                } else {
                    out.write_v_int(data.doc_freq << 1)?;
                    out.write_v_long(data.total_term_freq - i64::from(data.doc_freq))?;
                }
            } else {
                out.write_v_int(data.doc_freq)?;
            }
        }
        Ok(())
    }

    fn read(&self, input: &mut dyn DataInput) -> Result<TermData> {
        let mut bytes = None;
        let mut doc_freq = 0;
        let mut total_term_freq = -1;
        let bits = i32::from(input.read_byte()?);
        let bit0 = bits & 1;
        let bit1 = bits & 2;
        let mut bytes_size = (bits as u32 >> 2) as i32;
        if bit0 > 0 && bytes_size == 0 {
            // determine extra length
            bytes_size = input.read_v_int()?;
        }
        if bit0 > 0 {
            // bytes exists
            let mut b = vec![0u8; bytes_size as usize];
            input.read_bytes(Some(&mut b), 0, bytes_size as usize)?;
            bytes = Some(Arc::from(b));
        }
        if bit1 > 0 {
            // stats exist
            let code = input.read_v_int()?;
            if self.has_pos {
                doc_freq = (code as u32 >> 1) as i32;
                total_term_freq = i64::from(doc_freq);
                if (code & 1) == 0 {
                    total_term_freq += input.read_v_long()?;
                }
            } else {
                doc_freq = code;
            }
        }
        Ok(TermData::new(bytes, doc_freq, total_term_freq))
    }

    fn skip_output(&self, input: &mut dyn DataInput) -> Result<()> {
        let bits = i32::from(input.read_byte()?);
        let bit0 = bits & 1;
        let bit1 = bits & 2;
        let mut bytes_size = (bits as u32 >> 2) as i32;
        if bit0 > 0 && bytes_size == 0 {
            // determine extra length
            bytes_size = input.read_v_int()?;
        }
        if bit0 > 0 {
            // bytes exists
            input.skip_bytes(i64::from(bytes_size))?;
        }
        if bit1 > 0 {
            // stats exist
            let code = input.read_v_int()?;
            if self.has_pos && (code & 1) == 0 {
                input.read_v_long()?;
            }
        }
        Ok(())
    }

    fn equals(&self, a: &TermData, b: &TermData) -> bool {
        // `TermData.equals`.
        stats_equal(a, b) && bytes_equal(a, b)
    }

    fn no_output(&self) -> TermData {
        TermData::default()
    }

    fn output_to_string(&self, output: &TermData) -> String {
        output.to_string()
    }

    fn ram_bytes_used(&self, output: &TermData) -> i64 {
        output.ram_bytes_used()
    }

    fn output_hash(&self, output: &TermData) -> i64 {
        i64::from(output.hash_code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outputs(has_pos: bool) -> FSTTermOutputs {
        FSTTermOutputs { has_pos }
    }

    /// Java compares with `NO_OUTPUT` by identity; the port by value. They
    /// agree on every value these outputs handle: `NO_OUTPUT` and term
    /// outputs (`docFreq >= 1`), combined the ways the FST code combines
    /// them. `FSTCompiler.add` takes `c = common(output, arcOutput)` and then
    /// `subtract(arcOutput, c)` and `subtract(output, c)`; the readers and
    /// `prependOutput` call `add` on any two values. For each, the port's
    /// result is equal to `NO_OUTPUT` exactly when Java's is `NO_OUTPUT`
    /// itself (transcribed from `FSTTermOutputs.java`, where a result that is
    /// not `NO_OUTPUT` is an operand or a copy of one that is not either).
    ///
    /// The one call that would tell them apart, `subtract(NO_OUTPUT, t)` with
    /// `t` not `NO_OUTPUT` (Java returns a copy of `NO_OUTPUT`, which is not
    /// `NO_OUTPUT`), never happens: `c` is `NO_OUTPUT` whenever an operand
    /// is, and then `subtract(x, NO_OUTPUT)` returns `x`.
    #[test]
    fn no_output_identity_is_value_equality() {
        let o = outputs(true);
        let no = o.no_output();
        let a = TermData::new(Some(Arc::from(vec![1u8, 2])), 3, 5);
        let b = TermData::new(None, 1, 1);
        let c = TermData::new(Some(Arc::from(vec![1u8, 2])), 3, 5);
        let operands = [no.clone(), a.clone(), b.clone(), c.clone()];
        // Java's `common` is NO_OUTPUT itself unless both are terms and equal.
        let java_common_is_no =
            |t1: &TermData, t2: &TermData| t1.is_no_output() || t2.is_no_output() || t1 != t2;
        // Java's `subtract` is NO_OUTPUT itself when t2 is and t1 is, or when
        // t2 is not and the two are equal.
        let java_subtract_is_no = |t1: &TermData, t2: &TermData| {
            if t2.is_no_output() {
                t1.is_no_output()
            } else {
                t1 == t2
            }
        };
        // Java's `add` is NO_OUTPUT itself only when both are.
        let java_add_is_no = |t1: &TermData, t2: &TermData| t1.is_no_output() && t2.is_no_output();
        for output in &operands {
            for arc_output in &operands {
                let common = o.common(output, arc_output);
                assert_eq!(common.is_no_output(), java_common_is_no(output, arc_output));
                for t1 in [arc_output, output] {
                    // The only subtract the compiler makes.
                    assert!(!(t1.is_no_output() && !common.is_no_output()));
                    assert_eq!(
                        o.subtract(t1, &common).is_no_output(),
                        java_subtract_is_no(t1, &common)
                    );
                }
                assert_eq!(
                    o.add(output, arc_output).is_no_output(),
                    java_add_is_no(output, arc_output)
                );
            }
        }
        // `read` of what `write` wrote for a term is a term, never NO_OUTPUT.
        for t in [&a, &b] {
            let mut out = rucene::store::ByteBuffersDataOutput::new();
            o.write(t, &mut out).unwrap();
            let bytes = out.to_array_copy().unwrap();
            let mut input = rucene::store::ByteArrayDataInput::new(&bytes);
            let read = o.read(&mut input).unwrap();
            assert_eq!(&read, t);
            assert!(!read.is_no_output());
        }
    }

    /// `TermData.hashCode()` in Java's arithmetic: signed bytes, and the
    /// `long` sum narrowed to `int`.
    #[test]
    fn hash_code_is_javas() {
        let t = TermData::new(Some(Arc::from(vec![0xffu8, 0x7f, 0x80])), 7, 9);
        // -1 + 127 - 128 + (7 + 9)
        assert_eq!(t.hash_code(), -2 + 16);
        let big = TermData::new(None, i32::MAX, i64::from(i32::MAX) + 10);
        assert_eq!(big.hash_code(), ((i64::from(i32::MAX) * 2) + 10) as i32);
        assert_eq!(TermData::default().hash_code(), -1);
    }

    /// The flag byte, the inline and vInt byte lengths and the two stats
    /// encodings round-trip, byte for byte as `FSTTermOutputs.write`.
    #[test]
    fn write_read_skip_round_trip() {
        for has_pos in [true, false] {
            let o = outputs(has_pos);
            let ttf = |df: i64, extra: i64| if has_pos { df + extra } else { -1 };
            let cases = [
                TermData::new(None, 1, ttf(1, 0)),
                TermData::new(None, 4, ttf(4, 3)),
                TermData::new(Some(Arc::from(vec![9u8; 31])), 2, ttf(2, 0)),
                TermData::new(Some(Arc::from(vec![9u8; 32])), 2, ttf(2, 1)),
                TermData::new(Some(Arc::from(vec![1u8])), 0, -1),
            ];
            for t in cases {
                let mut out = rucene::store::ByteBuffersDataOutput::new();
                o.write(&t, &mut out).unwrap();
                let bytes = out.to_array_copy().unwrap();
                let len = t.bytes.as_ref().map_or(0, |b| b.len());
                let expected_flag = i32::from(len != 0)
                    | (i32::from(t.doc_freq != 0) << 1)
                    | if len > 0 && len < 32 {
                        (len as i32) << 2
                    } else {
                        0
                    };
                assert_eq!(i32::from(bytes[0]), expected_flag);
                let mut input = rucene::store::ByteArrayDataInput::new(&bytes);
                assert_eq!(o.read(&mut input).unwrap(), t);
                let mut skip = rucene::store::ByteArrayDataInput::new(&bytes);
                o.skip_output(&mut skip).unwrap();
                assert_eq!(skip.position(), bytes.len());
            }
        }
    }
}
