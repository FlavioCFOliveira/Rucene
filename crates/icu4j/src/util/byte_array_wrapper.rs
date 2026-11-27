// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.ByteArrayWrapper`.

use std::cmp::Ordering;
use std::fmt;

use rucene::error::Result;

use crate::jdk::ByteBuffer;
use crate::r#impl::utility::Utility;

/// A simple utility class to wrap a byte array: the bytes and the number
/// of them in use.
///
/// Equivalent to `com.ibm.icu.util.ByteArrayWrapper`, which implements
/// `Comparable<ByteArrayWrapper>` ([`Ord`], [`compare_to`](Self::compare_to)).
#[derive(Debug, Clone, Default)]
pub struct ByteArrayWrapper {
    /// `bytes`: internal byte array; `None` for Java's `null`.
    pub bytes: Option<Vec<i8>>,
    /// `size`: size of the internal byte array used. Different from
    /// `bytes.len()`; `size` is at most `bytes.len()`.
    pub size: i32,
}

impl ByteArrayWrapper {
    /// `ByteArrayWrapper()`: no bytes, size 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// `ByteArrayWrapper(byte[] bytesToAdopt, int size)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException("illegal size: <size>")` when `size` is
    /// negative, above the array length, or not 0 for a `None` array.
    pub fn with_bytes(bytes_to_adopt: Option<Vec<i8>>, size: i32) -> Self {
        let bad = match &bytes_to_adopt {
            None => size != 0,
            Some(b) => size > b.len() as i32,
        } || size < 0;
        if bad {
            crate::jdk::index_out_of_bounds(Some(format!("illegal size: {size}")));
        }
        Self {
            bytes: bytes_to_adopt,
            size,
        }
    }

    /// `ByteArrayWrapper(ByteBuffer source)`: copies `source.limit()` bytes
    /// from the buffer's position.
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` when fewer bytes remain.
    pub fn from_byte_buffer(source: &mut ByteBuffer) -> Result<Self> {
        let size = source.limit();
        let mut bytes = vec![0u8; size as usize];
        source.get_bytes(&mut bytes)?;
        Ok(Self {
            bytes: Some(bytes.into_iter().map(|b| b as i8).collect()),
            size,
        })
    }

    /// `ensureCapacity(int capacity)`: grows the array to at least
    /// `capacity` bytes, keeping the used bytes.
    pub fn ensure_capacity(&mut self, capacity: i32) -> &mut Self {
        if self
            .bytes
            .as_ref()
            .is_none_or(|b| (b.len() as i32) < capacity)
        {
            let mut newbytes = vec![0i8; capacity as usize];
            if let Some(b) = &self.bytes {
                copy_bytes(b, 0, &mut newbytes, 0, self.size);
            }
            self.bytes = Some(newbytes);
        }
        self
    }

    /// `set(byte[] src, int start, int limit)` (final): the bytes
    /// `src[start..limit]`.
    pub fn set(&mut self, src: &[i8], start: i32, limit: i32) -> &mut Self {
        self.size = 0;
        self.append(src, start, limit);
        self
    }

    /// `append(byte[] src, int start, int limit)` (final): appends
    /// `src[start..limit]`.
    pub fn append(&mut self, src: &[i8], start: i32, limit: i32) -> &mut Self {
        let len = limit - start;
        self.ensure_capacity(self.size + len);
        let size = self.size;
        copy_bytes(
            src,
            start,
            self.bytes.as_mut().expect("ensureCapacity allocated"),
            size,
            len,
        );
        self.size += len;
        self
    }

    /// `releaseBytes()` (final): hands the array over and empties the
    /// wrapper.
    pub fn release_bytes(&mut self) -> Option<Vec<i8>> {
        let result = self.bytes.take();
        self.size = 0;
        result
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        let mut result = self.size;
        if let Some(b) = &self.bytes {
            for &v in &b[..self.size as usize] {
                result = result.wrapping_mul(37).wrapping_add(v as i32);
            }
        }
        result
    }

    /// `compareTo(ByteArrayWrapper other)`: unsigned byte order, then size.
    pub fn compare_to(&self, other: &ByteArrayWrapper) -> i32 {
        if std::ptr::eq(self, other) {
            return 0;
        }
        let min_size = self.size.min(other.size) as usize;
        let (a, b) = (
            self.bytes.as_deref().unwrap_or(&[]),
            other.bytes.as_deref().unwrap_or(&[]),
        );
        for i in 0..min_size {
            if a[i] != b[i] {
                return (a[i] as i32 & 0xFF) - (b[i] as i32 & 0xFF);
            }
        }
        self.size - other.size
    }
}

impl PartialEq for ByteArrayWrapper {
    /// `equals(Object other)`: the same size and the same used bytes.
    fn eq(&self, that: &Self) -> bool {
        if std::ptr::eq(self, that) {
            return true;
        }
        if self.size != that.size {
            return false;
        }
        let (a, b) = (
            self.bytes.as_deref().unwrap_or(&[]),
            that.bytes.as_deref().unwrap_or(&[]),
        );
        for i in 0..self.size as usize {
            if a[i] != b[i] {
                return false;
            }
        }
        true
    }
}

impl Eq for ByteArrayWrapper {}

impl PartialOrd for ByteArrayWrapper {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ByteArrayWrapper {
    /// The order of [`compare_to`](ByteArrayWrapper::compare_to).
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare_to(other).cmp(&0)
    }
}

impl fmt::Display for ByteArrayWrapper {
    /// `toString()`: the used bytes in two-digit hexadecimal, separated by
    /// spaces.
    #[allow(clippy::needless_range_loop)] // indexed loop mirrors the Java for-loop
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = self.bytes.as_deref().unwrap_or(&[]);
        for i in 0..self.size as usize {
            if i != 0 {
                f.write_str(" ")?;
            }
            f.write_str(&Utility::hex_places(b[i] as i64 & 0xFF, 2))?;
        }
        Ok(())
    }
}

/// `copyBytes(byte[] src, int srcoff, byte[] tgt, int tgtoff, int length)`
/// (private, static): a byte loop for short lengths, `System.arraycopy`
/// otherwise (the same result).
fn copy_bytes(src: &[i8], srcoff: i32, tgt: &mut [i8], tgtoff: i32, length: i32) {
    let (s, t, n) = (srcoff as usize, tgtoff as usize, length as usize);
    tgt[t..t + n].copy_from_slice(&src[s..s + n]);
}
