//! Port of `org.apache.lucene.util.fst.PositiveIntOutputs`.

use std::fmt;

use crate::error::Result;
use crate::store::{DataInput, DataOutput};

use super::outputs::Outputs;

/// An FST [`Outputs`] implementation where each output is a non-negative
/// `i64`.
///
/// Equivalent to `org.apache.lucene.util.fst.PositiveIntOutputs`. `0` is the
/// "no output" value, so every real output must be strictly positive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PositiveIntOutputs;

/// The value that stands for "no output".
///
/// Equivalent to `PositiveIntOutputs.NO_OUTPUT`.
const NO_OUTPUT: i64 = 0;

impl PositiveIntOutputs {
    /// Returns the singleton instance.
    ///
    /// Equivalent to `PositiveIntOutputs.getSingleton`.
    pub fn get_singleton() -> Self {
        Self
    }
}

impl fmt::Display for PositiveIntOutputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PositiveIntOutputs")
    }
}

impl Outputs for PositiveIntOutputs {
    type Output = i64;

    fn common(&self, output1: &i64, output2: &i64) -> i64 {
        if *output1 == NO_OUTPUT || *output2 == NO_OUTPUT {
            NO_OUTPUT
        } else {
            debug_assert!(*output1 > 0 && *output2 > 0);
            (*output1).min(*output2)
        }
    }

    fn subtract(&self, output: &i64, inc: &i64) -> i64 {
        debug_assert!(*output >= *inc);
        if *inc == NO_OUTPUT {
            *output
        } else if *output == *inc {
            NO_OUTPUT
        } else {
            // Java arithmetic wraps; a corrupt index must not abort a debug
            // build where the JVM would simply produce a wrapped value.
            output.wrapping_sub(*inc)
        }
    }

    fn add(&self, prefix: &i64, output: &i64) -> i64 {
        if *prefix == NO_OUTPUT {
            *output
        } else if *output == NO_OUTPUT {
            *prefix
        } else {
            // See `subtract`: Java arithmetic wraps rather than aborting.
            prefix.wrapping_add(*output)
        }
    }

    fn write(&self, output: &i64, out: &mut dyn DataOutput) -> Result<()> {
        out.write_v_long(*output)
    }

    fn read(&self, input: &mut dyn DataInput) -> Result<i64> {
        input.read_v_long()
    }

    fn equals(&self, a: &i64, b: &i64) -> bool {
        a == b
    }

    fn no_output(&self) -> i64 {
        NO_OUTPUT
    }

    fn output_to_string(&self, output: &i64) -> String {
        output.to_string()
    }

    fn ram_bytes_used(&self, _output: &i64) -> i64 {
        // Lucene reports `RamUsageEstimator.sizeOf(Long)`, the size of a boxed
        // `Long`; a Rust `i64` is the eight bytes reported here.
        std::mem::size_of::<i64>() as i64
    }

    fn output_hash(&self, output: &i64) -> i64 {
        // Mirrors `Long.hashCode()`.
        i64::from((*output ^ ((*output as u64) >> 32) as i64) as i32)
    }
}
