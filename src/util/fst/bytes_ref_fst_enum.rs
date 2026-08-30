//! Port of `org.apache.lucene.util.fst.BytesRefFSTEnum`.

use crate::error::Result;
use crate::util::{ArrayUtil, BytesRef};

use super::fst::{END_LABEL, FST};
use super::fst_enum::{FSTEnum, FSTEnumTarget};
use super::outputs::Outputs;

/// Holds a single input ([`BytesRef`]) and output pair.
///
/// Equivalent to `BytesRefFSTEnum.InputOutput<T>`.
///
/// # Java to Rust adaptations
///
/// * Lucene returns a reusable object whose `input` field aliases the enum's
///   own scratch `BytesRef`. This port returns a borrowed view of the same
///   buffers instead, which is the closest safe equivalent and copies nothing.
#[derive(Debug)]
pub struct InputOutput<'e, T> {
    /// The current term.
    pub input: &'e BytesRef,
    /// The output the FST maps the current term to.
    pub output: &'e T,
}

/// The current term and the seek target of a [`BytesRefFSTEnum`].
///
/// Equivalent to the `current` and `target` fields of Lucene's
/// `BytesRefFSTEnum`, together with the four label accessors it overrides.
struct BytesRefEnumState {
    current: BytesRef,
    target: BytesRef,
}

impl FSTEnumTarget for BytesRefEnumState {
    fn target_label(&self, upto: usize) -> i32 {
        if upto - 1 == self.target.length {
            END_LABEL
        } else {
            i32::from(self.target.bytes[self.target.offset + upto - 1])
        }
    }

    fn current_label(&self, upto: usize) -> i32 {
        // current.offset is fixed at 1.
        i32::from(self.current.bytes[upto])
    }

    fn set_current_label(&mut self, upto: usize, label: i32) {
        self.current.bytes[upto] = label as u8;
    }

    fn grow(&mut self, upto: usize) {
        let min_size = upto + 1;
        if self.current.bytes.len() < min_size {
            let new_len = ArrayUtil::oversize(min_size, 1).max(min_size);
            self.current.bytes.resize(new_len, 0);
        }
    }
}

/// Enumerates all input ([`BytesRef`]) and output pairs of an FST.
///
/// Equivalent to `org.apache.lucene.util.fst.BytesRefFSTEnum<T>`.
pub struct BytesRefFSTEnum<'a, O: Outputs> {
    inner: FSTEnum<'a, O>,
    state: BytesRefEnumState,
}

impl<'a, O: Outputs> BytesRefFSTEnum<'a, O> {
    /// Creates an enumeration positioned before the first term.
    ///
    /// Equivalent to `new BytesRefFSTEnum(FST)`.
    ///
    /// # Errors
    ///
    /// Propagates the error raised while opening the FST's byte reader.
    pub fn new(fst: &'a FST<O>) -> Result<Self> {
        Ok(Self {
            inner: FSTEnum::new(fst)?,
            state: BytesRefEnumState {
                // The current term is written from index 1 onwards, so its
                // offset is fixed at 1, exactly as in Lucene.
                current: BytesRef {
                    bytes: vec![0u8; 10],
                    offset: 1,
                    length: 0,
                },
                target: BytesRef::default(),
            },
        })
    }

    /// Returns the current input and output pair, or `None` before the first
    /// term and after the last one.
    ///
    /// Equivalent to `BytesRefFSTEnum.current`.
    pub fn current(&self) -> Option<InputOutput<'_, O::Output>> {
        let upto = self.inner.upto();
        if upto == 0 {
            None
        } else {
            Some(InputOutput {
                input: &self.state.current,
                output: self.inner.output_at(upto),
            })
        }
    }

    /// Advances to the next term.
    ///
    /// Equivalent to `BytesRefFSTEnum.next`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    // Named after Lucene's `next()`; it cannot implement `Iterator`
    // because the item borrows the enumerator.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<InputOutput<'_, O::Output>>> {
        self.inner.do_next(&mut self.state)?;
        Ok(self.set_result())
    }

    /// Seeks to the smallest term that is greater than or equal to `target`.
    ///
    /// Equivalent to `BytesRefFSTEnum.seekCeil`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn seek_ceil(&mut self, target: &BytesRef) -> Result<Option<InputOutput<'_, O::Output>>> {
        self.state.target = target.clone();
        self.inner.set_target_length(target.length);
        self.inner.do_seek_ceil(&mut self.state)?;
        Ok(self.set_result())
    }

    /// Seeks to the biggest term that is less than or equal to `target`.
    ///
    /// Equivalent to `BytesRefFSTEnum.seekFloor`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn seek_floor(&mut self, target: &BytesRef) -> Result<Option<InputOutput<'_, O::Output>>> {
        self.state.target = target.clone();
        self.inner.set_target_length(target.length);
        self.inner.do_seek_floor(&mut self.state)?;
        Ok(self.set_result())
    }

    /// Seeks to exactly `target`, returning `None` when the term does not
    /// exist.
    ///
    /// Equivalent to `BytesRefFSTEnum.seekExact`. This is faster than
    /// [`BytesRefFSTEnum::seek_floor`] or [`BytesRefFSTEnum::seek_ceil`]
    /// because it short-circuits as soon as the match fails.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn seek_exact(&mut self, target: &BytesRef) -> Result<Option<InputOutput<'_, O::Output>>> {
        self.state.target = target.clone();
        self.inner.set_target_length(target.length);
        if self.inner.do_seek_exact(&mut self.state)? {
            debug_assert_eq!(self.inner.upto(), 1 + target.length);
            Ok(self.set_result())
        } else {
            Ok(None)
        }
    }

    /// Equivalent to the private `BytesRefFSTEnum.setResult`.
    fn set_result(&mut self) -> Option<InputOutput<'_, O::Output>> {
        let upto = self.inner.upto();
        if upto == 0 {
            return None;
        }
        self.state.current.length = upto - 1;
        Some(InputOutput {
            input: &self.state.current,
            output: self.inner.output_at(upto),
        })
    }
}
