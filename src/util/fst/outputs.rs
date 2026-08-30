//! Port of `org.apache.lucene.util.fst.Outputs`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`Outputs`] | `Outputs<T>` |

use std::fmt::Display;

use crate::error::{LuceneError, Result};
use crate::store::{DataInput, DataOutput};

/// Represents the outputs for an FST, providing the basic algebra required for
/// building and traversing the FST.
///
/// Equivalent to `org.apache.lucene.util.fst.Outputs<T>`.
///
/// # Java to Rust adaptations
///
/// * Lucene parameterises the class by the output type (`Outputs<T>`). This
///   port makes the output type an associated type ([`Outputs::Output`]), so
///   that every FST is generic over the *outputs implementation* rather than
///   over the value type. This removes the need for trait objects and keeps
///   every call monomorphised; `FST<PositiveIntOutputs>` is this port's
///   spelling of Lucene's `FST<Long>`.
/// * Lucene documents that "any operation that returns NO_OUTPUT must return
///   the same singleton object", and several byte-layout decisions in
///   `FSTCompiler.addNode` are written as reference comparisons
///   (`arc.output != NO_OUTPUT`). `FSTCompiler.validOutput` asserts the exact
///   invariant that makes reference equality and value equality
///   interchangeable: an output either *is* the singleton, or is not equal to
///   it. This port therefore compares outputs by value, which selects the same
///   arc flags and hence writes the same bytes.
/// * Outputs are compared through [`Outputs::equals`] rather than through
///   [`PartialEq`]. Lucene calls `Object.equals` on the output values
///   directly, but not every value type this crate uses as an FST output
///   implements [`PartialEq`] -- `crate::util::IntsRef` does not -- and those
///   types live outside this module. Routing equality through the outputs
///   instance keeps the port self-contained and matches what Lucene compares:
///   the output values themselves.
/// * [`Outputs::Output`] must implement [`Default`] because Rust has no `null`:
///   `FST.Arc` fields that Lucene leaves null until the first read are
///   default-constructed here instead. Every code path assigns them before use,
///   exactly as in Lucene.
/// * [`Outputs::output_hash`] replaces Java's `Object.hashCode()`, which
///   `NodeHash` calls on outputs. It only decides hash-slot placement inside
///   the in-memory suffix cache and can never change the serialized FST: two
///   equal nodes always hash alike, so the set of de-duplicated nodes -- and
///   therefore the bytes written -- is independent of the hash values. Lucene
///   relies on the same property, since `BytesRef.hashCode()` is seeded from a
///   per-JVM random value.
pub trait Outputs: Clone + Display {
    /// The output value carried by every arc.
    ///
    /// Equivalent to the type parameter `T` of Lucene's `Outputs<T>`.
    type Output: Clone + Default;

    /// Returns the longest common prefix of two outputs.
    ///
    /// Equivalent to `Outputs.common`, e.g. `common("foobar", "food")` is
    /// `"foo"`.
    fn common(&self, output1: &Self::Output, output2: &Self::Output) -> Self::Output;

    /// Removes the prefix `inc` from `output`.
    ///
    /// Equivalent to `Outputs.subtract`, e.g. `subtract("foobar", "foo")` is
    /// `"bar"`.
    fn subtract(&self, output: &Self::Output, inc: &Self::Output) -> Self::Output;

    /// Concatenates `prefix` and `output`.
    ///
    /// Equivalent to `Outputs.add`, e.g. `add("foo", "bar")` is `"foobar"`.
    fn add(&self, prefix: &Self::Output, output: &Self::Output) -> Self::Output;

    /// Encodes an output value into a [`DataOutput`].
    ///
    /// Equivalent to `Outputs.write`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the underlying output.
    fn write(&self, output: &Self::Output, out: &mut dyn DataOutput) -> Result<()>;

    /// Encodes a final node output value into a [`DataOutput`].
    ///
    /// Equivalent to `Outputs.writeFinalOutput`; by default this just calls
    /// [`Outputs::write`].
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the underlying output.
    fn write_final_output(&self, output: &Self::Output, out: &mut dyn DataOutput) -> Result<()> {
        self.write(output, out)
    }

    /// Decodes an output value previously written with [`Outputs::write`].
    ///
    /// Equivalent to `Outputs.read`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the underlying input.
    fn read(&self, input: &mut dyn DataInput) -> Result<Self::Output>;

    /// Skips an output value previously written with [`Outputs::write`].
    ///
    /// Equivalent to `Outputs.skipOutput`; by default this just calls
    /// [`Outputs::read`] and discards the result.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the underlying input.
    fn skip_output(&self, input: &mut dyn DataInput) -> Result<()> {
        self.read(input)?;
        Ok(())
    }

    /// Decodes an output value previously written with
    /// [`Outputs::write_final_output`].
    ///
    /// Equivalent to `Outputs.readFinalOutput`; by default this just calls
    /// [`Outputs::read`].
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the underlying input.
    fn read_final_output(&self, input: &mut dyn DataInput) -> Result<Self::Output> {
        self.read(input)
    }

    /// Skips an output value previously written with
    /// [`Outputs::write_final_output`].
    ///
    /// Equivalent to `Outputs.skipFinalOutput`; by default this just calls
    /// [`Outputs::skip_output`].
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the underlying input.
    fn skip_final_output(&self, input: &mut dyn DataInput) -> Result<()> {
        self.skip_output(input)
    }

    /// Returns whether two outputs are equal.
    ///
    /// Equivalent to calling `equals` on the output values in Lucene. See the
    /// trait documentation for why this is a method rather than a
    /// [`PartialEq`] bound.
    fn equals(&self, a: &Self::Output, b: &Self::Output) -> bool;

    /// Returns the value that represents "no output".
    ///
    /// Equivalent to `Outputs.getNoOutput`.
    fn no_output(&self) -> Self::Output;

    /// Renders an output for debugging and for `Util::to_dot`.
    ///
    /// Equivalent to `Outputs.outputToString`.
    fn output_to_string(&self, output: &Self::Output) -> String;

    /// Merges two outputs that were added for the same input.
    ///
    /// Equivalent to `Outputs.merge`, which throws
    /// `UnsupportedOperationException` unless the implementation overrides it.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] unless overridden.
    fn merge(&self, _first: &Self::Output, _second: &Self::Output) -> Result<Self::Output> {
        Err(LuceneError::UnsupportedOperation(
            "this Outputs implementation does not support merge".to_string(),
        ))
    }

    /// Returns the estimated memory usage of the provided output.
    ///
    /// Equivalent to `Outputs.ramBytesUsed`.
    fn ram_bytes_used(&self, output: &Self::Output) -> i64;

    /// Returns the hash of the provided output, as `NodeHash` needs it.
    ///
    /// Equivalent to calling `hashCode()` on the output in Lucene. See the
    /// trait documentation for why this value never reaches the index.
    fn output_hash(&self, output: &Self::Output) -> i64;
}
