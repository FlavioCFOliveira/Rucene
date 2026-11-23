// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.Edits`.

use std::fmt;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use rucene::error::{LuceneError, Result};

/// `MAX_UNCHANGED_LENGTH`.
const MAX_UNCHANGED_LENGTH: i32 = 0x1000;
/// `MAX_UNCHANGED`.
const MAX_UNCHANGED: i32 = MAX_UNCHANGED_LENGTH - 1;
/// `MAX_SHORT_CHANGE_OLD_LENGTH`.
const MAX_SHORT_CHANGE_OLD_LENGTH: i32 = 6;
/// `MAX_SHORT_CHANGE_NEW_LENGTH`.
const MAX_SHORT_CHANGE_NEW_LENGTH: i32 = 7;
/// `SHORT_CHANGE_NUM_MASK`.
const SHORT_CHANGE_NUM_MASK: i32 = 0x1ff;
/// `MAX_SHORT_CHANGE`.
const MAX_SHORT_CHANGE: i32 = 0x6fff;
/// `LENGTH_IN_1TRAIL`.
const LENGTH_IN_1TRAIL: i32 = 61;
/// `LENGTH_IN_2TRAIL`.
const LENGTH_IN_2TRAIL: i32 = 62;
/// `STACK_CAPACITY`.
const STACK_CAPACITY: usize = 100;

/// A `char[]` that an [`Edits`] and its iterators share, as Java's
/// iterators keep a reference to the array the `Edits` had when they were
/// created (a later in-place update of a unit is visible to them; a grown
/// array is not).
type SharedArray = Arc<[AtomicU16]>;

fn new_array(capacity: usize) -> SharedArray {
    (0..capacity).map(|_| AtomicU16::new(0)).collect()
}

fn get(array: &SharedArray, i: i32) -> i32 {
    array[i as usize].load(Ordering::Relaxed) as i32
}

fn set(array: &SharedArray, i: i32, v: i32) {
    array[i as usize].store(v as u16, Ordering::Relaxed);
}

/// Records lengths of string edits but not replacement text. Supports
/// replacements, insertions, deletions in linear progression. Does not
/// support moving/reordering of text.
///
/// Equivalent to `com.ibm.icu.text.Edits`.
pub struct Edits {
    array: SharedArray,
    length: i32,
    delta: i32,
    num_changes: i32,
}

impl Default for Edits {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Edits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Edits")
            .field("length", &self.length)
            .field("delta", &self.delta)
            .field("num_changes", &self.num_changes)
            .finish()
    }
}

impl Edits {
    /// `Edits()`: an empty object.
    pub fn new() -> Self {
        Self {
            array: new_array(STACK_CAPACITY),
            length: 0,
            delta: 0,
            num_changes: 0,
        }
    }

    /// `reset()`: empties the object.
    pub fn reset(&mut self) {
        self.length = 0;
        self.delta = 0;
        self.num_changes = 0;
    }

    /// `setLastUnit(int last)` (private).
    fn set_last_unit(&mut self, last: i32) {
        set(&self.array, self.length - 1, last);
    }

    /// `lastUnit()` (private).
    fn last_unit(&self) -> i32 {
        if self.length > 0 {
            get(&self.array, self.length - 1)
        } else {
            0xffff
        }
    }

    /// `addUnchanged(int unchangedLength)`: adds a record for an unchanged
    /// segment of text.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a negative length;
    /// `BufferOverflowException` when the record array cannot grow.
    pub fn add_unchanged(&mut self, mut unchanged_length: i32) -> Result<()> {
        if unchanged_length < 0 {
            return Err(LuceneError::illegal_argument(format!(
                "addUnchanged({unchanged_length}): length must not be negative"
            )));
        }
        // Merge into previous unchanged-text record, if any.
        let last = self.last_unit();
        if last < MAX_UNCHANGED {
            let remaining = MAX_UNCHANGED - last;
            if remaining >= unchanged_length {
                self.set_last_unit(last + unchanged_length);
                return Ok(());
            }
            self.set_last_unit(MAX_UNCHANGED);
            unchanged_length -= remaining;
        }
        // Split large lengths into multiple units.
        while unchanged_length >= MAX_UNCHANGED_LENGTH {
            self.append(MAX_UNCHANGED)?;
            unchanged_length -= MAX_UNCHANGED_LENGTH;
        }
        // Write a small (remaining) length.
        if unchanged_length > 0 {
            self.append(unchanged_length - 1)?;
        }
        Ok(())
    }

    /// `addReplace(int oldLength, int newLength)`: adds a record for a text
    /// replacement/insertion/deletion.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a negative length;
    /// `BufferOverflowException` when the record array cannot grow.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when the length delta overflows an `int`.
    pub fn add_replace(&mut self, old_length: i32, new_length: i32) -> Result<()> {
        if old_length < 0 || new_length < 0 {
            return Err(LuceneError::illegal_argument(format!(
                "addReplace({old_length}, {new_length}): both lengths must be non-negative"
            )));
        }
        if old_length == 0 && new_length == 0 {
            return Ok(());
        }
        self.num_changes += 1;
        let new_delta = new_length - old_length;
        if new_delta != 0 {
            if (new_delta > 0 && self.delta >= 0 && new_delta > (i32::MAX - self.delta))
                || (new_delta < 0 && self.delta < 0 && new_delta < (i32::MIN - self.delta))
            {
                // Integer overflow or underflow.
                crate::jdk::index_out_of_bounds(None);
            }
            self.delta += new_delta;
        }

        if 0 < old_length && old_length <= MAX_SHORT_CHANGE_OLD_LENGTH && new_length <= MAX_SHORT_CHANGE_NEW_LENGTH {
            // Merge into previous same-lengths short-replacement record, if any.
            let u = (old_length << 12) | (new_length << 9);
            let last = self.last_unit();
            if MAX_UNCHANGED < last
                && last < MAX_SHORT_CHANGE
                && (last & !SHORT_CHANGE_NUM_MASK) == u
                && (last & SHORT_CHANGE_NUM_MASK) < SHORT_CHANGE_NUM_MASK
            {
                // Increment the count of this record.
                self.set_last_unit(last + 1);
                return Ok(());
            }
            self.append(u)?;
            return Ok(());
        }

        let mut head = 0x7000;
        if old_length < LENGTH_IN_1TRAIL && new_length < LENGTH_IN_1TRAIL {
            head |= old_length << 6;
            head |= new_length;
            self.append(head)?;
        } else if (self.array.len() as i32 - self.length) >= 5 || self.grow_array()? {
            let mut limit = self.length + 1;
            if old_length < LENGTH_IN_1TRAIL {
                head |= old_length << 6;
            } else if old_length <= 0x7fff {
                head |= LENGTH_IN_1TRAIL << 6;
                set(&self.array, limit, 0x8000 | old_length);
                limit += 1;
            } else {
                head |= (LENGTH_IN_2TRAIL + (old_length >> 30)) << 6;
                set(&self.array, limit, 0x8000 | (old_length >> 15));
                limit += 1;
                set(&self.array, limit, 0x8000 | old_length);
                limit += 1;
            }
            if new_length < LENGTH_IN_1TRAIL {
                head |= new_length;
            } else if new_length <= 0x7fff {
                head |= LENGTH_IN_1TRAIL;
                set(&self.array, limit, 0x8000 | new_length);
                limit += 1;
            } else {
                head |= LENGTH_IN_2TRAIL + (new_length >> 30);
                set(&self.array, limit, 0x8000 | (new_length >> 15));
                limit += 1;
                set(&self.array, limit, 0x8000 | new_length);
                limit += 1;
            }
            set(&self.array, self.length, head);
            self.length = limit;
        }
        Ok(())
    }

    /// `append(int r)` (private).
    fn append(&mut self, r: i32) -> Result<()> {
        if self.length < self.array.len() as i32 || self.grow_array()? {
            set(&self.array, self.length, r);
            self.length += 1;
        }
        Ok(())
    }

    /// `growArray()` (private).
    fn grow_array(&mut self) -> Result<bool> {
        let len = self.array.len() as i32;
        let new_capacity = if self.array.len() == STACK_CAPACITY {
            2000
        } else if len == i32::MAX {
            return Err(crate::jdk::unchecked_no_message("java.nio.BufferOverflowException"));
        } else if len >= (i32::MAX / 2) {
            i32::MAX
        } else {
            2 * len
        };
        // Grow by at least 5 units so that a maximal change record will fit.
        if (new_capacity - len) < 5 {
            return Err(crate::jdk::unchecked_no_message("java.nio.BufferOverflowException"));
        }
        let new = new_array(new_capacity as usize);
        for (i, v) in self.array.iter().enumerate() {
            new[i].store(v.load(Ordering::Relaxed), Ordering::Relaxed);
        }
        self.array = new;
        Ok(true)
    }

    /// `lengthDelta()`: how much longer the destination is than the source.
    pub fn length_delta(&self) -> i32 {
        self.delta
    }

    /// `hasChanges()`: true if there are any change edits.
    pub fn has_changes(&self) -> bool {
        self.num_changes != 0
    }

    /// `numberOfChanges()`: the number of change edits.
    pub fn number_of_changes(&self) -> i32 {
        self.num_changes
    }

    /// `getCoarseChangesIterator()`: adjacent changes merged, unchanged
    /// spans skipped.
    pub fn get_coarse_changes_iterator(&self) -> Iterator {
        Iterator::new(self.array.clone(), self.length, true, true)
    }

    /// `getCoarseIterator()`: adjacent changes merged.
    pub fn get_coarse_iterator(&self) -> Iterator {
        Iterator::new(self.array.clone(), self.length, false, true)
    }

    /// `getFineChangesIterator()`: every change separately, unchanged spans
    /// skipped.
    pub fn get_fine_changes_iterator(&self) -> Iterator {
        Iterator::new(self.array.clone(), self.length, true, false)
    }

    /// `getFineIterator()`: every change separately.
    pub fn get_fine_iterator(&self) -> Iterator {
        Iterator::new(self.array.clone(), self.length, false, false)
    }

    /// `mergeAndAppend(Edits ab, Edits bc)`: merges the edits of a→b and
    /// b→c into a→c and appends them to this object.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the ab output and bc input lengths
    /// differ; the errors of [`add_replace`](Self::add_replace) and
    /// [`add_unchanged`](Self::add_unchanged).
    pub fn merge_and_append(&mut self, ab: &Edits, bc: &Edits) -> Result<&mut Self> {
        // Picture string a --(Edits ab)--> string b --(Edits bc)--> string c.
        // Parallel iteration over both Edits.
        let mut ab_iter = ab.get_fine_iterator();
        let mut bc_iter = bc.get_fine_iterator();
        let mut ab_has_next = true;
        let mut bc_has_next = true;
        // Copy iterator state into local variables, so that we can modify and subdivide spans.
        // ab old & new length, bc old & new length
        let mut a_length = 0;
        let mut ab_b_length = 0;
        let mut bc_b_length = 0;
        let mut c_length = 0;
        // When we have different-intermediate-length changes, we accumulate a larger change.
        let mut pending_a_length = 0;
        let mut pending_c_length = 0;
        loop {
            // At this point, for each of the two iterators:
            // Either we are done with the locally cached current edit,
            // and its intermediate-string length has been reset,
            // or we will continue to work with a truncated remainder of this edit.
            //
            // If the current edit is done, and the iterator has not yet reached the end,
            // then we fetch the next edit. This is true for at least one of the iterators.
            //
            // Normally it does not matter whether we fetch from ab and then bc or vice versa.
            // However, the result is observably different when
            // ab deletions meet bc insertions at the same intermediate-string index.
            // Some users expect the bc insertions to come first, so we fetch from bc first.
            if bc_b_length == 0 {
                if bc_has_next && {
                    bc_has_next = bc_iter.next();
                    bc_has_next
                } {
                    bc_b_length = bc_iter.old_length();
                    c_length = bc_iter.new_length();
                    if bc_b_length == 0 {
                        // insertion
                        if ab_b_length == 0 || !ab_iter.has_change() {
                            self.add_replace(pending_a_length, pending_c_length + c_length)?;
                            pending_a_length = 0;
                            pending_c_length = 0;
                        } else {
                            pending_c_length += c_length;
                        }
                        continue;
                    }
                }
                // else see if the other iterator is done, too.
            }
            if ab_b_length == 0 {
                if ab_has_next && {
                    ab_has_next = ab_iter.next();
                    ab_has_next
                } {
                    a_length = ab_iter.old_length();
                    ab_b_length = ab_iter.new_length();
                    if ab_b_length == 0 {
                        // deletion
                        if bc_b_length == bc_iter.old_length() || !bc_iter.has_change() {
                            self.add_replace(pending_a_length + a_length, pending_c_length)?;
                            pending_a_length = 0;
                            pending_c_length = 0;
                        } else {
                            pending_a_length += a_length;
                        }
                        continue;
                    }
                } else if bc_b_length == 0 {
                    // Both iterators are done at the same time:
                    // The intermediate-string lengths match.
                    break;
                } else {
                    return Err(LuceneError::illegal_argument(
                        "The ab output string is shorter than the bc input string.",
                    ));
                }
            }
            if bc_b_length == 0 {
                return Err(LuceneError::illegal_argument(
                    "The bc input string is shorter than the ab output string.",
                ));
            }
            //  Done fetching: ab_bLength > 0 && bc_bLength > 0

            // The current state has two parts:
            // - Past: We accumulate a longer ac edit in the "pending" variables.
            // - Current: We have copies of the current ab/bc edits in local variables.
            //   At least one side is newly fetched.
            //   One side might be a truncated remainder of an edit we fetched earlier.

            if !ab_iter.has_change() && !bc_iter.has_change() {
                // An unchanged span all the way from string a to string c.
                if pending_a_length != 0 || pending_c_length != 0 {
                    self.add_replace(pending_a_length, pending_c_length)?;
                    pending_a_length = 0;
                    pending_c_length = 0;
                }
                let unchanged_length = if a_length <= c_length { a_length } else { c_length };
                self.add_unchanged(unchanged_length)?;
                a_length -= unchanged_length;
                ab_b_length = a_length;
                c_length -= unchanged_length;
                bc_b_length = c_length;
                // At least one of the unchanged spans is now empty.
                continue;
            }
            if !ab_iter.has_change() && bc_iter.has_change() {
                // Unchanged a->b but changed b->c.
                if ab_b_length >= bc_b_length {
                    // Split the longer unchanged span into change + remainder.
                    self.add_replace(pending_a_length + bc_b_length, pending_c_length + c_length)?;
                    pending_a_length = 0;
                    pending_c_length = 0;
                    ab_b_length -= bc_b_length;
                    a_length = ab_b_length;
                    bc_b_length = 0;
                    continue;
                }
                // Handle the shorter unchanged span below like a change.
            } else if ab_iter.has_change() && !bc_iter.has_change() {
                // Changed a->b and then unchanged b->c.
                if ab_b_length <= bc_b_length {
                    // Split the longer unchanged span into change + remainder.
                    self.add_replace(pending_a_length + a_length, pending_c_length + ab_b_length)?;
                    pending_a_length = 0;
                    pending_c_length = 0;
                    bc_b_length -= ab_b_length;
                    c_length = bc_b_length;
                    ab_b_length = 0;
                    continue;
                }
                // Handle the shorter unchanged span below like a change.
            } else {
                // both abIter.hasChange() && bcIter.hasChange()
                if ab_b_length == bc_b_length {
                    // Changes on both sides up to the same position. Emit & reset.
                    self.add_replace(pending_a_length + a_length, pending_c_length + c_length)?;
                    pending_a_length = 0;
                    pending_c_length = 0;
                    ab_b_length = 0;
                    bc_b_length = 0;
                    continue;
                }
            }
            // Accumulate the a->c change, reset the shorter side,
            // keep a remainder of the longer one.
            pending_a_length += a_length;
            pending_c_length += c_length;
            if ab_b_length < bc_b_length {
                bc_b_length -= ab_b_length;
                ab_b_length = 0;
                c_length = 0;
            } else {
                // ab_bLength > bc_bLength
                ab_b_length -= bc_b_length;
                bc_b_length = 0;
                a_length = 0;
            }
        }
        if pending_a_length != 0 || pending_c_length != 0 {
            self.add_replace(pending_a_length, pending_c_length)?;
        }
        Ok(self)
    }
}

/// Access to the list of edits.
///
/// Equivalent to `com.ibm.icu.text.Edits.Iterator`: it reads the record
/// array the [`Edits`] had when the iterator was created.
pub struct Iterator {
    array: SharedArray,
    index: i32,
    length: i32,
    /// 0 if we are not within compressed equal-length changes.
    /// Otherwise the number of remaining changes, including the current one.
    remaining: i32,
    only_changes: bool,
    coarse: bool,
    /// 0: no current edit, 1: last call was next(), -1: last call was previous().
    dir: i8,
    changed: bool,
    old_length: i32,
    new_length: i32,
    src_index: i32,
    repl_index: i32,
    dest_index: i32,
}

impl Iterator {
    /// `Iterator(char[] a, int len, boolean oc, boolean crs)` (private).
    fn new(a: SharedArray, len: i32, oc: bool, crs: bool) -> Self {
        Self {
            array: a,
            index: 0,
            length: len,
            remaining: 0,
            only_changes: oc,
            coarse: crs,
            dir: 0,
            changed: false,
            old_length: 0,
            new_length: 0,
            src_index: 0,
            repl_index: 0,
            dest_index: 0,
        }
    }

    fn at(&self, i: i32) -> i32 {
        get(&self.array, i)
    }

    /// `readLength(int head)` (private).
    fn read_length(&mut self, head: i32) -> i32 {
        if head < LENGTH_IN_1TRAIL {
            head
        } else if head < LENGTH_IN_2TRAIL {
            debug_assert!(self.index < self.length);
            debug_assert!(self.at(self.index) >= 0x8000);
            let v = self.at(self.index) & 0x7fff;
            self.index += 1;
            v
        } else {
            debug_assert!((self.index + 2) <= self.length);
            debug_assert!(self.at(self.index) >= 0x8000);
            debug_assert!(self.at(self.index + 1) >= 0x8000);
            let len = ((head & 1) << 30) | ((self.at(self.index) & 0x7fff) << 15) | (self.at(self.index + 1) & 0x7fff);
            self.index += 2;
            len
        }
    }

    /// `updateNextIndexes()` (private).
    fn update_next_indexes(&mut self) {
        self.src_index += self.old_length;
        if self.changed {
            self.repl_index += self.new_length;
        }
        self.dest_index += self.new_length;
    }

    /// `updatePreviousIndexes()` (private).
    fn update_previous_indexes(&mut self) {
        self.src_index -= self.old_length;
        if self.changed {
            self.repl_index -= self.new_length;
        }
        self.dest_index -= self.new_length;
    }

    /// `noNext()` (private).
    fn no_next(&mut self) -> bool {
        // No change before or beyond the string.
        self.dir = 0;
        self.changed = false;
        self.old_length = 0;
        self.new_length = 0;
        false
    }

    /// `next()`: advances to the next edit; false at the end.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> bool {
        self.next_with(self.only_changes)
    }

    /// `next(boolean onlyChanges)` (private).
    fn next_with(&mut self, only_changes: bool) -> bool {
        // Forward iteration: Update the string indexes to the limit of the current span,
        // and post-increment-read array units to assemble a new span.
        // Leaves the array index one after the last unit of that span.
        if self.dir > 0 {
            self.update_next_indexes();
        } else {
            if self.dir < 0 {
                // Turn around from previous() to next().
                // Post-increment-read the same span again.
                if self.remaining > 0 {
                    // Fine-grained iterator:
                    // Stay on the current one of a sequence of compressed changes.
                    self.index += 1; // next() rests on the index after the sequence unit.
                    self.dir = 1;
                    return true;
                }
            }
            self.dir = 1;
        }
        if self.remaining >= 1 {
            // Fine-grained iterator: Continue a sequence of compressed changes.
            if self.remaining > 1 {
                self.remaining -= 1;
                return true;
            }
            self.remaining = 0;
        }
        if self.index >= self.length {
            return self.no_next();
        }
        let mut u = self.at(self.index);
        self.index += 1;
        if u <= MAX_UNCHANGED {
            // Combine adjacent unchanged ranges.
            self.changed = false;
            self.old_length = u + 1;
            while self.index < self.length && {
                u = self.at(self.index);
                u <= MAX_UNCHANGED
            } {
                self.index += 1;
                self.old_length += u + 1;
            }
            self.new_length = self.old_length;
            if only_changes {
                self.update_next_indexes();
                if self.index >= self.length {
                    return self.no_next();
                }
                // already fetched u > MAX_UNCHANGED at index
                self.index += 1;
            } else {
                return true;
            }
        }
        self.changed = true;
        if u <= MAX_SHORT_CHANGE {
            let old_len = u >> 12;
            let new_len = (u >> 9) & MAX_SHORT_CHANGE_NEW_LENGTH;
            let num = (u & SHORT_CHANGE_NUM_MASK) + 1;
            if self.coarse {
                self.old_length = num * old_len;
                self.new_length = num * new_len;
            } else {
                // Split a sequence of changes that was compressed into one unit.
                self.old_length = old_len;
                self.new_length = new_len;
                if num > 1 {
                    self.remaining = num; // This is the first of two or more changes.
                }
                return true;
            }
        } else {
            debug_assert!(u <= 0x7fff);
            self.old_length = self.read_length((u >> 6) & 0x3f);
            self.new_length = self.read_length(u & 0x3f);
            if !self.coarse {
                return true;
            }
        }
        // Combine adjacent changes.
        while self.index < self.length && {
            u = self.at(self.index);
            u > MAX_UNCHANGED
        } {
            self.index += 1;
            if u <= MAX_SHORT_CHANGE {
                let num = (u & SHORT_CHANGE_NUM_MASK) + 1;
                self.old_length += (u >> 12) * num;
                self.new_length += ((u >> 9) & MAX_SHORT_CHANGE_NEW_LENGTH) * num;
            } else {
                debug_assert!(u <= 0x7fff);
                let o = self.read_length((u >> 6) & 0x3f);
                self.old_length += o;
                let n = self.read_length(u & 0x3f);
                self.new_length += n;
            }
        }
        true
    }

    /// `previous()` (private).
    fn previous(&mut self) -> bool {
        // Backward iteration: Pre-decrement-read array units to assemble a new span,
        // then update the string indexes to the start of that span.
        // Leaves the array index on the head unit of that span.
        if self.dir >= 0 {
            if self.dir > 0 {
                // Turn around from next() to previous().
                // Set the string indexes to the span limit and
                // pre-decrement-read the same span again.
                if self.remaining > 0 {
                    // Fine-grained iterator:
                    // Stay on the current one of a sequence of compressed changes.
                    self.index -= 1; // previous() rests on the sequence unit.
                    self.dir = -1;
                    return true;
                }
                self.update_next_indexes();
            }
            self.dir = -1;
        }
        if self.remaining > 0 {
            // Fine-grained iterator: Continue a sequence of compressed changes.
            let u = self.at(self.index);
            debug_assert!(MAX_UNCHANGED < u && u <= MAX_SHORT_CHANGE);
            if self.remaining <= (u & SHORT_CHANGE_NUM_MASK) {
                self.remaining += 1;
                self.update_previous_indexes();
                return true;
            }
            self.remaining = 0;
        }
        if self.index <= 0 {
            return self.no_next();
        }
        self.index -= 1;
        let mut u = self.at(self.index);
        if u <= MAX_UNCHANGED {
            // Combine adjacent unchanged ranges.
            self.changed = false;
            self.old_length = u + 1;
            while self.index > 0 && {
                u = self.at(self.index - 1);
                u <= MAX_UNCHANGED
            } {
                self.index -= 1;
                self.old_length += u + 1;
            }
            self.new_length = self.old_length;
            // No need to handle onlyChanges as long as previous() is called only from findIndex().
            self.update_previous_indexes();
            return true;
        }
        self.changed = true;
        if u <= MAX_SHORT_CHANGE {
            let old_len = u >> 12;
            let new_len = (u >> 9) & MAX_SHORT_CHANGE_NEW_LENGTH;
            let num = (u & SHORT_CHANGE_NUM_MASK) + 1;
            if self.coarse {
                self.old_length = num * old_len;
                self.new_length = num * new_len;
            } else {
                // Split a sequence of changes that was compressed into one unit.
                self.old_length = old_len;
                self.new_length = new_len;
                if num > 1 {
                    self.remaining = 1; // This is the last of two or more changes.
                }
                self.update_previous_indexes();
                return true;
            }
        } else {
            if u <= 0x7fff {
                // The change is encoded in u alone.
                self.old_length = self.read_length((u >> 6) & 0x3f);
                self.new_length = self.read_length(u & 0x3f);
            } else {
                // Back up to the head of the change, read the lengths,
                // and reset the index to the head again.
                debug_assert!(self.index > 0);
                loop {
                    self.index -= 1;
                    u = self.at(self.index);
                    if u <= 0x7fff {
                        break;
                    }
                }
                debug_assert!(u > MAX_SHORT_CHANGE);
                let head_index = self.index;
                self.index += 1;
                self.old_length = self.read_length((u >> 6) & 0x3f);
                self.new_length = self.read_length(u & 0x3f);
                self.index = head_index;
            }
            if !self.coarse {
                self.update_previous_indexes();
                return true;
            }
        }
        // Combine adjacent changes.
        while self.index > 0 && {
            u = self.at(self.index - 1);
            u > MAX_UNCHANGED
        } {
            self.index -= 1;
            if u <= MAX_SHORT_CHANGE {
                let num = (u & SHORT_CHANGE_NUM_MASK) + 1;
                self.old_length += (u >> 12) * num;
                self.new_length += ((u >> 9) & MAX_SHORT_CHANGE_NEW_LENGTH) * num;
            } else if u <= 0x7fff {
                // Read the lengths, and reset the index to the head again.
                let head_index = self.index;
                self.index += 1;
                let o = self.read_length((u >> 6) & 0x3f);
                self.old_length += o;
                let n = self.read_length(u & 0x3f);
                self.new_length += n;
                self.index = head_index;
            }
        }
        self.update_previous_indexes();
        true
    }

    /// `findSourceIndex(int i)`: moves to the edit containing source index
    /// `i`; false when it is beyond the source.
    pub fn find_source_index(&mut self, i: i32) -> bool {
        self.find_index(i, true) == 0
    }

    /// `findDestinationIndex(int i)`.
    pub fn find_destination_index(&mut self, i: i32) -> bool {
        self.find_index(i, false) == 0
    }

    /// `findIndex(int i, boolean findSource)` (private): 0 when found, -1
    /// for a negative index, 1 beyond the string.
    fn find_index(&mut self, i: i32, find_source: bool) -> i32 {
        if i < 0 {
            return -1;
        }
        let mut span_start;
        let mut span_length;
        if find_source {
            // find source index
            span_start = self.src_index;
            span_length = self.old_length;
        } else {
            // find destination index
            span_start = self.dest_index;
            span_length = self.new_length;
        }
        if i < span_start {
            if i >= (span_start / 2) {
                // Search backwards.
                loop {
                    let has_previous = self.previous();
                    debug_assert!(has_previous); // because i>=0 and the first span starts at 0
                    span_start = if find_source { self.src_index } else { self.dest_index };
                    if i >= span_start {
                        // The index is in the current span.
                        return 0;
                    }
                    if self.remaining > 0 {
                        // Is the index in one of the remaining compressed edits?
                        // spanStart is the start of the current span, first of the remaining ones.
                        span_length = if find_source { self.old_length } else { self.new_length };
                        let u = self.at(self.index);
                        debug_assert!(MAX_UNCHANGED < u && u <= MAX_SHORT_CHANGE);
                        let num = (u & SHORT_CHANGE_NUM_MASK) + 1 - self.remaining;
                        let len = num * span_length;
                        if i >= (span_start - len) {
                            let n = ((span_start - i - 1) / span_length) + 1;
                            // 1 <= n <= num
                            self.src_index -= n * self.old_length;
                            self.repl_index -= n * self.new_length;
                            self.dest_index -= n * self.new_length;
                            self.remaining += n;
                            return 0;
                        }
                        // Skip all of these edits at once.
                        self.src_index -= num * self.old_length;
                        self.repl_index -= num * self.new_length;
                        self.dest_index -= num * self.new_length;
                        self.remaining = 0;
                    }
                }
            }
            // Reset the iterator to the start.
            self.dir = 0;
            self.index = 0;
            self.remaining = 0;
            self.old_length = 0;
            self.new_length = 0;
            self.src_index = 0;
            self.repl_index = 0;
            self.dest_index = 0;
        } else if i < (span_start + span_length) {
            // The index is in the current span.
            return 0;
        }
        while self.next_with(false) {
            if find_source {
                span_start = self.src_index;
                span_length = self.old_length;
            } else {
                span_start = self.dest_index;
                span_length = self.new_length;
            }
            if i < (span_start + span_length) {
                // The index is in the current span.
                return 0;
            }
            if self.remaining > 1 {
                // Is the index in one of the remaining compressed edits?
                // spanStart is the start of the current span, first of the remaining ones.
                let len = self.remaining * span_length;
                if i < (span_start + len) {
                    let n = (i - span_start) / span_length; // 1 <= n <= remaining - 1
                    self.src_index += n * self.old_length;
                    self.repl_index += n * self.new_length;
                    self.dest_index += n * self.new_length;
                    self.remaining -= n;
                    return 0;
                }
                // Make next() skip all of these edits at once.
                self.old_length *= self.remaining;
                self.new_length *= self.remaining;
                self.remaining = 0;
            }
        }
        1
    }

    /// `destinationIndexFromSourceIndex(int i)`.
    pub fn destination_index_from_source_index(&mut self, i: i32) -> i32 {
        let where_ = self.find_index(i, true);
        if where_ < 0 {
            // Clamp to the start of the string.
            return 0;
        }
        if where_ > 0 || i == self.src_index {
            // At or after string length, or at start of the found span.
            return self.dest_index;
        }
        if self.changed {
            // In a change span, map to its end.
            self.dest_index + self.new_length
        } else {
            // In an unchanged span, offset 1:1 within it.
            self.dest_index + (i - self.src_index)
        }
    }

    /// `sourceIndexFromDestinationIndex(int i)`.
    pub fn source_index_from_destination_index(&mut self, i: i32) -> i32 {
        let where_ = self.find_index(i, false);
        if where_ < 0 {
            // Clamp to the start of the string.
            return 0;
        }
        if where_ > 0 || i == self.dest_index {
            // At or after string length, or at start of the found span.
            return self.src_index;
        }
        if self.changed {
            // In a change span, map to its end.
            self.src_index + self.old_length
        } else {
            // In an unchanged span, offset within it.
            self.src_index + (i - self.dest_index)
        }
    }

    /// `hasChange()`: true if this edit replaces old with new text.
    pub fn has_change(&self) -> bool {
        self.changed
    }

    /// `oldLength()`: the length of the source span.
    pub fn old_length(&self) -> i32 {
        self.old_length
    }

    /// `newLength()`: the length of the destination span.
    pub fn new_length(&self) -> i32 {
        self.new_length
    }

    /// `sourceIndex()`.
    pub fn source_index(&self) -> i32 {
        self.src_index
    }

    /// `replacementIndex()`: the index into the replacement-characters-only
    /// string, not counting unchanged spans.
    pub fn replacement_index(&self) -> i32 {
        // TODO: Throw an exception if we aren't in a change edit?
        self.repl_index
    }

    /// `destinationIndex()`.
    pub fn destination_index(&self) -> i32 {
        self.dest_index
    }
}

impl fmt::Display for Iterator {
    /// `toString()`: `super.toString()` (Java's class name and identity
    /// hash; here the class name and the iterator's address, the identity
    /// a Rust value has) followed by the current span.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "com.ibm.icu.text.Edits$Iterator@{:x}", self as *const Self as usize)?;
        write!(f, "{{ src[{}..{}", self.src_index, self.src_index + self.old_length)?;
        if self.changed {
            f.write_str("] ⇝ dest[")?;
        } else {
            f.write_str("] ≡ dest[")?;
        }
        write!(f, "{}..{}", self.dest_index, self.dest_index + self.new_length)?;
        if self.changed {
            write!(f, "], repl[{}..{}] }}", self.repl_index, self.repl_index + self.new_length)
        } else {
            f.write_str("] (no-change) }")
        }
    }
}
