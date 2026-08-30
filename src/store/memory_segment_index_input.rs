//! Memory-mapped [`IndexInput`] built on an array of memory segments.
//!
//! Ported from `org.apache.lucene.store.MemorySegmentIndexInput` and
//! `org.apache.lucene.store.MemorySegmentAccessInput` (the Java 21 sources
//! under `lucene/core/src/java21`).

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use super::memory_segment::{Arena, MemorySegment, Scope};
use super::native_access;
use super::{DataInput, IOContext, IndexInput, RandomAccessInput, ReadAdvice};
use crate::error::{LuceneError, Result};
use crate::util::BitUtil;

/// Provides access to the backing memory segment of an input.
///
/// Equivalent to `org.apache.lucene.store.MemorySegmentAccessInput`, which
/// Lucene describes as an expert API allowing access to the backing memory.
pub trait MemorySegmentAccessInput: RandomAccessInput {
    /// Returns the memory segment covering `len` bytes at `pos`, or `None` when
    /// the range spans more than one segment.
    ///
    /// Equivalent to `MemorySegmentAccessInput.segmentSliceOrNull(long, long)`.
    ///
    /// # Errors
    ///
    /// Returns an error when `pos` is negative or the range runs past the end
    /// of the input, as opposed to merely crossing a segment boundary, which
    /// yields `Ok(None)`.
    fn segment_slice_or_null(&self, pos: i64, len: i64) -> Result<Option<MemorySegment>>;

    /// Returns an independent clone positioned at the same place.
    ///
    /// Equivalent to `MemorySegmentAccessInput.clone()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] if the input has been closed.
    fn clone_access_input(&self) -> Result<Box<dyn MemorySegmentAccessInput>>;
}

/// Maps an [`IOContext`] onto the [`ReadAdvice`] a file should be read with.
///
/// Equivalent to the `Function<IOContext, ReadAdvice>` Lucene threads from
/// `MMapDirectory` down into every input, clone and slice it creates.
pub type ToReadAdvice = Arc<dyn Fn(&dyn IOContext) -> ReadAdvice + Send + Sync>;

/// Which of Lucene's two `MemorySegmentIndexInput` subclasses this input is.
///
/// Java splits the class in two: `SingleSegmentImpl`, an optimisation for a
/// file (or slice) that fits in one segment, and `MultiSegmentImpl`, which adds
/// the offset support slices need. Rust has no inheritance, so the two live in
/// one type distinguished by this field; every method that Java overrides
/// matches on it, and the arms are the two Java bodies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    /// `MemorySegmentIndexInput.SingleSegmentImpl`.
    Single,
    /// `MemorySegmentIndexInput.MultiSegmentImpl`, carrying its `offset` field.
    Multi { offset: i64 },
}

/// [`IndexInput`] implementation that uses an array of memory segments to
/// represent a file.
///
/// Equivalent to `org.apache.lucene.store.MemorySegmentIndexInput`, the Java 21
/// memory-mapped input behind `MMapDirectory`. For efficiency the segment size
/// must be a power of two, given as `chunk_size_power`.
///
/// Create instances with [`MemorySegmentIndexInputProvider`], which maps the
/// file and chooses the layout.
///
/// # Divergences from Lucene 10.5.0
///
/// * **Java's two subclasses are one type here.** See [`Layout`].
/// * **Java signals out-of-bounds access with exceptions used as control
///   flow** — `IndexOutOfBoundsException` from `MemorySegment` selects the
///   boundary-crossing path, and `NullPointerException` marks a closed input.
///   Rust has no exceptions, so the same decisions are taken with explicit
///   bounds checks and an explicit closed flag. The errors produced, and the
///   conditions producing them, are unchanged.
/// * **Closing releases the mapping when the last clone is dropped**, not at
///   the instant `close` is called; every clone reports
///   [`LuceneError::AlreadyClosed`] from that instant, as in Java. See the
///   [`memory_segment`](super::memory_segment) module documentation.
/// * **[`file_pointer`](IndexInput::file_pointer) cannot report a closed
///   input.** Java's `getFilePointer` calls `ensureOpen()` and throws
///   `AlreadyClosedException`; the Rucene trait method returns a plain `i64`,
///   so it returns the last position instead. Every method that can return an
///   error still reports the closed state.
/// * **`isLoaded` always answers "unknown"**, because `mincore` has no safe
///   wrapper; `None` is the "no guarantee" answer the trait already defines.
///   [`prefetch`](IndexInput::prefetch) therefore treats every page as a cache
///   miss, which issues the `madvise` Java would skip for resident pages. The
///   advice is a hint, so this costs a system call and changes no result.
pub struct MemorySegmentIndexInput {
    resource_description: String,
    length: i64,
    chunk_size_mask: i64,
    chunk_size_power: i32,
    confined: bool,
    /// Present only on the input that owns the mapping; clones and slices
    /// cannot close, exactly as in Java where they are built with a `null`
    /// arena.
    arena: Option<Arc<dyn Arena>>,
    /// The scope shared by every segment, cached so that the open check is one
    /// atomic load.
    scope: Arc<Scope>,
    segments: Vec<MemorySegment>,
    to_read_advice: ToReadAdvice,
    shared_prefetch_counter: Arc<AtomicI32>,
    layout: Layout,

    cur_segment_index: i32,
    /// Redundant with `segments[cur_segment_index]` for speed, and `None` marks
    /// the input closed — the role Java's `null` `curSegment` plays.
    cur_segment: Option<MemorySegment>,
    /// Relative to `cur_segment`, not to the file.
    cur_position: i64,
}

impl MemorySegmentIndexInput {
    /// Builds the input for a freshly mapped file, choosing the single- or
    /// multi-segment layout.
    ///
    /// Equivalent to `MemorySegmentIndexInput.newInstance(...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `segments` is empty.
    pub fn new_instance(
        resource_description: impl Into<String>,
        arena: Arc<dyn Arena>,
        segments: Vec<MemorySegment>,
        length: i64,
        chunk_size_power: i32,
        confined: bool,
        to_read_advice: ToReadAdvice,
    ) -> Result<Self> {
        let shared_prefetch_counter = Arc::new(AtomicI32::new(0));
        let layout = if segments.len() == 1 {
            Layout::Single
        } else {
            Layout::Multi { offset: 0 }
        };
        Self::build(
            resource_description.into(),
            Some(arena),
            segments,
            length,
            chunk_size_power,
            confined,
            to_read_advice,
            shared_prefetch_counter,
            layout,
        )
    }

    /// The shared part of Lucene's two private constructors.
    #[allow(clippy::too_many_arguments)]
    fn build(
        resource_description: String,
        arena: Option<Arc<dyn Arena>>,
        segments: Vec<MemorySegment>,
        length: i64,
        chunk_size_power: i32,
        confined: bool,
        to_read_advice: ToReadAdvice,
        shared_prefetch_counter: Arc<AtomicI32>,
        layout: Layout,
    ) -> Result<Self> {
        let first = segments.first().ok_or_else(|| {
            LuceneError::IllegalArgument(
                "MemorySegmentIndexInput requires at least one segment".to_string(),
            )
        })?;
        let scope = Arc::clone(first.scope());
        let cur_segment = Some(first.clone());
        let mut input = Self {
            resource_description,
            length,
            chunk_size_mask: (1i64 << chunk_size_power) - 1,
            chunk_size_power,
            confined,
            arena,
            scope,
            segments,
            to_read_advice,
            shared_prefetch_counter,
            layout,
            cur_segment_index: match layout {
                // `SingleSegmentImpl`'s constructor sets it to zero.
                Layout::Single => 0,
                Layout::Multi { .. } => -1,
            },
            cur_segment,
            cur_position: 0,
        };
        if matches!(layout, Layout::Multi { .. }) {
            // `MultiSegmentImpl`'s constructor seeks to zero.
            input.seek(0)?;
        }
        Ok(input)
    }

    /// Returns the offset the multi-segment layout applies to every position.
    ///
    /// Equivalent to `MultiSegmentImpl.offset`, and zero for
    /// `SingleSegmentImpl`.
    fn layout_offset(&self) -> i64 {
        match self.layout {
            Layout::Single => 0,
            Layout::Multi { offset } => offset,
        }
    }

    /// Returns the error Java's `alreadyClosed` produces.
    fn already_closed(&self) -> LuceneError {
        LuceneError::AlreadyClosed(format!("Already closed: {self}"))
    }

    /// Returns the error Java's `handlePositionalIOOBE` produces, for a
    /// position already expressed in this input's own coordinates.
    fn positional_error(&self, action: &str, pos: i64) -> LuceneError {
        if pos < 0 {
            LuceneError::IllegalArgument(format!("{action} negative position (pos={pos}): {self}"))
        } else {
            LuceneError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!("{action} past EOF (pos={pos}): {self}"),
            ))
        }
    }

    /// Returns the `EOFException("read past EOF: …")` Java raises from the
    /// sequential read path.
    fn read_past_eof(&self) -> LuceneError {
        LuceneError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            format!("read past EOF: {self}"),
        ))
    }

    /// Fails if this input, or the arena backing it, has been closed.
    ///
    /// Equivalent to `MemorySegmentIndexInput.ensureOpen()` combined with the
    /// scope-liveness check Java performs inside `alreadyClosed`.
    fn ensure_open(&self) -> Result<()> {
        if self.cur_segment.is_none() || !self.scope.is_alive() {
            return Err(self.already_closed());
        }
        Ok(())
    }

    /// Fails if this input is confined to another thread.
    ///
    /// Equivalent to `MemorySegmentIndexInput.ensureAccessible()`.
    fn ensure_accessible(&self) -> Result<()> {
        if self.confined && !self.scope.is_accessible_by(std::thread::current().id()) {
            return Err(LuceneError::IllegalState("confined".to_string()));
        }
        Ok(())
    }

    /// Returns the current segment, which [`ensure_open`](Self::ensure_open)
    /// has already proven present.
    fn cur_segment(&self) -> &MemorySegment {
        self.cur_segment
            .as_ref()
            .expect("INVARIANT: ensure_open() has verified the input is open")
    }

    /// Advances to the next segment that has any bytes, as Java's `readByte`
    /// boundary loop does.
    fn advance_segment(&mut self) -> Result<()> {
        loop {
            self.cur_segment_index += 1;
            let index = self.cur_segment_index;
            if index < 0 || index as usize >= self.segments.len() {
                return Err(self.read_past_eof());
            }
            let segment = self.segments[index as usize].clone();
            let empty = segment.byte_size() == 0;
            self.cur_segment = Some(segment);
            self.cur_position = 0;
            if !empty {
                return Ok(());
            }
        }
    }

    /// Reads `dst.len()` bytes that are known to cross a segment boundary.
    ///
    /// Equivalent to `MemorySegmentIndexInput.readBytesBoundary(byte[], int, int)`.
    fn read_bytes_boundary(&mut self, dst: &mut [u8]) -> Result<()> {
        let mut written = 0usize;
        let mut remaining = dst.len() as i64;
        let mut cur_avail = self.cur_segment().byte_size() - self.cur_position;
        while remaining > cur_avail {
            let take = cur_avail.max(0) as usize;
            if take > 0 {
                let pos = self.cur_position;
                if !self
                    .cur_segment()
                    .copy_to(pos, &mut dst[written..written + take])
                {
                    return Err(self.read_past_eof());
                }
            }
            remaining -= take as i64;
            written += take;
            self.cur_segment_index += 1;
            let index = self.cur_segment_index;
            if index < 0 || index as usize >= self.segments.len() {
                return Err(self.read_past_eof());
            }
            let segment = self.segments[index as usize].clone();
            cur_avail = segment.byte_size();
            self.cur_segment = Some(segment);
            self.cur_position = 0;
        }
        let pos = self.cur_position;
        let take = remaining as usize;
        if !self
            .cur_segment()
            .copy_to(pos, &mut dst[written..written + take])
        {
            return Err(self.read_past_eof());
        }
        self.cur_position += remaining;
        Ok(())
    }

    /// Positions the sequential cursor at an absolute file offset, used only by
    /// the positional reads to handle a read that crosses a boundary.
    ///
    /// Equivalent to `MemorySegmentIndexInput.setPos(long, int)`.
    fn set_pos(&mut self, absolute: i64, segment_index: i64, reported: i64) -> Result<()> {
        if segment_index < 0 || segment_index as usize >= self.segments.len() {
            return Err(self.positional_error("read", reported));
        }
        self.cur_position = absolute & self.chunk_size_mask;
        self.cur_segment_index = segment_index as i32;
        self.cur_segment = Some(self.segments[segment_index as usize].clone());
        Ok(())
    }

    /// Reads the byte at an absolute file offset.
    ///
    /// Equivalent to the base `MemorySegmentIndexInput.readByte(long)`.
    fn read_byte_absolute(&self, absolute: i64, reported: i64) -> Result<u8> {
        let index = absolute >> self.chunk_size_power;
        if index < 0 || index as usize >= self.segments.len() {
            return Err(self.positional_error("read", reported));
        }
        self.segments[index as usize]
            .get_u8(absolute & self.chunk_size_mask)
            .ok_or_else(|| self.positional_error("read", reported))
    }

    /// Reads bytes at an absolute file offset, spanning segments as needed.
    ///
    /// Equivalent to the base `MemorySegmentIndexInput.readBytes(long, byte[], int, int)`.
    fn read_bytes_absolute(&self, absolute: i64, dst: &mut [u8], reported: i64) -> Result<()> {
        let mut index = absolute >> self.chunk_size_power;
        if index < 0 || index as usize >= self.segments.len() {
            return Err(self.positional_error("read", reported));
        }
        let mut pos = absolute & self.chunk_size_mask;
        let mut written = 0usize;
        let mut remaining = dst.len() as i64;
        let mut cur_avail = self.segments[index as usize].byte_size() - pos;
        while remaining > cur_avail {
            let take = cur_avail.max(0) as usize;
            if take > 0
                && !self.segments[index as usize].copy_to(pos, &mut dst[written..written + take])
            {
                return Err(self.positional_error("read", reported));
            }
            remaining -= take as i64;
            written += take;
            index += 1;
            if index as usize >= self.segments.len() {
                return Err(self.read_past_eof());
            }
            pos = 0;
            cur_avail = self.segments[index as usize].byte_size();
        }
        let take = remaining as usize;
        if !self.segments[index as usize].copy_to(pos, &mut dst[written..written + take]) {
            return Err(self.positional_error("read", reported));
        }
        Ok(())
    }

    /// Applies `advice` to the part of one segment that covers
    /// `[offset, offset + length)`, aligned to the operating system's pages.
    ///
    /// Equivalent to `MemorySegmentIndexInput.advise(long, long, IOConsumer)`.
    /// `offset` is absolute, that is, it already includes the layout offset.
    fn advise<F>(&self, mut offset: i64, mut length: i64, advice: F) -> Result<()>
    where
        F: Fn(&MemorySegment) -> Result<()>,
    {
        let Some(native) = native_access::get_implementation() else {
            return Ok(());
        };
        self.ensure_open()?;

        let index = offset >> self.chunk_size_power;
        if index < 0 || index as usize >= self.segments.len() {
            return Err(self.read_past_eof());
        }
        let segment = &self.segments[index as usize];
        offset &= self.chunk_size_mask;
        // Compute the intersection of the current segment and the region that
        // should be advised. Only bytes stored in this segment are advised;
        // bytes on the next one are rare enough not to be worth the complexity.
        if offset + length > segment.byte_size() {
            length = segment.byte_size() - offset;
        }
        // Now align the offset with the page size, which madvise requires.
        let page_size = native.get_page_size();
        let offset_in_page = (segment.address().wrapping_add(offset as usize) % page_size) as i64;
        offset -= offset_in_page;
        length += offset_in_page;
        if offset < 0 {
            // The start of the page is before the start of this segment, so
            // ignore the first page.
            offset += page_size as i64;
            length -= page_size as i64;
            if length <= 0 {
                // This segment has no data beyond the first page.
                return Ok(());
            }
        }

        let advised = segment
            .as_slice(offset, length)
            .ok_or_else(|| self.read_past_eof())?;
        advice(&advised)
    }

    /// Re-advises the whole file for the read pattern `context` implies.
    ///
    /// Equivalent to `IndexInput.updateIOContext(IOContext)` as
    /// `MemorySegmentIndexInput` implements it.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] if the input has been closed, or
    /// [`LuceneError::Io`] if the advice call fails.
    pub fn update_io_context(&self, context: &dyn IOContext) -> Result<()> {
        self.update_read_advice((self.to_read_advice)(context))
    }

    /// Equivalent to `MemorySegmentIndexInput.updateReadAdvice(ReadAdvice)`.
    fn update_read_advice(&self, read_advice: ReadAdvice) -> Result<()> {
        let Some(native) = native_access::get_implementation() else {
            return Ok(());
        };
        let mut offset = 0i64;
        for segment in &self.segments {
            let byte_size = segment.byte_size();
            self.advise(offset, byte_size, |slice| {
                native.madvise(slice, read_advice)
            })?;
            offset += byte_size;
        }
        Ok(())
    }

    /// Creates a slice and advises the kernel about how `context` says it will
    /// be read.
    ///
    /// Equivalent to
    /// `MemorySegmentIndexInput.slice(String, long, long, IOContext)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the range is out of bounds,
    /// and [`LuceneError::AlreadyClosed`] if the input has been closed.
    pub fn slice_with_context(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
        context: &dyn IOContext,
    ) -> Result<Self> {
        let slice = self.slice_impl(slice_description, offset, length)?;
        let advice = (self.to_read_advice)(context);
        if let Some(native) = native_access::get_implementation() {
            // No need to madvise with a normal advice, since it is the OS'
            // default.
            if advice != ReadAdvice::Normal && length >= native.get_page_size() as i64 {
                // Only set the read advice if the inner file is large enough.
                // Otherwise the cons are likely to outweigh the pros: we would
                // potentially override the advice of other files sharing the
                // same pages, and pay for a madvise system call for little
                // value.
                slice.advise(0, slice.length, |segment| native.madvise(segment, advice))?;
            }
        }
        Ok(slice)
    }

    /// Creates a slice of this input, positioned at its beginning.
    ///
    /// Equivalent to `MemorySegmentIndexInput.slice(String, long, long)`, and
    /// the concretely typed counterpart of
    /// [`IndexInput::slice`](IndexInput::slice).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the range is out of bounds,
    /// and [`LuceneError::AlreadyClosed`] if the input has been closed.
    pub fn slice_impl(&self, slice_description: &str, offset: i64, length: i64) -> Result<Self> {
        if (length | offset) < 0 || length > self.length - offset {
            return Err(LuceneError::IllegalArgument(format!(
                "slice() {slice_description} out of bounds: offset={offset},length={length},fileLength={}: {self}",
                self.length
            )));
        }
        self.build_slice(slice_description, offset, length)
    }

    /// Builds the sliced input, applying the layout's extra offset.
    ///
    /// Equivalent to `MemorySegmentIndexInput.buildSlice(String, long, long)`
    /// together with `MultiSegmentImpl`'s override of it.
    fn build_slice(&self, slice_description: &str, offset: i64, length: i64) -> Result<Self> {
        self.ensure_open()?;
        self.ensure_accessible()?;

        // `MultiSegmentImpl.buildSlice` shifts by its own offset first.
        let offset = self.layout_offset() + offset;
        let is_clone = offset == 0 && length == self.length;
        let mut segment_offset = offset;
        let slices: Vec<MemorySegment> = if is_clone {
            self.segments.clone()
        } else {
            let slice_end = offset + length;
            let start_index = (offset >> self.chunk_size_power) as usize;
            let end_index = (slice_end >> self.chunk_size_power) as usize;
            if start_index >= self.segments.len() || end_index >= self.segments.len() {
                return Err(self.positional_error("slice", offset));
            }
            // Always take one more slice: after truncating with `as_slice` the
            // last one may be zero bytes long.
            let mut slices = self.segments[start_index..=end_index].to_vec();
            let last = slices.len() - 1;
            slices[last] = slices[last]
                .as_slice(0, slice_end & self.chunk_size_mask)
                .ok_or_else(|| self.positional_error("slice", offset))?;
            segment_offset = offset & self.chunk_size_mask;
            slices
        };

        let new_resource_description = self.full_slice_description(slice_description);
        if slices.len() == 1 {
            let segment = if is_clone {
                slices[0].clone()
            } else {
                slices[0]
                    .as_slice(segment_offset, length)
                    .ok_or_else(|| self.positional_error("slice", offset))?
            };
            Self::build(
                new_resource_description,
                // Clones and slices have no arena, as they cannot close.
                None,
                vec![segment],
                length,
                self.chunk_size_power,
                self.confined,
                Arc::clone(&self.to_read_advice),
                Arc::clone(&self.shared_prefetch_counter),
                Layout::Single,
            )
        } else {
            Self::build(
                new_resource_description,
                None,
                slices,
                length,
                self.chunk_size_power,
                self.confined,
                Arc::clone(&self.to_read_advice),
                Arc::clone(&self.shared_prefetch_counter),
                Layout::Multi {
                    offset: segment_offset,
                },
            )
        }
    }

    /// Returns an independent clone positioned at the same place.
    ///
    /// Equivalent to `MemorySegmentIndexInput.clone()`, and the concretely
    /// typed counterpart of [`IndexInput::clone_input`].
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] if the input has been closed.
    pub fn clone_impl(&self) -> Result<Self> {
        self.ensure_open()?;
        self.ensure_accessible()?;
        let mut clone = Self::build(
            self.resource_description.clone(),
            // Clones don't have an arena, as they can't close.
            None,
            self.segments.clone(),
            self.length,
            self.chunk_size_power,
            self.confined,
            Arc::clone(&self.to_read_advice),
            Arc::clone(&self.shared_prefetch_counter),
            self.layout,
        )?;
        clone.seek(self.file_pointer())?;
        Ok(clone)
    }

    /// Returns `true` if `index` is a valid position in a region of `length`
    /// bytes.
    ///
    /// Equivalent to `MemorySegmentIndexInput.checkIndex(long, long)`.
    fn check_index(index: i64, length: i64) -> bool {
        index >= 0 && index < length
    }
}

/// Validates that `offset` and `length` describe a sub-range of `size`.
///
/// Equivalent to `Objects.checkFromIndexSize(long, long, long)`.
fn check_from_index_size(offset: i64, length: i64, size: i64) -> Result<()> {
    if offset < 0 || length < 0 || offset > size - length {
        return Err(LuceneError::IllegalArgument(format!(
            "Range [{offset}, {offset} + {length}) out of bounds for length {size}"
        )));
    }
    Ok(())
}

/// Validates that `offset` and `length` describe a sub-slice of `len`
/// elements.
fn check_dst(offset: usize, length: usize, len: usize) -> Result<()> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| LuceneError::IllegalArgument("offset + length overflowed".to_string()))?;
    if end > len {
        return Err(LuceneError::IllegalArgument(format!(
            "offset {offset} + length {length} exceeds array length {len}"
        )));
    }
    Ok(())
}

impl DataInput for MemorySegmentIndexInput {
    fn read_byte(&mut self) -> Result<u8> {
        self.ensure_open()?;
        let pos = self.cur_position;
        if let Some(value) = self.cur_segment().get_u8(pos) {
            self.cur_position += 1;
            return Ok(value);
        }
        self.advance_segment()?;
        let pos = self.cur_position;
        let value = self
            .cur_segment()
            .get_u8(pos)
            .ok_or_else(|| self.read_past_eof())?;
        self.cur_position += 1;
        Ok(value)
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        self.ensure_open()?;
        check_dst(offset, len, b.len())?;
        let pos = self.cur_position;
        if self
            .cur_segment()
            .copy_to(pos, &mut b[offset..offset + len])
        {
            self.cur_position += len as i64;
            return Ok(());
        }
        self.read_bytes_boundary(&mut b[offset..offset + len])
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be >= 0, got {num_bytes}"
            )));
        }
        let skip_to = self.file_pointer() + num_bytes;
        self.seek(skip_to)
    }

    fn read_short(&mut self) -> Result<i16> {
        self.ensure_open()?;
        let pos = self.cur_position;
        if let Some(value) = self.cur_segment().get_i16_le(pos) {
            self.cur_position += 2;
            return Ok(value);
        }
        // Crosses a segment boundary, or runs past the end: fall back to the
        // byte-at-a-time decoding `DataInput` defines.
        let b1 = self.read_byte()? as u16;
        let b2 = self.read_byte()? as u16;
        Ok(((b2 << 8) | b1) as i16)
    }

    fn read_int(&mut self) -> Result<i32> {
        self.ensure_open()?;
        let pos = self.cur_position;
        if let Some(value) = self.cur_segment().get_i32_le(pos) {
            self.cur_position += 4;
            return Ok(value);
        }
        let b1 = self.read_byte()? as u32;
        let b2 = self.read_byte()? as u32;
        let b3 = self.read_byte()? as u32;
        let b4 = self.read_byte()? as u32;
        Ok(((b4 << 24) | (b3 << 16) | (b2 << 8) | b1) as i32)
    }

    fn read_long(&mut self) -> Result<i64> {
        self.ensure_open()?;
        let pos = self.cur_position;
        if let Some(value) = self.cur_segment().get_i64_le(pos) {
            self.cur_position += 8;
            return Ok(value);
        }
        let low = self.read_int()? as u32 as i64;
        let high = self.read_int()? as i64;
        Ok((high << 32) | low)
    }

    fn read_ints(&mut self, dst: &mut [i32], offset: usize, length: usize) -> Result<()> {
        self.ensure_open()?;
        check_dst(offset, length, dst.len())?;
        let pos = self.cur_position;
        let byte_len = length as i64 * 4;
        if pos >= 0 && self.cur_segment().byte_size() - pos >= byte_len {
            {
                let segment = self.cur_segment();
                for (index, slot) in dst[offset..offset + length].iter_mut().enumerate() {
                    *slot = segment
                        .get_i32_le(pos + 4 * index as i64)
                        .ok_or_else(|| self.read_past_eof())?;
                }
            }
            self.cur_position += byte_len;
            return Ok(());
        }
        for slot in dst[offset..offset + length].iter_mut() {
            *slot = self.read_int()?;
        }
        Ok(())
    }

    fn read_longs(&mut self, dst: &mut [i64], offset: usize, length: usize) -> Result<()> {
        self.ensure_open()?;
        check_dst(offset, length, dst.len())?;
        let pos = self.cur_position;
        let byte_len = length as i64 * 8;
        if pos >= 0 && self.cur_segment().byte_size() - pos >= byte_len {
            {
                let segment = self.cur_segment();
                for (index, slot) in dst[offset..offset + length].iter_mut().enumerate() {
                    *slot = segment
                        .get_i64_le(pos + 8 * index as i64)
                        .ok_or_else(|| self.read_past_eof())?;
                }
            }
            self.cur_position += byte_len;
            return Ok(());
        }
        for slot in dst[offset..offset + length].iter_mut() {
            *slot = self.read_long()?;
        }
        Ok(())
    }

    fn read_floats(&mut self, floats: &mut [f32], offset: usize, length: usize) -> Result<()> {
        self.ensure_open()?;
        check_dst(offset, length, floats.len())?;
        let pos = self.cur_position;
        let byte_len = length as i64 * 4;
        if pos >= 0 && self.cur_segment().byte_size() - pos >= byte_len {
            {
                let segment = self.cur_segment();
                for (index, slot) in floats[offset..offset + length].iter_mut().enumerate() {
                    *slot = segment
                        .get_f32_le(pos + 4 * index as i64)
                        .ok_or_else(|| self.read_past_eof())?;
                }
            }
            self.cur_position += byte_len;
            return Ok(());
        }
        for slot in floats[offset..offset + length].iter_mut() {
            *slot = self.read_float()?;
        }
        Ok(())
    }
}

impl IndexInput for MemorySegmentIndexInput {
    fn close(&mut self) -> Result<()> {
        if self.cur_segment.is_none() {
            return Ok(());
        }
        // The input that owns the arena releases every segment of the group; a
        // side effect is that clones still in use start reporting
        // `AlreadyClosed`.
        if let Some(arena) = self.arena.take() {
            arena.close();
        }
        // Make sure all further accesses to this instance report the closed
        // state, the role Java's `curSegment = null` plays.
        self.cur_segment = None;
        self.segments.clear();
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        match self.layout {
            Layout::Single => self.cur_position,
            Layout::Multi { offset } => {
                ((self.cur_segment_index as i64) << self.chunk_size_power) + self.cur_position
                    - offset
            }
        }
    }

    fn length(&self) -> i64 {
        self.length
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        self.ensure_open()?;
        match self.layout {
            Layout::Single => {
                if !Self::check_index(pos, self.length + 1) {
                    return Err(self.positional_error("seek", pos));
                }
                self.cur_position = pos;
                Ok(())
            }
            Layout::Multi { offset } => {
                let absolute = pos + offset;
                // Java uses `>>` here to preserve the sign, so that a negative
                // position is caught as an out-of-bounds array index.
                let index = absolute >> self.chunk_size_power;
                if index < 0 || index as usize >= self.segments.len() {
                    return Err(self.positional_error("seek", pos));
                }
                if index as i32 != self.cur_segment_index {
                    // Write both values, so that on failure all is unchanged.
                    self.cur_segment_index = index as i32;
                    self.cur_segment = Some(self.segments[index as usize].clone());
                }
                let in_segment = absolute & self.chunk_size_mask;
                if !Self::check_index(in_segment, self.cur_segment().byte_size() + 1) {
                    return Err(self.positional_error("seek", pos));
                }
                self.cur_position = in_segment;
                Ok(())
            }
        }
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(self.slice_impl(
            slice_description,
            offset,
            length,
        )?))
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(self.clone_impl()?))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }

    fn prefetch(&self, offset: i64, length: i64) -> Result<()> {
        // Both `SingleSegmentImpl` and `MultiSegmentImpl` validate the range
        // against this input's own length before delegating.
        check_from_index_size(offset, length, self.length)?;

        let Some(native) = native_access::get_implementation() else {
            return Ok(());
        };
        self.ensure_open()?;

        if !BitUtil::is_zero_or_power_of_two(
            self.shared_prefetch_counter.fetch_add(1, Ordering::SeqCst),
        ) {
            // We've had enough consecutive hits on the page cache that this
            // number is neither zero nor a power of two. There is a good chance
            // that a good chunk of this index input is cached in physical
            // memory. Skip the overhead of the madvise system call; we'll try
            // again on the next power of two of the counter.
            return Ok(());
        }

        let counter = &self.shared_prefetch_counter;
        self.advise(self.layout_offset() + offset, length, move |segment| {
            // `MemorySegment.isLoaded()` has no safe counterpart, so an unknown
            // answer is treated as a cache miss, which is the conservative
            // choice: the advice is only a hint.
            if segment.is_loaded() != Some(true) {
                // We have a cache miss on at least one page, so reset the
                // counter.
                counter.store(0, Ordering::SeqCst);
                native.madvise_will_need(segment)?;
            }
            Ok(())
        })
    }

    fn is_loaded(&self) -> Option<bool> {
        // `MemorySegment.isLoaded()` is backed by `mincore`, which has no safe
        // wrapper here, so this input can make no claim either way.
        None
    }

    fn random_access_slice(&self, offset: i64, length: i64) -> Result<Box<dyn RandomAccessInput>> {
        Ok(Box::new(self.slice_impl("randomaccess", offset, length)?))
    }
}

impl RandomAccessInput for MemorySegmentIndexInput {
    fn read_byte_at(&mut self, pos: i64) -> Result<u8> {
        self.ensure_open()?;
        match self.layout {
            Layout::Single => self
                .cur_segment()
                .get_u8(pos)
                .ok_or_else(|| self.positional_error("read", pos)),
            Layout::Multi { offset } => self.read_byte_absolute(pos + offset, pos),
        }
    }

    fn read_bytes_at(
        &mut self,
        pos: i64,
        bytes: &mut [u8],
        offset: usize,
        len: usize,
    ) -> Result<()> {
        self.ensure_open()?;
        check_dst(offset, len, bytes.len())?;
        match self.layout {
            Layout::Single => {
                if self
                    .cur_segment()
                    .copy_to(pos, &mut bytes[offset..offset + len])
                {
                    Ok(())
                } else {
                    Err(self.positional_error("read", pos))
                }
            }
            Layout::Multi { offset: base } => {
                self.read_bytes_absolute(pos + base, &mut bytes[offset..offset + len], pos)
            }
        }
    }

    fn read_short_at(&mut self, pos: i64) -> Result<i16> {
        self.ensure_open()?;
        match self.layout {
            Layout::Single => self
                .cur_segment()
                .get_i16_le(pos)
                .ok_or_else(|| self.positional_error("read", pos)),
            Layout::Multi { offset } => {
                let absolute = pos + offset;
                let index = absolute >> self.chunk_size_power;
                if index >= 0 && (index as usize) < self.segments.len() {
                    if let Some(value) =
                        self.segments[index as usize].get_i16_le(absolute & self.chunk_size_mask)
                    {
                        return Ok(value);
                    }
                }
                // Either it's a boundary, or a read past EOF: fall back.
                self.set_pos(absolute, index, pos)?;
                self.read_short()
            }
        }
    }

    fn read_int_at(&mut self, pos: i64) -> Result<i32> {
        self.ensure_open()?;
        match self.layout {
            Layout::Single => self
                .cur_segment()
                .get_i32_le(pos)
                .ok_or_else(|| self.positional_error("read", pos)),
            Layout::Multi { offset } => {
                let absolute = pos + offset;
                let index = absolute >> self.chunk_size_power;
                if index >= 0 && (index as usize) < self.segments.len() {
                    if let Some(value) =
                        self.segments[index as usize].get_i32_le(absolute & self.chunk_size_mask)
                    {
                        return Ok(value);
                    }
                }
                self.set_pos(absolute, index, pos)?;
                self.read_int()
            }
        }
    }

    fn read_long_at(&mut self, pos: i64) -> Result<i64> {
        self.ensure_open()?;
        match self.layout {
            Layout::Single => self
                .cur_segment()
                .get_i64_le(pos)
                .ok_or_else(|| self.positional_error("read", pos)),
            Layout::Multi { offset } => {
                let absolute = pos + offset;
                let index = absolute >> self.chunk_size_power;
                if index >= 0 && (index as usize) < self.segments.len() {
                    if let Some(value) =
                        self.segments[index as usize].get_i64_le(absolute & self.chunk_size_mask)
                    {
                        return Ok(value);
                    }
                }
                self.set_pos(absolute, index, pos)?;
                self.read_long()
            }
        }
    }
}

impl MemorySegmentAccessInput for MemorySegmentIndexInput {
    fn segment_slice_or_null(&self, pos: i64, len: i64) -> Result<Option<MemorySegment>> {
        self.ensure_open()?;
        match self.layout {
            Layout::Single => {
                if !Self::check_index(pos.saturating_add(len), self.length + 1) {
                    return Err(self.positional_error("segmentSliceOrNull", pos));
                }
                // Java wraps both the bounds check and `asSlice` in one `try`,
                // so a negative position is an error here too, never `null`.
                Ok(Some(self.cur_segment().as_slice(pos, len).ok_or_else(
                    || self.positional_error("segmentSliceOrNull", pos),
                )?))
            }
            Layout::Multi { offset } => {
                if pos + len > self.length {
                    return Err(self.positional_error("segmentSliceOrNull", pos));
                }
                let absolute = pos + offset;
                let index = absolute >> self.chunk_size_power;
                if index < 0 || index as usize >= self.segments.len() {
                    return Err(self.positional_error("segmentSliceOrNull", pos));
                }
                let segment = &self.segments[index as usize];
                let segment_offset = absolute & self.chunk_size_mask;
                if Self::check_index(segment_offset + len, segment.byte_size() + 1) {
                    return Ok(segment.as_slice(segment_offset, len));
                }
                Ok(None)
            }
        }
    }

    fn clone_access_input(&self) -> Result<Box<dyn MemorySegmentAccessInput>> {
        Ok(Box::new(self.clone_impl()?))
    }
}

impl std::fmt::Display for MemorySegmentIndexInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Java's `IndexInput.toString()` returns the resource description.
        f.write_str(&self.resource_description)
    }
}

impl std::fmt::Debug for MemorySegmentIndexInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemorySegmentIndexInput")
            .field("resource_description", &self.resource_description)
            .field("length", &self.length)
            .field("chunk_size_power", &self.chunk_size_power)
            .field("layout", &self.layout)
            .field("file_pointer", &self.file_pointer())
            .field(
                "open",
                &(self.cur_segment.is_some() && self.scope.is_alive()),
            )
            .finish()
    }
}
