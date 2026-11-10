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

//! Port of `org.apache.lucene.codecs.blocktreeords.FSTOrdsOutputs`.

use std::fmt;

use rucene::error::Result;
use rucene::store::{DataInput, DataOutput};
use rucene::util::fst::Outputs;
use rucene::util::RamUsageEstimator;

/// One output of the terms index: the block data, `startOrd` and `endOrd`.
///
/// Port of the record `FSTOrdsOutputs.Output(BytesRef bytes, long startOrd,
/// long endOrd)`; both ords are inclusive. Java's `BytesRef` may slice an
/// array another output shares; the port owns its bytes, which holds the
/// same values (only their sharing differs, and no caller compares arrays
/// by identity).
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Output {
    bytes: Vec<u8>,
    start_ord: i64,
    end_ord: i64,
}

impl Output {
    /// The record's canonical constructor, with its two assertions.
    fn new(bytes: Vec<u8>, start_ord: i64, end_ord: i64) -> Self {
        debug_assert!(start_ord >= 0, "startOrd={start_ord}");
        debug_assert!(end_ord >= 0, "endOrd={end_ord}");
        Self {
            bytes,
            start_ord,
            end_ord,
        }
    }

    /// Port of the accessor `bytes()`.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Port of the accessor `startOrd()`.
    pub fn start_ord(&self) -> i64 {
        self.start_ord
    }

    /// Port of the accessor `endOrd()`.
    pub fn end_ord(&self) -> i64 {
        self.end_ord
    }

    /// Whether this is `NO_OUTPUT`.
    ///
    /// Java compares with `NO_OUTPUT` by identity (`inc == NO_OUTPUT`,
    /// `arc.output() != NO_OUTPUT`). The port compares values: every output
    /// is built by [`FSTOrdsOutputs::new_output`], which returns `NO_OUTPUT`
    /// itself for empty bytes and two zero ords, so `NO_OUTPUT` is the only
    /// output with that value (the test `no_output_identity_is_value_equality`
    /// exercises each operation).
    pub(crate) fn is_no_output(&self) -> bool {
        self.bytes.is_empty() && self.start_ord == 0 && self.end_ord == 0
    }
}

impl fmt::Debug for Output {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Output {
    /// Port of `Output.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let x = if self.end_ord > i64::MAX / 2 {
            i64::MAX - self.end_ord
        } else {
            debug_assert!(self.end_ord >= 0);
            -self.end_ord
        };
        write!(f, "{} to {x}", self.start_ord)
    }
}

/// A custom FST outputs implementation that stores block data (`BytesRef`),
/// long `ordStart`, long `numTerms`.
///
/// Port of the package-private
/// `org.apache.lucene.codecs.blocktreeords.FSTOrdsOutputs`.
#[derive(Clone, Copy, Debug, Default)]
pub struct FSTOrdsOutputs;

impl FSTOrdsOutputs {
    /// Port of `FSTOrdsOutputs.newOutput(BytesRef, long, long)`.
    pub fn new_output(&self, bytes: Vec<u8>, start_ord: i64, end_ord: i64) -> Output {
        if bytes.is_empty() && start_ord == 0 && end_ord == 0 {
            Output::default()
        } else {
            Output::new(bytes, start_ord, end_ord)
        }
    }
}

impl fmt::Display for FSTOrdsOutputs {
    /// `Object.toString()`, which `FSTOrdsOutputs` does not override.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rucene::index::index_reader::java_object_to_string(
            "org.apache.lucene.codecs.blocktreeords.FSTOrdsOutputs",
            self,
        ))
    }
}

impl Outputs for FSTOrdsOutputs {
    type Output = Output;

    fn common(&self, output1: &Output, output2: &Output) -> Output {
        let bytes1 = &output1.bytes;
        let bytes2 = &output2.bytes;
        let stop_at = bytes1.len().min(bytes2.len());
        let mut pos = 0;
        while pos < stop_at && bytes1[pos] == bytes2[pos] {
            pos += 1;
        }

        let prefix_bytes = if pos == 0 {
            // no common prefix
            Vec::new()
        } else if pos == bytes1.len() {
            // bytes1 is a prefix of bytes2
            bytes1.clone()
        } else if pos == bytes2.len() {
            // bytes2 is a prefix of bytes1
            bytes2.clone()
        } else {
            bytes1[..pos].to_vec()
        };

        self.new_output(
            prefix_bytes,
            output1.start_ord.min(output2.start_ord),
            output1.end_ord.min(output2.end_ord),
        )
    }

    fn subtract(&self, output: &Output, inc: &Output) -> Output {
        if inc.is_no_output() {
            // no prefix removed
            return output.clone();
        }
        debug_assert!(output.bytes.starts_with(&inc.bytes));
        let suffix = if inc.bytes.len() == output.bytes.len() {
            // entire output removed
            Vec::new()
        } else if inc.bytes.is_empty() {
            output.bytes.clone()
        } else {
            debug_assert!(
                inc.bytes.len() < output.bytes.len(),
                "inc.length={} vs output.length={}",
                inc.bytes.len(),
                output.bytes.len()
            );
            output.bytes[inc.bytes.len()..].to_vec()
        };
        debug_assert!(output.start_ord >= inc.start_ord);
        debug_assert!(output.end_ord >= inc.end_ord);
        self.new_output(
            suffix,
            output.start_ord - inc.start_ord,
            output.end_ord - inc.end_ord,
        )
    }

    fn add(&self, prefix: &Output, output: &Output) -> Output {
        if prefix.is_no_output() {
            return output.clone();
        } else if output.is_no_output() {
            return prefix.clone();
        }
        let mut bytes = Vec::with_capacity(prefix.bytes.len() + output.bytes.len());
        bytes.extend_from_slice(&prefix.bytes);
        bytes.extend_from_slice(&output.bytes);
        self.new_output(
            bytes,
            prefix.start_ord + output.start_ord,
            prefix.end_ord + output.end_ord,
        )
    }

    fn write(&self, prefix: &Output, out: &mut dyn DataOutput) -> Result<()> {
        out.write_v_int(prefix.bytes.len() as i32)?;
        out.write_bytes(&prefix.bytes, 0, prefix.bytes.len())?;
        out.write_v_long(prefix.start_ord)?;
        out.write_v_long(prefix.end_ord)
    }

    fn read(&self, input: &mut dyn DataInput) -> Result<Output> {
        let len = input.read_v_int()?;
        let bytes = if len == 0 {
            Vec::new()
        } else {
            // `new BytesRef(len)`: a negative length raises
            // `NegativeArraySizeException`.
            let len = usize::try_from(len).map_err(|_| {
                rucene::error::LuceneError::unchecked(
                    "java.lang.NegativeArraySizeException",
                    len.to_string(),
                )
            })?;
            let mut bytes = vec![0u8; len];
            input.read_bytes(Some(&mut bytes), 0, len)?;
            bytes
        };

        let start_ord = input.read_v_long()?;
        let end_ord = input.read_v_long()?;

        Ok(self.new_output(bytes, start_ord, end_ord))
    }

    fn skip_output(&self, input: &mut dyn DataInput) -> Result<()> {
        let len = input.read_v_int()?;
        input.skip_bytes(i64::from(len))?;
        input.read_v_long()?;
        input.read_v_long()?;
        Ok(())
    }

    fn skip_final_output(&self, input: &mut dyn DataInput) -> Result<()> {
        self.skip_output(input)
    }

    fn equals(&self, a: &Output, b: &Output) -> bool {
        a == b
    }

    fn no_output(&self) -> Output {
        Output::default()
    }

    fn output_to_string(&self, output: &Output) -> String {
        if (output.end_ord == 0 || output.end_ord == i64::MAX) && output.start_ord == 0 {
            String::new()
        } else {
            output.to_string()
        }
    }

    fn ram_bytes_used(&self, output: &Output) -> i64 {
        2 * RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
            + 2 * 8
            + 2 * RamUsageEstimator::NUM_BYTES_OBJECT_REF
            + RamUsageEstimator::NUM_BYTES_ARRAY_HEADER
            + 2 * 4
            + output.bytes.len() as i64
    }

    /// The record's `hashCode()`, which only `NodeHash`'s in-memory table
    /// sees: the components' hashes combined as `31 * h + c`. Java's
    /// `BytesRef.hashCode()` is seeded per JVM, so no file depends on it.
    fn output_hash(&self, output: &Output) -> i64 {
        let mut h: i64 = 0;
        for &b in &output.bytes {
            h = h.wrapping_mul(31).wrapping_add(i64::from(b));
        }
        h = h.wrapping_mul(31).wrapping_add(output.start_ord);
        h.wrapping_mul(31).wrapping_add(output.end_ord)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rucene::store::{ByteArrayDataInput, ByteBuffersDataOutput};

    const O: FSTOrdsOutputs = FSTOrdsOutputs;

    fn out(bytes: &[u8], start: i64, end: i64) -> Output {
        O.new_output(bytes.to_vec(), start, end)
    }

    /// Every operation returns `NO_OUTPUT`'s value only as `NO_OUTPUT`
    /// itself would: through `newOutput` of empty bytes and zero ords.
    #[test]
    fn no_output_identity_is_value_equality() {
        let a = out(b"ab", 1, 2);
        let b = out(b"cd", 3, 4);
        assert!(!O.common(&a, &b).is_no_output());
        assert!(O
            .common(&out(b"ab", 0, 0), &out(b"cd", 0, 9))
            .is_no_output());
        assert!(O.subtract(&a, &a).is_no_output());
        assert_eq!(O.subtract(&a, &O.no_output()), a);
        assert_eq!(O.add(&O.no_output(), &a), a);
        assert_eq!(O.add(&a, &O.no_output()), a);
        assert!(O.new_output(Vec::new(), 0, 0).is_no_output());
        assert!(!O.new_output(Vec::new(), 0, 1).is_no_output());
    }

    #[test]
    fn common_subtract_add_match_java() {
        let a = out(b"abc", 2, 10);
        let b = out(b"abd", 5, 7);
        let c = O.common(&a, &b);
        assert_eq!((c.bytes(), c.start_ord(), c.end_ord()), (&b"ab"[..], 2, 7));
        let s = O.subtract(&a, &c);
        assert_eq!((s.bytes(), s.start_ord(), s.end_ord()), (&b"c"[..], 0, 3));
        assert_eq!(O.add(&c, &s), a);
        // A prefix that is one operand.
        let p = O.common(&out(b"ab", 1, 1), &out(b"abc", 1, 1));
        assert_eq!(p.bytes(), b"ab");
    }

    #[test]
    fn write_read_skip_round_trip() {
        let a = out(b"xyz", 3, i64::MAX - 4);
        let mut buf = ByteBuffersDataOutput::new();
        O.write(&a, &mut buf).unwrap();
        O.write(&O.no_output(), &mut buf).unwrap();
        let bytes = buf.to_array_copy().unwrap();
        // vInt 3, "xyz", vLong 3, vLong (9 bytes), then 0, 0, 0.
        assert_eq!(&bytes[..5], &[3, b'x', b'y', b'z', 3]);
        let mut input = ByteArrayDataInput::new(&bytes);
        assert_eq!(O.read(&mut input).unwrap(), a);
        assert!(O.read(&mut input).unwrap().is_no_output());
        let mut input = ByteArrayDataInput::new(&bytes);
        O.skip_output(&mut input).unwrap();
        O.skip_final_output(&mut input).unwrap();
        assert!(input.eof());
    }

    #[test]
    fn to_string_matches_java() {
        assert_eq!(out(b"a", 3, i64::MAX - 7).to_string(), "3 to 7");
        assert_eq!(out(b"a", 3, 7).to_string(), "3 to -7");
        assert_eq!(O.output_to_string(&out(b"a", 0, i64::MAX)), "");
        assert_eq!(O.output_to_string(&out(b"a", 1, 2)), "1 to -2");
        assert_eq!(
            O.ram_bytes_used(&out(b"abc", 1, 2)),
            2 * 12 + 16 + 8 + 16 + 8 + 3
        );
    }
}
