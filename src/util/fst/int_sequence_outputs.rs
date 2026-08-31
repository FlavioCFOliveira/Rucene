//! Port of `org.apache.lucene.util.fst.IntSequenceOutputs`.

use std::fmt;

use crate::error::{LuceneError, Result};
use crate::store::{DataInput, DataOutput};
use crate::util::{IntsRef, RamUsageEstimator};

use super::byte_sequence_outputs::mismatch;
use super::outputs::Outputs;

/// Shallow size of an `IntsRef`, mirroring
/// `RamUsageEstimator.shallowSizeOf(NO_OUTPUT)` in Lucene.
const BASE_NUM_BYTES: i64 = 32;

/// An FST [`Outputs`] implementation where each output is a sequence of ints.
///
/// Equivalent to `org.apache.lucene.util.fst.IntSequenceOutputs`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IntSequenceOutputs;

impl IntSequenceOutputs {
    /// Returns the singleton instance.
    ///
    /// Equivalent to `IntSequenceOutputs.getSingleton`.
    pub fn get_singleton() -> Self {
        Self
    }
}

impl fmt::Display for IntSequenceOutputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IntSequenceOutputs")
    }
}

impl Outputs for IntSequenceOutputs {
    type Output = IntsRef;

    // Lucene enumerates the four prefix cases separately, two of which
    // return `output1`; keeping them apart matches the Java source.
    #[allow(clippy::if_same_then_else)]
    fn common(&self, output1: &IntsRef, output2: &IntsRef) -> IntsRef {
        let mismatch_pos = mismatch(output1.slice(), output2.slice());

        if mismatch_pos == 0 {
            self.no_output()
        } else if mismatch_pos == -1 {
            output1.clone()
        } else if mismatch_pos as usize == output1.length {
            output1.clone()
        } else if mismatch_pos as usize == output2.length {
            output2.clone()
        } else {
            IntsRef::new(output1.slice()[..mismatch_pos as usize].to_vec())
        }
    }

    fn subtract(&self, output: &IntsRef, inc: &IntsRef) -> IntsRef {
        if inc.length == 0 {
            output.clone()
        } else if inc.length == output.length {
            self.no_output()
        } else {
            debug_assert!(inc.length < output.length);
            IntsRef::new(output.slice()[inc.length..].to_vec())
        }
    }

    fn add(&self, prefix: &IntsRef, output: &IntsRef) -> IntsRef {
        if prefix.length == 0 {
            output.clone()
        } else if output.length == 0 {
            prefix.clone()
        } else {
            let mut result = Vec::with_capacity(prefix.length + output.length);
            result.extend_from_slice(prefix.slice());
            result.extend_from_slice(output.slice());
            IntsRef::new(result)
        }
    }

    fn write(&self, prefix: &IntsRef, out: &mut dyn DataOutput) -> Result<()> {
        out.write_v_int(prefix.length as i32)?;
        for value in prefix.slice() {
            out.write_v_int(*value)?;
        }
        Ok(())
    }

    fn read(&self, input: &mut dyn DataInput) -> Result<IntsRef> {
        let len = input.read_v_int()?;
        if len == 0 {
            Ok(self.no_output())
        } else {
            let len = usize::try_from(len)
                .map_err(|_| LuceneError::CorruptIndex(format!("invalid output length {len}")))?;
            let mut ints = Vec::with_capacity(len);
            for _ in 0..len {
                ints.push(input.read_v_int()?);
            }
            Ok(IntsRef::new(ints))
        }
    }

    fn skip_output(&self, input: &mut dyn DataInput) -> Result<()> {
        let len = input.read_v_int()?;
        if len == 0 {
            return Ok(());
        }
        for _ in 0..len {
            input.read_v_int()?;
        }
        Ok(())
    }

    fn equals(&self, a: &IntsRef, b: &IntsRef) -> bool {
        a.slice() == b.slice()
    }

    fn no_output(&self) -> IntsRef {
        IntsRef::default()
    }

    fn output_to_string(&self, output: &IntsRef) -> String {
        format!("{:?}", output.slice())
    }

    fn ram_bytes_used(&self, output: &IntsRef) -> i64 {
        BASE_NUM_BYTES + RamUsageEstimator::size_of_int(&output.ints)
    }

    fn output_hash(&self, output: &IntsRef) -> i64 {
        // Mirrors `IntsRef.hashCode()`: `result = 31 * result + ints[i]`.
        let mut result: i32 = 0;
        for value in output.slice() {
            result = result.wrapping_mul(31).wrapping_add(*value);
        }
        i64::from(result)
    }
}
