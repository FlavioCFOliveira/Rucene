//! Port of `org.apache.lucene.util.fst.ByteSequenceOutputs`.

use std::fmt;

use crate::error::Result;
use crate::store::{DataInput, DataOutput};
use crate::util::{BytesRef, RamUsageEstimator, StringHelper};

use super::outputs::Outputs;

/// Shallow size of a `BytesRef`, mirroring
/// `RamUsageEstimator.shallowSizeOf(NO_OUTPUT)` in Lucene.
const BASE_NUM_BYTES: i64 = 32;

/// An FST [`Outputs`] implementation where each output is a sequence of bytes.
///
/// Equivalent to `org.apache.lucene.util.fst.ByteSequenceOutputs`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ByteSequenceOutputs;

impl ByteSequenceOutputs {
    /// Returns the singleton instance.
    ///
    /// Equivalent to `ByteSequenceOutputs.getSingleton`.
    pub fn get_singleton() -> Self {
        Self
    }
}

impl fmt::Display for ByteSequenceOutputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ByteSequenceOutputs")
    }
}

/// Returns the index of the first differing element of the two slices, or `-1`
/// when they are equal.
///
/// Equivalent to `java.util.Arrays.mismatch` over the active ranges.
pub(crate) fn mismatch<T: PartialEq>(a: &[T], b: &[T]) -> i32 {
    let n = a.len().min(b.len());
    for i in 0..n {
        if a[i] != b[i] {
            return i as i32;
        }
    }
    if a.len() == b.len() {
        -1
    } else {
        n as i32
    }
}

impl Outputs for ByteSequenceOutputs {
    type Output = BytesRef;

    fn common(&self, output1: &BytesRef, output2: &BytesRef) -> BytesRef {
        let mismatch_pos = mismatch(output1.slice(), output2.slice());

        if mismatch_pos == 0 {
            // No common prefix.
            self.no_output()
        } else if mismatch_pos == -1 {
            // Exactly equal.
            output1.clone()
        } else if mismatch_pos as usize == output1.length {
            // output1 is a prefix of output2.
            output1.clone()
        } else if mismatch_pos as usize == output2.length {
            // output2 is a prefix of output1.
            output2.clone()
        } else {
            BytesRef::new(output1.slice()[..mismatch_pos as usize].to_vec())
        }
    }

    fn subtract(&self, output: &BytesRef, inc: &BytesRef) -> BytesRef {
        if inc.length == 0 {
            // No prefix removed.
            output.clone()
        } else if inc.length == output.length {
            // The entire output was removed.
            self.no_output()
        } else {
            debug_assert!(inc.length < output.length);
            BytesRef::new(output.slice()[inc.length..].to_vec())
        }
    }

    fn add(&self, prefix: &BytesRef, output: &BytesRef) -> BytesRef {
        if prefix.length == 0 {
            output.clone()
        } else if output.length == 0 {
            prefix.clone()
        } else {
            let mut result = Vec::with_capacity(prefix.length + output.length);
            result.extend_from_slice(prefix.slice());
            result.extend_from_slice(output.slice());
            BytesRef::new(result)
        }
    }

    fn write(&self, prefix: &BytesRef, out: &mut dyn DataOutput) -> Result<()> {
        out.write_v_int(prefix.length as i32)?;
        out.write_bytes(&prefix.bytes, prefix.offset, prefix.length)
    }

    fn read(&self, input: &mut dyn DataInput) -> Result<BytesRef> {
        let len = input.read_v_int()?;
        if len == 0 {
            Ok(self.no_output())
        } else {
            let len = usize::try_from(len).map_err(|_| {
                crate::error::LuceneError::CorruptIndex(format!("invalid output length {len}"))
            })?;
            let mut bytes = vec![0u8; len];
            input.read_bytes(&mut bytes, 0, len)?;
            Ok(BytesRef::new(bytes))
        }
    }

    fn skip_output(&self, input: &mut dyn DataInput) -> Result<()> {
        let len = input.read_v_int()?;
        if len != 0 {
            input.skip_bytes(i64::from(len))?;
        }
        Ok(())
    }

    fn equals(&self, a: &BytesRef, b: &BytesRef) -> bool {
        a.slice() == b.slice()
    }

    fn no_output(&self) -> BytesRef {
        BytesRef::default()
    }

    fn output_to_string(&self, output: &BytesRef) -> String {
        output.to_string()
    }

    fn ram_bytes_used(&self, output: &BytesRef) -> i64 {
        BASE_NUM_BYTES + RamUsageEstimator::size_of(&output.bytes)
    }

    fn output_hash(&self, output: &BytesRef) -> i64 {
        i64::from(StringHelper::murmurhash3_x86_32(
            &output.bytes,
            output.offset as i32,
            output.length as i32,
            0,
        ))
    }
}
