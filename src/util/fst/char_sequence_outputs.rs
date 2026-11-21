//! Port of `org.apache.lucene.util.fst.CharSequenceOutputs`.

use std::fmt;

use crate::error::Result;
use crate::store::{DataInput, DataOutput};
use crate::util::{CharsRef, RamUsageEstimator};

use super::byte_sequence_outputs::mismatch;
use super::outputs::Outputs;

/// Shallow size of a `CharsRef`, mirroring
/// `RamUsageEstimator.shallowSizeOf(NO_OUTPUT)` in Lucene.
const BASE_NUM_BYTES: i64 = 32;

/// An FST [`Outputs`] implementation where each output is a sequence of
/// characters.
///
/// Equivalent to `org.apache.lucene.util.fst.CharSequenceOutputs`.
///
/// Outputs are [`CharsRef`]s of UTF-16 code units, as Java's `char[]`;
/// [`Outputs::write`] emits the count, then one `VInt` per code unit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CharSequenceOutputs;

impl CharSequenceOutputs {
    /// Returns the singleton instance.
    ///
    /// Equivalent to `CharSequenceOutputs.getSingleton`.
    pub fn get_singleton() -> Self {
        Self
    }
}

impl fmt::Display for CharSequenceOutputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CharSequenceOutputs")
    }
}

impl Outputs for CharSequenceOutputs {
    type Output = CharsRef;

    // Lucene enumerates the four prefix cases separately, two of which
    // return `output1`; keeping them apart matches the Java source.
    #[allow(clippy::if_same_then_else)]
    fn common(&self, output1: &CharsRef, output2: &CharsRef) -> CharsRef {
        let mismatch_pos = mismatch(output1.chars(), output2.chars());

        if mismatch_pos == 0 {
            self.no_output()
        } else if mismatch_pos == -1 {
            output1.clone()
        } else if mismatch_pos as usize == output1.length {
            output1.clone()
        } else if mismatch_pos as usize == output2.length {
            output2.clone()
        } else {
            CharsRef::from_chars(output1.chars()[..mismatch_pos as usize].to_vec())
        }
    }

    fn subtract(&self, output: &CharsRef, inc: &CharsRef) -> CharsRef {
        if inc.length == 0 {
            output.clone()
        } else if inc.length == output.length {
            self.no_output()
        } else {
            debug_assert!(
                inc.length < output.length,
                "inc.length={} vs output.length={}",
                inc.length,
                output.length
            );
            CharsRef::from_chars(output.chars()[inc.length..].to_vec())
        }
    }

    fn add(&self, prefix: &CharsRef, output: &CharsRef) -> CharsRef {
        if prefix.length == 0 {
            output.clone()
        } else if output.length == 0 {
            prefix.clone()
        } else {
            let mut result = Vec::with_capacity(prefix.length + output.length);
            result.extend_from_slice(prefix.chars());
            result.extend_from_slice(output.chars());
            CharsRef::from_chars(result)
        }
    }

    fn write(&self, prefix: &CharsRef, out: &mut dyn DataOutput) -> Result<()> {
        out.write_v_int(prefix.length as i32)?;
        // TODO(Lucene): maybe UTF8?
        for c in prefix.chars() {
            out.write_v_int(i32::from(*c))?;
        }
        Ok(())
    }

    fn read(&self, input: &mut dyn DataInput) -> Result<CharsRef> {
        let len = input.read_v_int()?;
        if len == 0 {
            return Ok(self.no_output());
        }
        // `new CharsRef(len)`: `new char[len]`.
        let mut chars = super::java_new_array::<u16>(len)?;
        for c in chars.iter_mut() {
            // Java's `(char) in.readVInt()` keeps the low 16 bits.
            *c = input.read_v_int()? as u16;
        }
        Ok(CharsRef::from_chars(chars))
    }

    fn skip_output(&self, input: &mut dyn DataInput) -> Result<()> {
        let len = input.read_v_int()?;
        for _ in 0..len {
            input.read_v_int()?;
        }
        Ok(())
    }

    fn equals(&self, a: &CharsRef, b: &CharsRef) -> bool {
        a.chars() == b.chars()
    }

    fn no_output(&self) -> CharsRef {
        CharsRef::default()
    }

    fn output_to_string(&self, output: &CharsRef) -> String {
        output.to_string()
    }

    fn ram_bytes_used(&self, output: &CharsRef) -> i64 {
        // `RamUsageEstimator.sizeOf(char[])`: two bytes per element.
        BASE_NUM_BYTES
            + RamUsageEstimator::align_object_size(
                RamUsageEstimator::NUM_BYTES_ARRAY_HEADER + 2 * output.chars.len() as i64,
            )
    }

    fn output_hash(&self, output: &CharsRef) -> i64 {
        // Mirrors `CharsRef.hashCode()`: `result = 31 * result + chars[i]`.
        let mut result: i32 = 0;
        for c in output.chars() {
            result = result.wrapping_mul(31).wrapping_add(*c as i32);
        }
        i64::from(result)
    }
}
