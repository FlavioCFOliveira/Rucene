//! Port of `org.apache.lucene.util.fst.IntsRefFSTEnum`.

use crate::error::Result;
use crate::util::{ArrayUtil, IntsRef};

use super::fst::{END_LABEL, FST};
use super::fst_enum::{FSTEnum, FSTEnumTarget};
use super::outputs::Outputs;

/// Holds a single input ([`IntsRef`]) and output pair.
///
/// Equivalent to `IntsRefFSTEnum.InputOutput<T>`; see
/// [`super::bytes_ref_fst_enum::InputOutput`] for why it is a borrowed view.
#[derive(Debug)]
pub struct InputOutput<'e, T> {
    /// The current term.
    pub input: &'e IntsRef,
    /// The output the FST maps the current term to.
    pub output: &'e T,
}

/// The current term and the seek target of an [`IntsRefFSTEnum`].
///
/// Equivalent to the `current` and `target` fields of Lucene's
/// `IntsRefFSTEnum`, together with the four label accessors it overrides.
struct IntsRefEnumState {
    current: IntsRef,
    target: IntsRef,
}

impl FSTEnumTarget for IntsRefEnumState {
    fn target_label(&self, upto: usize) -> i32 {
        if upto - 1 == self.target.length {
            END_LABEL
        } else {
            self.target.ints[self.target.offset + upto - 1]
        }
    }

    fn current_label(&self, upto: usize) -> i32 {
        // current.offset is fixed at 1.
        self.current.ints[upto]
    }

    fn set_current_label(&mut self, upto: usize, label: i32) {
        self.current.ints[upto] = label;
    }

    fn grow(&mut self, upto: usize) {
        let min_size = upto + 1;
        if self.current.ints.len() < min_size {
            let new_len = ArrayUtil::oversize(min_size, 4).max(min_size);
            self.current.ints.resize(new_len, 0);
        }
    }
}

/// Enumerates all input ([`IntsRef`]) and output pairs of an FST.
///
/// Equivalent to `org.apache.lucene.util.fst.IntsRefFSTEnum<T>`.
pub struct IntsRefFSTEnum<'a, O: Outputs> {
    inner: FSTEnum<'a, O>,
    state: IntsRefEnumState,
}

impl<'a, O: Outputs> IntsRefFSTEnum<'a, O> {
    /// Creates an enumeration positioned before the first term.
    ///
    /// Equivalent to `new IntsRefFSTEnum(FST)`.
    ///
    /// # Errors
    ///
    /// Propagates the error raised while opening the FST's byte reader.
    pub fn new(fst: &'a FST<O>) -> Result<Self> {
        Ok(Self {
            inner: FSTEnum::new(fst)?,
            state: IntsRefEnumState {
                // The current term is written from index 1 onwards, so its
                // offset is fixed at 1, exactly as in Lucene.
                current: IntsRef {
                    ints: vec![0i32; 10],
                    offset: 1,
                    length: 0,
                },
                target: IntsRef::default(),
            },
        })
    }

    /// Returns the current input and output pair, or `None` before the first
    /// term and after the last one.
    ///
    /// Equivalent to `IntsRefFSTEnum.current`.
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
    /// Equivalent to `IntsRefFSTEnum.next`.
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
    /// Equivalent to `IntsRefFSTEnum.seekCeil`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn seek_ceil(&mut self, target: &IntsRef) -> Result<Option<InputOutput<'_, O::Output>>> {
        self.state.target = target.clone();
        self.inner.set_target_length(target.length);
        self.inner.do_seek_ceil(&mut self.state)?;
        Ok(self.set_result())
    }

    /// Seeks to the biggest term that is less than or equal to `target`.
    ///
    /// Equivalent to `IntsRefFSTEnum.seekFloor`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn seek_floor(&mut self, target: &IntsRef) -> Result<Option<InputOutput<'_, O::Output>>> {
        self.state.target = target.clone();
        self.inner.set_target_length(target.length);
        self.inner.do_seek_floor(&mut self.state)?;
        Ok(self.set_result())
    }

    /// Seeks to exactly `target`, returning `None` when the term does not
    /// exist.
    ///
    /// Equivalent to `IntsRefFSTEnum.seekExact`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn seek_exact(&mut self, target: &IntsRef) -> Result<Option<InputOutput<'_, O::Output>>> {
        self.state.target = target.clone();
        self.inner.set_target_length(target.length);
        if self.inner.do_seek_exact(&mut self.state)? {
            debug_assert_eq!(self.inner.upto(), 1 + target.length);
            Ok(self.set_result())
        } else {
            Ok(None)
        }
    }

    /// Equivalent to the private `IntsRefFSTEnum.setResult`.
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
