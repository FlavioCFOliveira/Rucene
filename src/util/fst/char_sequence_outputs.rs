//! Port of `org.apache.lucene.util.fst.CharSequenceOutputs`.

use std::fmt;

use crate::error::{LuceneError, Result};
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
/// # Divergence from Lucene 10.5.0
///
/// Lucene's `CharsRef` wraps a Java `char[]`, that is, a sequence of **UTF-16
/// code units**, and `write` emits one `VInt` per code unit. This crate's
/// [`CharsRef`] (`src/util/chars_ref.rs`, outside this module) wraps a
/// `Vec<char>`, that is, a sequence of **Unicode scalar values**.
///
/// To keep the serialized bytes identical, [`Outputs::write`] still emits the
/// UTF-16 code units -- the count first, then one `VInt` per unit -- and
/// [`Outputs::read`] decodes the units back into scalar values. For any text
/// made of BMP characters, which is what every Lucene caller of this class
/// produces, the two representations coincide element for element and the
/// bytes are exactly Lucene's.
///
/// Two observable consequences remain, both confined to supplementary
/// characters (`U+10000` and above):
///
/// * [`Outputs::common`] compares scalar values, so it never splits a surrogate
///   pair, while Lucene compares code units and can share a lone high
///   surrogate between two outputs. When that happens the FSTs differ in how an
///   output is split across arcs; both are correct, and both answer every
///   lookup with the same output.
/// * [`Outputs::read`] rejects an unpaired surrogate with
///   [`LuceneError::CorruptIndex`], because a Rust `char` cannot hold one.
///   Lucene stores it verbatim.
///
/// Removing this divergence requires changing `CharsRef` itself, which this
/// module does not own.
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

/// Returns the number of UTF-16 code units the active characters occupy.
fn utf16_len(output: &CharsRef) -> usize {
    output.chars().iter().map(|c| c.len_utf16()).sum()
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
            debug_assert!(inc.length < output.length);
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
        out.write_v_int(utf16_len(prefix) as i32)?;
        // TODO(Lucene): maybe UTF-8?
        let mut buffer = [0u16; 2];
        for c in prefix.chars() {
            for unit in c.encode_utf16(&mut buffer) {
                out.write_v_int(i32::from(*unit))?;
            }
        }
        Ok(())
    }

    fn read(&self, input: &mut dyn DataInput) -> Result<CharsRef> {
        let len = input.read_v_int()?;
        if len == 0 {
            return Ok(self.no_output());
        }
        let len = usize::try_from(len)
            .map_err(|_| LuceneError::CorruptIndex(format!("invalid output length {len}")))?;
        let mut units = Vec::with_capacity(len);
        for _ in 0..len {
            let value = input.read_v_int()?;
            let unit = u16::try_from(value).map_err(|_| {
                LuceneError::CorruptIndex(format!("invalid UTF-16 code unit {value}"))
            })?;
            units.push(unit);
        }
        let mut chars = Vec::with_capacity(len);
        for decoded in char::decode_utf16(units) {
            chars.push(decoded.map_err(|e| {
                LuceneError::CorruptIndex(format!(
                    "unpaired UTF-16 surrogate 0x{:04x} in FST output",
                    e.unpaired_surrogate()
                ))
            })?);
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
        output.chars().iter().collect()
    }

    fn ram_bytes_used(&self, output: &CharsRef) -> i64 {
        // Lucene reports a Java `char[]`, two bytes per element; this port
        // reports the four bytes a Rust `char` really occupies.
        BASE_NUM_BYTES
            + RamUsageEstimator::align_object_size(
                RamUsageEstimator::NUM_BYTES_ARRAY_HEADER + 4 * output.chars.len() as i64,
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
