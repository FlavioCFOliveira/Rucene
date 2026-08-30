//! Paged packed arrays that can hold more than two billion values.
//!
//! Ported from `org.apache.lucene.util.packed.AbstractPagedMutable`,
//! `org.apache.lucene.util.packed.PagedMutable` and
//! `org.apache.lucene.util.packed.PagedGrowableWriter` of Apache Lucene Core
//! 10.5.0.

#![warn(missing_docs)]

use super::growable_writer::GrowableWriter;
use super::reader::PackedIntsMutable;
use super::{Format, PackedInts};
use crate::error::Result;
use crate::util::{Accountable, LongValues, RamUsageEstimator};

/// The smallest page a paged mutable accepts.
///
/// Equivalent to `AbstractPagedMutable.MIN_BLOCK_SIZE`.
pub const MIN_BLOCK_SIZE: usize = 1 << 6;
/// The largest page a paged mutable accepts.
///
/// Equivalent to `AbstractPagedMutable.MAX_BLOCK_SIZE`.
pub const MAX_BLOCK_SIZE: usize = 1 << 30;

/// The state shared by [`PagedMutable`] and [`PagedGrowableWriter`].
///
/// Equivalent to the fields and the concrete methods of the abstract class
/// `org.apache.lucene.util.packed.AbstractPagedMutable<T>`. Java expresses the
/// two subclasses through a self-typed generic parameter; Rust expresses the
/// same split as this state struct plus the [`AbstractPagedMutableOps`] trait,
/// which supplies the two abstract hooks.
pub struct AbstractPagedMutable {
    size: i64,
    page_shift: i32,
    page_mask: i32,
    sub_mutables: Vec<Option<Box<dyn PackedIntsMutable>>>,
    bits_per_value: i32,
}

impl AbstractPagedMutable {
    /// Creates the shared state for `size` values over pages of `page_size`.
    ///
    /// Equivalent to `AbstractPagedMutable(int, long, int)`. The pages are left
    /// unallocated; [`AbstractPagedMutableOps::fill_pages`] allocates them.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument) when `page_size` is not a power
    /// of two in `[MIN_BLOCK_SIZE, MAX_BLOCK_SIZE]`, or when `size` is
    /// negative or too large for that page size.
    pub fn new(bits_per_value: i32, size: i64, page_size: usize) -> Result<Self> {
        let page_shift = PackedInts::check_block_size(page_size, MIN_BLOCK_SIZE, MAX_BLOCK_SIZE)?;
        let num_pages = PackedInts::num_blocks(size, page_size)?;
        let mut sub_mutables = Vec::with_capacity(num_pages);
        sub_mutables.resize_with(num_pages, || None);
        Ok(Self {
            size,
            page_shift,
            page_mask: page_size as i32 - 1,
            sub_mutables,
            bits_per_value,
        })
    }

    /// The number of values.
    ///
    /// Equivalent to `AbstractPagedMutable.size()`.
    pub fn size(&self) -> i64 {
        self.size
    }

    /// The number of values per page.
    ///
    /// Equivalent to `AbstractPagedMutable.pageSize()`.
    pub fn page_size(&self) -> usize {
        self.page_mask as usize + 1
    }

    /// The number of bits per value the pages start from.
    ///
    /// Equivalent to reading `AbstractPagedMutable.bitsPerValue`.
    pub fn bits_per_value(&self) -> i32 {
        self.bits_per_value
    }

    /// The number of values on the last page of an array of `size` values.
    ///
    /// Equivalent to `AbstractPagedMutable.lastPageSize(long)`.
    pub fn last_page_size(&self, size: i64) -> i32 {
        let sz = self.index_in_page(size);
        if sz == 0 {
            self.page_size() as i32
        } else {
            sz
        }
    }

    /// The page that holds `index`.
    ///
    /// Equivalent to `AbstractPagedMutable.pageIndex(long)`.
    pub fn page_index(&self, index: i64) -> usize {
        ((index as u64) >> self.page_shift) as usize
    }

    /// The offset of `index` inside its page.
    ///
    /// Equivalent to `AbstractPagedMutable.indexInPage(long)`.
    pub fn index_in_page(&self, index: i64) -> i32 {
        (index as i32) & self.page_mask
    }

    /// Returns the value at `index`.
    ///
    /// Equivalent to `AbstractPagedMutable.get(long)`.
    ///
    /// # Panics
    ///
    /// Panics when `index` is outside `[0, size)`, or when the pages have not
    /// been filled yet. Lucene asserts the same range and would otherwise fail
    /// with a null-pointer or index error.
    pub fn get(&self, index: i64) -> i64 {
        debug_assert!(
            index >= 0 && index < self.size,
            "index={index} size={}",
            self.size
        );
        let page_index = self.page_index(index);
        let index_in_page = self.index_in_page(index);
        self.sub_mutables[page_index]
            .as_ref()
            .expect("INVARIANT: the pages were filled before any access")
            .get(index_in_page)
    }

    /// Sets the value at `index`.
    ///
    /// Equivalent to `AbstractPagedMutable.set(long, long)`.
    ///
    /// # Panics
    ///
    /// Panics under the same conditions as [`Self::get`].
    pub fn set(&mut self, index: i64, value: i64) {
        debug_assert!(index >= 0 && index < self.size);
        let page_index = self.page_index(index);
        let index_in_page = self.index_in_page(index);
        self.sub_mutables[page_index]
            .as_mut()
            .expect("INVARIANT: the pages were filled before any access")
            .set(index_in_page, value);
    }

    /// The number of pages.
    pub fn page_count(&self) -> usize {
        self.sub_mutables.len()
    }

    /// Borrows the page at `page_index`.
    pub fn page(&self, page_index: usize) -> Option<&dyn PackedIntsMutable> {
        self.sub_mutables[page_index].as_deref()
    }

    /// The heap cost of the fields this state holds, before the pages.
    ///
    /// Equivalent to `AbstractPagedMutable.baseRamBytesUsed()`.
    pub fn base_ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
            + RamUsageEstimator::NUM_BYTES_OBJECT_REF
            + 8 // size
            + 3 * 4 // pageShift, pageMask, bitsPerValue
    }

    /// The heap cost of this state and every page, given the subclass's own
    /// base cost.
    ///
    /// Equivalent to `AbstractPagedMutable.ramBytesUsed()`.
    pub fn ram_bytes_used_with_base(&self, base_ram_bytes_used: i64) -> i64 {
        let mut bytes_used = RamUsageEstimator::align_object_size(base_ram_bytes_used);
        bytes_used += RamUsageEstimator::align_object_size(RamUsageEstimator::shallow_size_of(
            &self.sub_mutables,
        ));
        for page in self.sub_mutables.iter().flatten() {
            bytes_used += page.ram_bytes_used();
        }
        bytes_used
    }
}

/// The two hooks an [`AbstractPagedMutable`] subclass supplies, and the
/// operations built on them.
///
/// Equivalent to the abstract methods `newMutable` and `newUnfilledCopy` of
/// `org.apache.lucene.util.packed.AbstractPagedMutable<T>` together with the
/// `fillPages`, `resize` and `grow` methods that call them.
pub trait AbstractPagedMutableOps: Sized {
    /// Borrows the shared state.
    fn base(&self) -> &AbstractPagedMutable;

    /// Borrows the shared state mutably.
    fn base_mut(&mut self) -> &mut AbstractPagedMutable;

    /// Creates one page.
    ///
    /// Equivalent to `AbstractPagedMutable.newMutable(int, int)`.
    ///
    /// # Errors
    ///
    /// Returns the error the concrete page constructor raises.
    fn new_mutable(
        &self,
        value_count: i32,
        bits_per_value: i32,
    ) -> Result<Box<dyn PackedIntsMutable>>;

    /// Creates an instance of `new_size` values whose pages are not yet
    /// allocated.
    ///
    /// Equivalent to `AbstractPagedMutable.newUnfilledCopy(long)`.
    ///
    /// # Errors
    ///
    /// Returns the error the concrete constructor raises.
    fn new_unfilled_copy(&self, new_size: i64) -> Result<Self>;

    /// Allocates every page.
    ///
    /// Equivalent to `AbstractPagedMutable.fillPages()`.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::new_mutable`] raises.
    fn fill_pages(&mut self) -> Result<()> {
        let num_pages = self.base().page_count();
        let page_size = self.base().page_size() as i32;
        let size = self.base().size();
        let bits_per_value = self.base().bits_per_value();
        for i in 0..num_pages {
            // do not allocate for more entries than necessary on the last page
            let value_count = if i == num_pages - 1 {
                self.base().last_page_size(size)
            } else {
                page_size
            };
            let page = self.new_mutable(value_count, bits_per_value)?;
            self.base_mut().sub_mutables[i] = Some(page);
        }
        Ok(())
    }

    /// Returns the value at `index`.
    ///
    /// Equivalent to `AbstractPagedMutable.get(long)`.
    fn get(&self, index: i64) -> i64 {
        self.base().get(index)
    }

    /// Sets the value at `index`.
    ///
    /// Equivalent to `AbstractPagedMutable.set(long, long)`.
    fn set(&mut self, index: i64, value: i64) {
        self.base_mut().set(index, value);
    }

    /// The number of values.
    ///
    /// Equivalent to `AbstractPagedMutable.size()`.
    fn size(&self) -> i64 {
        self.base().size()
    }

    /// Returns a copy of `new_size` values holding this array's content.
    ///
    /// Equivalent to `AbstractPagedMutable.resize(long)`, which is much more
    /// efficient than copying value by value.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::new_unfilled_copy`] or [`Self::new_mutable`]
    /// raises.
    fn resize(&self, new_size: i64) -> Result<Self> {
        let mut copy = self.new_unfilled_copy(new_size)?;
        let num_common_pages = std::cmp::min(copy.base().page_count(), self.base().page_count());
        let mut copy_buffer = vec![0i64; 1024];
        let copy_pages = copy.base().page_count();
        let page_size = self.base().page_size() as i32;
        for i in 0..copy_pages {
            let value_count = if i == copy_pages - 1 {
                self.base().last_page_size(new_size)
            } else {
                page_size
            };
            let bpv = if i < num_common_pages {
                self.base()
                    .page(i)
                    .map_or(self.base().bits_per_value(), |page| page.bits_per_value())
            } else {
                self.base().bits_per_value()
            };
            let page = self.new_mutable(value_count, bpv)?;
            copy.base_mut().sub_mutables[i] = Some(page);
            if i < num_common_pages {
                if let Some(source) = self.base().page(i) {
                    let copy_length = std::cmp::min(value_count, source.size());
                    let dest = copy.base_mut().sub_mutables[i]
                        .as_mut()
                        .expect("INVARIANT: the page was just allocated");
                    PackedInts::copy_with_buffer(
                        source.as_packed_ints_reader(),
                        0,
                        dest.as_mut(),
                        0,
                        copy_length,
                        &mut copy_buffer,
                    );
                }
            }
        }
        Ok(copy)
    }

    /// Returns an array that holds at least `min_size` values.
    ///
    /// Equivalent to `AbstractPagedMutable.grow(long)`, which grows by an
    /// eighth of the requested size, and by at least three values.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::resize`] raises.
    fn grow(self, min_size: i64) -> Result<Self> {
        debug_assert!(min_size >= 0);
        if min_size <= self.size() {
            return Ok(self);
        }
        let mut extra = ((min_size as u64) >> 3) as i64;
        if extra < 3 {
            extra = 3;
        }
        self.resize(min_size + extra)
    }

    /// Returns an array that holds at least one more value.
    ///
    /// Equivalent to `AbstractPagedMutable.grow()`.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::resize`] raises.
    fn grow_by_one(self) -> Result<Self> {
        let min_size = self.size() + 1;
        self.grow(min_size)
    }
}

/// A paged packed array whose pages all use the same number of bits per value.
///
/// Equivalent to `org.apache.lucene.util.packed.PagedMutable`. It is a useful
/// replacement for a single [`PackedIntsMutable`](super::PackedIntsMutable)
/// when more than two billion values must be stored.
pub struct PagedMutable {
    base: AbstractPagedMutable,
    format: Format,
}

impl PagedMutable {
    /// Creates a paged array of `size` values.
    ///
    /// Equivalent to `new PagedMutable(long, int, int, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument) when the page size or the size
    /// is out of bounds, or the chosen format cannot store the width.
    pub fn new(
        size: i64,
        page_size: usize,
        bits_per_value: i32,
        acceptable_overhead_ratio: f32,
    ) -> Result<Self> {
        let format_and_bits = PackedInts::fastest_format_and_bits(
            page_size as i32,
            bits_per_value,
            acceptable_overhead_ratio,
        );
        let mut this = Self::with_format(
            size,
            page_size,
            format_and_bits.bits_per_value,
            format_and_bits.format,
        )?;
        this.fill_pages()?;
        Ok(this)
    }

    /// Creates a paged array with a pre-computed format and width, leaving the
    /// pages unallocated.
    ///
    /// Equivalent to the package-private
    /// `PagedMutable(long, int, int, PackedInts.Format)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument) when the page size or the size
    /// is out of bounds.
    pub fn with_format(
        size: i64,
        page_size: usize,
        bits_per_value: i32,
        format: Format,
    ) -> Result<Self> {
        Ok(Self {
            base: AbstractPagedMutable::new(bits_per_value, size, page_size)?,
            format,
        })
    }

    /// The format every page uses.
    pub fn format(&self) -> Format {
        self.format
    }

    /// Returns the value at `index`.
    ///
    /// Equivalent to `AbstractPagedMutable.get(long)`, which Java declares
    /// `final`; the inherent method keeps the call unambiguous next to the
    /// [`LongValues`] implementation.
    pub fn get(&self, index: i64) -> i64 {
        self.base.get(index)
    }

    /// Sets the value at `index`.
    ///
    /// Equivalent to `AbstractPagedMutable.set(long, long)`.
    pub fn set(&mut self, index: i64, value: i64) {
        self.base.set(index, value);
    }

    /// The number of values.
    ///
    /// Equivalent to `AbstractPagedMutable.size()`.
    pub fn size(&self) -> i64 {
        self.base.size()
    }

    /// The number of values per page.
    ///
    /// Equivalent to `AbstractPagedMutable.pageSize()`.
    pub fn page_size(&self) -> usize {
        self.base.page_size()
    }
}

impl AbstractPagedMutableOps for PagedMutable {
    fn base(&self) -> &AbstractPagedMutable {
        &self.base
    }

    fn base_mut(&mut self) -> &mut AbstractPagedMutable {
        &mut self.base
    }

    fn new_mutable(
        &self,
        value_count: i32,
        bits_per_value: i32,
    ) -> Result<Box<dyn PackedIntsMutable>> {
        debug_assert!(self.base.bits_per_value() >= bits_per_value);
        PackedInts::get_mutable_with_format(value_count, self.base.bits_per_value(), self.format)
    }

    fn new_unfilled_copy(&self, new_size: i64) -> Result<Self> {
        Self::with_format(
            new_size,
            self.base.page_size(),
            self.base.bits_per_value(),
            self.format,
        )
    }
}

impl LongValues for PagedMutable {
    fn get(&self, index: i64) -> i64 {
        self.base.get(index)
    }
}

impl Accountable for PagedMutable {
    fn ram_bytes_used(&self) -> i64 {
        self.base.ram_bytes_used_with_base(
            self.base.base_ram_bytes_used() + RamUsageEstimator::NUM_BYTES_OBJECT_REF,
        )
    }
}

impl std::fmt::Debug for PagedMutable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PagedMutable(size={},pageSize={})",
            self.base.size(),
            self.base.page_size()
        )
    }
}

/// A paged packed array whose pages grow their width independently.
///
/// Equivalent to `org.apache.lucene.util.packed.PagedGrowableWriter`. Prefer
/// the [`PackedLongValues`](super::PackedLongValues) family unless random
/// write access is needed; this class is slower and less memory-efficient.
pub struct PagedGrowableWriter {
    base: AbstractPagedMutable,
    acceptable_overhead_ratio: f32,
}

impl PagedGrowableWriter {
    /// Creates a paged writer of `size` values.
    ///
    /// Equivalent to `new PagedGrowableWriter(long, int, int, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument) when the page size or the size
    /// is out of bounds, or `start_bits_per_value` is not a usable width.
    pub fn new(
        size: i64,
        page_size: usize,
        start_bits_per_value: i32,
        acceptable_overhead_ratio: f32,
    ) -> Result<Self> {
        Self::with_fill(
            size,
            page_size,
            start_bits_per_value,
            acceptable_overhead_ratio,
            true,
        )
    }

    /// Creates a paged writer, optionally leaving the pages unallocated.
    ///
    /// Equivalent to the package-private
    /// `PagedGrowableWriter(long, int, int, float, boolean)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument) when the page size or the size
    /// is out of bounds, or `start_bits_per_value` is not a usable width.
    pub fn with_fill(
        size: i64,
        page_size: usize,
        start_bits_per_value: i32,
        acceptable_overhead_ratio: f32,
        fill_pages: bool,
    ) -> Result<Self> {
        let mut this = Self {
            base: AbstractPagedMutable::new(start_bits_per_value, size, page_size)?,
            acceptable_overhead_ratio,
        };
        if fill_pages {
            this.fill_pages()?;
        }
        Ok(this)
    }

    /// The overhead ratio every page is created with.
    pub fn acceptable_overhead_ratio(&self) -> f32 {
        self.acceptable_overhead_ratio
    }

    /// Returns the value at `index`.
    ///
    /// Equivalent to `AbstractPagedMutable.get(long)`, which Java declares
    /// `final`; the inherent method keeps the call unambiguous next to the
    /// [`LongValues`] implementation.
    pub fn get(&self, index: i64) -> i64 {
        self.base.get(index)
    }

    /// Sets the value at `index`.
    ///
    /// Equivalent to `AbstractPagedMutable.set(long, long)`.
    pub fn set(&mut self, index: i64, value: i64) {
        self.base.set(index, value);
    }

    /// The number of values.
    ///
    /// Equivalent to `AbstractPagedMutable.size()`.
    pub fn size(&self) -> i64 {
        self.base.size()
    }

    /// The number of values per page.
    ///
    /// Equivalent to `AbstractPagedMutable.pageSize()`.
    pub fn page_size(&self) -> usize {
        self.base.page_size()
    }
}

impl AbstractPagedMutableOps for PagedGrowableWriter {
    fn base(&self) -> &AbstractPagedMutable {
        &self.base
    }

    fn base_mut(&mut self) -> &mut AbstractPagedMutable {
        &mut self.base
    }

    fn new_mutable(
        &self,
        value_count: i32,
        bits_per_value: i32,
    ) -> Result<Box<dyn PackedIntsMutable>> {
        Ok(Box::new(GrowableWriter::new(
            bits_per_value,
            value_count,
            self.acceptable_overhead_ratio,
        )?))
    }

    fn new_unfilled_copy(&self, new_size: i64) -> Result<Self> {
        Self::with_fill(
            new_size,
            self.base.page_size(),
            self.base.bits_per_value(),
            self.acceptable_overhead_ratio,
            false,
        )
    }
}

impl LongValues for PagedGrowableWriter {
    fn get(&self, index: i64) -> i64 {
        self.base.get(index)
    }
}

impl Accountable for PagedGrowableWriter {
    fn ram_bytes_used(&self) -> i64 {
        self.base
            .ram_bytes_used_with_base(self.base.base_ram_bytes_used() + 4)
    }
}

impl std::fmt::Debug for PagedGrowableWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PagedGrowableWriter(size={},pageSize={})",
            self.base.size(),
            self.base.page_size()
        )
    }
}
