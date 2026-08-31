//! Port of `org.apache.lucene.util.fst.PairOutputs`.

use std::fmt;

use crate::error::Result;
use crate::store::{DataInput, DataOutput};

use super::outputs::Outputs;

/// Shallow size of a `Pair`, mirroring
/// `RamUsageEstimator.shallowSizeOf(new Pair<Object, Object>(null, null))`.
const BASE_NUM_BYTES: i64 = 24;

/// Holds a single pair of two outputs.
///
/// Equivalent to `PairOutputs.Pair<A, B>`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Pair<A, B> {
    /// The first output of the pair.
    pub output1: A,
    /// The second output of the pair.
    pub output2: B,
}

impl<A: fmt::Debug, B: fmt::Debug> fmt::Display for Pair<A, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Pair({:?},{:?})", self.output1, self.output2)
    }
}

/// An FST [`Outputs`] implementation holding two other outputs.
///
/// Equivalent to `org.apache.lucene.util.fst.PairOutputs<A, B>`.
///
/// # Java to Rust adaptations
///
/// * Lucene's `newPair` canonicalises each half to the corresponding
///   `NO_OUTPUT` singleton so that later reference comparisons work. This port
///   compares outputs by value, so [`PairOutputs::new_pair`] simply builds the
///   pair; see [`Outputs`] for why the two are equivalent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PairOutputs<O1, O2> {
    outputs1: O1,
    outputs2: O2,
}

impl<O1: Outputs, O2: Outputs> PairOutputs<O1, O2> {
    /// Creates a pair of the two provided outputs.
    ///
    /// Equivalent to `new PairOutputs(Outputs<A>, Outputs<B>)`.
    pub fn new(outputs1: O1, outputs2: O2) -> Self {
        Self { outputs1, outputs2 }
    }

    /// Creates a new [`Pair`].
    ///
    /// Equivalent to `PairOutputs.newPair`.
    pub fn new_pair(&self, a: O1::Output, b: O2::Output) -> Pair<O1::Output, O2::Output> {
        Pair {
            output1: a,
            output2: b,
        }
    }

    /// Returns the outputs of the first half of every pair.
    pub fn outputs1(&self) -> &O1 {
        &self.outputs1
    }

    /// Returns the outputs of the second half of every pair.
    pub fn outputs2(&self) -> &O2 {
        &self.outputs2
    }
}

impl<O1: Outputs, O2: Outputs> fmt::Display for PairOutputs<O1, O2> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PairOutputs<{},{}>", self.outputs1, self.outputs2)
    }
}

impl<O1: Outputs, O2: Outputs> Outputs for PairOutputs<O1, O2> {
    type Output = Pair<O1::Output, O2::Output>;

    fn common(&self, pair1: &Self::Output, pair2: &Self::Output) -> Self::Output {
        self.new_pair(
            self.outputs1.common(&pair1.output1, &pair2.output1),
            self.outputs2.common(&pair1.output2, &pair2.output2),
        )
    }

    fn subtract(&self, output: &Self::Output, inc: &Self::Output) -> Self::Output {
        self.new_pair(
            self.outputs1.subtract(&output.output1, &inc.output1),
            self.outputs2.subtract(&output.output2, &inc.output2),
        )
    }

    fn add(&self, prefix: &Self::Output, output: &Self::Output) -> Self::Output {
        self.new_pair(
            self.outputs1.add(&prefix.output1, &output.output1),
            self.outputs2.add(&prefix.output2, &output.output2),
        )
    }

    fn write(&self, output: &Self::Output, writer: &mut dyn DataOutput) -> Result<()> {
        self.outputs1.write(&output.output1, writer)?;
        self.outputs2.write(&output.output2, writer)
    }

    fn read(&self, input: &mut dyn DataInput) -> Result<Self::Output> {
        let output1 = self.outputs1.read(input)?;
        let output2 = self.outputs2.read(input)?;
        Ok(self.new_pair(output1, output2))
    }

    fn skip_output(&self, input: &mut dyn DataInput) -> Result<()> {
        self.outputs1.skip_output(input)?;
        self.outputs2.skip_output(input)
    }

    fn equals(&self, a: &Self::Output, b: &Self::Output) -> bool {
        self.outputs1.equals(&a.output1, &b.output1) && self.outputs2.equals(&a.output2, &b.output2)
    }

    fn no_output(&self) -> Self::Output {
        self.new_pair(self.outputs1.no_output(), self.outputs2.no_output())
    }

    fn output_to_string(&self, output: &Self::Output) -> String {
        format!(
            "<pair:{},{}>",
            self.outputs1.output_to_string(&output.output1),
            self.outputs2.output_to_string(&output.output2)
        )
    }

    fn ram_bytes_used(&self, output: &Self::Output) -> i64 {
        BASE_NUM_BYTES
            + self.outputs1.ram_bytes_used(&output.output1)
            + self.outputs2.ram_bytes_used(&output.output2)
    }

    fn output_hash(&self, output: &Self::Output) -> i64 {
        // Mirrors `Pair.hashCode()`: `output1.hashCode() + output2.hashCode()`.
        i64::from(
            (self.outputs1.output_hash(&output.output1) as i32)
                .wrapping_add(self.outputs2.output_hash(&output.output2) as i32),
        )
    }
}
