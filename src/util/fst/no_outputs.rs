//! Port of `org.apache.lucene.util.fst.NoOutputs`.

use std::fmt;

use crate::error::Result;
use crate::store::{DataInput, DataOutput};

use super::outputs::Outputs;

/// A null FST [`Outputs`] implementation; use it to build a plain FSA.
///
/// Equivalent to `org.apache.lucene.util.fst.NoOutputs`.
///
/// # Java to Rust adaptations
///
/// * Lucene's `NO_OUTPUT` is an anonymous `Object` whose `hashCode()` is fixed
///   at 42 so that hashing stays deterministic. Here the output type is the
///   unit type, and [`Outputs::output_hash`] returns the same 42.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoOutputs;

impl NoOutputs {
    /// Returns the singleton instance.
    ///
    /// Equivalent to `NoOutputs.getSingleton`.
    pub fn get_singleton() -> Self {
        Self
    }

    /// Returns the singleton "no output" value.
    ///
    /// Equivalent to `NoOutputs.getNoOutput`.
    pub fn no_output_value() {}
}

impl fmt::Display for NoOutputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NoOutputs")
    }
}

impl Outputs for NoOutputs {
    type Output = ();

    fn common(&self, _output1: &(), _output2: &()) {}

    fn subtract(&self, _output: &(), _inc: &()) {}

    fn add(&self, _prefix: &(), _output: &()) {}

    fn merge(&self, _first: &(), _second: &()) -> Result<()> {
        Ok(())
    }

    fn write(&self, _output: &(), _out: &mut dyn DataOutput) -> Result<()> {
        Ok(())
    }

    fn read(&self, _input: &mut dyn DataInput) -> Result<()> {
        Ok(())
    }

    fn skip_output(&self, _input: &mut dyn DataInput) -> Result<()> {
        Ok(())
    }

    fn equals(&self, _a: &(), _b: &()) -> bool {
        true
    }

    fn no_output(&self) {}

    fn output_to_string(&self, _output: &()) -> String {
        String::new()
    }

    fn ram_bytes_used(&self, _output: &()) -> i64 {
        0
    }

    fn output_hash(&self, _output: &()) -> i64 {
        // NodeHash calls hashCode on this output; Lucene fixes it at 42 so that
        // hashing is deterministic.
        42
    }
}
