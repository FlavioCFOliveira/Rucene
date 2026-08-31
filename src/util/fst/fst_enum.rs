//! Port of `org.apache.lucene.util.fst.FSTEnum`.

use crate::error::Result;
use crate::util::ArrayUtil;

use super::fst::{
    Arc, BitTable, BytesReader, ARCS_FOR_BINARY_SEARCH, ARCS_FOR_DIRECT_ADDRESSING, END_LABEL, FST,
};
use super::outputs::Outputs;
use super::util::binary_search;

/// The per-subclass state an [`FSTEnum`] drives.
///
/// Equivalent to the four abstract methods of `FSTEnum`:
/// `getTargetLabel`, `getCurrentLabel`, `setCurrentLabel` and `grow`. Lucene
/// implements them in the subclasses, which also own the current term and the
/// seek target; this port keeps that state in a separate value so that Rust can
/// borrow it independently of the shared enumeration state.
pub trait FSTEnumTarget {
    /// Returns the label of the seek target at the current depth.
    ///
    /// Equivalent to `FSTEnum.getTargetLabel`.
    fn target_label(&self, upto: usize) -> i32;

    /// Returns the label of the current term at the current depth.
    ///
    /// Equivalent to `FSTEnum.getCurrentLabel`.
    fn current_label(&self, upto: usize) -> i32;

    /// Sets the label of the current term at the current depth.
    ///
    /// Equivalent to `FSTEnum.setCurrentLabel`.
    fn set_current_label(&mut self, upto: usize, label: i32);

    /// Grows the current term so that it can hold `upto + 1` labels.
    ///
    /// Equivalent to `FSTEnum.grow`.
    fn grow(&mut self, upto: usize);
}

/// Returns a shared reference to `arcs[i]` and a mutable one to `arcs[j]`.
///
/// Lucene passes two distinct elements of its arc array to the FST reading
/// methods; Rust needs the two borrows to be provably disjoint.
///
/// # Panics
///
/// Panics when `i == j`; every call site follows a parent/child pair.
fn arc_pair<T>(arcs: &mut [T], i: usize, j: usize) -> (&T, &mut T) {
    assert_ne!(i, j, "arc_pair requires two distinct arcs");
    if i < j {
        let (head, tail) = arcs.split_at_mut(j);
        (&head[i], &mut tail[0])
    } else {
        let (head, tail) = arcs.split_at_mut(i);
        (&tail[0], &mut head[j])
    }
}

/// Walks the terms of an FST with `next()` and `advance()`.
///
/// Equivalent to the package-private abstract class
/// `org.apache.lucene.util.fst.FSTEnum<T>`. The state that Lucene keeps in the
/// subclass -- the current term and the seek target -- lives in an
/// [`FSTEnumTarget`] that every method receives.
pub struct FSTEnum<'a, O: Outputs> {
    fst: &'a FST<O>,
    arcs: Vec<Arc<O::Output>>,
    /// Cumulative outputs.
    output: Vec<O::Output>,
    no_output: O::Output,
    fst_reader: Box<dyn BytesReader + 'a>,
    upto: usize,
    target_length: usize,
}

impl<'a, O: Outputs> FSTEnum<'a, O> {
    /// Creates an enumeration positioned before the first term.
    ///
    /// Equivalent to the `FSTEnum(FST)` constructor.
    ///
    /// # Errors
    ///
    /// Propagates the error raised while opening the FST's byte reader.
    pub fn new(fst: &'a FST<O>) -> Result<Self> {
        let no_output = fst.outputs().no_output();
        let fst_reader = fst.get_bytes_reader()?;
        let mut arcs: Vec<Arc<O::Output>> = vec![Arc::default(); 10];
        let output: Vec<O::Output> = vec![no_output.clone(); 10];
        fst.get_first_arc(&mut arcs[0]);
        Ok(Self {
            fst,
            arcs,
            output,
            no_output,
            fst_reader,
            upto: 0,
            target_length: 0,
        })
    }

    /// Returns the current depth, that is, one more than the length of the
    /// current term.
    ///
    /// Equivalent to the package-private field `FSTEnum.upto`.
    pub fn upto(&self) -> usize {
        self.upto
    }

    /// Returns the cumulative output at the current depth.
    pub fn output_at(&self, upto: usize) -> &O::Output {
        &self.output[upto]
    }

    /// Sets the length of the seek target.
    ///
    /// Equivalent to the package-private field `FSTEnum.targetLength`.
    pub fn set_target_length(&mut self, target_length: usize) {
        self.target_length = target_length;
    }

    /// Rewinds the enum state to match the shared prefix between the current
    /// term and the target term.
    ///
    /// Equivalent to the private `FSTEnum.rewindPrefix`.
    fn rewind_prefix(&mut self, target: &mut dyn FSTEnumTarget) -> Result<()> {
        let fst = self.fst;
        if self.upto == 0 {
            self.upto = 1;
            let (follow, arc) = arc_pair(&mut self.arcs, 0, 1);
            fst.read_first_target_arc(follow, arc, &mut *self.fst_reader)?;
            return Ok(());
        }

        let current_limit = self.upto;
        self.upto = 1;
        while self.upto < current_limit && self.upto <= self.target_length + 1 {
            let cmp = target.current_label(self.upto) - target.target_label(self.upto);
            if cmp < 0 {
                // Seek forward.
                break;
            } else if cmp > 0 {
                // Seek backwards: reset this arc to the first arc.
                let upto = self.upto;
                let (follow, arc) = arc_pair(&mut self.arcs, upto - 1, upto);
                fst.read_first_target_arc(follow, arc, &mut *self.fst_reader)?;
                break;
            }
            self.upto += 1;
        }
        Ok(())
    }

    /// Advances to the next term.
    ///
    /// Equivalent to `FSTEnum.doNext`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn do_next(&mut self, target: &mut dyn FSTEnumTarget) -> Result<()> {
        let fst = self.fst;
        if self.upto == 0 {
            self.upto = 1;
            let (follow, arc) = arc_pair(&mut self.arcs, 0, 1);
            fst.read_first_target_arc(follow, arc, &mut *self.fst_reader)?;
        } else {
            // Pop.
            while self.arcs[self.upto].is_last() {
                self.upto -= 1;
                if self.upto == 0 {
                    return Ok(());
                }
            }
            let upto = self.upto;
            fst.read_next_arc(&mut self.arcs[upto], &mut *self.fst_reader)?;
        }

        self.push_first(target)
    }

    /// Seeks to the smallest term that is greater than or equal to the target.
    ///
    /// Equivalent to `FSTEnum.doSeekCeil`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn do_seek_ceil(&mut self, target: &mut dyn FSTEnumTarget) -> Result<()> {
        // Save time by starting at the end of the shared prefix between the
        // current term and the target.
        self.rewind_prefix(target)?;

        let fst = self.fst;
        let mut input = fst.get_bytes_reader()?;

        // Now scan forward, matching the new suffix of the target.
        loop {
            let target_label = target.target_label(self.upto);
            let (bytes_per_arc, arc_label, node_flags) = {
                let arc = &self.arcs[self.upto];
                (arc.bytes_per_arc(), arc.label(), arc.node_flags())
            };
            let keep_going = if bytes_per_arc != 0 && arc_label != END_LABEL {
                // Arcs are in an array.
                if node_flags == ARCS_FOR_DIRECT_ADDRESSING {
                    self.do_seek_ceil_array_direct_addressing(target, target_label, &mut *input)?
                } else if node_flags == ARCS_FOR_BINARY_SEARCH {
                    self.do_seek_ceil_array_packed(target, target_label, &mut *input)?
                } else {
                    self.do_seek_ceil_array_continuous(target, target_label, &mut *input)?
                }
            } else {
                self.do_seek_ceil_list(target, target_label)?
            };
            if !keep_going {
                return Ok(());
            }
        }
    }

    /// Equivalent to the private `FSTEnum.doSeekCeilArrayContinuous`.
    fn do_seek_ceil_array_continuous(
        &mut self,
        target: &mut dyn FSTEnumTarget,
        target_label: i32,
        input: &mut dyn BytesReader,
    ) -> Result<bool> {
        let fst = self.fst;
        let upto = self.upto;
        let target_index = target_label - self.arcs[upto].first_label();
        if target_index >= self.arcs[upto].num_arcs() {
            self.rollback_to_last_fork_then_push(target)?;
            return Ok(false);
        }
        if target_index < 0 {
            fst.read_arc_by_continuous(&mut self.arcs[upto], input, 0)?;
            self.push_first(target)?;
            return Ok(false);
        }
        fst.read_arc_by_continuous(&mut self.arcs[upto], input, target_index)?;
        self.accumulate_output();
        if target_label == END_LABEL {
            return Ok(false);
        }
        let label = self.arcs[upto].label();
        target.set_current_label(upto, label);
        self.incr(target);
        let upto = self.upto;
        let (follow, arc) = arc_pair(&mut self.arcs, upto - 1, upto);
        fst.read_first_target_arc(follow, arc, &mut *self.fst_reader)?;
        Ok(true)
    }

    /// Equivalent to the private `FSTEnum.doSeekCeilArrayDirectAddressing`.
    fn do_seek_ceil_array_direct_addressing(
        &mut self,
        target: &mut dyn FSTEnumTarget,
        target_label: i32,
        input: &mut dyn BytesReader,
    ) -> Result<bool> {
        // The array is addressed directly by label, with presence bits to
        // compute the actual arc offset.
        let fst = self.fst;
        let upto = self.upto;
        let mut target_index = target_label - self.arcs[upto].first_label();
        if target_index >= self.arcs[upto].num_arcs() {
            self.rollback_to_last_fork_then_push(target)?;
            return Ok(false);
        }
        if target_index < 0 {
            target_index = -1;
        } else if BitTable::is_bit_set(target_index, &self.arcs[upto], input)? {
            fst.read_arc_by_direct_addressing(&mut self.arcs[upto], input, target_index)?;
            self.accumulate_output();
            if target_label == END_LABEL {
                return Ok(false);
            }
            let label = self.arcs[upto].label();
            target.set_current_label(upto, label);
            self.incr(target);
            let upto = self.upto;
            let (follow, arc) = arc_pair(&mut self.arcs, upto - 1, upto);
            fst.read_first_target_arc(follow, arc, &mut *self.fst_reader)?;
            return Ok(true);
        }
        // Not found: return the next arc, the ceiling.
        let ceil_index = BitTable::next_bit_set(target_index, &self.arcs[upto], input)?;
        debug_assert_ne!(ceil_index, -1);
        fst.read_arc_by_direct_addressing(&mut self.arcs[upto], input, ceil_index)?;
        self.push_first(target)?;
        Ok(false)
    }

    /// Equivalent to the private `FSTEnum.doSeekCeilArrayPacked`.
    fn do_seek_ceil_array_packed(
        &mut self,
        target: &mut dyn FSTEnumTarget,
        target_label: i32,
        input: &mut dyn BytesReader,
    ) -> Result<bool> {
        // The array is packed: use binary search to find the target.
        let fst = self.fst;
        let upto = self.upto;
        let mut idx = binary_search(fst, &self.arcs[upto], target_label)?;
        if idx >= 0 {
            // Match.
            fst.read_arc_by_index(&mut self.arcs[upto], input, idx)?;
            self.accumulate_output();
            if target_label == END_LABEL {
                return Ok(false);
            }
            let label = self.arcs[upto].label();
            target.set_current_label(upto, label);
            self.incr(target);
            let upto = self.upto;
            let (follow, arc) = arc_pair(&mut self.arcs, upto - 1, upto);
            fst.read_first_target_arc(follow, arc, &mut *self.fst_reader)?;
            return Ok(true);
        }
        idx = -1 - idx;
        if idx == self.arcs[upto].num_arcs() {
            // Dead end: the target is after the last arc; roll back to the last
            // fork, then push.
            fst.read_arc_by_index(&mut self.arcs[upto], input, idx - 1)?;
            debug_assert!(self.arcs[upto].is_last());
            self.rollback_to_last_fork_then_push(target)?;
            Ok(false)
        } else {
            // Ceiling: the arc with the least higher label.
            fst.read_arc_by_index(&mut self.arcs[upto], input, idx)?;
            self.push_first(target)?;
            Ok(false)
        }
    }

    /// Equivalent to the private `FSTEnum.doSeekCeilList`.
    fn do_seek_ceil_list(
        &mut self,
        target: &mut dyn FSTEnumTarget,
        target_label: i32,
    ) -> Result<bool> {
        // Arcs are not arrayed: a linear scan is needed.
        let fst = self.fst;
        let upto = self.upto;
        let label = self.arcs[upto].label();
        if label == target_label {
            // Recurse.
            self.accumulate_output();
            if target_label == END_LABEL {
                return Ok(false);
            }
            target.set_current_label(upto, label);
            self.incr(target);
            let upto = self.upto;
            let (follow, arc) = arc_pair(&mut self.arcs, upto - 1, upto);
            fst.read_first_target_arc(follow, arc, &mut *self.fst_reader)?;
            Ok(true)
        } else if label > target_label {
            self.push_first(target)?;
            Ok(false)
        } else if self.arcs[upto].is_last() {
            // Dead end: the target is after the last arc; roll back to the last
            // fork, then push.
            self.rollback_to_last_fork_then_push(target)?;
            Ok(false)
        } else {
            // Keep scanning.
            fst.read_next_arc(&mut self.arcs[upto], &mut *self.fst_reader)?;
            Ok(true)
        }
    }

    /// Seeks to the largest term that is less than or equal to the target.
    ///
    /// Equivalent to `FSTEnum.doSeekFloor`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn do_seek_floor(&mut self, target: &mut dyn FSTEnumTarget) -> Result<()> {
        // Save CPU by starting at the end of the shared prefix between the
        // current term and the target.
        self.rewind_prefix(target)?;

        let fst = self.fst;
        let mut input = fst.get_bytes_reader()?;

        // Now scan forward, matching the new suffix of the target.
        loop {
            let target_label = target.target_label(self.upto);
            let (bytes_per_arc, arc_label, node_flags) = {
                let arc = &self.arcs[self.upto];
                (arc.bytes_per_arc(), arc.label(), arc.node_flags())
            };
            let keep_going = if bytes_per_arc != 0 && arc_label != END_LABEL {
                // Arcs are in an array.
                if node_flags == ARCS_FOR_DIRECT_ADDRESSING {
                    self.do_seek_floor_array_direct_addressing(target, target_label, &mut *input)?
                } else if node_flags == ARCS_FOR_BINARY_SEARCH {
                    self.do_seek_floor_array_packed(target, target_label, &mut *input)?
                } else {
                    self.do_seek_floor_continuous(target, target_label, &mut *input)?
                }
            } else {
                self.do_seek_floor_list(target, target_label)?
            };
            if !keep_going {
                return Ok(());
            }
        }
    }

    /// Equivalent to the private `FSTEnum.doSeekFloorContinuous`.
    fn do_seek_floor_continuous(
        &mut self,
        target: &mut dyn FSTEnumTarget,
        target_label: i32,
        input: &mut dyn BytesReader,
    ) -> Result<bool> {
        let fst = self.fst;
        let upto = self.upto;
        let target_index = target_label - self.arcs[upto].first_label();
        if target_index < 0 {
            // Before the first arc.
            return self.backtrack_to_floor_arc(target, target_label, input);
        } else if target_index >= self.arcs[upto].num_arcs() {
            // After the last arc.
            fst.read_last_arc_by_continuous(&mut self.arcs[upto], input)?;
            self.push_last(target)?;
            return Ok(false);
        }
        // Within the label range.
        fst.read_arc_by_continuous(&mut self.arcs[upto], input, target_index)?;
        self.accumulate_output();
        if target_label == END_LABEL {
            return Ok(false);
        }
        let label = self.arcs[upto].label();
        target.set_current_label(upto, label);
        self.incr(target);
        let upto = self.upto;
        let (follow, arc) = arc_pair(&mut self.arcs, upto - 1, upto);
        fst.read_first_target_arc(follow, arc, &mut *self.fst_reader)?;
        Ok(true)
    }

    /// Equivalent to the private `FSTEnum.doSeekFloorArrayDirectAddressing`.
    fn do_seek_floor_array_direct_addressing(
        &mut self,
        target: &mut dyn FSTEnumTarget,
        target_label: i32,
        input: &mut dyn BytesReader,
    ) -> Result<bool> {
        // The array is addressed directly by label, with presence bits to
        // compute the actual arc offset.
        let fst = self.fst;
        let upto = self.upto;
        let target_index = target_label - self.arcs[upto].first_label();
        if target_index < 0 {
            // Before the first arc.
            return self.backtrack_to_floor_arc(target, target_label, input);
        } else if target_index >= self.arcs[upto].num_arcs() {
            // After the last arc.
            fst.read_last_arc_by_direct_addressing(&mut self.arcs[upto], input)?;
            self.push_last(target)?;
            return Ok(false);
        }
        // Within the label range.
        if BitTable::is_bit_set(target_index, &self.arcs[upto], input)? {
            fst.read_arc_by_direct_addressing(&mut self.arcs[upto], input, target_index)?;
            self.accumulate_output();
            if target_label == END_LABEL {
                return Ok(false);
            }
            let label = self.arcs[upto].label();
            target.set_current_label(upto, label);
            self.incr(target);
            let upto = self.upto;
            let (follow, arc) = arc_pair(&mut self.arcs, upto - 1, upto);
            fst.read_first_target_arc(follow, arc, &mut *self.fst_reader)?;
            return Ok(true);
        }
        // Scan backwards to find a floor arc.
        let floor_index = BitTable::previous_bit_set(target_index, &self.arcs[upto], input)?;
        debug_assert_ne!(floor_index, -1);
        fst.read_arc_by_direct_addressing(&mut self.arcs[upto], input, floor_index)?;
        self.push_last(target)?;
        Ok(false)
    }

    /// The target is beyond the last arc, out of the label range: roll back to
    /// the last fork, then push.
    ///
    /// Equivalent to the private `FSTEnum.rollbackToLastForkThenPush`.
    fn rollback_to_last_fork_then_push(&mut self, target: &mut dyn FSTEnumTarget) -> Result<()> {
        let fst = self.fst;
        self.upto -= 1;
        loop {
            if self.upto == 0 {
                return Ok(());
            }
            let upto = self.upto;
            if !self.arcs[upto].is_last() {
                fst.read_next_arc(&mut self.arcs[upto], &mut *self.fst_reader)?;
                return self.push_first(target);
            }
            self.upto -= 1;
        }
    }

    /// Backtracks until it finds a node whose first arc is before the target
    /// label, then finds the arc just before the target label on that node.
    ///
    /// Equivalent to the private `FSTEnum.backtrackToFloorArc`; it always ends
    /// the seek loop, so it returns `false`.
    fn backtrack_to_floor_arc(
        &mut self,
        target: &mut dyn FSTEnumTarget,
        target_label: i32,
        input: &mut dyn BytesReader,
    ) -> Result<bool> {
        let fst = self.fst;
        let mut target_label = target_label;
        loop {
            // First, walk backwards until a node is found whose first arc is
            // before the target label.
            let upto = self.upto;
            let (follow, arc) = arc_pair(&mut self.arcs, upto - 1, upto);
            fst.read_first_target_arc(follow, arc, &mut *self.fst_reader)?;
            if self.arcs[upto].label() < target_label {
                // Then, on this node, find the arc just before the target label.
                if !self.arcs[upto].is_last() {
                    if self.arcs[upto].bytes_per_arc() != 0 && self.arcs[upto].label() != END_LABEL
                    {
                        let node_flags = self.arcs[upto].node_flags();
                        if node_flags == ARCS_FOR_BINARY_SEARCH {
                            self.find_next_floor_arc_binary_search(target_label, input)?;
                        } else if node_flags == ARCS_FOR_DIRECT_ADDRESSING {
                            self.find_next_floor_arc_direct_addressing(target_label, input)?;
                        } else {
                            self.find_next_floor_arc_continuous(target_label, input)?;
                        }
                    } else {
                        while !self.arcs[upto].is_last()
                            && fst.read_next_arc_label(&self.arcs[upto], input)? < target_label
                        {
                            fst.read_next_arc(&mut self.arcs[upto], &mut *self.fst_reader)?;
                        }
                    }
                }
                self.push_last(target)?;
                return Ok(false);
            }
            self.upto -= 1;
            if self.upto == 0 {
                return Ok(false);
            }
            target_label = target.target_label(self.upto);
        }
    }

    /// Finds and reads an arc of the current node whose label is strictly less
    /// than the given label.
    ///
    /// Equivalent to the private `FSTEnum.findNextFloorArcDirectAddressing`.
    /// The precondition is that the current arc is the first arc of the node.
    fn find_next_floor_arc_direct_addressing(
        &mut self,
        target_label: i32,
        input: &mut dyn BytesReader,
    ) -> Result<()> {
        let fst = self.fst;
        let upto = self.upto;
        if self.arcs[upto].num_arcs() > 1 {
            let target_index = target_label - self.arcs[upto].first_label();
            debug_assert!(target_index >= 0);
            if target_index >= self.arcs[upto].num_arcs() {
                // Beyond the last arc: take the last arc.
                fst.read_last_arc_by_direct_addressing(&mut self.arcs[upto], input)?;
            } else {
                // Take the preceding arc, even if the target is present.
                let floor_index =
                    BitTable::previous_bit_set(target_index, &self.arcs[upto], input)?;
                if floor_index > 0 {
                    fst.read_arc_by_direct_addressing(&mut self.arcs[upto], input, floor_index)?;
                }
            }
        }
        Ok(())
    }

    /// The continuous-node counterpart of
    /// [`FSTEnum::find_next_floor_arc_direct_addressing`].
    ///
    /// Equivalent to the private `FSTEnum.findNextFloorArcContinuous`.
    fn find_next_floor_arc_continuous(
        &mut self,
        target_label: i32,
        input: &mut dyn BytesReader,
    ) -> Result<()> {
        let fst = self.fst;
        let upto = self.upto;
        if self.arcs[upto].num_arcs() > 1 {
            let target_index = target_label - self.arcs[upto].first_label();
            debug_assert!(target_index >= 0);
            if target_index >= self.arcs[upto].num_arcs() {
                // Beyond the last arc: take the last arc.
                fst.read_last_arc_by_continuous(&mut self.arcs[upto], input)?;
            } else {
                fst.read_arc_by_continuous(&mut self.arcs[upto], input, target_index - 1)?;
            }
        }
        Ok(())
    }

    /// The binary-search-node counterpart of
    /// [`FSTEnum::find_next_floor_arc_direct_addressing`].
    ///
    /// Equivalent to the private `FSTEnum.findNextFloorArcBinarySearch`.
    fn find_next_floor_arc_binary_search(
        &mut self,
        target_label: i32,
        input: &mut dyn BytesReader,
    ) -> Result<()> {
        let fst = self.fst;
        let upto = self.upto;
        if self.arcs[upto].num_arcs() > 1 {
            let idx = binary_search(fst, &self.arcs[upto], target_label)?;
            debug_assert_ne!(idx, -1);
            if idx > 1 {
                fst.read_arc_by_index(&mut self.arcs[upto], input, idx - 1)?;
            } else if idx < -2 {
                fst.read_arc_by_index(&mut self.arcs[upto], input, -2 - idx)?;
            }
        }
        Ok(())
    }

    /// Equivalent to the private `FSTEnum.doSeekFloorArrayPacked`.
    fn do_seek_floor_array_packed(
        &mut self,
        target: &mut dyn FSTEnumTarget,
        target_label: i32,
        input: &mut dyn BytesReader,
    ) -> Result<bool> {
        // Arcs are a fixed array: use binary search to find the target.
        let fst = self.fst;
        let upto = self.upto;
        let idx = binary_search(fst, &self.arcs[upto], target_label)?;

        if idx >= 0 {
            // Match: recurse.
            fst.read_arc_by_index(&mut self.arcs[upto], input, idx)?;
            self.accumulate_output();
            if target_label == END_LABEL {
                return Ok(false);
            }
            let label = self.arcs[upto].label();
            target.set_current_label(upto, label);
            self.incr(target);
            let upto = self.upto;
            let (follow, arc) = arc_pair(&mut self.arcs, upto - 1, upto);
            fst.read_first_target_arc(follow, arc, &mut *self.fst_reader)?;
            Ok(true)
        } else if idx == -1 {
            // Before the first arc.
            self.backtrack_to_floor_arc(target, target_label, input)
        } else {
            // There is a floor arc; idx is -1 - (floor + 1).
            fst.read_arc_by_index(&mut self.arcs[upto], input, -2 - idx)?;
            self.push_last(target)?;
            Ok(false)
        }
    }

    /// Equivalent to the private `FSTEnum.doSeekFloorList`.
    fn do_seek_floor_list(
        &mut self,
        target: &mut dyn FSTEnumTarget,
        target_label: i32,
    ) -> Result<bool> {
        let fst = self.fst;
        let mut target_label = target_label;
        let upto = self.upto;
        let label = self.arcs[upto].label();
        if label == target_label {
            // Match: recurse.
            self.accumulate_output();
            if target_label == END_LABEL {
                return Ok(false);
            }
            target.set_current_label(upto, label);
            self.incr(target);
            let upto = self.upto;
            let (follow, arc) = arc_pair(&mut self.arcs, upto - 1, upto);
            fst.read_first_target_arc(follow, arc, &mut *self.fst_reader)?;
            Ok(true)
        } else if label > target_label {
            // TODO(Lucene): if each arc could read the arc just before it, this
            // re-scan could be saved. The ceiling case does not need it because
            // it reads the next arc instead.
            loop {
                // First, walk backwards until a first arc is found that is
                // before the target label.
                let upto = self.upto;
                let (follow, arc) = arc_pair(&mut self.arcs, upto - 1, upto);
                fst.read_first_target_arc(follow, arc, &mut *self.fst_reader)?;
                if self.arcs[upto].label() < target_label {
                    // Then scan forwards to the arc just before the target label.
                    while !self.arcs[upto].is_last()
                        && fst.read_next_arc_label(&self.arcs[upto], &mut *self.fst_reader)?
                            < target_label
                    {
                        fst.read_next_arc(&mut self.arcs[upto], &mut *self.fst_reader)?;
                    }
                    self.push_last(target)?;
                    return Ok(false);
                }
                self.upto -= 1;
                if self.upto == 0 {
                    return Ok(false);
                }
                target_label = target.target_label(self.upto);
            }
        } else if !self.arcs[upto].is_last() {
            if fst.read_next_arc_label(&self.arcs[upto], &mut *self.fst_reader)? > target_label {
                self.push_last(target)?;
                Ok(false)
            } else {
                // Keep scanning.
                fst.read_next_arc(&mut self.arcs[upto], &mut *self.fst_reader)?;
                Ok(true)
            }
        } else {
            self.push_last(target)?;
            Ok(false)
        }
    }

    /// Seeks to exactly the target term, returning whether it exists.
    ///
    /// Equivalent to `FSTEnum.doSeekExact`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn do_seek_exact(&mut self, target: &mut dyn FSTEnumTarget) -> Result<bool> {
        // Save time by starting at the end of the shared prefix between the
        // current term and the target.
        self.rewind_prefix(target)?;

        let fst = self.fst;
        let mut target_label = target.target_label(self.upto);

        let mut input = fst.get_bytes_reader()?;

        loop {
            let upto = self.upto;
            let found = {
                let (follow, arc) = arc_pair(&mut self.arcs, upto - 1, upto);
                fst.find_target_arc(target_label, follow, arc, &mut *input)?
            };
            if !found {
                // Short circuit.
                let (follow, arc) = arc_pair(&mut self.arcs, upto - 1, upto);
                fst.read_first_target_arc(follow, arc, &mut *self.fst_reader)?;
                return Ok(false);
            }
            // Match: recurse.
            self.accumulate_output();
            if target_label == END_LABEL {
                return Ok(true);
            }
            target.set_current_label(upto, target_label);
            self.incr(target);
            target_label = target.target_label(self.upto);
        }
    }

    /// Sets `output[upto]` to `output[upto - 1]` plus the current arc's output.
    ///
    /// This is the `output[upto] = fst.outputs.add(output[upto - 1],
    /// arc.output())` line Lucene repeats in every seek branch.
    fn accumulate_output(&mut self) {
        let upto = self.upto;
        let added = self
            .fst
            .outputs()
            .add(&self.output[upto - 1], self.arcs[upto].output());
        self.output[upto] = added;
    }

    /// Equivalent to the private `FSTEnum.incr`.
    fn incr(&mut self, target: &mut dyn FSTEnumTarget) {
        self.upto += 1;
        target.grow(self.upto);
        if self.arcs.len() <= self.upto {
            let new_len = ArrayUtil::oversize(1 + self.upto, 4).max(1 + self.upto);
            self.arcs.resize(new_len, Arc::default());
        }
        if self.output.len() <= self.upto {
            let new_len = ArrayUtil::oversize(1 + self.upto, 4).max(1 + self.upto);
            self.output.resize(new_len, self.no_output.clone());
        }
    }

    /// Appends the current arc and then recurses from its target, appending the
    /// first arc all the way to the final node.
    ///
    /// Equivalent to the private `FSTEnum.pushFirst`.
    fn push_first(&mut self, target: &mut dyn FSTEnumTarget) -> Result<()> {
        let fst = self.fst;
        loop {
            self.accumulate_output();
            let upto = self.upto;
            let label = self.arcs[upto].label();
            if label == END_LABEL {
                // Final node.
                return Ok(());
            }
            target.set_current_label(upto, label);
            self.incr(target);

            let upto = self.upto;
            let (follow, arc) = arc_pair(&mut self.arcs, upto - 1, upto);
            fst.read_first_target_arc(follow, arc, &mut *self.fst_reader)?;
        }
    }

    /// Recurses from the current arc, appending the last arc all the way to the
    /// first final node.
    ///
    /// Equivalent to the private `FSTEnum.pushLast`.
    fn push_last(&mut self, target: &mut dyn FSTEnumTarget) -> Result<()> {
        let fst = self.fst;
        loop {
            let upto = self.upto;
            let label = self.arcs[upto].label();
            target.set_current_label(upto, label);
            self.accumulate_output();
            if label == END_LABEL {
                // Final node.
                return Ok(());
            }
            self.incr(target);

            let upto = self.upto;
            let (follow, arc) = arc_pair(&mut self.arcs, upto - 1, upto);
            fst.read_last_target_arc(follow, arc, &mut *self.fst_reader)?;
        }
    }
}
