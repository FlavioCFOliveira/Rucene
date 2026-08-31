//! Compressed, append-only sequences of longs.
//!
//! Ported from `org.apache.lucene.util.packed.PackedLongValues`,
//! `org.apache.lucene.util.packed.DeltaPackedLongValues` and
//! `org.apache.lucene.util.packed.MonotonicLongValues` of Apache Lucene Core
//! 10.5.0, together with their nested builders and iterator.
//!
//! Java models the three as an inheritance chain — `MonotonicLongValues`
//! extends `DeltaPackedLongValues` extends `PackedLongValues` — and dispatches
//! `get(int, int)` and `decodeBlock` virtually. This port keeps the three names
//! and composes them in the same order, with [`PackedLongValuesAccess`] and
//! [`PackedLongValuesBuilderOps`] carrying the virtual methods.

#![warn(missing_docs)]

use super::block_packed::expected;
use super::reader::{NullReader, PackedIntsReader};
use super::PackedInts;
use crate::document::column::LongValuesCursor;
use crate::error::{LuceneError, Result};
use crate::util::{Accountable, ArrayUtil, LongValues, RamUsageEstimator};

/// The heap cost of a `PackedLongValues` instance itself.
///
/// Mirrors `RamUsageEstimator.shallowSizeOfInstance(PackedLongValues.class)`:
/// an object header, the `values` reference, two `int`s and two `long`s.
const PACKED_LONG_VALUES_BASE_RAM_BYTES_USED: i64 = 40;
/// The heap cost of a `DeltaPackedLongValues` instance itself.
///
/// The [`PackedLongValues`] cost plus the `mins` reference.
const DELTA_PACKED_LONG_VALUES_BASE_RAM_BYTES_USED: i64 = 48;
/// The heap cost of a `MonotonicLongValues` instance itself.
///
/// The [`DeltaPackedLongValues`] cost plus the `averages` reference.
const MONOTONIC_LONG_VALUES_BASE_RAM_BYTES_USED: i64 = 56;
/// The heap cost of a `PackedLongValues.Builder` instance itself.
const PACKED_LONG_VALUES_BUILDER_BASE_RAM_BYTES_USED: i64 = 56;
/// The heap cost of a `DeltaPackedLongValues.Builder` instance itself.
const DELTA_BUILDER_BASE_RAM_BYTES_USED: i64 = 64;
/// The heap cost of a `MonotonicLongValues.Builder` instance itself.
const MONOTONIC_BUILDER_BASE_RAM_BYTES_USED: i64 = 72;

/// The number of pages a builder starts with.
///
/// Equivalent to `PackedLongValues.Builder.INITIAL_PAGE_COUNT`.
const INITIAL_PAGE_COUNT: usize = 16;

/// A compressed, random-access sequence of longs.
///
/// Equivalent to `org.apache.lucene.util.packed.PackedLongValues`.
pub struct PackedLongValues {
    values: Vec<Box<dyn PackedIntsReader>>,
    page_shift: i32,
    page_mask: i32,
    size: i64,
    ram_bytes_used: i64,
}

impl PackedLongValues {
    /// The page size a builder uses unless one is given.
    ///
    /// Equivalent to `PackedLongValues.DEFAULT_PAGE_SIZE`.
    pub const DEFAULT_PAGE_SIZE: i32 = 256;
    /// The smallest page a builder accepts.
    ///
    /// Equivalent to `PackedLongValues.MIN_PAGE_SIZE`.
    pub const MIN_PAGE_SIZE: usize = 64;
    /// The largest page a builder accepts.
    ///
    /// Equivalent to `PackedLongValues.MAX_PAGE_SIZE`; more than a million
    /// values per page defeats the purpose of these buffers, which is to keep
    /// the number of bits per value small.
    pub const MAX_PAGE_SIZE: usize = 1 << 20;

    /// Returns a builder that compresses non-negative integers efficiently.
    ///
    /// Equivalent to `PackedLongValues.packedBuilder(int, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `page_size` is not a power
    /// of two in `[MIN_PAGE_SIZE, MAX_PAGE_SIZE]`.
    pub fn packed_builder(
        page_size: usize,
        acceptable_overhead_ratio: f32,
    ) -> Result<PackedLongValuesBuilder> {
        PackedLongValuesBuilder::new(page_size, acceptable_overhead_ratio)
    }

    /// Returns a builder that compresses integers close to one another.
    ///
    /// Equivalent to `PackedLongValues.deltaPackedBuilder(int, float)`.
    ///
    /// # Errors
    ///
    /// Returns the error [`PackedLongValues::packed_builder`] raises.
    pub fn delta_packed_builder(
        page_size: usize,
        acceptable_overhead_ratio: f32,
    ) -> Result<DeltaPackedLongValuesBuilder> {
        DeltaPackedLongValuesBuilder::new(page_size, acceptable_overhead_ratio)
    }

    /// Returns a builder that compresses integers that are close to a monotonic
    /// function of their index.
    ///
    /// Equivalent to `PackedLongValues.monotonicBuilder(int, float)`.
    ///
    /// # Errors
    ///
    /// Returns the error [`PackedLongValues::packed_builder`] raises.
    pub fn monotonic_builder(
        page_size: usize,
        acceptable_overhead_ratio: f32,
    ) -> Result<MonotonicLongValuesBuilder> {
        MonotonicLongValuesBuilder::new(page_size, acceptable_overhead_ratio)
    }
}

/// Reads the values of one page into `dest` and returns how many there were.
///
/// The body of `PackedLongValues.decodeBlock(int, long[])`, which the two
/// subclasses reach with `super.decodeBlock(...)`.
fn base_decode_block(values: &PackedLongValues, block: usize, dest: &mut [i64]) -> i32 {
    let vals = &values.values[block];
    let size = vals.size();
    let mut k = 0;
    while k < size {
        k += vals.get_bulk(k, dest, k as usize, (size - k) as usize);
    }
    size
}

/// The random access a [`PackedLongValues`] and its two refinements provide.
///
/// Equivalent to the methods Lucene dispatches virtually across
/// `PackedLongValues`, `DeltaPackedLongValues` and `MonotonicLongValues`:
/// `get(int, int)` and `decodeBlock(int, long[])`, plus the final `get(long)`
/// and `iterator()` built on them.
pub trait PackedLongValuesAccess {
    /// Borrows the page storage shared by every refinement.
    fn base(&self) -> &PackedLongValues;

    /// Returns the value at `element` of `block`.
    ///
    /// Equivalent to `PackedLongValues.get(int, int)`.
    fn get_block_element(&self, block: usize, element: i32) -> i64;

    /// Decodes the whole of `block` into `dest` and returns how many values it
    /// held.
    ///
    /// Equivalent to `PackedLongValues.decodeBlock(int, long[])`.
    fn decode_block(&self, block: usize, dest: &mut [i64]) -> i32;

    /// The number of values.
    ///
    /// Equivalent to `PackedLongValues.size()`.
    fn size(&self) -> i64 {
        self.base().size
    }

    /// Returns the value at `index`.
    ///
    /// Equivalent to `PackedLongValues.get(long)`.
    fn get(&self, index: i64) -> i64 {
        debug_assert!(index >= 0 && index < self.size());
        let base = self.base();
        let block = (index >> base.page_shift) as usize;
        let element = (index & i64::from(base.page_mask)) as i32;
        self.get_block_element(block, element)
    }

    /// Returns an iterator over every value.
    ///
    /// Equivalent to `PackedLongValues.iterator()`.
    fn iterator(&self) -> PackedLongValuesIterator<'_, Self> {
        PackedLongValuesIterator::new(self)
    }
}

impl PackedLongValuesAccess for PackedLongValues {
    fn base(&self) -> &PackedLongValues {
        self
    }

    fn get_block_element(&self, block: usize, element: i32) -> i64 {
        self.values[block].get(element)
    }

    fn decode_block(&self, block: usize, dest: &mut [i64]) -> i32 {
        base_decode_block(self, block, dest)
    }
}

impl PackedLongValues {
    /// Returns the value at `index`.
    ///
    /// Equivalent to `PackedLongValues.get(long)`, which Java declares
    /// `final`; the inherent method keeps the call unambiguous next to the
    /// [`LongValues`] and [`PackedLongValuesAccess`] implementations.
    pub fn get(&self, index: i64) -> i64 {
        PackedLongValuesAccess::get(self, index)
    }

    /// The number of values.
    ///
    /// Equivalent to `PackedLongValues.size()`.
    pub fn size(&self) -> i64 {
        PackedLongValuesAccess::size(self)
    }

    /// Returns an iterator over every value.
    ///
    /// Equivalent to `PackedLongValues.iterator()`.
    pub fn iterator(&self) -> PackedLongValuesIterator<'_, PackedLongValues> {
        PackedLongValuesAccess::iterator(self)
    }
}

impl LongValues for PackedLongValues {
    fn get(&self, index: i64) -> i64 {
        PackedLongValuesAccess::get(self, index)
    }
}

impl Accountable for PackedLongValues {
    fn ram_bytes_used(&self) -> i64 {
        self.ram_bytes_used
    }
}

/// A [`PackedLongValues`] that stores each page as a delta from its minimum.
///
/// Equivalent to `org.apache.lucene.util.packed.DeltaPackedLongValues`.
pub struct DeltaPackedLongValues {
    base: PackedLongValues,
    mins: Vec<i64>,
}

impl PackedLongValuesAccess for DeltaPackedLongValues {
    fn base(&self) -> &PackedLongValues {
        &self.base
    }

    fn get_block_element(&self, block: usize, element: i32) -> i64 {
        self.mins[block].wrapping_add(self.base.values[block].get(element))
    }

    fn decode_block(&self, block: usize, dest: &mut [i64]) -> i32 {
        let count = base_decode_block(&self.base, block, dest);
        let min = self.mins[block];
        for slot in dest.iter_mut().take(count as usize) {
            *slot = slot.wrapping_add(min);
        }
        count
    }
}

impl DeltaPackedLongValues {
    /// Returns the value at `index`.
    ///
    /// Equivalent to `PackedLongValues.get(long)`, which Java declares
    /// `final`; the inherent method keeps the call unambiguous next to the
    /// [`LongValues`] and [`PackedLongValuesAccess`] implementations.
    pub fn get(&self, index: i64) -> i64 {
        PackedLongValuesAccess::get(self, index)
    }

    /// The number of values.
    ///
    /// Equivalent to `PackedLongValues.size()`.
    pub fn size(&self) -> i64 {
        PackedLongValuesAccess::size(self)
    }

    /// Returns an iterator over every value.
    ///
    /// Equivalent to `PackedLongValues.iterator()`.
    pub fn iterator(&self) -> PackedLongValuesIterator<'_, DeltaPackedLongValues> {
        PackedLongValuesAccess::iterator(self)
    }
}

impl LongValues for DeltaPackedLongValues {
    fn get(&self, index: i64) -> i64 {
        PackedLongValuesAccess::get(self, index)
    }
}

impl Accountable for DeltaPackedLongValues {
    fn ram_bytes_used(&self) -> i64 {
        self.base.ram_bytes_used
    }
}

/// A [`DeltaPackedLongValues`] that also removes a linear trend from each page.
///
/// Equivalent to `org.apache.lucene.util.packed.MonotonicLongValues`.
pub struct MonotonicLongValues {
    base: DeltaPackedLongValues,
    averages: Vec<f32>,
}

impl PackedLongValuesAccess for MonotonicLongValues {
    fn base(&self) -> &PackedLongValues {
        &self.base.base
    }

    fn get_block_element(&self, block: usize, element: i32) -> i64 {
        expected(self.base.mins[block], self.averages[block], element)
            .wrapping_add(self.base.base.values[block].get(element))
    }

    fn decode_block(&self, block: usize, dest: &mut [i64]) -> i32 {
        let count = self.base.decode_block(block, dest);
        let average = self.averages[block];
        for (i, slot) in dest.iter_mut().take(count as usize).enumerate() {
            *slot = slot.wrapping_add(expected(0, average, i as i32));
        }
        count
    }
}

impl MonotonicLongValues {
    /// Returns the value at `index`.
    ///
    /// Equivalent to `PackedLongValues.get(long)`, which Java declares
    /// `final`; the inherent method keeps the call unambiguous next to the
    /// [`LongValues`] and [`PackedLongValuesAccess`] implementations.
    pub fn get(&self, index: i64) -> i64 {
        PackedLongValuesAccess::get(self, index)
    }

    /// The number of values.
    ///
    /// Equivalent to `PackedLongValues.size()`.
    pub fn size(&self) -> i64 {
        PackedLongValuesAccess::size(self)
    }

    /// Returns an iterator over every value.
    ///
    /// Equivalent to `PackedLongValues.iterator()`.
    pub fn iterator(&self) -> PackedLongValuesIterator<'_, MonotonicLongValues> {
        PackedLongValuesAccess::iterator(self)
    }
}

impl LongValues for MonotonicLongValues {
    fn get(&self, index: i64) -> i64 {
        PackedLongValuesAccess::get(self, index)
    }
}

impl Accountable for MonotonicLongValues {
    fn ram_bytes_used(&self) -> i64 {
        self.base.base.ram_bytes_used
    }
}

/// An iterator over the values of a [`PackedLongValues`].
///
/// Equivalent to the inner class `PackedLongValues.Iterator`.
pub struct PackedLongValuesIterator<'a, V: PackedLongValuesAccess + ?Sized> {
    values: &'a V,
    current_values: Vec<i64>,
    v_off: usize,
    p_off: usize,
    current_count: usize,
}

impl<'a, V: PackedLongValuesAccess + ?Sized> PackedLongValuesIterator<'a, V> {
    fn new(values: &'a V) -> Self {
        let base = values.base();
        let capacity = std::cmp::min(base.size, i64::from(base.page_mask) + 1) as usize;
        let mut this = Self {
            values,
            current_values: vec![0i64; capacity],
            v_off: 0,
            p_off: 0,
            current_count: 0,
        };
        this.fill_block();
        this
    }

    /// Equivalent to `PackedLongValues.Iterator.fillBlock()`.
    fn fill_block(&mut self) {
        if self.v_off == self.values.base().values.len() {
            self.current_count = 0;
        } else {
            self.current_count =
                self.values
                    .decode_block(self.v_off, &mut self.current_values) as usize;
            debug_assert!(self.current_count > 0);
        }
    }

    /// Returns whether any value remains.
    ///
    /// Equivalent to `PackedLongValues.Iterator.hasNext()`.
    pub fn has_next(&self) -> bool {
        self.p_off < self.current_count
    }

    /// Returns the next value.
    ///
    /// Equivalent to `PackedLongValues.Iterator.next()`.
    ///
    /// # Panics
    ///
    /// Panics when no value remains; Lucene asserts `hasNext()` here.
    pub fn next_value(&mut self) -> i64 {
        debug_assert!(self.has_next());
        let result = self.current_values[self.p_off];
        self.p_off += 1;
        if self.p_off == self.current_count {
            self.v_off += 1;
            self.p_off = 0;
            self.fill_block();
        }
        result
    }
}

impl<V: PackedLongValuesAccess + ?Sized> Iterator for PackedLongValuesIterator<'_, V> {
    type Item = i64;

    fn next(&mut self) -> Option<i64> {
        if self.has_next() {
            Some(self.next_value())
        } else {
            None
        }
    }
}

// -----------------------------------------------------------------------------
// Builders
// -----------------------------------------------------------------------------

/// The state every `PackedLongValues.Builder` holds.
///
/// Equivalent to the fields of the nested class
/// `org.apache.lucene.util.packed.PackedLongValues.Builder`. Java's `pending`
/// is set to `null` by `build()` to mark the builder spent; the [`Option`]
/// carries the same meaning.
pub struct PackedLongValuesBuilderState {
    page_shift: i32,
    page_mask: i32,
    acceptable_overhead_ratio: f32,
    pending: Option<Vec<i64>>,
    size: i64,
    values: Vec<Option<Box<dyn PackedIntsReader>>>,
    ram_bytes_used: i64,
    values_off: usize,
    pending_off: usize,
}

impl PackedLongValuesBuilderState {
    fn new(
        page_size: usize,
        acceptable_overhead_ratio: f32,
        base_ram_bytes_used: i64,
    ) -> Result<Self> {
        let page_shift = PackedInts::check_block_size(
            page_size,
            PackedLongValues::MIN_PAGE_SIZE,
            PackedLongValues::MAX_PAGE_SIZE,
        )?;
        let mut values = Vec::with_capacity(INITIAL_PAGE_COUNT);
        values.resize_with(INITIAL_PAGE_COUNT, || None);
        let pending = vec![0i64; page_size];
        let ram_bytes_used = base_ram_bytes_used
            + RamUsageEstimator::size_of_long(&pending)
            + RamUsageEstimator::shallow_size_of(&values);
        Ok(Self {
            page_shift,
            page_mask: page_size as i32 - 1,
            acceptable_overhead_ratio,
            pending: Some(pending),
            size: 0,
            values,
            ram_bytes_used,
            values_off: 0,
            pending_off: 0,
        })
    }

    fn pending_mut(&mut self) -> Result<&mut Vec<i64>> {
        self.pending
            .as_mut()
            .ok_or_else(|| LuceneError::IllegalState("Cannot be reused after build()".to_string()))
    }
}

/// The body of `PackedLongValues.Builder.pack(long[], int, int, float)`.
///
/// The two builder refinements reach it with `super.pack(...)` once they have
/// removed their own component from the pending values.
fn base_pack(
    state: &mut PackedLongValuesBuilderState,
    num_values: usize,
    block: usize,
) -> Result<()> {
    debug_assert!(num_values > 0);
    let pending = state
        .pending
        .as_ref()
        .ok_or_else(|| LuceneError::IllegalState("Cannot be reused after build()".to_string()))?;

    // compute max delta
    let mut min_value = pending[0];
    let mut max_value = pending[0];
    for value in &pending[1..num_values] {
        min_value = std::cmp::min(min_value, *value);
        max_value = std::cmp::max(max_value, *value);
    }

    // build a new packed reader
    if min_value == 0 && max_value == 0 {
        state.values[block] = Some(Box::new(NullReader::for_count(num_values as i32)));
    } else {
        let bits_required = if min_value < 0 {
            64
        } else {
            PackedInts::bits_required(max_value)?
        };
        let mut mutable = PackedInts::get_mutable(
            num_values as i32,
            bits_required,
            state.acceptable_overhead_ratio,
        )?;
        let pending = state
            .pending
            .as_ref()
            .expect("INVARIANT: checked above in this call");
        let mut i = 0usize;
        while i < num_values {
            i += mutable.set_bulk(i as i32, pending, i, num_values - i) as usize;
        }
        state.values[block] = Some(mutable.into_packed_ints_reader());
    }
    Ok(())
}

/// The body of `PackedLongValues.Builder.grow(int)`.
fn base_grow(state: &mut PackedLongValuesBuilderState, new_block_count: usize) {
    state.ram_bytes_used -= RamUsageEstimator::shallow_size_of(&state.values);
    state.values.resize_with(new_block_count, || None);
    state.ram_bytes_used += RamUsageEstimator::shallow_size_of(&state.values);
}

/// The body of `DeltaPackedLongValues.Builder.pack(long[], int, int, float)`.
fn delta_pack(
    state: &mut PackedLongValuesBuilderState,
    mins: &mut [i64],
    num_values: usize,
    block: usize,
) -> Result<()> {
    {
        let pending = state.pending_mut()?;
        let mut min = pending[0];
        for value in &pending[1..num_values] {
            min = std::cmp::min(min, *value);
        }
        for value in &mut pending[..num_values] {
            *value = value.wrapping_sub(min);
        }
        mins[block] = min;
    }
    base_pack(state, num_values, block)
}

/// The operations every `PackedLongValues.Builder` provides.
///
/// Equivalent to the nested class
/// `org.apache.lucene.util.packed.PackedLongValues.Builder`, with its two
/// virtual hooks — `pack` and `grow` — declared as required methods.
pub trait PackedLongValuesBuilderOps {
    /// Borrows the shared builder state.
    fn state(&self) -> &PackedLongValuesBuilderState;

    /// Borrows the shared builder state mutably.
    fn state_mut(&mut self) -> &mut PackedLongValuesBuilderState;

    /// The heap cost of the builder object itself.
    ///
    /// Equivalent to `PackedLongValues.Builder.baseRamBytesUsed()`.
    fn base_ram_bytes_used(&self) -> i64;

    /// Compresses the first `num_values` pending values into page `block`.
    ///
    /// Equivalent to `PackedLongValues.Builder.pack(long[], int, int, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the builder was already
    /// consumed by `build`, or the error the packed array constructor raises.
    fn pack_block(&mut self, num_values: usize, block: usize) -> Result<()>;

    /// Grows the page table to `new_block_count` entries.
    ///
    /// Equivalent to `PackedLongValues.Builder.grow(int)`.
    fn grow_blocks(&mut self, new_block_count: usize);

    /// The number of values added so far.
    ///
    /// Equivalent to `PackedLongValues.Builder.size()`.
    fn size(&self) -> i64 {
        self.state().size
    }

    /// The heap cost of this builder.
    ///
    /// Equivalent to `PackedLongValues.Builder.ramBytesUsed()`.
    fn ram_bytes_used(&self) -> i64 {
        self.state().ram_bytes_used
    }

    /// Compresses the pending page if it is full.
    ///
    /// Equivalent to `PackedLongValues.Builder.packIfFull()`.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::pack_block`] raises.
    fn pack_if_full(&mut self) -> Result<()> {
        let pending_len = self.state().pending.as_ref().map(Vec::len).ok_or_else(|| {
            LuceneError::IllegalState("Cannot be reused after build()".to_string())
        })?;
        if self.state().pending_off == pending_len {
            if self.state().values.len() == self.state().values_off {
                let new_length = ArrayUtil::oversize(self.state().values_off + 1, 8);
                self.grow_blocks(new_length);
            }
            self.pack()?;
        }
        Ok(())
    }

    /// Compresses the pending page.
    ///
    /// Equivalent to `PackedLongValues.Builder.pack()`.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::pack_block`] raises.
    fn pack(&mut self) -> Result<()> {
        let num_values = self.state().pending_off;
        let block = self.state().values_off;
        self.pack_block(num_values, block)?;
        let page_bytes = self.state().values[block]
            .as_ref()
            .map_or(0, |page| page.ram_bytes_used());
        let state = self.state_mut();
        state.ram_bytes_used += page_bytes;
        state.values_off += 1;
        // reset pending buffer
        state.pending_off = 0;
        Ok(())
    }

    /// Compresses whatever is still pending.
    ///
    /// Equivalent to `PackedLongValues.Builder.finish()`.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::pack_block`] raises.
    fn finish(&mut self) -> Result<()> {
        if self.state().pending_off > 0 {
            if self.state().values.len() == self.state().values_off {
                let new_length = self.state().values_off + 1;
                self.grow_blocks(new_length);
            }
            self.pack()?;
        }
        Ok(())
    }

    /// Adds a value.
    ///
    /// Equivalent to `PackedLongValues.Builder.add(long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the builder was already
    /// consumed by `build`, or the error [`Self::pack_block`] raises.
    fn add(&mut self, l: i64) -> Result<()> {
        if self.state().pending.is_none() {
            return Err(LuceneError::IllegalState(
                "Cannot be reused after build()".to_string(),
            ));
        }
        self.pack_if_full()?;
        let state = self.state_mut();
        let pending_off = state.pending_off;
        state
            .pending
            .as_mut()
            .expect("INVARIANT: checked above in this call")[pending_off] = l;
        state.pending_off += 1;
        state.size += 1;
        Ok(())
    }

    /// Adds `count` copies of `value`.
    ///
    /// Equivalent to `PackedLongValues.Builder.add(long, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the builder was already
    /// consumed by `build`, or the error [`Self::pack_block`] raises.
    fn add_repeated(&mut self, value: i64, count: i32) -> Result<()> {
        if self.state().pending.is_none() {
            return Err(LuceneError::IllegalState(
                "Cannot be reused after build()".to_string(),
            ));
        }
        let mut count = count;
        while count > 0 {
            self.pack_if_full()?;
            let state = self.state_mut();
            let pending_off = state.pending_off;
            let pending = state
                .pending
                .as_mut()
                .expect("INVARIANT: checked above in this call");
            let to_fill = std::cmp::min(count as usize, pending.len() - pending_off);
            pending[pending_off..pending_off + to_fill].fill(value);
            state.pending_off += to_fill;
            count -= to_fill as i32;
            state.size += to_fill as i64;
        }
        Ok(())
    }

    /// Adds every value the cursor produces.
    ///
    /// Equivalent to `PackedLongValues.Builder.add(LongValuesCursor)`, which
    /// uses the cursor's own `size()` as the bound.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the builder was already
    /// consumed by `build`, the error [`Self::pack_block`] raises, or the error
    /// the cursor raises.
    fn add_cursor(&mut self, cursor: &mut dyn LongValuesCursor) -> Result<()> {
        if self.state().pending.is_none() {
            return Err(LuceneError::IllegalState(
                "Cannot be reused after build()".to_string(),
            ));
        }
        let mut remaining = cursor.size();
        while remaining > 0 {
            self.pack_if_full()?;
            let state = self.state_mut();
            let pending_off = state.pending_off;
            let pending = state
                .pending
                .as_mut()
                .expect("INVARIANT: checked above in this call");
            let to_fill = std::cmp::min(remaining as usize, pending.len() - pending_off);
            cursor.fill_doc_values(pending, pending_off, to_fill)?;
            state.pending_off += to_fill;
            remaining -= to_fill as i32;
            state.size += to_fill as i64;
        }
        Ok(())
    }
}

/// Takes the first `values_off` pages out of the builder state.
///
/// Equivalent to `ArrayUtil.copyOfSubArray(this.values, 0, valuesOff)` in the
/// three `build()` methods; the pages are moved rather than copied, because
/// Rust cannot share one owned reader between the builder and the result.
fn take_pages(state: &mut PackedLongValuesBuilderState) -> Vec<Box<dyn PackedIntsReader>> {
    let values_off = state.values_off;
    let mut pages = Vec::with_capacity(values_off);
    for slot in state.values.iter_mut().take(values_off) {
        if let Some(page) = slot.take() {
            pages.push(page);
        }
    }
    pages
}

/// Builds a [`PackedLongValues`].
///
/// Equivalent to `org.apache.lucene.util.packed.PackedLongValues.Builder`.
pub struct PackedLongValuesBuilder {
    state: PackedLongValuesBuilderState,
}

impl PackedLongValuesBuilder {
    /// Creates a builder with pages of `page_size` values.
    ///
    /// Equivalent to `new PackedLongValues.Builder(int, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `page_size` is not a power
    /// of two in `[MIN_PAGE_SIZE, MAX_PAGE_SIZE]`.
    pub fn new(page_size: usize, acceptable_overhead_ratio: f32) -> Result<Self> {
        Ok(Self {
            state: PackedLongValuesBuilderState::new(
                page_size,
                acceptable_overhead_ratio,
                PACKED_LONG_VALUES_BUILDER_BASE_RAM_BYTES_USED,
            )?,
        })
    }

    /// Builds the compressed sequence. This consumes the builder.
    ///
    /// Equivalent to `PackedLongValues.Builder.build()`.
    ///
    /// # Errors
    ///
    /// Returns the error the final [`PackedLongValuesBuilderOps::pack_block`]
    /// raises.
    pub fn build(mut self) -> Result<PackedLongValues> {
        self.finish()?;
        self.state.pending = None;
        let values = take_pages(&mut self.state);
        let ram_bytes_used = PACKED_LONG_VALUES_BASE_RAM_BYTES_USED
            + RamUsageEstimator::shallow_size_of(&values)
            + values.iter().map(|page| page.ram_bytes_used()).sum::<i64>();
        Ok(PackedLongValues {
            values,
            page_shift: self.state.page_shift,
            page_mask: self.state.page_mask,
            size: self.state.size,
            ram_bytes_used,
        })
    }
}

impl PackedLongValuesBuilderOps for PackedLongValuesBuilder {
    fn state(&self) -> &PackedLongValuesBuilderState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut PackedLongValuesBuilderState {
        &mut self.state
    }

    fn base_ram_bytes_used(&self) -> i64 {
        PACKED_LONG_VALUES_BUILDER_BASE_RAM_BYTES_USED
    }

    fn pack_block(&mut self, num_values: usize, block: usize) -> Result<()> {
        base_pack(&mut self.state, num_values, block)
    }

    fn grow_blocks(&mut self, new_block_count: usize) {
        base_grow(&mut self.state, new_block_count);
    }
}

/// Builds a [`DeltaPackedLongValues`].
///
/// Equivalent to
/// `org.apache.lucene.util.packed.DeltaPackedLongValues.Builder`.
pub struct DeltaPackedLongValuesBuilder {
    state: PackedLongValuesBuilderState,
    mins: Vec<i64>,
}

impl DeltaPackedLongValuesBuilder {
    /// Creates a builder with pages of `page_size` values.
    ///
    /// Equivalent to `new DeltaPackedLongValues.Builder(int, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `page_size` is not a power
    /// of two in `[MIN_PAGE_SIZE, MAX_PAGE_SIZE]`.
    pub fn new(page_size: usize, acceptable_overhead_ratio: f32) -> Result<Self> {
        let mut state = PackedLongValuesBuilderState::new(
            page_size,
            acceptable_overhead_ratio,
            DELTA_BUILDER_BASE_RAM_BYTES_USED,
        )?;
        let mins = vec![0i64; state.values.len()];
        state.ram_bytes_used += RamUsageEstimator::size_of_long(&mins);
        Ok(Self { state, mins })
    }

    /// Builds the compressed sequence. This consumes the builder.
    ///
    /// Equivalent to `DeltaPackedLongValues.Builder.build()`.
    ///
    /// # Errors
    ///
    /// Returns the error the final [`PackedLongValuesBuilderOps::pack_block`]
    /// raises.
    pub fn build(mut self) -> Result<DeltaPackedLongValues> {
        self.finish()?;
        self.state.pending = None;
        let values = take_pages(&mut self.state);
        let mins = self.mins[..self.state.values_off].to_vec();
        let ram_bytes_used = DELTA_PACKED_LONG_VALUES_BASE_RAM_BYTES_USED
            + RamUsageEstimator::shallow_size_of(&values)
            + values.iter().map(|page| page.ram_bytes_used()).sum::<i64>()
            + RamUsageEstimator::size_of_long(&mins);
        Ok(DeltaPackedLongValues {
            base: PackedLongValues {
                values,
                page_shift: self.state.page_shift,
                page_mask: self.state.page_mask,
                size: self.state.size,
                ram_bytes_used,
            },
            mins,
        })
    }
}

impl PackedLongValuesBuilderOps for DeltaPackedLongValuesBuilder {
    fn state(&self) -> &PackedLongValuesBuilderState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut PackedLongValuesBuilderState {
        &mut self.state
    }

    fn base_ram_bytes_used(&self) -> i64 {
        DELTA_BUILDER_BASE_RAM_BYTES_USED
    }

    fn pack_block(&mut self, num_values: usize, block: usize) -> Result<()> {
        delta_pack(&mut self.state, &mut self.mins, num_values, block)
    }

    fn grow_blocks(&mut self, new_block_count: usize) {
        base_grow(&mut self.state, new_block_count);
        self.state.ram_bytes_used -= RamUsageEstimator::size_of_long(&self.mins);
        self.mins.resize(new_block_count, 0);
        self.state.ram_bytes_used += RamUsageEstimator::size_of_long(&self.mins);
    }
}

/// Builds a [`MonotonicLongValues`].
///
/// Equivalent to `org.apache.lucene.util.packed.MonotonicLongValues.Builder`.
pub struct MonotonicLongValuesBuilder {
    state: PackedLongValuesBuilderState,
    mins: Vec<i64>,
    averages: Vec<f32>,
}

impl MonotonicLongValuesBuilder {
    /// Creates a builder with pages of `page_size` values.
    ///
    /// Equivalent to `new MonotonicLongValues.Builder(int, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `page_size` is not a power
    /// of two in `[MIN_PAGE_SIZE, MAX_PAGE_SIZE]`.
    pub fn new(page_size: usize, acceptable_overhead_ratio: f32) -> Result<Self> {
        let mut state = PackedLongValuesBuilderState::new(
            page_size,
            acceptable_overhead_ratio,
            MONOTONIC_BUILDER_BASE_RAM_BYTES_USED,
        )?;
        let mins = vec![0i64; state.values.len()];
        let averages = vec![0f32; state.values.len()];
        state.ram_bytes_used += RamUsageEstimator::size_of_long(&mins);
        state.ram_bytes_used += size_of_float(&averages);
        Ok(Self {
            state,
            mins,
            averages,
        })
    }

    /// Builds the compressed sequence. This consumes the builder.
    ///
    /// Equivalent to `MonotonicLongValues.Builder.build()`.
    ///
    /// # Errors
    ///
    /// Returns the error the final [`PackedLongValuesBuilderOps::pack_block`]
    /// raises.
    pub fn build(mut self) -> Result<MonotonicLongValues> {
        self.finish()?;
        self.state.pending = None;
        let values = take_pages(&mut self.state);
        let mins = self.mins[..self.state.values_off].to_vec();
        let averages = self.averages[..self.state.values_off].to_vec();
        let ram_bytes_used = MONOTONIC_LONG_VALUES_BASE_RAM_BYTES_USED
            + RamUsageEstimator::shallow_size_of(&values)
            + values.iter().map(|page| page.ram_bytes_used()).sum::<i64>()
            + RamUsageEstimator::size_of_long(&mins)
            + size_of_float(&averages);
        Ok(MonotonicLongValues {
            base: DeltaPackedLongValues {
                base: PackedLongValues {
                    values,
                    page_shift: self.state.page_shift,
                    page_mask: self.state.page_mask,
                    size: self.state.size,
                    ram_bytes_used,
                },
                mins,
            },
            averages,
        })
    }
}

/// The heap cost of a `float[]`.
///
/// Mirrors `RamUsageEstimator.sizeOf(float[])`, which this crate's estimator
/// does not yet expose.
fn size_of_float(array: &[f32]) -> i64 {
    RamUsageEstimator::align_object_size(
        RamUsageEstimator::NUM_BYTES_ARRAY_HEADER + 4 * array.len() as i64,
    )
}

impl PackedLongValuesBuilderOps for MonotonicLongValuesBuilder {
    fn state(&self) -> &PackedLongValuesBuilderState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut PackedLongValuesBuilderState {
        &mut self.state
    }

    fn base_ram_bytes_used(&self) -> i64 {
        MONOTONIC_BUILDER_BASE_RAM_BYTES_USED
    }

    fn pack_block(&mut self, num_values: usize, block: usize) -> Result<()> {
        {
            let pending = self.state.pending_mut()?;
            let average = if num_values == 1 {
                0f32
            } else {
                (pending[num_values - 1].wrapping_sub(pending[0])) as f32 / (num_values - 1) as f32
            };
            for (i, value) in pending[..num_values].iter_mut().enumerate() {
                *value = value.wrapping_sub(expected(0, average, i as i32));
            }
            self.averages[block] = average;
        }
        delta_pack(&mut self.state, &mut self.mins, num_values, block)
    }

    fn grow_blocks(&mut self, new_block_count: usize) {
        base_grow(&mut self.state, new_block_count);
        self.state.ram_bytes_used -= RamUsageEstimator::size_of_long(&self.mins);
        self.mins.resize(new_block_count, 0);
        self.state.ram_bytes_used += RamUsageEstimator::size_of_long(&self.mins);
        self.state.ram_bytes_used -= size_of_float(&self.averages);
        self.averages.resize(new_block_count, 0f32);
        self.state.ram_bytes_used += size_of_float(&self.averages);
    }
}
